//! Tauri adapters for the host-neutral settings service.

use crate::settings;
use crate::settings::{DeleteUiStatePayload, SaveUiStatePayload, UiStateEntry};
use crate::{
    AppSettingsSnapshot, AppState, ChangeCredentialStoragePayload,
    ConfigureCredentialStoragePayload, SaveAppSettingsPayload, UnlockCredentialsPayload,
};
use tauri::State;

#[tauri::command]
pub async fn load_app_settings(state: State<'_, AppState>) -> Result<AppSettingsSnapshot, String> {
    settings::load_app_settings(state.inner()).await
}

#[tauri::command]
pub async fn save_app_settings(
    state: State<'_, AppState>,
    payload: SaveAppSettingsPayload,
) -> Result<AppSettingsSnapshot, String> {
    settings::save_app_settings(state.inner(), payload).await
}

#[tauri::command]
pub async fn configure_credential_storage(
    state: State<'_, AppState>,
    payload: ConfigureCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    settings::configure_credential_storage(state.inner(), payload).await
}

#[tauri::command]
pub async fn unlock_credentials(
    state: State<'_, AppState>,
    payload: UnlockCredentialsPayload,
) -> Result<AppSettingsSnapshot, String> {
    settings::unlock_credentials(state.inner(), payload).await
}

#[tauri::command]
pub async fn change_credential_storage(
    state: State<'_, AppState>,
    payload: ChangeCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    settings::change_credential_storage(state.inner(), payload).await
}

#[tauri::command]
pub async fn reset_credential_storage(
    state: State<'_, AppState>,
) -> Result<AppSettingsSnapshot, String> {
    settings::reset_credential_storage(state.inner()).await
}

#[tauri::command]
pub async fn load_ui_state(state: State<'_, AppState>) -> Result<Vec<UiStateEntry>, String> {
    settings::load_ui_state(state.inner()).await
}

#[tauri::command]
pub async fn save_ui_state(
    state: State<'_, AppState>,
    payload: SaveUiStatePayload,
) -> Result<(), String> {
    settings::save_ui_state(state.inner(), payload).await
}

#[tauri::command]
pub async fn delete_ui_state(
    state: State<'_, AppState>,
    payload: DeleteUiStatePayload,
) -> Result<(), String> {
    settings::delete_ui_state(state.inner(), payload).await
}
