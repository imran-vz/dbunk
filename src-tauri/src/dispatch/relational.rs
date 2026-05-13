//! Relational-class dispatch (PG / MySQL / SQLite / CH).
//!
//! Was the entirety of `dispatch.rs` before the StorageClass fork
//! (ADR-0008). The public-facing dispatcher in `dispatch.rs` now
//! routes by `engine.storage_class()` and calls into this module
//! when the active engine is relational.
//!
//! Every match here is exhaustive over `DatabaseEngine` — no `_ =>`
//! wildcards. The `Redis` arm is `unreachable!()` because the router
//! contract guarantees Redis never reaches here; this is an explicit
//! invariant assertion, not a fallback.

use std::str::FromStr;
use std::sync::Once;
use std::time::Instant;

use sqlx::any::{AnyConnectOptions, AnyRow};
use sqlx::{Any, AnyConnection, Column, Connection, Row};

use crate::{
    bytes_to_hex, clickhouse, postgres, CellEdit, CellEditKeyValue, ColumnInfo,
    CommitCellEditsResult, ConnectResult, CopyTableResult, DatabaseEngine, DatabaseOverviewStats,
    DeleteRowsResult, ExecuteDdlResult, ExportDdlResult, ImportRowsResult, InsertRowResult,
    MutationStatus, PgDumpResult, PgRestoreResult, QueryResult, RelationInfo, SchemaExplorer,
    SchemaRelationships, ServerDetails, StoredConnection, StructureCapabilities, TableStructure,
};

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

pub(crate) fn engine_name(engine: &DatabaseEngine) -> &'static str {
    match engine {
        DatabaseEngine::PostgreSQL => "PostgreSQL",
        DatabaseEngine::MySQL => "MySQL",
        DatabaseEngine::ClickHouse => "ClickHouse",
        DatabaseEngine::SQLite => "SQLite",
        DatabaseEngine::Redis => "Redis",
    }
}

/// "We will support this on this engine — it just hasn't been
/// implemented yet." Used for engines in the SQL family that haven't
/// caught up on a feature (e.g. MySQL DDL today).
fn not_implemented_yet(engine: &DatabaseEngine, operation: &str) -> String {
    format!(
        "{operation} is not yet implemented for {} (PostgreSQL only). \
         Per-engine coverage is tracked in designs/.",
        engine_name(engine)
    )
}

/// Translate a `sqlx::Error` from a connect attempt into a message
/// the user can act on. The default `Display` for `sqlx::Error::Io`
/// surfaces raw OS-error text like `Connection refused (os error 61)`,
/// which leaks platform detail and offers no guidance — this maps
/// the common failure modes (refused / DNS / timeout / TLS / auth)
/// to plain-English explanations.
pub(crate) fn friendly_sqlx_error(error: sqlx::Error, host: &str, port: u16) -> String {
    use std::io::ErrorKind;

    let endpoint = if port == 0 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };

    match &error {
        sqlx::Error::Io(io_err) => match io_err.kind() {
            ErrorKind::ConnectionRefused => format!(
                "Could not connect to {endpoint}: connection refused. \
                 Check that the database server is running and listening on this port."
            ),
            ErrorKind::TimedOut => format!(
                "Connection to {endpoint} timed out. \
                 The host may be unreachable or blocked by a firewall."
            ),
            ErrorKind::ConnectionReset => {
                format!("Connection to {endpoint} was reset by the server.")
            }
            ErrorKind::ConnectionAborted => {
                format!("Connection to {endpoint} was aborted by the server.")
            }
            ErrorKind::HostUnreachable => {
                format!("Host \"{host}\" is unreachable from this network.")
            }
            ErrorKind::NetworkUnreachable | ErrorKind::NetworkDown => {
                format!("Network is unreachable while connecting to {endpoint}.")
            }
            _ => {
                // DNS failures don't have a dedicated `ErrorKind` and vary
                // by platform, so sniff the message text as a fallback.
                let text = io_err.to_string();
                let lower = text.to_lowercase();
                if lower.contains("nodename nor servname")
                    || lower.contains("name or service not known")
                    || lower.contains("no such host")
                    || lower.contains("failed to lookup address")
                    || lower.contains("temporary failure in name resolution")
                {
                    format!(
                        "Could not resolve hostname \"{host}\". \
                         Check the address and your DNS settings."
                    )
                } else {
                    format!("Network error connecting to {endpoint}: {text}")
                }
            }
        },
        sqlx::Error::Tls(err) => {
            format!("TLS handshake with {endpoint} failed: {err}")
        }
        sqlx::Error::Database(err) => err.message().to_string(),
        sqlx::Error::PoolTimedOut => {
            format!("Timed out waiting for a connection to {endpoint}.")
        }
        sqlx::Error::PoolClosed => "Connection pool is closed.".to_string(),
        _ => error.to_string(),
    }
}

// ---------------------------------------------------------------------------
// sqlx-Any internals (private to this module)
// ---------------------------------------------------------------------------

static SQLX_DRIVER_INIT: Once = Once::new();

/// Install the sqlx-Any drivers once for the process. Called eagerly
/// from the Tauri app boot path so the first query doesn't pay the
/// init cost; also called defensively from every sqlx-Any code path.
pub(crate) fn ensure_sqlx_drivers() {
    SQLX_DRIVER_INIT.call_once(|| {
        sqlx::any::install_default_drivers();
    });
}

/// Distinguish row-returning queries (SELECT / WITH / SHOW / etc.) from
/// row-affected statements (INSERT / UPDATE / DELETE / DDL). Used by
/// every engine's `run_query` to pick between `fetch_all` and
/// `execute`. Public to the crate so engine modules
/// (`postgres::run_query`) can reuse the classifier.
pub(crate) fn should_fetch_rows(query: &str) -> bool {
    let trimmed = query.trim_start().to_lowercase();
    trimmed.starts_with("select")
        || trimmed.starts_with("with")
        || trimmed.starts_with("show")
        || trimmed.starts_with("describe")
        || trimmed.starts_with("pragma")
        || trimmed.starts_with("explain")
}

fn sqlite_dsn(database: &str) -> Result<String, String> {
    if database.trim().is_empty() {
        return Err("SQLite database path is required".to_string());
    }
    if database.starts_with("sqlite:") || database.starts_with("file:") {
        return Ok(database.to_string());
    }
    if database == ":memory:" {
        return Ok("sqlite::memory:".to_string());
    }
    Ok(format!("sqlite://{}", database))
}

fn sqlx_dsn(connection: &StoredConnection) -> Result<String, String> {
    match connection {
        StoredConnection::PostgreSQL(c) => {
            if c.host.is_empty() || c.user.is_empty() {
                return Err("PostgreSQL host and user are required".to_string());
            }
            let port = if c.port == 0 { 5432 } else { c.port };
            // ADR-0010 threads `ssl` into the PG DSN as
            // `sslmode=prefer` (default) or `sslmode=disable`. The
            // canonical PG sqlx path is `postgres.rs::connect` which
            // builds `PgConnectOptions` directly; this DSN string is
            // only used for the sqlx-Any fallback below and stays in
            // sync via the same toggle.
            let sslmode = if c.ssl { "prefer" } else { "disable" };
            Ok(format!(
                "postgres://{}:{}@{}:{}/{}?sslmode={}",
                c.user, c.password, c.host, port, c.database, sslmode
            ))
        }
        StoredConnection::MySQL(c) => {
            if c.host.is_empty() || c.user.is_empty() {
                return Err("MySQL host and user are required".to_string());
            }
            let port = if c.port == 0 { 3306 } else { c.port };
            // MySQL's sqlx driver reads `ssl-mode` from the URL.
            // Translate the binary toggle into the closest canonical
            // values: `preferred` (negotiate-if-server-supports) when
            // on, `disabled` when off.
            let ssl_mode = if c.ssl { "preferred" } else { "disabled" };
            Ok(format!(
                "mysql://{}:{}@{}:{}/{}?ssl-mode={}",
                c.user, c.password, c.host, port, c.database, ssl_mode
            ))
        }
        StoredConnection::SQLite(c) => sqlite_dsn(&c.database),
        StoredConnection::ClickHouse(_) => Err("ClickHouse uses HTTP client".to_string()),
        StoredConnection::Redis(_) => {
            unreachable!("BUG: relational dispatch reached for Redis — router contract violated")
        }
    }
}

fn value_to_string(row: &AnyRow, index: usize) -> String {
    use sqlx::TypeInfo;

    // Check the column type name to handle unsupported PostgreSQL types
    let column = &row.columns()[index];
    let type_name = column.type_info().name();

    // Handle PostgreSQL types not natively supported by the Any driver
    // by decoding them as text representation
    let unsupported_pg_types = [
        "BOOL",
        "TIMESTAMPTZ",
        "TIMESTAMP",
        "DATE",
        "TIME",
        "TIMETZ",
        "UUID",
        "JSON",
        "JSONB",
        "INTERVAL",
        "INET",
        "CIDR",
        "MACADDR",
    ];

    if unsupported_pg_types.contains(&type_name) {
        use sqlx::Row;
        use sqlx::ValueRef;
        if let Ok(raw_value) = row.try_get_raw(index) {
            if raw_value.is_null() {
                return "NULL".to_string();
            }
            if let Ok(s) = <&str as sqlx::Decode<sqlx::any::Any>>::decode(raw_value) {
                if type_name == "BOOL" {
                    return match s {
                        "t" | "true" | "1" => "true".to_string(),
                        "f" | "false" | "0" => "false".to_string(),
                        _ => s.to_string(),
                    };
                }
                return s.to_string();
            }
        }
        return "NULL".to_string();
    }

    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value.unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(index) {
        return value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i16>, _>(index) {
        return value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
        return value
            .map(|value| value.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(|bytes| bytes_to_hex(&bytes))
            .unwrap_or_else(|| "NULL".to_string());
    }
    "NULL".to_string()
}

fn row_to_strings(row: &AnyRow) -> Vec<String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, _)| value_to_string(row, index))
        .collect()
}

async fn fetch_rows(
    conn: &mut AnyConnection,
    query: &str,
    bind: Option<&str>,
) -> Result<Vec<Vec<String>>, String> {
    let mut builder = sqlx::query::<Any>(query);
    if let Some(value) = bind {
        builder = builder.bind(value);
    }
    let rows = builder
        .fetch_all(conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows.iter().map(row_to_strings).collect())
}

async fn fetch_column(
    conn: &mut AnyConnection,
    query: &str,
    bind: Option<&str>,
) -> Result<Vec<String>, String> {
    Ok(fetch_rows(conn, query, bind)
        .await?
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .filter(|value| value != "NULL")
        .collect())
}

/// Run a query through sqlx-Any. The shared path for engines we don't
/// have a native driver for yet (MySQL, SQLite). PostgreSQL bypasses
/// this via the native `postgres::run_query` route in
/// [`run_query`] below; ClickHouse never reaches here.
async fn run_sqlx_any(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    ensure_sqlx_drivers();
    let dsn = sqlx_dsn(connection)?;
    let options = AnyConnectOptions::from_str(&dsn).map_err(|error| error.to_string())?;
    let mut conn = AnyConnection::connect_with(&options)
        .await
        .map_err(|error| friendly_sqlx_error(error, connection.host(), connection.port()))?;
    let start = Instant::now();

    if should_fetch_rows(query) {
        let rows = sqlx::query::<Any>(query)
            .fetch_all(&mut conn)
            .await
            .map_err(|error| error.to_string())?;
        let columns = rows
            .first()
            .map(|row| {
                row.columns()
                    .iter()
                    .map(|column| column.name().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let values = rows.iter().map(row_to_strings).collect::<Vec<_>>();
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns,
            rows: values,
            runtime_ms,
            row_count: rows.len() as u64,
        })
    } else {
        let result = sqlx::query::<Any>(query)
            .execute(&mut conn)
            .await
            .map_err(|error| error.to_string())?;
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            runtime_ms,
            row_count: result.rows_affected(),
        })
    }
}

async fn fetch_schema_explorer_sqlx(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    ensure_sqlx_drivers();
    let dsn = sqlx_dsn(connection)?;
    let options = AnyConnectOptions::from_str(&dsn).map_err(|error| error.to_string())?;
    let mut conn = AnyConnection::connect_with(&options)
        .await
        .map_err(|error| friendly_sqlx_error(error, connection.host(), connection.port()))?;

    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            let schemas = fetch_column(
                &mut conn,
                "SELECT nspname::text FROM pg_namespace WHERE nspname NOT IN ('information_schema') AND nspname NOT LIKE 'pg_%' ORDER BY nspname",
                None,
            )
            .await?;
            let mut explorer = Vec::new();
            for schema in schemas {
                let tables = fetch_column(
                    &mut conn,
                    "SELECT tablename::text FROM pg_tables WHERE schemaname = $1 ORDER BY tablename",
                    Some(&schema),
                )
                .await?;
                let views = fetch_column(
                    &mut conn,
                    "SELECT viewname::text FROM pg_views WHERE schemaname = $1 ORDER BY viewname",
                    Some(&schema),
                )
                .await?;
                let materialized_views = fetch_column(
                    &mut conn,
                    "SELECT matviewname::text FROM pg_matviews WHERE schemaname = $1 ORDER BY matviewname",
                    Some(&schema),
                )
                .await?;
                let sequences = fetch_column(
                    &mut conn,
                    "SELECT sequence_name::text FROM information_schema.sequences WHERE sequence_schema = $1 ORDER BY sequence_name",
                    Some(&schema),
                )
                .await?;
                let foreign_tables = fetch_column(
                    &mut conn,
                    "SELECT foreign_table_name::text FROM information_schema.foreign_tables WHERE foreign_table_schema = $1 ORDER BY foreign_table_name",
                    Some(&schema),
                )
                .await?;
                let functions = fetch_column(
                    &mut conn,
                    "SELECT p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1 AND p.prokind = 'f' ORDER BY p.proname",
                    Some(&schema),
                )
                .await?;
                let procedures = fetch_column(
                    &mut conn,
                    "SELECT p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1 AND p.prokind = 'p' ORDER BY p.proname",
                    Some(&schema),
                )
                .await?;
                let aggregate_functions = fetch_column(
                    &mut conn,
                    "SELECT p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')' FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1 AND p.prokind = 'a' ORDER BY p.proname",
                    Some(&schema),
                )
                .await?;
                let types = fetch_column(
                    &mut conn,
                    "SELECT t.typname::text FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = $1 AND t.typtype IN ('c', 'e', 'r', 'm') AND t.typname NOT LIKE '\\_%' ORDER BY t.typname",
                    Some(&schema),
                )
                .await?;
                let domains = fetch_column(
                    &mut conn,
                    "SELECT domain_name::text FROM information_schema.domains WHERE domain_schema = $1 ORDER BY domain_name",
                    Some(&schema),
                )
                .await?;
                let extensions = fetch_column(
                    &mut conn,
                    "SELECT e.extname::text FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace WHERE n.nspname = $1 ORDER BY e.extname",
                    Some(&schema),
                )
                .await?;
                explorer.push(SchemaExplorer {
                    name: schema,
                    tables,
                    views,
                    materialized_views,
                    sequences,
                    foreign_tables,
                    functions,
                    procedures,
                    aggregate_functions,
                    types,
                    domains,
                    extensions,
                    event_triggers: vec![],
                    roles: vec![],
                    tablespaces: vec![],
                });
            }
            let event_triggers = fetch_column(
                &mut conn,
                "SELECT evtname::text FROM pg_event_trigger ORDER BY evtname",
                None,
            )
            .await?;
            let roles = fetch_column(
                &mut conn,
                "SELECT rolname::text FROM pg_roles ORDER BY rolname",
                None,
            )
            .await?;
            let tablespaces = fetch_column(
                &mut conn,
                "SELECT spcname::text FROM pg_tablespace ORDER BY spcname",
                None,
            )
            .await?;
            if !event_triggers.is_empty() || !roles.is_empty() || !tablespaces.is_empty() {
                explorer.push(SchemaExplorer {
                    name: "Database".to_string(),
                    tables: vec![],
                    views: vec![],
                    materialized_views: vec![],
                    sequences: vec![],
                    foreign_tables: vec![],
                    functions: vec![],
                    procedures: vec![],
                    aggregate_functions: vec![],
                    types: vec![],
                    domains: vec![],
                    extensions: vec![],
                    event_triggers,
                    roles,
                    tablespaces,
                });
            }
            Ok(explorer)
        }
        DatabaseEngine::MySQL => {
            if connection.database().trim().is_empty() {
                return Err("MySQL database is required".to_string());
            }
            let tables = fetch_column(
                &mut conn,
                "SELECT CAST(table_name AS CHAR) FROM information_schema.tables WHERE table_schema = ? AND table_type = 'BASE TABLE' ORDER BY table_name",
                Some(connection.database()),
            )
            .await?;
            let views = fetch_column(
                &mut conn,
                "SELECT CAST(table_name AS CHAR) FROM information_schema.views WHERE table_schema = ? ORDER BY table_name",
                Some(connection.database()),
            )
            .await?;
            Ok(vec![SchemaExplorer {
                name: connection.database().to_string(),
                tables,
                views,
                materialized_views: vec![],
                sequences: vec![],
                foreign_tables: vec![],
                functions: vec![],
                procedures: vec![],
                aggregate_functions: vec![],
                types: vec![],
                domains: vec![],
                extensions: vec![],
                event_triggers: vec![],
                roles: vec![],
                tablespaces: vec![],
            }])
        }
        DatabaseEngine::SQLite => {
            let rows = fetch_rows(
                &mut conn,
                "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
                None,
            )
            .await?;
            let mut tables = Vec::new();
            let mut views = Vec::new();
            for row in rows {
                let name = row.first().cloned().unwrap_or_default();
                let kind = row
                    .get(1)
                    .map(|value| value.to_lowercase())
                    .unwrap_or_default();
                if name.is_empty() || name == "NULL" {
                    continue;
                }
                if kind == "view" {
                    views.push(name);
                } else {
                    tables.push(name);
                }
            }
            Ok(vec![SchemaExplorer {
                name: "main".to_string(),
                tables,
                views,
                materialized_views: vec![],
                sequences: vec![],
                foreign_tables: vec![],
                functions: vec![],
                procedures: vec![],
                aggregate_functions: vec![],
                types: vec![],
                domains: vec![],
                extensions: vec![],
                event_triggers: vec![],
                roles: vec![],
                tablespaces: vec![],
            }])
        }
        DatabaseEngine::ClickHouse => {
            // Defensive: caller should have routed CH away before now.
            Err("ClickHouse does not use the sqlx-Any path".to_string())
        }
        DatabaseEngine::Redis => {
            unreachable!("BUG: relational dispatch reached for Redis")
        }
    }
}

/// Columns-only fallback for engines with no native introspection yet.
/// Probes the table with `SELECT * LIMIT 0` and reports column names with
/// empty type information. Capability flags surface the limitation to
/// the UI.
async fn fetch_table_structure_columns_only(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    let qualified = crate::qualified_table_name(&connection.engine(), schema, table);
    let probe = format!("SELECT * FROM {} LIMIT 0", qualified);
    let result = run_query(connection, &probe).await?;

    let columns = result
        .columns
        .into_iter()
        .enumerate()
        .map(|(index, name)| ColumnInfo {
            name,
            data_type: "unknown".to_string(),
            nullable: true,
            default_value: None,
            is_primary_key: false,
            ordinal_position: (index as i32) + 1,
        })
        .collect();

    Ok(TableStructure {
        columns,
        primary_key: None,
        foreign_keys: Vec::new(),
        indexes: Vec::new(),
        constraints: Vec::new(),
        capabilities: StructureCapabilities {
            columns: true,
            primary_key: false,
            foreign_keys: false,
            indexes: false,
            constraints: false,
            can_insert_rows: false,
            can_update_rows: false,
            can_delete_rows: false,
            can_alter_schema: false,
            uniqueness_guarantee: "best-effort".to_string(),
        },
        table_engine: None,
        partition_by: None,
        sample_by: None,
    })
}

// ---------------------------------------------------------------------------
// Public dispatch surface — relational only
// ---------------------------------------------------------------------------

/// Connect + `SELECT 1` to verify the connection is live and measure
/// latency. Routes ClickHouse through HTTP; everything else through
/// sqlx-Any connect (PostgreSQL doesn't need its native driver for a
/// liveness check).
pub async fn ping_connection(connection: &StoredConnection) -> Result<ConnectResult, String> {
    match connection.engine() {
        DatabaseEngine::ClickHouse => {
            let result = clickhouse::run_query(connection, "SELECT 1").await?;
            Ok(ConnectResult {
                latency_ms: result.runtime_ms,
                redis_capabilities: None,
            })
        }
        DatabaseEngine::PostgreSQL | DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            ensure_sqlx_drivers();
            let dsn = sqlx_dsn(connection)?;
            let start = Instant::now();
            let options = AnyConnectOptions::from_str(&dsn).map_err(|error| error.to_string())?;
            let _connection = AnyConnection::connect_with(&options)
                .await
                .map_err(|error| friendly_sqlx_error(error, connection.host(), connection.port()))?;
            Ok(ConnectResult {
                latency_ms: start.elapsed().as_millis() as u64,
                redis_capabilities: None,
            })
        }
        DatabaseEngine::Redis => {
            unreachable!("BUG: relational dispatch reached for Redis")
        }
    }
}

/// Run an ad-hoc query against the connection's engine. PostgreSQL uses
/// the native driver (for richer type coverage); ClickHouse uses HTTP;
/// MySQL/SQLite use sqlx-Any.
pub async fn run_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::run_query(connection, query).await,
        DatabaseEngine::ClickHouse => clickhouse::run_query(connection, query).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => run_sqlx_any(connection, query).await,
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn load_schema_explorer(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    match connection.engine() {
        DatabaseEngine::ClickHouse => clickhouse::fetch_schema_explorer(connection).await,
        DatabaseEngine::PostgreSQL | DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            fetch_schema_explorer_sqlx(connection).await
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn fetch_table_structure(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            postgres::fetch_table_structure(connection, schema, table).await
        }
        DatabaseEngine::ClickHouse => {
            clickhouse::fetch_table_structure(connection, schema, table).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            // Columns-only probe with all `capabilities.*` falsy — the UI
            // hides the disabled sections accordingly. Native engine
            // modules will replace this when MySQL / SQLite catch up.
            fetch_table_structure_columns_only(connection, schema, table)
                .await
                .map_err(|error| {
                    format!(
                        "Structure inspection is not yet supported for {}: {}",
                        engine_name(&connection.engine()),
                        error
                    )
                })
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn fetch_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
) -> Result<SchemaRelationships, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            postgres::fetch_schema_relationships(connection, schema).await
        }
        DatabaseEngine::ClickHouse => {
            clickhouse::fetch_schema_relationships(connection, schema).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            // Empty rather than error — keeps the relationship-map panel
            // renderable on engines we haven't introspected yet.
            Ok(SchemaRelationships {
                tables: Vec::new(),
                foreign_keys: Vec::new(),
            })
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn fetch_database_overview_stats(
    connection: &StoredConnection,
) -> Result<DatabaseOverviewStats, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::load_database_overview_stats(connection).await,
        DatabaseEngine::ClickHouse => clickhouse::fetch_database_overview_stats(connection).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => Err(not_implemented_yet(
            &connection.engine(),
            "Database overview stats",
        )),
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

/// Per-relation stats for the Tables + Schemas sub-tabs. Postgres-only
/// in Phase 1; other relational engines return an empty list so the
/// Tables sub-tab on MySQL / SQLite / ClickHouse can still render
/// schema/name/kind columns from `schemaExplorer` without the call
/// surfacing an error to the user. The Schemas sub-tab is gated to
/// Postgres in the UI.
pub async fn fetch_relation_stats(
    connection: &StoredConnection,
) -> Result<Vec<RelationInfo>, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::load_relation_stats(connection).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => Ok(Vec::new()),
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

/// Server-info snapshot for the Details sub-tab — pg_settings, pg_extension,
/// plus server version / encoding / locale / timezone. Postgres-only;
/// other relational engines return an explicit not-implemented error
/// since the UI gates the Details sub-tab to PG and never calls this.
pub async fn fetch_server_details(
    connection: &StoredConnection,
) -> Result<ServerDetails, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::load_server_details(connection).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => {
            Err(not_implemented_yet(&connection.engine(), "Server details"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn execute_ddl(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::execute_ddl(connection, sql).await,
        DatabaseEngine::ClickHouse => clickhouse::execute_ddl(connection, sql).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine(), "DDL execution"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn export_ddl(
    connection: &StoredConnection,
    scope: &str,
    schema: Option<&str>,
    table: Option<&str>,
) -> Result<ExportDdlResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::export_ddl(connection, scope, schema, table).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => {
            Err(not_implemented_yet(&connection.engine(), "DDL export"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn run_pg_dump(
    connection: &StoredConnection,
    scope: &str,
    schema: Option<&str>,
    table: Option<&str>,
    format: &str,
) -> Result<PgDumpResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            postgres::run_pg_dump(connection, scope, schema, table, format).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => {
            Err(not_implemented_yet(&connection.engine(), "PostgreSQL dump"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn run_pg_restore(
    connection: &StoredConnection,
    data_base64: &str,
    format: &str,
    clean: bool,
) -> Result<PgRestoreResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            postgres::run_pg_restore(connection, data_base64, format, clean).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => {
            Err(not_implemented_yet(&connection.engine(), "PostgreSQL restore"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn refresh_materialized_view(
    connection: &StoredConnection,
    schema: &str,
    view: &str,
    concurrently: bool,
) -> Result<ExecuteDdlResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            postgres::refresh_materialized_view(connection, schema, view, concurrently).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => {
            Err(not_implemented_yet(&connection.engine(), "Materialized view refresh"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn commit_cell_edits(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    edits: &[CellEdit],
) -> Result<CommitCellEditsResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => {
            postgres::commit_cell_edits(connection, schema, table, edits).await
        }
        DatabaseEngine::ClickHouse => {
            clickhouse::commit_cell_edits(connection, schema, table, edits).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine(), "Cell edit commit"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn insert_row(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    values: &[CellEditKeyValue],
) -> Result<InsertRowResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::insert_row(connection, schema, table, values).await,
        DatabaseEngine::ClickHouse => {
            clickhouse::insert_row(connection, schema, table, values).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine(), "Row insert"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn import_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
    use_copy: bool,
) -> Result<ImportRowsResult, String> {
    if rows.is_empty() {
        return Ok(ImportRowsResult {
            runtime_ms: 0,
            rows_affected: 0,
        });
    }
    match connection.engine() {
        DatabaseEngine::PostgreSQL if use_copy => {
            postgres::copy_import_rows(connection, schema, table, columns, rows).await
        }
        DatabaseEngine::PostgreSQL => {
            postgres::import_rows(connection, schema, table, columns, rows).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite | DatabaseEngine::ClickHouse => {
            let start = Instant::now();
            let mut rows_affected = 0;
            for row in rows {
                let values = columns
                    .iter()
                    .zip(row.iter())
                    .map(|(column, value)| CellEditKeyValue {
                        column: column.clone(),
                        value: value.clone(),
                    })
                    .collect::<Vec<_>>();
                rows_affected += insert_row(connection, schema, table, &values)
                    .await?
                    .rows_affected;
            }
            Ok(ImportRowsResult {
                runtime_ms: start.elapsed().as_millis() as u64,
                rows_affected,
            })
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

pub async fn copy_table_rows(
    source: &StoredConnection,
    destination: &StoredConnection,
    source_schema: &str,
    source_table: &str,
    destination_schema: &str,
    destination_table: &str,
    page_size: u32,
) -> Result<CopyTableResult, String> {
    let start = Instant::now();
    let source_qualified =
        crate::qualified_table_name(&source.engine(), source_schema, source_table);
    let destination_structure =
        fetch_table_structure(destination, destination_schema, destination_table).await?;
    let columns = destination_structure
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let select_columns = columns
        .iter()
        .map(|column| match source.engine() {
            DatabaseEngine::PostgreSQL | DatabaseEngine::SQLite => crate::quote_double(column),
            DatabaseEngine::MySQL | DatabaseEngine::ClickHouse => crate::quote_backtick(column),
            DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let page_size = page_size.clamp(1, 1000);
    let mut offset = 0_u64;
    let mut rows_copied = 0_u64;
    loop {
        let sql = format!(
            "SELECT {} FROM {} LIMIT {} OFFSET {}",
            select_columns, source_qualified, page_size, offset
        );
        let result = run_query(source, &sql).await?;
        if result.rows.is_empty() {
            break;
        }
        let rows = result
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| {
                        if value == "NULL" {
                            None
                        } else {
                            Some(value.clone())
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        rows_copied += import_rows(
            destination,
            destination_schema,
            destination_table,
            &columns,
            &rows,
            false,
        )
        .await?
        .rows_affected;
        if result.rows.len() < page_size as usize {
            break;
        }
        offset += page_size as u64;
    }
    Ok(CopyTableResult {
        runtime_ms: start.elapsed().as_millis() as u64,
        rows_copied,
    })
}

pub async fn delete_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    rows: &[Vec<CellEditKeyValue>],
) -> Result<DeleteRowsResult, String> {
    match connection.engine() {
        DatabaseEngine::PostgreSQL => postgres::delete_rows(connection, schema, table, rows).await,
        DatabaseEngine::ClickHouse => {
            clickhouse::delete_rows(connection, schema, table, rows).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine(), "Row delete"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

/// Async mutation polling — ClickHouse-only today (PG mutations are
/// synchronous; MySQL/SQLite haven't grown async mutations).
pub async fn poll_mutation_status(
    connection: &StoredConnection,
    database: &str,
    table: &str,
    mutation_ids: &[String],
) -> Result<Vec<MutationStatus>, String> {
    match connection.engine() {
        DatabaseEngine::ClickHouse => {
            clickhouse::poll_mutations(connection, database, table, mutation_ids).await
        }
        DatabaseEngine::PostgreSQL | DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine(), "Mutation polling"))
        }
        DatabaseEngine::Redis => unreachable!("BUG: relational dispatch reached for Redis"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn io_error(kind: io::ErrorKind, msg: &str) -> sqlx::Error {
        sqlx::Error::Io(io::Error::new(kind, msg))
    }

    #[test]
    fn connection_refused_explains_what_to_check() {
        let msg = friendly_sqlx_error(
            io_error(io::ErrorKind::ConnectionRefused, "Connection refused"),
            "localhost",
            5433,
        );
        assert!(msg.contains("localhost:5433"));
        assert!(msg.contains("connection refused"));
        assert!(msg.contains("listening on this port"));
        assert!(!msg.contains("os error"));
    }

    #[test]
    fn timed_out_mentions_firewall() {
        let msg = friendly_sqlx_error(
            io_error(io::ErrorKind::TimedOut, "operation timed out"),
            "10.0.0.1",
            5432,
        );
        assert!(msg.contains("10.0.0.1:5432"));
        assert!(msg.contains("timed out"));
        assert!(msg.contains("firewall"));
    }

    #[test]
    fn dns_failure_text_is_recognised() {
        let msg = friendly_sqlx_error(
            io_error(
                io::ErrorKind::Other,
                "failed to lookup address information: nodename nor servname provided",
            ),
            "no-such-host.invalid",
            5432,
        );
        assert!(msg.contains("Could not resolve hostname"));
        assert!(msg.contains("no-such-host.invalid"));
    }

    #[test]
    fn port_zero_is_omitted_from_endpoint() {
        let msg = friendly_sqlx_error(
            io_error(io::ErrorKind::ConnectionRefused, "Connection refused"),
            "db.example.com",
            0,
        );
        assert!(msg.contains("db.example.com"));
        assert!(!msg.contains(":0"));
    }

    #[test]
    fn unknown_io_error_falls_back_with_endpoint_context() {
        let msg = friendly_sqlx_error(
            io_error(io::ErrorKind::PermissionDenied, "permission denied"),
            "host",
            1234,
        );
        assert!(msg.contains("host:1234"));
        assert!(msg.contains("permission denied"));
    }
}
