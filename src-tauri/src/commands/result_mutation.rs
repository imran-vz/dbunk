//! Tauri adapters for `result_mutation::service`; wire types and command names stay unchanged.

use tauri::State;

use crate::result_mutation::{protocol::*, service};
use crate::AppState;

#[tauri::command]
pub async fn analyze_result_set(
    state: State<'_, AppState>,
    payload: AnalyzeResultSetPayload,
) -> Result<AnalyzeResultSetResult, ResultMutationError> {
    service::analyze(state.inner(), payload).await
}

#[tauri::command]
pub async fn preview_result_mutations(
    state: State<'_, AppState>,
    payload: PreviewResultMutationsPayload,
) -> Result<PreviewResult, ResultMutationError> {
    service::preview(state.inner(), payload).await
}

#[tauri::command]
pub async fn apply_result_mutations(
    state: State<'_, AppState>,
    payload: ApplyResultMutationsPayload,
) -> Result<ApplyResult, ResultMutationError> {
    service::apply(state.inner(), payload).await
}

#[tauri::command]
pub async fn cancel_result_mutation(
    state: State<'_, AppState>,
    payload: CancelResultMutationPayload,
) -> Result<CancelResultMutationResult, ResultMutationError> {
    service::cancel(state.inner(), payload).await
}

#[tauri::command]
pub async fn close_result_mutation_for_connection(
    state: State<'_, AppState>,
    payload: CloseResultMutationPayload,
) -> Result<(), ResultMutationError> {
    service::close_connection(state.inner(), payload).await
}

#[tauri::command]
pub async fn load_virtual_key(
    state: State<'_, AppState>,
    payload: LoadVirtualKeyPayload,
) -> Result<Option<VirtualKey>, ResultMutationError> {
    service::load_virtual_key(state.inner(), payload).await
}

#[tauri::command]
pub async fn save_virtual_key(
    state: State<'_, AppState>,
    payload: SaveVirtualKeyPayload,
) -> Result<(), ResultMutationError> {
    service::save_virtual_key(state.inner(), payload).await
}

#[tauri::command]
pub async fn clear_virtual_key(
    state: State<'_, AppState>,
    payload: ClearVirtualKeyPayload,
) -> Result<(), ResultMutationError> {
    service::clear_virtual_key(state.inner(), payload).await
}
