//! SQLite persistence for dbunk's local app state.
//!
//! ADR-0007 makes `~/.config/dbunk/dbunk.sqlite` the primary local store.
//! This module owns path resolution, pool setup, lightweight embedded
//! migrations, and per-entity read/write helpers.

use std::{fs, path::PathBuf, str::FromStr};

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

use crate::{
    ClickHouseStoredConnection, CredentialStorageMode, DatabaseEngine, MySqlStoredConnection,
    PgStoredConnection, PositionRow, QueryHistoryEntry, RedisStoredConnection, SavedQuery,
    SchemaMapPrefs, SchemaMapPrefsPatch, SqliteStoredConnection, StoredConnection,
};

const DB_FILE: &str = "dbunk.sqlite";

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

pub async fn read_connections(pool: &SqlitePool) -> Result<Vec<StoredConnection>, String> {
    let rows = sqlx::query(
        "SELECT id, name, database_name, engine, host, port, user_name, role,
                last_activity_at, use_https, url_path,
                db_number, use_tls, verify_tls_cert, ssl, driver_options
         FROM connections
         ORDER BY name COLLATE NOCASE ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;

    rows.into_iter()
        .map(|row| {
            // Row → variant construction. The SQLite schema stays flat
            // (one row per connection, engine-irrelevant columns simply
            // unread per variant); we match on the `engine` column to
            // pick which variant to build. ADR-0010 covers the design.
            let engine = DatabaseEngine::from_str(row.get::<String, _>("engine").as_str())?;
            let id: String = row.get("id");
            let name: String = row.get("name");
            let database: String = row.get("database_name");
            let host: String = row.get("host");
            let port = i64_to_u16(row.get("port"));
            let user: String = row.get("user_name");
            let role: String = row.get("role");
            let last_activity_at: Option<String> = row.get("last_activity_at");

            let driver_options_json: Option<String> = row.get("driver_options");
            // Parse the JSON blob lazily; a corrupted blob falls back
            // to "no options" rather than failing the whole load. We
            // log to stderr so the bad row is discoverable.
            let driver_options = driver_options_json.and_then(|raw| {
                if raw.is_empty() {
                    return None;
                }
                match serde_json::from_str::<crate::types::PgDriverOptions>(&raw) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        eprintln!(
                            "warning: ignoring malformed driver_options for connection: {error}"
                        );
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
                DatabaseEngine::ClickHouse => {
                    StoredConnection::ClickHouse(ClickHouseStoredConnection {
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
                    })
                }
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
                }),
            })
        })
        .collect()
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
    let (db_number, use_tls, verify_tls_cert) = match connection {
        StoredConnection::Redis(c) => (
            i64::from(c.db_number),
            bool_to_i64(c.use_tls),
            bool_to_i64(c.verify_tls_cert),
        ),
        _ => (0, 0, 1),
    };
    let ssl = match connection {
        StoredConnection::PostgreSQL(c) => bool_to_i64(c.ssl),
        StoredConnection::MySQL(c) => bool_to_i64(c.ssl),
        _ => 1,
    };
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
            db_number, use_tls, verify_tls_cert, ssl, driver_options
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            driver_options = excluded.driver_options",
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

// ---------------------------------------------------------------------------
// SQLite credentials
// ---------------------------------------------------------------------------

pub async fn read_sqlite_credentials(
    pool: &SqlitePool,
) -> Result<Vec<(String, Option<String>, String)>, String> {
    let rows = sqlx::query("SELECT connection_id, nonce, password_value FROM credentials")
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.get("connection_id"),
                row.get("nonce"),
                row.get("password_value"),
            )
        })
        .collect())
}

pub async fn upsert_sqlite_credential(
    pool: &SqlitePool,
    connection_id: &str,
    mode: CredentialStorageMode,
    nonce: Option<&str>,
    password_value: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO credentials (connection_id, storage_mode, nonce, password_value, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(connection_id) DO UPDATE SET
            storage_mode = excluded.storage_mode,
            nonce = excluded.nonce,
            password_value = excluded.password_value,
            updated_at = excluded.updated_at",
    )
    .bind(connection_id)
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

#[cfg(test)]
mod tests {
    use super::*;
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
        })
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
}
