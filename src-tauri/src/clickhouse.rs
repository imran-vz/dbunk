//! ClickHouse-specific implementations.
//!
//! Unlike the other engines (PostgreSQL/MySQL/SQLite via sqlx), ClickHouse
//! is reached over HTTP using its `JSONCompact` response format. This
//! module owns the URL building, request execution, JSON parsing, and
//! schema-explorer probe used by the dispatchers in `lib.rs`.
//!
//! ## Public surface
//!
//! - [`run_query`] — execute one statement; SELECTs come back with column
//!   names + row strings, DML returns 0 rows. Routes by HTTP POST.
//! - [`fetch_schema_explorer`] — list tables + views in the connection's
//!   active database via `system.tables`.
//!
//! ## Internals
//!
//! - [`url`] composes the connection's HTTP endpoint with `default_format`
//!   and an optional `database` query param.
//! - [`escape`] doubles single quotes for embedding into SQL string
//!   literals (CH catalog queries use string-literal database names rather
//!   than parameter binding).
//! - [`parse_response`] decodes the `JSONCompact` envelope into the shared
//!   `QueryResult` shape.

use std::time::Instant;

use crate::{QueryResult, SchemaExplorer, StoredConnection};

// ---------------------------------------------------------------------------
// URL + escaping helpers
// ---------------------------------------------------------------------------

fn database(connection: &StoredConnection) -> String {
    if connection.database.trim().is_empty() {
        "default".to_string()
    } else {
        connection.database.clone()
    }
}

fn escape(value: &str) -> String {
    value.replace('\'', "''")
}

fn url(connection: &StoredConnection) -> Result<reqwest::Url, String> {
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

// ---------------------------------------------------------------------------
// JSON response parsing
// ---------------------------------------------------------------------------

fn json_value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
    }
}

fn parse_response(payload: serde_json::Value, runtime_ms: u64) -> Result<QueryResult, String> {
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

// ---------------------------------------------------------------------------
// Public surface
// ---------------------------------------------------------------------------

/// Run an ad-hoc query against ClickHouse over HTTP.
///
/// Successful responses come back as `JSONCompact` and are decoded into
/// columns + rows; non-2xx responses surface the body verbatim as the
/// error so the user sees whatever ClickHouse said. Bodies that aren't
/// JSON (DML statements with no result set) collapse to an empty
/// `QueryResult` with `runtime_ms` populated.
pub async fn run_query(
    connection: &StoredConnection,
    query: &str,
) -> Result<QueryResult, String> {
    let url = url(connection)?;
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
        parse_response(payload, runtime_ms)
    } else {
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            runtime_ms,
            row_count: 0,
        })
    }
}

pub async fn fetch_schema_explorer(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    let database = database(connection);
    let escaped = escape(&database);
    let tables_query = format!(
        "SELECT name FROM system.tables WHERE database = '{}' AND engine NOT IN ('View', 'MaterializedView', 'LiveView') ORDER BY name",
        escaped
    );
    let views_query = format!(
        "SELECT name FROM system.tables WHERE database = '{}' AND engine IN ('View', 'MaterializedView', 'LiveView') ORDER BY name",
        escaped
    );
    let tables_result = run_query(connection, &tables_query).await?;
    let views_result = run_query(connection, &views_query).await?;

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

#[cfg(test)]
mod tests {
    //! Unit tests for the URL/escape/parse helpers — every other public
    //! function in this module needs a live ClickHouse to exercise. The
    //! parser is the highest-leverage thing to test in isolation: it
    //! consumes opaque JSON from the network and turns it into the shape
    //! the rest of the app trusts.
    use super::*;
    use crate::DatabaseEngine;

    fn ch_conn(host: &str, database: &str, port: u16) -> StoredConnection {
        StoredConnection {
            id: "ch".into(),
            name: "ch".into(),
            database: database.into(),
            engine: DatabaseEngine::ClickHouse,
            host: host.into(),
            port,
            user: String::new(),
            password: String::new(),
            role: String::new(),
            last_activity_at: None,
        }
    }

    #[test]
    fn database_defaults_to_literal_default_when_blank() {
        let connection = ch_conn("localhost", "", 0);
        assert_eq!(database(&connection), "default");
    }

    #[test]
    fn database_preserves_user_value() {
        let connection = ch_conn("localhost", "analytics", 0);
        assert_eq!(database(&connection), "analytics");
    }

    #[test]
    fn escape_doubles_single_quotes() {
        assert_eq!(escape("plain"), "plain");
        assert_eq!(escape("o'brien"), "o''brien");
        assert_eq!(escape("''"), "''''");
    }

    #[test]
    fn url_uses_8123_when_port_is_zero() {
        let connection = ch_conn("localhost", "", 0);
        let built = url(&connection).expect("url");
        assert_eq!(built.host_str(), Some("localhost"));
        assert_eq!(built.port(), Some(8123));
        assert!(built.query().unwrap().contains("default_format=JSONCompact"));
    }

    #[test]
    fn url_respects_explicit_port() {
        let connection = ch_conn("ch.internal", "", 9000);
        let built = url(&connection).expect("url");
        assert_eq!(built.port(), Some(9000));
    }

    #[test]
    fn url_passes_through_explicit_scheme() {
        // Hosts that already carry a scheme are taken verbatim — port stays
        // whatever the URL specified, not 8123.
        let connection = ch_conn("https://ch.example.com:443", "", 0);
        let built = url(&connection).expect("url");
        assert_eq!(built.scheme(), "https");
        assert_eq!(built.host_str(), Some("ch.example.com"));
    }

    #[test]
    fn url_includes_database_query_param_when_set() {
        let connection = ch_conn("localhost", "analytics", 0);
        let built = url(&connection).expect("url");
        let query = built.query().unwrap_or("");
        assert!(query.contains("database=analytics"));
    }

    #[test]
    fn url_omits_database_param_when_blank() {
        let connection = ch_conn("localhost", "", 0);
        let built = url(&connection).expect("url");
        let query = built.query().unwrap_or("");
        assert!(!query.contains("database="));
    }

    #[test]
    fn parse_response_extracts_columns_rows_and_runtime() {
        let payload = serde_json::json!({
            "meta": [{ "name": "id" }, { "name": "name" }],
            "data": [["1", "Alice"], ["2", "Bob"]],
            "rows": 2
        });
        let result = parse_response(payload, 42).expect("parse");
        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.rows, vec![vec!["1", "Alice"], vec!["2", "Bob"]]);
        assert_eq!(result.row_count, 2);
        assert_eq!(result.runtime_ms, 42);
    }

    #[test]
    fn parse_response_falls_back_to_data_length_when_rows_missing() {
        // Some CH responses elide the top-level `rows` field; we should
        // still report a row count so the caller can show "n rows".
        let payload = serde_json::json!({
            "meta": [{ "name": "x" }],
            "data": [["a"], ["b"], ["c"]]
        });
        let result = parse_response(payload, 0).expect("parse");
        assert_eq!(result.row_count, 3);
    }

    #[test]
    fn parse_response_handles_null_and_nested_values() {
        let payload = serde_json::json!({
            "meta": [{ "name": "x" }, { "name": "y" }, { "name": "z" }],
            "data": [[null, true, [1, 2, 3]]],
            "rows": 1
        });
        let result = parse_response(payload, 0).expect("parse");
        // NULL → "NULL"; bool → "true"; array → JSON-encoded string.
        assert_eq!(result.rows[0][0], "NULL");
        assert_eq!(result.rows[0][1], "true");
        assert_eq!(result.rows[0][2], "[1,2,3]");
    }

    #[test]
    fn parse_response_returns_empty_for_missing_meta_and_data() {
        // Defensive: malformed responses shouldn't panic.
        let payload = serde_json::json!({});
        let result = parse_response(payload, 0).expect("parse");
        assert!(result.columns.is_empty());
        assert!(result.rows.is_empty());
        assert_eq!(result.row_count, 0);
    }
}
