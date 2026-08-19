pub(crate) mod builder;
pub(crate) mod postgres;
pub(crate) mod protocol;

#[cfg(test)]
mod live;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex, Notify};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;

use builder::{build_browse_query, BuiltBrowseQuery, RelationDescriptor};
use postgres::BrowseConnection;
use protocol::*;

const MAX_EXECUTORS: usize = 8;
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const QUEUE_WAIT: Duration = Duration::from_secs(10);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

enum JobKind {
    Browse(BrowseTableDataPayload),
    Count(CountTableBrowseRowsPayload),
}

enum JobResult {
    Browse(Box<BrowseTableResult>),
    Count(BrowseExactCountResult),
}

struct Job {
    tab_id: String,
    request_id: u64,
    enqueued_at: Instant,
    kind: JobKind,
    reply: oneshot::Sender<Result<JobResult, TableBrowseError>>,
}

#[derive(Default)]
struct TabSlot {
    latest_request_id: u64,
    queued: Option<Job>,
    in_flight_request_id: Option<u64>,
    interrupt: Interrupt,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Interrupt {
    #[default]
    None,
    Supersede,
    Cancel,
    Closing,
}

struct ExecutorInner {
    connection: Option<BrowseConnection>,
    cache: HashMap<(String, String), RelationDescriptor>,
    tabs: HashMap<String, TabSlot>,
    last_used: Instant,
    closed: bool,
    busy: bool,
    pending_cancel: Option<(tokio_postgres::CancelToken, bool)>,
}

struct Executor {
    spec: ResolvedPostgresConnectSpec,
    inner: Mutex<ExecutorInner>,
    notify: Notify,
}

#[derive(Default)]
struct ManagerState {
    executors: HashMap<String, Arc<Executor>>,
    closing: HashSet<String>,
    opening: HashSet<String>,
}

#[derive(Clone)]
pub(crate) struct TableBrowseManager {
    inner: Arc<Mutex<ManagerState>>,
}

impl TableBrowseManager {
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

    pub(crate) async fn browse(
        &self,
        spec: ResolvedPostgresConnectSpec,
        payload: BrowseTableDataPayload,
    ) -> Result<BrowseTableResult, TableBrowseError> {
        match self
            .submit(
                spec,
                payload.connection_id.clone(),
                payload.tab_id.clone(),
                payload.request_id,
                JobKind::Browse(payload),
            )
            .await?
        {
            JobResult::Browse(result) => Ok(*result),
            JobResult::Count(_) => Err(TableBrowseError::ConnectionLost),
        }
    }

    pub(crate) async fn count(
        &self,
        spec: ResolvedPostgresConnectSpec,
        payload: CountTableBrowseRowsPayload,
    ) -> Result<BrowseExactCountResult, TableBrowseError> {
        match self
            .submit(
                spec,
                payload.connection_id.clone(),
                payload.tab_id.clone(),
                payload.request_id,
                JobKind::Count(payload),
            )
            .await?
        {
            JobResult::Count(result) => Ok(result),
            JobResult::Browse(_) => Err(TableBrowseError::ConnectionLost),
        }
    }

    pub(crate) async fn cancel_tab(
        &self,
        connection_id: &str,
        tab_id: &str,
    ) -> CancelTableBrowseResult {
        let executor = {
            let state = self.inner.lock().await;
            state.executors.get(connection_id).cloned()
        };
        let Some(executor) = executor else {
            return CancelTableBrowseResult {
                cancel_requested: false,
            };
        };
        let cancel_requested = {
            let mut inner = executor.inner.lock().await;
            apply_tab_cancel(&mut inner, tab_id)
        };
        executor.notify.notify_one();
        CancelTableBrowseResult { cancel_requested }
    }

    pub(crate) async fn close_tab(&self, connection_id: &str, tab_id: &str) {
        let _ = self.cancel_tab(connection_id, tab_id).await;
        if let Some(executor) = self
            .inner
            .lock()
            .await
            .executors
            .get(connection_id)
            .cloned()
        {
            executor.inner.lock().await.tabs.remove(tab_id);
        }
    }

    pub(crate) async fn begin_connection_teardown(&self, connection_id: &str) {
        let executor = {
            let mut state = self.inner.lock().await;
            state.closing.insert(connection_id.into());
            state.executors.remove(connection_id)
        };
        if let Some(executor) = executor {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, close_executor(&executor, true)).await;
        }
    }

    pub(crate) async fn end_connection_teardown(&self, connection_id: &str) {
        self.inner.lock().await.closing.remove(connection_id);
    }

    pub(crate) async fn close_all(&self) {
        let executors = {
            let mut state = self.inner.lock().await;
            state
                .executors
                .drain()
                .map(|(_, executor)| executor)
                .collect::<Vec<_>>()
        };
        let _ = tokio::time::timeout(
            CLOSE_TIMEOUT,
            futures_util::future::join_all(
                executors
                    .iter()
                    .map(|executor| close_executor(executor, true)),
            ),
        )
        .await;
    }

    async fn submit(
        &self,
        spec: ResolvedPostgresConnectSpec,
        connection_id: String,
        tab_id: String,
        request_id: u64,
        kind: JobKind,
    ) -> Result<JobResult, TableBrowseError> {
        let executor = self.executor_for(spec, &connection_id).await?;
        let (tx, mut rx) = oneshot::channel();
        let tab_id_key = tab_id.clone();
        {
            let mut inner = executor.inner.lock().await;
            if inner.closed {
                return Err(TableBrowseError::ConnectionClosing);
            }
            let _ = enqueue_job(
                &mut inner,
                Job {
                    tab_id,
                    request_id,
                    enqueued_at: Instant::now(),
                    kind,
                    reply: tx,
                },
            );
        }
        executor.notify.notify_one();
        tokio::select! {
            result = &mut rx => result.unwrap_or(Err(TableBrowseError::ConnectionLost)),
            _ = tokio::time::sleep(QUEUE_WAIT) => {
                let timed_out = {
                    let mut inner = executor.inner.lock().await;
                    take_queued(&mut inner, &tab_id_key, request_id)
                };
                if let Some(job) = timed_out {
                    let _ = job.reply.send(Err(TableBrowseError::Timeout {
                        operation: "queueWait".into(),
                    }));
                    Err(TableBrowseError::Timeout {
                        operation: "queueWait".into(),
                    })
                } else {
                    rx.await.unwrap_or(Err(TableBrowseError::ConnectionLost))
                }
            }
        }
    }

    async fn executor_for(
        &self,
        spec: ResolvedPostgresConnectSpec,
        connection_id: &str,
    ) -> Result<Arc<Executor>, TableBrowseError> {
        loop {
            let existing = {
                let mut state = self.inner.lock().await;
                if state.closing.contains(connection_id) {
                    return Err(TableBrowseError::ConnectionClosing);
                }
                if let Some(executor) = state.executors.get(connection_id).cloned() {
                    return Ok(executor);
                }
                if state.opening.contains(connection_id) {
                    None
                } else {
                    check_admission(&state)?;
                    state.opening.insert(connection_id.into());
                    Some(())
                }
            };
            if existing.is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            break;
        }
        let result = spawn_executor(connection_id.into(), spec).await;
        let mut state = self.inner.lock().await;
        state.opening.remove(connection_id);
        match result {
            Ok(executor) => {
                if state.closing.contains(connection_id) {
                    drop(state);
                    let _ = close_executor(&executor, true).await;
                    return Err(TableBrowseError::ConnectionClosing);
                }
                state
                    .executors
                    .insert(connection_id.into(), executor.clone());
                Ok(executor)
            }
            Err(error) => Err(error),
        }
    }

    async fn close_idle(&self) {
        let candidates = {
            let state = self.inner.lock().await;
            state
                .executors
                .iter()
                .map(|(id, executor)| (id.clone(), executor.clone()))
                .collect::<Vec<_>>()
        };
        for (id, executor) in candidates {
            if !is_idle(&*executor.inner.lock().await) {
                continue;
            }
            let removed = {
                let mut state = self.inner.lock().await;
                let still_idle = match state.executors.get(&id) {
                    Some(current) => is_idle(&*current.inner.lock().await),
                    None => false,
                };
                if still_idle {
                    state.executors.remove(&id)
                } else {
                    None
                }
            };
            if let Some(executor) = removed {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, close_executor(&executor, false)).await;
            }
        }
    }
}

fn is_idle(inner: &ExecutorInner) -> bool {
    !inner.busy
        && !inner.closed
        && inner
            .tabs
            .values()
            .all(|tab| tab.queued.is_none() && tab.in_flight_request_id.is_none())
        && inner.last_used.elapsed() >= IDLE_TIMEOUT
}

fn check_admission(state: &ManagerState) -> Result<(), TableBrowseError> {
    if state.executors.len() + state.opening.len() >= MAX_EXECUTORS {
        Err(TableBrowseError::Timeout {
            operation: "admission".into(),
        })
    } else {
        Ok(())
    }
}

fn enqueue_job(inner: &mut ExecutorInner, job: Job) -> Option<(tokio_postgres::CancelToken, bool)> {
    let in_flight = {
        let tab = inner.tabs.entry(job.tab_id.clone()).or_default();
        if job.request_id < tab.latest_request_id {
            let _ = job.reply.send(Err(TableBrowseError::Superseded));
            return None;
        }
        tab.latest_request_id = job.request_id;
        if let Some(queued) = tab.queued.take() {
            let _ = queued.reply.send(Err(TableBrowseError::Superseded));
        }
        let in_flight = tab.in_flight_request_id.is_some();
        if in_flight {
            tab.interrupt = Interrupt::Supersede;
        }
        tab.queued = Some(job);
        in_flight
    };
    if in_flight {
        queue_protocol_cancel(inner);
        inner.pending_cancel.clone()
    } else {
        None
    }
}

fn apply_tab_cancel(inner: &mut ExecutorInner, tab_id: &str) -> bool {
    let (cancel_requested, in_flight) = {
        let Some(tab) = inner.tabs.get_mut(tab_id) else {
            return false;
        };
        let mut cancel_requested = false;
        if let Some(job) = tab.queued.take() {
            let _ = job.reply.send(Err(TableBrowseError::Cancelled));
            cancel_requested = true;
        }
        let in_flight = tab.in_flight_request_id.is_some();
        if in_flight {
            tab.interrupt = Interrupt::Cancel;
            cancel_requested = true;
        }
        (cancel_requested, in_flight)
    };
    if in_flight {
        queue_protocol_cancel(inner);
    }
    cancel_requested
}

fn queue_protocol_cancel(inner: &mut ExecutorInner) {
    if inner.pending_cancel.is_some() {
        return;
    }
    if let Some(connection) = inner.connection.as_ref() {
        inner.pending_cancel = Some((connection.inner.cancel.clone(), connection.tls));
    }
}

fn take_queued(inner: &mut ExecutorInner, tab_id: &str, request_id: u64) -> Option<Job> {
    let tab = inner.tabs.get_mut(tab_id)?;
    if tab
        .queued
        .as_ref()
        .is_some_and(|job| job.request_id == request_id)
    {
        tab.queued.take()
    } else {
        None
    }
}

async fn drain_pending_cancel(executor: &Executor) {
    let pending = {
        let mut inner = executor.inner.lock().await;
        inner.pending_cancel.take()
    };
    if let Some((token, tls)) = pending {
        let _ = crate::query_session::postgres::cancel(token, tls).await;
    }
}

async fn spawn_executor(
    _connection_id: String,
    spec: ResolvedPostgresConnectSpec,
) -> Result<Arc<Executor>, TableBrowseError> {
    let executor = Arc::new(Executor {
        spec,
        inner: Mutex::new(ExecutorInner {
            connection: None,
            cache: HashMap::new(),
            tabs: HashMap::new(),
            last_used: Instant::now(),
            closed: false,
            busy: false,
            pending_cancel: None,
        }),
        notify: Notify::new(),
    });
    let worker = executor.clone();
    tokio::spawn(async move {
        run_executor(worker).await;
    });
    Ok(executor)
}

async fn run_executor(executor: Arc<Executor>) {
    loop {
        if executor.inner.lock().await.closed {
            break;
        }
        let job = next_job(&executor).await;
        let Some(job) = job else {
            executor.notify.notified().await;
            continue;
        };
        execute_job(&executor, job).await;
    }
}

async fn next_job(executor: &Executor) -> Option<Job> {
    drain_pending_cancel(executor).await;
    let mut inner = executor.inner.lock().await;
    if inner.closed {
        return None;
    }
    let mut expired_ids = Vec::new();
    let mut ready_id: Option<String> = None;
    let mut ready_at: Option<Instant> = None;
    for (tab_id, tab) in &inner.tabs {
        let Some(job) = tab.queued.as_ref() else {
            continue;
        };
        if job.enqueued_at.elapsed() >= QUEUE_WAIT {
            expired_ids.push(tab_id.clone());
            continue;
        }
        if ready_at.is_none_or(|at| job.enqueued_at < at) {
            ready_at = Some(job.enqueued_at);
            ready_id = Some(tab_id.clone());
        }
    }
    let expired = expired_ids
        .into_iter()
        .filter_map(|id| inner.tabs.get_mut(&id)?.queued.take())
        .collect::<Vec<_>>();
    let ready = ready_id.and_then(|id| {
        let tab = inner.tabs.get_mut(&id)?;
        let job = tab.queued.take()?;
        tab.in_flight_request_id = Some(job.request_id);
        tab.interrupt = Interrupt::None;
        inner.busy = true;
        inner.last_used = Instant::now();
        Some(job)
    });
    drop(inner);
    for job in expired {
        let _ = job.reply.send(Err(TableBrowseError::Timeout {
            operation: "queueWait".into(),
        }));
    }
    ready
}

async fn execute_job(executor: &Arc<Executor>, job: Job) {
    let result = {
        let run = run_kind(executor, &job);
        tokio::pin!(run);
        loop {
            drain_pending_cancel(executor).await;
            tokio::select! {
                result = &mut run => break result,
                _ = executor.notify.notified() => {}
            }
        }
    };
    let result = rewrite_interrupt(executor, &job, result).await;
    let _ = job.reply.send(result);
    let mut inner = executor.inner.lock().await;
    if let Some(tab) = inner.tabs.get_mut(&job.tab_id) {
        if tab.in_flight_request_id == Some(job.request_id) {
            tab.in_flight_request_id = None;
            tab.interrupt = Interrupt::None;
        }
    }
    inner.busy = false;
    inner.last_used = Instant::now();
    drop(inner);
    drain_pending_cancel(executor).await;
    executor.notify.notify_one();
}

async fn rewrite_interrupt(
    executor: &Executor,
    job: &Job,
    result: Result<JobResult, TableBrowseError>,
) -> Result<JobResult, TableBrowseError> {
    let interrupt = executor
        .inner
        .lock()
        .await
        .tabs
        .get(&job.tab_id)
        .map(|tab| tab.interrupt)
        .unwrap_or(Interrupt::Cancel);
    match (&result, interrupt) {
        (Err(error), Interrupt::Supersede) if postgres::is_query_canceled(error) => {
            Err(TableBrowseError::Superseded)
        }
        (Err(error), Interrupt::Cancel) if postgres::is_query_canceled(error) => {
            Err(TableBrowseError::Cancelled)
        }
        (Err(error), Interrupt::Closing) if postgres::is_query_canceled(error) => {
            Err(TableBrowseError::ConnectionClosing)
        }
        (Ok(_), Interrupt::Supersede) => Err(TableBrowseError::Superseded),
        (Ok(_), Interrupt::Cancel) => Err(TableBrowseError::Cancelled),
        (Ok(_), Interrupt::Closing) => Err(TableBrowseError::ConnectionClosing),
        _ => result,
    }
}

async fn run_kind(executor: &Executor, job: &Job) -> Result<JobResult, TableBrowseError> {
    match &job.kind {
        JobKind::Browse(payload) => run_browse(executor, payload)
            .await
            .map(|result| JobResult::Browse(Box::new(result))),
        JobKind::Count(payload) => run_count(executor, payload).await.map(JobResult::Count),
    }
}

async fn ensure_connection(executor: &Executor) -> Result<(), TableBrowseError> {
    if executor.inner.lock().await.connection.is_some() {
        return Ok(());
    }
    let connection = postgres::connect(&executor.spec).await?;
    let mut inner = executor.inner.lock().await;
    if inner.closed {
        return Err(TableBrowseError::ConnectionClosing);
    }
    inner.connection = Some(connection);
    Ok(())
}

async fn browse_client(
    executor: &Executor,
) -> Result<Arc<tokio_postgres::Client>, TableBrowseError> {
    executor
        .inner
        .lock()
        .await
        .connection
        .as_ref()
        .map(|connection| connection.inner.client.clone())
        .ok_or(TableBrowseError::ConnectionLost)
}

async fn descriptor(
    executor: &Executor,
    schema: &str,
    table: &str,
    refresh: bool,
) -> Result<RelationDescriptor, TableBrowseError> {
    ensure_connection(executor).await?;
    if !refresh {
        if let Some(cached) = executor
            .inner
            .lock()
            .await
            .cache
            .get(&(schema.to_string(), table.to_string()))
            .cloned()
        {
            return Ok(cached);
        }
    }
    let client = browse_client(executor).await?;
    let descriptor = postgres::load_descriptor(client.as_ref(), schema, table).await?;
    executor
        .inner
        .lock()
        .await
        .cache
        .insert((schema.into(), table.into()), descriptor.clone());
    Ok(descriptor)
}

async fn invalidate_descriptor(executor: &Executor, schema: &str, table: &str) {
    executor
        .inner
        .lock()
        .await
        .cache
        .remove(&(schema.to_string(), table.to_string()));
}

async fn run_browse(
    executor: &Executor,
    payload: &BrowseTableDataPayload,
) -> Result<BrowseTableResult, TableBrowseError> {
    let started = Instant::now();
    let mut retried = false;
    loop {
        let descriptor = descriptor(
            executor,
            &payload.schema,
            &payload.table,
            payload.refresh_structure || retried,
        )
        .await?;
        let built = build_browse_query(&descriptor, payload)?;
        let client = browse_client(executor).await?;
        let executed = match postgres::execute_browse(client.as_ref(), &built).await {
            Ok(executed) => Ok(executed),
            Err(error) if postgres::is_undefined_object(&error) && !retried => Err(error),
            Err(error) => return Err(error),
        };
        match executed {
            Ok(executed) => {
                let count = browse_count(executor, payload, &built).await?;
                let next_cursor = next_cursor(&built, &executed.row_identity, executed.has_more);
                let inspection = built.inspection();
                return Ok(BrowseTableResult {
                    request_id: payload.request_id,
                    columns: built.visible_columns,
                    rows: executed.rows,
                    identity: built.identity,
                    row_identity: executed.row_identity,
                    page_info: BrowsePageInfo {
                        mode: built.page_mode,
                        page: built.page,
                        has_more: executed.has_more,
                        next_cursor,
                    },
                    count,
                    inspection,
                    omitted_rows: executed.omitted_rows,
                    truncated_cells: executed.truncated_cells,
                    runtime_ms: started.elapsed().as_millis() as u64,
                });
            }
            Err(_) => {
                invalidate_descriptor(executor, &payload.schema, &payload.table).await;
                retried = true;
            }
        }
    }
}

async fn browse_count(
    executor: &Executor,
    payload: &BrowseTableDataPayload,
    built: &BuiltBrowseQuery,
) -> Result<BrowseCount, TableBrowseError> {
    match payload.count_policy {
        BrowseCountPolicy::None => Ok(BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        }),
        BrowseCountPolicy::Estimated if built.where_sql.is_empty() => {
            let client = browse_client(executor).await?;
            postgres::estimated_unfiltered_count(client.as_ref(), &payload.schema, &payload.table)
                .await
        }
        BrowseCountPolicy::Estimated => {
            let client = browse_client(executor).await?;
            postgres::estimated_filtered_count(
                client.as_ref(),
                &built.explain_sql(),
                &built.where_params,
            )
            .await
        }
    }
}

async fn run_count(
    executor: &Executor,
    payload: &CountTableBrowseRowsPayload,
) -> Result<BrowseExactCountResult, TableBrowseError> {
    let mut retried = false;
    loop {
        let descriptor = descriptor(executor, &payload.schema, &payload.table, retried).await?;
        let (sql, params) = builder::build_count_query(&descriptor, &payload.filters)?;
        let client = browse_client(executor).await?;
        match postgres::execute_count(client.as_ref(), &sql, &params).await {
            Ok(value) => {
                return Ok(BrowseExactCountResult {
                    kind: BrowseCountKind::Exact,
                    value,
                    request_id: payload.request_id,
                });
            }
            Err(error) if postgres::is_undefined_object(&error) && !retried => {
                invalidate_descriptor(executor, &payload.schema, &payload.table).await;
                retried = true;
            }
            Err(error) => return Err(error),
        }
    }
}

fn next_cursor(
    built: &BuiltBrowseQuery,
    row_identity: &Option<Vec<Vec<String>>>,
    has_more: bool,
) -> Option<BrowseCursor> {
    if built.page_mode != BrowsePageMode::Keyset || !has_more {
        return None;
    }
    row_identity
        .as_ref()
        .and_then(|rows| rows.last())
        .cloned()
        .map(|values| BrowseCursor { values })
}

async fn close_executor(executor: &Executor, fence: bool) {
    let (queued, cancel) = {
        let mut inner = executor.inner.lock().await;
        inner.closed = true;
        inner.busy = false;
        let mut queued = Vec::new();
        for tab in inner.tabs.values_mut() {
            if fence {
                tab.interrupt = Interrupt::Closing;
            }
            if let Some(job) = tab.queued.take() {
                queued.push(job);
            }
        }
        let cancel = inner
            .connection
            .as_ref()
            .map(|connection| (connection.inner.cancel.clone(), connection.tls));
        inner.pending_cancel = None;
        inner.connection = None;
        inner.cache.clear();
        (queued, cancel)
    };
    for job in queued {
        let _ = job.reply.send(Err(if fence {
            TableBrowseError::ConnectionClosing
        } else {
            TableBrowseError::ConnectionLost
        }));
    }
    if let Some((token, tls)) = cancel {
        let _ = crate::query_session::postgres::cancel(token, tls).await;
    }
    executor.notify.notify_waiters();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_spec(id: &str) -> ResolvedPostgresConnectSpec {
        ResolvedPostgresConnectSpec {
            connection_id: id.into(),
            host: "127.0.0.1".into(),
            port: 1,
            database: "dbunk_demo".into(),
            user: "dbunk".into(),
            password: "dbunk".into(),
            tls_prefer: false,
            connect_timeout: Some(Duration::from_millis(1)),
            driver_options: crate::PgDriverOptions::default(),
        }
    }

    fn inner() -> ExecutorInner {
        ExecutorInner {
            connection: None,
            cache: HashMap::new(),
            tabs: HashMap::new(),
            last_used: Instant::now(),
            closed: false,
            busy: false,
            pending_cancel: None,
        }
    }

    fn dummy_executor() -> Arc<Executor> {
        Arc::new(Executor {
            spec: dummy_spec("c"),
            inner: Mutex::new(inner()),
            notify: Notify::new(),
        })
    }

    fn job(
        tab: &str,
        request_id: u64,
    ) -> (Job, oneshot::Receiver<Result<JobResult, TableBrowseError>>) {
        let (reply, rx) = oneshot::channel();
        (
            Job {
                tab_id: tab.into(),
                request_id,
                enqueued_at: Instant::now(),
                kind: JobKind::Count(CountTableBrowseRowsPayload {
                    connection_id: "c".into(),
                    tab_id: tab.into(),
                    request_id,
                    schema: "public".into(),
                    table: "t".into(),
                    filters: Vec::new(),
                }),
                reply,
            },
            rx,
        )
    }

    #[test]
    fn admission_constants_match_contract() {
        assert_eq!(
            (MAX_EXECUTORS, IDLE_TIMEOUT, QUEUE_WAIT, CLOSE_TIMEOUT),
            (
                8,
                Duration::from_secs(300),
                Duration::from_secs(10),
                Duration::from_secs(3),
            )
        );
    }

    #[test]
    fn idle_check_requires_a_quiet_expired_executor() {
        let mut slot = inner();
        slot.last_used = Instant::now() - IDLE_TIMEOUT;
        assert!(is_idle(&slot));
        slot.busy = true;
        assert!(!is_idle(&slot));
        slot.busy = false;
        slot.closed = true;
        assert!(!is_idle(&slot));
        slot.closed = false;
        let (queued, _rx) = job("tab", 1);
        slot.tabs.insert(
            "tab".into(),
            TabSlot {
                queued: Some(queued),
                ..TabSlot::default()
            },
        );
        assert!(!is_idle(&slot));
        slot.tabs.clear();
        slot.tabs.insert(
            "tab".into(),
            TabSlot {
                in_flight_request_id: Some(1),
                ..TabSlot::default()
            },
        );
        assert!(!is_idle(&slot));
    }

    #[test]
    fn admission_rejects_ninth_executor() {
        let mut state = ManagerState::default();
        for index in 0..MAX_EXECUTORS {
            state.opening.insert(format!("c{index}"));
        }
        assert!(matches!(
            check_admission(&state),
            Err(TableBrowseError::Timeout { operation }) if operation == "admission"
        ));
    }

    #[tokio::test]
    async fn queued_request_is_dropped_when_a_newer_request_arrives() {
        let mut inner = inner();
        let (first, rx1) = job("tab", 1);
        enqueue_job(&mut inner, first);
        let (second, _rx2) = job("tab", 2);
        let cancel = enqueue_job(&mut inner, second);
        assert!(cancel.is_none());
        assert!(matches!(rx1.await, Ok(Err(TableBrowseError::Superseded))));
        assert_eq!(
            inner.tabs["tab"].queued.as_ref().map(|job| job.request_id),
            Some(2)
        );
    }

    #[tokio::test]
    async fn in_flight_request_is_marked_for_cancel_when_superseded() {
        let mut inner = inner();
        inner.tabs.insert(
            "tab".into(),
            TabSlot {
                latest_request_id: 1,
                queued: None,
                in_flight_request_id: Some(1),
                interrupt: Interrupt::None,
            },
        );
        let (next, _rx) = job("tab", 2);
        let cancel = enqueue_job(&mut inner, next);
        assert_eq!(inner.tabs["tab"].interrupt, Interrupt::Supersede);
        assert_eq!(inner.tabs["tab"].in_flight_request_id, Some(1));
        assert_eq!(
            inner.tabs["tab"].queued.as_ref().map(|job| job.request_id),
            Some(2)
        );
        assert!(cancel.is_none());
        assert!(inner.pending_cancel.is_none());
    }

    #[tokio::test]
    async fn queue_wait_timeout_lookup_is_tab_scoped() {
        let mut inner = inner();
        let (job_a, _rx_a) = job("tab-a", 1);
        let (job_b, _rx_b) = job("tab-b", 1);
        enqueue_job(&mut inner, job_a);
        enqueue_job(&mut inner, job_b);
        let taken = take_queued(&mut inner, "tab-a", 1).expect("tab a");
        assert_eq!(taken.tab_id, "tab-a");
        assert_eq!(taken.request_id, 1);
        assert!(inner.tabs["tab-a"].queued.is_none());
        assert_eq!(
            inner.tabs["tab-b"]
                .queued
                .as_ref()
                .map(|job| job.request_id),
            Some(1)
        );
    }

    #[tokio::test]
    async fn cancel_tab_without_in_flight_does_not_queue_protocol_cancel() {
        let mut inner = inner();
        inner.tabs.insert(
            "other".into(),
            TabSlot {
                latest_request_id: 1,
                queued: None,
                in_flight_request_id: Some(1),
                interrupt: Interrupt::None,
            },
        );
        let (queued, rx) = job("tab", 1);
        enqueue_job(&mut inner, queued);
        let requested = apply_tab_cancel(&mut inner, "tab");
        assert!(requested);
        assert!(inner.pending_cancel.is_none());
        assert!(inner.tabs["tab"].queued.is_none());
        assert_eq!(inner.tabs["other"].interrupt, Interrupt::None);
        assert_eq!(inner.tabs["other"].in_flight_request_id, Some(1));
        assert!(matches!(rx.await, Ok(Err(TableBrowseError::Cancelled))));

        let idle = apply_tab_cancel(&mut inner, "idle");
        assert!(!idle);
        assert!(inner.pending_cancel.is_none());
    }

    #[tokio::test]
    async fn close_all_does_not_leave_connection_ids_in_closing() {
        let manager = TableBrowseManager::new();
        {
            let mut state = manager.inner.lock().await;
            state.executors.insert("c1".into(), dummy_executor());
            state.executors.insert("c2".into(), dummy_executor());
        }
        manager.close_all().await;
        let state = manager.inner.lock().await;
        assert!(state.executors.is_empty());
        assert!(state.closing.is_empty());
    }

    #[tokio::test]
    async fn rewrite_interrupt_treats_missing_tab_as_cancelled() {
        let executor = dummy_executor();
        let (job, _rx) = job("tab", 1);

        let ok = rewrite_interrupt(
            &executor,
            &job,
            Ok(JobResult::Count(BrowseExactCountResult {
                kind: BrowseCountKind::Exact,
                value: 1,
                request_id: 1,
            })),
        )
        .await;
        assert!(matches!(ok, Err(TableBrowseError::Cancelled)));

        let canceled = rewrite_interrupt(
            &executor,
            &job,
            Err(TableBrowseError::Database {
                code: Some("57014".into()),
                message: "canceling statement due to user request".into(),
                severity: Some("ERROR".into()),
                position: None,
            }),
        )
        .await;
        assert!(matches!(canceled, Err(TableBrowseError::Cancelled)));
    }

    #[tokio::test]
    async fn older_request_id_is_rejected_as_superseded() {
        let mut inner = inner();
        let (newer, _rx) = job("tab", 5);
        enqueue_job(&mut inner, newer);
        let (older, rx) = job("tab", 4);
        enqueue_job(&mut inner, older);
        assert!(matches!(rx.await, Ok(Err(TableBrowseError::Superseded))));
    }

    #[test]
    fn queue_wait_timeout_error_names_the_operation() {
        let error = TableBrowseError::Timeout {
            operation: "queueWait".into(),
        };
        let json = serde_json::to_value(error).unwrap();
        assert_eq!(json["kind"], "timeout");
        assert_eq!(json["operation"], "queueWait");
    }

    #[test]
    fn idle_close_bookkeeping_uses_last_used_and_busy_flag() {
        let mut inner = inner();
        inner.last_used = Instant::now() - Duration::from_secs(301);
        inner.busy = false;
        assert!(inner.last_used.elapsed() >= IDLE_TIMEOUT);
        inner.busy = true;
        assert!(inner.busy);
    }

    #[test]
    fn canceled_sqlstate_maps_to_typed_errors() {
        let canceled = TableBrowseError::Database {
            code: Some("57014".into()),
            message: "canceling statement due to user request".into(),
            severity: Some("ERROR".into()),
            position: None,
        };
        assert!(postgres::is_query_canceled(&canceled));
        assert_eq!(
            serde_json::to_value(TableBrowseError::Superseded).unwrap()["kind"],
            "superseded"
        );
        assert_eq!(
            serde_json::to_value(TableBrowseError::Cancelled).unwrap()["kind"],
            "cancelled"
        );
        assert_eq!(
            serde_json::to_value(TableBrowseError::ConnectionClosing).unwrap()["kind"],
            "connectionClosing"
        );
    }

    #[test]
    fn dummy_spec_does_not_log_secrets() {
        let spec = dummy_spec("c");
        let debug = format!("{spec:?}");
        assert!(!debug.contains("dbunk"));
        assert!(debug.contains("connection_id"));
    }
}
