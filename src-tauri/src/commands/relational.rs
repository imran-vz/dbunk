//! Relational-engine commands: queries, table data, DDL, schema
//! introspection, mutations, history, and saved queries.

use tauri::Emitter;
use tauri::State;

use crate::dispatch;
use crate::safety::policy::{AuditDisposition, WriteIntent};
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

use super::{
    find_connection, touch_connection_activity, with_active_connection,
    with_gated_active_connection,
};

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
    run_query_inner(state.inner(), payload).await
}

pub(crate) async fn run_query_inner(
    state: &AppState,
    payload: RunQueryPayload,
) -> Result<QueryResult, String> {
    let RunQueryPayload {
        connection_id,
        query,
        confirmed,
    } = payload;
    let query_for_policy = query.clone();
    with_gated_active_connection(
        state,
        &connection_id,
        "run_query",
        confirmed,
        move |connection| WriteIntent::Statement {
            classes: if connection.engine() == crate::DatabaseEngine::PostgreSQL {
                crate::postgres::sql_class::classify_script(&query_for_policy)
            } else {
                vec![crate::postgres::sql_class::StatementClass::Unknown]
            },
        },
        |connection| async move { dispatch::run_query(&connection, &query).await },
    )
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
        let structure = dispatch::fetch_table_structure(&connection, &schema, &table)
            .await
            .ok();
        let select_query = dispatch::build_paged_select_query(
            &connection.engine(),
            "*",
            &qualified,
            page_size,
            offset,
            structure.as_ref(),
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
    execute_ddl_inner(state.inner(), payload).await
}

pub(crate) async fn execute_ddl_inner(
    state: &AppState,
    payload: ExecuteDdlPayload,
) -> Result<ExecuteDdlResult, String> {
    let ExecuteDdlPayload {
        connection_id,
        sql,
        confirmed,
    } = payload;
    if sql.trim().is_empty() {
        return Err("DDL statement is empty".to_string());
    }
    with_gated_active_connection(
        state,
        &connection_id,
        "execute_ddl",
        confirmed,
        |_| WriteIntent::Ddl,
        |connection| async move { dispatch::execute_ddl(&connection, &sql).await },
    )
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
    run_pg_restore_inner(state.inner(), payload).await
}

pub(crate) async fn run_pg_restore_inner(
    state: &AppState,
    payload: PgRestorePayload,
) -> Result<PgRestoreResult, String> {
    let PgRestorePayload {
        connection_id,
        data_base64,
        format,
        clean,
        confirmed,
    } = payload;
    with_gated_active_connection(
        state,
        &connection_id,
        "run_pg_restore",
        confirmed,
        |_| WriteIntent::Restore,
        |connection| async move {
            dispatch::run_pg_restore(&connection, &data_base64, &format, clean).await
        },
    )
    .await
}

#[tauri::command]
pub async fn refresh_materialized_view(
    state: State<'_, AppState>,
    payload: RefreshMaterializedViewPayload,
) -> Result<ExecuteDdlResult, String> {
    refresh_materialized_view_inner(state.inner(), payload).await
}

pub(crate) async fn refresh_materialized_view_inner(
    state: &AppState,
    payload: RefreshMaterializedViewPayload,
) -> Result<ExecuteDdlResult, String> {
    let RefreshMaterializedViewPayload {
        connection_id,
        schema,
        view,
        concurrently,
        confirmed,
    } = payload;
    with_gated_active_connection(
        state,
        &connection_id,
        "refresh_materialized_view",
        confirmed,
        |_| WriteIntent::RefreshMatView,
        |connection| async move {
            dispatch::refresh_materialized_view(&connection, &schema, &view, concurrently).await
        },
    )
    .await
}

#[tauri::command]
pub async fn run_pg_maintenance(
    state: State<'_, AppState>,
    payload: PgMaintenancePayload,
) -> Result<ExecuteDdlResult, String> {
    run_pg_maintenance_inner(state.inner(), payload).await
}

pub(crate) async fn run_pg_maintenance_inner(
    state: &AppState,
    payload: PgMaintenancePayload,
) -> Result<ExecuteDdlResult, String> {
    let PgMaintenancePayload {
        connection_id,
        schema,
        table,
        action,
        confirmed,
    } = payload;
    with_gated_active_connection(
        state,
        &connection_id,
        "run_pg_maintenance",
        confirmed,
        |_| WriteIntent::Maintenance,
        |connection| async move {
            dispatch::run_maintenance(&connection, &schema, &table, &action).await
        },
    )
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
    commit_cell_edits_inner(state.inner(), payload).await
}

pub(crate) async fn commit_cell_edits_inner(
    state: &AppState,
    payload: CommitCellEditsPayload,
) -> Result<CommitCellEditsResult, String> {
    let CommitCellEditsPayload {
        connection_id,
        schema,
        table,
        edits,
        confirmed,
    } = payload;
    if edits.is_empty() {
        return Err("no edits to commit".to_string());
    }
    with_gated_active_connection(
        state,
        &connection_id,
        "commit_cell_edits",
        confirmed,
        |_| WriteIntent::RowMutation,
        |connection| async move {
            dispatch::commit_cell_edits(&connection, &schema, &table, &edits).await
        },
    )
    .await
}

#[tauri::command]
pub async fn insert_row(
    state: State<'_, AppState>,
    payload: InsertRowPayload,
) -> Result<InsertRowResult, String> {
    insert_row_inner(state.inner(), payload).await
}

pub(crate) async fn insert_row_inner(
    state: &AppState,
    payload: InsertRowPayload,
) -> Result<InsertRowResult, String> {
    let InsertRowPayload {
        connection_id,
        schema,
        table,
        values,
        confirmed,
    } = payload;
    if values.is_empty() {
        return Err("no values provided".to_string());
    }
    with_gated_active_connection(
        state,
        &connection_id,
        "insert_row",
        confirmed,
        |_| WriteIntent::RowMutation,
        |connection| async move {
            dispatch::insert_row(&connection, &schema, &table, &values).await
        },
    )
    .await
}

#[tauri::command]
pub async fn seed_table(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: SeedTablePayload,
) -> Result<SeedTableResult, String> {
    let operation_id = payload.operation_id.clone();
    let total_rows = u64::from(payload.row_count);
    seed_table_inner(state.inner(), payload, move |rows_completed| {
        if let Err(error) = app.emit(
            "seed-table-progress",
            crate::SeedTableProgress {
                operation_id: operation_id.clone(),
                rows_completed,
                total_rows,
            },
        ) {
            log::warn!("seed progress emit failed: {error}");
        }
    })
    .await
}

pub(crate) async fn seed_table_inner<F>(
    state: &AppState,
    payload: SeedTablePayload,
    report_progress: F,
) -> Result<SeedTableResult, String>
where
    F: Fn(u64) + Send + Sync + 'static,
{
    let SeedTablePayload {
        operation_id: _,
        connection_id,
        schema,
        table,
        row_count,
        seed,
        columns,
        confirmed,
    } = payload;
    // An omitted seed is picked here and echoed back in the result so
    // the run stays reproducible after the fact (ADR-0020).
    let seed = seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5eed_5eed)
    });
    with_gated_active_connection(
        state,
        &connection_id,
        "seed_table",
        confirmed,
        |_| WriteIntent::Seed,
        |connection| async move {
            dispatch::seed_table(
                &connection,
                &schema,
                &table,
                row_count,
                seed,
                &columns,
                report_progress,
            )
            .await
        },
    )
    .await
}

#[tauri::command]
pub async fn import_rows(
    state: State<'_, AppState>,
    payload: ImportRowsPayload,
) -> Result<ImportRowsResult, String> {
    import_rows_inner(state.inner(), payload).await
}

pub(crate) async fn import_rows_inner(
    state: &AppState,
    payload: ImportRowsPayload,
) -> Result<ImportRowsResult, String> {
    let ImportRowsPayload {
        connection_id,
        schema,
        table,
        columns,
        rows,
        use_copy,
        confirmed,
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
    with_gated_active_connection(
        state,
        &connection_id,
        "import_rows",
        confirmed,
        |_| WriteIntent::Import,
        |connection| async move {
            dispatch::import_rows(&connection, &schema, &table, &columns, &rows, use_copy).await
        },
    )
    .await
}

#[tauri::command]
pub async fn copy_table_rows(
    state: State<'_, AppState>,
    payload: CopyTablePayload,
) -> Result<CopyTableResult, String> {
    copy_table_rows_inner(state.inner(), payload).await
}

pub(crate) async fn copy_table_rows_inner(
    state: &AppState,
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
        confirmed,
    } = payload;
    let source = find_connection(state, &source_connection_id).await?;
    let destination = find_connection(state, &destination_connection_id).await?;
    let intent = WriteIntent::CopyDestination;
    let authorization = super::safety::assert_legacy_permitted(&destination, &intent, confirmed)?;
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
    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
        super::safety::record_override(
            &state.pool,
            &destination_connection_id,
            "copy_table_rows",
            &intent,
        )
        .await;
    }
    touch_connection_activity(state, &source_connection_id).await;
    touch_connection_activity(state, &destination_connection_id).await;
    Ok(result)
}

#[tauri::command]
pub async fn delete_rows(
    state: State<'_, AppState>,
    payload: DeleteRowsPayload,
) -> Result<DeleteRowsResult, String> {
    delete_rows_inner(state.inner(), payload).await
}

pub(crate) async fn delete_rows_inner(
    state: &AppState,
    payload: DeleteRowsPayload,
) -> Result<DeleteRowsResult, String> {
    let DeleteRowsPayload {
        connection_id,
        schema,
        table,
        rows,
        confirmed,
    } = payload;
    if rows.is_empty() {
        return Err("no rows provided".to_string());
    }
    with_gated_active_connection(
        state,
        &connection_id,
        "delete_rows",
        confirmed,
        |_| WriteIntent::RowMutation,
        |connection| async move {
            dispatch::delete_rows(&connection, &schema, &table, &rows).await
        },
    )
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
    cancel_pg_backend_inner(state.inner(), payload).await
}

pub(crate) async fn cancel_pg_backend_inner(
    state: &AppState,
    payload: PgBackendActionPayload,
) -> Result<PgBackendActionResult, String> {
    with_gated_active_connection(
        state,
        &payload.connection_id,
        "cancel_pg_backend",
        payload.confirmed,
        |_| WriteIntent::CancelBackend,
        |connection| async move { dispatch::cancel_backend(&connection, payload.pid).await },
    )
    .await
}

#[tauri::command]
pub async fn terminate_pg_backend(
    state: State<'_, AppState>,
    payload: PgBackendActionPayload,
) -> Result<PgBackendActionResult, String> {
    terminate_pg_backend_inner(state.inner(), payload).await
}

pub(crate) async fn terminate_pg_backend_inner(
    state: &AppState,
    payload: PgBackendActionPayload,
) -> Result<PgBackendActionResult, String> {
    with_gated_active_connection(
        state,
        &payload.connection_id,
        "terminate_pg_backend",
        payload.confirmed,
        |_| WriteIntent::TerminateBackend,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CellEdit, CellEditKeyValue, Environment, MySqlStoredConnection, SafeMode, SshTunnelConfig,
        StoredConnection,
    };

    #[derive(Debug, Clone, Copy)]
    enum LegacyCommand {
        RunQuery,
        ExecuteDdl,
        RunPgRestore,
        RefreshMaterializedView,
        RunPgMaintenance,
        CommitCellEdits,
        InsertRow,
        SeedTable,
        ImportRows,
        CopyTableRows,
        DeleteRows,
        TerminatePgBackend,
    }

    impl LegacyCommand {
        const ALL: [Self; 12] = [
            Self::RunQuery,
            Self::ExecuteDdl,
            Self::RunPgRestore,
            Self::RefreshMaterializedView,
            Self::RunPgMaintenance,
            Self::CommitCellEdits,
            Self::InsertRow,
            Self::SeedTable,
            Self::ImportRows,
            Self::CopyTableRows,
            Self::DeleteRows,
            Self::TerminatePgBackend,
        ];

        fn name(self) -> &'static str {
            match self {
                Self::RunQuery => "run_query",
                Self::ExecuteDdl => "execute_ddl",
                Self::RunPgRestore => "run_pg_restore",
                Self::RefreshMaterializedView => "refresh_materialized_view",
                Self::RunPgMaintenance => "run_pg_maintenance",
                Self::CommitCellEdits => "commit_cell_edits",
                Self::InsertRow => "insert_row",
                Self::SeedTable => "seed_table",
                Self::ImportRows => "import_rows",
                Self::CopyTableRows => "copy_table_rows",
                Self::DeleteRows => "delete_rows",
                Self::TerminatePgBackend => "terminate_pg_backend",
            }
        }

        async fn call(
            self,
            state: &AppState,
            connection_id: &str,
            confirmed: bool,
        ) -> Result<(), String> {
            let identity = || CellEditKeyValue {
                column: "id".into(),
                value: Some("1".into()),
            };
            match self {
                Self::RunQuery => run_query_inner(
                    state,
                    RunQueryPayload {
                        connection_id: connection_id.into(),
                        query: "UPDATE items SET active = TRUE WHERE id = 1".into(),
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::ExecuteDdl => execute_ddl_inner(
                    state,
                    ExecuteDdlPayload {
                        connection_id: connection_id.into(),
                        sql: "CREATE TABLE command_gate_probe (id INTEGER)".into(),
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::RunPgRestore => run_pg_restore_inner(
                    state,
                    PgRestorePayload {
                        connection_id: connection_id.into(),
                        data_base64: String::new(),
                        format: "plain".into(),
                        clean: false,
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::RefreshMaterializedView => refresh_materialized_view_inner(
                    state,
                    RefreshMaterializedViewPayload {
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        view: "command_gate_probe".into(),
                        concurrently: false,
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::RunPgMaintenance => run_pg_maintenance_inner(
                    state,
                    PgMaintenancePayload {
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        table: "command_gate_probe".into(),
                        action: "analyze".into(),
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::CommitCellEdits => commit_cell_edits_inner(
                    state,
                    CommitCellEditsPayload {
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        table: "command_gate_probe".into(),
                        edits: vec![CellEdit {
                            row_index: 0,
                            identity: vec![identity()],
                            set: vec![CellEditKeyValue {
                                column: "active".into(),
                                value: Some("true".into()),
                            }],
                        }],
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::InsertRow => insert_row_inner(
                    state,
                    InsertRowPayload {
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        table: "command_gate_probe".into(),
                        values: vec![identity()],
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::SeedTable => seed_table_inner(
                    state,
                    SeedTablePayload {
                        operation_id: "command-gate-probe".into(),
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        table: "command_gate_probe".into(),
                        row_count: 1,
                        seed: Some(7),
                        columns: Vec::new(),
                        confirmed,
                    },
                    |_| {},
                )
                .await
                .map(|_| ()),
                Self::ImportRows => import_rows_inner(
                    state,
                    ImportRowsPayload {
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        table: "command_gate_probe".into(),
                        columns: vec!["id".into()],
                        rows: vec![vec![Some("1".into())]],
                        use_copy: false,
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::CopyTableRows => copy_table_rows_inner(
                    state,
                    CopyTablePayload {
                        source_connection_id: SOURCE_ID.into(),
                        source_schema: "public".into(),
                        source_table: "command_gate_source".into(),
                        destination_connection_id: connection_id.into(),
                        destination_schema: "public".into(),
                        destination_table: "command_gate_probe".into(),
                        page_size: Some(1),
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::DeleteRows => delete_rows_inner(
                    state,
                    DeleteRowsPayload {
                        connection_id: connection_id.into(),
                        schema: "public".into(),
                        table: "command_gate_probe".into(),
                        rows: vec![vec![identity()]],
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
                Self::TerminatePgBackend => terminate_pg_backend_inner(
                    state,
                    PgBackendActionPayload {
                        connection_id: connection_id.into(),
                        pid: 1,
                        confirmed,
                    },
                )
                .await
                .map(|_| ()),
            }
        }
    }

    const SOURCE_ID: &str = "legacy-command-source";

    fn mysql_connection(id: &str, environment: Environment) -> StoredConnection {
        StoredConnection::MySQL(MySqlStoredConnection {
            organization: Default::default(),
            id: id.into(),
            name: id.into(),
            database: "dbunk_demo".into(),
            host: "127.0.0.1".into(),
            port: 1,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: "read/write".into(),
            environment,
            safe_mode: SafeMode::Inherit,
            read_only: false,
            last_activity_at: None,
            ssl: false,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    async fn assert_no_activity_or_audit(state: &AppState, connection_id: &str, command: &str) {
        let connection = storage::read_connection_by_id(&state.pool, connection_id)
            .await
            .expect("read stored connection")
            .expect("connection exists");
        assert!(
            connection.last_activity_at().is_none(),
            "{command} unexpectedly bumped lastActivityAt"
        );
        assert!(
            storage::read_safety_overrides(&state.pool, connection_id)
                .await
                .expect("read safety audits")
                .is_empty(),
            "{command} unexpectedly recorded an audit"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn every_legacy_command_core_refuses_before_dispatch_and_confirmed_reaches_dispatch() {
        let (_directory, state) = crate::test_app_state().await;
        let connection_id = "legacy-command-strict";
        crate::commands::connections::save_connection_inner(
            &state,
            mysql_connection(SOURCE_ID, Environment::Development),
        )
        .await
        .expect("save source");
        crate::commands::connections::save_connection_inner(
            &state,
            mysql_connection(connection_id, Environment::Production),
        )
        .await
        .expect("save strict connection");

        for command in LegacyCommand::ALL {
            let name = command.name();
            let refusal = command
                .call(&state, connection_id, false)
                .await
                .expect_err("strict command unexpectedly passed unconfirmed");
            assert_eq!(
                refusal.get(..crate::safety::CONFIRM_TAG.len()),
                Some(crate::safety::CONFIRM_TAG),
                "{name} returned the wrong confirmation tag: {refusal}"
            );
            assert_no_activity_or_audit(&state, connection_id, name).await;

            let dispatch_error = command
                .call(&state, connection_id, true)
                .await
                .expect_err("the test connection must fail after the policy gate");
            assert!(
                !dispatch_error.starts_with(crate::safety::CONFIRM_TAG)
                    && !dispatch_error.starts_with(crate::safety::READ_ONLY_TAG),
                "{name} did not pass the confirmed policy gate: {dispatch_error}"
            );
            assert_no_activity_or_audit(&state, connection_id, name).await;
        }

        assert_no_activity_or_audit(&state, SOURCE_ID, "copy_table_rows source").await;
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn cancel_command_core_remains_ungated() {
        let (_directory, state) = crate::test_app_state().await;
        let connection_id = "cancel-command-strict";
        crate::commands::connections::save_connection_inner(
            &state,
            mysql_connection(connection_id, Environment::Production),
        )
        .await
        .expect("save strict connection");

        let dispatch_error = cancel_pg_backend_inner(
            &state,
            PgBackendActionPayload {
                connection_id: connection_id.into(),
                pid: 1,
                confirmed: false,
            },
        )
        .await
        .expect_err("the unsupported engine must reject after command admission");
        assert!(
            !dispatch_error.starts_with(crate::safety::CONFIRM_TAG)
                && !dispatch_error.starts_with(crate::safety::READ_ONLY_TAG)
        );
        assert_no_activity_or_audit(&state, connection_id, "cancel_pg_backend").await;
    }
}
