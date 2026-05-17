mod clickhouse;
mod credentials;
mod dispatch;
mod keychain;
mod postgres;
mod redis;
mod storage;
mod types;

// Re-export DTOs at the crate root so existing `crate::Foo` paths in child
// modules and the `#[tauri::command]` macros keep working unchanged.
pub(crate) use types::*;

use sqlx::SqlitePool;
use tauri::{Manager, State};

use crate::storage::Paths;

const MAX_QUERY_HISTORY: usize = storage::QUERY_HISTORY_CAP as usize;

const DEFAULT_TABLE_PAGE_SIZE: u32 = 100;
const MAX_TABLE_PAGE_SIZE: u32 = 1000;
// Keep in sync with `app.windows[0].trafficLightPosition` in tauri.conf.json.
const MACOS_TRAFFIC_LIGHT_X: f64 = 18.0;
const MACOS_TRAFFIC_LIGHT_Y: f64 = 26.0;

pub(crate) struct AppState {
    pool: SqlitePool,
    paths: Paths,
}

pub(crate) fn quote_double(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(crate) fn quote_backtick(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

pub(crate) fn qualified_table_name(engine: &DatabaseEngine, schema: &str, table: &str) -> String {
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
        DatabaseEngine::Redis => {
            // Redis has no tables — qualified_table_name should never be
            // called on a Redis connection. The dispatch router prevents
            // it; this branch is an invariant assertion.
            unreachable!("BUG: qualified_table_name called on Redis connection")
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

#[tauri::command]
fn restore_window_traffic_light_position(window: tauri::Window) -> Result<(), String> {
    apply_window_traffic_light_position(&window)
}

#[cfg(target_os = "macos")]
fn apply_window_traffic_light_position(window: &tauri::Window) -> Result<(), String> {
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};

    let ns_window = window.ns_window().map_err(|error| error.to_string())?;
    let ns_window: &NSWindow = unsafe { &*ns_window.cast::<NSWindow>() };

    unsafe {
        let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
            return Ok(());
        };
        let Some(miniaturize) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton)
        else {
            return Ok(());
        };
        let zoom = ns_window.standardWindowButton(NSWindowButton::ZoomButton);
        let Some(title_bar_container_view) = close.superview().and_then(|view| view.superview())
        else {
            return Ok(());
        };

        let close_rect = NSView::frame(&close);
        let title_bar_frame_height = close_rect.size.height + MACOS_TRAFFIC_LIGHT_Y;
        let mut title_bar_rect = NSView::frame(&title_bar_container_view);
        title_bar_rect.size.height = title_bar_frame_height;
        title_bar_rect.origin.y = ns_window.frame().size.height - title_bar_frame_height;
        title_bar_container_view.setFrame(title_bar_rect);

        let space_between = NSView::frame(&miniaturize).origin.x - close_rect.origin.x;
        let mut window_buttons = vec![close, miniaturize];
        if let Some(zoom) = zoom {
            window_buttons.push(zoom);
        }

        for (index, button) in window_buttons.into_iter().enumerate() {
            let mut rect = NSView::frame(&button);
            rect.origin.x = MACOS_TRAFFIC_LIGHT_X + (index as f64 * space_between);
            button.setFrameOrigin(rect.origin);
        }
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_window_traffic_light_position(_window: &tauri::Window) -> Result<(), String> {
    Ok(())
}

// Engine-aware operations (run_query, DDL, mutations, introspection)
// live in `dispatch.rs`. The `#[tauri::command]` functions in this file
// validate payloads and track activity; the dispatch module owns the
// per-engine match statements. See ADR-0001 and CONTEXT.md's "Engine
// Dispatch" entry.

// Path resolution, SQLite persistence, and lightweight migrations live in
// `storage`. Credential backend routing lives in `credentials`.

async fn public_connections(state: &AppState) -> Result<Vec<StoredConnection>, String> {
    let mut entries = storage::read_connections(&state.pool).await?;
    for entry in entries.iter_mut() {
        entry.set_password(String::new());
    }
    Ok(entries)
}

async fn current_credential_mode(state: &AppState) -> Result<CredentialStorageMode, String> {
    credentials::credential_mode(&state.pool)
        .await?
        .ok_or_else(|| "Credential storage is not configured".to_string())
}

async fn find_connection(
    state: &AppState,
    connection_id: &str,
) -> Result<StoredConnection, String> {
    let mode = current_credential_mode(state).await?;
    let mut connections = storage::read_connections(&state.pool).await?;
    for connection in connections.iter_mut() {
        credentials::hydrate(&state.pool, mode, connection).await?;
    }
    connections
        .into_iter()
        .find(|connection| connection.id() == connection_id)
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
    state: &AppState,
    connection_id: &str,
    op: impl FnOnce(StoredConnection) -> Fut,
) -> Result<T, String>
where
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let connection = find_connection(state, connection_id).await?;
    let result = op(connection).await?;
    touch_connection_activity(state, connection_id).await;
    Ok(result)
}

/// Bump the `lastActivityAt` field on a connection record. Best-effort —
/// failures are logged but never bubble up because activity tracking should
/// not break the underlying operation.
async fn touch_connection_activity(state: &AppState, connection_id: &str) {
    if let Err(error) = storage::touch_connection_activity(&state.pool, connection_id).await {
        eprintln!("Failed to touch lastActivityAt: {error}");
    }
}

#[tauri::command]
async fn load_connections(state: State<'_, AppState>) -> Result<Vec<StoredConnection>, String> {
    public_connections(state.inner()).await
}

#[tauri::command]
async fn save_connection(
    state: State<'_, AppState>,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    storage::upsert_connection(&state.pool, &connection).await?;
    if !connection.password().is_empty() {
        credentials::upsert(&state.pool, mode, &connection).await?;
    }
    public_connections(state).await
}

#[tauri::command]
async fn delete_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    if !storage::delete_connection(&state.pool, &payload.connection_id).await? {
        return Err(format!("Connection '{}' not found", payload.connection_id));
    }
    if let Err(error) = credentials::delete(&state.pool, mode, &payload.connection_id).await {
        eprintln!(
            "Failed to delete credential for {}: {error}",
            payload.connection_id
        );
    }
    public_connections(state).await
}

#[tauri::command]
async fn connect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<ConnectResult, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::ping_connection(&connection).await },
    )
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
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<HealthCheckResult, String> {
    let connection = find_connection(state.inner(), &payload.connection_id).await?;
    match dispatch::ping_connection(&connection).await {
        Ok(result) => Ok(HealthCheckResult::Healthy {
            latency_ms: result.latency_ms,
        }),
        Err(error) => Ok(HealthCheckResult::Error { error }),
    }
}

#[tauri::command]
async fn load_schema_explorer(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<SchemaExplorer>, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::load_schema_explorer(&connection).await },
    )
    .await
}

#[tauri::command]
async fn run_query(
    state: State<'_, AppState>,
    payload: RunQueryPayload,
) -> Result<QueryResult, String> {
    let RunQueryPayload {
        connection_id,
        query,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::run_query(&connection, &query).await
    })
    .await
}

#[tauri::command]
async fn load_table_structure(
    state: State<'_, AppState>,
    payload: LoadTableStructurePayload,
) -> Result<TableStructure, String> {
    let LoadTableStructurePayload {
        connection_id,
        schema,
        table,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::fetch_table_structure(&connection, &schema, &table).await
    })
    .await
}

#[tauri::command]
async fn execute_ddl(
    state: State<'_, AppState>,
    payload: ExecuteDdlPayload,
) -> Result<ExecuteDdlResult, String> {
    let ExecuteDdlPayload { connection_id, sql } = payload;
    if sql.trim().is_empty() {
        return Err("DDL statement is empty".to_string());
    }
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::execute_ddl(&connection, &sql).await
    })
    .await
}

#[tauri::command]
async fn export_ddl(
    state: State<'_, AppState>,
    payload: ExportDdlPayload,
) -> Result<ExportDdlResult, String> {
    let ExportDdlPayload {
        connection_id,
        scope,
        schema,
        table,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::export_ddl(&connection, &scope, schema.as_deref(), table.as_deref()).await
    })
    .await
}

#[tauri::command]
async fn run_pg_dump(
    state: State<'_, AppState>,
    payload: PgDumpPayload,
) -> Result<PgDumpResult, String> {
    let PgDumpPayload {
        connection_id,
        scope,
        schema,
        table,
        format,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::run_pg_dump(
            &connection,
            &scope,
            schema.as_deref(),
            table.as_deref(),
            &format,
        )
        .await
    })
    .await
}

#[tauri::command]
async fn run_pg_restore(
    state: State<'_, AppState>,
    payload: PgRestorePayload,
) -> Result<PgRestoreResult, String> {
    let PgRestorePayload {
        connection_id,
        data_base64,
        format,
        clean,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::run_pg_restore(&connection, &data_base64, &format, clean).await
    })
    .await
}

#[tauri::command]
async fn refresh_materialized_view(
    state: State<'_, AppState>,
    payload: RefreshMaterializedViewPayload,
) -> Result<ExecuteDdlResult, String> {
    let RefreshMaterializedViewPayload {
        connection_id,
        schema,
        view,
        concurrently,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::refresh_materialized_view(&connection, &schema, &view, concurrently).await
    })
    .await
}

#[tauri::command]
async fn run_pg_maintenance(
    state: State<'_, AppState>,
    payload: PgMaintenancePayload,
) -> Result<ExecuteDdlResult, String> {
    let PgMaintenancePayload {
        connection_id,
        schema,
        table,
        action,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::run_maintenance(&connection, &schema, &table, &action).await
    })
    .await
}

#[tauri::command]
async fn commit_cell_edits(
    state: State<'_, AppState>,
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
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::commit_cell_edits(&connection, &schema, &table, &edits).await
    })
    .await
}

#[tauri::command]
async fn insert_row(
    state: State<'_, AppState>,
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
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::insert_row(&connection, &schema, &table, &values).await
    })
    .await
}

#[tauri::command]
async fn import_rows(
    state: State<'_, AppState>,
    payload: ImportRowsPayload,
) -> Result<ImportRowsResult, String> {
    let ImportRowsPayload {
        connection_id,
        schema,
        table,
        columns,
        rows,
        use_copy,
    } = payload;
    if columns.is_empty() {
        return Err("no columns mapped for import".to_string());
    }
    if rows.is_empty() {
        return Ok(ImportRowsResult {
            runtime_ms: 0,
            rows_affected: 0,
        });
    }
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::import_rows(&connection, &schema, &table, &columns, &rows, use_copy).await
    })
    .await
}

#[tauri::command]
async fn copy_table_rows(
    state: State<'_, AppState>,
    payload: CopyTablePayload,
) -> Result<CopyTableResult, String> {
    let CopyTablePayload {
        source_connection_id,
        source_schema,
        source_table,
        destination_connection_id,
        destination_schema,
        destination_table,
        page_size,
    } = payload;
    let source = find_connection(state.inner(), &source_connection_id).await?;
    let destination = find_connection(state.inner(), &destination_connection_id).await?;
    let result = dispatch::copy_table_rows(
        &source,
        &destination,
        &source_schema,
        &source_table,
        &destination_schema,
        &destination_table,
        page_size.unwrap_or(DEFAULT_TABLE_PAGE_SIZE),
    )
    .await?;
    touch_connection_activity(state.inner(), &source_connection_id).await;
    touch_connection_activity(state.inner(), &destination_connection_id).await;
    Ok(result)
}

#[tauri::command]
async fn delete_rows(
    state: State<'_, AppState>,
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
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::delete_rows(&connection, &schema, &table, &rows).await
    })
    .await
}

#[tauri::command]
async fn poll_mutation_status(
    state: State<'_, AppState>,
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
    let connection = find_connection(state.inner(), &connection_id).await?;
    dispatch::poll_mutation_status(&connection, &database, &table, &mutation_ids).await
}

#[tauri::command]
async fn load_schema_relationships(
    state: State<'_, AppState>,
    payload: LoadSchemaRelationshipsPayload,
) -> Result<SchemaRelationships, String> {
    let LoadSchemaRelationshipsPayload {
        connection_id,
        schema,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::fetch_schema_relationships(&connection, &schema).await
    })
    .await
}

#[tauri::command]
async fn load_schema_map_positions(
    state: State<'_, AppState>,
    payload: SchemaMapScopePayload,
) -> Result<Vec<PositionRow>, String> {
    storage::read_schema_map_positions(&state.inner().pool, &payload.connection_id, &payload.schema)
        .await
}

#[tauri::command]
async fn save_schema_map_position(
    state: State<'_, AppState>,
    payload: SaveSchemaMapPositionPayload,
) -> Result<(), String> {
    storage::upsert_schema_map_position(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table_id,
        payload.x,
        payload.y,
    )
    .await
}

#[tauri::command]
async fn reset_schema_map_positions(
    state: State<'_, AppState>,
    payload: SchemaMapScopePayload,
) -> Result<(), String> {
    storage::clear_schema_map_positions(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
    )
    .await
}

#[tauri::command]
async fn load_schema_map_prefs(
    state: State<'_, AppState>,
    payload: SchemaMapScopePayload,
) -> Result<SchemaMapPrefs, String> {
    storage::read_schema_map_prefs(&state.inner().pool, &payload.connection_id, &payload.schema)
        .await
}

#[tauri::command]
async fn save_schema_map_prefs(
    state: State<'_, AppState>,
    payload: SaveSchemaMapPrefsPayload,
) -> Result<SchemaMapPrefs, String> {
    storage::upsert_schema_map_prefs(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        payload.patch,
    )
    .await
}

#[tauri::command]
async fn load_database_overview_stats(
    state: State<'_, AppState>,
    payload: LoadDatabaseOverviewStatsPayload,
) -> Result<DatabaseOverviewStats, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::fetch_database_overview_stats(&connection).await },
    )
    .await
}

#[tauri::command]
async fn load_relation_stats(
    state: State<'_, AppState>,
    payload: LoadRelationStatsPayload,
) -> Result<Vec<RelationInfo>, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::fetch_relation_stats(&connection).await },
    )
    .await
}

#[tauri::command]
async fn load_server_details(
    state: State<'_, AppState>,
    payload: LoadServerDetailsPayload,
) -> Result<ServerDetails, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::fetch_server_details(&connection).await },
    )
    .await
}

#[tauri::command]
async fn load_pg_admin_snapshot(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<PgAdminSnapshot, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::fetch_admin_snapshot(&connection).await },
    )
    .await
}

#[tauri::command]
async fn cancel_pg_backend(
    state: State<'_, AppState>,
    payload: PgBackendActionPayload,
) -> Result<PgBackendActionResult, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::cancel_backend(&connection, payload.pid).await },
    )
    .await
}

#[tauri::command]
async fn terminate_pg_backend(
    state: State<'_, AppState>,
    payload: PgBackendActionPayload,
) -> Result<PgBackendActionResult, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::terminate_backend(&connection, payload.pid).await },
    )
    .await
}

#[tauri::command]
async fn load_table_data(
    state: State<'_, AppState>,
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

    with_active_connection(state.inner(), &connection_id, |connection| async move {
        let qualified = qualified_table_name(&connection.engine(), &schema, &table);

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
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    storage::read_query_history(&state.inner().pool, limit).await
}

#[tauri::command]
async fn append_query_history(
    state: State<'_, AppState>,
    entry: QueryHistoryEntry,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let state = state.inner();
    storage::insert_query_history(&state.pool, &entry).await?;
    storage::read_query_history(&state.pool, Some(MAX_QUERY_HISTORY as u32)).await
}

#[tauri::command]
async fn clear_query_history(state: State<'_, AppState>) -> Result<(), String> {
    storage::clear_query_history(&state.inner().pool).await
}

#[tauri::command]
async fn load_redis_cli_history(
    state: State<'_, AppState>,
    connection_id: String,
    limit: Option<u32>,
) -> Result<Vec<RedisCliHistoryEntry>, String> {
    storage::read_redis_cli_history(&state.inner().pool, &connection_id, limit).await
}

#[tauri::command]
async fn append_redis_cli_history(
    state: State<'_, AppState>,
    entry: RedisCliHistoryEntry,
) -> Result<(), String> {
    storage::insert_redis_cli_history(&state.inner().pool, &entry).await
}

#[tauri::command]
async fn load_saved_queries(state: State<'_, AppState>) -> Result<Vec<SavedQuery>, String> {
    storage::read_saved_queries(&state.inner().pool).await
}

/// Insert or update by `id` (idempotent). Bumps `updatedAt` automatically;
/// callers leave that field as the previous value.
#[tauri::command]
async fn save_saved_query(
    state: State<'_, AppState>,
    query: SavedQuery,
) -> Result<Vec<SavedQuery>, String> {
    let state = state.inner();
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = query.clone();
    next.updated_at = now.clone();
    if query.created_at.is_empty() {
        next.created_at = now;
    }
    storage::upsert_saved_query(&state.pool, &next).await?;
    storage::read_saved_queries(&state.pool).await
}

#[tauri::command]
async fn delete_saved_query(
    state: State<'_, AppState>,
    payload: DeleteSavedQueryPayload,
) -> Result<Vec<SavedQuery>, String> {
    let state = state.inner();
    storage::delete_saved_query(&state.pool, &payload.id).await?;
    storage::read_saved_queries(&state.pool).await
}

#[tauri::command]
async fn load_saved_redis_commands(
    state: State<'_, AppState>,
) -> Result<Vec<SavedRedisCommand>, String> {
    storage::read_saved_redis_commands(&state.inner().pool).await
}

#[tauri::command]
async fn save_saved_redis_command(
    state: State<'_, AppState>,
    command: SavedRedisCommand,
) -> Result<Vec<SavedRedisCommand>, String> {
    let state = state.inner();
    let now = chrono::Utc::now().to_rfc3339();
    let mut next = command.clone();
    next.updated_at = now.clone();
    if command.created_at.is_empty() {
        next.created_at = now;
    }
    storage::upsert_saved_redis_command(&state.pool, &next).await?;
    storage::read_saved_redis_commands(&state.pool).await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteSavedRedisCommandPayload {
    id: String,
}

#[tauri::command]
async fn delete_saved_redis_command(
    state: State<'_, AppState>,
    payload: DeleteSavedRedisCommandPayload,
) -> Result<Vec<SavedRedisCommand>, String> {
    let state = state.inner();
    storage::delete_saved_redis_command(&state.pool, &payload.id).await?;
    storage::read_saved_redis_commands(&state.pool).await
}

// ---------------------------------------------------------------------------
// Redis commands (Phase 1.2)
// ---------------------------------------------------------------------------

#[tauri::command]
async fn redis_scan_keys(
    state: State<'_, AppState>,
    payload: redis::keyspace::ScanKeysPayload,
) -> Result<redis::keyspace::ScanKeysResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::scan_keys(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_open_scan_session(
    state: State<'_, AppState>,
    payload: redis::keyspace::OpenScanSessionPayload,
) -> Result<redis::keyspace::OpenScanSessionResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::open_scan_session(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_cancel_scan_session(
    state: State<'_, AppState>,
    payload: redis::keyspace::CancelScanSessionPayload,
) -> Result<redis::keyspace::CancelScanSessionResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::cancel_scan_session(&connection, &payload).await
    })
    .await
}

#[tauri::command]
fn redis_close_scan_session(payload: redis::keyspace::CloseScanSessionPayload) {
    dispatch::keyvalue::close_scan_session(&payload);
}

#[tauri::command]
async fn redis_fetch_key_metadata(
    state: State<'_, AppState>,
    payload: redis::key_inspector::KeyPayload,
) -> Result<redis::key_inspector::KeyMetadata, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_key_metadata(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_string(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchStringPayload,
) -> Result<redis::key_inspector::StringValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_string(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_hash(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchHashPayload,
) -> Result<redis::key_inspector::HashValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_hash(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_list(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchListPayload,
) -> Result<redis::key_inspector::ListValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_list(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_set(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchSetPayload,
) -> Result<redis::key_inspector::SetValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_set(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_sorted_set(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchSortedSetPayload,
) -> Result<redis::key_inspector::SortedSetValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_sorted_set(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_stream(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchStreamPayload,
) -> Result<redis::key_inspector::StreamValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_stream(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_stream_groups(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchStreamGroupsPayload,
) -> Result<redis::key_inspector::StreamGroupsPayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_stream_groups(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_json(
    state: State<'_, AppState>,
    payload: redis::key_inspector::FetchJsonPayload,
) -> Result<redis::key_inspector::JsonValuePayload, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::fetch_json(&connection, &payload).await
    })
    .await
}

#[tauri::command]
fn redis_cli_close_session(payload: redis::cli::CloseSessionPayload) {
    dispatch::keyvalue::cli_close_session(&payload);
}

#[tauri::command]
async fn redis_run_command(
    state: State<'_, AppState>,
    payload: redis::cli::RunCommandPayload,
) -> Result<redis::cli::RunCommandResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::run_command(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_fetch_overview(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::KeyValueOverviewStats, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_overview(&connection).await },
    )
    .await
}

#[tauri::command]
async fn redis_fetch_client_list(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::ClientListPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_client_list(&connection).await },
    )
    .await
}

#[tauri::command]
async fn redis_fetch_acl_self(
    state: State<'_, AppState>,
    payload: FetchAclSelfPayload,
) -> Result<redis::server_info::AclSelfPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_acl_self(&connection).await },
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchAclSelfPayload {
    connection_id: String,
}

#[tauri::command]
async fn redis_fetch_acl_list(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::AclListPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_acl_list(&connection).await },
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchConfigPayload {
    connection_id: String,
    #[serde(default = "default_star")]
    pattern: String,
}

fn default_star() -> String {
    "*".to_string()
}

#[tauri::command]
async fn redis_fetch_config(
    state: State<'_, AppState>,
    payload: FetchConfigPayload,
) -> Result<redis::server_info::ConfigPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move {
            dispatch::keyvalue::fetch_config(&connection, &payload.pattern).await
        },
    )
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetConfigPayload {
    connection_id: String,
    key: String,
    value: String,
}

#[tauri::command]
async fn redis_set_config(
    state: State<'_, AppState>,
    payload: SetConfigPayload,
) -> Result<(), String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move {
            dispatch::keyvalue::set_config(&connection, &payload.key, &payload.value).await
        },
    )
    .await
}

#[tauri::command]
async fn redis_fetch_latency(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<redis::server_info::LatencyPayload, String> {
    with_active_connection(
        state.inner(),
        &payload.connection_id,
        |connection| async move { dispatch::keyvalue::fetch_latency(&connection).await },
    )
    .await
}

#[tauri::command]
async fn redis_pubsub_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: redis::pubsub::StartSessionPayload,
) -> Result<redis::pubsub::StartSessionResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::pubsub_start(&app, &connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_pubsub_discover(
    state: State<'_, AppState>,
    payload: redis::pubsub::DiscoverChannelsPayload,
) -> Result<redis::pubsub::DiscoverChannelsResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::pubsub_discover(&connection, &payload).await
    })
    .await
}

#[tauri::command]
fn redis_pubsub_drain(payload: redis::pubsub::DrainPayload) -> redis::pubsub::DrainResult {
    dispatch::keyvalue::pubsub_drain(&payload)
}

#[tauri::command]
fn redis_pubsub_close(payload: redis::pubsub::CloseSessionPayload) {
    dispatch::keyvalue::pubsub_close(&payload);
}

#[tauri::command]
async fn redis_pubsub_publish(
    state: State<'_, AppState>,
    payload: redis::pubsub::PublishPayload,
) -> Result<redis::pubsub::PublishResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::pubsub_publish(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_set_string(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetStringPayload,
) -> Result<redis::key_ops::SetStringResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_string(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_set_hash_fields(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetHashFieldsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_hash_fields(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_delete_hash_fields(
    state: State<'_, AppState>,
    payload: redis::key_ops::DeleteHashFieldsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::delete_hash_fields(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_del_keys(
    state: State<'_, AppState>,
    payload: redis::key_ops::DelKeysPayload,
) -> Result<redis::key_ops::DelKeysResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::del_keys(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_bulk_delete_by_pattern(
    state: State<'_, AppState>,
    payload: redis::key_ops::BulkDeleteByPatternPayload,
) -> Result<redis::key_ops::BulkDeleteByPatternResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::bulk_delete_by_pattern(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_bulk_expire_by_pattern(
    state: State<'_, AppState>,
    payload: redis::key_ops::BulkExpireByPatternPayload,
) -> Result<redis::key_ops::BulkExpireByPatternResult, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::bulk_expire_by_pattern(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_set_expire(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetExpirePayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_expire(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_rename_key(
    state: State<'_, AppState>,
    payload: redis::key_ops::RenameKeyPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::rename_key(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_create_key(
    state: State<'_, AppState>,
    payload: redis::key_ops::CreateKeyPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::create_key(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_apply_list_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::ApplyListEditsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_list_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_apply_set_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::SetMembersPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_set_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_apply_sorted_set_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::SortedSetEditsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_sorted_set_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_apply_stream_edits(
    state: State<'_, AppState>,
    payload: redis::key_ops::StreamEditsPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::apply_stream_edits(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_create_stream_group(
    state: State<'_, AppState>,
    payload: redis::key_ops::CreateStreamGroupPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::create_stream_group(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_destroy_stream_group(
    state: State<'_, AppState>,
    payload: redis::key_ops::DestroyStreamGroupPayload,
) -> Result<u64, String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::destroy_stream_group(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_set_json_path(
    state: State<'_, AppState>,
    payload: redis::key_ops::JsonSetPayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::set_json_path(&connection, &payload).await
    })
    .await
}

#[tauri::command]
async fn redis_delete_json_path(
    state: State<'_, AppState>,
    payload: redis::key_ops::JsonDeletePayload,
) -> Result<(), String> {
    let connection_id = payload.connection_id.clone();
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::keyvalue::delete_json_path(&connection, &payload).await
    })
    .await
}

/// Persisted theme keys on the shared `app_settings` table.
const SETTING_THEME: &str = "theme";
const SETTING_THEME_PRESET: &str = "themePreset";

fn validate_theme(value: &str) -> Result<&str, String> {
    match value {
        "system" | "light" | "dark" => Ok(value),
        _ => Err(format!("unknown theme mode '{value}'")),
    }
}

fn validate_theme_preset(value: &str) -> Result<&str, String> {
    match value {
        "default" | "dracula" | "github" | "gruvbox" => Ok(value),
        _ => Err(format!("unknown theme preset '{value}'")),
    }
}

#[tauri::command]
async fn load_app_settings(state: State<'_, AppState>) -> Result<AppSettingsSnapshot, String> {
    let state = state.inner();
    let onboarding_completed = credentials::onboarding_completed(&state.pool).await?;
    let credential_storage_mode = credentials::credential_mode(&state.pool).await?;
    let credential_state = if !onboarding_completed || credential_storage_mode.is_none() {
        CredentialState::NeedsOnboarding
    } else if credential_storage_mode == Some(CredentialStorageMode::EncryptedSqlite)
        && !credentials::is_unlocked()
    {
        CredentialState::NeedsUnlock
    } else {
        CredentialState::Ready
    };
    let theme = storage::get_setting(&state.pool, SETTING_THEME)
        .await?
        .and_then(|raw| validate_theme(&raw).ok().map(str::to_string));
    let theme_preset = storage::get_setting(&state.pool, SETTING_THEME_PRESET)
        .await?
        .and_then(|raw| validate_theme_preset(&raw).ok().map(str::to_string));
    Ok(AppSettingsSnapshot {
        onboarding_completed,
        credential_storage_mode,
        credential_state,
        config_dir: state.paths.config_dir().display().to_string(),
        theme,
        theme_preset,
    })
}

#[tauri::command]
async fn save_app_settings(
    state: State<'_, AppState>,
    payload: SaveAppSettingsPayload,
) -> Result<AppSettingsSnapshot, String> {
    let inner = state.inner();
    if let Some(theme) = payload.theme.as_deref() {
        validate_theme(theme)?;
        storage::set_setting(&inner.pool, SETTING_THEME, theme).await?;
    }
    if let Some(preset) = payload.theme_preset.as_deref() {
        validate_theme_preset(preset)?;
        storage::set_setting(&inner.pool, SETTING_THEME_PRESET, preset).await?;
    }
    load_app_settings(state).await
}

#[tauri::command]
async fn configure_credential_storage(
    state: State<'_, AppState>,
    payload: ConfigureCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    credentials::configure(
        &state.inner().pool,
        payload.mode,
        payload.password.as_deref(),
    )
    .await?;
    load_app_settings(state).await
}

#[tauri::command]
async fn unlock_credentials(
    state: State<'_, AppState>,
    payload: UnlockCredentialsPayload,
) -> Result<AppSettingsSnapshot, String> {
    credentials::unlock(&state.inner().pool, &payload.password).await?;
    load_app_settings(state).await
}

#[tauri::command]
async fn change_credential_storage(
    state: State<'_, AppState>,
    payload: ChangeCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    if !payload.confirm {
        return Err("Credential storage change must be confirmed".to_string());
    }
    let current = current_credential_mode(state.inner()).await?;
    if current == payload.mode {
        return load_app_settings(state).await;
    }
    credentials::change_mode(
        &state.inner().pool,
        current,
        payload.mode,
        payload.password.as_deref(),
    )
    .await?;
    load_app_settings(state).await
}

#[tauri::command]
async fn reset_credential_storage(
    state: State<'_, AppState>,
) -> Result<AppSettingsSnapshot, String> {
    credentials::reset(&state.inner().pool).await?;
    load_app_settings(state).await
}

/// Builds the application-wide logger via `tauri-plugin-log`. Dev
/// targets are stdout (visible in the terminal where `pnpm tauri dev`
/// runs) and the webview console (visible in browser DevTools so
/// frontend developers see backend logs too). Release builds also
/// rotate to a file under the per-OS app log directory
/// (`~/Library/Logs/codes.imran.dbunk/` on macOS,
/// `%APPDATA%/codes.imran.dbunk/logs/` on Windows,
/// `~/.local/share/codes.imran.dbunk/logs/` on Linux) so a shipped
/// build leaves forensic data behind without the user having to
/// re-run from a terminal.
///
/// Level policy:
/// - `dbunk_lib` (this crate) at `debug` in development, `info` in release.
/// - Everything else (dependencies, redis-rs, sqlx, …) at `warn` so we
///   only see their output when something is genuinely off.
///
/// Override at runtime with `RUST_LOG=…` if needed.
fn build_log_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use tauri_plugin_log::{Target, TargetKind};

    let crate_level = if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };

    let mut targets = vec![
        Target::new(TargetKind::Stdout),
        Target::new(TargetKind::Webview),
    ];
    if !cfg!(debug_assertions) {
        // Release-only: dev builds already write to stdout / DevTools
        // and we don't want every `pnpm tauri dev` run to leave files
        // behind in the user's app-log directory.
        targets.push(Target::new(TargetKind::LogDir { file_name: None }));
    }

    tauri_plugin_log::Builder::default()
        .targets(targets)
        .level(log::LevelFilter::Warn)
        .level_for("dbunk_lib", crate_level)
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    dispatch::ensure_sqlx_drivers();
    tauri::Builder::default()
        .plugin(build_log_plugin())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            log::info!("dbunk starting up");
            let paths = Paths::from_app(app.handle())
                .map_err(|error| format!("Failed to resolve config dir: {error}"))?;
            log::info!("config dir: {}", paths.config_dir().display());
            let pool = tauri::async_runtime::block_on(storage::open_pool(&paths))
                .map_err(|error| format!("Failed to open local database: {error}"))?;
            log::info!("SQLite pool ready, migrations applied");
            app.manage(AppState { pool, paths });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            restore_window_traffic_light_position,
            load_app_settings,
            save_app_settings,
            configure_credential_storage,
            unlock_credentials,
            change_credential_storage,
            reset_credential_storage,
            load_connections,
            save_connection,
            delete_connection,
            connect_connection,
            test_connection,
            health_check_connection,
            load_schema_explorer,
            load_schema_relationships,
            load_schema_map_positions,
            save_schema_map_position,
            reset_schema_map_positions,
            load_schema_map_prefs,
            save_schema_map_prefs,
            load_database_overview_stats,
            load_relation_stats,
            load_server_details,
            load_pg_admin_snapshot,
            cancel_pg_backend,
            terminate_pg_backend,
            run_query,
            load_table_data,
            load_table_structure,
            execute_ddl,
            export_ddl,
            run_pg_dump,
            run_pg_restore,
            refresh_materialized_view,
            run_pg_maintenance,
            commit_cell_edits,
            insert_row,
            import_rows,
            copy_table_rows,
            delete_rows,
            poll_mutation_status,
            load_query_history,
            append_query_history,
            clear_query_history,
            load_redis_cli_history,
            append_redis_cli_history,
            load_saved_queries,
            save_saved_query,
            delete_saved_query,
            load_saved_redis_commands,
            save_saved_redis_command,
            delete_saved_redis_command,
            redis_scan_keys,
            redis_open_scan_session,
            redis_cancel_scan_session,
            redis_close_scan_session,
            redis_fetch_key_metadata,
            redis_fetch_string,
            redis_fetch_hash,
            redis_fetch_list,
            redis_fetch_set,
            redis_fetch_sorted_set,
            redis_fetch_stream,
            redis_fetch_stream_groups,
            redis_fetch_json,
            redis_run_command,
            redis_cli_close_session,
            redis_fetch_overview,
            redis_fetch_client_list,
            redis_fetch_acl_list,
            redis_fetch_acl_self,
            redis_fetch_config,
            redis_set_config,
            redis_fetch_latency,
            redis_pubsub_start,
            redis_pubsub_discover,
            redis_pubsub_drain,
            redis_pubsub_close,
            redis_pubsub_publish,
            redis_set_string,
            redis_set_hash_fields,
            redis_delete_hash_fields,
            redis_del_keys,
            redis_bulk_delete_by_pattern,
            redis_bulk_expire_by_pattern,
            redis_set_expire,
            redis_rename_key,
            redis_create_key,
            redis_apply_list_edits,
            redis_apply_set_edits,
            redis_apply_sorted_set_edits,
            redis_apply_stream_edits,
            redis_create_stream_group,
            redis_destroy_stream_group,
            redis_set_json_path,
            redis_delete_json_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
