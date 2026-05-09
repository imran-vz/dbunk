use serde::{Deserialize, Serialize};
use sqlx::{
    any::AnyConnectOptions, postgres::PgConnectOptions, Any, AnyConnection, Column, Connection,
    PgConnection, Row,
};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Once,
    time::Instant,
};
use tauri::{path::BaseDirectory, AppHandle, Manager};

#[derive(Debug, Serialize, Deserialize, Clone)]
enum DatabaseEngine {
    #[serde(rename = "PostgreSQL")]
    PostgreSQL,
    #[serde(rename = "MySQL")]
    MySQL,
    #[serde(rename = "ClickHouse")]
    ClickHouse,
    #[serde(rename = "SQLite")]
    SQLite,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct QueryHistoryEntry {
    id: String,
    sql: String,
    connection_id: String,
    connection_name: String,
    database: String,
    engine: DatabaseEngine,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_message: Option<String>,
    runtime_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    row_count: Option<u64>,
    started_at: String,
}

const MAX_QUERY_HISTORY: usize = 200;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StoredConnection {
    id: String,
    name: String,
    database: String,
    engine: DatabaseEngine,
    host: String,
    port: u16,
    user: String,
    password: String,
    role: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryResult {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    runtime_ms: u64,
    row_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectResult {
    latency_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionPayload {
    connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunQueryPayload {
    connection_id: String,
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadTableDataPayload {
    connection_id: String,
    schema: String,
    table: String,
    page: Option<u32>,
    page_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadTableStructurePayload {
    connection_id: String,
    schema: String,
    table: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteDdlPayload {
    connection_id: String,
    sql: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteDdlResult {
    runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ColumnInfo {
    name: String,
    data_type: String,
    nullable: bool,
    default_value: Option<String>,
    is_primary_key: bool,
    ordinal_position: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForeignKeyInfo {
    name: String,
    columns: Vec<String>,
    referenced_schema: String,
    referenced_table: String,
    referenced_columns: Vec<String>,
    on_update: Option<String>,
    on_delete: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexInfo {
    name: String,
    columns: Vec<String>,
    is_unique: bool,
    is_primary: bool,
    method: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConstraintInfo {
    name: String,
    kind: String,
    definition: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StructureCapabilities {
    columns: bool,
    primary_key: bool,
    foreign_keys: bool,
    indexes: bool,
    constraints: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TableStructure {
    columns: Vec<ColumnInfo>,
    primary_key: Option<Vec<String>>,
    foreign_keys: Vec<ForeignKeyInfo>,
    indexes: Vec<IndexInfo>,
    constraints: Vec<ConstraintInfo>,
    capabilities: StructureCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TableDataResult {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    page: u32,
    page_size: u32,
    total_rows: Option<u64>,
    runtime_ms: u64,
}

const DEFAULT_TABLE_PAGE_SIZE: u32 = 100;
const MAX_TABLE_PAGE_SIZE: u32 = 1000;

fn quote_double(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_backtick(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn qualified_table_name(engine: &DatabaseEngine, schema: &str, table: &str) -> String {
    match engine {
        DatabaseEngine::PostgreSQL | DatabaseEngine::SQLite => {
            if schema.is_empty() {
                quote_double(table)
            } else {
                format!("{}.{}", quote_double(schema), quote_double(table))
            }
        }
        DatabaseEngine::MySQL | DatabaseEngine::ClickHouse => {
            if schema.is_empty() {
                quote_backtick(table)
            } else {
                format!("{}.{}", quote_backtick(schema), quote_backtick(table))
            }
        }
    }
}

fn parse_total_rows(result: &QueryResult) -> Option<u64> {
    result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| cell.parse::<u64>().ok())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaExplorer {
    name: String,
    tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    views: Vec<String>,
}

static SQLX_DRIVER_INIT: Once = Once::new();

fn ensure_sqlx_drivers() {
    SQLX_DRIVER_INIT.call_once(|| {
        sqlx::any::install_default_drivers();
    });
}

fn clickhouse_database(connection: &StoredConnection) -> String {
    if connection.database.trim().is_empty() {
        "default".to_string()
    } else {
        connection.database.clone()
    }
}

fn escape_clickhouse(value: &str) -> String {
    value.replace('\'', "''")
}

fn config_directory(app: &AppHandle) -> Result<PathBuf, String> {
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

fn connections_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = config_directory(app)?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.join("connections.json"))
}

fn query_history_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = config_directory(app)?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.join("query_history.json"))
}

fn read_connections(path: &Path) -> Result<Vec<StoredConnection>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&data).map_err(|error| error.to_string())
}

fn write_connections(path: &Path, connections: &[StoredConnection]) -> Result<(), String> {
    let data = serde_json::to_string_pretty(connections).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| error.to_string())
}

fn read_query_history(path: &Path) -> Result<Vec<QueryHistoryEntry>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let data = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if data.trim().is_empty() {
        return Ok(vec![]);
    }
    // Be tolerant of corrupt files: a parse failure should not wipe the
    // user's app state on boot. Log and start fresh.
    match serde_json::from_str::<Vec<QueryHistoryEntry>>(&data) {
        Ok(entries) => Ok(entries),
        Err(error) => {
            eprintln!("query_history.json is unreadable, ignoring: {error}");
            Ok(vec![])
        }
    }
}

fn write_query_history(path: &Path, entries: &[QueryHistoryEntry]) -> Result<(), String> {
    let data = serde_json::to_string_pretty(entries).map_err(|error| error.to_string())?;
    fs::write(path, data).map_err(|error| error.to_string())
}

fn find_connection(app: &AppHandle, connection_id: &str) -> Result<StoredConnection, String> {
    let path = connections_file(app)?;
    let connections = read_connections(&path)?;
    connections
        .into_iter()
        .find(|connection| connection.id == connection_id)
        .ok_or_else(|| "Connection not found".to_string())
}

fn should_fetch_rows(query: &str) -> bool {
    let trimmed = query.trim_start().to_lowercase();
    trimmed.starts_with("select")
        || trimmed.starts_with("with")
        || trimmed.starts_with("show")
        || trimmed.starts_with("describe")
        || trimmed.starts_with("pragma")
        || trimmed.starts_with("explain")
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2 + 2);
    output.push_str("0x");
    for byte in bytes {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}

fn value_to_string(row: &sqlx::any::AnyRow, index: usize) -> String {
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
            // Try to decode as text representation
            if let Ok(s) = <&str as sqlx::Decode<sqlx::any::Any>>::decode(raw_value) {
                // Handle boolean special cases
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

fn row_to_strings(row: &sqlx::any::AnyRow) -> Vec<String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, _)| value_to_string(row, index))
        .collect()
}

fn pg_value_to_string(row: &sqlx::postgres::PgRow, index: usize) -> String {
    // Try string first
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value.unwrap_or_else(|| "NULL".to_string());
    }
    // Integers
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i32>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<i16>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    // Floats
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<f32>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    // Boolean
    if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    // Timestamps and dates
    if let Ok(value) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(index) {
        return value
            .map(|v| v.to_rfc3339())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveDateTime>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveDate>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    if let Ok(value) = row.try_get::<Option<chrono::NaiveTime>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    // UUID
    if let Ok(value) = row.try_get::<Option<uuid::Uuid>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    // JSON
    if let Ok(value) = row.try_get::<Option<serde_json::Value>, _>(index) {
        return value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "NULL".to_string());
    }
    // Bytes
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(|bytes| bytes_to_hex(&bytes))
            .unwrap_or_else(|| "NULL".to_string());
    }
    "NULL".to_string()
}

fn pg_row_to_strings(row: &sqlx::postgres::PgRow) -> Vec<String> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, _)| pg_value_to_string(row, index))
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

fn clickhouse_url(connection: &StoredConnection) -> Result<reqwest::Url, String> {
    let base = if connection.host.starts_with("http://") || connection.host.starts_with("https://")
    {
        connection.host.clone()
    } else {
        let port = if connection.port == 0 {
            8123
        } else {
            connection.port
        };
        format!("http://{}:{}", connection.host, port)
    };
    let mut url = reqwest::Url::parse(&base).map_err(|error| error.to_string())?;
    url.set_path("/");
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("default_format", "JSONCompact");
        if !connection.database.is_empty() {
            pairs.append_pair("database", &connection.database);
        }
    }
    Ok(url)
}

fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
    }
}

fn parse_clickhouse_json(
    payload: serde_json::Value,
    runtime_ms: u64,
) -> Result<QueryResult, String> {
    let columns = payload
        .get("meta")
        .and_then(|value| value.as_array())
        .map(|meta| {
            meta.iter()
                .filter_map(|entry| entry.get("name").and_then(|name| name.as_str()))
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let rows = payload
        .get("data")
        .and_then(|value| value.as_array())
        .map(|data| {
            data.iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| cells.iter().map(json_value_to_string).collect::<Vec<_>>())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let row_count = payload
        .get("rows")
        .and_then(|value| value.as_u64())
        .unwrap_or(rows.len() as u64);

    Ok(QueryResult {
        columns,
        rows,
        runtime_ms,
        row_count,
    })
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
                let name = row.get(0).cloned().unwrap_or_default();
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
        DatabaseEngine::ClickHouse => Err("ClickHouse uses HTTP client".to_string()),
    }
}

async fn fetch_schema_explorer_clickhouse(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    let database = clickhouse_database(connection);
    let escaped = escape_clickhouse(&database);
    let tables_query = format!(
        "SELECT name FROM system.tables WHERE database = '{}' AND engine NOT IN ('View', 'MaterializedView', 'LiveView') ORDER BY name",
        escaped
    );
    let views_query = format!(
        "SELECT name FROM system.tables WHERE database = '{}' AND engine IN ('View', 'MaterializedView', 'LiveView') ORDER BY name",
        escaped
    );
    let tables_result = run_clickhouse_query(connection, &tables_query).await?;
    let views_result = run_clickhouse_query(connection, &views_query).await?;

    let tables = tables_result
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .collect::<Vec<_>>();
    let views = views_result
        .rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .collect::<Vec<_>>();

    Ok(vec![SchemaExplorer {
        name: database,
        tables,
        views,
    }])
}

async fn run_postgres_query(
    connection: &StoredConnection,
    query: &str,
) -> Result<QueryResult, String> {
    let mut options = PgConnectOptions::new()
        .host(&connection.host)
        .username(&connection.user)
        .database(&connection.database);

    if connection.port != 0 {
        options = options.port(connection.port);
    } else {
        options = options.port(5432);
    }

    if !connection.password.is_empty() {
        options = options.password(&connection.password);
    }

    let mut conn = PgConnection::connect_with(&options)
        .await
        .map_err(|error| error.to_string())?;
    let start = Instant::now();

    if should_fetch_rows(query) {
        let rows = sqlx::query(query)
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
        let values = rows.iter().map(pg_row_to_strings).collect::<Vec<_>>();
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns,
            rows: values,
            runtime_ms,
            row_count: rows.len() as u64,
        })
    } else {
        let result = sqlx::query(query)
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

async fn run_sqlx_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    // Use native PostgreSQL driver to properly support all Postgres types
    if matches!(connection.engine, DatabaseEngine::PostgreSQL) {
        return run_postgres_query(connection, query).await;
    }

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

async fn run_clickhouse_query(
    connection: &StoredConnection,
    query: &str,
) -> Result<QueryResult, String> {
    let url = clickhouse_url(connection)?;
    let client = reqwest::Client::new();
    let start = Instant::now();
    let mut request = client.post(url).body(query.to_string());
    if !connection.user.is_empty() {
        request = request.basic_auth(connection.user.clone(), Some(connection.password.clone()));
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(text);
    }
    let runtime_ms = start.elapsed().as_millis() as u64;
    if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&text) {
        parse_clickhouse_json(payload, runtime_ms)
    } else {
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            runtime_ms,
            row_count: 0,
        })
    }
}

#[tauri::command]
async fn load_connections(app: AppHandle) -> Result<Vec<StoredConnection>, String> {
    let path = connections_file(&app)?;
    read_connections(&path)
}

#[tauri::command]
async fn save_connection(
    app: AppHandle,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    let path = connections_file(&app)?;
    let mut connections = read_connections(&path)?;

    if let Some(existing) = connections.iter_mut().find(|item| item.id == connection.id) {
        *existing = connection.clone();
    } else {
        connections.push(connection.clone());
    }

    write_connections(&path, &connections)?;
    Ok(connections)
}

#[tauri::command]
async fn delete_connection(
    app: AppHandle,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    let path = connections_file(&app)?;
    let mut connections = read_connections(&path)?;

    let initial_len = connections.len();
    connections.retain(|item| item.id != payload.connection_id);

    if connections.len() == initial_len {
        return Err(format!("Connection '{}' not found", payload.connection_id));
    }

    write_connections(&path, &connections)?;
    Ok(connections)
}

#[tauri::command]
async fn connect_connection(
    app: AppHandle,
    payload: ConnectionPayload,
) -> Result<ConnectResult, String> {
    let connection = find_connection(&app, &payload.connection_id)?;
    match connection.engine {
        DatabaseEngine::ClickHouse => {
            let result = run_clickhouse_query(&connection, "SELECT 1").await?;
            Ok(ConnectResult {
                latency_ms: result.runtime_ms,
            })
        }
        _ => {
            ensure_sqlx_drivers();
            let dsn = sqlx_dsn(&connection)?;
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

#[tauri::command]
async fn load_schema_explorer(
    app: AppHandle,
    payload: ConnectionPayload,
) -> Result<Vec<SchemaExplorer>, String> {
    let connection = find_connection(&app, &payload.connection_id)?;
    match connection.engine {
        DatabaseEngine::ClickHouse => fetch_schema_explorer_clickhouse(&connection).await,
        _ => fetch_schema_explorer_sqlx(&connection).await,
    }
}

#[tauri::command]
async fn run_query(app: AppHandle, payload: RunQueryPayload) -> Result<QueryResult, String> {
    let connection = find_connection(&app, &payload.connection_id)?;
    match connection.engine {
        DatabaseEngine::ClickHouse => run_clickhouse_query(&connection, &payload.query).await,
        _ => run_sqlx_query(&connection, &payload.query).await,
    }
}

async fn run_engine_query(
    connection: &StoredConnection,
    query: &str,
) -> Result<QueryResult, String> {
    match connection.engine {
        DatabaseEngine::ClickHouse => run_clickhouse_query(connection, query).await,
        _ => run_sqlx_query(connection, query).await,
    }
}

async fn fetch_table_structure_postgres(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    let mut options = PgConnectOptions::new()
        .host(&connection.host)
        .username(&connection.user)
        .database(&connection.database);

    if connection.port != 0 {
        options = options.port(connection.port);
    } else {
        options = options.port(5432);
    }

    if !connection.password.is_empty() {
        options = options.password(&connection.password);
    }

    let mut conn = PgConnection::connect_with(&options)
        .await
        .map_err(|error| error.to_string())?;

    // Primary key columns first so we can mark them on the columns list.
    let pk_rows = sqlx::query(
        r#"
        SELECT kcu.column_name::text AS column_name
        FROM information_schema.table_constraints tc
        JOIN information_schema.key_column_usage kcu
          ON tc.constraint_name = kcu.constraint_name
         AND tc.table_schema = kcu.table_schema
         AND tc.table_name = kcu.table_name
        WHERE tc.constraint_type = 'PRIMARY KEY'
          AND tc.table_schema = $1
          AND tc.table_name = $2
        ORDER BY kcu.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let primary_key_cols: Vec<String> = pk_rows
        .iter()
        .map(|row| row.try_get::<String, _>("column_name").unwrap_or_default())
        .collect();

    // Columns
    let column_rows = sqlx::query(
        r#"
        SELECT column_name::text AS column_name,
               data_type::text AS data_type,
               udt_name::text AS udt_name,
               is_nullable::text AS is_nullable,
               column_default::text AS column_default,
               ordinal_position::int AS ordinal_position,
               character_maximum_length::int AS character_maximum_length,
               numeric_precision::int AS numeric_precision,
               numeric_scale::int AS numeric_scale
        FROM information_schema.columns
        WHERE table_schema = $1 AND table_name = $2
        ORDER BY ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let mut columns = Vec::with_capacity(column_rows.len());
    for row in column_rows {
        let name: String = row.try_get("column_name").unwrap_or_default();
        let data_type: String = row.try_get("data_type").unwrap_or_default();
        let udt_name: String = row.try_get("udt_name").unwrap_or_default();
        let is_nullable: String = row.try_get("is_nullable").unwrap_or_default();
        let default_value: Option<String> = row.try_get("column_default").ok();
        let ordinal_position: i32 = row.try_get("ordinal_position").unwrap_or(0);
        let char_len: Option<i32> = row.try_get("character_maximum_length").ok();
        let numeric_precision: Option<i32> = row.try_get("numeric_precision").ok();
        let numeric_scale: Option<i32> = row.try_get("numeric_scale").ok();

        // Build a richer rendered type. Trust `data_type` from information_schema
        // for the high-level family and fall back to `udt_name` for specifics.
        let rendered_type = match data_type.as_str() {
            "character varying" => match char_len {
                Some(len) => format!("varchar({})", len),
                None => "varchar".to_string(),
            },
            "character" => match char_len {
                Some(len) => format!("char({})", len),
                None => "char".to_string(),
            },
            "numeric" => match (numeric_precision, numeric_scale) {
                (Some(p), Some(s)) if s > 0 => format!("numeric({},{})", p, s),
                (Some(p), _) => format!("numeric({})", p),
                _ => "numeric".to_string(),
            },
            "USER-DEFINED" | "ARRAY" => udt_name.clone(),
            other => other.to_string(),
        };

        columns.push(ColumnInfo {
            name: name.clone(),
            data_type: rendered_type,
            nullable: is_nullable.eq_ignore_ascii_case("YES"),
            default_value,
            is_primary_key: primary_key_cols.iter().any(|pk| pk == &name),
            ordinal_position,
        });
    }

    // Foreign keys
    let fk_rows = sqlx::query(
        r#"
        SELECT con.conname::text AS name,
               nsp_ref.nspname::text AS referenced_schema,
               cls_ref.relname::text AS referenced_table,
               con.confupdtype::text AS on_update,
               con.confdeltype::text AS on_delete,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.conkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.conrelid AND att.attnum = u.attnum
               ) AS columns,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(con.confkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = con.confrelid AND att.attnum = u.attnum
               ) AS referenced_columns
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        JOIN pg_class cls_ref ON cls_ref.oid = con.confrelid
        JOIN pg_namespace nsp_ref ON nsp_ref.oid = cls_ref.relnamespace
        WHERE con.contype = 'f'
          AND nsp.nspname = $1
          AND cls.relname = $2
        ORDER BY con.conname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let foreign_keys = fk_rows
        .into_iter()
        .map(|row| {
            let name: String = row.try_get("name").unwrap_or_default();
            let referenced_schema: String = row.try_get("referenced_schema").unwrap_or_default();
            let referenced_table: String = row.try_get("referenced_table").unwrap_or_default();
            let on_update_code: String = row.try_get("on_update").unwrap_or_default();
            let on_delete_code: String = row.try_get("on_delete").unwrap_or_default();
            let columns: Vec<String> = row.try_get("columns").unwrap_or_default();
            let referenced_columns: Vec<String> =
                row.try_get("referenced_columns").unwrap_or_default();
            ForeignKeyInfo {
                name,
                columns,
                referenced_schema,
                referenced_table,
                referenced_columns,
                on_update: pg_fk_action_label(&on_update_code),
                on_delete: pg_fk_action_label(&on_delete_code),
            }
        })
        .collect::<Vec<_>>();

    // Indexes
    let index_rows = sqlx::query(
        r#"
        SELECT i.relname::text AS index_name,
               ix.indisunique AS is_unique,
               ix.indisprimary AS is_primary,
               am.amname::text AS method,
               (
                   SELECT array_agg(att.attname::text ORDER BY u.ord)
                   FROM unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord)
                   JOIN pg_attribute att
                     ON att.attrelid = ix.indrelid AND att.attnum = u.attnum
               ) AS columns
        FROM pg_class t
        JOIN pg_index ix ON t.oid = ix.indrelid
        JOIN pg_class i ON i.oid = ix.indexrelid
        JOIN pg_am am ON am.oid = i.relam
        JOIN pg_namespace n ON n.oid = t.relnamespace
        WHERE n.nspname = $1 AND t.relname = $2
        ORDER BY i.relname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let indexes = index_rows
        .into_iter()
        .map(|row| IndexInfo {
            name: row.try_get("index_name").unwrap_or_default(),
            columns: row.try_get("columns").unwrap_or_default(),
            is_unique: row.try_get("is_unique").unwrap_or(false),
            is_primary: row.try_get("is_primary").unwrap_or(false),
            method: row.try_get::<String, _>("method").ok(),
        })
        .collect::<Vec<_>>();

    // Other constraints (CHECK, UNIQUE, EXCLUDE) — skip primary key and foreign key here.
    let constraint_rows = sqlx::query(
        r#"
        SELECT con.conname::text AS name,
               con.contype::text AS contype,
               pg_get_constraintdef(con.oid) AS definition
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = cls.relnamespace
        WHERE nsp.nspname = $1
          AND cls.relname = $2
          AND con.contype IN ('c', 'u', 'x')
        ORDER BY con.conname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(&mut conn)
    .await
    .map_err(|error| error.to_string())?;

    let constraints = constraint_rows
        .into_iter()
        .map(|row| {
            let kind_code: String = row.try_get("contype").unwrap_or_default();
            ConstraintInfo {
                name: row.try_get("name").unwrap_or_default(),
                kind: pg_constraint_kind(&kind_code).to_string(),
                definition: row.try_get("definition").unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();

    let primary_key = if primary_key_cols.is_empty() {
        None
    } else {
        Some(primary_key_cols)
    };

    Ok(TableStructure {
        columns,
        primary_key,
        foreign_keys,
        indexes,
        constraints,
        capabilities: StructureCapabilities {
            columns: true,
            primary_key: true,
            foreign_keys: true,
            indexes: true,
            constraints: true,
        },
    })
}

fn pg_fk_action_label(code: &str) -> Option<String> {
    match code {
        "a" => Some("NO ACTION".to_string()),
        "r" => Some("RESTRICT".to_string()),
        "c" => Some("CASCADE".to_string()),
        "n" => Some("SET NULL".to_string()),
        "d" => Some("SET DEFAULT".to_string()),
        _ => None,
    }
}

fn pg_constraint_kind(code: &str) -> &'static str {
    match code {
        "c" => "check",
        "u" => "unique",
        "x" => "exclusion",
        "p" => "primary key",
        "f" => "foreign key",
        _ => "constraint",
    }
}

fn engine_name(engine: &DatabaseEngine) -> &'static str {
    match engine {
        DatabaseEngine::PostgreSQL => "PostgreSQL",
        DatabaseEngine::MySQL => "MySQL",
        DatabaseEngine::ClickHouse => "ClickHouse",
        DatabaseEngine::SQLite => "SQLite",
    }
}

async fn fetch_table_structure_columns_only(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    // Best-effort columns-only fallback for engines we have not yet
    // implemented full structure inspection for. We use a LIMIT 0
    // query to learn the column names; we cannot derive types,
    // nullability, defaults, or PK without engine-specific catalog
    // queries, so we expose those as unsupported via capabilities.
    let qualified = qualified_table_name(&connection.engine, schema, table);
    let probe = format!("SELECT * FROM {} LIMIT 0", qualified);
    let result = run_engine_query(connection, &probe).await?;

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
        },
    })
}

#[tauri::command]
async fn load_table_structure(
    app: AppHandle,
    payload: LoadTableStructurePayload,
) -> Result<TableStructure, String> {
    let connection = find_connection(&app, &payload.connection_id)?;
    match connection.engine {
        DatabaseEngine::PostgreSQL => {
            fetch_table_structure_postgres(&connection, &payload.schema, &payload.table).await
        }
        // For now, MySQL / SQLite / ClickHouse fall back to a columns-only
        // probe; the UI surfaces the disabled sections via capabilities.
        // Issue #2 ships PostgreSQL as the must-have engine; richer
        // metadata for the others is a deliberate follow-up.
        _ => {
            // The fallback discovers columns via a `LIMIT 0` probe;
            // surface a clear message if the engine cannot even do that.
            fetch_table_structure_columns_only(&connection, &payload.schema, &payload.table)
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

async fn execute_ddl_postgres(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    let mut options = PgConnectOptions::new()
        .host(&connection.host)
        .username(&connection.user)
        .database(&connection.database);

    if connection.port != 0 {
        options = options.port(connection.port);
    } else {
        options = options.port(5432);
    }

    if !connection.password.is_empty() {
        options = options.password(&connection.password);
    }

    let mut conn = PgConnection::connect_with(&options)
        .await
        .map_err(|error| error.to_string())?;

    let start = Instant::now();

    // Wrap the entire DDL batch in an explicit transaction so a partial
    // failure leaves the schema unchanged. sqlx's PgConnection::execute
    // accepts multi-statement strings, which lets us send the generated
    // DDL as a single round-trip.
    use sqlx::Executor;
    if let Err(error) = conn.execute("BEGIN").await {
        return Err(error.to_string());
    }
    match conn.execute(sql).await {
        Ok(_) => {
            if let Err(error) = conn.execute("COMMIT").await {
                // Best-effort rollback if COMMIT fails (rare).
                let _ = conn.execute("ROLLBACK").await;
                return Err(error.to_string());
            }
            Ok(ExecuteDdlResult {
                runtime_ms: start.elapsed().as_millis() as u64,
            })
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK").await;
            Err(error.to_string())
        }
    }
}

#[tauri::command]
async fn execute_ddl(
    app: AppHandle,
    payload: ExecuteDdlPayload,
) -> Result<ExecuteDdlResult, String> {
    if payload.sql.trim().is_empty() {
        return Err("DDL statement is empty".to_string());
    }
    let connection = find_connection(&app, &payload.connection_id)?;
    match connection.engine {
        DatabaseEngine::PostgreSQL => execute_ddl_postgres(&connection, &payload.sql).await,
        _ => Err(format!(
            "Structure commit is not supported for {} (PostgreSQL only)",
            engine_name(&connection.engine)
        )),
    }
}

#[tauri::command]
async fn load_table_data(
    app: AppHandle,
    payload: LoadTableDataPayload,
) -> Result<TableDataResult, String> {
    let connection = find_connection(&app, &payload.connection_id)?;
    let page = payload.page.unwrap_or(1).max(1);
    let page_size = payload
        .page_size
        .unwrap_or(DEFAULT_TABLE_PAGE_SIZE)
        .clamp(1, MAX_TABLE_PAGE_SIZE);
    let offset = (page - 1) as u64 * page_size as u64;

    let qualified = qualified_table_name(&connection.engine, &payload.schema, &payload.table);

    // SELECT with LIMIT/OFFSET works for all four supported engines
    // (PostgreSQL, MySQL, SQLite, ClickHouse).
    let select_query = format!(
        "SELECT * FROM {} LIMIT {} OFFSET {}",
        qualified, page_size, offset
    );

    let select_result = run_engine_query(&connection, &select_query).await?;

    // Best-effort COUNT(*) — never fail the call if the count fails.
    let count_query = format!("SELECT COUNT(*) FROM {}", qualified);
    let total_rows = match run_engine_query(&connection, &count_query).await {
        Ok(result) => parse_total_rows(&result),
        Err(_) => None,
    };

    Ok(TableDataResult {
        columns: select_result.columns,
        rows: select_result.rows,
        page,
        page_size,
        total_rows,
        runtime_ms: select_result.runtime_ms,
    })
}

#[tauri::command]
async fn load_query_history(
    app: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let path = query_history_file(&app)?;
    let mut entries = read_query_history(&path)?;
    if let Some(limit) = limit {
        entries.truncate(limit as usize);
    }
    Ok(entries)
}

#[tauri::command]
async fn append_query_history(
    app: AppHandle,
    entry: QueryHistoryEntry,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let path = query_history_file(&app)?;
    let mut entries = read_query_history(&path)?;
    // Newest first; cap to MAX_QUERY_HISTORY.
    entries.insert(0, entry);
    if entries.len() > MAX_QUERY_HISTORY {
        entries.truncate(MAX_QUERY_HISTORY);
    }
    write_query_history(&path, &entries)?;
    Ok(entries)
}

#[tauri::command]
async fn clear_query_history(app: AppHandle) -> Result<(), String> {
    let path = query_history_file(&app)?;
    write_query_history(&path, &[])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    ensure_sqlx_drivers();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            load_connections,
            save_connection,
            delete_connection,
            connect_connection,
            load_schema_explorer,
            run_query,
            load_table_data,
            load_table_structure,
            execute_ddl,
            load_query_history,
            append_query_history,
            clear_query_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
