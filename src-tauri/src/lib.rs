mod clickhouse;
mod dispatch;
mod keychain;
mod postgres;
mod storage;
mod types;

// Re-export DTOs at the crate root so existing `crate::Foo` paths in child
// modules and the `#[tauri::command]` macros keep working unchanged.
pub(crate) use types::*;

use tauri::{Manager, State};

use crate::{keychain::CredentialUpdate, storage::Paths};

const MAX_QUERY_HISTORY: usize = 200;


const DEFAULT_TABLE_PAGE_SIZE: u32 = 100;
const MAX_TABLE_PAGE_SIZE: u32 = 1000;

pub(crate) fn quote_double(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(crate) fn quote_backtick(identifier: &str) -> String {
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

/// Bytes → `0x` hex string. Used by every engine's value coercion path
/// for binary column display (Postgres `bytea`, sqlx-Any `Vec<u8>`),
/// kept at the crate root so each engine module can reach it without
/// re-implementing.
pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2 + 2);
    output.push_str("0x");
    for byte in bytes {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}

// Engine-aware operations (run_query, DDL, mutations, introspection)
// live in `dispatch.rs`. The `#[tauri::command]` functions in this file
// validate payloads and track activity; the dispatch module owns the
// per-engine match statements. See ADR-0001 and CONTEXT.md's "Engine
// Dispatch" entry.

// Path resolution + per-entity JSON read/write live in `storage` — see
// `storage.rs`. Internal helpers below take `&Paths` so they can be exercised
// over a tempdir without a Tauri runtime.

// Connection passwords live in the keychain — see `keychain.rs` for the
// single-blob shape (ADR-0005), the in-process cache, and migration helpers.

/// Hydrate a list of connections with passwords from the keychain. If
/// `connections.json` still carries plaintext passwords (a pre-A6 release
/// shape) they're folded into the keychain blob and the JSON is rewritten
/// clean.
fn read_connections_full(paths: &Paths) -> Result<Vec<StoredConnection>, String> {
    let mut entries = storage::read_connections(paths)?;

    // One-time migration: lift any plaintext passwords from JSON into the
    // keychain blob so a `git pull` on a working machine doesn't lose
    // credentials. Hard-saves a single keychain write covering all of them.
    let plaintext_set: Vec<CredentialUpdate<'_>> = entries
        .iter()
        .filter(|entry| !entry.password.is_empty())
        .map(|entry| CredentialUpdate::Set {
            id: entry.id.as_str(),
            password: entry.password.as_str(),
        })
        .collect();
    if !plaintext_set.is_empty() {
        if let Err(error) = keychain::upsert_many(&plaintext_set) {
            eprintln!("Failed to migrate plaintext passwords to keychain: {error}");
        }
        for entry in entries.iter_mut() {
            entry.password.clear();
        }
        if let Err(error) = storage::write_connections(paths, &entries) {
            eprintln!("Failed to clear plaintext from connections.json: {error}");
        }
    }

    // Fill from the keychain (cached after first call).
    for entry in entries.iter_mut() {
        if entry.password.is_empty() {
            if let Some(password) = keychain::get(&entry.id) {
                entry.password = password;
            }
        }
    }

    Ok(entries)
}

/// Persist the given connections: passwords go to the keychain blob, the
/// rest goes to JSON with empty `password` fields. Existing keychain entries
/// for connections outside this batch are preserved.
fn write_connections_full(
    paths: &Paths,
    connections: &[StoredConnection],
) -> Result<(), String> {
    let updates: Vec<CredentialUpdate<'_>> = connections
        .iter()
        .map(|connection| {
            if connection.password.is_empty() {
                CredentialUpdate::Clear {
                    id: connection.id.as_str(),
                }
            } else {
                CredentialUpdate::Set {
                    id: connection.id.as_str(),
                    password: connection.password.as_str(),
                }
            }
        })
        .collect();
    keychain::upsert_many(&updates)?;

    let mut clean = connections.to_vec();
    for entry in clean.iter_mut() {
        entry.password.clear();
    }
    storage::write_connections(paths, &clean)
}

fn find_connection(paths: &Paths, connection_id: &str) -> Result<StoredConnection, String> {
    let connections = read_connections_full(paths)?;
    connections
        .into_iter()
        .find(|connection| connection.id == connection_id)
        .ok_or_else(|| "Connection not found".to_string())
}

/// Run `op` against a connection and bump its `lastActivityAt` on success.
///
/// Owns the contract from ADR-0004: every successful operation against a
/// connection counts as activity. By making the bump a property of the
/// helper rather than each command, new commands inherit the behaviour for
/// free and can't quietly drift out of policy.
///
/// The bump only fires when `op` returns `Ok` — failed queries do not count
/// as activity, so a connection that's unreachable doesn't appear "fresh".
async fn with_active_connection<T, Fut>(
    paths: &Paths,
    connection_id: &str,
    op: impl FnOnce(StoredConnection) -> Fut,
) -> Result<T, String>
where
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let connection = find_connection(paths, connection_id)?;
    let result = op(connection).await?;
    touch_connection_activity(paths, connection_id);
    Ok(result)
}

/// Bump the `lastActivityAt` field on a connection record. Best-effort —
/// failures are logged but never bubble up because activity tracking should
/// not break the underlying operation.
fn touch_connection_activity(paths: &Paths, connection_id: &str) {
    // Reads the JSON layer only — we don't need passwords just to bump a
    // timestamp, and avoiding the keychain hop keeps this hot path cheap.
    let Ok(mut connections) = storage::read_connections(paths) else {
        return;
    };
    let now = chrono::Utc::now().to_rfc3339();
    let mut changed = false;
    for connection in connections.iter_mut() {
        if connection.id == connection_id {
            connection.last_activity_at = Some(now.clone());
            changed = true;
            break;
        }
    }
    if changed {
        if let Err(error) = storage::write_connections(paths, &connections) {
            eprintln!("Failed to touch lastActivityAt: {error}");
        }
    }
}

#[tauri::command]
async fn load_connections(
    paths: State<'_, Paths>,
) -> Result<Vec<StoredConnection>, String> {
    read_connections_full(paths.inner())
}

#[tauri::command]
async fn save_connection(
    paths: State<'_, Paths>,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    let paths = paths.inner();
    let mut connections = read_connections_full(paths)?;

    if let Some(existing) = connections.iter_mut().find(|item| item.id == connection.id) {
        *existing = connection.clone();
    } else {
        connections.push(connection.clone());
    }

    write_connections_full(paths, &connections)?;
    // Re-hydrate so the response has the keychain-backed passwords (and the
    // cleaned shape) the caller will use to build subsequent payloads.
    read_connections_full(paths)
}

#[tauri::command]
async fn delete_connection(
    paths: State<'_, Paths>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    let paths = paths.inner();
    let mut connections = read_connections_full(paths)?;

    let initial_len = connections.len();
    connections.retain(|item| item.id != payload.connection_id);

    if connections.len() == initial_len {
        return Err(format!("Connection '{}' not found", payload.connection_id));
    }

    // Best-effort: remove the keychain entry too. If the keychain write fails
    // we still want the JSON delete to land — the worst case is an orphan
    // credential the user can clean up manually.
    if let Err(error) = keychain::delete(&payload.connection_id) {
        eprintln!(
            "Failed to delete keychain entry for {}: {error}",
            payload.connection_id
        );
    }

    write_connections_full(paths, &connections)?;
    Ok(connections)
}

#[tauri::command]
async fn connect_connection(
    paths: State<'_, Paths>,
    payload: ConnectionPayload,
) -> Result<ConnectResult, String> {
    with_active_connection(paths.inner(), &payload.connection_id, |connection| async move {
        dispatch::ping_connection(&connection).await
    })
    .await
}

/// Validate credentials without saving them. Used by the New Connection
/// side panel's `Test Connection` button — connects, runs `SELECT 1`,
/// disconnects, returns latency or surfaces the underlying driver error.
#[tauri::command]
async fn test_connection(payload: TestConnectionPayload) -> Result<ConnectResult, String> {
    dispatch::ping_connection(&payload.connection).await
}

/// Periodic poll: returns "healthy" + latency or "error" + message. Designed
/// for a frontend interval that fans out across all stored connections — the
/// caller decides cadence and concurrency.
/// Health check is a probe, not a use of the connection — we deliberately do
/// NOT route through `with_active_connection` so a 30 s tick doesn't keep
/// `lastActivityAt` artificially fresh.
#[tauri::command]
async fn health_check_connection(
    paths: State<'_, Paths>,
    payload: ConnectionPayload,
) -> Result<HealthCheckResult, String> {
    let connection = find_connection(paths.inner(), &payload.connection_id)?;
    match dispatch::ping_connection(&connection).await {
        Ok(result) => Ok(HealthCheckResult::Healthy {
            latency_ms: result.latency_ms,
        }),
        Err(error) => Ok(HealthCheckResult::Error { error }),
    }
}

#[tauri::command]
async fn load_schema_explorer(
    paths: State<'_, Paths>,
    payload: ConnectionPayload,
) -> Result<Vec<SchemaExplorer>, String> {
    with_active_connection(paths.inner(), &payload.connection_id, |connection| async move {
        dispatch::load_schema_explorer(&connection).await
    })
    .await
}

#[tauri::command]
async fn run_query(
    paths: State<'_, Paths>,
    payload: RunQueryPayload,
) -> Result<QueryResult, String> {
    let RunQueryPayload {
        connection_id,
        query,
    } = payload;
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::run_query(&connection, &query).await
    })
    .await
}

#[tauri::command]
async fn load_table_structure(
    paths: State<'_, Paths>,
    payload: LoadTableStructurePayload,
) -> Result<TableStructure, String> {
    let LoadTableStructurePayload {
        connection_id,
        schema,
        table,
    } = payload;
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::fetch_table_structure(&connection, &schema, &table).await
    })
    .await
}

#[tauri::command]
async fn execute_ddl(
    paths: State<'_, Paths>,
    payload: ExecuteDdlPayload,
) -> Result<ExecuteDdlResult, String> {
    let ExecuteDdlPayload {
        connection_id,
        sql,
    } = payload;
    if sql.trim().is_empty() {
        return Err("DDL statement is empty".to_string());
    }
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::execute_ddl(&connection, &sql).await
    })
    .await
}

#[tauri::command]
async fn commit_cell_edits(
    paths: State<'_, Paths>,
    payload: CommitCellEditsPayload,
) -> Result<CommitCellEditsResult, String> {
    let CommitCellEditsPayload {
        connection_id,
        schema,
        table,
        edits,
    } = payload;
    if edits.is_empty() {
        return Err("no edits to commit".to_string());
    }
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::commit_cell_edits(&connection, &schema, &table, &edits).await
    })
    .await
}

#[tauri::command]
async fn insert_row(
    paths: State<'_, Paths>,
    payload: InsertRowPayload,
) -> Result<InsertRowResult, String> {
    let InsertRowPayload {
        connection_id,
        schema,
        table,
        values,
    } = payload;
    if values.is_empty() {
        return Err("no values provided".to_string());
    }
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::insert_row(&connection, &schema, &table, &values).await
    })
    .await
}

#[tauri::command]
async fn delete_rows(
    paths: State<'_, Paths>,
    payload: DeleteRowsPayload,
) -> Result<DeleteRowsResult, String> {
    let DeleteRowsPayload {
        connection_id,
        schema,
        table,
        rows,
    } = payload;
    if rows.is_empty() {
        return Err("no rows provided".to_string());
    }
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::delete_rows(&connection, &schema, &table, &rows).await
    })
    .await
}

#[tauri::command]
async fn poll_mutation_status(
    paths: State<'_, Paths>,
    payload: PollMutationStatusPayload,
) -> Result<Vec<MutationStatus>, String> {
    let PollMutationStatusPayload {
        connection_id,
        database,
        table,
        mutation_ids,
    } = payload;
    if mutation_ids.is_empty() {
        return Ok(Vec::new());
    }
    // Mutation polling does NOT mark connection activity (it's a probe
    // for in-flight async work, not a use of the connection). Same
    // policy as health_check_connection.
    let connection = find_connection(paths.inner(), &connection_id)?;
    dispatch::poll_mutation_status(&connection, &database, &table, &mutation_ids).await
}

#[tauri::command]
async fn load_schema_relationships(
    paths: State<'_, Paths>,
    payload: LoadSchemaRelationshipsPayload,
) -> Result<SchemaRelationships, String> {
    let LoadSchemaRelationshipsPayload {
        connection_id,
        schema,
    } = payload;
    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        dispatch::fetch_schema_relationships(&connection, &schema).await
    })
    .await
}

#[tauri::command]
async fn load_database_overview_stats(
    paths: State<'_, Paths>,
    payload: LoadDatabaseOverviewStatsPayload,
) -> Result<DatabaseOverviewStats, String> {
    with_active_connection(paths.inner(), &payload.connection_id, |connection| async move {
        dispatch::fetch_database_overview_stats(&connection).await
    })
    .await
}

#[tauri::command]
async fn load_table_data(
    paths: State<'_, Paths>,
    payload: LoadTableDataPayload,
) -> Result<TableDataResult, String> {
    let LoadTableDataPayload {
        connection_id,
        schema,
        table,
        page,
        page_size,
    } = payload;
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size
        .unwrap_or(DEFAULT_TABLE_PAGE_SIZE)
        .clamp(1, MAX_TABLE_PAGE_SIZE);
    let offset = (page - 1) as u64 * page_size as u64;

    with_active_connection(paths.inner(), &connection_id, |connection| async move {
        let qualified = qualified_table_name(&connection.engine, &schema, &table);

        // SELECT with LIMIT/OFFSET works for all four supported engines
        // (PostgreSQL, MySQL, SQLite, ClickHouse).
        let select_query = format!(
            "SELECT * FROM {} LIMIT {} OFFSET {}",
            qualified, page_size, offset
        );

        let select_result = dispatch::run_query(&connection, &select_query).await?;

        // Best-effort COUNT(*) — never fail the call if the count fails.
        let count_query = format!("SELECT COUNT(*) FROM {}", qualified);
        let total_rows = match dispatch::run_query(&connection, &count_query).await {
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
    })
    .await
}

#[tauri::command]
async fn load_query_history(
    paths: State<'_, Paths>,
    limit: Option<u32>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let mut entries = storage::read_query_history(paths.inner());
    if let Some(limit) = limit {
        entries.truncate(limit as usize);
    }
    Ok(entries)
}

#[tauri::command]
async fn append_query_history(
    paths: State<'_, Paths>,
    entry: QueryHistoryEntry,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let paths = paths.inner();
    let mut entries = storage::read_query_history(paths);
    // Newest first; cap to MAX_QUERY_HISTORY.
    entries.insert(0, entry);
    if entries.len() > MAX_QUERY_HISTORY {
        entries.truncate(MAX_QUERY_HISTORY);
    }
    storage::write_query_history(paths, &entries)?;
    Ok(entries)
}

#[tauri::command]
async fn clear_query_history(paths: State<'_, Paths>) -> Result<(), String> {
    storage::write_query_history(paths.inner(), &[])
}

#[tauri::command]
async fn load_saved_queries(
    paths: State<'_, Paths>,
) -> Result<Vec<SavedQuery>, String> {
    Ok(storage::read_saved_queries(paths.inner()))
}

/// Insert or update by `id` (idempotent). Bumps `updatedAt` automatically;
/// callers leave that field as the previous value.
#[tauri::command]
async fn save_saved_query(
    paths: State<'_, Paths>,
    query: SavedQuery,
) -> Result<Vec<SavedQuery>, String> {
    let paths = paths.inner();
    let mut entries = storage::read_saved_queries(paths);
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = query.clone();
    next.updated_at = now.clone();
    if let Some(existing) = entries.iter_mut().find(|entry| entry.id == query.id) {
        *existing = next;
    } else {
        next.created_at = now;
        entries.push(next);
    }
    storage::write_saved_queries(paths, &entries)?;
    Ok(entries)
}

#[tauri::command]
async fn delete_saved_query(
    paths: State<'_, Paths>,
    payload: DeleteSavedQueryPayload,
) -> Result<Vec<SavedQuery>, String> {
    let paths = paths.inner();
    let mut entries = storage::read_saved_queries(paths);
    entries.retain(|entry| entry.id != payload.id);
    storage::write_saved_queries(paths, &entries)?;
    Ok(entries)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    dispatch::ensure_sqlx_drivers();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Resolve and register the on-disk paths once. Every command
            // reaches the JSON layer via `tauri::State<Paths>` rather than
            // re-resolving from `AppHandle` on each invocation, which keeps
            // the persistence layer testable in pure Rust.
            let paths = Paths::from_app(&app.handle())
                .map_err(|error| format!("Failed to resolve config dir: {error}"))?;
            app.manage(paths);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_connections,
            save_connection,
            delete_connection,
            connect_connection,
            test_connection,
            health_check_connection,
            load_schema_explorer,
            load_schema_relationships,
            load_database_overview_stats,
            run_query,
            load_table_data,
            load_table_structure,
            execute_ddl,
            commit_cell_edits,
            insert_row,
            delete_rows,
            poll_mutation_status,
            load_query_history,
            append_query_history,
            clear_query_history,
            load_saved_queries,
            save_saved_query,
            delete_saved_query
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
