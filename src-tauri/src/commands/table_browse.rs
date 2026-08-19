use tauri::State;

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::table_browse::protocol::*;
use crate::{AppState, DatabaseEngine};

use super::{find_connection, touch_connection_activity};

#[tauri::command]
pub async fn browse_table_data(
    state: State<'_, AppState>,
    payload: BrowseTableDataPayload,
) -> Result<BrowseTableResult, TableBrowseError> {
    let spec = postgres_spec(state.inner(), &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let result = state.inner().table_browse.browse(spec, payload).await?;
    touch_connection_activity(state.inner(), &connection_id).await;
    Ok(result)
}

#[tauri::command]
pub async fn cancel_table_browse(
    state: State<'_, AppState>,
    payload: TableBrowseTabPayload,
) -> Result<CancelTableBrowseResult, TableBrowseError> {
    Ok(state
        .inner()
        .table_browse
        .cancel_tab(&payload.connection_id, &payload.tab_id)
        .await)
}

#[tauri::command]
pub async fn count_table_browse_rows(
    state: State<'_, AppState>,
    payload: CountTableBrowseRowsPayload,
) -> Result<BrowseExactCountResult, TableBrowseError> {
    let spec = postgres_spec(state.inner(), &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let result = state.inner().table_browse.count(spec, payload).await?;
    touch_connection_activity(state.inner(), &connection_id).await;
    Ok(result)
}

#[tauri::command]
pub async fn close_table_browse_for_tab(
    state: State<'_, AppState>,
    payload: TableBrowseTabPayload,
) -> Result<(), TableBrowseError> {
    state
        .inner()
        .table_browse
        .close_tab(&payload.connection_id, &payload.tab_id)
        .await;
    Ok(())
}

#[tauri::command]
pub async fn load_table_grid_prefs(
    state: State<'_, AppState>,
    payload: LoadTableGridPrefsPayload,
) -> Result<Option<TableGridPrefs>, String> {
    crate::storage::read_table_grid_prefs(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
    )
    .await
}

#[tauri::command]
pub async fn save_table_grid_prefs(
    state: State<'_, AppState>,
    payload: SaveTableGridPrefsPayload,
) -> Result<(), String> {
    let prefs = validate_table_grid_prefs(payload.prefs)?;
    crate::storage::upsert_table_grid_prefs(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
        &prefs,
    )
    .await
}

async fn postgres_spec(
    state: &AppState,
    connection_id: &str,
) -> Result<ResolvedPostgresConnectSpec, TableBrowseError> {
    let connection = find_connection(state, connection_id)
        .await
        .map_err(|_| TableBrowseError::ConnectionLost)?;
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(TableBrowseError::UnsupportedEngine);
    }
    ResolvedPostgresConnectSpec::from_connection(&connection)
        .map_err(|_| TableBrowseError::UnsupportedEngine)
}
