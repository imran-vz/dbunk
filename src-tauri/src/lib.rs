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
            load_table_data
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
