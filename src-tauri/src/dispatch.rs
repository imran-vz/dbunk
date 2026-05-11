//! Engine dispatch — single point of truth for routing engine-aware
//! operations to the right per-engine implementation.
//!
//! Each public function in this module matches on `DatabaseEngine` and
//! delegates to the appropriate engine module (`postgres::`,
//! `clickhouse::`, etc.) or to a sqlx-Any fallback. Adding a new engine
//! means editing **one** file (this one): the compiler enforces that
//! every match exhaustively handles every variant, so a new
//! `DatabaseEngine` variant won't slip through silently.
//!
//! Tauri commands in `lib.rs` validate payloads and track activity; they
//! delegate the "what does this operation mean on this engine" question
//! to these functions. See `CONTEXT.md` for the Engine Dispatch glossary
//! entry and ADR-0001 for the per-feature catch-up policy.
//!
//! ## Two error shapes
//!
//! - [`not_implemented_yet`] — the operation is in scope for the engine
//!   but hasn't been implemented (MySQL DDL today, say). The frontend
//!   shows it as a "not supported" wall.
//! - [`not_applicable`] — the operation does not exist for the engine's
//!   class (DDL on Redis once it lands). The frontend can choose to hide
//!   the affordance entirely.
//!
//! ## No `_ =>` wildcards
//!
//! Every match exhaustively handles every `DatabaseEngine` variant. This
//! costs a few extra lines per function but makes adding an engine a
//! compile-time forced choice — *"how does this engine handle each
//! operation?"* — instead of a silent inheritance of the fallback.

use std::str::FromStr;
use std::sync::Once;
use std::time::Instant;

use sqlx::any::{AnyConnectOptions, AnyRow};
use sqlx::{Any, AnyConnection, Column, Connection, Row};

use crate::{
    bytes_to_hex, clickhouse, postgres, CellEdit, CellEditKeyValue, ColumnInfo,
    CommitCellEditsResult, ConnectResult, DatabaseEngine, DatabaseOverviewStats,
    DeleteRowsResult, ExecuteDdlResult, InsertRowResult, MutationStatus, QueryResult,
    SchemaExplorer, SchemaRelationships, StoredConnection, StructureCapabilities,
    TableStructure,
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

/// "This operation does not exist on this engine's class." Reserved
/// for the day Redis (or another non-SQL engine) lands; flagged
/// distinctly so the frontend can choose to hide the affordance rather
/// than render a "not supported" wall.
#[allow(dead_code)]
fn not_applicable(engine: &DatabaseEngine, operation: &str) -> String {
    format!(
        "{operation} does not apply to {} — the concept does not exist on \
         this engine class.",
        engine_name(engine)
    )
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
    match connection.engine {
        DatabaseEngine::PostgreSQL => {
            if connection.host.is_empty() || connection.user.is_empty() {
                return Err("PostgreSQL host and user are required".to_string());
            }
            let port = if connection.port == 0 {
                5432
            } else {
                connection.port
            };
            Ok(format!(
                "postgres://{}:{}@{}:{}/{}",
                connection.user, connection.password, connection.host, port, connection.database
            ))
        }
        DatabaseEngine::MySQL => {
            if connection.host.is_empty() || connection.user.is_empty() {
                return Err("MySQL host and user are required".to_string());
            }
            let port = if connection.port == 0 {
                3306
            } else {
                connection.port
            };
            Ok(format!(
                "mysql://{}:{}@{}:{}/{}",
                connection.user, connection.password, connection.host, port, connection.database
            ))
        }
        DatabaseEngine::SQLite => sqlite_dsn(&connection.database),
        DatabaseEngine::ClickHouse => Err("ClickHouse uses HTTP client".to_string()),
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
        .map_err(|error| error.to_string())?;
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
        .map_err(|error| error.to_string())?;

    match connection.engine {
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
                explorer.push(SchemaExplorer {
                    name: schema,
                    tables,
                    views,
                });
            }
            Ok(explorer)
        }
        DatabaseEngine::MySQL => {
            if connection.database.trim().is_empty() {
                return Err("MySQL database is required".to_string());
            }
            let tables = fetch_column(
                &mut conn,
                "SELECT CAST(table_name AS CHAR) FROM information_schema.tables WHERE table_schema = ? AND table_type = 'BASE TABLE' ORDER BY table_name",
                Some(&connection.database),
            )
            .await?;
            let views = fetch_column(
                &mut conn,
                "SELECT CAST(table_name AS CHAR) FROM information_schema.views WHERE table_schema = ? ORDER BY table_name",
                Some(&connection.database),
            )
            .await?;
            Ok(vec![SchemaExplorer {
                name: connection.database.clone(),
                tables,
                views,
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
            }])
        }
        DatabaseEngine::ClickHouse => {
            // Defensive: caller should have routed CH away before now.
            Err("ClickHouse does not use the sqlx-Any path".to_string())
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
    let qualified = crate::qualified_table_name(&connection.engine, schema, table);
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
            update_semantics: "synchronous".to_string(),
            uniqueness_guarantee: "best-effort".to_string(),
        },
        table_engine: None,
        partition_by: None,
        sample_by: None,
    })
}

// ---------------------------------------------------------------------------
// Public dispatch surface
// ---------------------------------------------------------------------------

/// Connect + `SELECT 1` to verify the connection is live and measure
/// latency. Routes ClickHouse through HTTP; everything else through
/// sqlx-Any connect (PostgreSQL doesn't need its native driver for a
/// liveness check).
pub async fn ping_connection(connection: &StoredConnection) -> Result<ConnectResult, String> {
    match connection.engine {
        DatabaseEngine::ClickHouse => {
            let result = clickhouse::run_query(connection, "SELECT 1").await?;
            Ok(ConnectResult {
                latency_ms: result.runtime_ms,
            })
        }
        DatabaseEngine::PostgreSQL | DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            ensure_sqlx_drivers();
            let dsn = sqlx_dsn(connection)?;
            let start = Instant::now();
            let options = AnyConnectOptions::from_str(&dsn).map_err(|error| error.to_string())?;
            let _connection = AnyConnection::connect_with(&options)
                .await
                .map_err(|error| error.to_string())?;
            Ok(ConnectResult {
                latency_ms: start.elapsed().as_millis() as u64,
            })
        }
    }
}

/// Run an ad-hoc query against the connection's engine. PostgreSQL uses
/// the native driver (for richer type coverage); ClickHouse uses HTTP;
/// MySQL/SQLite use sqlx-Any.
pub async fn run_query(
    connection: &StoredConnection,
    query: &str,
) -> Result<QueryResult, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => postgres::run_query(connection, query).await,
        DatabaseEngine::ClickHouse => clickhouse::run_query(connection, query).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => run_sqlx_any(connection, query).await,
    }
}

pub async fn load_schema_explorer(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    match connection.engine {
        DatabaseEngine::ClickHouse => clickhouse::fetch_schema_explorer(connection).await,
        DatabaseEngine::PostgreSQL | DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            fetch_schema_explorer_sqlx(connection).await
        }
    }
}

pub async fn fetch_table_structure(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => postgres::fetch_table_structure(connection, schema, table).await,
        DatabaseEngine::ClickHouse => clickhouse::fetch_table_structure(connection, schema, table).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            // Columns-only probe with all `capabilities.*` falsy — the UI
            // hides the disabled sections accordingly. Native engine
            // modules will replace this when MySQL / SQLite catch up.
            fetch_table_structure_columns_only(connection, schema, table)
                .await
                .map_err(|error| {
                    format!(
                        "Structure inspection is not yet supported for {}: {}",
                        engine_name(&connection.engine),
                        error
                    )
                })
        }
    }
}

pub async fn fetch_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
) -> Result<SchemaRelationships, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => postgres::fetch_schema_relationships(connection, schema).await,
        DatabaseEngine::ClickHouse => clickhouse::fetch_schema_relationships(connection, schema).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            // Empty rather than error — keeps the relationship-map panel
            // renderable on engines we haven't introspected yet.
            Ok(SchemaRelationships {
                tables: Vec::new(),
                foreign_keys: Vec::new(),
            })
        }
    }
}

pub async fn fetch_database_overview_stats(
    connection: &StoredConnection,
) -> Result<DatabaseOverviewStats, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => postgres::load_database_overview_stats(connection).await,
        DatabaseEngine::ClickHouse => clickhouse::fetch_database_overview_stats(connection).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine, "Database overview stats"))
        }
    }
}

pub async fn execute_ddl(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => postgres::execute_ddl(connection, sql).await,
        DatabaseEngine::ClickHouse => clickhouse::execute_ddl(connection, sql).await,
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine, "DDL execution"))
        }
    }
}

pub async fn commit_cell_edits(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    edits: &[CellEdit],
) -> Result<CommitCellEditsResult, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => {
            postgres::commit_cell_edits(connection, schema, table, edits).await
        }
        DatabaseEngine::ClickHouse => {
            clickhouse::commit_cell_edits(connection, schema, table, edits).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine, "Cell edit commit"))
        }
    }
}

pub async fn insert_row(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    values: &[CellEditKeyValue],
) -> Result<InsertRowResult, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => {
            postgres::insert_row(connection, schema, table, values).await
        }
        DatabaseEngine::ClickHouse => {
            clickhouse::insert_row(connection, schema, table, values).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine, "Row insert"))
        }
    }
}

pub async fn delete_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    rows: &[Vec<CellEditKeyValue>],
) -> Result<DeleteRowsResult, String> {
    match connection.engine {
        DatabaseEngine::PostgreSQL => {
            postgres::delete_rows(connection, schema, table, rows).await
        }
        DatabaseEngine::ClickHouse => {
            clickhouse::delete_rows(connection, schema, table, rows).await
        }
        DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine, "Row delete"))
        }
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
    match connection.engine {
        DatabaseEngine::ClickHouse => {
            clickhouse::poll_mutations(connection, database, table, mutation_ids).await
        }
        DatabaseEngine::PostgreSQL | DatabaseEngine::MySQL | DatabaseEngine::SQLite => {
            Err(not_implemented_yet(&connection.engine, "Mutation polling"))
        }
    }
}
