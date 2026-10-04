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
pub(crate) fn classify(sql: &str) -> Vec<StatementClass> {
    let Ok(statements) = describe_script(sql) else {
        return vec![StatementClass::Unknown];
    };
    statements
        .into_iter()
        .map(|statement| match statement.head.as_deref() {
            Some("DESCRIBE" | "DESC" | "EXISTS" | "EXPLAIN" | "SHOW") => StatementClass::Read,
            Some("OPTIMIZE" | "SYSTEM" | "RENAME" | "EXCHANGE" | "ATTACH" | "KILL" | "UNDROP") => {
                StatementClass::Ddl { destructive: false }
            }
            Some("DETACH") => StatementClass::Ddl { destructive: true },
            Some("USE") => StatementClass::Session,
            _ => statement.class,
        })
        .collect()
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
        let rows = bounded::run(connection, sql, Some(query_id), limits).await?;
        if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
            gate::record_override(&self.0.pool, &self.0.id, AUDIT_COMMAND, &intent).await;
        }
        Ok(ClickHouseQueryOutcome::Rows(rows))
    }

    /// Best-effort server-side stop for a running [`Self::query`]. The caller
    /// also drops its request; this never retries.
    pub async fn cancel(&self, query_id: &str) {
        if let Ok(connection) = self.admit() {
            bounded::kill(connection, query_id).await;
        }
    }

    /// One page of a table or view: `limit` rows after `offset`, optionally
    /// ordered by one column. Reads only.
    pub async fn browse(
        &self,
        database: &str,
        table: &str,
        order: Option<(&str, bool)>,
        offset: u64,
        limit: usize,
    ) -> Result<ClickHouseRows, ClickHouseError> {
        let connection = self.admit()?;
        let limit = limit.clamp(1, BROWSE_PAGE_ROWS);
        let order = order
            .map(|(column, descending)| {
                format!(
                    " ORDER BY {}{}",
                    quote_identifier(column),
                    if descending { " DESC" } else { "" }
                )
            })
            .unwrap_or_default();
        let sql = format!(
            "SELECT * FROM {}.{}{order} LIMIT {limit} OFFSET {offset}",
            quote_identifier(database),
            quote_identifier(table),
        );
        let limits = Limits {
            max_rows: limit,
            max_bytes: BROWSE_MAX_BYTES,
            timeout: BROWSE_TIMEOUT,
        };
        bounded::run(connection, &sql, None, limits).await
    }

    /// Columns, sorting key, skip indexes, engine and stored DDL.
    pub async fn structure(
        &self,
        database: &str,
        table: &str,
    ) -> Result<ClickHouseStructure, ClickHouseError> {
        let connection = self.admit()?;
        let meta = bounded::run(
            connection,
            &format!(
                "SELECT engine, create_table_query, toString(total_rows), toString(total_bytes) \
                 FROM system.tables WHERE database = {} AND name = {}",
                literal(database),
                literal(table)
            ),
            None,
            Limits {
                max_rows: 1,
                max_bytes: 4 * 1024 * 1024,
                timeout: STRUCTURE_TIMEOUT,
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
        let structure = tokio::time::timeout(
            STRUCTURE_TIMEOUT,
            crate::clickhouse::fetch_table_structure(connection, database, table),
        )
        .await
        .map_err(|_| {
            ClickHouseError::new(ClickHouseErrorKind::Timeout, "Structure read timed out")
        })?
        .map_err(|message| ClickHouseError::new(ClickHouseErrorKind::Server, message))?;
        Ok(ClickHouseStructure {
            engine: cell(0).unwrap_or_default(),
            ddl: cell(1).unwrap_or_default(),
            total_rows: cell(2).and_then(|value| value.parse().ok()),
            total_bytes: cell(3).and_then(|value| value.parse().ok()),
            ..structure_of(structure)
        })
    }

    /// Marks the session closed and releases its route. Idempotent.
    pub async fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
        let route = self.0.route.lock().ok().and_then(|mut route| route.take());
        if let Some(route) = route {
            let _ = tokio::task::spawn_blocking(move || drop(route)).await;
        }
    }
}

#[cfg(test)]
#[path = "clickhouse_tests.rs"]
mod tests;
