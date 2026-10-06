pub(crate) mod observer;
pub(crate) mod postgres;
pub(crate) mod protocol;
pub(crate) mod service;

use futures_util::future::BoxFuture;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex, Notify};

use crate::host::SharedSink;
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::sql_params::{plan_execution, ExecutionPlan, ParameterValue};
use observer::Observer;
use protocol::*;

const MAX_SESSIONS_PER_CONNECTION: usize = 7;
const MAX_ACTIVE_CONNECTIONS: usize = 8;
const MAX_SESSIONS: usize = 24;
const LEASE: Duration = Duration::from_secs(120);

/// SQLSTATE `query_canceled`: raised for a cancel request and for a statement
/// timeout alike.
const QUERY_CANCELED: &str = "57014";

struct Credit {
    outstanding: VecDeque<(u64, usize)>,
    execution_id: Option<String>,
    terminal_sequence: Option<u64>,
    retain_more_rows: bool,
    cancel_requested: bool,
    /// Set once the execution starts its own cleanup statements.
    cleanup_started: bool,
    last_ack: Instant,
    last_acked_sequence: u64,
}
impl Credit {
    fn begin(&mut self, execution_id: String) {
        self.execution_id = Some(execution_id);
        self.terminal_sequence = None;
        self.retain_more_rows = true;
        self.cancel_requested = false;
        self.cleanup_started = false;
        self.last_ack = Instant::now();
        self.last_acked_sequence = 0;
    }
}
enum CreditGate<'a, T> {
    Ready(T),
    Closed,
    Wait(tokio::sync::futures::Notified<'a>),
}
/// Event delivery for one session: sequence assignment, the credit window,
/// and the closed flag. It owns no database handle.
struct Outbox {
    id: String,
    tab_id: String,
    connection_id: String,
    generation: u64,
    sink: SharedSink<QueryEventEnvelope>,
    sequence: Mutex<u64>,
    credit: Mutex<Credit>,
    credit_changed: Notify,
    closed: Mutex<bool>,
}
impl Outbox {
    fn new(
        id: String,
        tab_id: String,
        connection_id: String,
        generation: u64,
        sink: SharedSink<QueryEventEnvelope>,
    ) -> Self {
        Self {
            id,
            tab_id,
            connection_id,
            generation,
            sink,
            sequence: Mutex::new(0),
            credit: Mutex::new(Credit {
                outstanding: VecDeque::new(),
                execution_id: None,
                terminal_sequence: None,
                retain_more_rows: true,
                cancel_requested: false,
                cleanup_started: false,
                last_ack: Instant::now(),
                last_acked_sequence: 0,
            }),
            credit_changed: Notify::new(),
            closed: Mutex::new(false),
        }
    }
    /// One pass of a wait on the credit window. The wakeup is armed before the
    /// closed flag and the window are read: `notify_waiters` reaches a
    /// `Notified` from the moment it is created, so a close or an ACK that
    /// lands after the read still completes the returned wait.
    async fn credit_gate<T>(&self, ready: impl FnOnce(&Credit) -> Option<T>) -> CreditGate<'_, T> {
        let changed = self.credit_changed.notified();
        if *self.closed.lock().await {
            return CreditGate::Closed;
        }
        match ready(&*self.credit.lock().await) {
            Some(value) => CreditGate::Ready(value),
            None => CreditGate::Wait(changed),
        }
    }
    async fn acknowledge(&self, payload: &AckPayload) -> Result<(), QuerySessionError> {
        let current = *self.sequence.lock().await;
        let mut credit = self.credit.lock().await;
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
        self.credit_changed.notify_waiters();
        Ok(())
    }
    /// Records that a Stop was requested for the running execution. A stale
    /// execution id leaves the flag alone, and so does an execution already
    /// in its own cleanup: a cancel request sent then could only hit the
    /// cleanup statements.
    async fn request_cancel(&self, execution_id: &str) -> bool {
        let mut credit = self.credit.lock().await;
        if credit.execution_id.as_deref() != Some(execution_id) || credit.cleanup_started {
            return false;
        }
        credit.cancel_requested = true;
        true
    }
    async fn checkpoint(&self) -> postgres::Checkpoint {
        if *self.closed.lock().await {
            postgres::Checkpoint::Closed
        } else if self.credit.lock().await.cancel_requested {
            postgres::Checkpoint::Stop
        } else {
            postgres::Checkpoint::Proceed
        }
    }
    /// The permit holds the closed flag's lock, so `mark_closed` waits for
    /// cleanup to finish and nothing is sent to the server after a close.
    async fn begin_cleanup(&self) -> Option<postgres::CleanupPermit<'_>> {
        let open = self.closed.lock().await;
        if *open {
            return None;
        }
        let stop_requested = {
            let mut credit = self.credit.lock().await;
            credit.cleanup_started = true;
            credit.cancel_requested
        };
        Some(postgres::CleanupPermit {
            stop_requested,
            _open: Box::new(open),
        })
    }
    /// Returns false when the session was already closed.
    async fn mark_closed(&self) -> bool {
        {
            // Serialize the closed transition with event sequence assignment so an execution
            // event can never be delivered after SessionClosed.
            let _sequence = self.sequence.lock().await;
            let mut closed = self.closed.lock().await;
            if *closed {
                return false;
            }
            *closed = true;
        }
        self.credit_changed.notify_waiters();
        true
    }
}
struct Session {
    #[cfg(feature = "isolated-profile")]
    instance: uuid::Uuid,
    outbox: Outbox,
    owner_id: String,
    window_label: String,
    tls: crate::postgres::dedicated::TlsConfig,
    connection: Arc<postgres::SessionConnection>,
    observer: Arc<Mutex<Arc<Observer>>>,
    transaction: Mutex<QueryTransactionSnapshot>,
    probes: ProbeOrder,
    last_liveness: Mutex<Instant>,
    focused: Mutex<bool>,
    native_tasks: Option<crate::postgres::dedicated::DriverJoins>,
}
impl std::ops::Deref for Session {
    type Target = Outbox;
    fn deref(&self) -> &Self::Target {
        &self.outbox
    }
}
impl postgres::ExecutionControl for Session {
    fn checkpoint(&self) -> BoxFuture<'_, postgres::Checkpoint> {
        Box::pin(self.outbox.checkpoint())
    }
    fn begin_cleanup(&self) -> BoxFuture<'_, Option<postgres::CleanupPermit<'_>>> {
        Box::pin(self.outbox.begin_cleanup())
    }
}
/// Orders observer probes by when they started. A probe that started before
/// an execution finished can return after the execution's own probe; applying
/// it would cache a stale `Idle`, and a cursor read would then wrap, and
/// commit, a transaction the user opened.
#[derive(Default)]
struct ProbeOrder {
    started: AtomicU64,
    applied: AtomicU64,
}
impl ProbeOrder {
    fn start(&self) -> u64 {
        self.started.fetch_add(1, Ordering::SeqCst) + 1
    }
    /// True when no probe that started later has been applied.
    fn admit(&self, probe: u64) -> bool {
        self.applied.fetch_max(probe, Ordering::SeqCst) < probe
    }
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
    native_sessions: HashMap<String, NativeSessionTasks>,
    native_observers: HashMap<String, Vec<crate::postgres::dedicated::DriverJoins>>,
}
struct NativeSessionTasks {
    #[cfg(feature = "isolated-profile")]
    window: String,
    connection_id: String,
    tasks: crate::postgres::dedicated::DriverJoins,
}
#[derive(Clone)]
pub(crate) struct QuerySessionManager {
    inner: Arc<Mutex<ManagerState>>,
    pool: SqlitePool,
    native_tasks: Option<crate::postgres::dedicated::DriverJoins>,
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

pub(crate) struct ExecutionRequest {
    pub execution_id: String,
    pub sql: String,
    /// `Some`, even when empty, puts the execution in parameter mode.
    pub parameters: Option<Vec<ParameterValue>>,
    pub row_limit: Option<i64>,
}

pub(crate) struct ExecutionSafety<'a> {
    pub policy: &'a crate::safety::policy::ResolvedSafetyPolicy,
    pub confirmed: bool,
    pub on_success: Option<ExecutionSuccessHook>,
}

impl QuerySessionManager {
    fn native_child(&self) -> Option<crate::postgres::dedicated::DriverJoins> {
        #[cfg(feature = "isolated-profile")]
        {
            self.native_tasks
                .as_ref()
                .map(crate::postgres::dedicated::DriverJoins::child)
        }
        #[cfg(not(feature = "isolated-profile"))]
        {
            None
        }
    }
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManagerState::default())),
            pool,
            native_tasks: None,
        }
    }
    #[cfg(feature = "isolated-profile")]
    pub(crate) fn with_native_tasks(
        mut self,
        tasks: crate::postgres::dedicated::DriverJoins,
    ) -> Self {
        self.native_tasks = Some(tasks);
        self
    }
    pub(crate) fn start_monitor(&self, runtime: &tokio::runtime::Handle) {
        self.spawn_monitor(runtime);
    }
    pub(crate) fn spawn_monitor(
        &self,
        runtime: &tokio::runtime::Handle,
    ) -> tokio::task::JoinHandle<()> {
        let manager = self.clone();
        runtime.spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            loop {
                tick.tick().await;
                manager.expire_stalled().await;
            }
        })
    }
    /// Moves whenever a teardown of the connection completes. Native open
    /// reads it before connecting and refuses to finish if it moved, so a
    /// disconnect that starts and ends during the connect still wins.
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn connection_generation(&self, connection_id: &str) -> u64 {
        self.inner
            .lock()
            .await
            .generations
            .get(connection_id)
            .copied()
            .unwrap_or_default()
    }
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn session_alive(
        &self,
        window: &str,
        id: &str,
    ) -> Result<bool, QuerySessionError> {
        Ok(!self.bound(id, window).await?.connection.is_closed())
    }
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn retire_window(&self, window: &str) {
        self.inner.lock().await.owners.remove(window);
        self.close_window(window).await;
        let ids = self
            .inner
            .lock()
            .await
            .native_sessions
            .iter()
            .filter(|(_, entry)| entry.window == window)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.join_native_sessions(&ids).await;
    }
    /// Called only after native opens/workers have been joined or aborted.
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn clear_native_reservations(&self) {
        let mut state = self.inner.lock().await;
        state.owners.clear();
        state.opening.clear();
        state.observer_opening.clear();
        state.observers.clear();
        state.native_sessions.clear();
        state.native_observers.clear();
        state.sessions.clear();
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
        sink: SharedSink<QueryEventEnvelope>,
        spec: ResolvedPostgresConnectSpec,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        let session_tasks = self.native_child();
        {
            let mut state = self.inner.lock().await;
            self.check_admission(&state, window, &payload)?;
            state
                .opening
                .insert(payload.session_id.clone(), payload.connection_id.clone());
            if let Some(tasks) = &session_tasks {
                state.native_sessions.insert(
                    payload.session_id.clone(),
                    NativeSessionTasks {
                        #[cfg(feature = "isolated-profile")]
                        window: window.into(),
                        connection_id: payload.connection_id.clone(),
                        tasks: tasks.clone(),
                    },
                );
            }
        }
        let observer = match self.observer_for_open(&payload.connection_id, &spec).await {
            Ok(observer) => observer,
            Err(error) => {
                self.release_opening(&payload.session_id).await;
                return Err(error);
            }
        };
        let connection = match postgres::connect_tracked(&spec, session_tasks.as_ref()).await {
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
                #[cfg(feature = "isolated-profile")]
                instance: uuid::Uuid::new_v4(),
                outbox: Outbox::new(
                    payload.session_id.clone(),
                    payload.tab_id,
                    payload.connection_id,
                    generation,
                    sink,
                ),
                owner_id: payload.owner_id,
                window_label: window.into(),
                tls: connection.tls.clone(),
                connection: Arc::new(connection),
                observer,
                transaction: Mutex::new(QueryTransactionSnapshot::default()),
                probes: ProbeOrder::default(),
                last_liveness: Mutex::new(Instant::now()),
                focused: Mutex::new(true),
                native_tasks: session_tasks,
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

            let observer_tasks = self.native_child();
            if let Some(tasks) = &observer_tasks {
                self.inner
                    .lock()
                    .await
                    .native_observers
                    .entry(connection_id.into())
                    .or_default()
                    .push(tasks.clone());
            }
            let result = Observer::connect_tracked(spec, observer_tasks.as_ref())
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
            || state.native_sessions.contains_key(&payload.session_id)
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
    /// Native confirmation binds to this particular session, not a reusable
    /// caller-supplied ID or the connection's shared event generation.
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn instance(
        &self,
        id: &str,
        window: &str,
    ) -> Result<uuid::Uuid, QuerySessionError> {
        Ok(self.bound(id, window).await?.instance)
    }
    pub(crate) async fn execute(
        &self,
        id: &str,
        request: ExecutionRequest,
        window: &str,
        safety: ExecutionSafety<'_>,
    ) -> Result<AcceptedResult, QuerySessionError> {
        // Pure validation first, so nobody is asked to confirm a write that
        // is then refused for a missing value.
        let plan = plan_execution(
            request.sql,
            request.parameters.as_deref(),
            request.row_limit,
        )?;
        let execution_id = request.execution_id;
        let (intent, authorization) =
            assert_statement_policy(plan.policy_sql(), safety.policy, safety.confirmed)?;
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
        credit.begin(execution_id.clone());
        drop(credit);
        drop(sequence);
        let tracked = session.native_tasks.clone();
        let task = tokio::spawn(run_execution(
            self.clone(),
            session,
            execution_id,
            plan,
            snapshot,
            safety.on_success.map(|on_success| ExecutionAdmission {
                on_success,
                intent,
                authorization,
            }),
        ));
        if let Some(tracked) = &tracked {
            tracked.track_task(task);
        }
        Ok(AcceptedResult { accepted: true })
    }
    pub(crate) async fn ack(
        &self,
        payload: AckPayload,
        window: &str,
    ) -> Result<(), QuerySessionError> {
        let session = self.bound(&payload.session_id, window).await?;
        session.acknowledge(&payload).await
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
        // The flag is set before the request is sent, so the 57014 it provokes
        // can never be read as a timeout.
        if !session.request_cancel(&payload.execution_id).await {
            return Ok(CancelResult { requested: false });
        }
        let requested =
            postgres::cancel(session.connection.cancel.clone(), session.tls.clone()).await;
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
            let observer_tasks = self.native_child();
            if let Some(tasks) = &observer_tasks {
                self.inner
                    .lock()
                    .await
                    .native_observers
                    .entry(session.connection_id.clone())
                    .or_default()
                    .push(tasks.clone());
            }
            let replacement = Observer::connect_tracked(&spec, observer_tasks.as_ref()).await?;
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

    /// The native facade holds startup admission while closing. Records outlive
    /// failed opens and monitor removals so every socket and query worker can be
    /// joined even after its session has disappeared from the live map.
    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn close_native(
        &self,
        id: &str,
        window: &str,
    ) -> Result<(), QuerySessionError> {
        if self
            .inner
            .lock()
            .await
            .native_sessions
            .get(id)
            .is_some_and(|entry| entry.window != window)
        {
            return Err(QuerySessionError::OwnerMismatch);
        }
        match self.close(id, window).await {
            Ok(()) | Err(QuerySessionError::SessionNotFound) => {}
            Err(error) => return Err(error),
        }
        self.join_native_sessions(&[id.to_string()]).await;
        Ok(())
    }

    async fn join_native_sessions(&self, ids: &[String]) {
        let groups = {
            let mut state = self.inner.lock().await;
            let retired = ids
                .iter()
                .filter_map(|id| state.native_sessions.remove(id))
                .collect::<Vec<_>>();
            let connections = retired
                .iter()
                .map(|entry| entry.connection_id.clone())
                .collect::<HashSet<_>>();
            let mut groups = retired
                .into_iter()
                .map(|entry| entry.tasks)
                .collect::<Vec<_>>();
            for connection in connections {
                let in_use = state
                    .native_sessions
                    .values()
                    .any(|entry| entry.connection_id == connection)
                    || state.opening.values().any(|id| id == &connection);
                if !in_use {
                    state.observers.remove(&connection);
                    groups.extend(
                        state
                            .native_observers
                            .remove(&connection)
                            .into_iter()
                            .flatten(),
                    );
                }
            }
            groups
        };
        // Abort is scoped to these retired sessions. A late query task registered
        // after admission observes the child's latched abort; other tabs survive.
        for group in &groups {
            group.abort_all();
        }
        futures_util::future::join_all(groups.iter().map(|group| group.drain())).await;
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
        let native_ids = sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        let close = futures_util::future::join_all(
            sessions
                .into_iter()
                .map(|session| close_session(session, false)),
        );
        if self.native_tasks.is_some() {
            close.await;
            self.join_native_sessions(&native_ids).await;
        } else {
            let _ = tokio::time::timeout(Duration::from_secs(3), close).await;
        }
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
    plan: ExecutionPlan,
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
    // A cursor read only gets its own wrapper transaction from a cached
    // `Idle`; every other status runs inside whatever the session holds.
    let entry = match (initial.mode, initial.status) {
        (QueryTransactionMode::Autocommit, QueryTransactionStatus::Idle) => {
            postgres::TransactionEntry::Autocommit
        }
        (QueryTransactionMode::Manual, QueryTransactionStatus::Idle) => {
            let isolation = match initial.manual_isolation {
                QueryTransactionIsolation::ReadCommitted => "READ COMMITTED",
                QueryTransactionIsolation::RepeatableRead => "REPEATABLE READ",
                QueryTransactionIsolation::Serializable => "SERIALIZABLE",
            };
            postgres::TransactionEntry::Begin(format!("BEGIN ISOLATION LEVEL {isolation}"))
        }
        _ => postgres::TransactionEntry::Inside,
    };
    let autocommit = entry == postgres::TransactionEntry::Autocommit;
    // Every earlier response has been received, so this includes all
    // ParameterStatus values reported before the statement is sent.
    let reported_before = session.connection.reported.settled().await;
    let mut events = postgres::execute_plan_tracked(
        session.connection.client.clone(),
        session.connection.notices.clone(),
        plan,
        entry,
        session.clone(),
        session.native_tasks.as_ref(),
    );
    let mut terminal = None;
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
                limit,
            } => send(
                &session,
                Some(execution_id.clone()),
                false,
                QueryEvent::ResultSetCompleted {
                    result_set_index,
                    row_count,
                    partial: false,
                    limit,
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
                    limit: None,
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
            postgres::DriverEvent::Finished { totals, outcome } => {
                terminal = Some((totals, outcome));
                break;
            }
        };
        if delivered.is_err() {
            manager.remove_and_close(&session.id, false).await;
            return;
        }
    }
    let (totals, outcome) = terminal.unwrap_or((
        postgres::ExecutionTotals::default(),
        postgres::Outcome::Failed(QuerySessionError::ConnectionLost),
    ));
    // Sampled as soon as the outcome is known: a Stop that arrives during the
    // observer probe below did not cause this outcome.
    let cancel_requested = session.credit.lock().await.cancel_requested;
    let context = if matches!(outcome, postgres::Outcome::Completed) {
        execution_context(
            reported_before.as_ref(),
            session.connection.reported.settled().await.as_ref(),
            autocommit,
        )
    } else {
        None
    };
    let (status, error, refusal) = match outcome {
        postgres::Outcome::Abandoned => {
            manager.remove_and_close(&session.id, false).await;
            return;
        }
        postgres::Outcome::Completed => ("completed", None, None),
        postgres::Outcome::Stopped => ("cancelled", None, None),
        postgres::Outcome::Refused(refusal) => ("failed", None, Some(refusal)),
        postgres::Outcome::Failed(error) => {
            (failed_status(&error, cancel_requested), Some(error), None)
        }
    };
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
            refusal: refusal.map(Into::into),
            context,
        },
    )
    .await
    .is_err()
    {
        manager.remove_and_close(&session.id, false).await;
    }
}

/// A context is reported only when the same reported values applied before
/// submission and after the terminal response: any ParameterStatus in between
/// (for example `SELECT set_config(...)`) withdraws it.
fn execution_context(
    before: Option<&crate::postgres::dedicated::reported::ReportedSnapshot>,
    after: Option<&crate::postgres::dedicated::reported::ReportedSnapshot>,
    autocommit: bool,
) -> Option<Box<QueryExecutionContext>> {
    let (before, after) = (before?, after?);
    if before != after {
        return None;
    }
    let value = |name: &str| after.value(name).map(str::to_owned);
    Some(Box::new(QueryExecutionContext {
        client_encoding: value("client_encoding"),
        date_style: value("DateStyle"),
        interval_style: value("IntervalStyle"),
        search_path: value("search_path"),
        autocommit,
    }))
}

#[cfg(test)]
mod execution_context_tests {
    use super::execution_context;
    use crate::postgres::dedicated::reported::ReportedSnapshot;

    fn snapshot(generation: u64, encoding: &str, path: Option<&str>) -> ReportedSnapshot {
        ReportedSnapshot {
            generation,
            values: [
                Some(encoding.into()),
                Some("ISO, MDY".into()),
                Some("postgres".into()),
                path.map(str::to_owned),
            ],
        }
    }

    #[test]
    fn stable_reported_values_become_the_execution_context() {
        let before = snapshot(3, "UTF8", Some("\"$user\", public"));
        let context = execution_context(Some(&before), Some(&before.clone()), true).unwrap();
        assert!(context.utf8());
        assert!(context.iso_dates());
        assert_eq!(context.name_resolution_path(), Some("\"$user\", public"));
        let manual = execution_context(Some(&before), Some(&before), false).unwrap();
        assert_eq!(manual.name_resolution_path(), None);
        let unreported = snapshot(3, "UTF8", None);
        assert_eq!(
            execution_context(Some(&unreported), Some(&unreported), true)
                .unwrap()
                .name_resolution_path(),
            None
        );
    }

    #[test]
    fn any_report_during_the_execution_or_an_unsettled_driver_withdraws_it() {
        let before = snapshot(3, "UTF8", Some("public"));
        // set_config inside the statement reports a change, even one that is
        // later reverted to the same value (the generation still moves).
        for after in [
            snapshot(4, "LATIN1", Some("public")),
            snapshot(5, "UTF8", Some("public")),
        ] {
            assert!(execution_context(Some(&before), Some(&after), true).is_none());
        }
        assert!(execution_context(None, Some(&before), true).is_none());
        assert!(execution_context(Some(&before), None, true).is_none());
    }
}

fn execution_lost_connection(error: Option<&QuerySessionError>) -> bool {
    matches!(error, Some(QuerySessionError::ConnectionLost))
}

/// A 57014 is `cancelled` only when a Stop was requested for this execution;
/// a statement timeout raises the same SQLSTATE and stays `failed`.
fn failed_status(error: &QuerySessionError, cancel_requested: bool) -> &'static str {
    match error {
        QuerySessionError::Database {
            code: Some(code), ..
        } if cancel_requested && code == QUERY_CANCELED => "cancelled",
        _ => "failed",
    }
}

async fn wait_for_row_credit(session: &Outbox) {
    loop {
        let gate = session
            .credit_gate(|credit| credit.outstanding.is_empty().then_some(()))
            .await;
        match gate {
            CreditGate::Wait(changed) => changed.await,
            CreditGate::Ready(()) | CreditGate::Closed => return,
        }
    }
}

async fn send_with_credit(
    session: &Outbox,
    execution_id: String,
    event: QueryEvent,
) -> Result<u64, ()> {
    let bytes = serde_json::to_vec(&event).map_err(|_| ())?.len();
    loop {
        // `Some(false)` means the frontend stopped retaining rows.
        let gate = session
            .credit_gate(|credit| {
                if !credit.retain_more_rows {
                    return Some(false);
                }
                let used = credit
                    .outstanding
                    .iter()
                    .map(|(_, bytes)| bytes)
                    .sum::<usize>();
                (credit.outstanding.len() < 4 && used + bytes <= 4 * 1024 * 1024).then_some(true)
            })
            .await;
        match gate {
            CreditGate::Ready(true) => break,
            CreditGate::Ready(false) => return Ok(0),
            CreditGate::Closed => return Err(()),
            CreditGate::Wait(changed) => changed.await,
        }
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
            .sink
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
    session: &Outbox,
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
            .sink
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
    session: &Outbox,
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
        .sink
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
    let probe = session.probes.start();
    let observer = session.observer.lock().await.clone();
    let status = observer
        .observe(
            session.connection.pid,
            session.connection.backend_start.clone(),
        )
        .await;
    let mut transaction = session.transaction.lock().await;
    if session.probes.admit(probe) {
        transaction.status = status;
    }
}
async fn close_session(session: Arc<Session>, emit: bool) {
    if !session.mark_closed().await {
        return;
    }
    if session.credit.lock().await.execution_id.is_some() {
        let _ = postgres::cancel(session.connection.cancel.clone(), session.tls.clone()).await;
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
        let _ = session.sink.send(QueryEventEnvelope {
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

    fn request(sql: &str) -> ExecutionRequest {
        ExecutionRequest {
            execution_id: "execution".into(),
            sql: sql.into(),
            parameters: None,
            row_limit: None,
        }
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
                    request("DELETE FROM users WHERE id = 1"),
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
                    request("DELETE FROM users WHERE id = 1"),
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

    #[test]
    fn start_monitor_is_callable_outside_a_tokio_runtime() {
        // Regression: a host's setup calls this on its main thread with no
        // Tokio runtime context; a bare tokio::spawn panics with "no reactor
        // running" and aborts app startup. The pool mirrors the real call
        // site (created under the host's block_on), so build it inside the
        // runtime and leave that context before the call.
        let runtime = crate::host::test_runtime();
        let manager = runtime.block_on(async { manager() });
        assert!(tokio::runtime::Handle::try_current().is_err());
        manager.start_monitor(runtime.handle());
    }

    fn outbox() -> (Arc<Outbox>, Events) {
        let (sink, events) = recording_sink();
        let outbox = Outbox::new("session".into(), "tab".into(), "connection".into(), 0, sink);
        (Arc::new(outbox), events)
    }

    fn batch() -> QueryEvent {
        QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![vec![Some("value".into())]],
        }
    }

    fn ack(sequence: u64) -> AckPayload {
        AckPayload {
            session_id: "session".into(),
            execution_id: "execution".into(),
            ack_through_sequence: sequence,
            retain_more_rows: true,
        }
    }

    fn database_error(code: &str) -> QuerySessionError {
        QuerySessionError::Database {
            code: Some(code.into()),
            message: "message".into(),
            severity: Some("ERROR".into()),
            position: None,
        }
    }

    #[tokio::test]
    async fn closing_with_a_full_credit_window_ends_the_parked_execution() {
        let (outbox, events) = outbox();
        outbox.credit.lock().await.begin("execution".into());
        let sender = outbox.clone();
        let execution = tokio::spawn(async move {
            for _ in 0..5 {
                send_with_credit(&sender, "execution".into(), batch()).await?;
            }
            Ok::<(), ()>(())
        });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert_eq!(kinds(&events, "rowBatch").len(), 4);
        assert!(!execution.is_finished());

        assert!(outbox.mark_closed().await);

        let ended = tokio::time::timeout(Duration::from_secs(1), execution)
            .await
            .expect("the parked execution ends once the session closes");
        assert_eq!(ended.unwrap(), Err(()));
        // The execution held the only other reference. With it released the
        // session can drop, and its socket with it.
        assert_eq!(Arc::strong_count(&outbox), 1);
        assert_eq!(kinds(&events, "rowBatch").len(), 4);
        assert!(!outbox.mark_closed().await);
    }

    #[tokio::test]
    async fn an_ack_between_the_credit_check_and_the_wait_is_not_missed() {
        let (outbox, _events) = outbox();
        outbox.credit.lock().await.begin("execution".into());
        let sequence = send_with_credit(&outbox, "execution".into(), batch())
            .await
            .unwrap();

        // The wakeup fires while the window is being read, the earliest an ACK
        // can land after the check. A wait armed after the read never sees it.
        let gate = outbox
            .credit_gate(|credit| {
                outbox.credit_changed.notify_waiters();
                credit.outstanding.is_empty().then_some(())
            })
            .await;
        let CreditGate::Wait(changed) = gate else {
            panic!("one batch is still outstanding");
        };
        tokio::time::timeout(Duration::from_secs(1), changed)
            .await
            .expect("a wakeup during the read completes the wait");

        // The same holds for a real ACK and a real close after the check.
        let gate = outbox
            .credit_gate(|credit| credit.outstanding.is_empty().then_some(()))
            .await;
        let CreditGate::Wait(changed) = gate else {
            panic!("one batch is still outstanding");
        };
        outbox.acknowledge(&ack(sequence)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), changed)
            .await
            .expect("the ACK completes the wait");

        let CreditGate::Wait(changed) = outbox.credit_gate(|_| None::<()>).await else {
            panic!("nothing is ready");
        };
        assert!(outbox.mark_closed().await);
        tokio::time::timeout(Duration::from_secs(1), changed)
            .await
            .expect("the close completes the wait");
        assert!(matches!(
            outbox.credit_gate(|_| Some(())).await,
            CreditGate::Closed
        ));
    }

    #[tokio::test]
    async fn a_stop_only_flags_the_execution_it_names() {
        let (outbox, _events) = outbox();
        assert!(!outbox.request_cancel("first").await);

        outbox.credit.lock().await.begin("first".into());
        assert!(!outbox.request_cancel("stale").await);
        assert!(!outbox.credit.lock().await.cancel_requested);
        assert!(outbox.request_cancel("first").await);
        assert!(outbox.credit.lock().await.cancel_requested);

        // The next execution starts clear, and a late Stop for the previous
        // one cannot reach it.
        outbox.credit.lock().await.begin("second".into());
        assert!(!outbox.credit.lock().await.cancel_requested);
        assert!(!outbox.request_cancel("first").await);
        assert!(!outbox.credit.lock().await.cancel_requested);
    }

    #[test]
    fn only_a_requested_stop_turns_57014_into_cancelled() {
        let canceled = database_error("57014");
        assert_eq!(failed_status(&canceled, true), "cancelled");
        // A statement timeout raises the same SQLSTATE without a Stop.
        assert_eq!(failed_status(&canceled, false), "failed");
        assert_eq!(failed_status(&database_error("42601"), true), "failed");
        assert_eq!(
            failed_status(&QuerySessionError::ConnectionLost, true),
            "failed"
        );
    }

    #[tokio::test]
    async fn cleanup_closes_the_cancel_window_and_holds_the_session_open() {
        let (outbox, _events) = outbox();
        outbox.credit.lock().await.begin("first".into());
        assert_eq!(outbox.checkpoint().await, postgres::Checkpoint::Proceed);
        assert!(outbox.request_cancel("first").await);
        assert_eq!(outbox.checkpoint().await, postgres::Checkpoint::Stop);

        // A Stop that came before cleanup is reported to it.
        let permit = outbox.begin_cleanup().await.expect("session is open");
        assert!(permit.stop_requested);
        drop(permit);

        // One that comes after is refused, so no cancel request is sent that
        // could land on the wrapper's own COMMIT or ROLLBACK.
        outbox.credit.lock().await.begin("second".into());
        let permit = outbox.begin_cleanup().await.expect("session is open");
        assert!(!permit.stop_requested);
        assert!(!outbox.request_cancel("second").await);
        assert!(!outbox.credit.lock().await.cancel_requested);

        // A close waits for cleanup, so nothing is sent after it.
        let closing = outbox.clone();
        let close = tokio::spawn(async move { closing.mark_closed().await });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(!close.is_finished());
        drop(permit);
        assert!(tokio::time::timeout(Duration::from_secs(1), close)
            .await
            .expect("close proceeds once cleanup is done")
            .unwrap());
        assert_eq!(outbox.checkpoint().await, postgres::Checkpoint::Closed);
        assert!(outbox.begin_cleanup().await.is_none());

        // The next execution gets a fresh cancel window.
        let (outbox, _events) = self::outbox();
        outbox.credit.lock().await.begin("first".into());
        drop(outbox.begin_cleanup().await);
        outbox.credit.lock().await.begin("second".into());
        assert!(outbox.request_cancel("second").await);
    }

    #[tokio::test]
    async fn a_closed_sink_fails_the_send_and_leaves_no_credit_behind() {
        let (sink, events) = closing_sink(0);
        let outbox = Outbox::new("session".into(), "tab".into(), "connection".into(), 0, sink);
        outbox.credit.lock().await.begin("execution".into());

        assert_eq!(
            send(
                &outbox,
                Some("execution".into()),
                false,
                QueryEvent::ExecutionStarted
            )
            .await,
            Err(())
        );
        // A row batch that was never delivered must not hold a credit slot:
        // nothing would ever acknowledge it.
        assert_eq!(
            send_with_credit(&outbox, "execution".into(), batch()).await,
            Err(())
        );
        assert!(outbox.credit.lock().await.outstanding.is_empty());
        // Nor may a terminal that was never delivered stay ackable.
        assert_eq!(
            send_terminal(&outbox, "execution".into(), QueryEvent::SessionClosed).await,
            Err(())
        );
        assert_eq!(outbox.credit.lock().await.terminal_sequence, None);
        assert!(events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stop_is_accepted_while_the_credit_window_is_full() {
        let (outbox, events) = outbox();
        outbox.credit.lock().await.begin("execution".into());
        let sender = outbox.clone();
        let execution = tokio::spawn(async move {
            let mut last = 0;
            for _ in 0..5 {
                last = send_with_credit(&sender, "execution".into(), batch()).await?;
            }
            Ok::<u64, ()>(last)
        });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert_eq!(kinds(&events, "rowBatch").len(), 4);
        assert!(!execution.is_finished());

        // The consumer is not acknowledging, and the Stop still lands: the
        // parked sender holds no lock the request needs, and the driver sees
        // the flag at its next checkpoint.
        let requested =
            tokio::time::timeout(Duration::from_secs(1), outbox.request_cancel("execution"))
                .await
                .expect("a parked sender does not block a Stop");
        assert!(requested);
        assert_eq!(outbox.checkpoint().await, postgres::Checkpoint::Stop);
        // A Stop is not credit: the parked batch stays parked.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(!execution.is_finished());
        assert_eq!(kinds(&events, "rowBatch").len(), 4);

        // One acknowledged batch frees one slot, no more.
        outbox.acknowledge(&ack(1)).await.unwrap();
        let delivered = tokio::time::timeout(Duration::from_secs(1), execution)
            .await
            .expect("the freed slot admits the parked batch")
            .unwrap();
        assert_eq!(delivered, Ok(5));
        assert_eq!(kinds(&events, "rowBatch").len(), 5);
    }

    #[tokio::test]
    async fn teardown_refuses_opens_before_any_connection_is_made() {
        let manager = manager();
        manager.register_owner("window", "owner".into()).await;
        // The spec points at nothing: a refused open never reaches it.
        let mut spec = live_spec(None);
        spec.port = 1;
        let open = |session: &'static str, connection: &'static str| {
            let manager = manager.clone();
            let spec = spec.clone();
            async move {
                manager
                    .open(
                        "window",
                        payload(session, connection),
                        recording_sink().0,
                        spec,
                    )
                    .await
            }
        };

        manager.begin_connection_teardown("closing").await;
        assert!(matches!(
            open("a", "closing").await,
            Err(QuerySessionError::ConnectionClosing)
        ));
        manager.end_connection_teardown("closing").await;

        manager.begin_global_teardown().await;
        assert!(matches!(
            open("b", "other").await,
            Err(QuerySessionError::ConnectionClosing)
        ));
        manager.end_global_teardown().await;

        let state = manager.inner.lock().await;
        assert!(state.opening.is_empty());
        assert!(manager
            .check_admission(&state, "window", &payload("c", "closing"))
            .is_ok());
    }

    #[test]
    fn a_probe_that_started_earlier_cannot_overwrite_a_later_one() {
        let order = ProbeOrder::default();
        // A Recheck starts, then the execution finishes and probes.
        let recheck = order.start();
        let execution = order.start();
        assert!(order.admit(execution));
        // The Recheck's answer predates the execution's and is dropped.
        assert!(!order.admit(recheck));
        assert!(order.admit(order.start()));
    }

    #[tokio::test]
    async fn parameters_and_row_limit_are_refused_before_policy_and_session_lookup() {
        let manager = manager();
        let production = crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
            environment: crate::Environment::Production,
            safe_mode: crate::SafeMode::Inherit,
            read_only: false,
        });
        let execute = |sql: &str, parameters: Option<Vec<(&str, &str)>>, row_limit, confirmed| {
            let request = ExecutionRequest {
                execution_id: "execution".into(),
                sql: sql.into(),
                parameters: parameters.map(|parameters| {
                    parameters
                        .into_iter()
                        .map(|(name, value)| ParameterValue {
                            name: name.into(),
                            value: Some(value.into()),
                        })
                        .collect()
                }),
                row_limit,
            };
            let manager = manager.clone();
            let policy = &production;
            async move {
                manager
                    .execute(
                        "missing-session",
                        request,
                        "window",
                        ExecutionSafety {
                            policy,
                            confirmed,
                            on_success: None,
                        },
                    )
                    .await
            }
        };
        let rejected = |result: Result<AcceptedResult, QuerySessionError>| match result {
            Err(QuerySessionError::ParametersRejected { reason, names }) => {
                (serde_json::to_value(reason).unwrap(), names)
            }
            other => panic!("expected a parameter refusal, got {:?}", other.err()),
        };
        let long = "n".repeat(64);
        let many = (0..257)
            .map(|index| format!(":p{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let huge = "v".repeat(1024 * 1024 + 1);
        // Each is an unconfirmed write on production against a session that
        // does not exist: the refusal comes first.
        for (sql, parameters, reason, names) in [
            (
                "UPDATE t SET a = é WHERE b = :a",
                vec![("a", "1")],
                "unlexable",
                vec![],
            ),
            (
                "UPDATE t SET a = :a; DELETE FROM t",
                vec![("a", "1")],
                "multipleStatements",
                vec![],
            ),
            (
                "UPDATE t SET a = :a WHERE b = $1",
                vec![("a", "1")],
                "positionalPlaceholder",
                vec![],
            ),
            (
                "UPDATE t SET a = :a WHERE b = :b",
                vec![("a", "1")],
                "missingValue",
                vec!["b"],
            ),
            (
                "UPDATE t SET a = :a",
                vec![("a", "1"), ("a", "2")],
                "duplicateName",
                vec!["a"],
            ),
            (
                &format!("UPDATE t SET a = :{long}"),
                vec![],
                "nameTooLong",
                vec![long.as_str()],
            ),
            (
                &format!("UPDATE t SET a = ARRAY[{many}]"),
                vec![],
                "tooManyParameters",
                vec![],
            ),
            (
                "UPDATE t SET a = :a",
                vec![("a", huge.as_str())],
                "valueTooLarge",
                vec!["a"],
            ),
        ] {
            let (found, found_names) = rejected(execute(sql, Some(parameters), None, false).await);
            assert_eq!(found, reason, "{sql}");
            assert_eq!(found_names, names, "{reason}");
        }
        assert!(matches!(
            execute("DELETE FROM t", None, Some(0), false).await,
            Err(QuerySessionError::InvalidRowLimit)
        ));

        // Once valid, the policy sees the rewritten text: `:where` is a value,
        // so this update is unbounded and needs confirmation.
        let sql = "UPDATE t SET a = :where";
        match execute(sql, Some(vec![("where", "1")]), None, false).await {
            Err(QuerySessionError::PolicyNeedsConfirmation { statements }) => {
                assert!(statements[0].unbounded);
            }
            other => panic!("expected a confirmation request, got {:?}", other.err()),
        }
        assert!(matches!(
            execute(sql, Some(vec![("where", "1")]), None, true).await,
            Err(QuerySessionError::SessionNotFound)
        ));
    }

    #[test]
    fn describe_reports_the_names_execution_binds_in_the_same_order() {
        use crate::postgres::sql_params::ExecutionShape;
        use service::describe_parameters as describe;
        let sql = "SELECT :b, ':skip', :a::int, x[1:n], ARRAY[:c] FROM t WHERE y = :b -- :d";
        let names = describe(sql).expect("describe").names;
        assert_eq!(names, ["b", "a", "c"]);
        // Binding each name to itself shows the order execution uses.
        let supplied = names
            .iter()
            .rev()
            .map(|name| ParameterValue {
                name: name.clone(),
                value: Some(name.clone()),
            })
            .collect::<Vec<_>>();
        let plan = plan_execution(sql.into(), Some(&supplied), None).expect("plan");
        let ExecutionShape::CursorRead { values, .. } = plan.shape else {
            panic!("a read with parameters is a cursor read");
        };
        assert_eq!(
            values.0,
            names.iter().cloned().map(Some).collect::<Vec<_>>()
        );
        assert!(matches!(
            describe("SELECT 'open, :a"),
            Err(QuerySessionError::ParametersRejected { names, .. }) if names.is_empty()
        ));
    }

    type Events = Arc<std::sync::Mutex<Vec<serde_json::Value>>>;

    /// Records each envelope as the JSON a serializing host would put on the
    /// wire, so assertions read the same field names the frontend does.
    fn recording_sink() -> (SharedSink<QueryEventEnvelope>, Events) {
        let events = Events::default();
        let recorded = events.clone();
        let sink: SharedSink<QueryEventEnvelope> = Arc::new(move |envelope: QueryEventEnvelope| {
            recorded
                .lock()
                .unwrap()
                .push(serde_json::to_value(&envelope).expect("event JSON"));
            Ok(())
        });
        (sink, events)
    }

    /// A host whose consumer goes away: the first `accepted` events are
    /// recorded, every later one is refused.
    fn closing_sink(accepted: usize) -> (SharedSink<QueryEventEnvelope>, Events) {
        let (recording, events) = recording_sink();
        let delivered = AtomicU64::new(0);
        let sink: SharedSink<QueryEventEnvelope> = Arc::new(move |envelope: QueryEventEnvelope| {
            if delivered.fetch_add(1, Ordering::SeqCst) >= accepted as u64 {
                return Err(crate::host::SinkClosed);
            }
            recording.send(envelope)
        });
        (sink, events)
    }

    fn kinds(events: &Events, kind: &str) -> Vec<serde_json::Value> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|envelope| envelope["event"]["kind"] == kind)
            .cloned()
            .collect()
    }

    const LIVE_WINDOW: &str = "live-window";
    const LIVE_WAIT: Duration = Duration::from_secs(10);

    fn live_spec(statement_timeout_ms: Option<u32>) -> ResolvedPostgresConnectSpec {
        ResolvedPostgresConnectSpec {
            connection_id: "live-15432".into(),
            host: "127.0.0.1".into(),
            port: 15432,
            database: "dbunk_demo".into(),
            user: "dbunk".into(),
            password: "dbunk".into(),
            tls: crate::postgres::tls::ResolvedTls::plain("127.0.0.1"),
            connect_timeout: Some(Duration::from_secs(5)),
            keepalive: None,
            driver_options: crate::PgDriverOptions {
                statement_timeout_ms,
                ..Default::default()
            },
            safety_policy: Default::default(),
        }
    }

    /// One real session on the disposable fixture. Events are recorded and
    /// never acknowledged unless a test calls `ack`.
    struct Live {
        manager: QuerySessionManager,
        events: Events,
        admin: postgres::SessionConnection,
        session_id: String,
        pid: i32,
    }

    impl Live {
        async fn open(session_id: &str, statement_timeout_ms: Option<u32>) -> Self {
            Self::open_spec(session_id, live_spec(statement_timeout_ms)).await
        }

        async fn open_spec(session_id: &str, spec: ResolvedPostgresConnectSpec) -> Self {
            Self::open_sink(session_id, spec, recording_sink()).await
        }

        async fn open_sink(
            session_id: &str,
            spec: ResolvedPostgresConnectSpec,
            (sink, events): (SharedSink<QueryEventEnvelope>, Events),
        ) -> Self {
            let manager = manager();
            manager.register_owner(LIVE_WINDOW, "owner".into()).await;
            manager
                .open(LIVE_WINDOW, payload(session_id, "live-15432"), sink, spec)
                .await
                .expect("open live session");
            let pid = manager.inner.lock().await.sessions[session_id]
                .connection
                .pid;
            Self {
                manager,
                events,
                admin: postgres::connect(&live_spec(None))
                    .await
                    .expect("admin connection"),
                session_id: session_id.into(),
                pid,
            }
        }

        async fn execute(&self, execution_id: &str, sql: &str) {
            self.execute_with(execution_id, sql, None, None)
                .await
                .expect("execution admitted");
        }

        async fn execute_with(
            &self,
            execution_id: &str,
            sql: &str,
            parameters: Option<&[(&str, Option<&str>)]>,
            row_limit: Option<i64>,
        ) -> Result<AcceptedResult, QuerySessionError> {
            let policy = crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
                environment: crate::Environment::Development,
                safe_mode: crate::SafeMode::Inherit,
                read_only: false,
            });
            self.manager
                .execute(
                    &self.session_id,
                    ExecutionRequest {
                        execution_id: execution_id.into(),
                        sql: sql.into(),
                        parameters: parameters.map(|parameters| {
                            parameters
                                .iter()
                                .map(|(name, value)| ParameterValue {
                                    name: (*name).into(),
                                    value: value.map(Into::into),
                                })
                                .collect()
                        }),
                        row_limit,
                    },
                    LIVE_WINDOW,
                    ExecutionSafety {
                        policy: &policy,
                        confirmed: true,
                        on_success: None,
                    },
                )
                .await
        }

        /// Runs one execution to its terminal event and acknowledges it, so
        /// the session can admit the next one.
        async fn settle(
            &self,
            execution_id: &str,
            sql: &str,
            parameters: Option<&[(&str, Option<&str>)]>,
            row_limit: Option<i64>,
        ) -> serde_json::Value {
            self.execute_with(execution_id, sql, parameters, row_limit)
                .await
                .expect("execution admitted");
            let terminal = self.terminal(execution_id).await;
            self.ack(&terminal).await;
            terminal["event"].clone()
        }

        fn events_of(&self, execution_id: &str, kind: &str) -> Vec<serde_json::Value> {
            kinds(&self.events, kind)
                .into_iter()
                .filter(|envelope| envelope["executionId"] == execution_id)
                .map(|envelope| envelope["event"].clone())
                .collect()
        }

        fn rows_of(&self, execution_id: &str) -> Vec<serde_json::Value> {
            self.events_of(execution_id, "rowBatch")
                .iter()
                .flat_map(|batch| batch["rows"].as_array().cloned().unwrap_or_default())
                .collect()
        }

        /// The cached status must agree with what the backend is doing: a
        /// cached `Idle` is what lets a cursor read open its own transaction.
        async fn assert_cached_status_is_true(&self, expected: QueryTransactionStatus) {
            let cached = self.manager.inner.lock().await.sessions[&self.session_id]
                .transaction
                .lock()
                .await
                .status;
            assert_eq!(cached, expected);
            let state = match expected {
                QueryTransactionStatus::Idle => "idle",
                QueryTransactionStatus::Active => "idle in transaction",
                QueryTransactionStatus::Failed => "idle in transaction (aborted)",
                QueryTransactionStatus::Unknown => panic!("unknown is never asserted"),
            };
            self.backend(&format!("state = '{state}'"), 1).await;
        }

        /// Waits for the terminal event, acknowledging row batches as the
        /// frontend would: the terminal is only sent once they are.
        async fn terminal(&self, execution_id: &str) -> serde_json::Value {
            tokio::time::timeout(LIVE_WAIT, async {
                let mut acknowledged = 0;
                loop {
                    let delivered = kinds(&self.events, "rowBatch")
                        .into_iter()
                        .filter(|envelope| envelope["executionId"] == execution_id)
                        .filter_map(|envelope| envelope["sequence"].as_u64())
                        .max()
                        .unwrap_or(0);
                    if delivered > acknowledged {
                        acknowledged = delivered;
                        let _ = self
                            .manager
                            .ack(
                                AckPayload {
                                    session_id: self.session_id.clone(),
                                    execution_id: execution_id.into(),
                                    ack_through_sequence: delivered,
                                    retain_more_rows: true,
                                },
                                LIVE_WINDOW,
                            )
                            .await;
                    }
                    let terminal = kinds(&self.events, "executionCompleted")
                        .into_iter()
                        .find(|envelope| envelope["executionId"] == execution_id);
                    if let Some(terminal) = terminal {
                        return terminal;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                let kinds = self
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|envelope| envelope["executionId"] == execution_id)
                    .map(|envelope| envelope["event"]["kind"].to_string())
                    .collect::<Vec<_>>();
                panic!("no terminal event for {execution_id}; saw {kinds:?}")
            })
        }

        async fn cancel(&self, execution_id: &str) -> CancelResult {
            self.manager
                .cancel(
                    ExecutionPayload {
                        session_id: self.session_id.clone(),
                        execution_id: execution_id.into(),
                    },
                    LIVE_WINDOW,
                )
                .await
                .expect("cancel accepted")
        }

        async fn rollback(&self) {
            self.manager
                .transaction_action(&self.session_id, LIVE_WINDOW, false)
                .await
                .expect("rollback");
        }

        async fn commit(&self) {
            self.manager
                .transaction_action(&self.session_id, LIVE_WINDOW, true)
                .await
                .expect("commit");
        }

        async fn ack(&self, envelope: &serde_json::Value) {
            self.manager
                .ack(
                    AckPayload {
                        session_id: self.session_id.clone(),
                        execution_id: envelope["executionId"].as_str().unwrap().into(),
                        ack_through_sequence: envelope["sequence"].as_u64().unwrap(),
                        retain_more_rows: true,
                    },
                    LIVE_WINDOW,
                )
                .await
                .expect("ack accepted");
        }

        /// Waits until the session's backend matches `condition` in
        /// `pg_stat_activity` exactly `expected` times.
        async fn backend(&self, condition: &str, expected: i64) {
            let sql =
                format!("SELECT count(*) FROM pg_stat_activity WHERE pid = $1 AND {condition}");
            tokio::time::timeout(LIVE_WAIT, async {
                loop {
                    let count: i64 = self
                        .admin
                        .client
                        .query_one(&sql, &[&self.pid])
                        .await
                        .expect("read pg_stat_activity")
                        .get(0);
                    if count == expected {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("backend never reached {expected} x ({condition})"));
        }
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_stop_settles_cancelled_once() {
        let live = Live::open("stop", None).await;
        live.execute("stopped", "SELECT pg_sleep(30)").await;
        live.backend("state = 'active' AND query LIKE '%pg_sleep%'", 1)
            .await;
        let cancel = live
            .manager
            .cancel(
                ExecutionPayload {
                    session_id: "stop".into(),
                    execution_id: "stopped".into(),
                },
                LIVE_WINDOW,
            )
            .await
            .expect("cancel accepted");
        assert!(cancel.requested);

        let terminal = live.terminal("stopped").await;
        assert_eq!(terminal["event"]["status"], "cancelled");
        assert_eq!(terminal["event"]["error"]["code"], "57014");
        assert_eq!(terminal["event"]["transaction"]["status"], "idle");
        live.ack(&terminal).await;

        // The Stop belongs to the execution it was sent for: the same
        // SQLSTATE from a timeout on the next execution is a failure.
        live.execute(
            "timed-out",
            "SET statement_timeout = 200; SELECT pg_sleep(30)",
        )
        .await;
        let terminal = live.terminal("timed-out").await;
        assert_eq!(terminal["event"]["status"], "failed");
        assert_eq!(terminal["event"]["error"]["code"], "57014");
        assert_eq!(kinds(&live.events, "executionCompleted").len(), 2);
        live.manager.close_window(LIVE_WINDOW).await;
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_statement_timeout_settles_failed() {
        let live = Live::open("timeout", Some(200)).await;
        live.execute("timed-out", "SELECT pg_sleep(30)").await;

        let terminal = live.terminal("timed-out").await;
        assert_eq!(terminal["event"]["status"], "failed");
        assert_eq!(terminal["event"]["error"]["code"], "57014");
        assert_eq!(terminal["event"]["transaction"]["status"], "idle");
        assert_eq!(kinds(&live.events, "executionCompleted").len(), 1);
        live.manager.close_window(LIVE_WINDOW).await;
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_close_with_withheld_acks_releases_the_backend() {
        let live = Live::open("withheld", None).await;
        live.execute(
            "stream",
            "SELECT repeat('x', 1000) FROM generate_series(1, 200000)",
        )
        .await;
        tokio::time::timeout(LIVE_WAIT, async {
            while kinds(&live.events, "rowBatch").len() < 4 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("credit window filled");
        // Nothing is acknowledged, so the window stays full and the fifth
        // batch is never delivered.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(kinds(&live.events, "rowBatch").len(), 4);
        live.backend("true", 1).await;

        live.manager
            .close("withheld", LIVE_WINDOW)
            .await
            .expect("close accepted");

        live.backend("true", 0).await;
        assert_eq!(kinds(&live.events, "rowBatch").len(), 4);
        assert!(kinds(&live.events, "executionCompleted").is_empty());
        assert_eq!(
            live.events.lock().unwrap().last().unwrap()["event"]["kind"],
            "sessionClosed"
        );
    }
    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_row_limit_is_truthful_at_the_boundary() {
        let live = Live::open("limits", None).await;
        let series = "SELECT g FROM generate_series(1, 5) g";

        // A limit equal to the row count withheld nothing.
        let terminal = live.settle("exact", series, None, Some(5)).await;
        assert_eq!(terminal["status"], "completed");
        let completed = &live.events_of("exact", "resultSetCompleted")[0];
        assert_eq!(completed["limit"], serde_json::Value::Null);
        assert_eq!(completed["rowCount"], 5);
        assert_eq!(live.rows_of("exact").len(), 5);

        // One below it: the server stopped, and says more rows exist.
        let terminal = live.settle("below", series, None, Some(4)).await;
        let completed = &live.events_of("below", "resultSetCompleted")[0];
        assert_eq!(completed["limit"], "stopped");
        assert_eq!(completed["rowCount"], 4);
        assert_eq!(completed["partial"], false);
        assert_eq!(live.rows_of("below").len(), 4);
        assert_eq!(terminal["omittedRows"], 0);
        assert_eq!(terminal["truncationReasons"], serde_json::json!([]));

        // No rows at all still reports the columns.
        live.settle("empty", "SELECT 1 AS only WHERE false", None, Some(10))
            .await;
        assert_eq!(
            live.events_of("empty", "resultSetStarted")[0]["columns"],
            serde_json::json!(["only"])
        );
        assert_eq!(
            live.events_of("empty", "resultSetCompleted")[0]["rowCount"],
            0
        );

        // The cell cap applies before the limit does.
        let terminal = live
            .settle(
                "wide",
                "SELECT repeat('x', 1100000) FROM generate_series(1, 5)",
                None,
                Some(3),
            )
            .await;
        assert_eq!(live.rows_of("wide").len(), 3);
        assert_eq!(
            live.rows_of("wide")[0][0].as_str().unwrap().len(),
            1024 * 1024
        );
        assert_eq!(
            live.events_of("wide", "resultSetCompleted")[0]["limit"],
            "stopped"
        );
        assert_eq!(
            terminal["truncationReasons"],
            serde_json::json!(["cellBytes"])
        );

        // A script is not cursor-eligible: every row is read, and the limit
        // caps what each Result Set retains.
        let script = format!("{series}; {series} WHERE g < 3");
        let terminal = live.settle("drained", &script, None, Some(3)).await;
        let completed = live.events_of("drained", "resultSetCompleted");
        assert_eq!(completed[0]["limit"], "drained");
        assert_eq!(completed[0]["rowCount"], 5);
        assert_eq!(completed[1]["limit"], serde_json::Value::Null);
        assert_eq!(completed[1]["rowCount"], 2);
        assert_eq!(live.rows_of("drained").len(), 5);
        assert_eq!(terminal["omittedRows"], 2);
        assert_eq!(
            terminal["truncationReasons"],
            serde_json::json!(["rowLimit"])
        );

        // Without the new fields nothing is limited or refused.
        let terminal = live.settle("plain", series, None, None).await;
        assert_eq!(terminal["refusal"], serde_json::Value::Null);
        assert_eq!(
            live.events_of("plain", "resultSetCompleted")[0]["limit"],
            serde_json::Value::Null
        );
        live.manager.close_window(LIVE_WINDOW).await;
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_wrapper_always_leaves_the_session_idle() {
        let live = Live::open("wrapper", None).await;
        let idle = QueryTransactionStatus::Idle;
        let series = "SELECT g FROM generate_series(1, 5) g WHERE g >= :from";
        let from: &[(&str, Option<&str>)] = &[("from", Some("2"))];

        // Success, with and without a limit stop.
        let terminal = live.settle("all", series, Some(from), None).await;
        assert_eq!(terminal["status"], "completed");
        assert_eq!(terminal["transaction"]["status"], "idle");
        assert_eq!(live.rows_of("all").len(), 4);
        live.assert_cached_status_is_true(idle).await;
        let terminal = live.settle("some", series, Some(from), Some(2)).await;
        assert_eq!(terminal["status"], "completed");
        assert_eq!(
            live.rows_of("some"),
            serde_json::json!([["2"], ["3"]])
                .as_array()
                .unwrap()
                .clone()
        );
        live.assert_cached_status_is_true(idle).await;

        // A server error, with its position in the user's text.
        let sql = "SELECT nope FROM generate_series(1, 5) g WHERE g >= :from";
        let terminal = live.settle("error", sql, Some(from), None).await;
        assert_eq!(terminal["status"], "failed");
        assert_eq!(terminal["error"]["code"], "42703");
        assert_eq!(terminal["error"]["position"], 8);
        live.assert_cached_status_is_true(idle).await;

        // A Stop during the FETCH.
        live.execute_with("fetching", "SELECT pg_sleep(30)", None, Some(1))
            .await
            .expect("execution admitted");
        live.backend("state = 'active' AND query LIKE 'FETCH%'", 1)
            .await;
        let cancel = live.cancel("fetching").await;
        assert!(cancel.requested);
        let terminal = live.terminal("fetching").await;
        assert_eq!(terminal["event"]["status"], "cancelled");
        assert_eq!(terminal["event"]["error"]["code"], "57014");
        assert_eq!(terminal["event"]["transaction"]["status"], "idle");
        live.ack(&terminal).await;
        live.assert_cached_status_is_true(idle).await;

        // A Stop before anything was sent: the execution task has not run
        // yet, so the flag is the first thing it sees.
        live.execute_with("early", "SELECT pg_sleep(30)", None, Some(1))
            .await
            .expect("execution admitted");
        assert!(live.cancel("early").await.requested);
        let terminal = live.terminal("early").await;
        assert_eq!(terminal["event"]["status"], "cancelled");
        assert_eq!(terminal["event"]["error"], serde_json::Value::Null);
        assert!(live.events_of("early", "resultSetStarted").is_empty());
        live.ack(&terminal).await;
        live.assert_cached_status_is_true(idle).await;

        // A user's holdable cursor with the reserved name: reported as it is.
        live.settle(
            "hold",
            "BEGIN; DECLARE dbunk_query_cursor CURSOR WITH HOLD FOR SELECT 1; COMMIT",
            None,
            None,
        )
        .await;
        let terminal = live.settle("collide", series, Some(from), None).await;
        assert_eq!(terminal["error"]["code"], "42P03");
        live.assert_cached_status_is_true(idle).await;
        live.settle("release", "CLOSE dbunk_query_cursor", None, None)
            .await;

        // Each execution settled exactly once, and nothing was left behind.
        for execution in ["all", "some", "error", "fetching", "early", "collide"] {
            assert_eq!(
                live.events_of(execution, "executionCompleted").len(),
                1,
                "{execution}"
            );
        }
        live.settle(
            "left",
            "SELECT (SELECT count(*) FROM pg_cursors) + (SELECT count(*) FROM pg_prepared_statements)",
            None,
            None,
        )
        .await;
        assert_eq!(
            live.rows_of("left"),
            serde_json::json!([["0"]]).as_array().unwrap().clone()
        );
        live.manager.close_window(LIVE_WINDOW).await;
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_user_transactions_are_never_committed_or_left_with_a_cursor()
    {
        let live = Live::open("owned", None).await;
        let active = QueryTransactionStatus::Active;
        let idle = QueryTransactionStatus::Idle;
        // A temp table made on the Script shape is visible to the other shapes.
        live.settle(
            "fixture",
            "CREATE TEMP TABLE owned_rows(id int4, v int4); INSERT INTO owned_rows VALUES (1, 0), (2, 0)",
            None,
            None,
        )
        .await;
        let read = "SELECT id FROM owned_rows WHERE v = :v ORDER BY id";
        let cursors = "SELECT count(*) FROM pg_cursors";
        let zero = serde_json::json!([["0"]]).as_array().unwrap().clone();

        // Autocommit, with a transaction the user opened in a script. The
        // cached status is not Idle, so no wrapper is started.
        live.settle("open", "BEGIN; UPDATE owned_rows SET v = 7", None, None)
            .await;
        live.assert_cached_status_is_true(active).await;
        let terminal = live
            .settle("inside", read, Some(&[("v", Some("7"))]), Some(1))
            .await;
        assert_eq!(terminal["status"], "completed");
        assert_eq!(terminal["transaction"]["status"], "active");
        assert_eq!(live.rows_of("inside").len(), 1);
        assert_eq!(
            live.events_of("inside", "resultSetCompleted")[0]["limit"],
            "stopped"
        );
        live.assert_cached_status_is_true(active).await;
        live.settle("cursors-1", cursors, None, None).await;
        assert_eq!(live.rows_of("cursors-1"), zero);
        // Rolling back proves the cursor read did not commit the update.
        live.rollback().await;
        live.assert_cached_status_is_true(idle).await;
        live.settle("check", "SELECT sum(v) FROM owned_rows", None, None)
            .await;
        assert_eq!(live.rows_of("check"), zero);

        // Manual mode opens the user's transaction and leaves it open.
        live.manager
            .set_mode("owned", LIVE_WINDOW, QueryTransactionMode::Manual)
            .await
            .expect("manual mode");
        let terminal = live
            .settle("manual", read, Some(&[("v", Some("0"))]), None)
            .await;
        assert_eq!(terminal["transaction"]["status"], "active");
        assert_eq!(live.rows_of("manual").len(), 2);
        live.assert_cached_status_is_true(active).await;
        live.settle("cursors-2", cursors, None, None).await;
        assert_eq!(live.rows_of("cursors-2"), zero);

        // A bound command runs inside it, as one command-only Result Set.
        let update = "UPDATE owned_rows SET v = :v WHERE id <= :id";
        let terminal = live
            .settle(
                "bound",
                update,
                Some(&[("v", Some("3")), ("id", Some("2"))]),
                Some(1),
            )
            .await;
        assert_eq!(terminal["status"], "completed");
        assert_eq!(
            live.events_of("bound", "resultSetStarted")[0]["columns"],
            serde_json::json!([])
        );
        assert_eq!(
            live.events_of("bound", "resultSetCompleted")[0]["rowCount"],
            2
        );
        live.assert_cached_status_is_true(active).await;

        // A statement that returns rows is refused unrun, with no error.
        let terminal = live
            .settle(
                "refused",
                &format!("{update} RETURNING id"),
                Some(&[("v", Some("9")), ("id", Some("2"))]),
                None,
            )
            .await;
        assert_eq!(terminal["status"], "failed");
        assert_eq!(terminal["refusal"], "parametersReturnRows");
        assert_eq!(terminal["error"], serde_json::Value::Null);
        assert!(live.events_of("refused", "resultSetStarted").is_empty());
        live.assert_cached_status_is_true(active).await;

        // An error inside the user's transaction leaves it failed.
        let terminal = live
            .settle("bad", read, Some(&[("v", Some("abc"))]), None)
            .await;
        assert_eq!(terminal["status"], "failed");
        assert_eq!(terminal["error"]["code"], "22P02");
        assert_eq!(terminal["transaction"]["status"], "failed");
        live.assert_cached_status_is_true(QueryTransactionStatus::Failed)
            .await;
        live.rollback().await;
        live.assert_cached_status_is_true(idle).await;

        // A refusal in manual mode opens no transaction at all.
        live.settle(
            "refused-idle",
            &format!("{update} RETURNING id"),
            Some(&[("v", Some("9")), ("id", Some("2"))]),
            None,
        )
        .await;
        live.assert_cached_status_is_true(idle).await;

        // A Stop inside the user's transaction keeps it, and closes the cursor.
        live.settle("reopen", "SELECT 1", None, None).await;
        live.assert_cached_status_is_true(active).await;
        live.execute_with("stopped", "SELECT pg_sleep(30)", None, Some(1))
            .await
            .expect("execution admitted");
        assert!(live.cancel("stopped").await.requested);
        let terminal = live.terminal("stopped").await;
        assert_eq!(terminal["event"]["status"], "cancelled");
        assert_eq!(terminal["event"]["transaction"]["status"], "active");
        live.ack(&terminal).await;
        live.assert_cached_status_is_true(active).await;

        live.commit().await;
        live.assert_cached_status_is_true(idle).await;
        live.manager
            .set_mode("owned", LIVE_WINDOW, QueryTransactionMode::Autocommit)
            .await
            .expect("autocommit mode");
        live.settle(
            "left",
            "SELECT count(*) FROM pg_prepared_statements",
            None,
            None,
        )
        .await;
        assert_eq!(live.rows_of("left"), zero);
        live.manager.close_window(LIVE_WINDOW).await;
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_close_mid_fetch_releases_the_backend() {
        let live = Live::open("mid-fetch", None).await;
        live.execute_with(
            "stream",
            "SELECT repeat('x', 1000) FROM generate_series(1, 200000) g WHERE g > :from",
            Some(&[("from", Some("0"))]),
            Some(10_000),
        )
        .await
        .expect("execution admitted");
        tokio::time::timeout(LIVE_WAIT, async {
            while kinds(&live.events, "rowBatch").len() < 4 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("credit window filled");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(kinds(&live.events, "rowBatch").len(), 4);
        // Credit holds back rows, not the transaction: the read was fetched
        // and its wrapper committed before the first row was handed over.
        live.backend("state = 'idle'", 1).await;

        live.manager
            .close("mid-fetch", LIVE_WINDOW)
            .await
            .expect("close accepted");

        live.backend("true", 0).await;
        assert!(kinds(&live.events, "executionCompleted").is_empty());
        assert!(kinds(&live.events, "resultSetCompleted").is_empty());
        assert_eq!(
            live.events.lock().unwrap().last().unwrap()["event"]["kind"],
            "sessionClosed"
        );
    }

    const LEFT_OVERS: &str =
        "SELECT (SELECT count(*) FROM pg_cursors) + (SELECT count(*) FROM pg_prepared_statements)";

    /// A Stop can land on the wrapper's BEGIN, DECLARE, FETCH, CLOSE or
    /// COMMIT, or after all of them. Whichever it hits, the execution settles
    /// once and the session is idle afterwards.
    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_stop_at_any_moment_settles_once_and_idle() {
        let live = Live::open("stop-any", None).await;
        let sql = "SELECT g FROM generate_series(1, 300) g WHERE g >= :from";
        let from: &[(&str, Option<&str>)] = &[("from", Some("1"))];
        let mut statuses = std::collections::BTreeMap::<String, usize>::new();
        for round in 0..300_u64 {
            let id = format!("round-{round}");
            live.execute_with(&id, sql, Some(from), Some(100))
                .await
                .expect("execution admitted");
            // The whole wrapper takes about a millisecond on a local fixture.
            tokio::time::sleep(Duration::from_micros(round * 7 % 1200)).await;
            let requested = live.cancel(&id).await.requested;
            let terminal = live.terminal(&id).await;
            let event = &terminal["event"];
            let status = event["status"].as_str().unwrap().to_owned();
            // A cancel request is a separate connection; the signal for one
            // round can reach the backend during the next. That 57014 has no
            // Stop of its own and is reported as the failure it is.
            let late_signal = status == "failed" && event["error"]["code"] == "57014";
            assert!(
                status == "completed" || status == "cancelled" || late_signal,
                "round {round}: {event}"
            );
            assert_eq!(event["transaction"]["status"], "idle", "round {round}");
            live.ack(&terminal).await;
            assert_eq!(live.events_of(&id, "executionCompleted").len(), 1);
            live.assert_cached_status_is_true(QueryTransactionStatus::Idle)
                .await;
            // A 57014 means the request reached a running statement; without
            // one the flag was seen between statements.
            let how = match (late_signal, event["error"]["code"].as_str()) {
                (true, _) => "late signal".into(),
                (false, Some(code)) => format!("{status} by {code}"),
                (false, None) => status,
            };
            *statuses
                .entry(format!("{how}, requested={requested}"))
                .or_default() += 1;
        }
        println!("stop-at-any-moment outcomes: {statuses:?}");
        live.settle("left", LEFT_OVERS, None, None).await;
        assert_eq!(live.rows_of("left"), [serde_json::json!(["0"])]);
        live.manager.close_window(LIVE_WINDOW).await;
    }

    /// The same for a close: the backend goes away, nothing follows
    /// `sessionClosed`, and the execution never settles twice.
    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_close_at_any_moment_releases_the_backend() {
        let mut settled = 0;
        for round in 0..40_u64 {
            let session = format!("close-{round}");
            let live = Live::open(&session, None).await;
            live.execute_with(
                "stream",
                "SELECT repeat('x', 200) FROM generate_series(1, 20000) g WHERE g > :from",
                Some(&[("from", Some("0"))]),
                Some(5_000),
            )
            .await
            .expect("execution admitted");
            tokio::time::sleep(Duration::from_micros(round * 173 % 6000)).await;
            live.manager
                .close(&session, LIVE_WINDOW)
                .await
                .expect("close accepted");
            live.backend("true", 0).await;
            let events = live.events.lock().unwrap().clone();
            assert_eq!(
                events.last().unwrap()["event"]["kind"],
                "sessionClosed",
                "round {round}"
            );
            let terminals = kinds(&live.events, "executionCompleted").len();
            assert!(terminals <= 1, "round {round}");
            settled += terminals;
        }
        println!("close-at-any-moment: {settled} of 40 executions settled before the close");
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_read_only_and_timeouts_on_the_new_shapes() {
        let read = "SELECT g FROM generate_series(1, 3) g WHERE g > :from";
        let from: &[(&str, Option<&str>)] = &[("from", Some("0"))];

        // A read-only connection: the wrapper uses the server defaults, so
        // the read works and a bound write is the server's to refuse.
        let mut spec = live_spec(None);
        spec.safety_policy = crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
            environment: crate::Environment::Development,
            safe_mode: crate::SafeMode::Inherit,
            read_only: true,
        });
        let live = Live::open_spec("read-only", spec).await;
        live.admin
            .client
            .batch_execute("CREATE TABLE IF NOT EXISTS plan023_read_only(a int4)")
            .await
            .expect("fixture table");
        let terminal = live.settle("read", read, Some(from), None).await;
        assert_eq!(terminal["status"], "completed");
        assert_eq!(live.rows_of("read").len(), 3);
        let terminal = live
            .settle(
                "write",
                "INSERT INTO plan023_read_only VALUES (:a)",
                Some(&[("a", Some("1"))]),
                None,
            )
            .await;
        assert_eq!(terminal["status"], "failed");
        assert_eq!(terminal["error"]["code"], "25006");
        live.assert_cached_status_is_true(QueryTransactionStatus::Idle)
            .await;
        live.admin
            .client
            .batch_execute("DROP TABLE plan023_read_only")
            .await
            .expect("drop fixture table");
        live.manager.close_window(LIVE_WINDOW).await;

        // A statement timeout during the FETCH is a failure, not a Stop.
        let live = Live::open("fetch-timeout", Some(200)).await;
        let terminal = live
            .settle("timed-out", "SELECT pg_sleep(30)", None, Some(1))
            .await;
        assert_eq!(terminal["status"], "failed");
        assert_eq!(terminal["error"]["code"], "57014");
        assert_eq!(terminal["transaction"]["status"], "idle");
        assert_eq!(live.events_of("timed-out", "executionCompleted").len(), 1);
        live.assert_cached_status_is_true(QueryTransactionStatus::Idle)
            .await;
        live.manager.close_window(LIVE_WINDOW).await;

        // An idle-in-transaction timeout must not end a session whose
        // frontend is slow to acknowledge rows: by then the wrapper is over.
        // The timeout only has to outlast the backend's own read of the
        // result, which no frontend can slow down.
        let mut spec = live_spec(None);
        spec.driver_options.idle_in_transaction_timeout_ms = Some(1_000);
        let live = Live::open_spec("idle-timeout", spec).await;
        live.execute_with(
            "held",
            "SELECT repeat('x', 1000) FROM generate_series(1, 200000) g WHERE g > :from",
            Some(from),
            Some(2_000),
        )
        .await
        .expect("execution admitted");
        tokio::time::timeout(LIVE_WAIT, async {
            while kinds(&live.events, "rowBatch").len() < 4 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("credit window filled");
        // Withhold acknowledgements for longer than the timeout.
        tokio::time::sleep(Duration::from_millis(2_000)).await;
        assert_eq!(kinds(&live.events, "rowBatch").len(), 4);
        live.backend("state = 'idle'", 1).await;
        // Once the frontend acknowledges, the same execution completes.
        let terminal = live.terminal("held").await;
        assert_eq!(terminal["event"]["status"], "completed");
        assert_eq!(terminal["event"]["transaction"]["status"], "idle");
        assert_eq!(live.rows_of("held").len(), 2_000);
        assert_eq!(
            live.events_of("held", "resultSetCompleted")[0]["limit"],
            "stopped"
        );
        live.manager.close_window(LIVE_WINDOW).await;
    }

    /// Forwards to the fixture until dropped, so a test can cut the session's
    /// socket without the server saying goodbye.
    async fn severable_route() -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy");
        let port = listener.local_addr().unwrap().port();
        let route = tokio::spawn(async move {
            let mut links = tokio::task::JoinSet::new();
            while let Ok((mut inbound, _)) = listener.accept().await {
                links.spawn(async move {
                    if let Ok(mut outbound) =
                        tokio::net::TcpStream::connect("127.0.0.1:15432").await
                    {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
            }
        });
        (port, route)
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_connection_drop_mid_fetch_loses_the_session() {
        let (port, route) = severable_route().await;
        let mut spec = live_spec(None);
        spec.port = port;
        let live = Live::open_spec("dropped", spec).await;
        // The server takes two seconds to run this FETCH, so the cut below
        // lands while the driver is waiting on it.
        live.execute_with(
            "stream",
            "SELECT g, pg_sleep(0.5) FROM generate_series(1, 4) g WHERE g > :from",
            Some(&[("from", Some("0"))]),
            Some(10),
        )
        .await
        .expect("execution admitted");
        live.backend("state = 'active' AND query LIKE 'FETCH%'", 1)
            .await;

        route.abort();
        let _ = route.await;

        let lost = tokio::time::timeout(LIVE_WAIT, async {
            loop {
                if let Some(lost) = kinds(&live.events, "sessionLost").pop() {
                    return lost;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("session lost");
        assert_eq!(lost["event"]["reason"], "connectionLost");
        assert!(kinds(&live.events, "executionCompleted").is_empty());
        assert!(live.manager.inner.lock().await.sessions.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_a_sink_that_closes_mid_stream_retires_the_session() {
        // SessionState, ExecutionStarted, ResultSetStarted and one row batch
        // are delivered; the second batch is refused.
        let live = Live::open_sink("sink-closed", live_spec(None), closing_sink(4)).await;
        live.execute(
            "stream",
            "SELECT repeat('x', 1000) FROM generate_series(1, 200000)",
        )
        .await;

        live.backend("true", 0).await;
        assert!(live.manager.inner.lock().await.sessions.is_empty());
        assert_eq!(kinds(&live.events, "rowBatch").len(), 1);
        assert!(kinds(&live.events, "executionCompleted").is_empty());
        assert_eq!(live.events.lock().unwrap().len(), 4);
        assert!(matches!(
            live.execute_with("next", "SELECT 1", None, None).await,
            Err(QuerySessionError::SessionNotFound)
        ));
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_owner_replacement_closes_the_old_owners_sessions() {
        let live = Live::open("replaced", None).await;
        live.settle("first", "SELECT 1", None, None).await;
        let delivered = live.events.lock().unwrap().len();

        // The same owner registering again is a no-op.
        let same = live
            .manager
            .register_owner(LIVE_WINDOW, "owner".into())
            .await;
        assert_eq!(same.replaced_session_count, 0);
        live.backend("true", 1).await;

        // A new owner for the window (a reloaded document, a rebuilt view)
        // retires everything the old one held.
        let replaced = live
            .manager
            .register_owner(LIVE_WINDOW, "next-owner".into())
            .await;
        assert_eq!(replaced.replaced_session_count, 1);
        live.backend("true", 0).await;
        assert!(matches!(
            live.execute_with("after", "SELECT 1", None, None).await,
            Err(QuerySessionError::SessionNotFound)
        ));
        // Nothing reaches the old owner's sink, not even a close notice.
        assert_eq!(live.events.lock().unwrap().len(), delivered);
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_an_unfocused_window_keeps_its_lease() {
        let live = Live::open("lease", None).await;
        let expire_liveness = || async {
            let session = live.manager.inner.lock().await.sessions["lease"].clone();
            *session.last_liveness.lock().await = Instant::now()
                .checked_sub(LEASE)
                .expect("the clock is older than one lease");
        };

        // A window in the background stops heartbeating; that is not a lost
        // owner.
        live.manager.set_focused(LIVE_WINDOW, false).await;
        expire_liveness().await;
        live.manager.expire_stalled().await;
        assert!(live
            .manager
            .inner
            .lock()
            .await
            .sessions
            .contains_key("lease"));

        // Refocus renews the lease before the monitor can see the stale one.
        live.manager.set_focused(LIVE_WINDOW, true).await;
        live.manager.expire_stalled().await;
        assert!(live
            .manager
            .inner
            .lock()
            .await
            .sessions
            .contains_key("lease"));
        assert!(kinds(&live.events, "sessionLost").is_empty());

        // A focused window that stops heartbeating is a lost owner.
        expire_liveness().await;
        live.manager.expire_stalled().await;
        assert!(live.manager.inner.lock().await.sessions.is_empty());
        let lost = kinds(&live.events, "sessionLost");
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0]["event"]["reason"], "ownerTimeout");
        live.backend("true", 0).await;
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_global_teardown_closes_sessions_before_it_returns() {
        let live = Live::open("shutdown", None).await;
        live.execute("running", "SELECT pg_sleep(30)").await;
        live.backend("state = 'active' AND query LIKE '%pg_sleep%'", 1)
            .await;

        let session = live.manager.inner.lock().await.sessions["shutdown"].clone();

        live.manager.begin_global_teardown().await;

        // Teardown awaited the close: the session is already marked closed
        // and its cancel request already sent when the call returns. A close
        // that was only scheduled would still read open here.
        assert!(*session.closed.lock().await);
        assert!(live.manager.inner.lock().await.sessions.is_empty());
        drop(session);
        // The execution task then lets go of the session and the socket goes.
        live.backend("true", 0).await;
        assert!(matches!(
            live.manager
                .open(
                    LIVE_WINDOW,
                    payload("late", "live-15432"),
                    recording_sink().0,
                    live_spec(None),
                )
                .await,
            Err(QuerySessionError::ConnectionClosing)
        ));
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_a_refused_first_event_fails_the_open_and_releases_the_backend(
    ) {
        // This test's sessions are the only ones on the `postgres` database,
        // so its backends can be counted while other live tests run.
        let mut spec = live_spec(None);
        spec.database = "postgres".into();
        let admin = postgres::connect(&live_spec(None))
            .await
            .expect("admin connection");
        let backends = || async {
            admin
                .client
                .query_one(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = 'postgres'",
                    &[],
                )
                .await
                .expect("read pg_stat_activity")
                .get::<_, i64>(0)
        };
        assert_eq!(backends().await, 0);
        let manager = manager();
        manager.register_owner(LIVE_WINDOW, "owner".into()).await;
        let (sink, events) = closing_sink(0);

        let opened = manager
            .open(LIVE_WINDOW, payload("refused", "live-15432"), sink, spec)
            .await;

        assert!(matches!(opened, Err(QuerySessionError::ConnectionLost)));
        assert!(events.lock().unwrap().is_empty());
        let state = manager.inner.lock().await;
        assert!(state.sessions.is_empty());
        assert!(state.opening.is_empty());
        assert!(state.observers.is_empty());
        drop(state);
        tokio::time::timeout(LIVE_WAIT, async {
            while backends().await != 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the session and observer backends are released");
    }

    #[tokio::test]
    #[ignore = "requires make -C infrastructure/test-db postgres"]
    async fn query_session_actor_live_a_refused_terminal_event_retires_the_session() {
        // SessionState, ExecutionStarted, ResultSetStarted and
        // ResultSetCompleted are delivered. No row batch needs an ACK, so the
        // next event is the terminal one, and it is refused.
        let live = Live::open_sink("terminal-refused", live_spec(None), closing_sink(4)).await;
        live.execute("empty", "SELECT 1 WHERE false").await;

        live.backend("true", 0).await;
        assert!(live.manager.inner.lock().await.sessions.is_empty());
        assert!(kinds(&live.events, "executionCompleted").is_empty());
        assert_eq!(
            live.events.lock().unwrap().last().unwrap()["event"]["kind"],
            "resultSetCompleted"
        );
    }
}

#[cfg(all(test, feature = "isolated-profile"))]
mod native_cleanup_tests;
