//! Pure data shapes used across the Tauri command surface.
//!
//! Everything in this module is a serde DTO — payloads the frontend sends,
//! result types it receives, and the persisted shapes for connections,
//! query history, and saved queries. There is no behaviour here; the
//! domain glossary in `CONTEXT.md` is the load-bearing reading.
//!
//! All items are `pub(crate)` so command dispatchers, the persistence layer
//! (`storage.rs`), the credential layer (`keychain.rs`), and the engine
//! implementations (`postgres.rs`) can use them via `crate::Foo` after the
//! re-export in `lib.rs`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Engine + persisted entities
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub(crate) enum DatabaseEngine {
    #[serde(rename = "PostgreSQL")]
    PostgreSQL,
    #[serde(rename = "MySQL")]
    MySQL,
    #[serde(rename = "ClickHouse")]
    ClickHouse,
    #[serde(rename = "SQLite")]
    SQLite,
    #[serde(rename = "Redis")]
    Redis,
}

/// Top-level engine class — `Relational` engines share schemas/tables/
/// rows/SQL; `KeyValue` engines share a keyspace of typed keys. The
/// class is derived from `DatabaseEngine::storage_class()`; never
/// stored. See ADR-0008.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StorageClass {
    Relational,
    KeyValue,
}

impl DatabaseEngine {
    /// Class lookup — the single source of truth for "is this engine a
    /// relational or keyvalue thing?" Exhaustive over variants so a new
    /// engine forces the question at compile time.
    pub fn storage_class(&self) -> StorageClass {
        match self {
            Self::PostgreSQL | Self::MySQL | Self::SQLite | Self::ClickHouse => {
                StorageClass::Relational
            }
            Self::Redis => StorageClass::KeyValue,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CredentialStorageMode {
    Keychain,
    EncryptedSqlite,
    PlainSqlite,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettingsSnapshot {
    pub onboarding_completed: bool,
    pub credential_storage_mode: Option<CredentialStorageMode>,
    pub credential_state: CredentialState,
    pub config_dir: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CredentialState {
    NeedsOnboarding,
    NeedsUnlock,
    Ready,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigureCredentialStoragePayload {
    pub mode: CredentialStorageMode,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnlockCredentialsPayload {
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangeCredentialStoragePayload {
    pub mode: CredentialStorageMode,
    pub password: Option<String>,
    pub confirm: bool,
}

/// `StoredConnection` is a per-engine tagged union — the backend's
/// counterpart to the frontend's `Connection` discriminated union
/// (see ADR-0010). Each variant carries exactly the fields its engine
/// uses: `ssl` on PG/MySQL, `useHttps`/`urlPath` on ClickHouse,
/// `dbNumber`/`useTls`/`verifyTlsCert` on Redis, nothing extra on
/// SQLite.
///
/// Serialized internally-tagged on `engine`, so the wire JSON is flat:
/// `{ "engine": "PostgreSQL", "id": "...", "host": "...", "ssl": true, ... }`.
/// This matches the previous flat-struct shape byte-for-byte except
/// that engine-irrelevant fields are now absent from the wire when
/// they don't apply (the frontend's optional `useHttps?` / `dbNumber?`
/// types tolerate the absence).
///
/// The enum mirrors the **Credential Backend** pattern: closed
/// variant set, exhaustive dispatch, per-variant logic concentrated.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "engine")]
pub(crate) enum StoredConnection {
    PostgreSQL(PgStoredConnection),
    MySQL(MySqlStoredConnection),
    SQLite(SqliteStoredConnection),
    ClickHouse(ClickHouseStoredConnection),
    Redis(RedisStoredConnection),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgStoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// TLS for the wire-protocol upgrade. Distinct concept from
    /// ClickHouse's `useHttps` (TLS for HTTP transport) and Redis's
    /// `useTls` (`rediss://` scheme) — see CONTEXT.md `Connection`.
    #[serde(default = "default_true")]
    pub ssl: bool,
    /// Optional driver/session knobs applied after every connect.
    /// See ADR-0013. Missing or empty fields fall back to PG defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_options: Option<PgDriverOptions>,
}

/// Per-connection driver/session knobs persisted on the Postgres
/// connection record. Reads as a single JSON blob in SQLite so adding
/// a new knob is a struct field, not a schema migration. See
/// ADR-0013.
///
/// Every field is optional — `None` means "use the server default".
/// Empty `default_search_path` is treated the same as `None`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDriverOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_in_transaction_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_search_path: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_role: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MySqlStoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    #[serde(default = "default_true")]
    pub ssl: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SqliteStoredConnection {
    pub id: String,
    pub name: String,
    /// File path to the SQLite database (or `:memory:`).
    pub database: String,
    /// Sentinel fields preserved for wire-compatibility with the
    /// frontend's flat `Connection` shape (host/port/user/password
    /// are `required` on the TS side). Always empty for SQLite.
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClickHouseStoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// When true the URL builder uses `https://` and defaults the
    /// port to 8443 instead of 8123.
    #[serde(default)]
    pub use_https: bool,
    /// Optional URL path prefix for deployments that proxy CH behind
    /// a path (e.g. `/clickhouse`). Empty = root.
    #[serde(default)]
    pub url_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisStoredConnection {
    pub id: String,
    pub name: String,
    /// Frontend sends this as `""` for Redis; Redis uses `db_number`
    /// for keyspace selection. Kept on the wire for shape stability.
    #[serde(default)]
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Which numbered DB (0–15 on standalone).
    #[serde(default)]
    pub db_number: u8,
    /// Connect over TLS (`rediss://`).
    #[serde(default)]
    pub use_tls: bool,
    /// Verify the TLS certificate. Only meaningful when `use_tls` is
    /// true. Default true; users disable for self-signed dev servers.
    #[serde(default = "default_true")]
    pub verify_tls_cert: bool,
}

fn default_true() -> bool {
    true
}

impl StoredConnection {
    pub fn engine(&self) -> DatabaseEngine {
        match self {
            Self::PostgreSQL(_) => DatabaseEngine::PostgreSQL,
            Self::MySQL(_) => DatabaseEngine::MySQL,
            Self::SQLite(_) => DatabaseEngine::SQLite,
            Self::ClickHouse(_) => DatabaseEngine::ClickHouse,
            Self::Redis(_) => DatabaseEngine::Redis,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.id,
            Self::MySQL(c) => &c.id,
            Self::SQLite(c) => &c.id,
            Self::ClickHouse(c) => &c.id,
            Self::Redis(c) => &c.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.name,
            Self::MySQL(c) => &c.name,
            Self::SQLite(c) => &c.name,
            Self::ClickHouse(c) => &c.name,
            Self::Redis(c) => &c.name,
        }
    }

    pub fn database(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.database,
            Self::MySQL(c) => &c.database,
            Self::SQLite(c) => &c.database,
            Self::ClickHouse(c) => &c.database,
            Self::Redis(c) => &c.database,
        }
    }

    pub fn host(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.host,
            Self::MySQL(c) => &c.host,
            Self::SQLite(c) => &c.host,
            Self::ClickHouse(c) => &c.host,
            Self::Redis(c) => &c.host,
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            Self::PostgreSQL(c) => c.port,
            Self::MySQL(c) => c.port,
            Self::SQLite(c) => c.port,
            Self::ClickHouse(c) => c.port,
            Self::Redis(c) => c.port,
        }
    }

    pub fn user(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.user,
            Self::MySQL(c) => &c.user,
            Self::SQLite(c) => &c.user,
            Self::ClickHouse(c) => &c.user,
            Self::Redis(c) => &c.user,
        }
    }

    pub fn password(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.password,
            Self::MySQL(c) => &c.password,
            Self::SQLite(c) => &c.password,
            Self::ClickHouse(c) => &c.password,
            Self::Redis(c) => &c.password,
        }
    }

    pub fn role(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.role,
            Self::MySQL(c) => &c.role,
            Self::SQLite(c) => &c.role,
            Self::ClickHouse(c) => &c.role,
            Self::Redis(c) => &c.role,
        }
    }

    pub fn last_activity_at(&self) -> Option<&str> {
        match self {
            Self::PostgreSQL(c) => c.last_activity_at.as_deref(),
            Self::MySQL(c) => c.last_activity_at.as_deref(),
            Self::SQLite(c) => c.last_activity_at.as_deref(),
            Self::ClickHouse(c) => c.last_activity_at.as_deref(),
            Self::Redis(c) => c.last_activity_at.as_deref(),
        }
    }

    /// Replace the password on whichever variant is active. Used by
    /// `credentials::hydrate` to inject the resolved password before
    /// passing the connection to the engine dispatchers.
    pub fn set_password(&mut self, password: String) {
        match self {
            Self::PostgreSQL(c) => c.password = password,
            Self::MySQL(c) => c.password = password,
            Self::SQLite(c) => c.password = password,
            Self::ClickHouse(c) => c.password = password,
            Self::Redis(c) => c.password = password,
        }
    }
}

/// Connect-time pipeline result for Redis. Surfaced in the connection-
/// test "modules-detected" banner. Every field is `Option<>` because
/// managed Redis (Upstash hobby tier, ElastiCache locked-down ACLs)
/// often restricts `INFO` sections or `MODULE LIST` — degrading
/// per-field is better than failing the whole connect-test.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisCapabilities {
    /// `redis_version` from `INFO server` — e.g. `"7.2.4"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// `role` from `INFO replication` — `"master"` or `"replica"`.
    /// Drives auto-read-only (ADR-0009).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// `connected_slaves` from `INFO replication`. Drives the soft
    /// "this may be a production master" notice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_slaves: Option<u32>,
    /// `MODULE LIST` results. `None` when the command was rejected;
    /// `Some(vec![])` when it succeeded with zero modules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<RedisModuleInfo>>,
    /// `DBSIZE` for the active DB. `None` if rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_size: Option<u64>,
    /// `maxmemory-policy` from `CONFIG GET`. Drives whether
    /// `OBJECT FREQ` is meaningful on keys (LFU policies only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maxmemory_policy: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisModuleInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryHistoryEntry {
    pub id: String,
    pub sql: String,
    pub connection_id: String,
    pub connection_name: String,
    pub database: String,
    pub engine: DatabaseEngine,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub runtime_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u64>,
    pub started_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedQuery {
    pub id: String,
    pub name: String,
    pub body: String,
    /// `None` = saved query is not pinned to a specific connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub is_favorite: bool,
    /// Reserved for future cloud-sync. Local-only writes leave this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Generic command results
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub runtime_ms: u64,
    pub row_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectResult {
    pub latency_ms: u64,
    /// Populated when the connection is to a Redis server — the
    /// post-test-connection banner reads from here. `None` for
    /// relational engines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redis_capabilities: Option<RedisCapabilities>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub(crate) enum HealthCheckResult {
    #[serde(rename = "healthy")]
    Healthy { latency_ms: u64 },
    #[serde(rename = "error")]
    Error { error: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableDataResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub page: u32,
    pub page_size: u32,
    pub total_rows: Option<u64>,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteDdlResult {
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitCellEditsResult {
    pub rows_affected: u64,
    pub runtime_ms: u64,
    /// "committed" — the change is applied (PostgreSQL).
    /// "queued"    — the change is accepted but applied asynchronously
    ///   (ClickHouse `ALTER … UPDATE`). The frontend polls
    ///   `poll_mutation_status` until all `mutation_ids` report
    ///   `is_done = true`.
    pub state: String,
    /// Database the mutations apply to. Needed alongside `mutation_ids`
    /// because CH's `system.mutations` is keyed by `(database, table,
    /// mutation_id)`. Empty for synchronous engines.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub database: String,
    /// Table the mutations apply to. Empty for synchronous engines.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub table: String,
    /// One ID per `ALTER TABLE … UPDATE` we issued. Empty for
    /// synchronous engines (PostgreSQL).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mutation_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsertRowResult {
    pub runtime_ms: u64,
    pub rows_affected: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRowsResult {
    pub runtime_ms: u64,
    pub rows_affected: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportDdlResult {
    pub sql: String,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDumpResult {
    pub data_base64: String,
    pub extension: String,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgRestoreResult {
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyTableResult {
    pub runtime_ms: u64,
    pub rows_copied: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgSessionInfo {
    pub pid: i32,
    pub user: String,
    pub database: Option<String>,
    pub application_name: String,
    pub client_addr: Option<String>,
    pub state: Option<String>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub query_age_seconds: Option<i64>,
    pub transaction_age_seconds: Option<i64>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgLockInfo {
    pub pid: i32,
    pub lock_type: String,
    pub relation: Option<String>,
    pub mode: String,
    pub granted: bool,
    pub blocked_by: Vec<i32>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgPendingTransactionInfo {
    pub pid: i32,
    pub user: String,
    pub state: Option<String>,
    pub transaction_age_seconds: Option<i64>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgAdminStats {
    pub database_size_bytes: i64,
    pub cache_hit_ratio: Option<f64>,
    pub active_sessions: i64,
    pub idle_in_transaction: i64,
    pub blocked_locks: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgAdminSnapshot {
    pub sessions: Vec<PgSessionInfo>,
    pub locks: Vec<PgLockInfo>,
    pub pending_transactions: Vec<PgPendingTransactionInfo>,
    pub stats: PgAdminStats,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgBackendActionResult {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteRowsResult {
    pub runtime_ms: u64,
    pub rows_affected: u64,
    /// "committed" or "queued" — same semantics as `CommitCellEditsResult.state`.
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub database: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub table: String,
    /// One per `ALTER TABLE … DELETE` (CH). Empty for PG.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mutation_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationStatus {
    pub mutation_id: String,
    pub is_done: bool,
    /// Populated when CH reports `latest_fail_reason` on
    /// `system.mutations`.
    pub latest_fail_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PollMutationStatusPayload {
    pub connection_id: String,
    pub database: String,
    pub table: String,
    pub mutation_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Command payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestConnectionPayload {
    pub connection: StoredConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunQueryPayload {
    pub connection_id: String,
    pub query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadTableDataPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadTableStructurePayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadSchemaRelationshipsPayload {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaMapScopePayload {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveSchemaMapPositionPayload {
    pub connection_id: String,
    pub schema: String,
    pub table_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaMapPrefsPatch {
    #[serde(default)]
    pub routing: Option<String>,
    #[serde(default)]
    pub attr_mode: Option<String>,
    #[serde(default)]
    pub show_types: Option<bool>,
    #[serde(default)]
    pub show_nulls: Option<bool>,
    #[serde(default)]
    pub show_comments: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveSchemaMapPrefsPayload {
    pub connection_id: String,
    pub schema: String,
    pub patch: SchemaMapPrefsPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadDatabaseOverviewStatsPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadRelationStatsPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadServerDetailsPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteDdlPayload {
    pub connection_id: String,
    pub sql: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellEditKeyValue {
    pub column: String,
    // None == NULL when binding the parameter. Serde turns missing or null
    // JSON values into None, anything else becomes the string verbatim. We
    // intentionally keep types as strings here: the UI only ever has the
    // string form in hand and Postgres is happy to accept text params for
    // most types via `&str` binding (it coerces server-side).
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellEdit {
    // Echoed back through to the caller for UI mapping; ignored by SQL.
    #[serde(default)]
    #[allow(dead_code)]
    pub row_index: u32,
    pub identity: Vec<CellEditKeyValue>,
    pub set: Vec<CellEditKeyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitCellEditsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub edits: Vec<CellEdit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsertRowPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    // column -> Option<String> (None = NULL). Columns omitted from this list
    // get the database default — that lets users insert rows that rely on
    // SERIAL/identity columns or DEFAULT NOW() etc.
    pub values: Vec<CellEditKeyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRowsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub use_copy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportDdlPayload {
    pub connection_id: String,
    pub scope: String,
    pub schema: Option<String>,
    pub table: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDumpPayload {
    pub connection_id: String,
    pub scope: String,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub format: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgRestorePayload {
    pub connection_id: String,
    pub data_base64: String,
    pub format: String,
    #[serde(default)]
    pub clean: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyTablePayload {
    pub source_connection_id: String,
    pub source_schema: String,
    pub source_table: String,
    pub destination_connection_id: String,
    pub destination_schema: String,
    pub destination_table: String,
    pub page_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefreshMaterializedViewPayload {
    pub connection_id: String,
    pub schema: String,
    pub view: String,
    #[serde(default)]
    pub concurrently: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgBackendActionPayload {
    pub connection_id: String,
    pub pid: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgMaintenancePayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub action: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteRowsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    // Each Vec<CellEditKeyValue> is the identity for one row. We delete each
    // identified row inside a single transaction; if any identified row
    // misses (rows_affected == 0) we ROLLBACK the whole batch.
    pub rows: Vec<Vec<CellEditKeyValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteSavedQueryPayload {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Schema introspection — returned by load_table_structure
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub ordinal_position: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
    pub method: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConstraintInfo {
    pub name: String,
    pub kind: String,
    pub definition: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StructureCapabilities {
    pub columns: bool,
    pub primary_key: bool,
    pub foreign_keys: bool,
    pub indexes: bool,
    pub constraints: bool,
    /// Per-table mutation capabilities. The frontend reads these to
    /// decide whether to show edit/insert/delete UI rather than gating
    /// on engine name — this matters for ClickHouse where MergeTree
    /// tables accept mutations and Distributed/View tables don't.
    pub can_insert_rows: bool,
    pub can_update_rows: bool,
    pub can_delete_rows: bool,
    pub can_alter_schema: bool,
    /// "exact" — identity columns guarantee at most one matching row
    ///   (PostgreSQL with a real PK).
    /// "best-effort" — identity may match multiple rows (ClickHouse,
    ///   where sorting key is not a uniqueness constraint).
    pub uniqueness_guarantee: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableStructure {
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Option<Vec<String>>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub indexes: Vec<IndexInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub capabilities: StructureCapabilities,
    /// Engine-specific extension fields. Currently populated only for
    /// ClickHouse — the storage engine name (`MergeTree`,
    /// `ReplicatedMergeTree`, `Distributed`, `View`, …) and the
    /// MergeTree partition / sampling expressions. Left `None` for
    /// engines that don't have these concepts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_by: Option<String>,
}

// ---------------------------------------------------------------------------
// Schema relationships — returned by load_schema_relationships
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaTableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub ordinal_position: i32,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaTableNode {
    pub schema: String,
    pub name: String,
    pub column_count: u32,
    pub columns: Vec<SchemaTableColumn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaForeignKey {
    pub constraint_name: String,
    pub from_schema: String,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_schema: String,
    pub to_table: String,
    pub to_columns: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaRelationships {
    pub tables: Vec<SchemaTableNode>,
    pub foreign_keys: Vec<SchemaForeignKey>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PositionRow {
    pub table_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaMapPrefs {
    pub routing: String,
    pub attr_mode: String,
    pub show_types: bool,
    pub show_nulls: bool,
    pub show_comments: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaExplorer {
    pub name: String,
    pub tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materialized_views: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign_tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aggregate_functions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_triggers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tablespaces: Vec<String>,
}

// ---------------------------------------------------------------------------
// Database overview stats
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatabaseOverviewStats {
    pub database_size_bytes: i64,
    pub table_size_bytes: i64,
    pub index_size_bytes: i64,
    pub table_count: i64,
    pub schema_count: i64,
    pub row_count_estimate: i64,
    pub index_count: i64,
    pub connection_count: i64,
}

// ---------------------------------------------------------------------------
// Per-relation stats (Tables + Schemas sub-tabs)
// ---------------------------------------------------------------------------

/// One row per user-visible relation (table, view, materialised view)
/// in the connection's catalogue, used to populate the Tables sub-tab
/// and to derive the Schemas sub-tab's per-schema aggregates in the
/// frontend. Today only Postgres is implemented; other relational
/// engines return an empty list — the Schemas sub-tab is gated to PG
/// in the UI and the Tables sub-tab degrades to schema/name/kind
/// columns on non-PG.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelationInfo {
    pub schema: String,
    pub name: String,
    /// One of "table", "view", "materialized view". Matches the
    /// kind values rendered by the Tables sub-tab badge.
    pub kind: String,
    /// `pg_class.reltuples` cast to bigint — a planner estimate, not
    /// an exact COUNT. Zero when the estimate is missing or the row
    /// is a view (where it has no useful meaning).
    pub row_count_estimate: i64,
    /// `pg_total_relation_size` in bytes — table plus its TOAST and
    /// every index. Zero for views.
    pub total_size_bytes: i64,
}

// ---------------------------------------------------------------------------
// Server details (Details sub-tab)
// ---------------------------------------------------------------------------

/// One row from `pg_settings` — a single GUC parameter. The category
/// + short_desc + source fields drive the Details sub-tab's grouping,
///   tooltip, and "modified from default" highlight.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgSetting {
    pub name: String,
    pub setting: String,
    pub unit: Option<String>,
    pub category: String,
    pub short_desc: Option<String>,
    /// `default`, `configuration file`, `command line`, `session`,
    /// `client`, `database`, `user`, `override`, etc. The frontend
    /// highlights any non-"default" value to surface operator
    /// overrides.
    pub source: String,
    pub boot_val: Option<String>,
    pub reset_val: Option<String>,
}

/// One row per installed Postgres extension, surfaced read-only on
/// the Details sub-tab. Install/drop UI is explicitly Phase 6, not
/// Phase 1.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgExtension {
    pub name: String,
    pub version: String,
    pub schema: String,
    pub description: Option<String>,
}

/// Aggregate snapshot returned by `load_server_details` — the Details
/// sub-tab's data source. Postgres-only; non-PG connections never
/// invoke the command (the UI renders a Postgres-only panel).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerDetails {
    pub server_version: String,
    pub encoding: String,
    pub locale: String,
    pub timezone: String,
    pub settings: Vec<PgSetting>,
    pub extensions: Vec<PgExtension>,
}
