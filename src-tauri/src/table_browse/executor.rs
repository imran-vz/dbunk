use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{oneshot, Mutex, Notify};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;

use super::builder::RelationDescriptor;
use super::interrupt::{finalize_interrupt, Interrupt};
use super::postgres::BrowseConnection;
use super::protocol::*;
use super::{postgres, service};

pub(crate) enum JobKind {
    Browse(BrowseTableDataPayload),
    Count(CountTableBrowseRowsPayload),
}

pub(crate) enum JobResult {
    Browse(Box<BrowseTableResult>),
    Count(BrowseExactCountResult),
}

pub(crate) struct Job {
    pub(crate) tab_id: String,
    pub(crate) request_id: u64,
    pub(crate) enqueued_at: Instant,
    pub(crate) kind: JobKind,
    pub(crate) reply: oneshot::Sender<Result<JobResult, TableBrowseError>>,
}

#[derive(Default)]
pub(crate) struct TabSlot {
    pub(crate) latest_request_id: u64,
    pub(crate) queued: Option<Job>,
    pub(crate) in_flight_request_id: Option<u64>,
    pub(crate) interrupt: Interrupt,
}

pub(crate) struct ExecutorInner {
    pub(crate) connection: Option<BrowseConnection>,
    pub(crate) cache: HashMap<(String, String), RelationDescriptor>,
    pub(crate) tabs: HashMap<String, TabSlot>,
    pub(crate) last_used: Instant,
    pub(crate) closed: bool,
    pub(crate) busy: bool,
    pub(crate) pending_cancel: Option<(tokio_postgres::CancelToken, bool)>,
}

pub(crate) struct Executor {
    pub(crate) spec: ResolvedPostgresConnectSpec,
    pub(crate) inner: Mutex<ExecutorInner>,
    pub(crate) notify: Notify,
}

pub(crate) fn enqueue_job(
    inner: &mut ExecutorInner,
    job: Job,
) -> Option<(tokio_postgres::CancelToken, bool)> {
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

pub(crate) fn apply_tab_cancel(inner: &mut ExecutorInner, tab_id: &str) -> bool {
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
        inner.pending_cancel = Some((connection.inner.cancel.clone(), connection.inner.tls));
    }
}

pub(crate) fn take_queued(inner: &mut ExecutorInner, tab_id: &str, request_id: u64) -> Option<Job> {
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
        let _ = crate::postgres::dedicated::cancel(token, tls).await;
    }
}

pub(crate) async fn spawn_executor(
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
        if job.enqueued_at.elapsed() >= super::QUEUE_WAIT {
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
        let run = service::run_kind(executor, &job);
        tokio::pin!(run);
        loop {
            drain_pending_cancel(executor).await;
            tokio::select! {
                result = &mut run => break result,
                _ = executor.notify.notified() => {}
            }
        }
    };
    let terminal = {
        let mut inner = executor.inner.lock().await;
        complete_job(&mut inner, &job.tab_id, job.request_id, result)
    };
    let _ = job.reply.send(terminal);
    drain_pending_cancel(executor).await;
    executor.notify.notify_one();
}

pub(crate) fn complete_job(
    inner: &mut ExecutorInner,
    tab_id: &str,
    request_id: u64,
    result: Result<JobResult, TableBrowseError>,
) -> Result<JobResult, TableBrowseError> {
    let interrupt = inner
        .tabs
        .get(tab_id)
        .map(|tab| tab.interrupt)
        .unwrap_or(Interrupt::Cancel);
    let terminal = finalize_interrupt(interrupt, result);
    if matches!(&terminal, Err(error) if postgres::is_dead_socket(error)) {
        inner.connection = None;
    }
    if let Some(tab) = inner.tabs.get_mut(tab_id) {
        if tab.in_flight_request_id == Some(request_id) {
            tab.in_flight_request_id = None;
            tab.interrupt = Interrupt::None;
        }
    }
    inner.busy = false;
    inner.last_used = Instant::now();
    terminal
}

pub(crate) async fn close_executor(executor: &Executor, fence: bool) {
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
            .map(|connection| (connection.inner.cancel.clone(), connection.inner.tls));
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
        let _ = crate::postgres::dedicated::cancel(token, tls).await;
    }
    executor.notify.notify_waiters();
}

#[cfg(test)]
pub(crate) fn dummy_spec(id: &str) -> ResolvedPostgresConnectSpec {
    ResolvedPostgresConnectSpec {
        connection_id: id.into(),
        host: "127.0.0.1".into(),
        port: 1,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        password: "dbunk".into(),
        tls_prefer: false,
        connect_timeout: Some(std::time::Duration::from_millis(1)),
        driver_options: crate::PgDriverOptions::default(),
    }
}

#[cfg(test)]
pub(crate) fn dummy_executor() -> Arc<Executor> {
    Arc::new(Executor {
        spec: dummy_spec("c"),
        inner: Mutex::new(inner()),
        notify: Notify::new(),
    })
}

#[cfg(test)]
pub(crate) fn inner() -> ExecutorInner {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn count_ok() -> JobResult {
        JobResult::Count(BrowseExactCountResult {
            kind: BrowseCountKind::Exact,
            value: 1,
            request_id: 1,
        })
    }

    fn database() -> TableBrowseError {
        TableBrowseError::Database {
            code: Some("42703".into()),
            message: "undefined column".into(),
            severity: Some("ERROR".into()),
            position: None,
        }
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
    async fn older_request_id_is_rejected_as_superseded() {
        let mut inner = inner();
        let (newer, _rx) = job("tab", 5);
        enqueue_job(&mut inner, newer);
        let (older, rx) = job("tab", 4);
        enqueue_job(&mut inner, older);
        assert!(matches!(rx.await, Ok(Err(TableBrowseError::Superseded))));
    }

    #[test]
    fn complete_job_applies_interrupt_and_clears_inflight_atomically() {
        let mut inner = inner();
        inner.busy = true;
        inner.tabs.insert(
            "tab".into(),
            TabSlot {
                latest_request_id: 1,
                queued: None,
                in_flight_request_id: Some(1),
                interrupt: Interrupt::Supersede,
            },
        );
        let terminal = complete_job(&mut inner, "tab", 1, Ok(count_ok()));
        assert!(matches!(terminal, Err(TableBrowseError::Superseded)));
        assert!(inner.tabs["tab"].in_flight_request_id.is_none());
        assert_eq!(inner.tabs["tab"].interrupt, Interrupt::None);
        assert!(!inner.busy);

        inner.busy = true;
        inner.tabs.insert(
            "tab".into(),
            TabSlot {
                latest_request_id: 2,
                queued: None,
                in_flight_request_id: Some(2),
                interrupt: Interrupt::Cancel,
            },
        );
        let terminal = complete_job(&mut inner, "tab", 2, Err(database()));
        assert!(matches!(terminal, Err(TableBrowseError::Cancelled)));
        assert!(inner.tabs["tab"].in_flight_request_id.is_none());
    }

    #[test]
    fn complete_job_treats_a_missing_tab_as_cancelled() {
        let mut inner = inner();
        inner.busy = true;
        let terminal = complete_job(&mut inner, "gone", 1, Ok(count_ok()));
        assert!(matches!(terminal, Err(TableBrowseError::Cancelled)));
        assert!(!inner.busy);
    }

    #[test]
    fn dummy_spec_does_not_log_secrets() {
        let spec = dummy_spec("c");
        let debug = format!("{spec:?}");
        assert!(!debug.contains("dbunk"));
        assert!(debug.contains("connection_id"));
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
}
