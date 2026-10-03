//! Tauri adapters for `table_browse::service`; wire types and command names stay unchanged.

use tauri::State;

use crate::table_browse::{protocol::*, service};
use crate::AppState;

#[tauri::command]
pub async fn browse_table_data(
    state: State<'_, AppState>,
    payload: BrowseTableDataPayload,
) -> Result<BrowseTableResult, TableBrowseError> {
    service::browse(state.inner(), payload).await
}

#[tauri::command]
pub async fn cancel_table_browse(
    state: State<'_, AppState>,
    payload: TableBrowseTabPayload,
) -> Result<CancelTableBrowseResult, TableBrowseError> {
    service::cancel(state.inner(), payload).await
}

#[tauri::command]
pub async fn count_table_browse_rows(
    state: State<'_, AppState>,
    payload: CountTableBrowseRowsPayload,
) -> Result<BrowseExactCountResult, TableBrowseError> {
    service::count(state.inner(), payload).await
}

#[tauri::command]
pub async fn close_table_browse_for_tab(
    state: State<'_, AppState>,
    payload: TableBrowseTabPayload,
) -> Result<(), TableBrowseError> {
    service::close_tab(state.inner(), payload).await
}

#[tauri::command]
pub async fn load_table_grid_prefs(
    state: State<'_, AppState>,
    payload: LoadTableGridPrefsPayload,
) -> Result<Option<TableGridPrefs>, String> {
    service::load_grid_prefs(state.inner(), payload).await
}

#[tauri::command]
pub async fn save_table_grid_prefs(
    state: State<'_, AppState>,
    payload: SaveTableGridPrefsPayload,
) -> Result<(), String> {
    service::save_grid_prefs(state.inner(), payload).await
}
