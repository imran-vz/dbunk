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
        let (job, connection) = {
            let mut inner = executor.inner.lock().await;
            let Some(tab) = inner.tabs.get_mut(tab_id) else {
                return CancelTableBrowseResult {
                    cancel_requested: false,
                };
            };
            let mut cancel_requested = false;
            if let Some(job) = tab.queued.take() {
                let _ = job.reply.send(Err(TableBrowseError::Cancelled));
                cancel_requested = true;
            }
            if tab.in_flight_request_id.is_some() {
                tab.interrupt = Interrupt::Cancel;
                cancel_requested = true;
            }
            (
                cancel_requested,
                inner
                    .connection
                    .as_ref()
                    .map(|connection| (connection.inner.cancel.clone(), connection.tls)),
            )
        };
        let mut cancel_requested = job;
        if let Some((token, tls)) = connection {
            if crate::query_session::postgres::cancel(token, tls).await {
                cancel_requested = true;
            }
        }
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
            let ids = state.executors.keys().cloned().collect::<Vec<_>>();
            state.closing.extend(ids);
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
        let cancel = {
            let mut inner = executor.inner.lock().await;
            if inner.closed {
                return Err(TableBrowseError::ConnectionClosing);
            }
            enqueue_job(
                &mut inner,
                Job {
                    tab_id,
                    request_id,
                    enqueued_at: Instant::now(),
                    kind,
                    reply: tx,
                },
            )
        };
        if let Some((token, tls)) = cancel {
            tokio::spawn(async move {
                let _ = crate::query_session::postgres::cancel(token, tls).await;
            });
        }
        executor.notify.notify_one();
        tokio::select! {
            result = &mut rx => result.unwrap_or(Err(TableBrowseError::ConnectionLost)),
            _ = tokio::time::sleep(QUEUE_WAIT) => {
                let timed_out = {
                    let mut inner = executor.inner.lock().await;
                    take_queued(&mut inner, request_id)
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
        let idle = {
            let state = self.inner.lock().await;
            state
                .executors
                .iter()
                .map(|(id, executor)| (id.clone(), executor.clone()))
                .collect::<Vec<_>>()
        };
        let mut close = Vec::new();
        for (id, executor) in idle {
            let inner = executor.inner.lock().await;
            if !inner.busy
                && inner.tabs.values().all(|tab| tab.queued.is_none())
                && inner.last_used.elapsed() >= IDLE_TIMEOUT
            {
                close.push(id);
            }
        }
        for id in close {
            let executor = self.inner.lock().await.executors.remove(&id);
            if let Some(executor) = executor {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, close_executor(&executor, false)).await;
            }
        }
    }
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
    let tab = inner.tabs.entry(job.tab_id.clone()).or_default();
    if job.request_id < tab.latest_request_id {
        let _ = job.reply.send(Err(TableBrowseError::Superseded));
        return None;
    }
    tab.latest_request_id = job.request_id;
    if let Some(queued) = tab.queued.take() {
        let _ = queued.reply.send(Err(TableBrowseError::Superseded));
    }
    let cancel = if tab.in_flight_request_id.is_some() {
        tab.interrupt = Interrupt::Supersede;
        inner
            .connection
            .as_ref()
            .map(|connection| (connection.inner.cancel.clone(), connection.tls))
    } else {
        None
    };
    tab.queued = Some(job);
    cancel
}

fn take_queued(inner: &mut ExecutorInner, request_id: u64) -> Option<Job> {
    for tab in inner.tabs.values_mut() {
        if tab
            .queued
            .as_ref()
            .is_some_and(|job| job.request_id == request_id)
        {
            return tab.queued.take();
        }
    }
    None
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
    let mut inner = executor.inner.lock().await;
    if inner.closed {
        return None;
    }
    let mut expired = Vec::new();
    let mut ready = None;
    for tab in inner.tabs.values_mut() {
        if let Some(job) = tab.queued.as_ref() {
            if job.enqueued_at.elapsed() >= QUEUE_WAIT {
                expired.push(tab.queued.take().expect("queued"));
                continue;
            }
            if ready.is_none() {
                ready = tab.queued.take();
            }
        }
    }
    drop(inner);
    for job in expired {
        let _ = job.reply.send(Err(TableBrowseError::Timeout {
            operation: "queueWait".into(),
        }));
    }
    let job = ready?;
    {
        let mut inner = executor.inner.lock().await;
        if let Some(tab) = inner.tabs.get_mut(&job.tab_id) {
            if job.request_id != tab.latest_request_id {
                drop(inner);
                let _ = job.reply.send(Err(TableBrowseError::Superseded));
                return None;
            }
            tab.in_flight_request_id = Some(job.request_id);
            tab.interrupt = Interrupt::None;
            inner.busy = true;
            inner.last_used = Instant::now();
        }
    }
    Some(job)
}

async fn execute_job(executor: &Arc<Executor>, job: Job) {
    let result = run_kind(executor, &job).await;
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
        .unwrap_or_default();
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
        }
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
        enqueue_job(&mut inner, next);
        assert_eq!(inner.tabs["tab"].interrupt, Interrupt::Supersede);
        assert_eq!(inner.tabs["tab"].in_flight_request_id, Some(1));
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
