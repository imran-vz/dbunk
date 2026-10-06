//! Plan 031 step 4: native MySQL sessions.
//!
//! One session is one dedicated `MySqlConnection` owned by a worker task that
//! the backend tracks for shutdown. Requests queue on a bounded channel and
//! run one at a time. A lost connection, a metadata request that cannot be
//! interrupted, or a retirement (disconnect, connection or credential change,
//! shutdown) closes the session for good: nothing reconnects automatically,
//! the host opens a new one.
//!
//! Every request carries an id. The worker records the id it is running, so
//! a cancel withdraws a queued request before it starts and sends
//! `KILL QUERY` only while the target request is the one running (checked
//! under the tracker lock, which the worker also takes between requests).
//!
//! Resolution follows the native health probe: the development gate and the
//! credential guard are held while the record, its secret and its SSH route
//! are resolved and the socket opens, so a concurrent edit cannot be
//! overtaken. The route then lives exactly as long as the worker.
mod rows;
mod worker;

#[cfg(test)]
mod tests;

pub use rows::{MYSQL_MAX_CELL_BYTES, MYSQL_MAX_RESULT_BYTES, MYSQL_MAX_ROWS};

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};
use tokio::sync::{mpsc, oneshot, watch, Mutex};

use super::{Backend, StatementClassSummary};
use crate::{credentials, storage, StoredConnection};

/// Connect, including the SSH route, must finish within this budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Catalog, browse, structure and definition requests. Expiry fails the
/// request and interrupts it with `KILL QUERY`; the session stays open.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
/// After an expired metadata request is interrupted, it must hand the
/// connection back within this budget, or the session closes (state unknown).
const METADATA_KILL_GRACE: Duration = Duration::from_secs(10);
/// Connect plus `KILL QUERY` on the short side connection.
const KILL_TIMEOUT: Duration = Duration::from_secs(5);
/// Requests waiting behind the running one.
const QUEUE: usize = 8;
/// Names listed per catalog kind; more are reported as truncated.
pub const MYSQL_MAX_CATALOG_ITEMS: usize = 5000;
/// Rows per browse page; one more is read to detect a next page, so this
/// stays below the result row budget.
pub const MYSQL_MAX_PAGE_ROWS: u32 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MySqlSessionError {
    /// The session cannot be opened (record, credentials, route or server).
    Unavailable(String),
    /// The session is closed; open a new one explicitly. `None` is a normal
    /// disconnect or retirement, `Some` a failure.
    Closed(Option<String>),
    /// The server rejected the request; the session remains usable.
    Database(String),
    /// The connection is read-only and the statement may write.
    ReadOnly(String),
    /// The connection's safety policy requires an explicit confirmation.
    NeedsConfirmation(Vec<StatementClassSummary>),
    /// Too many requests are already queued on this session.
    Busy,
    /// The request was cancelled before it ran, or interrupted while running.
    Cancelled,
}

impl std::fmt::Display for MySqlSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) | Self::Database(reason) | Self::ReadOnly(reason) => {
                f.write_str(reason)
            }
            Self::Closed(None) => f.write_str("Session closed"),
            Self::Closed(Some(reason)) => write!(f, "Session closed: {reason}"),
            Self::NeedsConfirmation(_) => f.write_str("This statement requires confirmation"),
            Self::Busy => f.write_str("Session is busy; wait for the running request"),
            Self::Cancelled => f.write_str("Cancelled"),
        }
    }
}

/// Lifecycle as observed by the host. `Closed` is terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MySqlSessionStatus {
    Open,
    Closed(Option<String>),
}

/// Server facts captured right after connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlServerInfo {
    pub version: String,
    /// Default schema of the connection, if any.
    pub database: Option<String>,
    pub read_only: bool,
}

/// One statement's (or script's) result. Only the last row-returning result
/// set is kept; `result_sets` counts all of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MySqlResult {
    pub columns: Vec<String>,
    /// `None` is SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows the server returned in the kept set, including dropped ones.
    pub total_rows: u64,
    /// Rows were dropped by the row or byte budget.
    pub truncated: bool,
    pub truncated_cells: u64,
    pub rows_affected: u64,
    pub result_sets: u32,
    /// Browse only: another page exists after this one.
    pub has_more: bool,
    /// Browse only: the object has no unique non-null key and some columns
    /// cannot be ordered exactly, so pages may overlap or skip rows.
    pub approximate: bool,
    /// Query only: the session's database after the script ran.
    pub database: Option<String>,
    pub runtime_ms: u64,
}

/// Identity of one query or browse request, for cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MySqlRequestId(u64);

/// What a cancel did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MySqlCancel {
    /// The request was still queued; it will not run and replies `Cancelled`.
    Withdrawn,
    /// `KILL QUERY` was sent while the request was running.
    Interrupted,
    /// The request is neither queued nor running (finished or unknown).
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MySqlRoutineKind {
    Procedure,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlRoutine {
    pub name: String,
    pub kind: MySqlRoutineKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlTrigger {
    pub name: String,
    pub table: String,
}

/// Objects of one database (MySQL's schema), each list name-ordered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MySqlObjects {
    pub database: String,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub routines: Vec<MySqlRoutine>,
    pub events: Vec<String>,
    pub triggers: Vec<MySqlTrigger>,
    /// At least one list hit `MYSQL_MAX_CATALOG_ITEMS`.
    pub truncated: bool,
}

/// Object kinds with a `SHOW CREATE` definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MySqlObjectKind {
    Table,
    View,
    Procedure,
    Function,
    Event,
    Trigger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub primary_key: bool,
    /// `VIRTUAL` or `STORED` for generated columns.
    pub generated: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlIndex {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySqlConstraint {
    pub name: String,
    pub kind: String,
    pub definition: String,
}

/// Table structure from the shared MySQL introspection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MySqlStructure {
    pub columns: Vec<MySqlColumn>,
    pub primary_key: Vec<String>,
    pub indexes: Vec<MySqlIndex>,
    pub foreign_keys: Vec<MySqlForeignKey>,
    pub constraints: Vec<MySqlConstraint>,
}

enum Request {
    Objects(String),
    Databases,
    Query {
        sql: String,
        database: Option<String>,
        confirmed: bool,
    },
    Browse {
        database: String,
        table: String,
        offset: u64,
        limit: u32,
    },
    Structure {
        database: String,
        table: String,
    },
    Definition {
        database: String,
        kind: MySqlObjectKind,
        name: String,
    },
}

enum Reply {
    Databases(Vec<String>, bool),
    Objects(MySqlObjects),
    Result(MySqlResult),
    Structure(MySqlStructure),
    Definition(String),
}

type Envelope = (
    u64,
    Request,
    oneshot::Sender<Result<Reply, MySqlSessionError>>,
);

/// Queued and running request ids, shared by the handles and the worker.
#[derive(Debug, Default)]
struct Tracker {
    /// Queued ids; `true` once cancelled.
    queued: HashMap<u64, bool>,
    running: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Queued,
    Running,
    Idle,
}

impl Tracker {
    fn enqueue(&mut self, id: u64) {
        self.queued.insert(id, false);
    }

    /// The request never reached the queue.
    fn withdraw(&mut self, id: u64) {
        self.queued.remove(&id);
    }

    /// Worker: `id` was dequeued. False when it was cancelled and must not run.
    fn start(&mut self, id: u64) -> bool {
        if self.queued.remove(&id).unwrap_or(false) {
            return false;
        }
        self.running = Some(id);
        true
    }

    fn finish(&mut self, id: u64) {
        if self.running == Some(id) {
            self.running = None;
        }
    }

    /// Marks a queued request cancelled; reports where `id` is.
    fn cancel(&mut self, id: u64) -> Target {
        if let Some(cancelled) = self.queued.get_mut(&id) {
            *cancelled = true;
            Target::Queued
        } else if self.running == Some(id) {
            Target::Running
        } else {
            Target::Idle
        }
    }
}

/// Cloneable handle to one open session. Dropping every handle closes it.
#[derive(Clone)]
pub struct MySqlSession(Arc<Shared>);

struct Shared {
    connection_id: String,
    server: MySqlServerInfo,
    requests: mpsc::Sender<Envelope>,
    retire: watch::Sender<bool>,
    status: watch::Receiver<MySqlSessionStatus>,
    /// Side connection target for `KILL QUERY`; the route stays up while the
    /// worker lives.
    cancel: (MySqlConnectOptions, u64),
    tracker: Arc<Mutex<Tracker>>,
    next_id: AtomicU64,
}

impl MySqlSession {
    pub fn connection_id(&self) -> &str {
        &self.0.connection_id
    }

    pub fn server(&self) -> &MySqlServerInfo {
        &self.0.server
    }

    pub fn status(&self) -> MySqlSessionStatus {
        self.0.status.borrow().clone()
    }

    /// Resolves when the session reaches `Closed`.
    pub async fn closed(&self) -> Option<String> {
        wait_closed(self.0.status.clone()).await
    }

    /// Like `closed`, but holds no handle, so a watcher never keeps the
    /// session open.
    pub fn closed_signal(
        &self,
    ) -> impl std::future::Future<Output = Option<String>> + Send + 'static {
        wait_closed(self.0.status.clone())
    }

    /// Requests a normal close and waits (bounded) for the worker to finish.
    pub async fn close(&self) {
        self.0.retire.send_replace(true);
        let _ = tokio::time::timeout(Duration::from_secs(5), self.closed()).await;
    }

    /// A fresh id for a query or browse request, so it can be cancelled.
    pub fn request_id(&self) -> MySqlRequestId {
        MySqlRequestId(self.0.next_id.fetch_add(1, Ordering::Relaxed) + 1)
    }

    async fn request(&self, id: u64, request: Request) -> Result<Reply, MySqlSessionError> {
        if let MySqlSessionStatus::Closed(reason) = self.status() {
            return Err(MySqlSessionError::Closed(reason));
        }
        let (reply, receive) = oneshot::channel();
        {
            // Registered before the worker can dequeue it, under the lock the
            // worker takes to start it.
            let mut tracker = self.0.tracker.lock().await;
            tracker.enqueue(id);
            if let Err(error) = self.0.requests.try_send((id, request, reply)) {
                tracker.withdraw(id);
                return Err(match error {
                    mpsc::error::TrySendError::Full(_) => MySqlSessionError::Busy,
                    mpsc::error::TrySendError::Closed(_) => MySqlSessionError::Closed(None),
                });
            }
        }
        match receive.await {
            Ok(result) => result,
            // The worker ended before answering; report why.
            Err(_) => Err(MySqlSessionError::Closed(self.closed().await)),
        }
    }

    /// Databases the account can see, name-ordered; the flag reports truncation.
    pub async fn databases(&self) -> Result<(Vec<String>, bool), MySqlSessionError> {
        match self
            .request(self.request_id().0, Request::Databases)
            .await?
        {
            Reply::Databases(names, truncated) => Ok((names, truncated)),
            _ => unreachable!("databases reply"),
        }
    }

    pub async fn objects(&self, database: String) -> Result<MySqlObjects, MySqlSessionError> {
        match self
            .request(self.request_id().0, Request::Objects(database))
            .await?
        {
            Reply::Objects(objects) => Ok(objects),
            _ => unreachable!("objects reply"),
        }
    }

    /// Runs a script on the session. Its database is always pinned first:
    /// `database` when given, else the connection's default; with neither,
    /// the script is refused while an earlier script left the session in a
    /// database. Writes follow the connection's read-only flag and safety
    /// policy. `id` (from `request_id`) identifies it for cancellation.
    pub async fn query(
        &self,
        sql: String,
        database: Option<String>,
        confirmed: bool,
        id: MySqlRequestId,
    ) -> Result<MySqlResult, MySqlSessionError> {
        match self
            .request(
                id.0,
                Request::Query {
                    sql,
                    database,
                    confirmed,
                },
            )
            .await?
        {
            Reply::Result(result) => Ok(result),
            _ => unreachable!("query reply"),
        }
    }

    /// One page of a table or view, ordered by its primary key, else a unique
    /// non-null index, else every exactly orderable column (`approximate`
    /// when that is not a total order). `id` (from `request_id`) identifies
    /// it for cancellation.
    pub async fn browse(
        &self,
        database: String,
        table: String,
        offset: u64,
        limit: u32,
        id: MySqlRequestId,
    ) -> Result<MySqlResult, MySqlSessionError> {
        match self
            .request(
                id.0,
                Request::Browse {
                    database,
                    table,
                    offset,
                    limit: limit.clamp(1, MYSQL_MAX_PAGE_ROWS),
                },
            )
            .await?
        {
            Reply::Result(result) => Ok(result),
            _ => unreachable!("browse reply"),
        }
    }

    pub async fn structure(
        &self,
        database: String,
        table: String,
    ) -> Result<MySqlStructure, MySqlSessionError> {
        match self
            .request(self.request_id().0, Request::Structure { database, table })
            .await?
        {
            Reply::Structure(structure) => Ok(structure),
            _ => unreachable!("structure reply"),
        }
    }

    /// `SHOW CREATE …` text for one object.
    pub async fn definition(
        &self,
        database: String,
        kind: MySqlObjectKind,
        name: String,
    ) -> Result<String, MySqlSessionError> {
        match self
            .request(
                self.request_id().0,
                Request::Definition {
                    database,
                    kind,
                    name,
                },
            )
            .await?
        {
            Reply::Definition(text) => Ok(text),
            _ => unreachable!("definition reply"),
        }
    }
}

async fn wait_closed(mut status: watch::Receiver<MySqlSessionStatus>) -> Option<String> {
    let result = status
        .wait_for(|status| matches!(status, MySqlSessionStatus::Closed(_)))
        .await;
    match result.as_deref() {
        Ok(MySqlSessionStatus::Closed(reason)) => reason.clone(),
        _ => None,
    }
}

/// Live sessions, so lifecycle changes and shutdown can retire them.
#[derive(Default)]
pub(super) struct Registry(StdMutex<RegistryState>);

#[derive(Default)]
struct RegistryState {
    closed: bool,
    next: u64,
    sessions: Vec<(u64, String, watch::Sender<bool>)>,
}

impl Registry {
    fn register(&self, connection: &str, retire: watch::Sender<bool>) -> Option<u64> {
        let mut state = self.0.lock().unwrap();
        if state.closed {
            return None;
        }
        state.next += 1;
        let key = state.next;
        state.sessions.push((key, connection.to_owned(), retire));
        Some(key)
    }

    fn remove(&self, key: u64) {
        self.0
            .lock()
            .unwrap()
            .sessions
            .retain(|(entry, _, _)| *entry != key);
    }

    /// Signals every session on `connection` (all when `None`) to close.
    pub(super) fn retire(&self, connection: Option<&str>) {
        for (_, id, retire) in self.0.lock().unwrap().sessions.iter() {
            if connection.is_none_or(|connection| connection == id) {
                retire.send_replace(true);
            }
        }
    }

    /// Shutdown fence: retire everything and refuse new sessions.
    pub(super) fn close(&self) {
        self.0.lock().unwrap().closed = true;
        self.retire(None);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().unwrap().sessions.len()
    }
}

/// Connect options from a resolved record (secret set, route applied).
/// Server session defaults (time zone, `sql_mode`) are left untouched.
pub(super) fn connect_options(
    connection: &crate::MySqlStoredConnection,
) -> Result<MySqlConnectOptions, String> {
    if connection.host.trim().is_empty() || connection.user.is_empty() {
        return Err("MySQL host and user are required".into());
    }
    let mut options = MySqlConnectOptions::new()
        .host(&connection.host)
        .port(if connection.port == 0 {
            3306
        } else {
            connection.port
        })
        .username(&connection.user)
        .password(&connection.password)
        // Same mapping as the dispatch DSN: negotiate when enabled.
        .ssl_mode(if connection.ssl {
            MySqlSslMode::Preferred
        } else {
            MySqlSslMode::Disabled
        })
        .timezone(None)
        .pipes_as_concat(false)
        .no_engine_substitution(false)
        .statement_cache_capacity(32);
    if !connection.database.trim().is_empty() {
        options = options.database(&connection.database);
    }
    Ok(options)
}

impl Backend {
    /// Opens a dedicated session for a saved MySQL connection. One attempt;
    /// failures are returned, never retried.
    pub async fn open_mysql_session(
        &self,
        connection_id: String,
    ) -> Result<MySqlSession, MySqlSessionError> {
        let authority = self.development().map_err(MySqlSessionError::Unavailable)?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let _guard = credentials::mutation_guard(&state.credentials).await;
                super::admit_connection(&state, Some(&authority), &connection_id)
                    .await
                    .map_err(|_| unavailable("Connection is not available; reload and retry"))?;
                let Some((connection, _)) =
                    storage::read_native_connection_by_id(&state.pool, &connection_id)
                        .await
                        .map_err(MySqlSessionError::Unavailable)?
                else {
                    return Err(unavailable("Connection no longer exists"));
                };
                if !matches!(connection, StoredConnection::MySQL(_)) {
                    return Err(unavailable("Connection is not a MySQL connection"));
                }
                if !credentials::onboarding_completed(&state.pool)
                    .await
                    .map_err(MySqlSessionError::Unavailable)?
                {
                    return Err(unavailable(
                        "Configure credential storage before connecting",
                    ));
                }
                let mode = crate::app::current_credential_mode(&state)
                    .await
                    .map_err(MySqlSessionError::Unavailable)?;
                let secrets = credentials::read_all_cached(&state.credentials, mode)
                    .await
                    .map_err(MySqlSessionError::Unavailable)?;
                let mut connection = connection;
                connection.set_password(secrets.get(&connection_id).cloned().unwrap_or_default());
                let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
                let (route, resolved) = super::bastions::route_probe(&state, connection, deadline)
                    .await
                    .map_err(|reason| unavailable(route_message(&reason)))?;
                let StoredConnection::MySQL(mysql) = &resolved else {
                    unreachable!("route keeps the engine");
                };
                let options = connect_options(mysql).map_err(MySqlSessionError::Unavailable)?;
                let opened =
                    tokio::time::timeout_at(deadline, worker::connect(&options, mysql.read_only))
                        .await
                        .map_err(|_| unavailable("Connecting to MySQL timed out"))??;
                crate::app::touch_connection_activity(&state, &connection_id).await;
                Ok(worker::spawn(
                    &inner,
                    state.clone(),
                    worker::Opened {
                        connection_id,
                        resolved,
                        route,
                        options,
                        session: opened,
                    },
                ))
            }
            .await)
        })
        .await
        .map_err(|_| MySqlSessionError::Unavailable("Native backend is closing".into()))?
        .and_then(|spawned| spawned.ok_or(unavailable("Native backend is closing")))
    }

    /// Cancels one request: a queued one is withdrawn before it runs; a
    /// running one is stopped with `KILL QUERY` on a short side connection,
    /// sent only while that request is still the one running. Other requests
    /// on the session are never touched, and the session stays open.
    pub async fn cancel_mysql_query(
        &self,
        session: &MySqlSession,
        request: MySqlRequestId,
    ) -> Result<MySqlCancel, String> {
        // Only the pieces, so a pending cancel never keeps the session open.
        let (options, thread) = session.0.cancel.clone();
        let tracker = session.0.tracker.clone();
        self.call(move |_| async move {
            Ok(tokio::time::timeout(
                KILL_TIMEOUT,
                worker::cancel(&options, thread, &tracker, request.0),
            )
            .await
            .unwrap_or_else(|_| Err("Cancel timed out".into())))
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }
}

fn unavailable(reason: &str) -> MySqlSessionError {
    MySqlSessionError::Unavailable(reason.into())
}

fn route_message(reason: &super::DevelopmentConnectionFailure) -> &'static str {
    use super::DevelopmentConnectionFailure as Failure;
    match reason {
        Failure::SshHostKey => {
            "The bastion host key is untrusted or changed; review it under Bastion Servers"
        }
        Failure::Timeout => "The SSH route timed out",
        _ => "The SSH route could not be established",
    }
}
