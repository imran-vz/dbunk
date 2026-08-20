//! SQLite persistence for dbunk's local app state.
//!
//! ADR-0007 makes `~/.config/dbunk/dbunk.sqlite` the primary local store.
//! This module owns path resolution, pool setup, lightweight embedded
//! migrations, and per-entity read/write helpers.

use std::{fmt, fs, path::PathBuf, str::FromStr};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::result_mutation::protocol::VirtualKey;
use crate::table_browse::protocol::TableGridPrefs;
use crate::{
    ClickHouseStoredConnection, CredentialStorageMode, DatabaseEngine, MySqlStoredConnection,
    PgStoredConnection, PositionRow, QueryHistoryEntry, RedisCliHistoryEntry,
    RedisStoredConnection, SavedQuery, SavedRedisCommand, SchemaMapPrefs, SchemaMapPrefsPatch,
    SqliteStoredConnection, SshTunnelConfig, StoredConnection,
};

pub(crate) mod bastions;
pub(crate) mod managed;

const DB_FILE: &str = "dbunk.sqlite";
pub(crate) const VIRTUAL_KEY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VirtualKeyValidationError {
    UnsupportedVersion(u32),
    EmptyIdentity,
    DuplicateColumn,
}

impl fmt::Display for VirtualKeyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "Unsupported virtual key version {version}")
            }
            Self::EmptyIdentity => {
                formatter.write_str("A virtual key must contain non-empty column names")
            }
            Self::DuplicateColumn => {
                formatter.write_str("A virtual key cannot contain duplicate columns")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VirtualKeyStorageError {
    ConnectionNotFound,
    UnsupportedEngine,
    InvalidInput(VirtualKeyValidationError),
    CorruptDocument(String),
    Database(String),
}

impl fmt::Display for VirtualKeyStorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionNotFound => formatter.write_str("Connection not found"),
            Self::UnsupportedEngine => formatter.write_str("Unsupported database engine"),
            Self::InvalidInput(error) => error.fmt(formatter),
            Self::CorruptDocument(error) => {
                write!(formatter, "Invalid stored virtual key: {error}")
            }
            Self::Database(error) => error.fmt(formatter),
        }
    }
}

const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
CREATE TABLE app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE connections (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  database_name TEXT NOT NULL,
  engine TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  user_name TEXT NOT NULL,
  role TEXT NOT NULL,
  last_activity_at TEXT,
  use_https INTEGER NOT NULL DEFAULT 0,
  url_path TEXT NOT NULL DEFAULT ''
);

CREATE TABLE credentials (
  connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
  storage_mode TEXT NOT NULL,
  nonce TEXT,
  password_value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE credential_verifier (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  kdf TEXT NOT NULL,
  salt TEXT NOT NULL,
  nonce TEXT NOT NULL,
  ciphertext TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE query_history (
  id TEXT PRIMARY KEY,
  sql TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  connection_name TEXT NOT NULL,
  database_name TEXT NOT NULL,
  engine TEXT NOT NULL,
  status TEXT NOT NULL,
  error_message TEXT,
  runtime_ms INTEGER NOT NULL,
  row_count INTEGER,
  started_at TEXT NOT NULL
);

CREATE INDEX idx_query_history_started_at ON query_history(started_at DESC);

CREATE TABLE saved_queries (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  connection_id TEXT,
  is_favorite INTEGER NOT NULL DEFAULT 0,
  owner_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_saved_queries_updated_at ON saved_queries(updated_at DESC);
"#,
    ),
    (
        2,
        // Redis-specific columns on the shared `connections` table. Mirror the
        // pattern ClickHouse already uses for `use_https`/`url_path`: nullable
        // / default-valued, ignored by engines that don't need them.
        r#"
ALTER TABLE connections ADD COLUMN db_number INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN use_tls INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN verify_tls_cert INTEGER NOT NULL DEFAULT 1;
"#,
    ),
    (
        3,
        // PG/MySQL TLS-on-the-wire toggle. Default 1 matches the
        // previously-hidden form default; existing connections continue
        // to negotiate TLS the way they did before. The column lives on
        // the shared `connections` table because the SQLite schema stays
        // flat (per ADR-0010); engines other than PG/MySQL simply ignore
        // the column when their variant is constructed.
        r#"
ALTER TABLE connections ADD COLUMN ssl INTEGER NOT NULL DEFAULT 1;
"#,
    ),
    (
        4,
        r#"
CREATE TABLE schema_map_positions (
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema        TEXT NOT NULL,
  table_id      TEXT NOT NULL,
  x             REAL NOT NULL,
  y             REAL NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_id)
);

CREATE TABLE schema_map_prefs (
  connection_id  TEXT    NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema         TEXT    NOT NULL,
  routing        TEXT    NOT NULL DEFAULT 'bezier',
  attr_mode      TEXT    NOT NULL DEFAULT 'all',
  show_types     INTEGER NOT NULL DEFAULT 1,
  show_nulls     INTEGER NOT NULL DEFAULT 0,
  show_comments  INTEGER NOT NULL DEFAULT 0,
  updated_at     TEXT    NOT NULL,
  PRIMARY KEY (connection_id, schema)
);
"#,
    ),
    (
        5,
        // ADR-0013: optional Postgres driver/session knobs serialised
        // as one JSON blob. Adding a new knob is a struct field, not
        // a schema migration. Engines other than PG ignore the
        // column (it stays NULL on their rows).
        r#"
ALTER TABLE connections ADD COLUMN driver_options TEXT;
"#,
    ),
    (
        6,
        // Redis CLI command history. One row per submitted command,
        // scoped to the connection. Results are NOT persisted — only
        // the command text + when it was submitted, mirroring the
        // shell-history idea. Capped globally at 1000 rows via
        // `REDIS_CLI_HISTORY_CAP` (trimmed on every insert).
        r#"
CREATE TABLE redis_cli_history (
  id            TEXT NOT NULL PRIMARY KEY,
  connection_id TEXT NOT NULL,
  command       TEXT NOT NULL,
  submitted_at  TEXT NOT NULL
);

CREATE INDEX idx_redis_cli_history_connection_submitted_at
  ON redis_cli_history(connection_id, submitted_at DESC);
"#,
    ),
    (
        7,
        // Belt-and-braces Redis read-only toggle (ADR-0009). Default
        // 0 (not read-only). When 1, `assert_writable` rejects writes
        // without even consulting the replica-role cache.
        r#"
ALTER TABLE connections ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        8,
        // Saved Redis CLI commands — analogous to `saved_queries` for
        // SQL, but the connection ref is optional so users can save
        // engine-portable commands. Parameter substitution is
        // deferred — `body` is treated as-is by the CLI at load time.
        r#"
CREATE TABLE saved_redis_commands (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  body          TEXT NOT NULL,
  connection_id TEXT,
  is_favorite   INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE INDEX idx_saved_redis_commands_updated_at
  ON saved_redis_commands(updated_at DESC);
"#,
    ),
    (
        9,
        // ADR-0018: the credentials backend now stores database
        // passwords and bastion secrets. Existing rows are database
        // password credentials keyed by connection id; moving to a
        // generic credential_id preserves them while removing the
        // connection FK that would block bastion-secret keys.
        r#"
CREATE TABLE credentials_new (
  credential_id TEXT PRIMARY KEY,
  storage_mode TEXT NOT NULL,
  nonce TEXT,
  password_value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

INSERT INTO credentials_new (credential_id, storage_mode, nonce, password_value, updated_at)
SELECT connection_id, storage_mode, nonce, password_value, updated_at FROM credentials;

DROP TABLE credentials;

ALTER TABLE credentials_new RENAME TO credentials;
"#,
    ),
    (
        10,
        // ADR-0018 first slice: first-class bastion servers plus
        // per-connection SSH tunnel config on network-backed engines.
        // SQLite ignores these shared-table columns because it has no
        // network transport and no SshTunnelConfig field.
        r#"
CREATE TABLE bastion_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  user_name TEXT NOT NULL,
  auth_method TEXT NOT NULL,
  private_key_path TEXT,
  host_key_fingerprint TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_bastion_servers_name ON bastion_servers(name COLLATE NOCASE);

ALTER TABLE connections ADD COLUMN ssh_tunnel_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN ssh_tunnel_bastion_server_id TEXT;
ALTER TABLE connections ADD COLUMN ssh_tunnel_local_bind_host TEXT;
ALTER TABLE connections ADD COLUMN ssh_tunnel_local_port INTEGER;
"#,
    ),
    (
        11,
        // ADR-0018 deferred polish: advanced per-Connection SSH
        // Tunnel options. Jump chains store Bastion Server IDs as JSON
        // so the first-class Bastion records and their separate secret
        // namespace remain the source of truth for every hop.
        r#"
ALTER TABLE connections ADD COLUMN ssh_tunnel_compression INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN ssh_tunnel_keepalive_interval_seconds INTEGER;
ALTER TABLE connections ADD COLUMN ssh_tunnel_keepalive_want_reply INTEGER NOT NULL DEFAULT 1;
ALTER TABLE connections ADD COLUMN ssh_tunnel_jump_chain TEXT;
ALTER TABLE connections ADD COLUMN ssh_tunnel_proxy_command TEXT;
"#,
    ),
    (
        12,
        // ADR-0019: Managed Servers — Docker-provisioned local
        // databases. The Connection link is one-way (managed server →
        // connection_id); status is never stored, it is derived live
        // from Docker.
        r#"
CREATE TABLE managed_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  engine TEXT NOT NULL,
  version TEXT NOT NULL,
  port INTEGER NOT NULL,
  container_name TEXT NOT NULL,
  volume_name TEXT NOT NULL,
  database_name TEXT NOT NULL,
  user_name TEXT NOT NULL,
  connection_id TEXT,
  created_at TEXT NOT NULL
);

CREATE INDEX idx_managed_servers_name ON managed_servers(name COLLATE NOCASE);
"#,
    ),
    (
        13,
        // ADR-0022: opaque per-table Grid Preferences JSON for Table Browse.
        r#"
CREATE TABLE table_grid_prefs (
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema        TEXT NOT NULL,
  table_name    TEXT NOT NULL,
  prefs         TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_name)
);
"#,
    ),
    (
        14,
        // ADR-0023: user-selected ordered column sets for relations whose
        // catalog identity cannot safely identify the projected result.
        r#"
CREATE TABLE virtual_keys (
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema        TEXT NOT NULL,
  table_name    TEXT NOT NULL,
  virtual_key   TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_name)
);
"#,
    ),
];

pub struct Paths {
    config_dir: PathBuf,
}

impl Paths {
    pub fn from_app(app: &AppHandle) -> Result<Self, String> {
        let config_dir = resolve_config_dir(app)?;
        Ok(Self { config_dir })
    }

    #[allow(dead_code)]
    pub fn from_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    pub fn db_file(&self) -> PathBuf {
        self.config_dir.join(DB_FILE)
    }

    pub fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }

    fn ensure_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.config_dir).map_err(|error| error.to_string())
    }
}

fn resolve_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        app.path()
            .resolve("dbunk", BaseDirectory::AppData)
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        app.path()
            .resolve(".config/dbunk", BaseDirectory::Home)
            .map_err(|error| error.to_string())
    }
}

pub async fn open_pool(paths: &Paths) -> Result<SqlitePool, String> {
    paths.ensure_dir()?;
    let options = SqliteConnectOptions::new()
        .filename(paths.db_file())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .map_err(|error| error.to_string())?;
    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;

    for (version, sql) in MIGRATIONS {
        let applied: Option<(i64,)> =
            sqlx::query_as("SELECT version FROM schema_migrations WHERE version = ?")
                .bind(version)
                .fetch_optional(pool)
                .await
                .map_err(|error| error.to_string())?;
        if applied.is_some() {
            continue;
        }
        let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
        for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
        }
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)")
            .bind(version)
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?;
        tx.commit().await.map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn i64_to_bool(value: i64) -> bool {
    value != 0
}

fn i64_to_u16(value: i64) -> u16 {
    u16::try_from(value).unwrap_or(0)
}

impl FromStr for DatabaseEngine {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "PostgreSQL" => Ok(Self::PostgreSQL),
            "MySQL" => Ok(Self::MySQL),
            "ClickHouse" => Ok(Self::ClickHouse),
            "SQLite" => Ok(Self::SQLite),
            "Redis" => Ok(Self::Redis),
            _ => Err(format!("unknown database engine '{value}'")),
        }
    }
}

impl DatabaseEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PostgreSQL => "PostgreSQL",
            Self::MySQL => "MySQL",
            Self::ClickHouse => "ClickHouse",
            Self::SQLite => "SQLite",
            Self::Redis => "Redis",
        }
    }
}

impl CredentialStorageMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keychain => "keychain",
            Self::EncryptedSqlite => "encrypted-sqlite",
            Self::PlainSqlite => "plain-sqlite",
        }
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, String> {
    let row = sqlx::query("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(row.map(|row| row.get::<String, _>("value")))
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(value)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// Shared column list used by both `read_connections` and
/// `read_connection_by_id`.
const CONNECTION_COLUMNS: &str = "id, name, database_name, engine, host, port, user_name, role,
     last_activity_at, use_https, url_path,
     db_number, use_tls, verify_tls_cert, ssl, driver_options,
     read_only,
     ssh_tunnel_enabled, ssh_tunnel_bastion_server_id,
     ssh_tunnel_local_bind_host, ssh_tunnel_local_port,
     ssh_tunnel_compression, ssh_tunnel_keepalive_interval_seconds,
     ssh_tunnel_keepalive_want_reply, ssh_tunnel_jump_chain,
     ssh_tunnel_proxy_command";

pub(super) fn parse_ssh_tunnel_jump_chain(
    raw: Option<&str>,
    connection_id: &str,
) -> Result<Vec<String>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<String>>(raw).map_err(|error| {
        format!("Malformed SSH jump chain for Connection '{connection_id}': {error}")
    })
}

fn parse_ssh_keepalive_interval(
    raw: Option<i64>,
    connection_id: &str,
) -> Result<Option<u32>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let interval = u32::try_from(raw).map_err(|_| {
        format!("Malformed SSH keepalive interval for Connection '{connection_id}': {raw}")
    })?;
    if !(2..=3600).contains(&interval) {
        return Err(format!(
            "Malformed SSH keepalive interval for Connection '{connection_id}': {raw}"
        ));
    }
    Ok(Some(interval))
}

fn row_to_ssh_tunnel(
    row: &sqlx::sqlite::SqliteRow,
    connection_id: &str,
) -> Result<SshTunnelConfig, String> {
    let jump_chain_json: Option<String> = row.get("ssh_tunnel_jump_chain");
    Ok(SshTunnelConfig {
        enabled: row.get::<i64, _>("ssh_tunnel_enabled") != 0,
        bastion_server_id: row.get("ssh_tunnel_bastion_server_id"),
        local_bind_host: row.get("ssh_tunnel_local_bind_host"),
        local_port: row
            .get::<Option<i64>, _>("ssh_tunnel_local_port")
            .map(i64_to_u16),
        compression: row.get::<i64, _>("ssh_tunnel_compression") != 0,
        keepalive_interval_seconds: parse_ssh_keepalive_interval(
            row.get("ssh_tunnel_keepalive_interval_seconds"),
            connection_id,
        )?,
        keepalive_want_reply: row.get::<i64, _>("ssh_tunnel_keepalive_want_reply") != 0,
        jump_chain: parse_ssh_tunnel_jump_chain(jump_chain_json.as_deref(), connection_id)?,
        proxy_command: row.get("ssh_tunnel_proxy_command"),
    }
    .normalized())
}

/// Map a single SQLite row into a `StoredConnection` variant.
/// Row → variant construction. The SQLite schema stays flat
/// (one row per connection, engine-irrelevant columns simply
/// unread per variant); we match on the `engine` column to
/// pick which variant to build. ADR-0010 covers the design.
fn row_to_connection(row: sqlx::sqlite::SqliteRow) -> Result<StoredConnection, String> {
    let engine = DatabaseEngine::from_str(row.get::<String, _>("engine").as_str())?;
    let id: String = row.get("id");
    let name: String = row.get("name");
    let database: String = row.get("database_name");
    let host: String = row.get("host");
    let port = i64_to_u16(row.get("port"));
    let user: String = row.get("user_name");
    let role: String = row.get("role");
    let last_activity_at: Option<String> = row.get("last_activity_at");
    let ssh_tunnel = row_to_ssh_tunnel(&row, &id)?;

    let driver_options_json: Option<String> = row.get("driver_options");
    let driver_options = driver_options_json.and_then(|raw| {
        if raw.is_empty() {
            return None;
        }
        match serde_json::from_str::<crate::types::PgDriverOptions>(&raw) {
            Ok(value) => Some(value),
            Err(error) => {
                log::warn!("ignoring malformed driver_options for connection {id}: {error}");
                None
            }
        }
    });

    Ok(match engine {
        DatabaseEngine::PostgreSQL => StoredConnection::PostgreSQL(PgStoredConnection {
            id,
            name,
            database,
            host,
            port,
            user,
            password: String::new(),
            role,
            last_activity_at,
            ssl: row.get::<i64, _>("ssl") != 0,
            driver_options,
            ssh_tunnel,
        }),
        DatabaseEngine::MySQL => StoredConnection::MySQL(MySqlStoredConnection {
            id,
            name,
            database,
            host,
            port,
            user,
            password: String::new(),
            role,
            last_activity_at,
            ssl: row.get::<i64, _>("ssl") != 0,
            ssh_tunnel,
        }),
        DatabaseEngine::SQLite => StoredConnection::SQLite(SqliteStoredConnection {
            id,
            name,
            database,
            host,
            port,
            user,
            password: String::new(),
            role,
            last_activity_at,
        }),
        DatabaseEngine::ClickHouse => StoredConnection::ClickHouse(ClickHouseStoredConnection {
            id,
            name,
            database,
            host,
            port,
            user,
            password: String::new(),
            role,
            last_activity_at,
            use_https: row.get::<i64, _>("use_https") != 0,
            url_path: row.get("url_path"),
            ssh_tunnel,
        }),
        DatabaseEngine::Redis => StoredConnection::Redis(RedisStoredConnection {
            id,
            name,
            database,
            host,
            port,
            user,
            password: String::new(),
            role,
            last_activity_at,
            db_number: u8::try_from(row.get::<i64, _>("db_number")).unwrap_or(0),
            use_tls: row.get::<i64, _>("use_tls") != 0,
            verify_tls_cert: row.get::<i64, _>("verify_tls_cert") != 0,
            read_only: row.get::<i64, _>("read_only") != 0,
            ssh_tunnel,
        }),
    })
}

pub async fn read_connections(pool: &SqlitePool) -> Result<Vec<StoredConnection>, String> {
    let query =
        format!("SELECT {CONNECTION_COLUMNS} FROM connections ORDER BY name COLLATE NOCASE ASC");
    let rows = sqlx::query(&query)
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;

    rows.into_iter().map(row_to_connection).collect()
}

/// Fetch a single connection by ID. Returns `None` when the ID doesn't
/// exist — callers map that to a user-facing "Connection not found" error.
/// This avoids reading and deserialising every stored connection when
/// only one is needed (the hot path for every Tauri command).
pub async fn read_connection_by_id(
    pool: &SqlitePool,
    connection_id: &str,
) -> Result<Option<StoredConnection>, String> {
    let query = format!("SELECT {CONNECTION_COLUMNS} FROM connections WHERE id = ?");
    let row = sqlx::query(&query)
        .bind(connection_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?;

    row.map(row_to_connection).transpose()
}

pub async fn upsert_connection(
    pool: &SqlitePool,
    connection: &StoredConnection,
) -> Result<(), String> {
    // Per-variant binding: each variant supplies values for the columns
    // it owns; engine-irrelevant columns get neutral defaults so the
    // flat row shape remains consistent. The SQLite schema is unchanged
    // by the enum refactor — only the variant -> column mapping is.
    let engine = connection.engine();
    let id = connection.id().to_string();
    let name = connection.name().to_string();
    let database = connection.database().to_string();
    let host = connection.host().to_string();
    let port = i64::from(connection.port());
    let user = connection.user().to_string();
    let role = connection.role().to_string();
    let last_activity_at = connection.last_activity_at().map(str::to_string);

    let (use_https, url_path) = match connection {
        StoredConnection::ClickHouse(c) => (bool_to_i64(c.use_https), c.url_path.clone()),
        _ => (0, String::new()),
    };
    let (db_number, use_tls, verify_tls_cert, read_only) = match connection {
        StoredConnection::Redis(c) => (
            i64::from(c.db_number),
            bool_to_i64(c.use_tls),
            bool_to_i64(c.verify_tls_cert),
            bool_to_i64(c.read_only),
        ),
        _ => (0, 0, 1, 0),
    };
    let ssl = match connection {
        StoredConnection::PostgreSQL(c) => bool_to_i64(c.ssl),
        StoredConnection::MySQL(c) => bool_to_i64(c.ssl),
        _ => 1,
    };
    let tunnel = connection
        .ssh_tunnel()
        .map(SshTunnelConfig::normalized)
        .unwrap_or_default();
    let ssh_tunnel_enabled = bool_to_i64(tunnel.enabled);
    let ssh_tunnel_bastion_server_id = tunnel.bastion_server_id;
    let ssh_tunnel_local_bind_host = tunnel.local_bind_host;
    let ssh_tunnel_local_port = tunnel.local_port.map(i64::from);
    let ssh_tunnel_compression = bool_to_i64(tunnel.compression);
    let ssh_tunnel_keepalive_interval_seconds = tunnel.keepalive_interval_seconds.map(i64::from);
    let ssh_tunnel_keepalive_want_reply = bool_to_i64(tunnel.keepalive_want_reply);
    let ssh_tunnel_jump_chain = if tunnel.jump_chain.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&tunnel.jump_chain).map_err(|error| error.to_string())?)
    };
    let ssh_tunnel_proxy_command = tunnel.proxy_command;
    // ADR-0013: PG driver_options serialise to one JSON blob; other
    // engines persist NULL so the column stays variant-aware without
    // adding per-engine columns.
    let driver_options: Option<String> = match connection {
        StoredConnection::PostgreSQL(c) => match &c.driver_options {
            Some(options) => {
                Some(serde_json::to_string(options).map_err(|error| error.to_string())?)
            }
            None => None,
        },
        _ => None,
    };

    sqlx::query(
        "INSERT INTO connections (
            id, name, database_name, engine, host, port, user_name, role,
            last_activity_at, use_https, url_path,
            db_number, use_tls, verify_tls_cert, ssl, driver_options,
            read_only,
            ssh_tunnel_enabled, ssh_tunnel_bastion_server_id,
            ssh_tunnel_local_bind_host, ssh_tunnel_local_port,
            ssh_tunnel_compression, ssh_tunnel_keepalive_interval_seconds,
            ssh_tunnel_keepalive_want_reply, ssh_tunnel_jump_chain,
            ssh_tunnel_proxy_command
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            database_name = excluded.database_name,
            engine = excluded.engine,
            host = excluded.host,
            port = excluded.port,
            user_name = excluded.user_name,
            role = excluded.role,
            last_activity_at = excluded.last_activity_at,
            use_https = excluded.use_https,
            url_path = excluded.url_path,
            db_number = excluded.db_number,
            use_tls = excluded.use_tls,
            verify_tls_cert = excluded.verify_tls_cert,
            ssl = excluded.ssl,
            driver_options = excluded.driver_options,
            read_only = excluded.read_only,
            ssh_tunnel_enabled = excluded.ssh_tunnel_enabled,
            ssh_tunnel_bastion_server_id = excluded.ssh_tunnel_bastion_server_id,
            ssh_tunnel_local_bind_host = excluded.ssh_tunnel_local_bind_host,
            ssh_tunnel_local_port = excluded.ssh_tunnel_local_port,
            ssh_tunnel_compression = excluded.ssh_tunnel_compression,
            ssh_tunnel_keepalive_interval_seconds = excluded.ssh_tunnel_keepalive_interval_seconds,
            ssh_tunnel_keepalive_want_reply = excluded.ssh_tunnel_keepalive_want_reply,
            ssh_tunnel_jump_chain = excluded.ssh_tunnel_jump_chain,
            ssh_tunnel_proxy_command = excluded.ssh_tunnel_proxy_command",
    )
    .bind(id)
    .bind(name)
    .bind(database)
    .bind(engine.as_str())
    .bind(host)
    .bind(port)
    .bind(user)
    .bind(role)
    .bind(last_activity_at)
    .bind(use_https)
    .bind(url_path)
    .bind(db_number)
    .bind(use_tls)
    .bind(verify_tls_cert)
    .bind(ssl)
    .bind(driver_options)
    .bind(read_only)
    .bind(ssh_tunnel_enabled)
    .bind(ssh_tunnel_bastion_server_id)
    .bind(ssh_tunnel_local_bind_host)
    .bind(ssh_tunnel_local_port)
    .bind(ssh_tunnel_compression)
    .bind(ssh_tunnel_keepalive_interval_seconds)
    .bind(ssh_tunnel_keepalive_want_reply)
    .bind(ssh_tunnel_jump_chain)
    .bind(ssh_tunnel_proxy_command)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn delete_connection(pool: &SqlitePool, connection_id: &str) -> Result<bool, String> {
    let result = sqlx::query("DELETE FROM connections WHERE id = ?")
        .bind(connection_id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(result.rows_affected() > 0)
}

pub async fn touch_connection_activity(
    pool: &SqlitePool,
    connection_id: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE connections SET last_activity_at = ? WHERE id = ?")
        .bind(now())
        .bind(connection_id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema map positions + prefs
// ---------------------------------------------------------------------------

pub fn default_schema_map_prefs() -> SchemaMapPrefs {
    SchemaMapPrefs {
        routing: "bezier".to_string(),
        attr_mode: "all".to_string(),
        show_types: true,
        show_nulls: false,
        show_comments: false,
    }
}

fn validate_schema_map_prefs(prefs: &SchemaMapPrefs) -> Result<(), String> {
    if !matches!(prefs.routing.as_str(), "bezier" | "step") {
        return Err(format!("invalid schema map routing '{}'", prefs.routing));
    }
    if !matches!(prefs.attr_mode.as_str(), "all" | "keys-only" | "none") {
        return Err(format!(
            "invalid schema map attribute mode '{}'",
            prefs.attr_mode
        ));
    }
    Ok(())
}

pub async fn read_schema_map_positions(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
) -> Result<Vec<PositionRow>, String> {
    let rows = sqlx::query(
        "SELECT table_id, x, y
         FROM schema_map_positions
         WHERE connection_id = ? AND schema = ?
         ORDER BY table_id COLLATE NOCASE ASC",
    )
    .bind(connection_id)
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| PositionRow {
            table_id: row.get("table_id"),
            x: row.get("x"),
            y: row.get("y"),
        })
        .collect())
}

pub async fn upsert_schema_map_position(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table_id: &str,
    x: f64,
    y: f64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO schema_map_positions (connection_id, schema, table_id, x, y, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(connection_id, schema, table_id) DO UPDATE SET
            x = excluded.x,
            y = excluded.y,
            updated_at = excluded.updated_at",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(table_id)
    .bind(x)
    .bind(y)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn clear_schema_map_positions(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
) -> Result<(), String> {
    sqlx::query("DELETE FROM schema_map_positions WHERE connection_id = ? AND schema = ?")
        .bind(connection_id)
        .bind(schema)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn read_schema_map_prefs(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
) -> Result<SchemaMapPrefs, String> {
    let row = sqlx::query(
        "SELECT routing, attr_mode, show_types, show_nulls, show_comments
         FROM schema_map_prefs
         WHERE connection_id = ? AND schema = ?",
    )
    .bind(connection_id)
    .bind(schema)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;

    let Some(row) = row else {
        return Ok(default_schema_map_prefs());
    };

    let prefs = SchemaMapPrefs {
        routing: row.get("routing"),
        attr_mode: row.get("attr_mode"),
        show_types: i64_to_bool(row.get("show_types")),
        show_nulls: i64_to_bool(row.get("show_nulls")),
        show_comments: i64_to_bool(row.get("show_comments")),
    };
    validate_schema_map_prefs(&prefs)?;
    Ok(prefs)
}

pub async fn upsert_schema_map_prefs(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    patch: SchemaMapPrefsPatch,
) -> Result<SchemaMapPrefs, String> {
    let mut prefs = read_schema_map_prefs(pool, connection_id, schema).await?;
    if let Some(routing) = patch.routing {
        prefs.routing = routing;
    }
    if let Some(attr_mode) = patch.attr_mode {
        prefs.attr_mode = attr_mode;
    }
    if let Some(show_types) = patch.show_types {
        prefs.show_types = show_types;
    }
    if let Some(show_nulls) = patch.show_nulls {
        prefs.show_nulls = show_nulls;
    }
    if let Some(show_comments) = patch.show_comments {
        prefs.show_comments = show_comments;
    }
    validate_schema_map_prefs(&prefs)?;

    sqlx::query(
        "INSERT INTO schema_map_prefs (
            connection_id, schema, routing, attr_mode,
            show_types, show_nulls, show_comments, updated_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(connection_id, schema) DO UPDATE SET
            routing = excluded.routing,
            attr_mode = excluded.attr_mode,
            show_types = excluded.show_types,
            show_nulls = excluded.show_nulls,
            show_comments = excluded.show_comments,
            updated_at = excluded.updated_at",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(&prefs.routing)
    .bind(&prefs.attr_mode)
    .bind(bool_to_i64(prefs.show_types))
    .bind(bool_to_i64(prefs.show_nulls))
    .bind(bool_to_i64(prefs.show_comments))
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(prefs)
}

pub async fn read_table_grid_prefs(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
) -> Result<Option<TableGridPrefs>, String> {
    let row = sqlx::query(
        "SELECT prefs
         FROM table_grid_prefs
         WHERE connection_id = ? AND schema = ? AND table_name = ?",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(table)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let json: String = row.get("prefs");
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|error| error.to_string())?;
    Ok(Some(
        crate::table_browse::protocol::validate_table_grid_prefs(TableGridPrefs(value))?,
    ))
}

pub async fn upsert_table_grid_prefs(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
    prefs: &TableGridPrefs,
) -> Result<(), String> {
    let prefs = crate::table_browse::protocol::validate_table_grid_prefs(prefs.clone())?;
    let json = serde_json::to_string(&prefs.0).map_err(|error| error.to_string())?;
    sqlx::query(
        "INSERT INTO table_grid_prefs (connection_id, schema, table_name, prefs, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(connection_id, schema, table_name) DO UPDATE SET
            prefs = excluded.prefs,
            updated_at = excluded.updated_at",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(table)
    .bind(json)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn read_virtual_key(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
) -> Result<Option<VirtualKey>, VirtualKeyStorageError> {
    let row = sqlx::query(
        "SELECT virtual_key
         FROM virtual_keys
         WHERE connection_id = ? AND schema = ? AND table_name = ?",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(table)
    .fetch_optional(pool)
    .await
    .map_err(|error| VirtualKeyStorageError::Database(error.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let json: String = row.get("virtual_key");
    let virtual_key = serde_json::from_str(&json)
        .map_err(|error| VirtualKeyStorageError::CorruptDocument(error.to_string()))?;
    validate_virtual_key(&virtual_key)
        .map_err(|error| VirtualKeyStorageError::CorruptDocument(error.to_string()))?;
    Ok(Some(virtual_key))
}

pub async fn read_postgres_virtual_key(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
) -> Result<Option<VirtualKey>, VirtualKeyStorageError> {
    ensure_postgres_virtual_key_connection(pool, connection_id).await?;
    read_virtual_key(pool, connection_id, schema, table).await
}

pub async fn upsert_virtual_key(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
    virtual_key: &VirtualKey,
) -> Result<(), VirtualKeyStorageError> {
    validate_virtual_key(virtual_key).map_err(VirtualKeyStorageError::InvalidInput)?;
    let json = serde_json::to_string(virtual_key)
        .map_err(|error| VirtualKeyStorageError::Database(error.to_string()))?;
    sqlx::query(
        "INSERT INTO virtual_keys (connection_id, schema, table_name, virtual_key, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(connection_id, schema, table_name) DO UPDATE SET
            virtual_key = excluded.virtual_key,
            updated_at = excluded.updated_at",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(table)
    .bind(json)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| VirtualKeyStorageError::Database(error.to_string()))?;
    Ok(())
}

pub async fn upsert_postgres_virtual_key(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
    virtual_key: &VirtualKey,
) -> Result<(), VirtualKeyStorageError> {
    ensure_postgres_virtual_key_connection(pool, connection_id).await?;
    upsert_virtual_key(pool, connection_id, schema, table, virtual_key).await
}

pub async fn clear_virtual_key(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
) -> Result<(), VirtualKeyStorageError> {
    sqlx::query(
        "DELETE FROM virtual_keys
         WHERE connection_id = ? AND schema = ? AND table_name = ?",
    )
    .bind(connection_id)
    .bind(schema)
    .bind(table)
    .execute(pool)
    .await
    .map_err(|error| VirtualKeyStorageError::Database(error.to_string()))?;
    Ok(())
}

pub async fn clear_postgres_virtual_key(
    pool: &SqlitePool,
    connection_id: &str,
    schema: &str,
    table: &str,
) -> Result<(), VirtualKeyStorageError> {
    ensure_postgres_virtual_key_connection(pool, connection_id).await?;
    clear_virtual_key(pool, connection_id, schema, table).await
}

async fn ensure_postgres_virtual_key_connection(
    pool: &SqlitePool,
    connection_id: &str,
) -> Result<(), VirtualKeyStorageError> {
    let engine: Option<(String,)> = sqlx::query_as("SELECT engine FROM connections WHERE id = ?")
        .bind(connection_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| VirtualKeyStorageError::Database(error.to_string()))?;
    let Some((engine,)) = engine else {
        return Err(VirtualKeyStorageError::ConnectionNotFound);
    };
    let engine = DatabaseEngine::from_str(&engine).map_err(VirtualKeyStorageError::Database)?;
    if engine != DatabaseEngine::PostgreSQL {
        return Err(VirtualKeyStorageError::UnsupportedEngine);
    }
    Ok(())
}

pub(crate) fn validate_virtual_key(
    virtual_key: &VirtualKey,
) -> Result<(), VirtualKeyValidationError> {
    if virtual_key.version != VIRTUAL_KEY_VERSION {
        return Err(VirtualKeyValidationError::UnsupportedVersion(
            virtual_key.version,
        ));
    }
    if virtual_key.columns.is_empty() || virtual_key.columns.iter().any(String::is_empty) {
        return Err(VirtualKeyValidationError::EmptyIdentity);
    }
    let unique_columns = virtual_key
        .columns
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if unique_columns.len() != virtual_key.columns.len() {
        return Err(VirtualKeyValidationError::DuplicateColumn);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SQLite credentials
// ---------------------------------------------------------------------------

pub async fn read_sqlite_credentials(
    pool: &SqlitePool,
) -> Result<Vec<(String, Option<String>, String)>, String> {
    let rows = sqlx::query("SELECT credential_id, nonce, password_value FROM credentials")
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get("credential_id"),
                row.get("nonce"),
                row.get("password_value"),
            )
        })
        .collect())
}

pub async fn upsert_sqlite_credential(
    pool: &SqlitePool,
    credential_id: &str,
    mode: CredentialStorageMode,
    nonce: Option<&str>,
    password_value: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO credentials (credential_id, storage_mode, nonce, password_value, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(credential_id) DO UPDATE SET
            storage_mode = excluded.storage_mode,
            nonce = excluded.nonce,
            password_value = excluded.password_value,
            updated_at = excluded.updated_at",
    )
    .bind(credential_id)
    .bind(mode.as_str())
    .bind(nonce)
    .bind(password_value)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn clear_sqlite_credentials(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("DELETE FROM credentials")
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn read_verifier(
    pool: &SqlitePool,
) -> Result<Option<(String, String, String, String)>, String> {
    let row =
        sqlx::query("SELECT kdf, salt, nonce, ciphertext FROM credential_verifier WHERE id = 1")
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?;
    Ok(row.map(|row| {
        (
            row.get("kdf"),
            row.get("salt"),
            row.get("nonce"),
            row.get("ciphertext"),
        )
    }))
}

pub async fn write_verifier(
    pool: &SqlitePool,
    kdf: &str,
    salt: &str,
    nonce: &str,
    ciphertext: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO credential_verifier (id, kdf, salt, nonce, ciphertext, updated_at)
         VALUES (1, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            kdf = excluded.kdf,
            salt = excluded.salt,
            nonce = excluded.nonce,
            ciphertext = excluded.ciphertext,
            updated_at = excluded.updated_at",
    )
    .bind(kdf)
    .bind(salt)
    .bind(nonce)
    .bind(ciphertext)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn clear_verifier(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("DELETE FROM credential_verifier")
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Query history
// ---------------------------------------------------------------------------

/// Maximum number of query-history rows kept in SQLite. Mirrors the
/// `QUERY_HISTORY_CAP` constant in `src/lib/store/relational-queries.ts`;
/// keep both in sync. Phase 1 raised this from 200 to 2000 so the
/// dedicated Query History sub-tab can surface more than a day or
/// two of activity on a busy connection.
pub const QUERY_HISTORY_CAP: u32 = 2000;

pub async fn read_query_history(
    pool: &SqlitePool,
    limit: Option<u32>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let limit = limit.unwrap_or(QUERY_HISTORY_CAP);
    let rows = sqlx::query(
        "SELECT id, sql, connection_id, connection_name, database_name, engine,
                status, error_message, runtime_ms, row_count, started_at
         FROM query_history
         ORDER BY started_at DESC
         LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    rows.into_iter()
        .map(|row| {
            let engine = DatabaseEngine::from_str(row.get::<String, _>("engine").as_str())?;
            Ok(QueryHistoryEntry {
                id: row.get("id"),
                sql: row.get("sql"),
                connection_id: row.get("connection_id"),
                connection_name: row.get("connection_name"),
                database: row.get("database_name"),
                engine,
                status: row.get("status"),
                error_message: row.get("error_message"),
                runtime_ms: row.get::<i64, _>("runtime_ms").max(0) as u64,
                row_count: row
                    .get::<Option<i64>, _>("row_count")
                    .map(|value| value.max(0) as u64),
                started_at: row.get("started_at"),
            })
        })
        .collect()
}

pub async fn insert_query_history(
    pool: &SqlitePool,
    entry: &QueryHistoryEntry,
) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO query_history (
            id, sql, connection_id, connection_name, database_name, engine,
            status, error_message, runtime_ms, row_count, started_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&entry.id)
    .bind(&entry.sql)
    .bind(&entry.connection_id)
    .bind(&entry.connection_name)
    .bind(&entry.database)
    .bind(entry.engine.as_str())
    .bind(&entry.status)
    .bind(&entry.error_message)
    .bind(i64::try_from(entry.runtime_ms).unwrap_or(i64::MAX))
    .bind(
        entry
            .row_count
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
    )
    .bind(&entry.started_at)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(&format!(
        "DELETE FROM query_history
         WHERE id NOT IN (
           SELECT id FROM query_history ORDER BY started_at DESC LIMIT {}
         )",
        QUERY_HISTORY_CAP
    ))
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Redis CLI history
// ---------------------------------------------------------------------------

/// Maximum number of Redis CLI history rows kept in SQLite. The cap
/// is global (not per-connection) — same shape as the query-history
/// cap — and trimmed on every insert.
pub const REDIS_CLI_HISTORY_CAP: u32 = 1000;

pub async fn read_redis_cli_history(
    pool: &SqlitePool,
    connection_id: &str,
    limit: Option<u32>,
) -> Result<Vec<RedisCliHistoryEntry>, String> {
    let limit = limit.unwrap_or(REDIS_CLI_HISTORY_CAP);
    let rows = sqlx::query(
        "SELECT id, connection_id, command, submitted_at
         FROM redis_cli_history
         WHERE connection_id = ?
         ORDER BY submitted_at DESC
         LIMIT ?",
    )
    .bind(connection_id)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| RedisCliHistoryEntry {
            id: row.get("id"),
            connection_id: row.get("connection_id"),
            command: row.get("command"),
            submitted_at: row.get("submitted_at"),
        })
        .collect())
}

pub async fn insert_redis_cli_history(
    pool: &SqlitePool,
    entry: &RedisCliHistoryEntry,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO redis_cli_history (id, connection_id, command, submitted_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&entry.id)
    .bind(&entry.connection_id)
    .bind(&entry.command)
    .bind(&entry.submitted_at)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    sqlx::query(&format!(
        "DELETE FROM redis_cli_history
         WHERE id NOT IN (
           SELECT id FROM redis_cli_history ORDER BY submitted_at DESC LIMIT {}
         )",
        REDIS_CLI_HISTORY_CAP
    ))
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn clear_query_history(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("DELETE FROM query_history")
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Saved queries
// ---------------------------------------------------------------------------

pub async fn read_saved_queries(pool: &SqlitePool) -> Result<Vec<SavedQuery>, String> {
    let rows = sqlx::query(
        "SELECT id, name, body, connection_id, is_favorite, owner_id, created_at, updated_at
         FROM saved_queries
         ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| SavedQuery {
            id: row.get("id"),
            name: row.get("name"),
            body: row.get("body"),
            connection_id: row.get("connection_id"),
            is_favorite: row.get::<i64, _>("is_favorite") != 0,
            owner_id: row.get("owner_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn upsert_saved_query(pool: &SqlitePool, query: &SavedQuery) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO saved_queries (
            id, name, body, connection_id, is_favorite, owner_id, created_at, updated_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            body = excluded.body,
            connection_id = excluded.connection_id,
            is_favorite = excluded.is_favorite,
            owner_id = excluded.owner_id,
            updated_at = excluded.updated_at",
    )
    .bind(&query.id)
    .bind(&query.name)
    .bind(&query.body)
    .bind(&query.connection_id)
    .bind(bool_to_i64(query.is_favorite))
    .bind(&query.owner_id)
    .bind(&query.created_at)
    .bind(&query.updated_at)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn delete_saved_query(pool: &SqlitePool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM saved_queries WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Saved Redis commands (mirror of saved_queries for the Redis CLI)
// ---------------------------------------------------------------------------

pub async fn read_saved_redis_commands(
    pool: &SqlitePool,
) -> Result<Vec<SavedRedisCommand>, String> {
    let rows = sqlx::query(
        "SELECT id, name, body, connection_id, is_favorite, created_at, updated_at
         FROM saved_redis_commands
         ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| SavedRedisCommand {
            id: row.get("id"),
            name: row.get("name"),
            body: row.get("body"),
            connection_id: row.get("connection_id"),
            is_favorite: row.get::<i64, _>("is_favorite") != 0,
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn upsert_saved_redis_command(
    pool: &SqlitePool,
    command: &SavedRedisCommand,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO saved_redis_commands (
            id, name, body, connection_id, is_favorite, created_at, updated_at
         )
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            body = excluded.body,
            connection_id = excluded.connection_id,
            is_favorite = excluded.is_favorite,
            updated_at = excluded.updated_at",
    )
    .bind(&command.id)
    .bind(&command.name)
    .bind(&command.body)
    .bind(&command.connection_id)
    .bind(bool_to_i64(command.is_favorite))
    .bind(&command.created_at)
    .bind(&command.updated_at)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub async fn delete_saved_redis_command(pool: &SqlitePool, id: &str) -> Result<(), String> {
    sqlx::query("DELETE FROM saved_redis_commands WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BastionAuthMethod, BastionServer};
    use tempfile::tempdir;

    async fn test_pool() -> SqlitePool {
        let dir = tempdir().expect("temp dir");
        let paths = Paths::from_dir(dir.path().to_path_buf());
        let pool = open_pool(&paths).await.expect("pool");
        std::mem::forget(dir);
        pool
    }

    fn connection(id: &str) -> StoredConnection {
        StoredConnection::PostgreSQL(PgStoredConnection {
            id: id.to_string(),
            name: "Primary".to_string(),
            database: "postgres".to_string(),
            host: "localhost".to_string(),
            port: 5432,
            user: "postgres".to_string(),
            password: String::new(),
            role: String::new(),
            last_activity_at: None,
            ssl: true,
            driver_options: None,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    async fn set_connection_engine(pool: &SqlitePool, connection_id: &str, engine: &str) {
        sqlx::query("UPDATE connections SET engine = ? WHERE id = ?")
            .bind(engine)
            .bind(connection_id)
            .execute(pool)
            .await
            .expect("set connection engine");
    }

    fn bastion(id: &str) -> BastionServer {
        let timestamp = now();
        BastionServer {
            id: id.to_string(),
            name: "Primary bastion".to_string(),
            host: "bastion.example.com".to_string(),
            port: 22,
            user: "ubuntu".to_string(),
            auth_method: BastionAuthMethod::PrivateKeyPath,
            private_key_path: Some("/Users/me/.ssh/id_ed25519".to_string()),
            host_key_fingerprint: Some("SHA256:abc".to_string()),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        }
    }

    #[tokio::test]
    async fn schema_map_positions_round_trip_and_reset() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");

        upsert_schema_map_position(&pool, "conn-1", "public", "public.users", 10.0, 20.0)
            .await
            .expect("position");

        assert_eq!(
            read_schema_map_positions(&pool, "conn-1", "public")
                .await
                .expect("positions"),
            vec![PositionRow {
                table_id: "public.users".to_string(),
                x: 10.0,
                y: 20.0,
            }]
        );

        clear_schema_map_positions(&pool, "conn-1", "public")
            .await
            .expect("reset");
        assert!(read_schema_map_positions(&pool, "conn-1", "public")
            .await
            .expect("positions")
            .is_empty());
    }

    #[tokio::test]
    async fn bastion_and_connection_tunnel_fields_round_trip() {
        let pool = test_pool().await;
        bastions::upsert_bastion_server(&pool, &bastion("bastion-1"))
            .await
            .expect("bastion");

        let mut connection = connection("conn-1");
        if let StoredConnection::PostgreSQL(pg) = &mut connection {
            pg.ssh_tunnel = SshTunnelConfig {
                enabled: true,
                bastion_server_id: Some(" bastion-1 ".to_string()),
                local_bind_host: Some(" 127.0.0.2 ".to_string()),
                local_port: Some(15432),
                compression: true,
                keepalive_interval_seconds: Some(30),
                keepalive_want_reply: false,
                jump_chain: vec![" jump-1 ".to_string(), String::new()],
                proxy_command: Some(" ssh -W %h:%p edge ".to_string()),
            };
        }
        upsert_connection(&pool, &connection)
            .await
            .expect("connection");

        let servers = bastions::read_bastion_servers(&pool)
            .await
            .expect("servers");
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0].private_key_path.as_deref(),
            Some("/Users/me/.ssh/id_ed25519")
        );
        assert_eq!(
            bastions::count_connections_referencing_bastion(&pool, "bastion-1")
                .await
                .expect("count"),
            1
        );
        assert_eq!(
            bastions::connection_ids_referencing_bastion(&pool, "bastion-1")
                .await
                .expect("ids"),
            vec!["conn-1".to_string()]
        );
        assert_eq!(
            bastions::connection_ids_referencing_bastion(&pool, "jump-1")
                .await
                .expect("jump ids"),
            vec!["conn-1".to_string()]
        );

        let stored = read_connections(&pool).await.expect("connections");
        let StoredConnection::PostgreSQL(pg) = &stored[0] else {
            panic!("expected postgres");
        };
        assert!(pg.ssh_tunnel.enabled);
        assert_eq!(
            pg.ssh_tunnel.bastion_server_id.as_deref(),
            Some("bastion-1")
        );
        assert_eq!(pg.ssh_tunnel.local_bind_host.as_deref(), Some("127.0.0.2"));
        assert_eq!(pg.ssh_tunnel.local_port, Some(15432));
        assert!(pg.ssh_tunnel.compression);
        assert_eq!(pg.ssh_tunnel.keepalive_interval_seconds, Some(30));
        assert!(!pg.ssh_tunnel.keepalive_want_reply);
        assert_eq!(pg.ssh_tunnel.jump_chain, vec!["jump-1".to_string()]);
        assert_eq!(
            pg.ssh_tunnel.proxy_command.as_deref(),
            Some("ssh -W %h:%p edge")
        );
    }

    #[tokio::test]
    async fn bastion_reference_lookup_normalizes_stored_tunnel_ids() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");
        sqlx::query(
            "UPDATE connections
             SET ssh_tunnel_enabled = 1,
                 ssh_tunnel_bastion_server_id = ?,
                 ssh_tunnel_jump_chain = ?
             WHERE id = ?",
        )
        .bind(" bastion-1 ")
        .bind(r#"[" jump-1 ",""]"#)
        .bind("conn-1")
        .execute(&pool)
        .await
        .expect("raw tunnel ids");

        assert_eq!(
            bastions::connection_ids_referencing_bastion(&pool, "bastion-1")
                .await
                .expect("final ids"),
            vec!["conn-1".to_string()]
        );
        assert_eq!(
            bastions::connection_ids_referencing_bastion(&pool, "jump-1")
                .await
                .expect("jump ids"),
            vec!["conn-1".to_string()]
        );
    }

    #[tokio::test]
    async fn malformed_ssh_jump_chain_is_not_silently_dropped() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");
        sqlx::query(
            "UPDATE connections
             SET ssh_tunnel_enabled = 1,
                 ssh_tunnel_bastion_server_id = ?,
                 ssh_tunnel_jump_chain = ?
             WHERE id = ?",
        )
        .bind("bastion-1")
        .bind("not-json")
        .bind("conn-1")
        .execute(&pool)
        .await
        .expect("corrupt jump chain");

        let read_error = read_connections(&pool).await.expect_err("read fails");
        assert!(read_error.contains("Malformed SSH jump chain for Connection 'conn-1'"));

        let refs_error = bastions::connection_ids_referencing_bastion(&pool, "jump-1")
            .await
            .expect_err("reference lookup fails");
        assert!(refs_error.contains("Malformed SSH jump chain for Connection 'conn-1'"));

        let final_ref_error = bastions::connection_ids_referencing_bastion(&pool, "bastion-1")
            .await
            .expect_err("final reference lookup fails");
        assert!(final_ref_error.contains("Malformed SSH jump chain for Connection 'conn-1'"));
    }

    #[tokio::test]
    async fn malformed_ssh_keepalive_interval_is_not_silently_dropped() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");
        sqlx::query(
            "UPDATE connections
             SET ssh_tunnel_enabled = 1,
                 ssh_tunnel_keepalive_interval_seconds = ?
             WHERE id = ?",
        )
        .bind(-1)
        .bind("conn-1")
        .execute(&pool)
        .await
        .expect("corrupt keepalive");

        let read_error = read_connections(&pool).await.expect_err("read fails");
        assert!(read_error.contains("Malformed SSH keepalive interval for Connection 'conn-1'"));
    }

    #[tokio::test]
    async fn schema_map_prefs_default_and_patch_round_trip() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");

        assert_eq!(
            read_schema_map_prefs(&pool, "conn-1", "public")
                .await
                .expect("default prefs"),
            default_schema_map_prefs()
        );

        let prefs = upsert_schema_map_prefs(
            &pool,
            "conn-1",
            "public",
            SchemaMapPrefsPatch {
                routing: Some("step".to_string()),
                attr_mode: Some("keys-only".to_string()),
                show_types: Some(false),
                show_nulls: Some(true),
                show_comments: Some(true),
            },
        )
        .await
        .expect("prefs");

        assert_eq!(
            prefs,
            SchemaMapPrefs {
                routing: "step".to_string(),
                attr_mode: "keys-only".to_string(),
                show_types: false,
                show_nulls: true,
                show_comments: true,
            }
        );
        assert_eq!(
            read_schema_map_prefs(&pool, "conn-1", "public")
                .await
                .expect("saved prefs"),
            prefs
        );
    }

    #[tokio::test]
    async fn table_grid_prefs_round_trip_and_missing_is_null() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");

        assert!(read_table_grid_prefs(&pool, "conn-1", "public", "users")
            .await
            .expect("missing")
            .is_none());

        let prefs = TableGridPrefs(serde_json::json!({
            "version": 1,
            "pageSize": 50,
            "filterHistory": (0..25).collect::<Vec<u32>>(),
        }));
        upsert_table_grid_prefs(&pool, "conn-1", "public", "users", &prefs)
            .await
            .expect("save");
        let loaded = read_table_grid_prefs(&pool, "conn-1", "public", "users")
            .await
            .expect("load")
            .expect("present");
        assert_eq!(loaded.0["version"], 1);
        assert_eq!(loaded.0["pageSize"], 50);
        assert_eq!(loaded.0["filterHistory"].as_array().unwrap().len(), 20);
    }

    #[tokio::test]
    async fn virtual_key_migration_is_applied() {
        let pool = test_pool().await;
        let migration: (i64,) =
            sqlx::query_as("SELECT version FROM schema_migrations WHERE version = 14")
                .fetch_one(&pool)
                .await
                .expect("migration 14");
        let table: (String,) = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'virtual_keys'",
        )
        .fetch_one(&pool)
        .await
        .expect("virtual_keys table");
        assert_eq!(migration.0, 14);
        assert_eq!(table.0, "virtual_keys");
    }

    #[tokio::test]
    async fn virtual_key_round_trip_preserves_order_and_clear_removes_it() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");
        assert_eq!(
            read_postgres_virtual_key(&pool, "conn-1", "public", "users")
                .await
                .expect("missing"),
            None
        );

        let virtual_key = VirtualKey {
            version: 1,
            columns: vec!["tenant_id".to_string(), "email".to_string()],
        };
        upsert_postgres_virtual_key(&pool, "conn-1", "public", "users", &virtual_key)
            .await
            .expect("save");
        assert_eq!(
            read_postgres_virtual_key(&pool, "conn-1", "public", "users")
                .await
                .expect("load"),
            Some(virtual_key)
        );

        let replacement = VirtualKey {
            version: VIRTUAL_KEY_VERSION,
            columns: vec!["email".to_string(), "tenant_id".to_string()],
        };
        upsert_postgres_virtual_key(&pool, "conn-1", "public", "users", &replacement)
            .await
            .expect("replace");
        assert_eq!(
            read_postgres_virtual_key(&pool, "conn-1", "public", "users")
                .await
                .expect("load replacement"),
            Some(replacement)
        );

        clear_postgres_virtual_key(&pool, "conn-1", "public", "users")
            .await
            .expect("clear");
        assert_eq!(
            read_postgres_virtual_key(&pool, "conn-1", "public", "users")
                .await
                .expect("cleared"),
            None
        );
    }

    #[tokio::test]
    async fn read_virtual_key_rejects_every_non_postgres_engine() {
        let pool = test_pool().await;
        for engine in ["MySQL", "SQLite", "ClickHouse", "Redis"] {
            let connection_id = format!("read-{engine}");
            upsert_connection(&pool, &connection(&connection_id))
                .await
                .expect("connection");
            upsert_virtual_key(
                &pool,
                &connection_id,
                "public",
                "users",
                &VirtualKey {
                    version: VIRTUAL_KEY_VERSION,
                    columns: vec!["id".to_string()],
                },
            )
            .await
            .expect("seed virtual key");
            set_connection_engine(&pool, &connection_id, engine).await;

            assert_eq!(
                read_postgres_virtual_key(&pool, &connection_id, "public", "users")
                    .await
                    .expect_err("non-Postgres load must fail"),
                VirtualKeyStorageError::UnsupportedEngine
            );
        }
    }

    #[tokio::test]
    async fn upsert_virtual_key_rejects_every_non_postgres_engine_without_overwriting() {
        let pool = test_pool().await;
        let original = VirtualKey {
            version: VIRTUAL_KEY_VERSION,
            columns: vec!["id".to_string()],
        };
        let replacement = VirtualKey {
            version: VIRTUAL_KEY_VERSION,
            columns: vec!["email".to_string()],
        };
        for engine in ["MySQL", "SQLite", "ClickHouse", "Redis"] {
            let connection_id = format!("save-{engine}");
            upsert_connection(&pool, &connection(&connection_id))
                .await
                .expect("connection");
            upsert_virtual_key(&pool, &connection_id, "public", "users", &original)
                .await
                .expect("seed virtual key");
            set_connection_engine(&pool, &connection_id, engine).await;

            assert_eq!(
                upsert_postgres_virtual_key(
                    &pool,
                    &connection_id,
                    "public",
                    "users",
                    &replacement,
                )
                    .await
                    .expect_err("non-Postgres save must fail"),
                VirtualKeyStorageError::UnsupportedEngine
            );
            let stored: (String,) = sqlx::query_as(
                "SELECT virtual_key FROM virtual_keys
                 WHERE connection_id = ? AND schema = ? AND table_name = ?",
            )
            .bind(&connection_id)
            .bind("public")
            .bind("users")
            .fetch_one(&pool)
            .await
            .expect("stored virtual key");
            assert_eq!(
                serde_json::from_str::<VirtualKey>(&stored.0).expect("valid stored virtual key"),
                original
            );
        }
    }

    #[tokio::test]
    async fn clear_virtual_key_rejects_every_non_postgres_engine_without_deleting() {
        let pool = test_pool().await;
        let virtual_key = VirtualKey {
            version: VIRTUAL_KEY_VERSION,
            columns: vec!["id".to_string()],
        };
        for engine in ["MySQL", "SQLite", "ClickHouse", "Redis"] {
            let connection_id = format!("clear-{engine}");
            upsert_connection(&pool, &connection(&connection_id))
                .await
                .expect("connection");
            upsert_virtual_key(&pool, &connection_id, "public", "users", &virtual_key)
                .await
                .expect("seed virtual key");
            set_connection_engine(&pool, &connection_id, engine).await;

            assert_eq!(
                clear_postgres_virtual_key(&pool, &connection_id, "public", "users")
                    .await
                    .expect_err("non-Postgres clear must fail"),
                VirtualKeyStorageError::UnsupportedEngine
            );
            let row_count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM virtual_keys
                 WHERE connection_id = ? AND schema = ? AND table_name = ?",
            )
            .bind(&connection_id)
            .bind("public")
            .bind("users")
            .fetch_one(&pool)
            .await
            .expect("virtual key count");
            assert_eq!(row_count.0, 1);
        }
    }

    #[tokio::test]
    async fn virtual_key_validation_rejects_invalid_documents() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");

        for (invalid, expected) in [
            (
                VirtualKey {
                    version: 2,
                    columns: vec!["id".to_string()],
                },
                VirtualKeyValidationError::UnsupportedVersion(2),
            ),
            (
                VirtualKey {
                    version: 1,
                    columns: Vec::new(),
                },
                VirtualKeyValidationError::EmptyIdentity,
            ),
            (
                VirtualKey {
                    version: 1,
                    columns: vec!["id".to_string(), "id".to_string()],
                },
                VirtualKeyValidationError::DuplicateColumn,
            ),
        ] {
            assert_eq!(
                upsert_postgres_virtual_key(&pool, "conn-1", "public", "users", &invalid)
                    .await
                    .expect_err("invalid virtual key"),
                VirtualKeyStorageError::InvalidInput(expected)
            );
        }
        let rows_after_invalid_saves: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM virtual_keys WHERE connection_id = ?")
                .bind("conn-1")
                .fetch_one(&pool)
                .await
                .expect("virtual key count");
        assert_eq!(rows_after_invalid_saves.0, 0);

        sqlx::query(
            "INSERT INTO virtual_keys
                (connection_id, schema, table_name, virtual_key, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind("conn-1")
        .bind("public")
        .bind("users")
        .bind(r#"{"version":2,"columns":["id"]}"#)
        .bind(now())
        .execute(&pool)
        .await
        .expect("seed unsupported document");
        assert!(matches!(
            read_virtual_key(&pool, "conn-1", "public", "users")
                .await
                .expect_err("read validates the stored document"),
            VirtualKeyStorageError::CorruptDocument(_)
        ));
    }

    #[tokio::test]
    async fn deleting_connection_cascades_virtual_keys() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");
        upsert_virtual_key(
            &pool,
            "conn-1",
            "public",
            "users",
            &VirtualKey {
                version: 1,
                columns: vec!["id".to_string()],
            },
        )
        .await
        .expect("virtual key");

        assert!(delete_connection(&pool, "conn-1").await.expect("delete"));
        let remaining: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM virtual_keys
             WHERE connection_id = ? AND schema = ? AND table_name = ?",
        )
        .bind("conn-1")
        .bind("public")
        .bind("users")
        .fetch_one(&pool)
        .await
        .expect("cascaded virtual key count");
        assert_eq!(remaining.0, 0);
    }

    #[tokio::test]
    async fn delete_connection_cascades_schema_map_rows() {
        let pool = test_pool().await;
        upsert_connection(&pool, &connection("conn-1"))
            .await
            .expect("connection");
        upsert_schema_map_position(&pool, "conn-1", "public", "public.users", 10.0, 20.0)
            .await
            .expect("position");
        upsert_schema_map_prefs(
            &pool,
            "conn-1",
            "public",
            SchemaMapPrefsPatch {
                routing: Some("step".to_string()),
                ..SchemaMapPrefsPatch::default()
            },
        )
        .await
        .expect("prefs");

        assert!(delete_connection(&pool, "conn-1").await.expect("delete"));

        assert!(read_schema_map_positions(&pool, "conn-1", "public")
            .await
            .expect("positions")
            .is_empty());
        assert_eq!(
            read_schema_map_prefs(&pool, "conn-1", "public")
                .await
                .expect("prefs"),
            default_schema_map_prefs()
        );
    }

    fn cli_entry(id: &str, connection_id: &str, command: &str, ts: &str) -> RedisCliHistoryEntry {
        RedisCliHistoryEntry {
            id: id.to_string(),
            connection_id: connection_id.to_string(),
            command: command.to_string(),
            submitted_at: ts.to_string(),
        }
    }

    #[tokio::test]
    async fn redis_cli_history_round_trip_filters_by_connection() {
        let pool = test_pool().await;
        insert_redis_cli_history(
            &pool,
            &cli_entry("1", "conn-1", "GET foo", "2026-05-16T10:00:00Z"),
        )
        .await
        .expect("insert");
        insert_redis_cli_history(
            &pool,
            &cli_entry("2", "conn-2", "SET bar baz", "2026-05-16T10:01:00Z"),
        )
        .await
        .expect("insert");
        insert_redis_cli_history(
            &pool,
            &cli_entry("3", "conn-1", "HGETALL h", "2026-05-16T10:02:00Z"),
        )
        .await
        .expect("insert");

        let conn1 = read_redis_cli_history(&pool, "conn-1", None)
            .await
            .expect("read");
        assert_eq!(conn1.len(), 2);
        // Newest first.
        assert_eq!(conn1[0].command, "HGETALL h");
        assert_eq!(conn1[1].command, "GET foo");

        let conn2 = read_redis_cli_history(&pool, "conn-2", None)
            .await
            .expect("read");
        assert_eq!(conn2.len(), 1);
        assert_eq!(conn2[0].command, "SET bar baz");
    }

    #[tokio::test]
    async fn redis_cli_history_trims_to_cap_globally() {
        let pool = test_pool().await;
        // Insert REDIS_CLI_HISTORY_CAP + 5 rows so trim has work to do.
        for i in 0..(REDIS_CLI_HISTORY_CAP + 5) {
            // Lexicographic-sortable ISO-ish timestamp so ORDER BY DESC
            // matches insertion order.
            let ts = format!("2026-05-16T10:{:06}Z", i);
            insert_redis_cli_history(&pool, &cli_entry(&format!("id-{i}"), "conn-1", "PING", &ts))
                .await
                .expect("insert");
        }
        let rows = read_redis_cli_history(&pool, "conn-1", None)
            .await
            .expect("read");
        assert_eq!(rows.len(), REDIS_CLI_HISTORY_CAP as usize);
        // The oldest 5 entries should have been trimmed.
        assert!(rows.iter().all(|row| row.id != "id-0"));
        assert!(rows.iter().all(|row| row.id != "id-4"));
    }
}
