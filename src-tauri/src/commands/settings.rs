//! App settings, credential onboarding, and theme commands.

use tauri::State;

use crate::credentials;
use crate::storage;
use crate::{
    AppSettingsSnapshot, AppState, ChangeCredentialStoragePayload,
    ConfigureCredentialStoragePayload, CredentialState, CredentialStorageMode,
    SaveAppSettingsPayload, UnlockCredentialsPayload,
};

// ---------------------------------------------------------------------------
// Theme validation
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn load_app_settings(state: State<'_, AppState>) -> Result<AppSettingsSnapshot, String> {
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
pub async fn save_app_settings(
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
pub async fn configure_credential_storage(
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
pub async fn unlock_credentials(
    state: State<'_, AppState>,
    payload: UnlockCredentialsPayload,
) -> Result<AppSettingsSnapshot, String> {
    credentials::unlock(&state.inner().pool, &payload.password).await?;
    load_app_settings(state).await
}

#[tauri::command]
pub async fn change_credential_storage(
    state: State<'_, AppState>,
    payload: ChangeCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    if !payload.confirm {
        return Err("Credential storage change must be confirmed".to_string());
    }
    let current = super::current_credential_mode(state.inner()).await?;
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
pub async fn reset_credential_storage(
    state: State<'_, AppState>,
) -> Result<AppSettingsSnapshot, String> {
    crate::socket_lifecycle::with_global_fence(state.inner(), async {
        credentials::reset(&state.inner().pool).await
    })
    .await?;
    load_app_settings(state).await
}
