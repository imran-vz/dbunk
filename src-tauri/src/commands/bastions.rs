//! Bastion Server CRUD and testing commands.

use std::collections::HashMap;

use tauri::State;

use crate::credentials;
use crate::postgres;
use crate::redis;
use crate::storage;
use crate::tunnel;
use crate::{
    AppState, BastionAuthMethod, BastionServer, BastionServerPayload, CredentialStorageMode,
    PublicBastionServer, SaveBastionServerPayload, SecretChange, TestBastionResult,
};

use super::current_credential_mode;

#[tauri::command]
pub async fn load_bastion_servers(
    state: State<'_, AppState>,
) -> Result<Vec<PublicBastionServer>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    public_bastion_servers(state, mode).await
}

#[tauri::command]
pub async fn save_bastion_server(
    state: State<'_, AppState>,
    payload: SaveBastionServerPayload,
) -> Result<Vec<PublicBastionServer>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    validate_bastion_payload(&payload)?;

    let existing = storage::bastions::read_bastion_server_by_id(&state.pool, &payload.id).await?;
    let current_secrets = credentials::read_all(&state.pool, mode).await?;
    let secret_patch = secret_patch_from_payload(&payload);
    let next_secrets = credentials::apply_bastion_secret_patch(current_secrets, &secret_patch);
    validate_required_secret(&next_secrets, &payload)?;
    let now = storage::now();
    let bastion = BastionServer {
        id: payload.id.clone(),
        name: payload.name.trim().to_string(),
        host: payload.host.trim().to_string(),
        port: payload.port,
        user: payload.user.trim().to_string(),
        auth_method: payload.auth_method,
        private_key_path: payload
            .private_key_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        host_key_fingerprint: existing
            .as_ref()
            .and_then(|server| server.host_key_fingerprint.clone()),
        created_at: existing
            .as_ref()
            .map(|server| server.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
    };

    storage::bastions::upsert_bastion_server(&state.pool, &bastion).await?;
    if let Err(error) = credentials::write_all(&state.pool, mode, &next_secrets).await {
        let rollback = rollback_bastion_metadata(&state.pool, existing.as_ref(), &payload.id).await;
        if let Err(rollback_error) = rollback {
            return Err(format!(
                "Failed to save Bastion secrets: {error}; metadata rollback failed: {rollback_error}"
            ));
        }
        return Err(format!("Failed to save Bastion secrets: {error}"));
    }
    invalidate_referenced_connections(state, &payload.id).await?;
    public_bastion_servers(state, mode).await
}

#[tauri::command]
pub async fn delete_bastion_server(
    state: State<'_, AppState>,
    payload: BastionServerPayload,
) -> Result<Vec<PublicBastionServer>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    let refs = storage::bastions::count_connections_referencing_bastion(
        &state.pool,
        &payload.bastion_server_id,
    )
    .await?;
    if refs > 0 {
        return Err(format!(
            "Cannot delete Bastion Server while {refs} Connection(s) reference it"
        ));
    }
    if !storage::bastions::delete_bastion_server(&state.pool, &payload.bastion_server_id).await? {
        return Err(format!(
            "Bastion Server '{}' not found",
            payload.bastion_server_id
        ));
    }
    credentials::delete_bastion_secrets(&state.pool, mode, &payload.bastion_server_id).await?;
    tunnel::drop_bastion(&payload.bastion_server_id);
    public_bastion_servers(state, mode).await
}

#[tauri::command]
pub async fn reset_bastion_host_key(
    state: State<'_, AppState>,
    payload: BastionServerPayload,
) -> Result<Vec<PublicBastionServer>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    storage::bastions::update_bastion_host_key_fingerprint(
        &state.pool,
        &payload.bastion_server_id,
        None,
    )
    .await?;
    invalidate_referenced_connections(state, &payload.bastion_server_id).await?;
    public_bastion_servers(state, mode).await
}

#[tauri::command]
pub async fn test_bastion_server(
    state: State<'_, AppState>,
    payload: BastionServerPayload,
) -> Result<TestBastionResult, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    tunnel::test_bastion(&state.pool, mode, &payload.bastion_server_id).await
}

async fn public_bastion_servers(
    state: &AppState,
    mode: CredentialStorageMode,
) -> Result<Vec<PublicBastionServer>, String> {
    let servers = storage::bastions::read_bastion_servers(&state.pool).await?;
    let all_secrets = credentials::read_all(&state.pool, mode).await?;
    Ok(servers
        .into_iter()
        .map(|server| public_bastion(server, &all_secrets))
        .collect())
}

fn public_bastion(server: BastionServer, secrets: &HashMap<String, String>) -> PublicBastionServer {
    let id = server.id.clone();
    let has_secret = |slot: &str| secrets.contains_key(&credentials::bastion_secret_id(&id, slot));
    PublicBastionServer {
        id: server.id,
        name: server.name,
        host: server.host,
        port: server.port,
        user: server.user,
        auth_method: server.auth_method,
        private_key_path: server.private_key_path,
        host_key_fingerprint: server.host_key_fingerprint,
        created_at: server.created_at,
        updated_at: server.updated_at,
        has_password: has_secret("password"),
        has_private_key_content: has_secret("privateKeyContent"),
        has_passphrase: has_secret("passphrase"),
    }
}

fn validate_bastion_payload(payload: &SaveBastionServerPayload) -> Result<(), String> {
    if payload.id.trim().is_empty() {
        return Err("Bastion Server id is required".to_string());
    }
    if payload.name.trim().is_empty() {
        return Err("Bastion Server name is required".to_string());
    }
    if payload.host.trim().is_empty() {
        return Err("Bastion Server host is required".to_string());
    }
    if payload.port == 0 {
        return Err("Bastion Server port must be between 1 and 65535".to_string());
    }
    if payload.user.trim().is_empty() {
        return Err("Bastion Server username is required".to_string());
    }
    if matches!(payload.auth_method, BastionAuthMethod::PrivateKeyPath)
        && payload
            .private_key_path
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
    {
        return Err("Private key path is required".to_string());
    }
    Ok(())
}

fn validate_required_secret(
    secrets: &HashMap<String, String>,
    payload: &SaveBastionServerPayload,
) -> Result<(), String> {
    match payload.auth_method {
        BastionAuthMethod::Password => validate_secret_present(
            secrets,
            &payload.id,
            "password",
            "Bastion password is required",
        ),
        BastionAuthMethod::PrivateKeyPath => Ok(()),
        BastionAuthMethod::PrivateKeyContent => validate_secret_present(
            secrets,
            &payload.id,
            "privateKeyContent",
            "Private key content is required",
        ),
    }
}

fn validate_secret_present(
    secrets: &HashMap<String, String>,
    bastion_id: &str,
    slot: &str,
    message: &str,
) -> Result<(), String> {
    if credentials::bastion_secret_present(secrets, bastion_id, slot) {
        return Ok(());
    }
    Err(message.to_string())
}

fn secret_patch_from_payload(
    payload: &SaveBastionServerPayload,
) -> credentials::BastionSecretPatch {
    credentials::BastionSecretPatch {
        bastion_id: payload.id.clone(),
        auth_method: payload.auth_method,
        password: nonblank_secret_change(&payload.password),
        private_key_content: nonblank_secret_change(&payload.private_key_content),
        passphrase: nonblank_secret_change(&payload.passphrase),
    }
}

fn nonblank_secret_change(change: &SecretChange) -> SecretChange {
    match change {
        SecretChange::Set { value } if value.trim().is_empty() => SecretChange::Clear,
        _ => change.clone(),
    }
}

async fn rollback_bastion_metadata(
    pool: &sqlx::SqlitePool,
    existing: Option<&BastionServer>,
    bastion_id: &str,
) -> Result<(), String> {
    if let Some(existing) = existing {
        storage::bastions::upsert_bastion_server(pool, existing).await
    } else {
        storage::bastions::delete_bastion_server(pool, bastion_id)
            .await
            .map(|_| ())
    }
}

async fn invalidate_referenced_connections(
    state: &AppState,
    bastion_id: &str,
) -> Result<(), String> {
    let connection_ids =
        storage::bastions::connection_ids_referencing_bastion(&state.pool, bastion_id).await?;
    tunnel::drop_bastion(bastion_id);
    for connection_id in connection_ids {
        postgres::drop_pool(&connection_id);
        redis::connection::drop_cached(&connection_id);
        tunnel::drop_connection(&connection_id);
    }
    Ok(())
}
