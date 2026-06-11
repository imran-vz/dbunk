//! Relational-engine commands: queries, table data, DDL, schema
//! introspection, mutations, history, and saved queries.

use tauri::State;

use crate::dispatch;
use crate::storage;
use crate::{
    parse_total_rows, qualified_table_name, AppState, CommitCellEditsPayload,
    CommitCellEditsResult, ConnectionPayload, CopyTablePayload, CopyTableResult,
    DatabaseOverviewStats, DeleteRowsPayload, DeleteRowsResult, DeleteSavedQueryPayload,
    ExecuteDdlPayload, ExecuteDdlResult, ExportDdlPayload, ExportDdlResult, ImportRowsPayload,
    ImportRowsResult, InsertRowPayload, InsertRowResult, LoadDatabaseOverviewStatsPayload,
    LoadRelationStatsPayload, LoadSchemaRelationshipsPayload, LoadServerDetailsPayload,
    LoadTableDataPayload, LoadTableSchemaRelationshipsPayload, LoadTableStructurePayload,
    MutationStatus, PgAdminSnapshot, PgBackendActionPayload, PgBackendActionResult, PgDumpPayload,
    PgDumpResult, PgMaintenancePayload, PgRestorePayload, PgRestoreResult,
    PollMutationStatusPayload, PositionRow, QueryHistoryEntry, QueryResult,
    RefreshMaterializedViewPayload, RelationInfo, RunQueryPayload, SaveSchemaMapPositionPayload,
    SaveSchemaMapPrefsPayload, SavedQuery, SchemaExplorer, SchemaMapPrefs, SchemaMapScopePayload,
    SchemaRelationships, SeedTablePayload, SeedTableResult, ServerDetails, TableDataResult,
    TableStructure, DEFAULT_TABLE_PAGE_SIZE, MAX_QUERY_HISTORY, MAX_TABLE_PAGE_SIZE,
};

use super::{find_connection, touch_connection_activity, with_active_connection};

// ---------------------------------------------------------------------------
// Schema exploration
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn load_schema_explorer(
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
pub async fn load_schema_relationships(
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
pub async fn load_table_schema_relationships(
    state: State<'_, AppState>,
    payload: LoadTableSchemaRelationshipsPayload,
) -> Result<SchemaRelationships, String> {
    let LoadTableSchemaRelationshipsPayload {
        connection_id,
        schema,
        table,
    } = payload;
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::fetch_table_schema_relationships(&connection, &schema, &table).await
    })
    .await
}

#[tauri::command]
pub async fn load_schema_map_positions(
    state: State<'_, AppState>,
    payload: SchemaMapScopePayload,
) -> Result<Vec<PositionRow>, String> {
    storage::read_schema_map_positions(&state.inner().pool, &payload.connection_id, &payload.schema)
        .await
}

#[tauri::command]
pub async fn save_schema_map_position(
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
pub async fn reset_schema_map_positions(
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
pub async fn load_schema_map_prefs(
    state: State<'_, AppState>,
    payload: SchemaMapScopePayload,
) -> Result<SchemaMapPrefs, String> {
    storage::read_schema_map_prefs(&state.inner().pool, &payload.connection_id, &payload.schema)
        .await
}

#[tauri::command]
pub async fn save_schema_map_prefs(
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

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn run_query(
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
pub async fn load_table_data(
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

// ---------------------------------------------------------------------------
// Table structure + DDL
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn load_table_structure(
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
pub async fn execute_ddl(
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
pub async fn export_ddl(
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
pub async fn run_pg_dump(
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
pub async fn run_pg_restore(
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
pub async fn refresh_materialized_view(
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
pub async fn run_pg_maintenance(
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

// ---------------------------------------------------------------------------
// Data mutations
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn commit_cell_edits(
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
pub async fn insert_row(
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
pub async fn seed_table(
    state: State<'_, AppState>,
    payload: SeedTablePayload,
) -> Result<SeedTableResult, String> {
    let SeedTablePayload {
        connection_id,
        schema,
        table,
        row_count,
        seed,
        columns,
    } = payload;
    // An omitted seed is picked here and echoed back in the result so
    // the run stays reproducible after the fact (ADR-0020).
    let seed = seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5eed_5eed)
    });
    with_active_connection(state.inner(), &connection_id, |connection| async move {
        dispatch::seed_table(&connection, &schema, &table, row_count, seed, &columns).await
    })
    .await
}

#[tauri::command]
pub async fn import_rows(
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
pub async fn copy_table_rows(
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
pub async fn delete_rows(
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
pub async fn poll_mutation_status(
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

// ---------------------------------------------------------------------------
// Overview + server details + admin
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn load_database_overview_stats(
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
pub async fn load_relation_stats(
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
pub async fn load_server_details(
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
pub async fn load_pg_admin_snapshot(
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
pub async fn cancel_pg_backend(
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
pub async fn terminate_pg_backend(
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

// ---------------------------------------------------------------------------
// Query history + saved queries
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn load_query_history(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> Result<Vec<QueryHistoryEntry>, String> {
    storage::read_query_history(&state.inner().pool, limit).await
}

#[tauri::command]
pub async fn append_query_history(
    state: State<'_, AppState>,
    entry: QueryHistoryEntry,
) -> Result<Vec<QueryHistoryEntry>, String> {
    let state = state.inner();
    storage::insert_query_history(&state.pool, &entry).await?;
    storage::read_query_history(&state.pool, Some(MAX_QUERY_HISTORY as u32)).await
}

#[tauri::command]
pub async fn clear_query_history(state: State<'_, AppState>) -> Result<(), String> {
    storage::clear_query_history(&state.inner().pool).await
}

#[tauri::command]
pub async fn load_saved_queries(state: State<'_, AppState>) -> Result<Vec<SavedQuery>, String> {
    storage::read_saved_queries(&state.inner().pool).await
}

/// Insert or update by `id` (idempotent). Bumps `updatedAt` automatically;
/// callers leave that field as the previous value.
#[tauri::command]
pub async fn save_saved_query(
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
pub async fn delete_saved_query(
    state: State<'_, AppState>,
    payload: DeleteSavedQueryPayload,
) -> Result<Vec<SavedQuery>, String> {
    let state = state.inner();
    storage::delete_saved_query(&state.pool, &payload.id).await?;
    storage::read_saved_queries(&state.pool).await
}
