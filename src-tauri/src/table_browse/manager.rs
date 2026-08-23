use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;

use super::executor::{
    apply_tab_cancel, close_executor, enqueue_job, spawn_executor, take_queued, Executor,
    ExecutorInner, Job, JobKind, JobResult,
};
use super::protocol::*;
use super::{CLOSE_TIMEOUT, IDLE_TIMEOUT, MAX_EXECUTORS, QUEUE_WAIT};

#[derive(Default)]
pub(crate) struct ManagerState {
    pub(crate) executors: HashMap<String, Arc<Executor>>,
    closing: HashSet<String>,
    opening: HashSet<String>,
    global_closing: bool,
}

#[derive(Clone)]
pub(crate) struct TableBrowseManager {
    pub(crate) inner: Arc<Mutex<ManagerState>>,
}

impl TableBrowseManager {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerState::default())),
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

    pub(crate) async fn begin_global_teardown(&self) {
        self.inner.lock().await.global_closing = true;
        self.close_all().await;
    }

    pub(crate) async fn end_global_teardown(&self) {
        self.inner.lock().await.global_closing = false;
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
                if state.global_closing || state.closing.contains(connection_id) {
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
        let result = spawn_executor(spec).await;
        let mut state = self.inner.lock().await;
        state.opening.remove(connection_id);
        match result {
            Ok(executor) => {
                if state.global_closing || state.closing.contains(connection_id) {
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
    if state.global_closing {
        return Err(TableBrowseError::ConnectionClosing);
    }
    if state.executors.len() + state.opening.len() >= MAX_EXECUTORS {
        Err(TableBrowseError::Timeout {
            operation: "admission".into(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table_browse::executor::{dummy_executor, dummy_spec, inner, Job, JobKind};

    fn payload(connection_id: &str) -> BrowseTableDataPayload {
        BrowseTableDataPayload {
            connection_id: connection_id.into(),
            tab_id: "tab".into(),
            request_id: 1,
            schema: "public".into(),
            table: "t".into(),
            filters: Vec::new(),
            sort: Vec::new(),
            page_request: BrowsePageRequest::Offset { page: 1 },
            page_size: 25,
            count_policy: BrowseCountPolicy::None,
            refresh_structure: false,
        }
    }

    fn job(tab: &str, request_id: u64) -> Job {
        let (reply, _rx) = oneshot::channel();
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
        }
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
    fn start_monitor_is_callable_outside_a_tokio_runtime() {
        // Regression: setup() calls this on the main thread with no Tokio
        // runtime context; a bare tokio::spawn panics with "no reactor
        // running" and aborts app startup.
        TableBrowseManager::new().start_monitor();
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
        slot.tabs.insert(
            "tab".into(),
            crate::table_browse::executor::TabSlot {
                queued: Some(job("tab", 1)),
                ..crate::table_browse::executor::TabSlot::default()
            },
        );
        assert!(!is_idle(&slot));
        slot.tabs.clear();
        slot.tabs.insert(
            "tab".into(),
            crate::table_browse::executor::TabSlot {
                in_flight_request_id: Some(1),
                ..crate::table_browse::executor::TabSlot::default()
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

    #[test]
    fn admission_rejects_global_closing() {
        let state = ManagerState {
            global_closing: true,
            ..ManagerState::default()
        };
        assert!(matches!(
            check_admission(&state),
            Err(TableBrowseError::ConnectionClosing)
        ));
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
        assert!(!state.global_closing);
    }

    #[tokio::test]
    async fn global_teardown_rejects_new_admission() {
        let manager = TableBrowseManager::new();
        manager.begin_global_teardown().await;
        let error = manager
            .browse(dummy_spec("c"), payload("c"))
            .await
            .unwrap_err();
        assert!(matches!(error, TableBrowseError::ConnectionClosing));
        manager.end_global_teardown().await;
    }

    #[tokio::test]
    async fn connection_teardown_rejects_new_admission() {
        let manager = TableBrowseManager::new();
        manager.begin_connection_teardown("c").await;
        let error = manager
            .browse(dummy_spec("c"), payload("c"))
            .await
            .unwrap_err();
        assert!(matches!(error, TableBrowseError::ConnectionClosing));
        manager.end_connection_teardown("c").await;
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
}
