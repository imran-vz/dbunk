//! ClickHouse-specific implementations.
//!
//! Unlike the other engines (PostgreSQL/MySQL/SQLite via sqlx), ClickHouse
//! is reached over HTTP using its `JSONCompact` response format. This
//! module owns the URL building, request execution, JSON parsing, and
//! schema-explorer + structure + overview probes used by the dispatchers
//! in `lib.rs`.
//!
//! ## Public surface
//!
//! - [`run_query`] — execute one statement; SELECTs come back with column
//!   names + row strings, DML returns 0 rows. Routes by HTTP POST.
//! - [`fetch_schema_explorer`] — list tables + views in the connection's
//!   active database via `system.tables`.
//! - [`fetch_table_structure`] — full structure (columns, sorting key as
//!   PK analogue, skip indices, constraints) from `system.*`.
//! - [`fetch_database_overview_stats`] — aggregate sizes + counts from
//!   `system.parts` + `system.data_skipping_indices` + `system.processes`.
//! - [`fetch_schema_relationships`] — list tables + columns; foreign keys
//!   are always empty (CH has no FKs by design).
//!
//! ## Internals
//!
//! - [`url`] composes the connection's HTTP endpoint with `default_format`
//!   and an optional `database` query param. Honours `useHttps` and
//!   `urlPath` from the stored connection.
//! - [`escape`] doubles single quotes for embedding into SQL string
//!   literals (CH catalog queries use string-literal database names rather
//!   than parameter binding).
//! - [`parse_response`] decodes the `JSONCompact` envelope into the shared
//!   `QueryResult` shape.
//! - [`shared_client`] caches a `reqwest::Client` per process so TLS
//!   handshakes amortize across the schema-explorer fan-out.

use std::sync::OnceLock;
use std::time::Instant;

use crate::{
    quote_backtick, CellEdit, CellEditKeyValue, ClickHouseStoredConnection, ColumnInfo,
    CommitCellEditsResult, ConstraintInfo, DatabaseOverviewStats, DeleteRowsResult,
    ExecuteDdlResult, IndexInfo, InsertRowResult, MutationStatus, QueryResult, SchemaExplorer,
    SchemaForeignKey, SchemaRelationships, SchemaTableColumn, SchemaTableNode, StoredConnection,
    StructureCapabilities, TableStructure,
};

// ---------------------------------------------------------------------------
// Shared HTTP client
// ---------------------------------------------------------------------------

/// Single `reqwest::Client` cached for the lifetime of the process. The
/// client owns a connection pool and reusable TLS session cache, so the
/// schema-explorer fan-out (which fires several queries back-to-back)
/// avoids re-handshaking for every call.
fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

// ---------------------------------------------------------------------------
// URL + escaping helpers
// ---------------------------------------------------------------------------

/// Narrow a `StoredConnection` to its ClickHouse variant. The
/// dispatch layer (`dispatch/relational.rs`) guarantees this module
/// is only reached for CH connections; the helper localizes the
/// contract so URL/database builders can use typed field access.
fn as_ch(connection: &StoredConnection) -> Result<&ClickHouseStoredConnection, String> {
    match connection {
        StoredConnection::ClickHouse(ch) => Ok(ch),
        _ => Err(
            "clickhouse module reached with a non-ClickHouse connection — dispatch bug".to_string(),
        ),
    }
}

fn database(connection: &StoredConnection) -> Result<String, String> {
    let ch = as_ch(connection)?;
    Ok(if ch.database.trim().is_empty() {
        "default".to_string()
    } else {
        ch.database.clone()
    })
}

fn escape(value: &str) -> String {
    value.replace('\'', "''")
}

fn url(connection: &StoredConnection) -> Result<reqwest::Url, String> {
    let ch = as_ch(connection)?;
    // If the user pasted a fully-qualified URL into `host`, honour it.
    // Otherwise compose `<scheme>://<host>:<port>` from the discrete
    // fields. `useHttps` and the explicit port both map to the canonical
    // ClickHouse HTTP endpoints (8123 plain, 8443 TLS).
    let base = if ch.host.starts_with("http://") || ch.host.starts_with("https://") {
        ch.host.clone()
    } else {
        let scheme = if ch.use_https { "https" } else { "http" };
        let port = if ch.port == 0 {
            if ch.use_https {
                8443
            } else {
                8123
            }
        } else {
            ch.port
        };
        format!("{}://{}:{}", scheme, ch.host, port)
    };
    let mut url = reqwest::Url::parse(&base).map_err(|error| error.to_string())?;
    let path = if ch.url_path.trim().is_empty() {
        "/".to_string()
    } else if ch.url_path.starts_with('/') {
        ch.url_path.clone()
    } else {
        format!("/{}", ch.url_path)
    };
    url.set_path(&path);
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("default_format", "JSONCompact");
        if !ch.database.is_empty() {
            pairs.append_pair("database", &ch.database);
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
// Public surface — query execution
// ---------------------------------------------------------------------------

/// Run an ad-hoc query against ClickHouse over HTTP.
///
/// Successful responses come back as `JSONCompact` and are decoded into
/// columns + rows; non-2xx responses surface the body verbatim as the
/// error so the user sees whatever ClickHouse said. Bodies that aren't
/// JSON (DML statements with no result set) collapse to an empty
/// `QueryResult` with `runtime_ms` populated.
pub async fn run_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    let ch = as_ch(connection)?;
    let url = url(connection)?;
    let client = shared_client();
    let start = Instant::now();
    let mut request = client.post(url).body(query.to_string());
    if !ch.user.is_empty() {
        request = request.basic_auth(ch.user.clone(), Some(ch.password.clone()));
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

// ---------------------------------------------------------------------------
// Schema explorer
// ---------------------------------------------------------------------------

pub async fn fetch_schema_explorer(
    connection: &StoredConnection,
) -> Result<Vec<SchemaExplorer>, String> {
    let database = database(connection)?;
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

// ---------------------------------------------------------------------------
// Helpers — column-by-name access on a string-only QueryResult
// ---------------------------------------------------------------------------

fn column_index(result: &QueryResult, name: &str) -> Option<usize> {
    result.columns.iter().position(|column| column == name)
}

fn cell(row: &[String], index: Option<usize>) -> Option<&str> {
    index.and_then(|i| row.get(i)).map(|value| value.as_str())
}

fn cell_owned(row: &[String], index: Option<usize>) -> String {
    cell(row, index).unwrap_or("").to_string()
}

fn parse_csv_list(raw: &str) -> Vec<String> {
    // CH reports composite values like sorting_key as a comma-separated
    // string ("col1, col2, col3"). Tolerate optional whitespace.
    raw.split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_int(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Table structure
// ---------------------------------------------------------------------------

/// Describe a single ClickHouse table.
///
/// The `primary_key` slot carries the **sorting key** column list — CH's
/// closest analogue to PG's primary key. Sorting keys do not enforce
/// uniqueness; the `capabilities.primary_key = true` flag tells the UI
/// the field is populated, and the UI is responsible for relabeling it
/// "Sorting key" when the engine is ClickHouse.
///
/// Indexes come from `system.data_skipping_indices` (minmax / set /
/// bloom_filter / ngrambf — these are CH's secondary indices, not
/// uniqueness indexes).
pub async fn fetch_table_structure(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<TableStructure, String> {
    let escaped_schema = escape(schema);
    let escaped_table = escape(table);

    // Sorting key (PK analogue), partition key, sample key, and engine
    // — read from system.tables in one round trip.
    let table_meta_sql = format!(
        "SELECT sorting_key, partition_key, sampling_key, engine \
         FROM system.tables \
         WHERE database = '{}' AND name = '{}'",
        escaped_schema, escaped_table
    );
    let table_meta = run_query(connection, &table_meta_sql).await?;
    let sorting_key_idx = column_index(&table_meta, "sorting_key");
    let partition_key_idx = column_index(&table_meta, "partition_key");
    let sampling_key_idx = column_index(&table_meta, "sampling_key");
    let engine_idx = column_index(&table_meta, "engine");
    let table_meta_row = table_meta.rows.first();
    let sorting_key_cols: Vec<String> = table_meta_row
        .map(|row| parse_csv_list(cell(row, sorting_key_idx).unwrap_or("")))
        .unwrap_or_default();
    let partition_by = table_meta_row
        .and_then(|row| cell(row, partition_key_idx))
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty());
    let sample_by = table_meta_row
        .and_then(|row| cell(row, sampling_key_idx))
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty());
    let table_engine_name = table_meta_row
        .and_then(|row| cell(row, engine_idx))
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty());

    // Columns — system.columns has everything we need.
    let columns_sql = format!(
        "SELECT name, type, default_kind, default_expression, position, is_in_sorting_key \
         FROM system.columns \
         WHERE database = '{}' AND table = '{}' \
         ORDER BY position",
        escaped_schema, escaped_table
    );
    let columns_result = run_query(connection, &columns_sql).await?;
    let name_idx = column_index(&columns_result, "name");
    let type_idx = column_index(&columns_result, "type");
    let default_kind_idx = column_index(&columns_result, "default_kind");
    let default_expr_idx = column_index(&columns_result, "default_expression");
    let position_idx = column_index(&columns_result, "position");
    let in_sorting_idx = column_index(&columns_result, "is_in_sorting_key");

    let columns: Vec<ColumnInfo> = columns_result
        .rows
        .iter()
        .map(|row| {
            let column_type = cell(row, type_idx).unwrap_or("").to_string();
            let nullable = column_type.starts_with("Nullable(");
            let default_kind = cell(row, default_kind_idx).unwrap_or("");
            let default_expression = cell(row, default_expr_idx).unwrap_or("");
            let default_value = if default_expression.is_empty() {
                None
            } else if default_kind.is_empty() {
                Some(default_expression.to_string())
            } else {
                Some(format!("{} {}", default_kind, default_expression))
            };
            // `default_kind` is `""` for plain `DEFAULT` and uppercase
            // for the derived variants. Empty stays None on the wire.
            let derivation_kind = match default_kind {
                "" | "DEFAULT" => None,
                other => Some(other.to_string()),
            };
            let in_sorting = matches!(cell(row, in_sorting_idx), Some("1") | Some("true"));
            ColumnInfo {
                name: cell_owned(row, name_idx),
                data_type: column_type,
                nullable,
                default_value,
                is_primary_key: in_sorting,
                ordinal_position: parse_int(cell(row, position_idx).unwrap_or("0")) as i32,
                derivation_kind,
            }
        })
        .collect();

    // Skip indices.
    let indices_sql = format!(
        "SELECT name, expr, type FROM system.data_skipping_indices \
         WHERE database = '{}' AND table = '{}' \
         ORDER BY name",
        escaped_schema, escaped_table
    );
    let indices_result = run_query(connection, &indices_sql).await?;
    let idx_name = column_index(&indices_result, "name");
    let idx_expr = column_index(&indices_result, "expr");
    let idx_type = column_index(&indices_result, "type");
    let indexes: Vec<IndexInfo> = indices_result
        .rows
        .iter()
        .map(|row| IndexInfo {
            name: cell_owned(row, idx_name),
            // The expression IS the indexed payload; render it as the
            // "columns" display since CH doesn't decompose it.
            columns: vec![cell_owned(row, idx_expr)],
            is_unique: false,
            is_primary: false,
            method: cell(row, idx_type).map(|value| value.to_string()),
        })
        .collect();

    // CHECK constraints — only present on more recent CH versions, so
    // best-effort: a missing column makes the request error and we
    // suppress that.
    let constraints = fetch_constraints(connection, &escaped_schema, &escaped_table)
        .await
        .unwrap_or_default();

    let primary_key = if sorting_key_cols.is_empty() {
        None
    } else {
        Some(sorting_key_cols)
    };

    // Mutation capabilities depend on the table engine: only the
    // MergeTree family accepts ALTER … UPDATE/DELETE. Distributed,
    // View, Kafka, etc. surface as read-only with the appropriate
    // capability flags.
    let is_mergetree_family = table_engine_name
        .as_deref()
        .map(|name| name.contains("MergeTree"))
        .unwrap_or(false);

    Ok(TableStructure {
        columns,
        primary_key,
        foreign_keys: Vec::new(),
        indexes,
        constraints,
        capabilities: StructureCapabilities {
            columns: true,
            primary_key: true,
            // CH has no foreign keys by design — this is a permanent
            // capability of the engine, not a dbunk gap.
            foreign_keys: false,
            indexes: true,
            constraints: true,
            // Inserts work on most engines that accept writes; we gate
            // generously on MergeTree-family tables.
            can_insert_rows: is_mergetree_family,
            can_update_rows: is_mergetree_family,
            can_delete_rows: is_mergetree_family,
            can_alter_schema: is_mergetree_family,
            uniqueness_guarantee: "best-effort".to_string(),
        },
        table_engine: table_engine_name,
        partition_by,
        sample_by,
    })
}

async fn fetch_constraints(
    connection: &StoredConnection,
    escaped_schema: &str,
    escaped_table: &str,
) -> Result<Vec<ConstraintInfo>, String> {
    // `system.tables` exposes a `constraints` Map(String, String) on
    // versions that support CHECK constraints. If the column is missing
    // (older CH) the query 500s and we return an empty list — the caller
    // already handles the error case as "no constraints".
    let sql = format!(
        "SELECT constraints FROM system.tables WHERE database = '{}' AND name = '{}'",
        escaped_schema, escaped_table
    );
    let result = run_query(connection, &sql).await?;
    let idx = column_index(&result, "constraints");
    let raw = result
        .rows
        .first()
        .and_then(|row| cell(row, idx))
        .unwrap_or("");
    if raw.is_empty() || raw == "{}" {
        return Ok(Vec::new());
    }
    // The Map serializes to JSON in JSONCompact format. Parse defensively.
    let map: serde_json::Value = serde_json::from_str(raw).unwrap_or(serde_json::Value::Null);
    let entries = map
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(name, definition)| ConstraintInfo {
                    name: name.clone(),
                    kind: "check".to_string(),
                    definition: definition.as_str().unwrap_or("").to_string(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Schema relationships (no foreign keys ever)
// ---------------------------------------------------------------------------

pub async fn fetch_schema_relationships(
    connection: &StoredConnection,
    schema: &str,
) -> Result<SchemaRelationships, String> {
    let escaped = escape(schema);

    // Tables in the schema, MergeTree-family + others (we want the graph
    // node even for Distributed/View tables — users are looking at
    // shape, not editability).
    let tables_sql = format!(
        "SELECT name FROM system.tables WHERE database = '{}' \
         AND engine NOT IN ('View', 'MaterializedView', 'LiveView') \
         ORDER BY name",
        escaped
    );
    let tables_result = run_query(connection, &tables_sql).await?;
    let table_name_idx = column_index(&tables_result, "name");
    let table_names: Vec<String> = tables_result
        .rows
        .iter()
        .map(|row| cell_owned(row, table_name_idx))
        .filter(|name| !name.is_empty())
        .collect();

    if table_names.is_empty() {
        return Ok(SchemaRelationships {
            tables: Vec::new(),
            foreign_keys: Vec::new(),
        });
    }

    // Columns for every table in one round trip.
    let columns_sql = format!(
        "SELECT table, name, type, position, is_in_sorting_key \
         FROM system.columns \
         WHERE database = '{}' \
         ORDER BY table, position",
        escaped
    );
    let columns_result = run_query(connection, &columns_sql).await?;
    let table_idx = column_index(&columns_result, "table");
    let name_idx = column_index(&columns_result, "name");
    let type_idx = column_index(&columns_result, "type");
    let position_idx = column_index(&columns_result, "position");
    let sorting_idx = column_index(&columns_result, "is_in_sorting_key");

    let mut by_table: std::collections::HashMap<String, Vec<SchemaTableColumn>> =
        std::collections::HashMap::new();
    for row in &columns_result.rows {
        let table = cell_owned(row, table_idx);
        let column_type = cell(row, type_idx).unwrap_or("").to_string();
        let nullable = column_type.starts_with("Nullable(");
        let in_sorting = matches!(cell(row, sorting_idx), Some("1") | Some("true"));
        by_table.entry(table).or_default().push(SchemaTableColumn {
            name: cell_owned(row, name_idx),
            data_type: column_type,
            nullable,
            is_primary_key: in_sorting,
            ordinal_position: parse_int(cell(row, position_idx).unwrap_or("0")) as i32,
            comment: None,
        });
    }

    let tables: Vec<SchemaTableNode> = table_names
        .into_iter()
        .map(|name| {
            let columns = by_table.remove(&name).unwrap_or_default();
            SchemaTableNode {
                schema: schema.to_string(),
                name,
                column_count: columns.len() as u32,
                columns,
                // No FKs in ClickHouse, so junction detection and
                // relationship metadata never apply — omit rather than
                // guess.
                is_junction_table: None,
                triggers: Vec::new(),
            }
        })
        .collect();

    Ok(SchemaRelationships {
        tables,
        // CH has no foreign keys, ever.
        foreign_keys: Vec::<SchemaForeignKey>::new(),
    })
}

// ---------------------------------------------------------------------------
// Mutations (synchronous)
// ---------------------------------------------------------------------------

/// CH literal: `'string'` with single quotes doubled, or `NULL`.
///
/// Used by INSERT VALUES and ALTER … UPDATE/DELETE. CH's HTTP interface
/// has no `$1`-style binding, so the value is interpolated as a literal
/// — escaping is mandatory. NULL is the keyword, not a string.
fn ch_literal(value: Option<&str>) -> String {
    match value {
        None => "NULL".to_string(),
        Some(raw) => format!("'{}'", escape(raw)),
    }
}

fn build_insert(schema: &str, table: &str, values: &[CellEditKeyValue]) -> String {
    // Backtick-quote identifiers per `qualified_table_name`'s convention
    // for CH — keeps reserved words and odd column names safe.
    let qualified = format!("{}.{}", quote_backtick(schema), quote_backtick(table));
    let columns = values
        .iter()
        .map(|kv| quote_backtick(&kv.column))
        .collect::<Vec<_>>()
        .join(", ");
    let literals = values
        .iter()
        .map(|kv| ch_literal(kv.value.as_deref()))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({}) VALUES ({})",
        qualified, columns, literals
    )
}

/// Synchronous INSERT into a ClickHouse table.
///
/// Plain `INSERT INTO db.table (cols) VALUES (...)` over HTTP. Returns
/// after the data is written and is the only mutation in CH that maps
/// cleanly onto PostgreSQL's "rowsAffected" semantics — UPDATE/DELETE
/// queue async mutations and need separate polling.
pub async fn insert_row(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    values: &[CellEditKeyValue],
) -> Result<InsertRowResult, String> {
    let sql = build_insert(schema, table, values);
    let result = run_query(connection, &sql).await?;
    Ok(InsertRowResult {
        rows_affected: 1,
        runtime_ms: result.runtime_ms,
    })
}

// ---------------------------------------------------------------------------
// Table Seeding (ADR-0020)
// ---------------------------------------------------------------------------

/// ClickHouse writes one INSERT as a single atomic block, so the whole
/// run goes out as one statement — that is how the "never partially
/// apply" invariant is met on an engine with no transactions. Blocks
/// larger than the server's `max_insert_block_size` (1,048,576 rows by
/// default) get split into separate parts, which would break that, so
/// the run is capped well below it.
const CH_MAX_SEED_ROWS: u32 = 100_000;

pub async fn seed_table<F>(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    row_count: u32,
    seed: u64,
    specs: &[crate::SeedColumnSpec],
    report_progress: F,
) -> Result<crate::SeedTableResult, String>
where
    F: Fn(u64) + Send + Sync,
{
    use crate::seed::{
        analyze_plan, finalize_plan, generate_rows_from, insert_columns, SeedDialect, SeedRng,
    };

    if row_count == 0 {
        return Err("row count must be at least 1".to_string());
    }
    if row_count > CH_MAX_SEED_ROWS {
        return Err(format!(
            "ClickHouse seeding is capped at {CH_MAX_SEED_ROWS} rows per run — \
             it has no transactions, so a larger run could not be rolled back \
             as a unit. Run it several times instead."
        ));
    }

    let structure = fetch_table_structure(connection, schema, table).await?;
    if !structure.capabilities.can_insert_rows {
        return Err(format!(
            "table `{schema}`.`{table}` uses the {} engine, which does not accept inserts",
            structure.table_engine.as_deref().unwrap_or("unknown")
        ));
    }
    let start = Instant::now();

    let draft = analyze_plan(SeedDialect::ClickHouse, &structure, specs)?;
    // ClickHouse has no foreign keys, so no parent pool can ever be
    // needed; unique integer sequences have nothing to sample either.
    let plan = finalize_plan(
        &structure,
        draft,
        Vec::new(),
        &[],
        chrono::Utc::now().timestamp(),
    )?;

    let columns = insert_columns(&plan);
    if columns.is_empty() {
        return Err("nothing to seed: every column is skipped".to_string());
    }

    let mut rng = SeedRng::new(seed);
    let rows = generate_rows_from(&plan, 0, row_count, &mut rng);
    let sql = build_bulk_insert(schema, table, &columns, &rows);
    let result = run_query(connection, &sql).await?;
    report_progress(u64::from(row_count));

    Ok(crate::SeedTableResult {
        rows_inserted: u64::from(row_count),
        seed_used: seed,
        // `run_query` measures the HTTP round trip; the caller wants the
        // whole run, generation included.
        runtime_ms: start.elapsed().as_millis().max(result.runtime_ms as u128) as u64,
    })
}

/// `INSERT INTO db.table (cols) VALUES (…), (…), …` with every value
/// rendered as a literal — ClickHouse's HTTP interface has no bind
/// parameters for multi-row inserts.
fn build_bulk_insert(
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> String {
    let qualified = format!("{}.{}", quote_backtick(schema), quote_backtick(table));
    let column_list = columns
        .iter()
        .map(|column| quote_backtick(column))
        .collect::<Vec<_>>()
        .join(", ");
    let values = rows
        .iter()
        .map(|row| {
            let literals = (0..columns.len())
                .map(|index| ch_literal(row.get(index).and_then(|value| value.as_deref())))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({literals})")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("INSERT INTO {qualified} ({column_list}) VALUES {values}")
}

// ---------------------------------------------------------------------------
// Mutations (asynchronous — ALTER … UPDATE/DELETE)
// ---------------------------------------------------------------------------

/// Build a single `ALTER TABLE … UPDATE col1 = v1, col2 = v2 WHERE k = ?`
/// statement. CH does not group SET clauses across rows — one ALTER per
/// row keeps the WHERE clause precise (and per-row mutation IDs let the
/// frontend show fine-grained queue status).
fn build_alter_update(
    schema: &str,
    table: &str,
    set: &[CellEditKeyValue],
    identity: &[CellEditKeyValue],
) -> String {
    let qualified = format!("{}.{}", quote_backtick(schema), quote_backtick(table));
    let set_clause = set
        .iter()
        .map(|kv| {
            format!(
                "{} = {}",
                quote_backtick(&kv.column),
                ch_literal(kv.value.as_deref())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let where_clause = identity
        .iter()
        .map(|kv| match kv.value.as_deref() {
            None => format!("{} IS NULL", quote_backtick(&kv.column)),
            Some(_) => format!(
                "{} = {}",
                quote_backtick(&kv.column),
                ch_literal(kv.value.as_deref())
            ),
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "ALTER TABLE {} UPDATE {} WHERE {}",
        qualified, set_clause, where_clause
    )
}

fn build_alter_delete(schema: &str, table: &str, identity: &[CellEditKeyValue]) -> String {
    let qualified = format!("{}.{}", quote_backtick(schema), quote_backtick(table));
    let where_clause = identity
        .iter()
        .map(|kv| match kv.value.as_deref() {
            None => format!("{} IS NULL", quote_backtick(&kv.column)),
            Some(_) => format!(
                "{} = {}",
                quote_backtick(&kv.column),
                ch_literal(kv.value.as_deref())
            ),
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    format!("ALTER TABLE {} DELETE WHERE {}", qualified, where_clause)
}

/// Look up the most-recent mutation ID matching the predicate. CH does
/// not return the mutation ID from the ALTER call directly, so we infer
/// it from `system.mutations` immediately after submitting. The race
/// window between submit and lookup is small enough for interactive
/// editing; concurrent mutations against the same table from a third
/// party would need a stricter scheme.
async fn latest_mutation_id(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
) -> Result<Option<String>, String> {
    let escaped_schema = escape(schema);
    let escaped_table = escape(table);
    let sql = format!(
        "SELECT mutation_id FROM system.mutations \
         WHERE database = '{}' AND table = '{}' \
         ORDER BY create_time DESC LIMIT 1",
        escaped_schema, escaped_table
    );
    let result = run_query(connection, &sql).await?;
    let id_idx = column_index(&result, "mutation_id");
    Ok(result
        .rows
        .first()
        .and_then(|row| cell(row, id_idx))
        .map(|value| value.to_string()))
}

/// Apply cell edits via `ALTER TABLE … UPDATE`. Each edit becomes one
/// statement (CH does not batch UPDATE-with-WHERE). The returned
/// `mutation_ids` cover every issued ALTER; the frontend polls all of
/// them via [`poll_mutations`] before reporting "committed".
pub async fn commit_cell_edits(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    edits: &[CellEdit],
) -> Result<CommitCellEditsResult, String> {
    let start = Instant::now();
    let mut mutation_ids: Vec<String> = Vec::with_capacity(edits.len());
    for edit in edits {
        if edit.set.is_empty() {
            return Err("edit has no SET columns".to_string());
        }
        if edit.identity.is_empty() {
            return Err("edit has no identity columns".to_string());
        }
        let sql = build_alter_update(schema, table, &edit.set, &edit.identity);
        run_query(connection, &sql).await?;
        if let Some(id) = latest_mutation_id(connection, schema, table).await? {
            mutation_ids.push(id);
        }
    }
    Ok(CommitCellEditsResult {
        // ALTER UPDATE doesn't report a count — frontend treats this as
        // "queued, count unknown until is_done".
        rows_affected: 0,
        runtime_ms: start.elapsed().as_millis() as u64,
        state: "queued".to_string(),
        database: schema.to_string(),
        table: table.to_string(),
        mutation_ids,
    })
}

/// Apply row deletes via `ALTER TABLE … DELETE WHERE …`. Same async
/// semantics as [`commit_cell_edits`].
pub async fn delete_rows(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    rows: &[Vec<CellEditKeyValue>],
) -> Result<DeleteRowsResult, String> {
    let start = Instant::now();
    let mut mutation_ids: Vec<String> = Vec::with_capacity(rows.len());
    for identity in rows {
        if identity.is_empty() {
            return Err("missing identity".to_string());
        }
        let sql = build_alter_delete(schema, table, identity);
        run_query(connection, &sql).await?;
        if let Some(id) = latest_mutation_id(connection, schema, table).await? {
            mutation_ids.push(id);
        }
    }
    Ok(DeleteRowsResult {
        rows_affected: 0,
        runtime_ms: start.elapsed().as_millis() as u64,
        state: "queued".to_string(),
        database: schema.to_string(),
        table: table.to_string(),
        mutation_ids,
    })
}

// ---------------------------------------------------------------------------
// DDL — per-statement (no transaction wrapper, per ADR-0006)
// ---------------------------------------------------------------------------

/// Split a multi-statement DDL string on `;`, preserving non-empty
/// statements. The current splitter does not handle `;` inside string
/// literals or comments — same caveat as the PG path (which sends the
/// whole batch in one round trip and lets the server do the parsing).
fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(|stmt| stmt.trim())
        .filter(|stmt| !stmt.is_empty())
        .map(|stmt| stmt.to_string())
        .collect()
}

/// Run each `ALTER TABLE …` statement against ClickHouse independently.
///
/// CH does not support multi-statement transactions across DDL — there
/// is no `BEGIN/COMMIT` wrapper that would atomically apply or roll
/// back a batch of `ALTER` statements. So we execute one at a time and
/// stop on the first failure, returning whichever statement broke. The
/// frontend renders the per-statement preview so the user knows which
/// changes succeeded before the failure.
pub async fn execute_ddl(
    connection: &StoredConnection,
    sql: &str,
) -> Result<ExecuteDdlResult, String> {
    let start = Instant::now();
    let statements = split_statements(sql);
    if statements.is_empty() {
        return Ok(ExecuteDdlResult {
            runtime_ms: start.elapsed().as_millis() as u64,
        });
    }
    for (index, statement) in statements.iter().enumerate() {
        run_query(connection, statement).await.map_err(|error| {
            // Report which statement failed and how many were already
            // applied — that's the difference between "drop column
            // started failing" and "syntax error before any change".
            format!(
                "Statement {}/{} failed: {}\nApplied: {} earlier statement(s).",
                index + 1,
                statements.len(),
                error,
                index
            )
        })?;
    }
    Ok(ExecuteDdlResult {
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

/// Look up `is_done` + failure reason for a batch of mutation IDs.
///
/// One round trip per call: `system.mutations WHERE database = ? AND
/// table = ? AND mutation_id IN (...)` is cheap on CH (the table is a
/// small in-memory system table). The frontend calls this on a poll
/// cadence until all mutations are done.
pub async fn poll_mutations(
    connection: &StoredConnection,
    database: &str,
    table: &str,
    mutation_ids: &[String],
) -> Result<Vec<MutationStatus>, String> {
    if mutation_ids.is_empty() {
        return Ok(Vec::new());
    }
    let escaped_db = escape(database);
    let escaped_table = escape(table);
    let id_list = mutation_ids
        .iter()
        .map(|id| format!("'{}'", escape(id)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT mutation_id, is_done, latest_fail_reason \
         FROM system.mutations \
         WHERE database = '{}' AND table = '{}' AND mutation_id IN ({})",
        escaped_db, escaped_table, id_list
    );
    let result = run_query(connection, &sql).await?;
    let id_idx = column_index(&result, "mutation_id");
    let done_idx = column_index(&result, "is_done");
    let fail_idx = column_index(&result, "latest_fail_reason");
    let mut by_id: std::collections::HashMap<String, MutationStatus> =
        std::collections::HashMap::new();
    for row in &result.rows {
        let id = cell_owned(row, id_idx);
        let is_done = matches!(cell(row, done_idx), Some("1") | Some("true"));
        let fail = cell(row, fail_idx).map(|value| value.to_string());
        let latest_fail_reason = match fail {
            None => None,
            Some(value) if value.is_empty() => None,
            Some(value) => Some(value),
        };
        by_id.insert(
            id.clone(),
            MutationStatus {
                mutation_id: id,
                is_done,
                latest_fail_reason,
            },
        );
    }
    // Preserve caller's input order; missing IDs report not-yet-visible
    // (is_done = false) rather than 404 — the caller polls again.
    Ok(mutation_ids
        .iter()
        .map(|id| {
            by_id.remove(id).unwrap_or_else(|| MutationStatus {
                mutation_id: id.clone(),
                is_done: false,
                latest_fail_reason: None,
            })
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Database overview stats
// ---------------------------------------------------------------------------

pub async fn fetch_database_overview_stats(
    connection: &StoredConnection,
) -> Result<DatabaseOverviewStats, String> {
    let database = database(connection)?;
    let escaped = escape(&database);

    // One round trip for everything that lives in `system.parts`.
    let parts_sql = format!(
        "SELECT \
            sum(bytes_on_disk) AS database_bytes, \
            sum(primary_key_bytes_in_memory) AS pk_bytes, \
            sum(rows) AS row_count, \
            uniqExact(table) AS table_count \
         FROM system.parts \
         WHERE database = '{}' AND active",
        escaped
    );
    let parts = run_query(connection, &parts_sql).await?;
    let parts_row = parts.rows.first();
    let database_bytes = parts_row
        .and_then(|row| cell(row, column_index(&parts, "database_bytes")))
        .map(parse_int)
        .unwrap_or(0);
    let pk_bytes = parts_row
        .and_then(|row| cell(row, column_index(&parts, "pk_bytes")))
        .map(parse_int)
        .unwrap_or(0);
    let row_count = parts_row
        .and_then(|row| cell(row, column_index(&parts, "row_count")))
        .map(parse_int)
        .unwrap_or(0);
    let table_count = parts_row
        .and_then(|row| cell(row, column_index(&parts, "table_count")))
        .map(parse_int)
        .unwrap_or(0);

    // Skip-index count.
    let indices_sql = format!(
        "SELECT count() AS index_count FROM system.data_skipping_indices WHERE database = '{}'",
        escaped
    );
    let indices = run_query(connection, &indices_sql).await?;
    let index_count = indices
        .rows
        .first()
        .and_then(|row| cell(row, column_index(&indices, "index_count")))
        .map(parse_int)
        .unwrap_or(0);

    // Connection count from system.processes — this counts active queries
    // server-wide, not per-database. Best analogue we have.
    let processes = run_query(
        connection,
        "SELECT count() AS connection_count FROM system.processes",
    )
    .await?;
    let connection_count = processes
        .rows
        .first()
        .and_then(|row| cell(row, column_index(&processes, "connection_count")))
        .map(parse_int)
        .unwrap_or(0);

    Ok(DatabaseOverviewStats {
        database_size_bytes: database_bytes,
        // CH doesn't separate "table" vs "index" bytes the way PG does —
        // bytes_on_disk is the closest single-number analogue for both.
        table_size_bytes: database_bytes,
        index_size_bytes: pk_bytes,
        table_count,
        // The active connection scopes to one CH database.
        schema_count: 1,
        row_count_estimate: row_count,
        index_count,
        connection_count,
    })
}

#[cfg(test)]
mod tests {
    //! Unit tests for the URL/escape/parse helpers — every other public
    //! function in this module needs a live ClickHouse to exercise. The
    //! parser is the highest-leverage thing to test in isolation: it
    //! consumes opaque JSON from the network and turns it into the shape
    //! the rest of the app trusts.
    use super::*;
    use crate::ClickHouseStoredConnection;

    fn ch_conn(host: &str, database: &str, port: u16) -> StoredConnection {
        StoredConnection::ClickHouse(ClickHouseStoredConnection {
            id: "ch".into(),
            name: "ch".into(),
            database: database.into(),
            host: host.into(),
            port,
            user: String::new(),
            password: String::new(),
            role: String::new(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            use_https: false,
            url_path: String::new(),
            ssh_tunnel: crate::SshTunnelConfig::default(),
        })
    }

    /// Mutably borrow the inner CH variant of a fixture for tests that
    /// tweak engine-specific fields. Tests construct CH variants only,
    /// so the panic is a wiring bug rather than runtime input.
    fn ch_mut(connection: &mut StoredConnection) -> &mut ClickHouseStoredConnection {
        match connection {
            StoredConnection::ClickHouse(ch) => ch,
            _ => panic!("fixture is not a ClickHouse variant"),
        }
    }

    #[test]
    fn database_defaults_to_literal_default_when_blank() {
        let connection = ch_conn("localhost", "", 0);
        assert_eq!(database(&connection).unwrap(), "default");
    }

    #[test]
    fn database_preserves_user_value() {
        let connection = ch_conn("localhost", "analytics", 0);
        assert_eq!(database(&connection).unwrap(), "analytics");
    }

    #[test]
    fn escape_doubles_single_quotes() {
        assert_eq!(escape("plain"), "plain");
        assert_eq!(escape("o'brien"), "o''brien");
        assert_eq!(escape("''"), "''''");
    }

    #[test]
    fn url_uses_8123_when_port_is_zero_and_https_off() {
        let connection = ch_conn("localhost", "", 0);
        let built = url(&connection).expect("url");
        assert_eq!(built.scheme(), "http");
        assert_eq!(built.host_str(), Some("localhost"));
        assert_eq!(built.port(), Some(8123));
        assert!(built
            .query()
            .unwrap()
            .contains("default_format=JSONCompact"));
    }

    #[test]
    fn url_uses_8443_when_port_is_zero_and_https_on() {
        let mut connection = ch_conn("localhost", "", 0);
        ch_mut(&mut connection).use_https = true;
        let built = url(&connection).expect("url");
        assert_eq!(built.scheme(), "https");
        assert_eq!(built.port(), Some(8443));
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
    fn url_uses_custom_path_when_set() {
        let mut connection = ch_conn("localhost", "", 0);
        ch_mut(&mut connection).url_path = "/clickhouse".to_string();
        let built = url(&connection).expect("url");
        assert_eq!(built.path(), "/clickhouse");
    }

    #[test]
    fn url_normalizes_path_without_leading_slash() {
        let mut connection = ch_conn("localhost", "", 0);
        ch_mut(&mut connection).url_path = "clickhouse".to_string();
        let built = url(&connection).expect("url");
        assert_eq!(built.path(), "/clickhouse");
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

    #[test]
    fn parse_csv_list_splits_and_trims() {
        assert_eq!(parse_csv_list(""), Vec::<String>::new());
        assert_eq!(parse_csv_list("a"), vec!["a"]);
        assert_eq!(parse_csv_list("a, b ,c"), vec!["a", "b", "c"]);
        assert_eq!(parse_csv_list(", ,"), Vec::<String>::new());
    }

    fn kv(column: &str, value: Option<&str>) -> CellEditKeyValue {
        CellEditKeyValue {
            column: column.to_string(),
            value: value.map(|raw| raw.to_string()),
        }
    }

    #[test]
    fn ch_literal_quotes_strings_and_passes_null_through() {
        assert_eq!(ch_literal(None), "NULL");
        assert_eq!(ch_literal(Some("plain")), "'plain'");
        assert_eq!(ch_literal(Some("o'brien")), "'o''brien'");
    }

    #[test]
    fn build_insert_emits_backtick_qualified_table_with_literal_values() {
        let values = vec![
            kv("id", Some("1")),
            kv("name", Some("Alice")),
            kv("middle_name", None),
        ];
        let sql = build_insert("analytics", "users", &values);
        assert_eq!(
            sql,
            "INSERT INTO `analytics`.`users` (`id`, `name`, `middle_name`) VALUES ('1', 'Alice', NULL)"
        );
    }

    #[test]
    fn build_insert_quotes_identifiers_and_escapes_quotes_in_values() {
        let values = vec![kv("weird`col", Some("o'brien"))];
        let sql = build_insert("my`schema", "tbl", &values);
        assert_eq!(
            sql,
            "INSERT INTO `my``schema`.`tbl` (`weird``col`) VALUES ('o''brien')"
        );
    }

    #[test]
    fn build_alter_update_renders_set_and_where_with_literals() {
        let set = vec![kv("name", Some("Alice")), kv("email", Some("a@b.c"))];
        let identity = vec![kv("id", Some("42"))];
        let sql = build_alter_update("analytics", "users", &set, &identity);
        assert_eq!(
            sql,
            "ALTER TABLE `analytics`.`users` UPDATE `name` = 'Alice', `email` = 'a@b.c' WHERE `id` = '42'"
        );
    }

    #[test]
    fn build_alter_update_emits_is_null_for_null_identity() {
        let set = vec![kv("name", Some("Alice"))];
        let identity = vec![kv("deleted_at", None)];
        let sql = build_alter_update("analytics", "users", &set, &identity);
        assert_eq!(
            sql,
            "ALTER TABLE `analytics`.`users` UPDATE `name` = 'Alice' WHERE `deleted_at` IS NULL"
        );
    }

    #[test]
    fn build_alter_update_supports_setting_null_value() {
        let set = vec![kv("name", None)];
        let identity = vec![kv("id", Some("1"))];
        let sql = build_alter_update("analytics", "t", &set, &identity);
        assert_eq!(
            sql,
            "ALTER TABLE `analytics`.`t` UPDATE `name` = NULL WHERE `id` = '1'"
        );
    }

    #[test]
    fn build_alter_delete_renders_where_clause() {
        let identity = vec![kv("id", Some("42")), kv("tenant", Some("acme"))];
        let sql = build_alter_delete("analytics", "users", &identity);
        assert_eq!(
            sql,
            "ALTER TABLE `analytics`.`users` DELETE WHERE `id` = '42' AND `tenant` = 'acme'"
        );
    }

    #[test]
    fn build_alter_delete_handles_null_identity() {
        let identity = vec![kv("deleted_at", None), kv("id", Some("1"))];
        let sql = build_alter_delete("analytics", "t", &identity);
        assert_eq!(
            sql,
            "ALTER TABLE `analytics`.`t` DELETE WHERE `deleted_at` IS NULL AND `id` = '1'"
        );
    }

    #[test]
    fn split_statements_drops_empty_segments() {
        // Trailing semicolons + whitespace shouldn't manufacture phantom statements.
        let sql = "ALTER TABLE t DROP COLUMN a;  ALTER TABLE t DROP COLUMN b;\n\n";
        assert_eq!(
            split_statements(sql),
            vec![
                "ALTER TABLE t DROP COLUMN a".to_string(),
                "ALTER TABLE t DROP COLUMN b".to_string(),
            ]
        );
    }

    #[test]
    fn split_statements_returns_empty_for_blank_input() {
        assert!(split_statements("").is_empty());
        assert!(split_statements(";; ;").is_empty());
    }
}

#[cfg(test)]
mod seed_tests {
    use super::*;

    fn columns() -> Vec<String> {
        vec!["id".to_string(), "label".to_string()]
    }

    #[test]
    fn bulk_insert_emits_one_statement_with_every_row_as_literals() {
        let sql = build_bulk_insert(
            "demo",
            "events",
            &columns(),
            &[
                vec![Some("1".to_string()), Some("a".to_string())],
                vec![Some("2".to_string()), None],
            ],
        );
        assert_eq!(
            sql,
            "INSERT INTO `demo`.`events` (`id`, `label`) VALUES ('1', 'a'), ('2', NULL)"
        );
    }

    #[test]
    fn bulk_insert_escapes_quotes_in_generated_values() {
        let sql = build_bulk_insert(
            "demo",
            "events",
            &["label".to_string()],
            &[vec![Some("it's".to_string())]],
        );
        assert_eq!(
            sql,
            "INSERT INTO `demo`.`events` (`label`) VALUES ('it''s')"
        );
    }

    #[test]
    fn bulk_insert_pads_short_rows_with_null() {
        let sql = build_bulk_insert("demo", "events", &columns(), &[vec![Some("1".to_string())]]);
        assert_eq!(
            sql,
            "INSERT INTO `demo`.`events` (`id`, `label`) VALUES ('1', NULL)"
        );
    }

    #[tokio::test]
    async fn seeding_more_rows_than_one_block_is_refused() {
        let error = seed_table(
            &ch_test_connection(),
            "dbunk_demo",
            "anything",
            CH_MAX_SEED_ROWS + 1,
            1,
            &[],
            |_| {},
        )
        .await
        .expect_err("over-cap run must be refused");
        assert!(error.contains("capped"), "unexpected: {error}");
    }

    fn ch_test_connection() -> StoredConnection {
        StoredConnection::ClickHouse(ClickHouseStoredConnection {
            id: "seed-ch-test".into(),
            name: "seed ch test".into(),
            database: "dbunk_demo".into(),
            host: "localhost".into(),
            port: 18123,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            use_https: false,
            url_path: String::new(),
            ssh_tunnel: crate::SshTunnelConfig::default(),
        })
    }

    /// End-to-end against the compose ClickHouse
    /// (`infrastructure/test-db`, `make clickhouse`). Ignored by
    /// default: `cargo test --manifest-path src-tauri/Cargo.toml
    /// ch_seed_live -- --ignored`
    #[tokio::test]
    #[ignore = "requires the infrastructure/test-db clickhouse container"]
    async fn ch_seed_live_end_to_end() {
        let connection = ch_test_connection();
        run_query(&connection, "DROP TABLE IF EXISTS dbunk_demo.seed_events")
            .await
            .expect("drop");
        run_query(
            &connection,
            "CREATE TABLE dbunk_demo.seed_events (
               user_id UInt32,
               email String,
               score UInt8,
               tier LowCardinality(String),
               is_active Bool,
               tags Array(String),
               source String DEFAULT 'web',
               occurred_at DateTime,
               day Date MATERIALIZED toDate(occurred_at)
             ) ENGINE = MergeTree ORDER BY (user_id, occurred_at)",
        )
        .await
        .expect("create");

        let result = seed_table(
            &connection,
            "dbunk_demo",
            "seed_events",
            5_000,
            42,
            &[],
            |_| {},
        )
        .await
        .expect("seed events");
        assert_eq!(result.rows_inserted, 5_000);
        assert_eq!(result.seed_used, 42);

        let count = run_query(&connection, "SELECT count() FROM dbunk_demo.seed_events")
            .await
            .expect("count");
        assert_eq!(count.rows[0][0], "5000");

        // UInt8 stayed in range, Bool got digits, the MATERIALIZED
        // column was computed by the server, and DEFAULT was applied.
        let checks = run_query(
            &connection,
            "SELECT max(score), min(day) = toDate(min(occurred_at)), \
                    countIf(source != 'web'), countIf(length(tags) != 0) \
             FROM dbunk_demo.seed_events",
        )
        .await
        .expect("checks");
        assert!(
            checks.rows[0][0].parse::<u32>().expect("max score") <= 255,
            "score overflowed UInt8"
        );
        assert_eq!(checks.rows[0][1], "1", "MATERIALIZED day was not derived");
        assert_eq!(checks.rows[0][2], "0", "DEFAULT source was overwritten");
        assert_eq!(checks.rows[0][3], "0", "arrays should seed empty");

        run_query(&connection, "DROP TABLE dbunk_demo.seed_events")
            .await
            .expect("cleanup");
    }
}
