//! Plan 031 step 4: native MySQL sessions.
//!
//! One session is one dedicated `MySqlConnection` owned by a worker task that
//! the backend tracks for shutdown. Requests queue on a bounded channel and
//! run one at a time. A lost connection, a metadata timeout or a retirement
//! (disconnect, connection or credential change, shutdown) closes the session
//! for good: nothing reconnects automatically, the host opens a new one.
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

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use sqlx::mysql::{MySqlConnectOptions, MySqlSslMode};
use tokio::sync::{mpsc, oneshot, watch};

use super::{Backend, StatementClassSummary};
use crate::{credentials, storage, StoredConnection};

/// Connect, including the SSH route, must finish within this budget.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Catalog, browse, structure and definition requests. Expiry closes the
/// session because the connection state is then unknown.
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
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
    pub runtime_ms: u64,
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

type Envelope = (Request, oneshot::Sender<Result<Reply, MySqlSessionError>>);

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

    async fn request(&self, request: Request) -> Result<Reply, MySqlSessionError> {
        if let MySqlSessionStatus::Closed(reason) = self.status() {
            return Err(MySqlSessionError::Closed(reason));
        }
        let (reply, receive) = oneshot::channel();
        self.0
            .requests
            .try_send((request, reply))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => MySqlSessionError::Busy,
                mpsc::error::TrySendError::Closed(_) => MySqlSessionError::Closed(None),
            })?;
        match receive.await {
            Ok(result) => result,
            // The worker ended before answering; report why.
            Err(_) => Err(MySqlSessionError::Closed(self.closed().await)),
        }
    }

    /// Databases the account can see, name-ordered; the flag reports truncation.
    pub async fn databases(&self) -> Result<(Vec<String>, bool), MySqlSessionError> {
        match self.request(Request::Databases).await? {
            Reply::Databases(names, truncated) => Ok((names, truncated)),
            _ => unreachable!("databases reply"),
        }
    }

    pub async fn objects(&self, database: String) -> Result<MySqlObjects, MySqlSessionError> {
        match self.request(Request::Objects(database)).await? {
            Reply::Objects(objects) => Ok(objects),
            _ => unreachable!("objects reply"),
        }
    }

    /// Runs a script on the session. `database` is selected first when given.
    /// Writes follow the connection's read-only flag and safety policy.
    pub async fn query(
        &self,
        sql: String,
        database: Option<String>,
        confirmed: bool,
    ) -> Result<MySqlResult, MySqlSessionError> {
        match self
            .request(Request::Query {
                sql,
                database,
                confirmed,
            })
            .await?
        {
            Reply::Result(result) => Ok(result),
            _ => unreachable!("query reply"),
        }
    }

    /// One page of a table or view, ordered by its primary key when it has one.
    pub async fn browse(
        &self,
        database: String,
        table: String,
        offset: u64,
        limit: u32,
    ) -> Result<MySqlResult, MySqlSessionError> {
        match self
            .request(Request::Browse {
                database,
                table,
                offset,
                limit: limit.clamp(1, MYSQL_MAX_PAGE_ROWS),
            })
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
        match self.request(Request::Structure { database, table }).await? {
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
            .request(Request::Definition {
                database,
                kind,
                name,
            })
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

    /// Asks the server to stop the session's running statement (`KILL QUERY`
    /// on a short side connection). The session itself stays open.
    pub async fn cancel_mysql_query(&self, session: &MySqlSession) -> Result<(), String> {
        let (options, thread) = session.0.cancel.clone();
        self.call(move |_| async move {
            Ok(
                tokio::time::timeout(Duration::from_secs(5), worker::kill_query(&options, thread))
                    .await
                    .unwrap_or_else(|_| Err("Cancel timed out".into())),
            )
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
