//! Plan 031 step 4: native ClickHouse sessions.
//!
//! ClickHouse is reached over stateless HTTP, so a "session" is the resolved
//! connection (stored record, cached secret and, for tunnelled connections,
//! an owned SSH route) plus its safety policy. Opening one runs a single
//! bounded `SELECT 1`; nothing reconnects or retries on its own. Every read is
//! bounded by rows, bytes and time ([`crate::clickhouse::bounded`]), and every
//! statement passes the connection's read-only/safe-mode policy first.
//!
//! The session holds no socket at rest. Closing it (or dropping the last
//! handle) releases the route on the blocking pool.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use super::{bastions, Backend, Inner};
use crate::clickhouse::bounded::{self, Limits};
use crate::postgres::sql_class::{describe_script, StatementClass};
use crate::safety::gate;
use crate::safety::policy::{
    assert_permitted, resolve_policy, AuditDisposition, ResolvedSafetyPolicy, SafetyRefusal,
    WriteIntent,
};
use crate::{credentials, storage, DatabaseEngine, StoredConnection, TableStructure};

pub use crate::clickhouse::bounded::{
    ClickHouseColumn, ClickHouseError, ClickHouseErrorKind, ClickHouseRows, ClickHouseTruncation,
};
pub use crate::clickhouse::catalog::{
    ClickHouseCatalog, ClickHouseDatabase, ClickHouseDictionary, ClickHouseMaterializedView,
    ClickHouseTable,
};
pub use crate::postgres::sql_class::StatementClassSummary;

/// Rows kept from one query document run.
pub const QUERY_MAX_ROWS: usize = 10_000;
/// Bytes read from one query response (before decoding).
pub const QUERY_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Rows per table-browse page.
pub const BROWSE_PAGE_ROWS: usize = 200;
const QUERY_TIMEOUT: Duration = Duration::from_secs(300);
const BROWSE_MAX_BYTES: usize = 16 * 1024 * 1024;
const BROWSE_TIMEOUT: Duration = Duration::from_secs(60);
const STRUCTURE_TIMEOUT: Duration = Duration::from_secs(30);
/// Sorting keys remembered per session for browse ordering.
const SORTING_KEY_CACHE: usize = 256;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const AUDIT_COMMAND: &str = "clickhouse_query";

/// An open native ClickHouse session. Cheap to clone; all clones share one
/// route and one closed flag.
#[derive(Clone)]
pub struct ClickHouseSession(Arc<Session>);

struct Session {
    id: String,
    connection: StoredConnection,
    policy: ResolvedSafetyPolicy,
    pool: sqlx::SqlitePool,
    backend: Weak<Inner>,
    route: std::sync::Mutex<Option<bastions::ProbeRoute>>,
    closed: AtomicBool,
    runtime: tokio::runtime::Handle,
    /// Query ids of requests in flight. Whoever removes an id (the request
    /// finishing, being dropped, an explicit cancel, or close) decides
    /// whether a `KILL QUERY` is sent, so each id is killed at most once.
    running: std::sync::Mutex<HashSet<String>>,
    /// `(database, table)` → `system.tables.sorting_key` (empty when none).
    sorting_keys: std::sync::Mutex<HashMap<(String, String), String>>,
}

impl Session {
    /// Removes `query_id` from the running set; true when it was there.
    fn take_running(&self, query_id: &str) -> bool {
        self.running
            .lock()
            .map(|mut running| running.remove(query_id))
            .unwrap_or(false)
    }

    /// Best-effort `KILL QUERY` on the session's runtime. The task holds the
    /// session, so a tunnelled route stays up until the request settles.
    fn spawn_kill(self: &Arc<Self>, query_id: String) {
        let session = Arc::clone(self);
        self.runtime.spawn(async move {
            bounded::kill(&session.connection, &query_id).await;
        });
    }
}

/// One registered request. Dropped before [`Self::finish`] (the caller's
/// task was aborted: stop, document closed, session ended) or finished with
/// the server possibly still running it (client timeout, lost stream), it
/// sends a best-effort `KILL QUERY` for its id while the route is still up.
struct Running {
    session: Arc<Session>,
    query_id: String,
    settled: bool,
}

impl Running {
    fn new(session: &Arc<Session>, query_id: &str) -> Self {
        if let Ok(mut running) = session.running.lock() {
            running.insert(query_id.to_owned());
        }
        Self {
            session: Arc::clone(session),
            query_id: query_id.to_owned(),
            settled: false,
        }
    }

    fn finish<T>(mut self, result: &Result<T, ClickHouseError>) {
        self.settled = true;
        if self.session.take_running(&self.query_id) && may_still_run(result) {
            self.session.spawn_kill(std::mem::take(&mut self.query_id));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.settled && self.session.take_running(&self.query_id) {
            self.session.spawn_kill(std::mem::take(&mut self.query_id));
        }
    }
}

/// True when the client stopped waiting but the server may still be
/// executing: a client timeout, a dropped stream or an undecodable one.
fn may_still_run<T>(result: &Result<T, ClickHouseError>) -> bool {
    matches!(
        result,
        Err(ClickHouseError {
            kind: ClickHouseErrorKind::Timeout
                | ClickHouseErrorKind::Lost
                | ClickHouseErrorKind::Protocol,
            ..
        })
    )
}

/// Time left before `deadline`, or a timeout error once it has passed.
fn remaining(deadline: tokio::time::Instant) -> Result<Duration, ClickHouseError> {
    let left = deadline.saturating_duration_since(tokio::time::Instant::now());
    if left.is_zero() {
        return Err(ClickHouseError::new(
            ClickHouseErrorKind::Timeout,
            "ClickHouse request timed out",
        ));
    }
    Ok(left)
}

impl Drop for Session {
    fn drop(&mut self) {
        // Route teardown joins forwarding threads; keep it off UI threads.
        if let Some(route) = self.route.get_mut().ok().and_then(Option::take) {
            self.runtime.spawn_blocking(move || drop(route));
        }
    }
}

/// Outcome of a gated statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClickHouseQueryOutcome {
    Rows(ClickHouseRows),
    /// Safe mode asks for confirmation; rerun with `confirmed`.
    NeedsConfirmation(Vec<StatementClassSummary>),
}

/// One object's structure: columns, keys, skip indexes and stored DDL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClickHouseStructure {
    pub engine: String,
    pub ddl: String,
    pub total_rows: Option<u64>,
    pub total_bytes: Option<u64>,
    pub columns: Vec<ClickHouseStructureColumn>,
    /// The sorting key, ClickHouse's closest analogue to a primary key. It
    /// orders data and does not enforce uniqueness.
    pub sorting_key: Vec<String>,
    pub partition_by: Option<String>,
    pub sample_by: Option<String>,
    pub skip_indexes: Vec<ClickHouseSkipIndex>,
    /// CHECK constraints as `(name, expression)`.
    pub constraints: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClickHouseStructureColumn {
    pub name: String,
    pub type_name: String,
    /// `DEFAULT x`, `MATERIALIZED x`, `ALIAS x` or `EPHEMERAL x`.
    pub default: Option<String>,
    pub in_sorting_key: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClickHouseSkipIndex {
    pub name: String,
    pub expression: String,
    /// `minmax`, `set`, `bloom_filter`, …
    pub kind: String,
}

fn structure_of(structure: TableStructure) -> ClickHouseStructure {
    ClickHouseStructure {
        columns: structure
            .columns
            .into_iter()
            .map(|column| ClickHouseStructureColumn {
                name: column.name,
                type_name: column.data_type,
                default: column.default_value,
                in_sorting_key: column.is_primary_key,
            })
            .collect(),
        sorting_key: structure.primary_key.unwrap_or_default(),
        partition_by: structure.partition_by,
        sample_by: structure.sample_by,
        skip_indexes: structure
            .indexes
            .into_iter()
            .map(|index| ClickHouseSkipIndex {
                name: index.name,
                expression: index.columns.join(", "),
                kind: index.method.unwrap_or_default(),
            })
            .collect(),
        constraints: structure
            .constraints
            .into_iter()
            .map(|constraint| (constraint.name, constraint.definition))
            .collect(),
        ..Default::default()
    }
}

impl Backend {
    /// Resolves a saved ClickHouse connection and proves it with one bounded
    /// `SELECT 1`. Failure messages are classified, never server text or
    /// secrets.
    pub async fn open_clickhouse_session(
        &self,
        id: String,
    ) -> Result<ClickHouseSession, ClickHouseError> {
        let authority = self.development().map_err(ClickHouseError::refused)?;
        let inner = Arc::downgrade(&self.0);
        let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
        let resolved = self
            .development_call(move |state| async move {
                Ok(async {
                    let _guard = credentials::mutation_guard(&state.credentials).await;
                    let rows = storage::read_native_connections(&state.pool).await?;
                    let (connection, valid) = rows
                        .into_iter()
                        .find(|(connection, _)| connection.id() == id)
                        .ok_or("Connection no longer exists; reload and retry")?;
                    if connection.engine() != DatabaseEngine::ClickHouse {
                        return Err("Not a ClickHouse connection".to_string());
                    }
                    if !valid || !authority.permits(&connection) {
                        return Err("Connection fields are outside supported limits".into());
                    }
                    if !credentials::onboarding_completed(&state.pool).await? {
                        return Err("Configure credential storage before connecting".into());
                    }
                    let mode = crate::app::current_credential_mode(&state).await?;
                    let secrets = credentials::read_all_cached(&state.credentials, mode).await?;
                    let mut connection = connection;
                    connection.set_password(secrets.get(&id).cloned().unwrap_or_default());
                    let routed = bastions::route_probe(&state, connection, deadline)
                        .await
                        .map_err(|failure| route_failure(&failure))?;
                    Ok((id, routed, state.pool.clone()))
                }
                .await)
            })
            .await
            .map_err(|_| ClickHouseError::refused("Native backend is closing"))?
            .map_err(ClickHouseError::refused)?;
        let (id, (route, connection), pool) = resolved;
        let session = ClickHouseSession(Arc::new(Session {
            id,
            policy: resolve_policy(connection.policy()),
            connection,
            pool,
            backend: inner,
            route: std::sync::Mutex::new(Some(route)),
            closed: AtomicBool::new(false),
            runtime: tokio::runtime::Handle::current(),
            running: std::sync::Mutex::new(HashSet::new()),
            sorting_keys: std::sync::Mutex::new(HashMap::new()),
        }));
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let limits = Limits {
            max_rows: 1,
            max_bytes: 4096,
            timeout: remaining.max(Duration::from_millis(1)),
        };
        if let Err(error) = bounded::run(&session.0.connection, "SELECT 1", None, limits).await {
            session.close().await;
            return Err(connect_failure(&error));
        }
        Ok(session)
    }
}

/// Classified connect failure: kind plus at most the ClickHouse error code.
pub(crate) fn connect_failure(error: &ClickHouseError) -> ClickHouseError {
    let message = match error.kind {
        ClickHouseErrorKind::Lost => "Could not reach ClickHouse".to_string(),
        ClickHouseErrorKind::Timeout => "Connection timed out".to_string(),
        ClickHouseErrorKind::Refused => error.message.clone(),
        ClickHouseErrorKind::Protocol => "Endpoint did not answer like ClickHouse".to_string(),
        ClickHouseErrorKind::Server => match server_code(&error.message) {
            Some(516 | 192 | 193 | 194) => "Authentication failed".to_string(),
            Some(81) => "Database does not exist".to_string(),
            Some(code) => format!("ClickHouse refused the connection (code {code})"),
            None => "ClickHouse refused the connection".to_string(),
        },
    };
    ClickHouseError::new(error.kind, message)
}

fn route_failure(failure: &super::DevelopmentConnectionFailure) -> String {
    use super::DevelopmentConnectionFailure as Failure;
    match failure {
        Failure::SshHostKey => {
            "Bastion host key is untrusted or changed; review it under Bastion Servers".into()
        }
        Failure::Timeout => "SSH route timed out".into(),
        _ => "SSH route could not be established".into(),
    }
}

fn server_code(message: &str) -> Option<u32> {
    let rest = &message[message.find("Code: ")? + 6..];
    let digits = rest
        .bytes()
        .take_while(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    digits.parse().ok()
}

/// Backtick-quotes an identifier for ClickHouse.
pub fn quote_identifier(name: &str) -> String {
    format!("`{}`", name.replace('\\', "\\\\").replace('`', "\\`"))
}

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Statement classes for ClickHouse SQL. The shared classifier decides the
/// common heads; ClickHouse-only heads are mapped here. A script that does not
/// lex is one `Unknown` statement, which policy treats as a write.
///
/// Destructive (confirmed in protected mode): `DETACH`, `SYSTEM`, `KILL`,
/// `RENAME`, `EXCHANGE`, `REPLACE`, `CREATE OR REPLACE`, `OPTIMIZE …
/// DEDUPLICATE`, and an `ALTER` whose top-level actions delete, rewrite or
/// move data (`DELETE`, `UPDATE`, `DROP …`, `CLEAR …`, `DETACH …`,
/// `REPLACE …`, `MOVE …`, `UNFREEZE`, `TTL`). A read that calls a table
/// function reaching outside the server (`url(`, `s3(`, `remote(`, …) is
/// classed as a write, so read-only refuses it.
pub(crate) fn classify(sql: &str) -> Vec<StatementClass> {
    let Ok(statements) = describe_script(sql) else {
        return vec![StatementClass::Unknown];
    };
    statements
        .into_iter()
        .map(|statement| {
            let Some(words) = sql
                .get(statement.start..statement.end)
                .and_then(|text| top_level_words(text).ok())
            else {
                return StatementClass::Unknown;
            };
            let class = match statement.head.as_deref() {
                Some("DESCRIBE" | "DESC" | "EXISTS" | "EXPLAIN" | "SHOW") => StatementClass::Read,
                Some("ATTACH" | "UNDROP") => StatementClass::Ddl { destructive: false },
                Some("OPTIMIZE") => StatementClass::Ddl {
                    destructive: words.depth0.iter().any(|word| word == "DEDUPLICATE"),
                },
                Some("SYSTEM" | "RENAME" | "EXCHANGE" | "KILL" | "DETACH" | "REPLACE") => {
                    StatementClass::Ddl { destructive: true }
                }
                Some("ALTER") => StatementClass::Ddl {
                    destructive: alter_is_destructive(&words.depth0),
                },
                Some("CREATE") => StatementClass::Ddl {
                    destructive: words.depth0.get(1).map(String::as_str) == Some("OR")
                        && words.depth0.get(2).map(String::as_str) == Some("REPLACE"),
                },
                Some("USE") => StatementClass::Session,
                _ => statement.class,
            };
            if class == StatementClass::Read && words.external_table_function {
                // Reads another system; never a proven read.
                return StatementClass::Dml {
                    unbounded: false,
                    destructive: false,
                };
            }
            class
        })
        .collect()
}

/// Table functions that read (or write) outside the ClickHouse server.
const EXTERNAL_TABLE_FUNCTIONS: &[&str] = &[
    "url",
    "urlcluster",
    "s3",
    "s3cluster",
    "gcs",
    "oss",
    "cosn",
    "file",
    "filecluster",
    "remote",
    "remotesecure",
    "cluster",
    "clusterallreplicas",
    "mysql",
    "postgresql",
    "sqlite",
    "mongodb",
    "redis",
    "jdbc",
    "odbc",
    "hdfs",
    "hdfscluster",
    "azureblobstorage",
    "azureblobstoragecluster",
    "iceberg",
    "icebergs3",
    "icebergazure",
    "iceberghdfs",
    "deltalake",
    "hudi",
    "executable",
    "input",
];

struct StatementWords {
    /// Unquoted words outside any parentheses, uppercased, in order.
    depth0: Vec<String>,
    /// A call to one of [`EXTERNAL_TABLE_FUNCTIONS`] at any depth.
    external_table_function: bool,
}

fn top_level_words(sql: &str) -> Result<StatementWords, ()> {
    use crate::postgres::sql_lex::{lex_sql_spanned, SqlToken};
    let tokens = lex_sql_spanned(sql)?;
    let mut depth = 0usize;
    let mut depth0 = Vec::new();
    let mut external_table_function = false;
    for (index, spanned) in tokens.iter().enumerate() {
        match &spanned.token {
            SqlToken::Symbol('(') => depth += 1,
            SqlToken::Symbol(')') => depth = depth.saturating_sub(1),
            SqlToken::Identifier(identifier) if !identifier.quoted => {
                let calls = matches!(
                    tokens.get(index + 1).map(|next| &next.token),
                    Some(SqlToken::Symbol('('))
                );
                if calls
                    && EXTERNAL_TABLE_FUNCTIONS
                        .contains(&identifier.value.to_ascii_lowercase().as_str())
                {
                    external_table_function = true;
                }
                if depth == 0 {
                    depth0.push(identifier.value.to_ascii_uppercase());
                }
            }
            _ => {}
        }
    }
    Ok(StatementWords {
        depth0,
        external_table_function,
    })
}

/// True when any top-level `ALTER` action deletes, rewrites or moves data.
/// Words inside parentheses (subqueries, expressions) are not actions.
fn alter_is_destructive(depth0: &[String]) -> bool {
    depth0.iter().skip(1).any(|word| {
        matches!(
            word.as_str(),
            "DELETE"
                | "UPDATE"
                | "DROP"
                | "CLEAR"
                | "DETACH"
                | "REPLACE"
                | "MOVE"
                | "UNFREEZE"
                | "TTL"
        )
    })
}

impl ClickHouseSession {
    pub fn connection_id(&self) -> &str {
        &self.0.id
    }

    pub fn read_only(&self) -> bool {
        self.0.policy.read_only
    }

    /// True when both handles are clones of one opened session.
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::SeqCst)
    }

    fn admit(&self) -> Result<&StoredConnection, ClickHouseError> {
        let backend_closing = self
            .0
            .backend
            .upgrade()
            .is_none_or(|inner| inner.closing.load(Ordering::SeqCst));
        if self.is_closed() || backend_closing {
            return Err(ClickHouseError::refused(
                "Session is closed; reconnect to continue",
            ));
        }
        Ok(&self.0.connection)
    }

    /// Every permitted database and its objects; see [`ClickHouseCatalog`].
    pub async fn catalog(&self) -> Result<ClickHouseCatalog, ClickHouseError> {
        crate::clickhouse::catalog::fetch(self.admit()?).await
    }

    /// Runs exactly one statement under the connection's policy. Reads stop
    /// at [`QUERY_MAX_ROWS`] / [`QUERY_MAX_BYTES`]; `query_id` lets
    /// [`Self::cancel`] stop it on the server.
    pub async fn query(
        &self,
        sql: &str,
        confirmed: bool,
        query_id: &str,
    ) -> Result<ClickHouseQueryOutcome, ClickHouseError> {
        let connection = self.admit()?;
        let classes = classify(sql);
        match classes.len() {
            0 => return Err(ClickHouseError::refused("Nothing to run")),
            1 => {}
            _ => {
                return Err(ClickHouseError::refused(
                    "ClickHouse runs one statement per request; run statements one at a time",
                ))
            }
        }
        let intent = WriteIntent::Statement { classes };
        let authorization = match assert_permitted(&self.0.policy, &intent, confirmed) {
            Ok(authorization) => authorization,
            Err(SafetyRefusal::Blocked { reason, .. }) => {
                return Err(ClickHouseError::refused(reason))
            }
            Err(SafetyRefusal::NeedsConfirmation { statements }) => {
                return Ok(ClickHouseQueryOutcome::NeedsConfirmation(statements))
            }
        };
        let limits = Limits {
            max_rows: QUERY_MAX_ROWS,
            max_bytes: QUERY_MAX_BYTES,
            timeout: QUERY_TIMEOUT,
        };
        let running = Running::new(&self.0, query_id);
        let result = bounded::run(connection, sql, Some(query_id), limits).await;
        running.finish(&result);
        let rows = result?;
        if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
            gate::record_override(&self.0.pool, &self.0.id, AUDIT_COMMAND, &intent).await;
        }
        Ok(ClickHouseQueryOutcome::Rows(rows))
    }

    /// Best-effort server-side stop for a running [`Self::query`],
    /// [`Self::browse`] or [`Self::structure`]. The caller also drops its
    /// request; this never retries. An id that already settled, or was
    /// already killed, sends nothing.
    pub async fn cancel(&self, query_id: &str) {
        if self.0.take_running(query_id) {
            bounded::kill(&self.0.connection, query_id).await;
        }
    }

    /// One page of a table or view: `limit` rows after `offset`. Pages are
    /// ordered by the chosen column (if any), then by the table's sorting
    /// key, so offsets do not repeat or skip rows between pages. A table
    /// with no sorting key and no chosen column has no stable order; its
    /// page is marked [`ClickHouseRows::approximate_order`]. An empty
    /// `database` names a server-config dictionary, read through
    /// `dictionary()`. Reads only.
    pub async fn browse(
        &self,
        database: &str,
        table: &str,
        order: Option<(&str, bool)>,
        offset: u64,
        limit: usize,
        query_id: &str,
    ) -> Result<ClickHouseRows, ClickHouseError> {
        let connection = self.admit()?;
        let deadline = tokio::time::Instant::now() + BROWSE_TIMEOUT;
        let running = Running::new(&self.0, query_id);
        let result = async {
            // Server-config dictionaries are not in system.tables.
            let sorting_key = if database.is_empty() {
                String::new()
            } else {
                self.sorting_key(connection, database, table, query_id, deadline)
                    .await?
            };
            let sql = browse_sql(database, table, order, &sorting_key, offset, limit);
            let limits = Limits {
                max_rows: limit.clamp(1, BROWSE_PAGE_ROWS),
                max_bytes: BROWSE_MAX_BYTES,
                timeout: remaining(deadline)?,
            };
            let mut rows = bounded::run(connection, &sql, Some(query_id), limits).await?;
            rows.approximate_order = order.is_none() && sorting_key.is_empty();
            Ok::<_, ClickHouseError>(rows)
        }
        .await;
        running.finish(&result);
        result
    }

    /// The table's sorting key expression (empty when it has none), cached
    /// for the session's lifetime.
    async fn sorting_key(
        &self,
        connection: &StoredConnection,
        database: &str,
        table: &str,
        query_id: &str,
        deadline: tokio::time::Instant,
    ) -> Result<String, ClickHouseError> {
        let key = (database.to_owned(), table.to_owned());
        let cached = self
            .0
            .sorting_keys
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned());
        if let Some(cached) = cached {
            return Ok(cached);
        }
        let sql = format!(
            "SELECT sorting_key FROM system.tables WHERE database = {} AND name = {}",
            literal(database),
            literal(table)
        );
        let limits = Limits {
            max_rows: 1,
            max_bytes: 1024 * 1024,
            timeout: remaining(deadline)?,
        };
        let rows = bounded::run(connection, &sql, Some(query_id), limits).await?;
        let sorting_key = rows
            .rows
            .first()
            .and_then(|row| row.first().cloned().flatten())
            .map(|key| key.trim().to_owned())
            .unwrap_or_default();
        if let Ok(mut cache) = self.0.sorting_keys.lock() {
            if cache.len() >= SORTING_KEY_CACHE {
                cache.clear();
            }
            cache.insert(key, sorting_key.clone());
        }
        Ok(sorting_key)
    }

    /// Columns, sorting key, skip indexes, engine and stored DDL. Every read
    /// is bounded and all of them share one [`STRUCTURE_TIMEOUT`] deadline.
    pub async fn structure(
        &self,
        database: &str,
        table: &str,
        query_id: &str,
    ) -> Result<ClickHouseStructure, ClickHouseError> {
        let connection = self.admit()?;
        let deadline = tokio::time::Instant::now() + STRUCTURE_TIMEOUT;
        let running = Running::new(&self.0, query_id);
        let result = async {
            if database.is_empty() {
                return config_dictionary_structure(connection, table, query_id, deadline).await;
            }
            let meta = bounded::run(
                connection,
                &format!(
                    "SELECT engine, create_table_query, toString(total_rows), \
                     toString(total_bytes) FROM system.tables WHERE database = {} AND name = {}",
                    literal(database),
                    literal(table)
                ),
                Some(query_id),
                Limits {
                    max_rows: 1,
                    max_bytes: 4 * 1024 * 1024,
                    timeout: remaining(deadline)?,
                },
            )
            .await?;
            let Some(row) = meta.rows.into_iter().next() else {
                return Err(ClickHouseError::new(
                    ClickHouseErrorKind::Server,
                    format!("{database}.{table} no longer exists"),
                ));
            };
            let cell = |index: usize| row.get(index).cloned().flatten();
            let structure = crate::clickhouse::fetch_table_structure_bounded(
                connection,
                database,
                table,
                Some(query_id),
                deadline,
            )
            .await?;
            Ok::<_, ClickHouseError>(ClickHouseStructure {
                engine: cell(0).unwrap_or_default(),
                ddl: cell(1).unwrap_or_default(),
                total_rows: cell(2).and_then(|value| value.parse().ok()),
                total_bytes: cell(3).and_then(|value| value.parse().ok()),
                ..structure_of(structure)
            })
        }
        .await;
        running.finish(&result);
        result
    }

    /// Marks the session closed, sends a best-effort `KILL QUERY` for every
    /// request still in flight while the route is up, then releases the
    /// route. Idempotent.
    pub async fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        let running = self
            .0
            .running
            .lock()
            .map(|mut running| running.drain().collect::<Vec<_>>())
            .unwrap_or_default();
        let connection = &self.0.connection;
        futures_util::future::join_all(
            running
                .iter()
                .map(|query_id| bounded::kill(connection, query_id)),
        )
        .await;
        let route = self.0.route.lock().ok().and_then(|mut route| route.take());
        if let Some(route) = route {
            let _ = tokio::task::spawn_blocking(move || drop(route)).await;
        }
    }
}

/// The browse statement. A chosen column sorts first; the sorting key (the
/// table's own expression list from `system.tables`) breaks its ties and
/// orders unsorted pages.
fn browse_sql(
    database: &str,
    table: &str,
    order: Option<(&str, bool)>,
    sorting_key: &str,
    offset: u64,
    limit: usize,
) -> String {
    let limit = limit.clamp(1, BROWSE_PAGE_ROWS);
    let mut terms = Vec::new();
    if let Some((column, descending)) = order {
        terms.push(format!(
            "{}{}",
            quote_identifier(column),
            if descending { " DESC" } else { "" }
        ));
    }
    if !sorting_key.is_empty() {
        terms.push(sorting_key.to_owned());
    }
    let order = if terms.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", terms.join(", "))
    };
    format!(
        "SELECT * FROM {}{order} LIMIT {limit} OFFSET {offset}",
        object_source(database, table)
    )
}

/// Columns of a dictionary defined in server configuration. It has no
/// database, `system.tables` row or stored DDL, so its columns come from
/// `DESCRIBE TABLE dictionary('name')`.
async fn config_dictionary_structure(
    connection: &StoredConnection,
    name: &str,
    query_id: &str,
    deadline: tokio::time::Instant,
) -> Result<ClickHouseStructure, ClickHouseError> {
    let described = bounded::run(
        connection,
        &format!("DESCRIBE TABLE {}", object_source("", name)),
        Some(query_id),
        Limits {
            max_rows: 10_000,
            max_bytes: 4 * 1024 * 1024,
            timeout: remaining(deadline)?,
        },
    )
    .await?;
    Ok(config_dictionary_columns(described))
}

/// `DESCRIBE` rows (`name`, `type`, `default_type`, `default_expression`, …)
/// as a dictionary structure.
fn config_dictionary_columns(described: ClickHouseRows) -> ClickHouseStructure {
    let index = |wanted: &str| {
        described
            .columns
            .iter()
            .position(|column| column.name == wanted)
    };
    let (name, type_name) = (index("name"), index("type"));
    let (default_type, default_expression) = (index("default_type"), index("default_expression"));
    let cell = |row: &Vec<Option<String>>, index: Option<usize>| {
        index
            .and_then(|index| row.get(index).cloned().flatten())
            .unwrap_or_default()
    };
    ClickHouseStructure {
        engine: "Dictionary (server configuration)".into(),
        columns: described
            .rows
            .iter()
            .map(|row| {
                let (kind, expression) = (cell(row, default_type), cell(row, default_expression));
                ClickHouseStructureColumn {
                    name: cell(row, name),
                    type_name: cell(row, type_name),
                    default: (!expression.is_empty()).then(|| {
                        if kind.is_empty() {
                            expression.clone()
                        } else {
                            format!("{kind} {expression}")
                        }
                    }),
                    in_sorting_key: false,
                }
            })
            .collect(),
        ..Default::default()
    }
}

/// The `FROM` source for an object. A dictionary defined in server
/// configuration has no database (`database` is empty) and is read through
/// the `dictionary()` table function.
fn object_source(database: &str, table: &str) -> String {
    if database.is_empty() {
        format!("dictionary({})", literal(table))
    } else {
        format!("{}.{}", quote_identifier(database), quote_identifier(table))
    }
}

#[cfg(test)]
#[path = "clickhouse_tests.rs"]
mod tests;
