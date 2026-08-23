pub(crate) mod builder;
mod postgres;
pub(crate) mod protocol;

#[cfg(test)]
mod live;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use tokio::sync::{oneshot, Mutex, Notify};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;

use builder::build_mutation_plan;
use postgres::{DescriptorCache, MutationConnection};
use protocol::*;

const MAX_EXECUTORS: usize = 8;
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const QUEUE_WAIT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const ANALYSIS_CACHE_CAPACITY: usize = 16;
static NEXT_ANALYSIS_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) type VirtualKeyLookup = Arc<
    dyn Fn(
            String,
            String,
            String,
        ) -> BoxFuture<'static, Result<Option<VirtualKey>, ResultMutationError>>
        + Send
        + Sync,
>;

fn apply_write_intent(plan: &MutationPlan) -> crate::safety::policy::WriteIntent {
    crate::safety::policy::WriteIntent::ApplyMutations {
        classes: plan
            .operations
            .iter()
            .map(|_| crate::postgres::sql_class::StatementClass::Dml {
                unbounded: false,
                destructive: false,
            })
            .collect(),
    }
}

#[derive(Default)]
struct ManagerState {
    executors: HashMap<String, Arc<Executor>>,
    closing: HashSet<String>,
    lingering_closes: HashSet<String>,
    connection_end_requested: HashSet<String>,
    global_closing: bool,
    global_close_pending: usize,
    global_end_requested: bool,
}

#[derive(Clone)]
pub(crate) struct ResultMutationManager {
    inner: Arc<Mutex<ManagerState>>,
    changed: Arc<Notify>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyOutcome {
    result: ApplyResult,
    intent: crate::safety::policy::WriteIntent,
    authorization: crate::safety::policy::SafetyAuthorization,
}

impl ApplyOutcome {
    pub(crate) fn into_parts(
        self,
    ) -> (
        ApplyResult,
        crate::safety::policy::WriteIntent,
        crate::safety::policy::SafetyAuthorization,
    ) {
        (self.result, self.intent, self.authorization)
    }
}

impl std::ops::Deref for ApplyOutcome {
    type Target = ApplyResult;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

impl ResultMutationManager {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerState::default())),
            changed: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn start_monitor(&self) {
        let manager = self.clone();
        // Spawn via Tauri's global runtime: setup() calls this from the
        // main thread with no ambient Tokio context, where tokio::spawn
        // would panic.
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                manager.close_idle().await;
            }
        });
    }

    pub(crate) async fn analyze(
        &self,
        spec: ResolvedPostgresConnectSpec,
        payload: AnalyzeResultSetPayload,
        virtual_keys: VirtualKeyLookup,
    ) -> Result<AnalyzeResultSetResult, ResultMutationError> {
        let executor = self.executor_for(spec, &payload.connection_id).await?;
        let tab_id = payload.tab_id.clone();
        let request_id = payload.request_id;
        let (reply, mut result) = oneshot::channel();
        let cancel = {
            let mut state = executor.state.lock().await;
            if state.closed {
                return Err(ResultMutationError::ConnectionClosing);
            }
            if payload.refresh_structure {
                invalidate_structure_state(&mut state);
            }
            let generation = state.structure_generation;
            enqueue_analysis(
                &mut state,
                AnalysisJob {
                    tab_id: tab_id.clone(),
                    request_id,
                    enqueued_at: Instant::now(),
                    payload,
                    generation,
                    virtual_keys,
                    reply,
                },
            )
        };
        perform_cancel(&executor, cancel).await;
        executor.notify.notify_one();
        tokio::select! {
            response = &mut result => response.unwrap_or(Err(ResultMutationError::ConnectionLost)),
            _ = tokio::time::sleep(QUEUE_WAIT) => {
                let timed_out = take_queued(&mut *executor.state.lock().await, &tab_id, request_id);
                if let Some(job) = timed_out {
                    let error = ResultMutationError::Timeout { operation: "queueWait".into() };
                    let _ = job.reply.send(Err(error.clone()));
                    Err(error)
                } else {
                    result.await.unwrap_or(Err(ResultMutationError::ConnectionLost))
                }
            }
        }
    }

    pub(crate) async fn preview(
        &self,
        payload: PreviewResultMutationsPayload,
    ) -> Result<PreviewResult, ResultMutationError> {
        let executor = self
            .existing_executor(&payload.connection_id)
            .await
            .ok_or(ResultMutationError::AnalysisExpired)?;
        let descriptors = {
            let mut state = executor.state.lock().await;
            if state.closed {
                return Err(ResultMutationError::ConnectionClosing);
            }
            state.last_used = Instant::now();
            state
                .snapshots
                .get(payload.analysis_id)
                .filter(|snapshot| snapshot.tab_id == payload.tab_id)
                .map(|snapshot| snapshot.descriptors)
                .ok_or(ResultMutationError::AnalysisExpired)?
        };
        let mutation_descriptors = descriptors
            .iter()
            .map(|descriptor| descriptor.mutation.clone())
            .collect::<Vec<_>>();
        Ok(PreviewResult {
            statements: build_mutation_plan(&mutation_descriptors, &payload.plan)?,
        })
    }

    pub(crate) async fn apply(
        &self,
        spec: ResolvedPostgresConnectSpec,
        payload: ApplyResultMutationsPayload,
    ) -> Result<ApplyOutcome, ResultMutationError> {
        let intent = apply_write_intent(&payload.plan);
        let authorization = crate::safety::policy::assert_permitted(
            &spec.safety_policy,
            &intent,
            payload.confirmed,
        )
        .map_err(|refusal| {
            refusal.fold(
                |reason, _| ResultMutationError::PolicyBlocked {
                    reason: reason.to_string(),
                },
                |statements| ResultMutationError::PolicyNeedsConfirmation { statements },
            )
        })?;
        // An analysis snapshot can only live on an existing executor. Validate
        // this before opening or consuming admission capacity: stale IDs are a
        // cache miss, not a reason to create a database socket.
        let executor = self
            .existing_executor(&payload.connection_id)
            .await
            .ok_or(ResultMutationError::AnalysisExpired)?;
        {
            let mut state = executor.state.lock().await;
            begin_apply(&mut state, &payload)?;
        }
        let (reply, result) = oneshot::channel();
        let (caller_dropped, dropped) = oneshot::channel();
        let task_executor = executor.clone();
        tokio::spawn(async move {
            run_owned_apply(task_executor, payload, dropped, reply).await;
        });
        let mut guard = ApplyCallerGuard::new(caller_dropped);
        let result = result
            .await
            .unwrap_or(Err(ResultMutationError::ConnectionLost));
        guard.complete();
        result.map(|result| ApplyOutcome {
            result,
            intent,
            authorization,
        })
    }

    pub(crate) async fn cancel_tab(
        &self,
        connection_id: &str,
        tab_id: &str,
    ) -> CancelResultMutationResult {
        let Some(executor) = self.existing_executor(connection_id).await else {
            return CancelResultMutationResult {
                cancel_requested: false,
            };
        };
        let (cancel_requested, cancel) =
            cancel_tab_state(&mut *executor.state.lock().await, tab_id);
        perform_cancel(&executor, cancel).await;
        executor.notify.notify_waiters();
        CancelResultMutationResult { cancel_requested }
    }

    pub(crate) async fn invalidate_virtual_key(
        &self,
        connection_id: &str,
        schema: &str,
        table: &str,
    ) {
        if let Some(executor) = self.existing_executor(connection_id).await {
            let mut state = executor.state.lock().await;
            state.descriptors.remove(&(schema.into(), table.into()));
            invalidate_structure_state(&mut state);
        }
    }

    pub(crate) async fn close_connection(&self, connection_id: &str) {
        self.begin_connection_teardown(connection_id).await;
        self.end_connection_teardown(connection_id).await;
    }

    pub(crate) async fn begin_connection_teardown(&self, connection_id: &str) {
        self.begin_connection_teardown_with_timeout(connection_id, CLOSE_TIMEOUT)
            .await;
    }

    async fn begin_connection_teardown_with_timeout(
        &self,
        connection_id: &str,
        close_timeout: Duration,
    ) {
        let executor = {
            let mut state = self.inner.lock().await;
            state.connection_end_requested.remove(connection_id);
            state.closing.insert(connection_id.into());
            state.executors.remove(connection_id)
        };
        self.changed.notify_waiters();
        if let Some(executor) = executor {
            let preparation = prepare_executor_close(&executor, true).await;
            let close_executor = executor.clone();
            let mut close = tokio::spawn(async move {
                finish_executor_close(&close_executor, preparation).await;
            });
            if tokio::time::timeout(close_timeout, &mut close)
                .await
                .is_err()
            {
                self.inner
                    .lock()
                    .await
                    .lingering_closes
                    .insert(connection_id.into());
                let manager = self.clone();
                let connection_id = connection_id.to_string();
                tokio::spawn(async move {
                    let _ = close.await;
                    let mut state = manager.inner.lock().await;
                    state.lingering_closes.remove(&connection_id);
                    if state.connection_end_requested.remove(&connection_id) {
                        state.closing.remove(&connection_id);
                    }
                    drop(state);
                    manager.changed.notify_waiters();
                });
            }
        }
    }

    pub(crate) async fn end_connection_teardown(&self, connection_id: &str) {
        let mut state = self.inner.lock().await;
        if state.lingering_closes.contains(connection_id) {
            state.connection_end_requested.insert(connection_id.into());
        } else {
            state.closing.remove(connection_id);
            state.connection_end_requested.remove(connection_id);
        }
        drop(state);
        self.changed.notify_waiters();
    }

    pub(crate) async fn close_all(&self) {
        let (executors, track_global) = {
            let mut state = self.inner.lock().await;
            let executors = state
                .executors
                .drain()
                .map(|(_, executor)| executor)
                .collect::<Vec<_>>();
            let track_global = state.global_closing;
            if track_global {
                state.global_close_pending =
                    state.global_close_pending.saturating_add(executors.len());
            }
            (executors, track_global)
        };
        let mut closes = Vec::new();
        for executor in executors {
            let preparation = prepare_executor_close(&executor, true).await;
            let manager = self.clone();
            let close = tokio::spawn(async move {
                finish_executor_close(&executor, preparation).await;
                if track_global {
                    manager.complete_global_close().await;
                }
            });
            closes.push(close);
        }
        let _ = tokio::time::timeout(
            CLOSE_TIMEOUT,
            futures_util::future::join_all(closes.iter_mut()),
        )
        .await;
    }

    pub(crate) async fn begin_global_teardown(&self) {
        let mut state = self.inner.lock().await;
        state.global_closing = true;
        state.global_end_requested = false;
        drop(state);
        self.changed.notify_waiters();
        self.close_all().await;
    }

    pub(crate) async fn end_global_teardown(&self) {
        let mut state = self.inner.lock().await;
        if state.global_close_pending == 0 {
            state.global_closing = false;
        } else {
            state.global_end_requested = true;
        }
        drop(state);
        self.changed.notify_waiters();
    }

    async fn complete_global_close(&self) {
        let mut state = self.inner.lock().await;
        state.global_close_pending = state.global_close_pending.saturating_sub(1);
        if state.global_close_pending == 0 && state.global_end_requested {
            state.global_closing = false;
            state.global_end_requested = false;
        }
        drop(state);
        self.changed.notify_waiters();
    }

    async fn existing_executor(&self, connection_id: &str) -> Option<Arc<Executor>> {
        self.inner
            .lock()
            .await
            .executors
            .get(connection_id)
            .cloned()
    }

    async fn executor_for(
        &self,
        spec: ResolvedPostgresConnectSpec,
        connection_id: &str,
    ) -> Result<Arc<Executor>, ResultMutationError> {
        let mut state = self.inner.lock().await;
        if state.global_closing || state.closing.contains(connection_id) {
            return Err(ResultMutationError::ConnectionClosing);
        }
        if let Some(executor) = state.executors.get(connection_id).cloned() {
            return Ok(executor);
        }
        check_admission(&state)?;
        let executor = new_executor(spec);
        state
            .executors
            .insert(connection_id.into(), executor.clone());
        // Installation and worker startup are one cancellation-free critical
        // section. Before this point no worker exists; after it the manager
        // owns and can fence the worker.
        start_executor_worker(&executor);
        drop(state);
        self.changed.notify_waiters();
        Ok(executor)
    }

    async fn close_idle(&self) {
        let candidates = self
            .inner
            .lock()
            .await
            .executors
            .iter()
            .map(|(id, executor)| (id.clone(), executor.clone()))
            .collect::<Vec<_>>();
        for (connection_id, executor) in candidates {
            if !is_idle(&*executor.state.lock().await) {
                continue;
            }
            let removed = {
                let mut manager = self.inner.lock().await;
                let still_idle = match manager.executors.get(&connection_id) {
                    Some(current) => is_idle(&*current.state.lock().await),
                    None => false,
                };
                if still_idle {
                    manager.closing.insert(connection_id.clone());
                    manager.executors.remove(&connection_id)
                } else {
                    None
                }
            };
            if let Some(executor) = removed {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, close_executor(&executor, false)).await;
                self.inner.lock().await.closing.remove(&connection_id);
                self.changed.notify_waiters();
            }
        }
    }
}

struct Executor {
    spec: ResolvedPostgresConnectSpec,
    state: Mutex<ExecutorState>,
    notify: Notify,
    worker_running: AtomicBool,
    teardown_requested: AtomicBool,
}

struct ExecutorState {
    connection: Option<Arc<MutationConnection>>,
    descriptors: DescriptorCache,
    snapshots: SnapshotCache,
    tabs: HashMap<String, AnalysisSlot>,
    active: Option<ActiveRequest>,
    cancel_pending: Option<u64>,
    structure_generation: u64,
    last_used: Instant,
    closed: bool,
}

#[derive(Default)]
struct AnalysisSlot {
    latest_request_id: u64,
    queued: Option<AnalysisJob>,
}

struct AnalysisJob {
    tab_id: String,
    request_id: u64,
    enqueued_at: Instant,
    payload: AnalyzeResultSetPayload,
    generation: u64,
    virtual_keys: VirtualKeyLookup,
    reply: oneshot::Sender<Result<AnalyzeResultSetResult, ResultMutationError>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveKind {
    Analysis,
    Apply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    None,
    Supersede,
    Cancel,
    Closing,
}

struct ActiveRequest {
    tab_id: String,
    request_id: u64,
    kind: ActiveKind,
    interrupt: Interrupt,
    apply_phase: Option<ApplyPhase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyPhase {
    Preparing,
    CommitAdmitted,
}

impl ActiveRequest {
    fn new(tab_id: &str, request_id: u64, kind: ActiveKind) -> Self {
        Self {
            tab_id: tab_id.into(),
            request_id,
            kind,
            interrupt: Interrupt::None,
            apply_phase: (kind == ActiveKind::Apply).then_some(ApplyPhase::Preparing),
        }
    }

    fn commit_admitted(&self) -> bool {
        self.apply_phase == Some(ApplyPhase::CommitAdmitted)
    }
}

#[derive(Debug, Clone)]
struct AnalysisSnapshot {
    tab_id: String,
    descriptors: Vec<postgres::CatalogDescriptor>,
}

struct SnapshotCache {
    capacity: usize,
    order: VecDeque<u64>,
    values: HashMap<u64, AnalysisSnapshot>,
}

impl SnapshotCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            values: HashMap::with_capacity(capacity),
        }
    }
    fn insert(&mut self, snapshot: AnalysisSnapshot) -> u64 {
        let id = NEXT_ANALYSIS_ID.fetch_add(1, Ordering::Relaxed);
        if self.values.len() == self.capacity {
            if let Some(expired) = self.order.pop_front() {
                self.values.remove(&expired);
            }
        }
        self.order.push_back(id);
        self.values.insert(id, snapshot);
        id
    }
    fn get(&mut self, id: u64) -> Option<AnalysisSnapshot> {
        let snapshot = self.values.get(&id)?.clone();
        self.order.retain(|cached| *cached != id);
        self.order.push_back(id);
        Some(snapshot)
    }
    fn clear(&mut self) {
        self.order.clear();
        self.values.clear();
    }
}

fn new_executor(spec: ResolvedPostgresConnectSpec) -> Arc<Executor> {
    Arc::new(Executor {
        spec,
        state: Mutex::new(ExecutorState {
            connection: None,
            descriptors: HashMap::new(),
            snapshots: SnapshotCache::new(ANALYSIS_CACHE_CAPACITY),
            tabs: HashMap::new(),
            active: None,
            cancel_pending: None,
            structure_generation: 0,
            last_used: Instant::now(),
            closed: false,
        }),
        notify: Notify::new(),
        worker_running: AtomicBool::new(false),
        teardown_requested: AtomicBool::new(false),
    })
}

fn start_executor_worker(executor: &Arc<Executor>) {
    executor.worker_running.store(true, Ordering::Release);
    let worker = executor.clone();
    tokio::spawn(async move { run_analysis_worker(worker).await });
}

#[cfg(test)]
fn spawn_executor(spec: ResolvedPostgresConnectSpec) -> Arc<Executor> {
    let executor = new_executor(spec);
    start_executor_worker(&executor);
    executor
}

fn enqueue_analysis(state: &mut ExecutorState, job: AnalysisJob) -> Option<CancelRequest> {
    let tab_id = job.tab_id.clone();
    let slot = state.tabs.entry(tab_id.clone()).or_default();
    if job.request_id < slot.latest_request_id {
        let _ = job.reply.send(Err(ResultMutationError::Superseded));
        return None;
    }
    slot.latest_request_id = job.request_id;
    if let Some(queued) = slot.queued.take() {
        let _ = queued.reply.send(Err(ResultMutationError::Superseded));
    }
    slot.queued = Some(job);
    if let Some(active) = state.active.as_mut() {
        if active.kind == ActiveKind::Analysis && active.tab_id == tab_id {
            active.interrupt = Interrupt::Supersede;
            return queue_cancel(state);
        }
    }
    None
}

fn take_queued(state: &mut ExecutorState, tab_id: &str, request_id: u64) -> Option<AnalysisJob> {
    let slot = state.tabs.get_mut(tab_id)?;
    slot.queued
        .as_ref()
        .is_some_and(|job| job.request_id == request_id)
        .then(|| slot.queued.take())
        .flatten()
}

fn cancel_tab_state(state: &mut ExecutorState, tab_id: &str) -> (bool, Option<CancelRequest>) {
    let mut requested = false;
    if let Some(slot) = state.tabs.get_mut(tab_id) {
        if let Some(job) = slot.queued.take() {
            let _ = job.reply.send(Err(ResultMutationError::Cancelled));
            requested = true;
        }
    }
    let can_interrupt = state
        .active
        .as_ref()
        .is_some_and(|active| active.tab_id == tab_id && !active.commit_admitted());
    if can_interrupt {
        state.active.as_mut().unwrap().interrupt = Interrupt::Cancel;
        requested = true;
    }
    (
        requested,
        can_interrupt.then(|| queue_cancel(state)).flatten(),
    )
}

struct CancelRequest {
    token: tokio_postgres::CancelToken,
    tls: bool,
    request_id: u64,
}

fn queue_cancel(state: &mut ExecutorState) -> Option<CancelRequest> {
    if state.cancel_pending.is_some() {
        return None;
    }
    let active = state.active.as_ref()?;
    let connection = state.connection.as_ref()?;
    let request_id = active.request_id;
    let cancel = CancelRequest {
        token: connection.inner.cancel.clone(),
        tls: connection.inner.tls,
        request_id,
    };
    state.cancel_pending = Some(request_id);
    Some(cancel)
}

async fn perform_cancel(executor: &Executor, cancel: Option<CancelRequest>) {
    let Some(cancel) = cancel else {
        return;
    };
    let _ = crate::postgres::dedicated::cancel(cancel.token, cancel.tls).await;
    let mut state = executor.state.lock().await;
    if state.cancel_pending == Some(cancel.request_id) {
        state.cancel_pending = None;
    }
    drop(state);
    executor.notify.notify_waiters();
}

async fn run_analysis_worker(executor: Arc<Executor>) {
    loop {
        let notified = executor.notify.notified();
        tokio::pin!(notified);
        match next_analysis(&executor).await {
            Some(job) => execute_analysis(&executor, job).await,
            None => {
                if executor.state.lock().await.closed {
                    break;
                }
                notified.await;
            }
        }
    }
    executor.worker_running.store(false, Ordering::Release);
    executor.notify.notify_waiters();
}

async fn next_analysis(executor: &Executor) -> Option<AnalysisJob> {
    let mut state = executor.state.lock().await;
    if state.closed || state.active.is_some() || state.cancel_pending.is_some() {
        return None;
    }
    let mut expired_tabs = Vec::new();
    let mut next: Option<(String, Instant)> = None;
    for (tab_id, slot) in &state.tabs {
        let Some(job) = &slot.queued else {
            continue;
        };
        if job.enqueued_at.elapsed() >= QUEUE_WAIT {
            expired_tabs.push(tab_id.clone());
        } else if next
            .as_ref()
            .is_none_or(|(_, queued_at)| job.enqueued_at < *queued_at)
        {
            next = Some((tab_id.clone(), job.enqueued_at));
        }
    }
    let expired = expired_tabs
        .into_iter()
        .filter_map(|tab_id| state.tabs.get_mut(&tab_id)?.queued.take())
        .collect::<Vec<_>>();
    let job = next.and_then(|(tab_id, _)| state.tabs.get_mut(&tab_id)?.queued.take());
    if let Some(job) = &job {
        state.active = Some(ActiveRequest::new(
            &job.tab_id,
            job.request_id,
            ActiveKind::Analysis,
        ));
        state.last_used = Instant::now();
    }
    drop(state);
    for job in expired {
        let _ = job.reply.send(Err(ResultMutationError::Timeout {
            operation: "queueWait".into(),
        }));
    }
    job
}

async fn execute_analysis(executor: &Arc<Executor>, job: AnalysisJob) {
    let result = run_analysis(executor, &job).await;
    let result = finish_active(executor, result).await;
    let _ = job.reply.send(result);
    executor.notify.notify_waiters();
}

struct ApplyCallerGuard {
    dropped: Option<oneshot::Sender<()>>,
}

impl ApplyCallerGuard {
    fn new(dropped: oneshot::Sender<()>) -> Self {
        Self {
            dropped: Some(dropped),
        }
    }

    fn complete(&mut self) {
        self.dropped.take();
    }
}

impl Drop for ApplyCallerGuard {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}

async fn run_owned_apply(
    executor: Arc<Executor>,
    payload: ApplyResultMutationsPayload,
    mut caller_dropped: oneshot::Receiver<()>,
    reply: oneshot::Sender<Result<ApplyResult, ResultMutationError>>,
) {
    let apply = run_apply(&executor, &payload);
    tokio::pin!(apply);
    let result = tokio::select! {
        result = &mut apply => result,
        _ = &mut caller_dropped => {
            let cancel = cancel_dropped_apply(&mut *executor.state.lock().await);
            perform_cancel(&executor, cancel).await;
            apply.await
        }
    };
    let result = finish_active(&executor, result).await;
    executor.notify.notify_waiters();
    let _ = reply.send(result);
}

fn cancel_dropped_apply(state: &mut ExecutorState) -> Option<CancelRequest> {
    let can_cancel = state
        .active
        .as_ref()
        .is_some_and(|active| active.kind == ActiveKind::Apply && !active.commit_admitted());
    if !can_cancel {
        return None;
    }
    state.active.as_mut().unwrap().interrupt = Interrupt::Cancel;
    queue_cancel(state)
}

async fn run_analysis(
    executor: &Arc<Executor>,
    job: &AnalysisJob,
) -> Result<AnalyzeResultSetResult, ResultMutationError> {
    check_interrupted(executor).await?;
    let connection = ensure_connection(executor).await?;
    check_interrupted(executor).await?;
    let mut descriptors = std::mem::take(&mut executor.state.lock().await.descriptors);
    let result = postgres::analyze(
        &connection.inner.client,
        &job.payload.connection_id,
        &job.payload.source,
        job.payload.refresh_structure,
        &mut descriptors,
        &job.virtual_keys,
    )
    .await;
    let mut state = executor.state.lock().await;
    if state.structure_generation != job.generation {
        return Err(ResultMutationError::Superseded);
    }
    if !state.closed {
        state.descriptors = descriptors;
    }
    match result {
        Ok(analysis) => {
            let analysis_id = state.snapshots.insert(AnalysisSnapshot {
                tab_id: job.tab_id.clone(),
                descriptors: analysis.descriptors,
            });
            Ok(AnalyzeResultSetResult {
                request_id: job.request_id,
                analysis_id,
                columns: analysis.columns,
                tables: analysis.tables,
                statement: AnalysisStatement::Analyzed,
            })
        }
        Err(ResultMutationError::NotAnalyzable { reason }) => {
            let analysis_id = state.snapshots.insert(AnalysisSnapshot {
                tab_id: job.tab_id.clone(),
                descriptors: Vec::new(),
            });
            Ok(AnalyzeResultSetResult {
                request_id: job.request_id,
                analysis_id,
                columns: Vec::new(),
                tables: Vec::new(),
                statement: AnalysisStatement::NotAnalyzable { reason },
            })
        }
        Err(error) => Err(error),
    }
}

async fn run_apply(
    executor: &Arc<Executor>,
    payload: &ApplyResultMutationsPayload,
) -> Result<ApplyResult, ResultMutationError> {
    check_interrupted(executor).await?;
    let connection = ensure_connection(executor).await?;
    check_interrupted(executor).await?;
    let snapshot = {
        let mut state = executor.state.lock().await;
        state
            .snapshots
            .get(payload.analysis_id)
            .filter(|snapshot| snapshot.tab_id == payload.tab_id)
            .ok_or(ResultMutationError::AnalysisExpired)?
    };
    check_interrupted(executor).await?;
    let control_executor = executor.clone();
    let check: postgres::ApplyCheck = Arc::new(move |checkpoint| {
        let executor = control_executor.clone();
        Box::pin(async move {
            let mut state = executor.state.lock().await;
            let active = state
                .active
                .as_ref()
                .ok_or(ResultMutationError::ConnectionClosing)?;
            match active.interrupt {
                Interrupt::None => {
                    if checkpoint == postgres::ApplyCheckpoint::AdmitCommit {
                        admit_commit(&executor, &mut state)?;
                    }
                    Ok(())
                }
                Interrupt::Supersede => Err(ResultMutationError::Superseded),
                Interrupt::Cancel => Err(ResultMutationError::Cancelled),
                Interrupt::Closing => Err(ResultMutationError::ConnectionClosing),
            }
        })
    });
    let commit_connection = connection.clone();
    let commit: postgres::ApplyCommit = Arc::new(move || {
        let connection = commit_connection.clone();
        Box::pin(async move {
            match connection.inner.client.batch_execute("COMMIT").await {
                Ok(()) => postgres::CommitOutcome::Committed,
                Err(error) => postgres::CommitOutcome::Failed(postgres::database_error(error)),
            }
        })
    });
    let execution = postgres::execute_apply(
        &connection.inner.client,
        &snapshot.descriptors,
        &payload.plan,
        &check,
        &commit,
    )
    .await;
    if !execution.healthy {
        let mut state = executor.state.lock().await;
        state.connection = None;
        state.descriptors.clear();
        state.snapshots.clear();
    }
    let result = execution.result;
    if result.as_ref().is_err_and(postgres::is_undefined_object) {
        let mut state = executor.state.lock().await;
        invalidate_structure_state(&mut state);
    }
    result
}

fn admit_commit(executor: &Executor, state: &mut ExecutorState) -> Result<(), ResultMutationError> {
    if executor.teardown_requested.load(Ordering::Acquire) {
        return Err(ResultMutationError::ConnectionClosing);
    }
    let active = state
        .active
        .as_mut()
        .ok_or(ResultMutationError::ConnectionClosing)?;
    match active.interrupt {
        Interrupt::None => {
            active.apply_phase = Some(ApplyPhase::CommitAdmitted);
            Ok(())
        }
        Interrupt::Supersede => Err(ResultMutationError::Superseded),
        Interrupt::Cancel => Err(ResultMutationError::Cancelled),
        Interrupt::Closing => Err(ResultMutationError::ConnectionClosing),
    }
}

async fn ensure_connection(
    executor: &Arc<Executor>,
) -> Result<Arc<MutationConnection>, ResultMutationError> {
    if let Some(connection) = executor.state.lock().await.connection.clone() {
        if !connection.inner.is_closed() {
            return Ok(connection);
        }
    }
    let connection = Arc::new(postgres::connect(&executor.spec).await?);
    let mut state = executor.state.lock().await;
    if state.closed {
        return Err(ResultMutationError::ConnectionClosing);
    }
    state.connection = Some(connection.clone());
    Ok(connection)
}

async fn check_interrupted(executor: &Executor) -> Result<(), ResultMutationError> {
    match executor
        .state
        .lock()
        .await
        .active
        .as_ref()
        .map(|active| active.interrupt)
    {
        Some(Interrupt::None) => Ok(()),
        Some(Interrupt::Supersede) => Err(ResultMutationError::Superseded),
        Some(Interrupt::Cancel) => Err(ResultMutationError::Cancelled),
        Some(Interrupt::Closing) | None => Err(ResultMutationError::ConnectionClosing),
    }
}

async fn finish_active<T>(
    executor: &Executor,
    result: Result<T, ResultMutationError>,
) -> Result<T, ResultMutationError> {
    let mut state = executor.state.lock().await;
    let active = state.active.take();
    let terminal = match active {
        Some(ActiveRequest {
            kind: ActiveKind::Analysis,
            interrupt: Interrupt::Supersede,
            ..
        }) => Err(ResultMutationError::Superseded),
        Some(ActiveRequest {
            kind: ActiveKind::Analysis,
            interrupt: Interrupt::Cancel,
            ..
        }) => Err(ResultMutationError::Cancelled),
        Some(ActiveRequest {
            kind: ActiveKind::Analysis,
            interrupt: Interrupt::Closing,
            ..
        }) => Err(ResultMutationError::ConnectionClosing),
        Some(ActiveRequest {
            kind: ActiveKind::Apply,
            interrupt: Interrupt::Closing,
            ..
        }) => Err(ResultMutationError::ConnectionClosing),
        Some(ActiveRequest {
            kind: ActiveKind::Apply,
            interrupt: Interrupt::Cancel,
            ..
        }) if result.is_err() => Err(ResultMutationError::Cancelled),
        _ => result,
    };
    if terminal.as_ref().is_err_and(postgres::is_dead_socket)
        || state
            .connection
            .as_ref()
            .is_some_and(|connection| connection.inner.is_closed())
    {
        state.connection = None;
        state.descriptors.clear();
        state.snapshots.clear();
    }
    state.last_used = Instant::now();
    terminal
}

async fn close_executor(executor: &Executor, fence: bool) {
    let preparation = prepare_executor_close(executor, fence).await;
    finish_executor_close(executor, preparation).await;
}

struct ClosePreparation {
    queued: QueuedClose,
    cancel: CloseCancel,
    has_active: bool,
}

async fn prepare_executor_close(executor: &Executor, fence: bool) -> ClosePreparation {
    let mut state = executor.state.lock().await;
    if fence {
        // This lock is also the apply COMMIT admission lock. Whichever
        // transition wins defines whether teardown cancels or waits.
        executor.teardown_requested.store(true, Ordering::Release);
    }
    let (queued, cancel, has_active) = prepare_close(&mut state, fence);
    ClosePreparation {
        queued,
        cancel,
        has_active,
    }
}

async fn finish_executor_close(executor: &Executor, preparation: ClosePreparation) {
    for (job, error) in preparation.queued {
        let _ = job.reply.send(Err(error));
    }
    if let Some((token, tls)) = preparation.cancel {
        let _ = crate::postgres::dedicated::cancel(token, tls).await;
    }
    executor.notify.notify_waiters();
    if preparation.has_active {
        loop {
            let notified = executor.notify.notified();
            tokio::pin!(notified);
            if executor.state.lock().await.active.is_none() {
                break;
            }
            notified.await;
        }
    }
    let mut state = executor.state.lock().await;
    state.connection = None;
    state.descriptors.clear();
    state.snapshots.clear();
}

type QueuedClose = Vec<(AnalysisJob, ResultMutationError)>;
type CloseCancel = Option<(tokio_postgres::CancelToken, bool)>;

fn prepare_close(state: &mut ExecutorState, fence: bool) -> (QueuedClose, CloseCancel, bool) {
    state.closed = true;
    let error = if fence {
        ResultMutationError::ConnectionClosing
    } else {
        ResultMutationError::ConnectionLost
    };
    let queued = state
        .tabs
        .values_mut()
        .filter_map(|slot| slot.queued.take())
        .map(|job| (job, error.clone()))
        .collect::<Vec<_>>();
    if let Some(active) = state
        .active
        .as_mut()
        .filter(|active| !active.commit_admitted())
    {
        active.interrupt = if fence {
            Interrupt::Closing
        } else {
            Interrupt::Cancel
        };
    }
    let cancel = state
        .active
        .as_ref()
        .filter(|active| !active.commit_admitted())
        .and(state.connection.as_ref())
        .map(|connection| (connection.inner.cancel.clone(), connection.inner.tls));
    (queued, cancel, state.active.is_some())
}

fn is_idle(state: &ExecutorState) -> bool {
    !state.closed
        && state.active.is_none()
        && state.tabs.values().all(|slot| slot.queued.is_none())
        && state.last_used.elapsed() >= IDLE_TIMEOUT
}

fn check_admission(state: &ManagerState) -> Result<(), ResultMutationError> {
    if state.global_closing {
        return Err(ResultMutationError::ConnectionClosing);
    }
    if state.executors.len() + state.closing.len() >= MAX_EXECUTORS {
        Err(ResultMutationError::Timeout {
            operation: "admission".into(),
        })
    } else {
        Ok(())
    }
}

fn invalidate_structure_state(state: &mut ExecutorState) {
    state.structure_generation = state.structure_generation.saturating_add(1);
    state.snapshots.clear();
}

fn begin_apply(
    state: &mut ExecutorState,
    payload: &ApplyResultMutationsPayload,
) -> Result<(), ResultMutationError> {
    if state.closed {
        return Err(ResultMutationError::ConnectionClosing);
    }
    if state.active.is_some() || state.cancel_pending.is_some() {
        return Err(ResultMutationError::Busy);
    }
    if !state
        .snapshots
        .get(payload.analysis_id)
        .is_some_and(|snapshot| snapshot.tab_id == payload.tab_id)
    {
        return Err(ResultMutationError::AnalysisExpired);
    }
    state.active = Some(ActiveRequest::new(
        &payload.tab_id,
        payload.request_id,
        ActiveKind::Apply,
    ));
    state.last_used = Instant::now();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ExecutorState {
        ExecutorState {
            connection: None,
            descriptors: HashMap::new(),
            snapshots: SnapshotCache::new(ANALYSIS_CACHE_CAPACITY),
            tabs: HashMap::new(),
            active: None,
            cancel_pending: None,
            structure_generation: 0,
            last_used: Instant::now(),
            closed: false,
        }
    }

    fn lookup() -> VirtualKeyLookup {
        Arc::new(|_, _, _| Box::pin(async { Ok(None) }))
    }

    fn spec(connection_id: &str) -> ResolvedPostgresConnectSpec {
        ResolvedPostgresConnectSpec {
            connection_id: connection_id.into(),
            host: "127.0.0.1".into(),
            port: 5432,
            database: "dbunk".into(),
            user: "dbunk".into(),
            password: "dbunk".into(),
            tls_prefer: false,
            connect_timeout: None,
            driver_options: crate::PgDriverOptions::default(),
            safety_policy: Default::default(),
        }
    }

    fn apply_payload(tab_id: &str, analysis_id: u64) -> ApplyResultMutationsPayload {
        ApplyResultMutationsPayload {
            connection_id: "c".into(),
            tab_id: tab_id.into(),
            request_id: 10,
            analysis_id,
            confirmed: false,
            plan: MutationPlan {
                operations: Vec::new(),
            },
        }
    }

    fn job(
        tab_id: &str,
        request_id: u64,
    ) -> (
        AnalysisJob,
        oneshot::Receiver<Result<AnalyzeResultSetResult, ResultMutationError>>,
    ) {
        let (reply, result) = oneshot::channel();
        (
            AnalysisJob {
                tab_id: tab_id.into(),
                request_id,
                enqueued_at: Instant::now(),
                payload: AnalyzeResultSetPayload {
                    connection_id: "c".into(),
                    tab_id: tab_id.into(),
                    request_id,
                    source: AnalyzeSource::Relation {
                        schema: "public".into(),
                        table: "users".into(),
                    },
                    refresh_structure: false,
                },
                generation: 0,
                virtual_keys: lookup(),
                reply,
            },
            result,
        )
    }

    #[test]
    fn start_monitor_is_callable_outside_a_tokio_runtime() {
        // Regression: setup() calls this on the main thread with no Tokio
        // runtime context; a bare tokio::spawn panics with "no reactor
        // running" and aborts app startup.
        ResultMutationManager::new().start_monitor();
    }

    #[test]
    fn contract_limits_are_fixed() {
        assert_eq!(
            (
                MAX_EXECUTORS,
                IDLE_TIMEOUT,
                QUEUE_WAIT,
                CLOSE_TIMEOUT,
                ANALYSIS_CACHE_CAPACITY
            ),
            (
                8,
                Duration::from_secs(300),
                Duration::from_secs(10),
                Duration::from_secs(3),
                16
            )
        );
    }

    #[tokio::test]
    async fn newer_analysis_supersedes_queued_request() {
        let mut state = state();
        let (first, first_result) = job("tab", 1);
        enqueue_analysis(&mut state, first);
        let (second, _) = job("tab", 2);
        enqueue_analysis(&mut state, second);
        assert!(matches!(
            first_result.await,
            Ok(Err(ResultMutationError::Superseded))
        ));
        assert_eq!(state.tabs["tab"].queued.as_ref().unwrap().request_id, 2);
    }

    #[test]
    fn analysis_supersession_never_interrupts_apply() {
        let mut state = state();
        state.active = Some(ActiveRequest::new("tab", 1, ActiveKind::Analysis));
        let (next, _) = job("tab", 2);
        enqueue_analysis(&mut state, next);
        assert_eq!(
            state.active.as_ref().unwrap().interrupt,
            Interrupt::Supersede
        );
        state.active = Some(ActiveRequest::new("tab", 3, ActiveKind::Apply));
        let (next, _) = job("tab", 4);
        enqueue_analysis(&mut state, next);
        assert_eq!(state.active.as_ref().unwrap().interrupt, Interrupt::None);
    }

    #[tokio::test]
    async fn cancel_bookkeeping_is_scoped_to_tab() {
        let mut state = state();
        let (queued, result) = job("tab", 1);
        enqueue_analysis(&mut state, queued);
        assert!(cancel_tab_state(&mut state, "tab").0);
        assert!(matches!(
            result.await,
            Ok(Err(ResultMutationError::Cancelled))
        ));
        state.active = Some(ActiveRequest::new("tab", 2, ActiveKind::Apply));
        assert!(!cancel_tab_state(&mut state, "other").0);
        assert!(cancel_tab_state(&mut state, "tab").0);
        assert_eq!(state.active.as_ref().unwrap().interrupt, Interrupt::Cancel);
    }

    #[test]
    fn snapshot_cache_evicts_oldest_at_sixteen() {
        let mut cache = SnapshotCache::new(ANALYSIS_CACHE_CAPACITY);
        let ids = (0..=ANALYSIS_CACHE_CAPACITY)
            .map(|_| {
                cache.insert(AnalysisSnapshot {
                    tab_id: "tab".into(),
                    descriptors: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        assert!(!cache.values.contains_key(&ids[0]));
        assert!(cache.values.contains_key(&ids[1]));
        assert!(cache.values.contains_key(&ids[16]));
        assert_eq!(cache.values.len(), ANALYSIS_CACHE_CAPACITY);
    }

    #[test]
    fn snapshot_lookup_updates_lru_recency_and_preserves_tab_owner() {
        let mut cache = SnapshotCache::new(2);
        let first = cache.insert(AnalysisSnapshot {
            tab_id: "first-tab".into(),
            descriptors: Vec::new(),
        });
        let second = cache.insert(AnalysisSnapshot {
            tab_id: "second-tab".into(),
            descriptors: Vec::new(),
        });
        assert_eq!(cache.get(first).unwrap().tab_id, "first-tab");
        let third = cache.insert(AnalysisSnapshot {
            tab_id: "third-tab".into(),
            descriptors: Vec::new(),
        });
        assert!(!cache.values.contains_key(&second));
        assert!(cache.values.contains_key(&first));
        assert!(cache.values.contains_key(&third));
    }

    #[test]
    fn structure_invalidation_expires_snapshots_and_advances_generation() {
        let mut state = state();
        state.snapshots.insert(AnalysisSnapshot {
            tab_id: "tab".into(),
            descriptors: Vec::new(),
        });
        invalidate_structure_state(&mut state);
        assert_eq!(state.structure_generation, 1);
        assert!(state.snapshots.values.is_empty());
    }

    #[test]
    fn idle_requires_expired_and_quiet_executor() {
        let mut state = state();
        state.last_used = Instant::now() - IDLE_TIMEOUT;
        assert!(is_idle(&state));
        state.active = Some(ActiveRequest::new("tab", 1, ActiveKind::Apply));
        assert!(!is_idle(&state));
    }

    #[test]
    fn admission_rejects_the_ninth_executor() {
        let mut manager = ManagerState::default();
        for index in 0..MAX_EXECUTORS {
            manager.executors.insert(
                format!("c{index}"),
                new_executor(spec(&format!("c{index}"))),
            );
        }
        assert!(
            matches!(check_admission(&manager), Err(ResultMutationError::Timeout { operation }) if operation == "admission")
        );
    }

    #[test]
    fn apply_is_exclusive_and_snapshot_is_tab_scoped() {
        let mut state = state();
        let analysis_id = state.snapshots.insert(AnalysisSnapshot {
            tab_id: "owner".into(),
            descriptors: Vec::new(),
        });
        assert_eq!(
            begin_apply(&mut state, &apply_payload("other", analysis_id)),
            Err(ResultMutationError::AnalysisExpired)
        );
        assert!(begin_apply(&mut state, &apply_payload("owner", analysis_id)).is_ok());
        assert_eq!(
            begin_apply(&mut state, &apply_payload("owner", analysis_id)),
            Err(ResultMutationError::Busy)
        );
    }

    #[test]
    fn duplicate_cancel_is_not_scheduled_for_the_same_active_request() {
        let mut state = state();
        state.active = Some(ActiveRequest::new("tab", 7, ActiveKind::Analysis));
        state.cancel_pending = Some(7);
        assert!(queue_cancel(&mut state).is_none());
        assert_eq!(state.cancel_pending, Some(7));
    }

    #[test]
    fn teardown_before_commit_admission_cancels_apply() {
        let mut state = state();
        state.active = Some(ActiveRequest::new("tab", 7, ActiveKind::Apply));

        let (_, cancel, has_active) = prepare_close(&mut state, true);

        assert!(has_active);
        assert!(state.closed);
        let active = state.active.as_ref().unwrap();
        assert!(!active.commit_admitted());
        assert_eq!(active.interrupt, Interrupt::Closing);
        assert!(cancel.is_none());
    }

    #[test]
    fn commit_admission_linearizes_teardown_outcomes() {
        let mut executor_state = state();
        executor_state.active = Some(ActiveRequest::new("tab", 7, ActiveKind::Apply));
        let executor = Executor {
            spec: spec("c"),
            state: Mutex::new(state()),
            notify: Notify::new(),
            worker_running: AtomicBool::new(false),
            teardown_requested: AtomicBool::new(true),
        };

        assert_eq!(
            admit_commit(&executor, &mut executor_state),
            Err(ResultMutationError::ConnectionClosing)
        );

        executor.teardown_requested.store(false, Ordering::Release);
        admit_commit(&executor, &mut executor_state).unwrap();
        let (_, cancel, has_active) = prepare_close(&mut executor_state, true);
        assert!(has_active);
        assert!(cancel.is_none());
        let active = executor_state.active.as_ref().unwrap();
        assert!(active.commit_admitted());
        assert_eq!(active.interrupt, Interrupt::None);
    }

    #[tokio::test]
    async fn admitted_teardown_is_bounded_and_fence_releases_only_at_terminal() {
        let manager = ResultMutationManager::new();
        let connection_id = "blocked-commit";
        let executor = spawn_executor(spec(connection_id));
        {
            let mut state = executor.state.lock().await;
            state.active = Some(ActiveRequest::new("tab", 7, ActiveKind::Apply));
            admit_commit(&executor, &mut state).unwrap();
        }
        manager
            .inner
            .lock()
            .await
            .executors
            .insert(connection_id.into(), executor.clone());

        let started = Instant::now();
        manager
            .begin_connection_teardown_with_timeout(connection_id, Duration::from_millis(20))
            .await;
        assert!(started.elapsed() < Duration::from_secs(1));
        manager.end_connection_teardown(connection_id).await;
        {
            let state = manager.inner.lock().await;
            assert!(state.closing.contains(connection_id));
            assert!(state.lingering_closes.contains(connection_id));
        }

        assert_eq!(finish_active(&executor, Ok(())).await, Ok(()));
        executor.notify.notify_waiters();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let notified = manager.changed.notified();
                tokio::pin!(notified);
                if !manager.inner.lock().await.closing.contains(connection_id) {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("detached close released the fence after terminal success");
    }

    #[tokio::test]
    async fn terminal_close_does_not_release_connection_fence_before_lifecycle_end() {
        let manager = ResultMutationManager::new();
        let connection_id = "lifecycle-work";
        let executor = spawn_executor(spec(connection_id));
        {
            let mut state = executor.state.lock().await;
            state.active = Some(ActiveRequest::new("tab", 7, ActiveKind::Apply));
            admit_commit(&executor, &mut state).unwrap();
        }
        manager
            .inner
            .lock()
            .await
            .executors
            .insert(connection_id.into(), executor.clone());
        manager
            .begin_connection_teardown_with_timeout(connection_id, Duration::from_millis(20))
            .await;

        assert_eq!(finish_active(&executor, Ok(())).await, Ok(()));
        executor.notify.notify_waiters();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let notified = manager.changed.notified();
                tokio::pin!(notified);
                if !manager
                    .inner
                    .lock()
                    .await
                    .lingering_closes
                    .contains(connection_id)
                {
                    break;
                }
                notified.await;
            }
        })
        .await
        .expect("detached cleanup reached terminal state");
        assert!(manager.inner.lock().await.closing.contains(connection_id));

        manager.end_connection_teardown(connection_id).await;
        assert!(!manager.inner.lock().await.closing.contains(connection_id));
    }

    #[tokio::test]
    async fn global_fence_stays_closed_until_detached_closes_finish() {
        let manager = ResultMutationManager::new();
        {
            let mut state = manager.inner.lock().await;
            state.global_closing = true;
            state.global_close_pending = 2;
        }
        manager.end_global_teardown().await;
        assert!(manager.inner.lock().await.global_closing);
        manager.complete_global_close().await;
        assert!(manager.inner.lock().await.global_closing);
        manager.complete_global_close().await;
        let state = manager.inner.lock().await;
        assert!(!state.global_closing);
        assert_eq!(state.global_close_pending, 0);
        assert!(!state.global_end_requested);
    }

    #[tokio::test]
    async fn cancelled_executor_installation_cannot_spawn_an_unowned_worker() {
        let manager = ResultMutationManager::new();
        let manager_lock = manager.inner.lock().await;
        let opening_manager = manager.clone();
        let opening = tokio::spawn(async move {
            opening_manager
                .executor_for(spec("cancelled-install"), "cancelled-install")
                .await
        });
        tokio::task::yield_now().await;
        opening.abort();
        let cancellation = opening.await;
        assert!(matches!(cancellation, Err(error) if error.is_cancelled()));
        drop(manager_lock);

        let state = manager.inner.lock().await;
        assert!(state.executors.is_empty());
        assert!(check_admission(&state).is_ok());
    }

    #[tokio::test]
    async fn stale_apply_ids_do_not_create_or_count_executors() {
        let manager = ResultMutationManager::new();
        for index in 0..MAX_EXECUTORS {
            let mut payload = apply_payload("tab", index as u64 + 1);
            payload.connection_id = format!("stale-{index}");
            assert_eq!(
                manager.apply(spec(&payload.connection_id), payload).await,
                Err(ResultMutationError::AnalysisExpired)
            );
        }
        let state = manager.inner.lock().await;
        assert!(state.executors.is_empty());
        assert!(check_admission(&state).is_ok());
    }

    #[tokio::test]
    async fn policy_refusal_leaves_the_apply_executor_idle() {
        let manager = ResultMutationManager::new();
        let executor = new_executor(spec("c"));
        let analysis_id = executor
            .state
            .lock()
            .await
            .snapshots
            .insert(AnalysisSnapshot {
                tab_id: "tab".into(),
                descriptors: Vec::new(),
            });
        manager
            .inner
            .lock()
            .await
            .executors
            .insert("c".into(), executor.clone());

        let mut guarded_spec = spec("c");
        guarded_spec.safety_policy =
            crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
                environment: crate::Environment::Production,
                safe_mode: crate::SafeMode::Inherit,
                read_only: false,
            });
        let result = manager
            .apply(guarded_spec, apply_payload("tab", analysis_id))
            .await;
        assert!(matches!(
            result,
            Err(ResultMutationError::PolicyNeedsConfirmation { .. })
        ));
        let state = executor.state.lock().await;
        assert!(state.active.is_none());
        assert!(state.cancel_pending.is_none());
    }

    #[tokio::test]
    async fn close_idle_removes_executor_stops_worker_and_releases_capacity() {
        let manager = ResultMutationManager::new();
        let executor = spawn_executor(spec("idle"));
        executor.state.lock().await.last_used = Instant::now() - IDLE_TIMEOUT;
        manager
            .inner
            .lock()
            .await
            .executors
            .insert("idle".into(), executor.clone());

        manager.close_idle().await;

        let notified = executor.notify.notified();
        tokio::pin!(notified);
        if executor.worker_running.load(Ordering::Acquire) {
            tokio::time::timeout(Duration::from_secs(1), &mut notified)
                .await
                .expect("analysis worker stopped");
        }
        assert!(!executor.worker_running.load(Ordering::Acquire));
        let executor_state = executor.state.lock().await;
        assert!(executor_state.closed);
        assert!(executor_state.connection.is_none());
        drop(executor_state);
        let manager_state = manager.inner.lock().await;
        assert!(!manager_state.executors.contains_key("idle"));
        assert!(!manager_state.closing.contains("idle"));
        assert!(check_admission(&manager_state).is_ok());
    }

    #[tokio::test]
    async fn dropped_apply_waiter_cancels_and_finalizes_owned_state() {
        let executor = spawn_executor(spec("dropped"));
        executor.state.lock().await.active = Some(ActiveRequest::new("tab", 7, ActiveKind::Apply));
        let (dropped, mut signal) = oneshot::channel();
        drop(ApplyCallerGuard::new(dropped));
        (&mut signal).await.expect("drop signal");

        let cancel = cancel_dropped_apply(&mut *executor.state.lock().await);
        assert!(cancel.is_none());
        assert_eq!(
            executor
                .state
                .lock()
                .await
                .active
                .as_ref()
                .unwrap()
                .interrupt,
            Interrupt::Cancel
        );
        assert_eq!(
            finish_active::<()>(&executor, Err(ResultMutationError::Cancelled)).await,
            Err(ResultMutationError::Cancelled)
        );
        assert!(executor.state.lock().await.active.is_none());
        close_executor(&executor, false).await;
    }
}
