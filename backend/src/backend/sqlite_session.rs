//! Plan 031 step 4: native SQLite sessions.
//!
//! One session owns one `SqliteConnection` on one worker task. Requests queue
//! on a bounded channel and run in order, so the object tree, table pages,
//! structure reads and the query tab share the connection (and any open
//! transaction or `ATTACH`) exactly like a SQLite shell does.
//!
//! Contracts:
//! - The file must already exist; the session never creates a database.
//!   Read-only connections open with `SQLITE_OPEN_READONLY`.
//! - Every result is bounded: rows per result set, result sets per
//!   execution, cell bytes and total retained bytes. Rows beyond a bound are
//!   still read (every statement runs) but only counted.
//! - User SQL passes the stored safety policy before it runs (read-only,
//!   protected and strict modes; confirmation for writes where required).
//! - Cancellation interrupts only the request it names, through SQLite's
//!   progress handler. Every request carries its own cancel flag, and
//!   tickets are unique across the process, so a stale ticket (another
//!   request, an older session) never cancels anything else. A request
//!   cancelled while queued never starts. Nothing retries automatically.
//! - `close` (and dropping the session) refuses queued work, interrupts the
//!   running request and joins the worker within a deadline; an unjoined
//!   worker is aborted.
//! - Confirmed overrides are audited once execution starts, so a script
//!   that commits some statements and then fails is still recorded.
//! - Connections match the sqlite3 shell: foreign key enforcement is off
//!   unless the user's SQL turns it on (`PRAGMA foreign_keys = ON`).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Column, Connection, Decode, Either, Executor, Row, Sqlite, SqliteConnection};
use sqlx::{TypeInfo, ValueRef};
use tokio::sync::{mpsc, oneshot};

use super::Backend;
use crate::postgres::sql_class::{
    classify_script_dialect, describe_script_dialect, SqlDialect, StatementClassSummary,
};
use crate::safety::policy::{
    assert_permitted, AuditDisposition, ResolvedSafetyPolicy, SafetyRefusal, WriteIntent,
};
use crate::{quote_double, StoredConnection};

/// Rows retained per result set (query tab).
pub const SQLITE_MAX_ROWS_PER_SET: usize = 2_000;
/// Result sets retained per execution.
pub const SQLITE_MAX_RESULT_SETS: usize = 16;
/// Bytes retained per execution across all cells.
pub const SQLITE_MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
/// Bytes kept from one text cell; longer values are cut and counted.
pub const SQLITE_MAX_CELL_BYTES: usize = 64 * 1024;
/// Bytes of a BLOB rendered as hex.
pub const SQLITE_BLOB_PREVIEW_BYTES: usize = 256;
/// Largest table page.
pub const SQLITE_MAX_PAGE_ROWS: u32 = 1_000;
/// Objects listed per kind per database in the tree.
pub const SQLITE_MAX_TREE_OBJECTS: usize = 5_000;
/// Largest accepted SQL text.
pub const SQLITE_MAX_SQL_BYTES: usize = 1024 * 1024;
const MAX_DATABASES: usize = 128;
const MAX_STRUCTURE_ROWS: usize = 2_000;
const MAX_DEFINITION_BYTES: usize = 64 * 1024;
const QUEUE: usize = 8;
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// VM instructions between progress callbacks (cancellation latency).
const PROGRESS_OPS: i32 = 1_000;
/// Tickets remembered when cancelled before they were submitted.
const EARLY_CANCELS: usize = 64;

/// Process-wide ticket source: a ticket never repeats across sessions, so a
/// Stop aimed at an older session's request cannot match a newer one.
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteSessionError {
    /// The session was closed or its worker stopped.
    Closed,
    /// The request queue is full; nothing was queued.
    Busy,
    /// The request was interrupted by `cancel`.
    Cancelled,
    Failed(String),
}

impl std::fmt::Display for SqliteSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("SQLite session is closed"),
            Self::Busy => f.write_str("SQLite session is busy; try again when it finishes"),
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteSessionInfo {
    pub connection_id: String,
    pub path: String,
    pub read_only: bool,
    pub sqlite_version: String,
}

/// One schema from `PRAGMA database_list`: `main`, `temp` or an attachment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqliteDatabase {
    pub name: String,
    /// Backing file; empty for in-memory and temporary databases.
    pub file: String,
    pub tables: Vec<SqliteObject>,
    pub views: Vec<SqliteObject>,
    pub indexes: Vec<SqliteObject>,
    pub triggers: Vec<SqliteObject>,
    /// More objects of some kind exist than the tree lists.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteObject {
    pub name: String,
    /// Owning table for indexes and triggers; the object itself otherwise.
    pub table: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqliteResultSet {
    pub columns: Vec<String>,
    /// `None` is SQL NULL; BLOBs render as `x'…'` hex previews.
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows the statement produced, retained or not.
    pub row_count: u64,
    pub omitted_rows: u64,
    pub truncated_cells: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteExecution {
    Completed {
        sets: Vec<SqliteResultSet>,
        /// `total_changes()` delta, so trigger changes are included.
        rows_affected: u64,
        omitted_sets: u32,
        /// The retained-bytes bound stopped retention.
        byte_limited: bool,
        elapsed_ms: u64,
    },
    /// The policy needs an explicit confirmation; nothing ran.
    NeedsConfirmation {
        statements: Vec<StatementClassSummary>,
    },
    /// The policy refuses these statements; nothing ran.
    Blocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlitePage {
    /// The rows kept. The retained-bytes bound may keep fewer than the
    /// requested limit (never none while rows remain), so the next page
    /// starts at `offset + set.rows.len()`.
    pub set: SqliteResultSet,
    pub offset: u64,
    pub has_more: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqliteStructure {
    /// `table` or `view`.
    pub kind: String,
    pub definition: Option<String>,
    pub columns: Vec<SqliteColumn>,
    pub indexes: Vec<SqliteIndex>,
    pub foreign_keys: Vec<SqliteForeignKey>,
    pub triggers: Vec<SqliteTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteColumn {
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    /// 1-based position in the primary key; 0 when not a key column.
    pub primary_key: u32,
    /// Generated (`2`/`3`) or hidden (`1`) column marker from `table_xinfo`.
    pub hidden: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteIndex {
    pub name: String,
    pub unique: bool,
    /// `c` (CREATE INDEX), `u` (UNIQUE constraint) or `pk`.
    pub origin: String,
    pub partial: bool,
    /// Column names; expression members appear as `<expression>`.
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteForeignKey {
    pub id: i64,
    pub columns: Vec<String>,
    pub table: String,
    /// Empty entries reference the parent's primary key.
    pub referenced: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteTrigger {
    pub name: String,
    pub definition: Option<String>,
}

/// What the worker needs to open and police a session.
#[derive(Debug, Clone)]
pub(crate) struct SqliteSessionConfig {
    pub(crate) connection_id: String,
    pub(crate) path: String,
    pub(crate) read_only: bool,
    pub(crate) policy: ResolvedSafetyPolicy,
    /// Confirmed overrides are audited here when present.
    pub(crate) audit: Option<sqlx::SqlitePool>,
}

enum Work {
    Objects(oneshot::Sender<Result<SqliteObjects, SqliteSessionError>>),
    Execute {
        sql: String,
        confirmed: bool,
        reply: oneshot::Sender<Result<SqliteExecution, SqliteSessionError>>,
    },
    Browse {
        schema: String,
        name: String,
        offset: u64,
        limit: u32,
        reply: oneshot::Sender<Result<SqlitePage, SqliteSessionError>>,
    },
    Structure {
        schema: String,
        name: String,
        reply: oneshot::Sender<Result<SqliteStructure, SqliteSessionError>>,
    },
}

impl Work {
    fn refuse(self, error: SqliteSessionError) {
        match self {
            Work::Objects(reply) => drop(reply.send(Err(error))),
            Work::Execute { reply, .. } => drop(reply.send(Err(error))),
            Work::Browse { reply, .. } => drop(reply.send(Err(error))),
            Work::Structure { reply, .. } => drop(reply.send(Err(error))),
        }
    }
}

struct Job {
    ticket: u64,
    /// This request's own cancel flag.
    cancel: Arc<AtomicBool>,
    work: Work,
}

/// A poisoned lock still guards plain data here; never panic on it (the
/// progress handler runs inside an SQLite callback).
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Requests {
    /// Cancel flags of queued and running requests, by ticket.
    live: HashMap<u64, Arc<AtomicBool>>,
    /// Tickets cancelled before they were submitted (oldest dropped first).
    early: VecDeque<u64>,
}

#[derive(Default)]
struct Shared {
    requests: Mutex<Requests>,
    /// Cancel flag of the request on the connection; `None` when idle.
    current: Mutex<Option<Arc<AtomicBool>>>,
    closing: AtomicBool,
}

impl Shared {
    /// Progress-handler verdict: false interrupts the running statement.
    fn keep_running(&self) -> bool {
        if self.closing.load(Ordering::Acquire) {
            return false;
        }
        !lock(&self.current)
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Acquire))
    }

    /// Registers a request's cancel flag; `None` when it was cancelled
    /// before it was submitted.
    fn admit(&self, ticket: u64) -> Option<Arc<AtomicBool>> {
        let mut requests = lock(&self.requests);
        if let Some(position) = requests.early.iter().position(|early| *early == ticket) {
            requests.early.remove(position);
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        requests.live.insert(ticket, cancel.clone());
        Some(cancel)
    }

    fn forget(&self, ticket: u64, cancel: &Arc<AtomicBool>) {
        let mut requests = lock(&self.requests);
        if requests
            .live
            .get(&ticket)
            .is_some_and(|live| Arc::ptr_eq(live, cancel))
        {
            requests.live.remove(&ticket);
        }
    }

    fn cancel(&self, ticket: u64) {
        let mut requests = lock(&self.requests);
        if let Some(cancel) = requests.live.get(&ticket) {
            cancel.store(true, Ordering::Release);
            return;
        }
        // Not submitted yet (or already finished): remember it so a late
        // submission is refused instead of running.
        if ticket == 0 || requests.early.contains(&ticket) {
            return;
        }
        if requests.early.len() >= EARLY_CANCELS {
            requests.early.pop_front();
        }
        requests.early.push_back(ticket);
    }
}

/// Tree contents for one session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SqliteObjects {
    pub databases: Vec<SqliteDatabase>,
}

pub struct SqliteSession {
    info: SqliteSessionInfo,
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    shared: Arc<Shared>,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl std::fmt::Debug for SqliteSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSession")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Drop for SqliteSession {
    fn drop(&mut self) {
        // An unclosed session must not leave a worker holding the file.
        // `closing` makes the progress handler interrupt the running
        // statement, which SQLite executes off this task and which aborting
        // the worker alone would not stop.
        self.shared.closing.store(true, Ordering::Release);
        lock(&self.jobs).take();
        if let Some(worker) = lock(&self.worker).take() {
            worker.abort();
        }
    }
}

impl Backend {
    /// Opens a session for a saved, supported SQLite connection. Resolution
    /// holds the development gate; the file check and open run after it is
    /// released, under their own deadline.
    pub async fn open_sqlite_session(&self, id: String) -> Result<SqliteSession, String> {
        let authority = self.development()?;
        let config = self
            .development_call(move |state| async move {
                Ok(async {
                    let rows = crate::storage::read_native_connections(&state.pool).await?;
                    let (connection, valid) = rows
                        .into_iter()
                        .find(|(connection, _)| connection.id() == id)
                        .ok_or("Connection no longer exists; reload and retry")?;
                    if !valid || !authority.permits(&connection) {
                        return Err::<_, String>(
                            "Stored connection options are unsupported or unreadable".into(),
                        );
                    }
                    let StoredConnection::SQLite(sqlite) = &connection else {
                        return Err("Connection is not a SQLite database".into());
                    };
                    Ok(SqliteSessionConfig {
                        connection_id: sqlite.id.clone(),
                        path: sqlite.database.clone(),
                        read_only: sqlite.read_only,
                        policy: crate::safety::gate::resolved_policy(&connection),
                        audit: Some(state.pool.clone()),
                    })
                }
                .await)
            })
            .await
            .map_err(|_| "Native backend is closing".to_string())??;
        tokio::time::timeout(
            OPEN_TIMEOUT,
            super::development::connections::engine_connections::check_sqlite_file(&config.path),
        )
        .await
        .map_err(|_| "SQLite database file check timed out".to_string())??;
        SqliteSession::open(config).await
    }
}

impl SqliteSession {
    pub(crate) async fn open(config: SqliteSessionConfig) -> Result<Self, String> {
        let options = SqliteConnectOptions::new()
            .filename(&config.path)
            .create_if_missing(false)
            .read_only(config.read_only)
            .busy_timeout(BUSY_TIMEOUT)
            // sqlx enables foreign keys by default; the sqlite3 shell (and
            // SQLite itself) does not. Match the shell so scripts behave as
            // they do there; users opt in with `PRAGMA foreign_keys = ON`.
            .foreign_keys(false)
            .statement_cache_capacity(32);
        let mut connection =
            tokio::time::timeout(OPEN_TIMEOUT, SqliteConnection::connect_with(&options))
                .await
                .map_err(|_| "Opening the SQLite database timed out".to_string())?
                .map_err(|error| format!("Could not open the SQLite database: {error}"))?;
        // Reading the schema proves the file is a database; SQLite opens
        // files lazily.
        let version = sqlx::query_scalar::<_, String>(
            "SELECT sqlite_version() FROM (SELECT count(*) FROM sqlite_master)",
        )
        .fetch_one(&mut connection)
        .await;
        let version = match version {
            Ok(version) => version,
            Err(error) => {
                let _ = connection.close().await;
                return Err(format!("SQLite database is unreadable: {error}"));
            }
        };
        let shared = Arc::new(Shared::default());
        {
            let shared = shared.clone();
            let mut handle = connection
                .lock_handle()
                .await
                .map_err(|error| format!("SQLite session failed to start: {error}"))?;
            handle.set_progress_handler(PROGRESS_OPS, move || shared.keep_running());
        }
        let info = SqliteSessionInfo {
            connection_id: config.connection_id.clone(),
            path: config.path.clone(),
            read_only: config.read_only,
            sqlite_version: version,
        };
        let (jobs, receiver) = mpsc::channel(QUEUE);
        let worker = tokio::spawn(run_worker(connection, receiver, shared.clone(), config));
        Ok(Self {
            info,
            jobs: Mutex::new(Some(jobs)),
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }

    pub fn info(&self) -> &SqliteSessionInfo {
        &self.info
    }

    /// A fresh request identity for `cancel`, unique across every session
    /// in the process.
    pub fn ticket(&self) -> u64 {
        NEXT_TICKET.fetch_add(1, Ordering::Relaxed)
    }

    /// Interrupts `ticket` if it is queued or running, or refuses it if it
    /// is submitted later; other requests are unaffected. Statements already
    /// committed stay committed.
    pub fn cancel(&self, ticket: u64) {
        self.shared.cancel(ticket);
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closing.load(Ordering::Acquire)
            || lock(&self.worker)
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
    }

    fn submit(&self, ticket: u64, work: Work) {
        let sender = lock(&self.jobs).clone();
        let Some(sender) = sender.filter(|_| !self.shared.closing.load(Ordering::Acquire)) else {
            return work.refuse(SqliteSessionError::Closed);
        };
        let Some(cancel) = self.shared.admit(ticket) else {
            return work.refuse(SqliteSessionError::Cancelled);
        };
        let job = Job {
            ticket,
            cancel: cancel.clone(),
            work,
        };
        if let Err(error) = sender.try_send(job) {
            self.shared.forget(ticket, &cancel);
            match error {
                mpsc::error::TrySendError::Full(job) => job.work.refuse(SqliteSessionError::Busy),
                mpsc::error::TrySendError::Closed(job) => {
                    job.work.refuse(SqliteSessionError::Closed)
                }
            }
        }
    }

    async fn request<T>(
        &self,
        ticket: u64,
        work: impl FnOnce(oneshot::Sender<Result<T, SqliteSessionError>>) -> Work,
    ) -> Result<T, SqliteSessionError> {
        let (reply, receiver) = oneshot::channel();
        self.submit(ticket, work(reply));
        receiver.await.unwrap_or(Err(SqliteSessionError::Closed))
    }

    pub async fn objects(&self, ticket: u64) -> Result<SqliteObjects, SqliteSessionError> {
        self.request(ticket, Work::Objects).await
    }

    pub async fn execute(
        &self,
        ticket: u64,
        sql: String,
        confirmed: bool,
    ) -> Result<SqliteExecution, SqliteSessionError> {
        if sql.len() > SQLITE_MAX_SQL_BYTES {
            return Err(SqliteSessionError::Failed(format!(
                "SQL is larger than {} KiB",
                SQLITE_MAX_SQL_BYTES / 1024
            )));
        }
        self.request(ticket, |reply| Work::Execute {
            sql,
            confirmed,
            reply,
        })
        .await
    }

    pub async fn browse(
        &self,
        ticket: u64,
        schema: String,
        name: String,
        offset: u64,
        limit: u32,
    ) -> Result<SqlitePage, SqliteSessionError> {
        self.request(ticket, |reply| Work::Browse {
            schema,
            name,
            offset,
            limit: limit.clamp(1, SQLITE_MAX_PAGE_ROWS),
            reply,
        })
        .await
    }

    pub async fn structure(
        &self,
        ticket: u64,
        schema: String,
        name: String,
    ) -> Result<SqliteStructure, SqliteSessionError> {
        self.request(ticket, |reply| Work::Structure {
            schema,
            name,
            reply,
        })
        .await
    }

    /// Refuses queued work, interrupts the running request and joins the
    /// worker (which closes the connection, rolling back an open
    /// transaction). Past `deadline` the worker is aborted and an error is
    /// returned; the session is closed either way.
    pub async fn close(&self, deadline: Duration) -> Result<(), String> {
        // The progress handler sees `closing` and interrupts the running
        // request; the worker refuses everything still queued.
        self.shared.closing.store(true, Ordering::Release);
        lock(&self.jobs).take();
        let worker = lock(&self.worker).take();
        let Some(mut worker) = worker else {
            return Ok(());
        };
        match tokio::time::timeout(deadline, &mut worker).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err("SQLite session worker failed".into()),
            Err(_) => {
                worker.abort();
                let _ = worker.await;
                Err("SQLite session did not close in time; it was stopped".into())
            }
        }
    }
}

async fn run_worker(
    mut connection: SqliteConnection,
    mut jobs: mpsc::Receiver<Job>,
    shared: Arc<Shared>,
    config: SqliteSessionConfig,
) {
    while let Some(Job {
        ticket,
        cancel,
        work,
    }) = jobs.recv().await
    {
        if shared.closing.load(Ordering::Acquire) {
            shared.forget(ticket, &cancel);
            work.refuse(SqliteSessionError::Closed);
            continue;
        }
        // Stopped while queued: it never starts.
        if cancel.load(Ordering::Acquire) {
            shared.forget(ticket, &cancel);
            work.refuse(SqliteSessionError::Cancelled);
            continue;
        }
        *lock(&shared.current) = Some(cancel.clone());
        match work {
            Work::Objects(reply) => {
                let result = outcome(&shared, &cancel, read_objects(&mut connection).await);
                let _ = reply.send(result);
            }
            Work::Execute {
                sql,
                confirmed,
                reply,
            } => {
                let result = outcome(
                    &shared,
                    &cancel,
                    execute(&mut connection, &config, &sql, confirmed).await,
                );
                let _ = reply.send(result);
            }
            Work::Browse {
                schema,
                name,
                offset,
                limit,
                reply,
            } => {
                let result = outcome(
                    &shared,
                    &cancel,
                    browse(&mut connection, &schema, &name, offset, limit).await,
                );
                let _ = reply.send(result);
            }
            Work::Structure {
                schema,
                name,
                reply,
            } => {
                let result = outcome(
                    &shared,
                    &cancel,
                    read_structure(&mut connection, &schema, &name).await,
                );
                let _ = reply.send(result);
            }
        }
        *lock(&shared.current) = None;
        shared.forget(ticket, &cancel);
    }
    let _ = connection.close().await;
}

/// A failure caused by `close` or `cancel` reports that cause, not SQLite's
/// "interrupted".
fn outcome<T>(
    shared: &Shared,
    cancel: &AtomicBool,
    result: Result<T, String>,
) -> Result<T, SqliteSessionError> {
    match result {
        Ok(value) => Ok(value),
        Err(_) if shared.closing.load(Ordering::Acquire) => Err(SqliteSessionError::Closed),
        Err(_) if cancel.load(Ordering::Acquire) => Err(SqliteSessionError::Cancelled),
        Err(error) => Err(SqliteSessionError::Failed(error)),
    }
}

fn database_error(error: sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(error) => error.message().to_string(),
        other => other.to_string(),
    }
}

/// Cuts at a char boundary at most `limit` bytes in.
fn cut(text: &str, limit: usize) -> (&str, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn blob_preview(bytes: &[u8]) -> (String, bool) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let shown = &bytes[..bytes.len().min(SQLITE_BLOB_PREVIEW_BYTES)];
    let mut text = String::with_capacity(shown.len() * 2 + 24);
    text.push_str("x'");
    for byte in shown {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0x0f) as usize] as char);
    }
    text.push('\'');
    let truncated = shown.len() < bytes.len();
    if truncated {
        text.push_str(&format!(" … {} bytes", bytes.len()));
    }
    (text, truncated)
}

/// The cell as text by its runtime storage class. Returns (value, truncated).
fn cell(row: &SqliteRow, index: usize) -> (Option<String>, bool) {
    let Ok(raw) = row.try_get_raw(index) else {
        return (None, false);
    };
    if raw.is_null() {
        return (None, false);
    }
    let kind = raw.type_info().name().to_owned();
    match kind.as_str() {
        "INTEGER" => match <i64 as Decode<Sqlite>>::decode(raw) {
            Ok(value) => (Some(value.to_string()), false),
            Err(_) => (None, false),
        },
        "REAL" => match <f64 as Decode<Sqlite>>::decode(raw) {
            Ok(value) => (Some(format_real(value)), false),
            Err(_) => (None, false),
        },
        "BLOB" => match <&[u8] as Decode<Sqlite>>::decode(raw) {
            Ok(bytes) => {
                let (text, truncated) = blob_preview(bytes);
                (Some(text), truncated)
            }
            Err(_) => (None, false),
        },
        _ => match <&[u8] as Decode<Sqlite>>::decode(raw) {
            Ok(bytes) => {
                // TEXT is not guaranteed to be valid UTF-8 in SQLite.
                let text = String::from_utf8_lossy(bytes);
                let (kept, truncated) = cut(&text, SQLITE_MAX_CELL_BYTES);
                (Some(kept.to_owned()), truncated)
            }
            Err(_) => (None, false),
        },
    }
}

/// SQLite prints whole REALs with a trailing `.0`; keep that distinction.
fn format_real(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

/// Bounded result collection shared by the query tab and table pages.
struct Collector {
    sets: Vec<SqliteResultSet>,
    current: Option<SqliteResultSet>,
    omitted_sets: u32,
    bytes: usize,
    byte_limited: bool,
    rows_per_set: usize,
    /// Keep the first row even past the byte bound, so a table page always
    /// advances (one row is bounded by the per-cell limits).
    keep_first_row: bool,
}

impl Collector {
    fn new(rows_per_set: usize) -> Self {
        Self {
            sets: Vec::new(),
            current: None,
            omitted_sets: 0,
            bytes: 0,
            byte_limited: false,
            rows_per_set,
            keep_first_row: false,
        }
    }

    fn row(&mut self, row: &SqliteRow) {
        let set = self.current.get_or_insert_with(|| SqliteResultSet {
            columns: row
                .columns()
                .iter()
                .map(|column| column.name().to_owned())
                .collect(),
            ..Default::default()
        });
        set.row_count += 1;
        if set.rows.len() >= self.rows_per_set || self.byte_limited {
            set.omitted_rows += 1;
            return;
        }
        let mut values = Vec::with_capacity(row.len());
        let mut bytes = 0usize;
        let mut truncated = 0u64;
        for index in 0..row.len() {
            let (value, cut) = cell(row, index);
            bytes += value.as_ref().map_or(1, String::len) + 8;
            truncated += u64::from(cut);
            values.push(value);
        }
        let first = self.keep_first_row && self.sets.is_empty() && set.rows.is_empty();
        if !first && self.bytes.saturating_add(bytes) > SQLITE_MAX_RESULT_BYTES {
            self.byte_limited = true;
            set.omitted_rows += 1;
            return;
        }
        self.bytes += bytes;
        set.truncated_cells += truncated;
        set.rows.push(values);
    }

    fn finish_statement(&mut self) {
        if let Some(set) = self.current.take() {
            if self.sets.len() < SQLITE_MAX_RESULT_SETS {
                self.sets.push(set);
            } else {
                self.omitted_sets += 1;
            }
        }
    }
}

async fn total_changes(connection: &mut SqliteConnection) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>("SELECT total_changes()")
        .fetch_one(&mut *connection)
        .await
        .map_err(database_error)
}

/// Policy first (nothing runs on a refusal), then every statement in order.
pub(crate) async fn execute(
    connection: &mut SqliteConnection,
    config: &SqliteSessionConfig,
    sql: &str,
    confirmed: bool,
) -> Result<SqliteExecution, String> {
    let intent = WriteIntent::Statement {
        classes: classify_script_dialect(sql, SqlDialect::Sqlite),
    };
    let authorization = match assert_permitted(&config.policy, &intent, confirmed) {
        Ok(authorization) => authorization,
        Err(SafetyRefusal::Blocked { reason, .. }) => {
            return Ok(SqliteExecution::Blocked {
                reason: reason.into(),
            })
        }
        Err(SafetyRefusal::NeedsConfirmation { statements }) => {
            return Ok(SqliteExecution::NeedsConfirmation { statements })
        }
    };
    let started = Instant::now();
    let before = total_changes(connection).await?;
    let mut collector = Collector::new(SQLITE_MAX_ROWS_PER_SET);
    let mut failure = None;
    {
        let mut stream = sqlx::raw_sql(sql).fetch_many(&mut *connection);
        loop {
            match stream.try_next().await {
                Ok(Some(Either::Left(_))) => collector.finish_statement(),
                Ok(Some(Either::Right(row))) => collector.row(&row),
                Ok(None) => break,
                Err(error) => {
                    failure = Some(database_error(error));
                    break;
                }
            }
        }
    }
    // Audit as soon as execution has started, whatever its outcome: earlier
    // statements may have committed before a later one failed or was
    // interrupted, and an override that changed data must be on record.
    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
        if let Some(pool) = &config.audit {
            crate::safety::gate::record_override(
                pool,
                &config.connection_id,
                "sqlite_execute",
                &intent,
            )
            .await;
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    collector.finish_statement();
    // A single read that returned no rows still shows its column header.
    if collector.sets.is_empty()
        && describe_script_dialect(sql, SqlDialect::Sqlite).is_ok_and(|statements| {
            statements.len() == 1
                && matches!(
                    statements[0].class,
                    crate::postgres::sql_class::StatementClass::Read
                )
        })
    {
        if let Ok(described) = (&mut *connection).describe(sql).await {
            let columns: Vec<String> = described
                .columns()
                .iter()
                .map(|column| column.name().to_owned())
                .collect();
            if !columns.is_empty() {
                collector.sets.push(SqliteResultSet {
                    columns,
                    ..Default::default()
                });
            }
        }
    }
    let after = total_changes(connection).await?;
    Ok(SqliteExecution::Completed {
        sets: collector.sets,
        rows_affected: u64::try_from(after.saturating_sub(before)).unwrap_or(0),
        omitted_sets: collector.omitted_sets,
        byte_limited: collector.byte_limited,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    })
}

/// `"schema"."name"`; both parts are quoted, never interpolated raw.
fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_double(schema), quote_double(name))
}

async fn browse(
    connection: &mut SqliteConnection,
    schema: &str,
    name: &str,
    offset: u64,
    limit: u32,
) -> Result<SqlitePage, String> {
    let limit = limit.clamp(1, SQLITE_MAX_PAGE_ROWS);
    let started = Instant::now();
    // One extra row tells whether a next page exists. Natural order: rowid
    // order for rowid tables, key order for WITHOUT ROWID tables.
    let sql = format!(
        "SELECT * FROM {} LIMIT {} OFFSET {}",
        qualified(schema, name),
        u64::from(limit) + 1,
        offset.min(i64::MAX as u64)
    );
    let mut collector = Collector::new(limit as usize + 1);
    collector.keep_first_row = true;
    {
        let mut stream = sqlx::query(&sql).persistent(false).fetch(&mut *connection);
        while let Some(row) = stream.try_next().await.map_err(database_error)? {
            collector.row(&row);
        }
    }
    collector.finish_statement();
    let mut set = collector.sets.pop().unwrap_or_default();
    if set.columns.is_empty() {
        // An empty page still names its columns.
        set.columns = read_column_names(connection, schema, name).await?;
    }
    // The page is the rows kept: the 16 MiB bound may keep fewer than
    // `limit`. Rows read but not kept are not lost; they start the next
    // page, which begins at `offset + rows.len()`.
    set.rows.truncate(limit as usize);
    let kept = set.rows.len() as u64;
    let has_more = set.row_count > kept;
    set.row_count = kept;
    set.omitted_rows = 0;
    Ok(SqlitePage {
        set,
        offset,
        has_more,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    })
}

async fn read_column_names(
    connection: &mut SqliteConnection,
    schema: &str,
    name: &str,
) -> Result<Vec<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_xinfo(?1, ?2) ORDER BY cid")
        .bind(name)
        .bind(schema)
        .fetch_all(&mut *connection)
        .await
        .map_err(database_error)
}

async fn read_objects(connection: &mut SqliteConnection) -> Result<SqliteObjects, String> {
    let databases: Vec<(String, String)> = sqlx::query_as::<_, (i64, String, Option<String>)>(
        "SELECT seq, name, file FROM pragma_database_list ORDER BY seq",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?
    .into_iter()
    .take(MAX_DATABASES)
    .map(|(_, name, file)| (name, file.unwrap_or_default()))
    .collect();
    let mut result = SqliteObjects::default();
    for (name, file) in databases {
        // `sqlite_%` names are SQLite's own (sequence, stat, autoindex).
        let sql = format!(
            "SELECT type, name, tbl_name FROM {}.sqlite_master \
             WHERE type IN ('table', 'view', 'index', 'trigger') \
             AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             ORDER BY type, name COLLATE NOCASE",
            quote_double(&name)
        );
        let mut database = SqliteDatabase {
            name,
            file,
            ..Default::default()
        };
        let mut stream =
            sqlx::query_as::<_, (String, String, String)>(&sql).fetch(&mut *connection);
        while let Some((kind, object, table)) = stream.try_next().await.map_err(database_error)? {
            let bucket = match kind.as_str() {
                "table" => &mut database.tables,
                "view" => &mut database.views,
                "index" => &mut database.indexes,
                "trigger" => &mut database.triggers,
                _ => continue,
            };
            if bucket.len() >= SQLITE_MAX_TREE_OBJECTS {
                database.truncated = true;
                continue;
            }
            bucket.push(SqliteObject {
                name: object,
                table,
            });
        }
        drop(stream);
        result.databases.push(database);
    }
    Ok(result)
}

fn bounded_definition(sql: Option<String>) -> Option<String> {
    sql.map(|sql| {
        let (kept, truncated) = cut(&sql, MAX_DEFINITION_BYTES);
        if truncated {
            format!("{kept}\n-- definition truncated")
        } else {
            sql
        }
    })
}

async fn read_structure(
    connection: &mut SqliteConnection,
    schema: &str,
    name: &str,
) -> Result<SqliteStructure, String> {
    let master = format!("{}.sqlite_master", quote_double(schema));
    let (kind, definition) = sqlx::query_as::<_, (String, Option<String>)>(&format!(
        "SELECT type, sql FROM {master} WHERE name = ?1 AND type IN ('table', 'view')"
    ))
    .bind(name)
    .fetch_optional(&mut *connection)
    .await
    .map_err(database_error)?
    .ok_or_else(|| format!("{schema}.{name} no longer exists"))?;
    let columns = sqlx::query_as::<_, (String, String, i64, Option<String>, i64, i64)>(
        "SELECT name, type, \"notnull\", dflt_value, pk, hidden \
         FROM pragma_table_xinfo(?1, ?2) ORDER BY cid LIMIT ?3",
    )
    .bind(name)
    .bind(schema)
    .bind(MAX_STRUCTURE_ROWS as i64)
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(
        |(name, declared_type, not_null, default_value, primary_key, hidden)| SqliteColumn {
            name,
            declared_type,
            not_null: not_null != 0,
            default_value,
            primary_key: u32::try_from(primary_key).unwrap_or(0),
            hidden: u32::try_from(hidden).unwrap_or(0),
        },
    )
    .collect();
    let index_rows = sqlx::query_as::<_, (String, i64, String, i64)>(
        "SELECT name, \"unique\", origin, partial FROM pragma_index_list(?1, ?2) \
         ORDER BY name LIMIT ?3",
    )
    .bind(name)
    .bind(schema)
    .bind(MAX_STRUCTURE_ROWS as i64)
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?;
    let mut indexes = Vec::with_capacity(index_rows.len());
    for (index, unique, origin, partial) in index_rows {
        let members = sqlx::query_scalar::<_, Option<String>>(
            "SELECT name FROM pragma_index_info(?1, ?2) ORDER BY seqno LIMIT ?3",
        )
        .bind(&index)
        .bind(schema)
        .bind(MAX_STRUCTURE_ROWS as i64)
        .fetch_all(&mut *connection)
        .await
        .map_err(database_error)?;
        indexes.push(SqliteIndex {
            name: index,
            unique: unique != 0,
            origin,
            partial: partial != 0,
            columns: members
                .into_iter()
                .map(|member| member.unwrap_or_else(|| "<expression>".into()))
                .collect(),
        });
    }
    let key_rows = sqlx::query_as::<_, (i64, String, String, Option<String>, String, String)>(
        "SELECT id, \"table\", \"from\", \"to\", on_update, on_delete \
         FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq LIMIT ?3",
    )
    .bind(name)
    .bind(schema)
    .bind(MAX_STRUCTURE_ROWS as i64)
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?;
    let mut foreign_keys: Vec<SqliteForeignKey> = Vec::new();
    for (id, table, from, to, on_update, on_delete) in key_rows {
        match foreign_keys.last_mut() {
            Some(key) if key.id == id => {
                key.columns.push(from);
                key.referenced.push(to.unwrap_or_default());
            }
            _ => foreign_keys.push(SqliteForeignKey {
                id,
                columns: vec![from],
                table,
                referenced: vec![to.unwrap_or_default()],
                on_update,
                on_delete,
            }),
        }
    }
    let triggers = sqlx::query_as::<_, (String, Option<String>)>(&format!(
        "SELECT name, sql FROM {master} WHERE type = 'trigger' AND tbl_name = ?1 \
         ORDER BY name LIMIT ?2"
    ))
    .bind(name)
    .bind(MAX_STRUCTURE_ROWS as i64)
    .fetch_all(&mut *connection)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(name, definition)| SqliteTrigger {
        name,
        definition: bounded_definition(definition),
    })
    .collect();
    Ok(SqliteStructure {
        kind,
        definition: bounded_definition(definition),
        columns,
        indexes,
        foreign_keys,
        triggers,
    })
}

#[cfg(test)]
#[path = "sqlite_session_tests.rs"]
mod tests;
