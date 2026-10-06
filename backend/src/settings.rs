//! Host-neutral settings and credential lifecycle services.
//! Commands preserve their wire contract and delegate policy and fences here.

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
// Services
// ---------------------------------------------------------------------------

pub async fn load_app_settings(state: &AppState) -> Result<AppSettingsSnapshot, String> {
    let onboarding_completed = credentials::onboarding_completed(&state.pool).await?;
    let credential_storage_mode = credentials::credential_mode(&state.pool).await?;
    let credential_state = if !onboarding_completed || credential_storage_mode.is_none() {
        CredentialState::NeedsOnboarding
    } else if credential_storage_mode == Some(CredentialStorageMode::EncryptedSqlite)
        && !credentials::is_unlocked(&state.credentials)
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

pub async fn save_app_settings(
    state: &AppState,
    payload: SaveAppSettingsPayload,
) -> Result<AppSettingsSnapshot, String> {
    if let Some(theme) = payload.theme.as_deref() {
        validate_theme(theme)?;
        storage::set_setting(&state.pool, SETTING_THEME, theme).await?;
    }
    if let Some(preset) = payload.theme_preset.as_deref() {
        validate_theme_preset(preset)?;
        storage::set_setting(&state.pool, SETTING_THEME_PRESET, preset).await?;
    }
    load_app_settings(state).await
}

pub async fn configure_credential_storage(
    state: &AppState,
    payload: ConfigureCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    configure_credential_storage_inner(state, payload).await?;
    load_app_settings(state).await
}

pub(crate) async fn configure_credential_storage_inner(
    state: &AppState,
    payload: ConfigureCredentialStoragePayload,
) -> Result<(), String> {
    crate::socket_lifecycle::with_global_fence(
        state,
        credentials::configure(
            &state.credentials,
            payload.mode,
            payload.password.as_deref(),
        ),
    )
    .await
}

pub async fn unlock_credentials(
    state: &AppState,
    payload: UnlockCredentialsPayload,
) -> Result<AppSettingsSnapshot, String> {
    credentials::unlock(&state.credentials, &payload.password).await?;
    load_app_settings(state).await
}

pub async fn change_credential_storage(
    state: &AppState,
    payload: ChangeCredentialStoragePayload,
) -> Result<AppSettingsSnapshot, String> {
    change_credential_storage_inner(state, payload).await?;
    load_app_settings(state).await
}

pub(crate) async fn change_credential_storage_inner(
    state: &AppState,
    payload: ChangeCredentialStoragePayload,
) -> Result<(), String> {
    if !payload.confirm {
        return Err("Credential storage change must be confirmed".to_string());
    }
    crate::socket_lifecycle::with_global_fence(state, async {
        let current = crate::app::current_credential_mode(state).await?;
        if current == payload.mode && !rekeys(payload.mode, payload.password.as_deref()) {
            return Ok(());
        }
        credentials::change_mode(
            &state.credentials,
            current,
            payload.mode,
            payload.password.as_deref(),
        )
        .await
    })
    .await
}

/// Staying in Encrypted SQLite with a new password is a password change:
/// same-mode `change_mode` re-encrypts every secret under a fresh verifier and
/// key (atomically for native and fixture profiles). Other same-mode requests
/// remain no-ops.
fn rekeys(mode: CredentialStorageMode, password: Option<&str>) -> bool {
    mode == CredentialStorageMode::EncryptedSqlite
        && password.is_some_and(|value| !value.is_empty())
}

pub async fn reset_credential_storage(state: &AppState) -> Result<AppSettingsSnapshot, String> {
    reset_credential_storage_inner(state).await?;
    load_app_settings(state).await
}

pub(crate) async fn reset_credential_storage_inner(state: &AppState) -> Result<(), String> {
    crate::socket_lifecycle::with_global_fence(state, credentials::reset(&state.credentials)).await
}

// ---------------------------------------------------------------------------
// UI state (P8) — the frontend's namespaced `ui.v1.*` layout/session store
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiStateEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveUiStatePayload {
    pub entries: Vec<UiStateEntry>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteUiStatePayload {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub prefixes: Vec<String>,
}

pub async fn load_ui_state(state: &AppState) -> Result<Vec<UiStateEntry>, String> {
    let entries = storage::read_ui_state(&state.pool).await?;
    Ok(entries
        .into_iter()
        .map(|(key, value)| UiStateEntry { key, value })
        .collect())
}

pub async fn save_ui_state(state: &AppState, payload: SaveUiStatePayload) -> Result<(), String> {
    let entries: Vec<(String, String)> = payload
        .entries
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect();
    storage::upsert_ui_state(&state.pool, &entries).await
}

pub async fn delete_ui_state(
    state: &AppState,
    payload: DeleteUiStatePayload,
) -> Result<(), String> {
    storage::delete_ui_state(&state.pool, &payload.keys, &payload.prefixes).await
}

#[cfg(test)]
mod tests;
