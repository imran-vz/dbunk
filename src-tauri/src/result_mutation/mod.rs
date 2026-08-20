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

#[derive(Default)]
struct ManagerState {
    executors: HashMap<String, Arc<Executor>>,
    opening: HashSet<String>,
    closing: HashSet<String>,
    lingering_closes: HashSet<String>,
    global_closing: bool,
    global_close_pending: usize,
    global_end_requested: bool,
}

#[derive(Clone)]
pub(crate) struct ResultMutationManager {
    inner: Arc<Mutex<ManagerState>>,
}

impl ResultMutationManager {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerState::default())),
        }
    }

    pub(crate) fn start_monitor(&self) {
        let manager = self.clone();
        tokio::spawn(async move {
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
    ) -> Result<ApplyResult, ResultMutationError> {
        let executor = self.executor_for(spec, &payload.connection_id).await?;
        {
            let mut state = executor.state.lock().await;
            begin_apply(&mut state, &payload)?;
        }
        let result = run_apply(&executor, &payload).await;
        let result = finish_active(&executor, result).await;
        executor.notify.notify_waiters();
        result
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
        let executor = {
            let mut state = self.inner.lock().await;
            state.closing.insert(connection_id.into());
            state.executors.remove(connection_id)
        };
        if let Some(executor) = executor {
            executor.teardown_requested.store(true, Ordering::Release);
            if tokio::time::timeout(CLOSE_TIMEOUT, close_executor(&executor, true))
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
                    close_executor(&executor, true).await;
                    let mut state = manager.inner.lock().await;
                    state.lingering_closes.remove(&connection_id);
                    state.closing.remove(&connection_id);
                });
            }
        }
    }

    pub(crate) async fn end_connection_teardown(&self, connection_id: &str) {
        let mut state = self.inner.lock().await;
        if !state.lingering_closes.contains(connection_id) {
            state.closing.remove(connection_id);
        }
    }

    pub(crate) async fn close_all(&self) {
        let (executors, track_global) = {
            let mut state = self.inner.lock().await;
            let executors = state
                .executors
                .drain()
                .map(|(_, executor)| executor)
                .collect::<Vec<_>>();
            for executor in &executors {
                executor.teardown_requested.store(true, Ordering::Release);
            }
            let track_global = state.global_closing;
            if track_global {
                state.global_close_pending =
                    state.global_close_pending.saturating_add(executors.len());
            }
            (executors, track_global)
        };
        let mut tasks = executors
            .into_iter()
            .map(|executor| {
                let manager = self.clone();
                tokio::spawn(async move {
                    close_executor(&executor, true).await;
                    if track_global {
                        manager.complete_global_close().await;
                    }
                })
            })
            .collect::<Vec<_>>();
        let _ = tokio::time::timeout(
            CLOSE_TIMEOUT,
            futures_util::future::join_all(tasks.iter_mut()),
        )
        .await;
    }

    pub(crate) async fn begin_global_teardown(&self) {
        let mut state = self.inner.lock().await;
        state.global_closing = true;
        state.global_end_requested = false;
        drop(state);
        self.close_all().await;
    }

    pub(crate) async fn end_global_teardown(&self) {
        let mut state = self.inner.lock().await;
        if state.global_close_pending == 0 {
            state.global_closing = false;
        } else {
            state.global_end_requested = true;
        }
    }

    async fn complete_global_close(&self) {
        let mut state = self.inner.lock().await;
        state.global_close_pending = state.global_close_pending.saturating_sub(1);
        if state.global_close_pending == 0 && state.global_end_requested {
            state.global_closing = false;
            state.global_end_requested = false;
        }
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
        loop {
            let should_open = {
                let mut state = self.inner.lock().await;
                if state.global_closing || state.closing.contains(connection_id) {
                    return Err(ResultMutationError::ConnectionClosing);
                }
                if let Some(executor) = state.executors.get(connection_id).cloned() {
                    return Ok(executor);
                }
                if state.opening.contains(connection_id) {
                    false
                } else {
                    check_admission(&state)?;
                    state.opening.insert(connection_id.into());
                    true
                }
            };
            if should_open {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let executor = spawn_executor(spec);
        self.install_spawned_executor(connection_id, executor).await
    }

    async fn install_spawned_executor(
        &self,
        connection_id: &str,
        executor: Arc<Executor>,
    ) -> Result<Arc<Executor>, ResultMutationError> {
        let mut state = self.inner.lock().await;
        state.opening.remove(connection_id);
        if state.global_closing || state.closing.contains(connection_id) {
            drop(state);
            close_executor(&executor, true).await;
            return Err(ResultMutationError::ConnectionClosing);
        }
        state
            .executors
            .insert(connection_id.into(), executor.clone());
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
            }
        }
    }
}

struct Executor {
    spec: ResolvedPostgresConnectSpec,
    state: Mutex<ExecutorState>,
    notify: Notify,
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
    commit_pending: bool,
}

impl ActiveRequest {
    fn new(tab_id: &str, request_id: u64, kind: ActiveKind) -> Self {
        Self {
            tab_id: tab_id.into(),
            request_id,
            kind,
            interrupt: Interrupt::None,
            commit_pending: false,
        }
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

fn spawn_executor(spec: ResolvedPostgresConnectSpec) -> Arc<Executor> {
    let executor = Arc::new(Executor {
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
        teardown_requested: AtomicBool::new(false),
    });
    let worker = executor.clone();
    tokio::spawn(async move { run_analysis_worker(worker).await });
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
        .is_some_and(|active| active.tab_id == tab_id);
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
        match next_analysis(&executor).await {
            Some(job) => execute_analysis(&executor, job).await,
            None => {
                if executor.state.lock().await.closed {
                    break;
                }
                executor.notify.notified().await;
            }
        }
    }
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
    let mut cache = std::mem::take(&mut executor.state.lock().await.descriptors);
    let first =
        postgres::refresh_for_apply(&connection.inner.client, &snapshot.descriptors, &mut cache)
            .await;
    let (descriptors, structurally_invalidated) = match first {
        Err(error) if postgres::is_undefined_object(&error) => {
            cache.clear();
            (
                postgres::refresh_for_apply(
                    &connection.inner.client,
                    &snapshot.descriptors,
                    &mut cache,
                )
                .await,
                true,
            )
        }
        Err(ResultMutationError::AnalysisExpired) => {
            (Err(ResultMutationError::AnalysisExpired), true)
        }
        result => (result, false),
    };
    {
        let mut state = executor.state.lock().await;
        if !state.closed {
            state.descriptors = cache;
        }
        if structurally_invalidated {
            invalidate_structure_state(&mut state);
        }
    }
    let descriptors = descriptors.map_err(|error| {
        if postgres::is_undefined_object(&error) {
            ResultMutationError::AnalysisExpired
        } else {
            error
        }
    })?;
    let statements = build_mutation_plan(&descriptors, &payload.plan)?;
    check_interrupted(executor).await?;
    let control_executor = executor.clone();
    let check: postgres::ApplyCheck = Arc::new(move |commit_pending| {
        let executor = control_executor.clone();
        Box::pin(async move {
            let mut state = executor.state.lock().await;
            let active = state
                .active
                .as_mut()
                .ok_or(ResultMutationError::ConnectionClosing)?;
            match active.interrupt {
                Interrupt::None => {
                    if commit_pending {
                        active.commit_pending = true;
                    }
                    Ok(())
                }
                Interrupt::Supersede => Err(ResultMutationError::Superseded),
                Interrupt::Cancel => Err(ResultMutationError::Cancelled),
                Interrupt::Closing => Err(ResultMutationError::ConnectionClosing),
            }
        })
    });
    let commit_executor = executor.clone();
    let commit_connection = connection.clone();
    let commit: postgres::ApplyCommit = Arc::new(move || {
        let executor = commit_executor.clone();
        let connection = commit_connection.clone();
        Box::pin(async move {
            // The atomic fence makes teardown win before COMMIT admission.
            // Once admitted, teardown marks the active request and sends a
            // protocol cancel while COMMIT is in flight.
            {
                let state = executor.state.lock().await;
                if let Some(error) = commit_rejection(&executor, &state) {
                    return postgres::CommitOutcome::Rejected(error);
                }
            }
            match connection.inner.client.batch_execute("COMMIT").await {
                Ok(()) if executor.teardown_requested.load(Ordering::Acquire) => {
                    postgres::CommitOutcome::Failed(ResultMutationError::ConnectionClosing)
                }
                Ok(()) => postgres::CommitOutcome::Committed,
                Err(error) => postgres::CommitOutcome::Failed(postgres::database_error(error)),
            }
        })
    });
    let execution =
        postgres::execute_apply(&connection.inner.client, &statements, &check, &commit).await;
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

fn commit_rejection(executor: &Executor, state: &ExecutorState) -> Option<ResultMutationError> {
    if executor.teardown_requested.load(Ordering::Acquire) {
        return Some(ResultMutationError::ConnectionClosing);
    }
    match state.active.as_ref().map(|active| active.interrupt) {
        Some(Interrupt::None) => None,
        Some(Interrupt::Supersede) => Some(ResultMutationError::Superseded),
        Some(Interrupt::Cancel) => Some(ResultMutationError::Cancelled),
        Some(Interrupt::Closing) | None => Some(ResultMutationError::ConnectionClosing),
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
    if fence {
        executor.teardown_requested.store(true, Ordering::Release);
    }
    let (queued, cancel, has_active) = {
        let mut state = executor.state.lock().await;
        prepare_close(&mut state, fence)
    };
    for (job, error) in queued {
        let _ = job.reply.send(Err(error));
    }
    if let Some((token, tls)) = cancel {
        let _ = crate::postgres::dedicated::cancel(token, tls).await;
    }
    executor.notify.notify_waiters();
    if has_active {
        loop {
            if executor.state.lock().await.active.is_none() {
                break;
            }
            executor.notify.notified().await;
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
    if let Some(active) = state.active.as_mut() {
        active.interrupt = if fence {
            Interrupt::Closing
        } else {
            Interrupt::Cancel
        };
    }
    let cancel = state
        .active
        .as_ref()
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
    if state.executors.len() + state.opening.len() + state.closing.len() >= MAX_EXECUTORS {
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
        }
    }

    fn apply_payload(tab_id: &str, analysis_id: u64) -> ApplyResultMutationsPayload {
        ApplyResultMutationsPayload {
            connection_id: "c".into(),
            tab_id: tab_id.into(),
            request_id: 10,
            analysis_id,
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
            manager.opening.insert(format!("c{index}"));
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
    fn teardown_revokes_apply_after_precommit_check() {
        let mut state = state();
        let mut active = ActiveRequest::new("tab", 7, ActiveKind::Apply);
        active.commit_pending = true;
        state.active = Some(active);

        let (_, _, has_active) = prepare_close(&mut state, true);

        assert!(has_active);
        assert!(state.closed);
        let active = state.active.as_ref().unwrap();
        assert!(active.commit_pending);
        assert_eq!(active.interrupt, Interrupt::Closing);
    }

    #[test]
    fn teardown_fence_wins_before_executor_state_close_is_installed() {
        let mut executor_state = state();
        let mut active = ActiveRequest::new("tab", 7, ActiveKind::Apply);
        active.commit_pending = true;
        executor_state.active = Some(active);
        let executor = Executor {
            spec: spec("c"),
            state: Mutex::new(state()),
            notify: Notify::new(),
            teardown_requested: AtomicBool::new(true),
        };

        assert_eq!(
            commit_rejection(&executor, &executor_state),
            Some(ResultMutationError::ConnectionClosing)
        );
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
    async fn spawned_executor_rejected_by_teardown_is_explicitly_closed() {
        let manager = ResultMutationManager::new();
        let connection_id = "opening-during-close";
        let executor = spawn_executor(spec(connection_id));
        {
            let mut state = manager.inner.lock().await;
            state.opening.insert(connection_id.into());
            state.closing.insert(connection_id.into());
        }

        assert!(matches!(
            manager
                .install_spawned_executor(connection_id, executor.clone())
                .await,
            Err(ResultMutationError::ConnectionClosing)
        ));
        let state = executor.state.lock().await;
        assert!(state.closed);
        assert!(state.connection.is_none());
    }
}
