pub(crate) mod observer;
pub(crate) mod postgres;
pub(crate) mod protocol;

use futures_util::future::BoxFuture;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tokio::sync::{watch, Mutex, Notify};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use observer::Observer;
use protocol::*;

const MAX_SESSIONS_PER_CONNECTION: usize = 7;
const MAX_ACTIVE_CONNECTIONS: usize = 8;
const MAX_SESSIONS: usize = 24;
const LEASE: Duration = Duration::from_secs(120);

struct Credit {
    outstanding: VecDeque<(u64, usize)>,
    execution_id: Option<String>,
    terminal_sequence: Option<u64>,
    retain_more_rows: bool,
    last_ack: Instant,
    last_acked_sequence: u64,
}
struct Session {
    id: String,
    tab_id: String,
    connection_id: String,
    generation: u64,
    owner_id: String,
    window_label: String,
    tls: bool,
    channel: Channel<QueryEventEnvelope>,
    connection: Arc<postgres::SessionConnection>,
    observer: Arc<Mutex<Arc<Observer>>>,
    transaction: Mutex<QueryTransactionSnapshot>,
    sequence: Mutex<u64>,
    credit: Mutex<Credit>,
    credit_changed: Notify,
    last_liveness: Mutex<Instant>,
    focused: Mutex<bool>,
    closed: Mutex<bool>,
}
#[derive(Default)]
struct ManagerState {
    owners: HashMap<String, String>,
    sessions: HashMap<String, Arc<Session>>,
    observers: HashMap<String, Arc<Mutex<Arc<Observer>>>>,
    generations: HashMap<String, u64>,
    closing: HashSet<String>,
    opening: HashMap<String, String>,
    observer_opening: HashMap<String, watch::Sender<bool>>,
    global_closing: bool,
}
#[derive(Clone)]
pub(crate) struct QuerySessionManager {
    inner: Arc<Mutex<ManagerState>>,
    pool: SqlitePool,
}

pub(crate) type ExecutionSuccessHook = Box<
    dyn FnOnce(
            crate::safety::policy::WriteIntent,
            crate::safety::policy::SafetyAuthorization,
        ) -> BoxFuture<'static, ()>
        + Send
        + 'static,
>;

struct ExecutionAdmission {
    on_success: ExecutionSuccessHook,
    intent: crate::safety::policy::WriteIntent,
    authorization: crate::safety::policy::SafetyAuthorization,
}

pub(crate) struct ExecutionSafety<'a> {
    pub policy: &'a crate::safety::policy::ResolvedSafetyPolicy,
    pub confirmed: bool,
    pub on_success: Option<ExecutionSuccessHook>,
}

impl QuerySessionManager {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerState::default())),
            pool,
        }
    }
    pub(crate) fn start_monitor(&self) {
        let manager = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            loop {
                tick.tick().await;
                manager.expire_stalled().await;
            }
        });
    }
    async fn expire_stalled(&self) {
        let sessions = self
            .inner
            .lock()
            .await
            .sessions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            if !*session.focused.lock().await {
                continue;
            }
            let owner_stalled = session.last_liveness.lock().await.elapsed() >= LEASE;
            let ack_stalled = {
                let credit = session.credit.lock().await;
                (!credit.outstanding.is_empty() || credit.terminal_sequence.is_some())
                    && credit.last_ack.elapsed() >= LEASE
            };
            if owner_stalled || ack_stalled {
                let _ = send(
                    &session,
                    None,
                    false,
                    QueryEvent::SessionLost {
                        reason: if ack_stalled {
                            "ackTimeout".into()
                        } else {
                            "ownerTimeout".into()
                        },
                    },
                )
                .await;
                self.remove_and_close(&session.id, false).await;
            }
        }
    }
    pub(crate) async fn register_owner(
        &self,
        window: &str,
        owner_id: String,
    ) -> RegisterOwnerResult {
        let ids = {
            let mut state = self.inner.lock().await;
            let replaced = state
                .owners
                .insert(window.into(), owner_id.clone())
                .is_some_and(|old| old != owner_id);
            if replaced {
                state
                    .sessions
                    .values()
                    .filter(|session| {
                        session.window_label == window && session.owner_id != owner_id
                    })
                    .map(|session| session.id.clone())
                    .collect()
            } else {
                Vec::new()
            }
        };
        let count = ids.len();
        for id in ids {
            self.remove_and_close(&id, false).await;
        }
        RegisterOwnerResult {
            replaced_session_count: count,
        }
    }
    pub(crate) async fn open(
        &self,
        window: &str,
        payload: OpenSessionPayload,
        channel: Channel<QueryEventEnvelope>,
        spec: ResolvedPostgresConnectSpec,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        {
            let mut state = self.inner.lock().await;
            self.check_admission(&state, window, &payload)?;
            state
                .opening
                .insert(payload.session_id.clone(), payload.connection_id.clone());
        }
        let observer = match self.observer_for_open(&payload.connection_id, &spec).await {
            Ok(observer) => observer,
            Err(error) => {
                self.release_opening(&payload.session_id).await;
                return Err(error);
            }
        };
        let connection = match postgres::connect(&spec).await {
            Ok(connection) => connection,
            Err(error) => {
                self.release_opening(&payload.session_id).await;
                return Err(error);
            }
        };
        let session = {
            let mut state = self.inner.lock().await;
            if state.owners.get(window) != Some(&payload.owner_id) {
                release_opening_locked(&mut state, &payload.session_id);
                return Err(QuerySessionError::OwnerMismatch);
            }
            if state.global_closing || state.closing.contains(&payload.connection_id) {
                release_opening_locked(&mut state, &payload.session_id);
                return Err(QuerySessionError::ConnectionClosing);
            }
            state.opening.remove(&payload.session_id);
            let generation = *state
                .generations
                .entry(payload.connection_id.clone())
                .or_default();
            let session = Arc::new(Session {
                id: payload.session_id.clone(),
                tab_id: payload.tab_id,
                connection_id: payload.connection_id,
                generation,
                owner_id: payload.owner_id,
                window_label: window.into(),
                tls: spec.tls_prefer,
                channel,
                connection: Arc::new(connection),
                observer,
                transaction: Mutex::new(QueryTransactionSnapshot::default()),
                sequence: Mutex::new(0),
                credit: Mutex::new(Credit {
                    outstanding: VecDeque::new(),
                    execution_id: None,
                    terminal_sequence: None,
                    retain_more_rows: true,
                    last_ack: Instant::now(),
                    last_acked_sequence: 0,
                }),
                credit_changed: Notify::new(),
                last_liveness: Mutex::new(Instant::now()),
                focused: Mutex::new(true),
                closed: Mutex::new(false),
            });
            state.sessions.insert(payload.session_id, session.clone());
            session
        };
        if send(
            &session,
            None,
            false,
            QueryEvent::SessionState {
                transaction: QueryTransactionSnapshot::default(),
            },
        )
        .await
        .is_err()
        {
            self.remove_and_close(&session.id, false).await;
            return Err(QuerySessionError::ConnectionLost);
        }
        Ok(QueryTransactionSnapshot::default())
    }
    async fn release_opening(&self, session_id: &str) {
        let mut state = self.inner.lock().await;
        release_opening_locked(&mut state, session_id);
    }
    async fn observer_for_open(
        &self,
        connection_id: &str,
        spec: &ResolvedPostgresConnectSpec,
    ) -> Result<Arc<Mutex<Arc<Observer>>>, QuerySessionError> {
        loop {
            let wait = {
                let mut state = self.inner.lock().await;
                if let Some(observer) = state.observers.get(connection_id) {
                    return Ok(observer.clone());
                }
                if let Some(opening) = state.observer_opening.get(connection_id) {
                    Some(opening.subscribe())
                } else {
                    let (completed, _) = watch::channel(false);
                    state
                        .observer_opening
                        .insert(connection_id.into(), completed);
                    None
                }
            };
            if let Some(mut wait) = wait {
                let _ = wait.changed().await;
                continue;
            }

            let result = Observer::connect(spec)
                .await
                .map(|observer| Arc::new(Mutex::new(observer)));
            let mut state = self.inner.lock().await;
            let opening = state.observer_opening.remove(connection_id);
            if let Ok(observer) = &result {
                state
                    .observers
                    .insert(connection_id.into(), observer.clone());
            }
            if let Some(opening) = opening {
                opening.send_replace(true);
            }
            return result;
        }
    }
    fn check_admission(
        &self,
        state: &ManagerState,
        window: &str,
        payload: &OpenSessionPayload,
    ) -> Result<(), QuerySessionError> {
        if state.owners.get(window) != Some(&payload.owner_id) {
            return Err(QuerySessionError::OwnerMismatch);
        }
        if state.global_closing || state.closing.contains(&payload.connection_id) {
            return Err(QuerySessionError::ConnectionClosing);
        }
        if state.sessions.contains_key(&payload.session_id)
            || state.opening.contains_key(&payload.session_id)
        {
            return Err(QuerySessionError::InvalidSequence);
        }
        if state.sessions.len() + state.opening.len() >= MAX_SESSIONS {
            return Err(QuerySessionError::SessionLimitReached {
                limit: "appSessions".into(),
            });
        }
        let count = state
            .sessions
            .values()
            .filter(|session| session.connection_id == payload.connection_id)
            .count()
            + state
                .opening
                .values()
                .filter(|connection_id| *connection_id == &payload.connection_id)
                .count();
        if count >= MAX_SESSIONS_PER_CONNECTION {
            return Err(QuerySessionError::SessionLimitReached {
                limit: "connectionSessions".into(),
            });
        }
        let active_connections = state
            .sessions
            .values()
            .map(|session| &session.connection_id)
            .chain(state.opening.values())
            .chain(state.observers.keys())
            .collect::<HashSet<_>>()
            .len();
        if count == 0 && active_connections >= MAX_ACTIVE_CONNECTIONS {
            return Err(QuerySessionError::SessionLimitReached {
                limit: "activeConnections".into(),
            });
        }
        Ok(())
    }
    async fn bound(&self, id: &str, window: &str) -> Result<Arc<Session>, QuerySessionError> {
        let session = self
            .inner
            .lock()
            .await
            .sessions
            .get(id)
            .cloned()
            .ok_or(QuerySessionError::SessionNotFound)?;
        if session.window_label != window {
            return Err(QuerySessionError::OwnerMismatch);
        }
        *session.last_liveness.lock().await = Instant::now();
        Ok(session)
    }
    pub(crate) async fn connection_id(
        &self,
        id: &str,
        window: &str,
    ) -> Result<String, QuerySessionError> {
        Ok(self.bound(id, window).await?.connection_id.clone())
    }
    pub(crate) async fn execute(
        &self,
        id: &str,
        execution_id: String,
        sql: String,
        window: &str,
        safety: ExecutionSafety<'_>,
    ) -> Result<AcceptedResult, QuerySessionError> {
        let (intent, authorization) =
            assert_statement_policy(&sql, safety.policy, safety.confirmed)?;
        let session = self.bound(id, window).await?;
        let sequence = session.sequence.lock().await;
        if *session.closed.lock().await {
            return Err(QuerySessionError::SessionNotFound);
        }
        let mut credit = session.credit.lock().await;
        if credit.execution_id.is_some() {
            return Err(QuerySessionError::ExecutionInProgress);
        }
        let snapshot = session.transaction.lock().await.clone();
        if snapshot.status == QueryTransactionStatus::Unknown {
            return Err(QuerySessionError::TransactionStateUnknown { can_recheck: true });
        }
        credit.execution_id = Some(execution_id.clone());
        credit.terminal_sequence = None;
        credit.retain_more_rows = true;
        credit.last_ack = Instant::now();
        credit.last_acked_sequence = 0;
        drop(credit);
        drop(sequence);
        tokio::spawn(run_execution(
            self.clone(),
            session,
            execution_id,
            sql,
            snapshot,
            safety.on_success.map(|on_success| ExecutionAdmission {
                on_success,
                intent,
                authorization,
            }),
        ));
        Ok(AcceptedResult { accepted: true })
    }
    pub(crate) async fn ack(
        &self,
        payload: AckPayload,
        window: &str,
    ) -> Result<(), QuerySessionError> {
        let session = self.bound(&payload.session_id, window).await?;
        let current = *session.sequence.lock().await;
        let mut credit = session.credit.lock().await;
        let max_ackable = credit
            .terminal_sequence
            .or_else(|| credit.outstanding.back().map(|(sequence, _)| *sequence));
        if credit.execution_id.as_deref() != Some(&payload.execution_id)
            || payload.ack_through_sequence > current
            || payload.ack_through_sequence <= credit.last_acked_sequence
            || max_ackable.is_none_or(|maximum| payload.ack_through_sequence > maximum)
        {
            return Err(QuerySessionError::InvalidSequence);
        }
        while credit
            .outstanding
            .front()
            .is_some_and(|(sequence, _)| *sequence <= payload.ack_through_sequence)
        {
            credit.outstanding.pop_front();
        }
        credit.retain_more_rows &= payload.retain_more_rows;
        credit.last_ack = Instant::now();
        credit.last_acked_sequence = payload.ack_through_sequence;
        if credit.terminal_sequence == Some(payload.ack_through_sequence) {
            credit.execution_id = None;
            credit.terminal_sequence = None;
        }
        drop(credit);
        session.credit_changed.notify_waiters();
        Ok(())
    }
    pub(crate) async fn heartbeat(
        &self,
        window: &str,
        payload: HeartbeatPayload,
    ) -> Result<HeartbeatResult, QuerySessionError> {
        let state = self.inner.lock().await;
        if state.owners.get(window) != Some(&payload.owner_id) {
            return Err(QuerySessionError::OwnerMismatch);
        }
        let sessions = payload
            .session_ids
            .iter()
            .filter_map(|id| {
                state
                    .sessions
                    .get(id)
                    .filter(|session| {
                        session.window_label == window && session.owner_id == payload.owner_id
                    })
                    .cloned()
            })
            .collect::<Vec<_>>();
        drop(state);
        for session in &sessions {
            *session.last_liveness.lock().await = Instant::now();
        }
        Ok(HeartbeatResult {
            refreshed_session_ids: sessions.iter().map(|session| session.id.clone()).collect(),
        })
    }
    pub(crate) async fn cancel(
        &self,
        payload: ExecutionPayload,
        window: &str,
    ) -> Result<CancelResult, QuerySessionError> {
        let session = self.bound(&payload.session_id, window).await?;
        if session.credit.lock().await.execution_id.as_deref() != Some(&payload.execution_id) {
            return Ok(CancelResult { requested: false });
        }
        let requested = postgres::cancel(session.connection.cancel.clone(), session.tls).await;
        Ok(CancelResult { requested })
    }
    pub(crate) async fn refresh(
        &self,
        id: &str,
        window: &str,
        spec: ResolvedPostgresConnectSpec,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        let session = self.bound(id, window).await?;
        observe_session(&session).await;
        if session.transaction.lock().await.status == QueryTransactionStatus::Unknown {
            let replacement = Observer::connect(&spec).await?;
            *session.observer.lock().await = replacement;
            observe_session(&session).await;
        }
        let snapshot = session.transaction.lock().await.clone();
        Ok(snapshot)
    }
    pub(crate) async fn set_mode(
        &self,
        id: &str,
        window: &str,
        mode: QueryTransactionMode,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        let session = self.bound(id, window).await?;
        let mut snapshot = session.transaction.lock().await;
        ensure_idle(&snapshot, "setMode")?;
        snapshot.mode = mode;
        Ok(snapshot.clone())
    }
    pub(crate) async fn set_isolation(
        &self,
        id: &str,
        window: &str,
        isolation: QueryTransactionIsolation,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        let session = self.bound(id, window).await?;
        let mut snapshot = session.transaction.lock().await;
        ensure_idle(&snapshot, "setIsolation")?;
        snapshot.manual_isolation = isolation;
        Ok(snapshot.clone())
    }
    pub(crate) async fn transaction_action(
        &self,
        id: &str,
        window: &str,
        commit: bool,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        let session = self.bound(id, window).await?;
        let sequence = session.sequence.lock().await;
        if *session.closed.lock().await {
            return Err(QuerySessionError::SessionNotFound);
        }
        let credit = session.credit.lock().await;
        if credit.execution_id.is_some() {
            return Err(QuerySessionError::ExecutionInProgress);
        }
        let status = session.transaction.lock().await.status;
        if (commit && status != QueryTransactionStatus::Active)
            || (!commit && status == QueryTransactionStatus::Idle)
        {
            return Err(invalid(status, if commit { "commit" } else { "rollback" }));
        }
        session
            .connection
            .client
            .batch_execute(if commit { "COMMIT" } else { "ROLLBACK" })
            .await
            .map_err(postgres::database_error)?;
        observe_session(&session).await;
        let snapshot = session.transaction.lock().await.clone();
        drop(credit);
        drop(sequence);
        crate::storage::touch_connection_activity(&self.pool, &session.connection_id)
            .await
            .ok();
        Ok(snapshot)
    }
    pub(crate) async fn close(&self, id: &str, window: &str) -> Result<(), QuerySessionError> {
        self.bound(id, window).await?;
        self.remove_and_close(id, true).await;
        Ok(())
    }
    async fn remove_and_close(&self, id: &str, emit: bool) {
        let session = {
            let mut state = self.inner.lock().await;
            let session = state.sessions.remove(id);
            if let Some(session) = &session {
                if !state
                    .sessions
                    .values()
                    .any(|other| other.connection_id == session.connection_id)
                    && !state
                        .opening
                        .values()
                        .any(|connection_id| connection_id == &session.connection_id)
                {
                    state.observers.remove(&session.connection_id);
                }
            }
            session
        };
        if let Some(session) = session {
            close_session(session, emit).await;
        }
    }
    pub(crate) async fn begin_connection_teardown(&self, connection_id: &str) {
        self.inner.lock().await.closing.insert(connection_id.into());
        self.close_matching(|session| session.connection_id == connection_id)
            .await;
    }
    pub(crate) async fn end_connection_teardown(&self, connection_id: &str) {
        let mut state = self.inner.lock().await;
        state.closing.remove(connection_id);
        *state.generations.entry(connection_id.into()).or_default() += 1;
    }
    pub(crate) async fn close_window(&self, window: &str) {
        self.close_matching(|session| session.window_label == window)
            .await;
    }
    pub(crate) async fn close_all(&self) {
        self.close_matching(|_| true).await;
    }
    pub(crate) async fn begin_global_teardown(&self) {
        self.inner.lock().await.global_closing = true;
        self.close_all().await;
    }
    pub(crate) async fn end_global_teardown(&self) {
        self.inner.lock().await.global_closing = false;
    }
    async fn close_matching(&self, predicate: impl Fn(&Session) -> bool) {
        let sessions = {
            let mut state = self.inner.lock().await;
            let ids = state
                .sessions
                .values()
                .filter(|session| predicate(session))
                .map(|session| session.id.clone())
                .collect::<Vec<_>>();
            let sessions = ids
                .into_iter()
                .filter_map(|id| state.sessions.remove(&id))
                .collect::<Vec<_>>();
            let live = state
                .sessions
                .values()
                .map(|session| session.connection_id.clone())
                .chain(state.opening.values().cloned())
                .collect::<HashSet<_>>();
            state.observers.retain(|id, _| live.contains(id));
            sessions
        };
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            futures_util::future::join_all(
                sessions
                    .into_iter()
                    .map(|session| close_session(session, false)),
            ),
        )
        .await;
    }
    pub(crate) async fn set_focused(&self, window: &str, focused: bool) {
        let sessions = self
            .inner
            .lock()
            .await
            .sessions
            .values()
            .filter(|session| session.window_label == window)
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            *session.focused.lock().await = focused;
            if focused {
                *session.last_liveness.lock().await = Instant::now();
                session.credit.lock().await.last_ack = Instant::now();
            }
        }
    }
}

fn assert_statement_policy(
    sql: &str,
    policy: &crate::safety::policy::ResolvedSafetyPolicy,
    confirmed: bool,
) -> Result<
    (
        crate::safety::policy::WriteIntent,
        crate::safety::policy::SafetyAuthorization,
    ),
    QuerySessionError,
> {
    let intent = crate::safety::policy::WriteIntent::Statement {
        classes: crate::postgres::sql_class::classify_script(sql),
    };
    let authorization = crate::safety::policy::assert_permitted(policy, &intent, confirmed)
        .map_err(|refusal| {
            refusal.fold(
                |reason, _| QuerySessionError::PolicyBlocked {
                    reason: reason.to_string(),
                },
                |statements| QuerySessionError::PolicyNeedsConfirmation { statements },
            )
        })?;
    Ok((intent, authorization))
}

fn release_opening_locked(state: &mut ManagerState, session_id: &str) {
    let Some(connection_id) = state.opening.remove(session_id) else {
        return;
    };
    if !state
        .sessions
        .values()
        .any(|session| session.connection_id == connection_id)
        && !state.opening.values().any(|id| id == &connection_id)
    {
        state.observers.remove(&connection_id);
    }
}

async fn run_execution(
    manager: QuerySessionManager,
    session: Arc<Session>,
    execution_id: String,
    sql: String,
    initial: QueryTransactionSnapshot,
    admission: Option<ExecutionAdmission>,
) {
    if send(
        &session,
        Some(execution_id.clone()),
        false,
        QueryEvent::ExecutionStarted,
    )
    .await
    .is_err()
    {
        manager.remove_and_close(&session.id, false).await;
        return;
    }
    let begin_result = if initial.mode == QueryTransactionMode::Manual
        && initial.status == QueryTransactionStatus::Idle
    {
        let isolation = match initial.manual_isolation {
            QueryTransactionIsolation::ReadCommitted => "READ COMMITTED",
            QueryTransactionIsolation::RepeatableRead => "REPEATABLE READ",
            QueryTransactionIsolation::Serializable => "SERIALIZABLE",
        };
        session
            .connection
            .client
            .batch_execute(&format!("BEGIN ISOLATION LEVEL {isolation}"))
            .await
    } else {
        Ok(())
    };
    let mut terminal = begin_result.err().map(|error| {
        (
            "failed",
            postgres::ExecutionTotals::default(),
            Some(postgres::database_error(error)),
        )
    });
    if terminal.is_none() {
        let mut events = postgres::execute_stream(
            session.connection.client.clone(),
            session.connection.notices.clone(),
            sql,
        );
        while let Some(event) = events.recv().await {
            let delivered = match event {
                postgres::DriverEvent::ResultStarted {
                    result_set_index,
                    columns,
                } => send(
                    &session,
                    Some(execution_id.clone()),
                    false,
                    QueryEvent::ResultSetStarted {
                        result_set_index,
                        columns,
                    },
                )
                .await
                .map(|_| ()),
                postgres::DriverEvent::RowBatch {
                    result_set_index,
                    rows,
                } => send_with_credit(
                    &session,
                    execution_id.clone(),
                    QueryEvent::RowBatch {
                        result_set_index,
                        rows,
                    },
                )
                .await
                .map(|_| ()),
                postgres::DriverEvent::ResultCompleted {
                    result_set_index,
                    row_count,
                } => send(
                    &session,
                    Some(execution_id.clone()),
                    false,
                    QueryEvent::ResultSetCompleted {
                        result_set_index,
                        row_count,
                        partial: false,
                    },
                )
                .await
                .map(|_| ()),
                postgres::DriverEvent::ResultAborted {
                    result_set_index,
                    row_count,
                } => send(
                    &session,
                    Some(execution_id.clone()),
                    false,
                    QueryEvent::ResultSetCompleted {
                        result_set_index,
                        row_count,
                        partial: true,
                    },
                )
                .await
                .map(|_| ()),
                postgres::DriverEvent::Notice(notice) => send(
                    &session,
                    Some(execution_id.clone()),
                    false,
                    QueryEvent::Notice {
                        severity: notice.severity,
                        message: notice.message,
                    },
                )
                .await
                .map(|_| ()),
                postgres::DriverEvent::Finished(result) => {
                    terminal = Some(match result {
                        Ok(totals) => ("completed", totals, None),
                        Err((error, totals)) => ("failed", totals, Some(error)),
                    });
                    break;
                }
            };
            if delivered.is_err() {
                manager.remove_and_close(&session.id, false).await;
                return;
            }
        }
    }
    let (status, totals, error) = terminal.unwrap_or((
        "failed",
        postgres::ExecutionTotals::default(),
        Some(QuerySessionError::ConnectionLost),
    ));
    if execution_lost_connection(error.as_ref()) {
        let _ = send(
            &session,
            None,
            false,
            QueryEvent::SessionLost {
                reason: "connectionLost".into(),
            },
        )
        .await;
        manager.remove_and_close(&session.id, false).await;
        return;
    }
    let omitted_rows = totals.omitted_rows;
    let omitted_result_sets = totals.omitted_result_sets;
    let omitted_metadata_bytes = totals.omitted_metadata_bytes;
    let reasons = totals.truncation_reasons;
    let omitted_notices = totals
        .omitted_notices
        .saturating_add(session.connection.take_dropped_notices());
    observe_session(&session).await;
    let snapshot = session.transaction.lock().await.clone();
    let display_error = error.as_ref().and_then(postgres::display_error);
    if status == "completed" {
        if let Some(admission) = admission {
            (admission.on_success)(admission.intent, admission.authorization).await;
        }
        crate::storage::touch_connection_activity(&manager.pool, &session.connection_id)
            .await
            .ok();
    }
    wait_for_row_credit(&session).await;
    if send_terminal(
        &session,
        execution_id,
        QueryEvent::ExecutionCompleted {
            status: status.into(),
            transaction: snapshot,
            omitted_rows,
            omitted_result_sets,
            omitted_notices,
            omitted_metadata_bytes,
            truncation_reasons: reasons,
            error: display_error,
        },
    )
    .await
    .is_err()
    {
        manager.remove_and_close(&session.id, false).await;
    }
}

fn execution_lost_connection(error: Option<&QuerySessionError>) -> bool {
    matches!(error, Some(QuerySessionError::ConnectionLost))
}

async fn wait_for_row_credit(session: &Session) {
    loop {
        if *session.closed.lock().await || session.credit.lock().await.outstanding.is_empty() {
            return;
        }
        session.credit_changed.notified().await;
    }
}

async fn send_with_credit(
    session: &Session,
    execution_id: String,
    event: QueryEvent,
) -> Result<u64, ()> {
    let bytes = serde_json::to_vec(&event).map_err(|_| ())?.len();
    loop {
        let credit = session.credit.lock().await;
        let used = credit
            .outstanding
            .iter()
            .map(|(_, bytes)| bytes)
            .sum::<usize>();
        if !credit.retain_more_rows {
            return Ok(0);
        }
        if credit.outstanding.len() < 4 && used + bytes <= 4 * 1024 * 1024 {
            break;
        }
        drop(credit);
        session.credit_changed.notified().await;
    }
    let sequence = {
        let mut sequence = session.sequence.lock().await;
        if *session.closed.lock().await {
            return Err(());
        }
        *sequence += 1;
        let current = *sequence;
        session
            .credit
            .lock()
            .await
            .outstanding
            .push_back((current, bytes));
        if session
            .channel
            .send(QueryEventEnvelope {
                session_id: session.id.clone(),
                tab_id: session.tab_id.clone(),
                connection_id: session.connection_id.clone(),
                generation: session.generation,
                sequence: current,
                execution_id: Some(execution_id),
                requires_ack: true,
                event,
            })
            .is_err()
        {
            session
                .credit
                .lock()
                .await
                .outstanding
                .retain(|(queued, _)| *queued != current);
            return Err(());
        }
        current
    };
    Ok(sequence)
}
async fn send_terminal(
    session: &Session,
    execution_id: String,
    event: QueryEvent,
) -> Result<u64, ()> {
    let sequence = {
        let mut sequence = session.sequence.lock().await;
        if *session.closed.lock().await {
            return Err(());
        }
        *sequence += 1;
        let current = *sequence;
        session.credit.lock().await.terminal_sequence = Some(current);
        if session
            .channel
            .send(QueryEventEnvelope {
                session_id: session.id.clone(),
                tab_id: session.tab_id.clone(),
                connection_id: session.connection_id.clone(),
                generation: session.generation,
                sequence: current,
                execution_id: Some(execution_id),
                requires_ack: true,
                event,
            })
            .is_err()
        {
            session.credit.lock().await.terminal_sequence = None;
            return Err(());
        }
        current
    };
    Ok(sequence)
}
async fn send(
    session: &Session,
    execution_id: Option<String>,
    requires_ack: bool,
    event: QueryEvent,
) -> Result<u64, ()> {
    let mut sequence = session.sequence.lock().await;
    if *session.closed.lock().await {
        return Err(());
    }
    *sequence += 1;
    let current = *sequence;
    session
        .channel
        .send(QueryEventEnvelope {
            session_id: session.id.clone(),
            tab_id: session.tab_id.clone(),
            connection_id: session.connection_id.clone(),
            generation: session.generation,
            sequence: current,
            execution_id,
            requires_ack,
            event,
        })
        .map_err(|_| ())?;
    Ok(current)
}
async fn observe_session(session: &Session) {
    let observer = session.observer.lock().await.clone();
    let status = observer
        .observe(
            session.connection.pid,
            session.connection.backend_start.clone(),
        )
        .await;
    session.transaction.lock().await.status = status;
}
async fn close_session(session: Arc<Session>, emit: bool) {
    {
        // Serialize the closed transition with event sequence assignment so an execution event
        // can never be delivered after SessionClosed.
        let _sequence = session.sequence.lock().await;
        let mut closed = session.closed.lock().await;
        if *closed {
            return;
        }
        *closed = true;
    }
    session.credit_changed.notify_waiters();
    if session.credit.lock().await.execution_id.is_some() {
        let _ = postgres::cancel(session.connection.cancel.clone(), session.tls).await;
    }
    if session.transaction.lock().await.status != QueryTransactionStatus::Idle {
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            session.connection.client.batch_execute("ROLLBACK"),
        )
        .await;
    }
    if emit {
        let mut sequence = session.sequence.lock().await;
        *sequence += 1;
        let _ = session.channel.send(QueryEventEnvelope {
            session_id: session.id.clone(),
            tab_id: session.tab_id.clone(),
            connection_id: session.connection_id.clone(),
            generation: session.generation,
            sequence: *sequence,
            execution_id: None,
            requires_ack: false,
            event: QueryEvent::SessionClosed,
        });
    }
}
fn ensure_idle(snapshot: &QueryTransactionSnapshot, action: &str) -> Result<(), QuerySessionError> {
    if snapshot.status == QueryTransactionStatus::Idle {
        Ok(())
    } else {
        Err(invalid(snapshot.status, action))
    }
}
fn invalid(status: QueryTransactionStatus, action: &str) -> QuerySessionError {
    QuerySessionError::InvalidTransactionTransition {
        status,
        attempted_action: action.into(),
        allowed_actions: if status == QueryTransactionStatus::Unknown {
            vec!["recheck".into(), "rollback".into(), "close".into()]
        } else {
            vec!["execute".into(), "close".into()]
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> QuerySessionManager {
        QuerySessionManager::new(
            sqlx::sqlite::SqlitePoolOptions::new()
                .connect_lazy("sqlite::memory:")
                .expect("in-memory pool URL should be valid"),
        )
    }

    fn payload(session: &str, connection: &str) -> OpenSessionPayload {
        OpenSessionPayload {
            owner_id: "owner".into(),
            session_id: session.into(),
            tab_id: "tab".into(),
            connection_id: connection.into(),
        }
    }

    #[test]
    fn admission_constants_match_contract() {
        assert_eq!(
            (
                MAX_SESSIONS_PER_CONNECTION,
                MAX_ACTIVE_CONNECTIONS,
                MAX_SESSIONS
            ),
            (7, 8, 24)
        );
    }

    #[tokio::test]
    async fn opening_sessions_are_reserved_for_admission() {
        let manager = manager();
        let mut state = ManagerState::default();
        state.owners.insert("window".into(), "owner".into());
        for index in 0..MAX_SESSIONS_PER_CONNECTION {
            state
                .opening
                .insert(format!("session-{index}"), "connection".into());
        }

        assert!(matches!(
            manager.check_admission(&state, "window", &payload("next", "connection")),
            Err(QuerySessionError::SessionLimitReached { limit })
                if limit == "connectionSessions"
        ));
    }

    #[tokio::test]
    async fn opening_connections_are_reserved_for_admission() {
        let manager = manager();
        let mut state = ManagerState::default();
        state.owners.insert("window".into(), "owner".into());
        for index in 0..MAX_ACTIVE_CONNECTIONS {
            state
                .opening
                .insert(format!("session-{index}"), format!("connection-{index}"));
        }

        assert!(matches!(
            manager.check_admission(&state, "window", &payload("next", "new-connection")),
            Err(QuerySessionError::SessionLimitReached { limit })
                if limit == "activeConnections"
        ));
    }

    #[test]
    fn connection_loss_retires_the_session() {
        assert!(execution_lost_connection(Some(
            &QuerySessionError::ConnectionLost
        )));
        assert!(!execution_lost_connection(Some(
            &QuerySessionError::TransactionStateUnknown { can_recheck: true }
        )));
        assert!(!execution_lost_connection(None));
    }

    #[tokio::test]
    async fn policy_is_asserted_before_session_lookup() {
        let manager = manager();
        let policy = crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
            environment: crate::Environment::Production,
            safe_mode: crate::SafeMode::Inherit,
            read_only: false,
        });

        assert!(matches!(
            manager
                .execute(
                    "missing-session",
                    "execution".into(),
                    "DELETE FROM users WHERE id = 1".into(),
                    "window",
                    ExecutionSafety {
                        policy: &policy,
                        confirmed: false,
                        on_success: None,
                    },
                )
                .await,
            Err(QuerySessionError::PolicyNeedsConfirmation { statements })
                if statements.len() == 1
        ));

        assert!(matches!(
            manager
                .execute(
                    "missing-session",
                    "execution".into(),
                    "DELETE FROM users WHERE id = 1".into(),
                    "window",
                    ExecutionSafety {
                        policy: &policy,
                        confirmed: true,
                        on_success: None,
                    },
                )
                .await,
            Err(QuerySessionError::SessionNotFound)
        ));
    }

    #[test]
    fn read_only_statement_admission_only_accepts_reads() {
        let policy = crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
            environment: crate::Environment::Development,
            safe_mode: crate::SafeMode::Inherit,
            read_only: true,
        });
        assert!(assert_statement_policy("SELECT 1", &policy, false).is_ok());
        assert!(matches!(
            assert_statement_policy("SET search_path = public", &policy, true),
            Err(QuerySessionError::PolicyBlocked { .. })
        ));
    }
}
