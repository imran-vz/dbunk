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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub engine: DatabaseEngine,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    /// ISO-8601 timestamp of the most recent successful query/connect.
    /// Optional so records created before activity tracking still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// ClickHouse-only: when true the URL builder uses the `https://`
    /// scheme and defaults the port to 8443 instead of 8123. Other
    /// engines ignore this field.
    #[serde(default)]
    pub use_https: bool,
    /// ClickHouse-only: optional URL path prefix for deployments that
    /// proxy CH behind a path (e.g. `/clickhouse`). Empty = root.
    #[serde(default)]
    pub url_path: String,
    /// Redis-only: which numbered DB (0–15 on standalone). Defaults to
    /// 0; ignored by relational engines.
    #[serde(default)]
    pub db_number: u8,
    /// Redis-only: connect over TLS (`rediss://`). Ignored by
    /// relational engines (which have their own SSL story via sqlx).
    #[serde(default)]
    pub use_tls: bool,
    /// Redis-only: verify the TLS certificate. Only meaningful when
    /// `use_tls` is true. Default true; users can disable for self-
    /// signed dev servers.
    #[serde(default = "default_true")]
    pub verify_tls_cert: bool,
}

fn default_true() -> bool {
    true
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
pub(crate) struct LoadDatabaseOverviewStatsPayload {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaExplorer {
    pub name: String,
    pub tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<String>,
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
