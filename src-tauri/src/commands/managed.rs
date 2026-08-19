//! Managed Server commands (ADR-0019): Docker runtime detection,
//! provisioning, lifecycle (start/stop/destroy/recreate), and listing
//! with live Docker-derived status.

use tauri::State;

use crate::{
    docker, managed, AppState, DockerStatus, ManagedServerPayload, ManagedServerWithStatus,
    ProvisionManagedServerPayload, ProvisionManagedServerResult,
};

use super::current_credential_mode;

#[tauri::command]
pub async fn check_docker() -> Result<DockerStatus, String> {
    Ok(docker::status().await)
}

#[tauri::command]
pub async fn provision_managed_server(
    state: State<'_, AppState>,
    payload: ProvisionManagedServerPayload,
) -> Result<ProvisionManagedServerResult, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    managed::provision(&state.pool, mode, payload).await
}

#[tauri::command]
pub async fn list_managed_servers(
    state: State<'_, AppState>,
) -> Result<Vec<ManagedServerWithStatus>, String> {
    managed::list(&state.inner().pool).await
}

#[tauri::command]
pub async fn start_managed_server(
    state: State<'_, AppState>,
    payload: ManagedServerPayload,
) -> Result<(), String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    managed::start(&state.pool, mode, &payload.managed_server_id).await
}

#[tauri::command]
pub async fn stop_managed_server(
    state: State<'_, AppState>,
    payload: ManagedServerPayload,
) -> Result<(), String> {
    let state = state.inner();
    let connection_id = managed_connection_id(state, &payload.managed_server_id).await?;
    if let Some(id) = &connection_id {
        state.query_sessions.begin_connection_teardown(id).await;
        state.table_browse.begin_connection_teardown(id).await;
    }
    let result = managed::stop(&state.pool, &payload.managed_server_id).await;
    if let Some(id) = &connection_id {
        state.query_sessions.end_connection_teardown(id).await;
        state.table_browse.end_connection_teardown(id).await;
    }
    result
}

#[tauri::command]
pub async fn destroy_managed_server(
    state: State<'_, AppState>,
    payload: ManagedServerPayload,
) -> Result<(), String> {
    let state = state.inner();
    let connection_id = managed_connection_id(state, &payload.managed_server_id).await?;
    if let Some(id) = &connection_id {
        state.query_sessions.begin_connection_teardown(id).await;
        state.table_browse.begin_connection_teardown(id).await;
    }
    let result = managed::destroy(&state.pool, &payload.managed_server_id).await;
    if let Some(id) = &connection_id {
        state.query_sessions.end_connection_teardown(id).await;
        state.table_browse.end_connection_teardown(id).await;
    }
    result
}

#[tauri::command]
pub async fn recreate_managed_server(
    state: State<'_, AppState>,
    payload: ManagedServerPayload,
) -> Result<(), String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    let connection_id = managed_connection_id(state, &payload.managed_server_id).await?;
    if let Some(id) = &connection_id {
        state.query_sessions.begin_connection_teardown(id).await;
        state.table_browse.begin_connection_teardown(id).await;
    }
    let result = managed::recreate(&state.pool, mode, &payload.managed_server_id).await;
    if let Some(id) = &connection_id {
        state.query_sessions.end_connection_teardown(id).await;
        state.table_browse.end_connection_teardown(id).await;
    }
    result
}

async fn managed_connection_id(state: &AppState, id: &str) -> Result<Option<String>, String> {
    Ok(
        crate::storage::managed::read_managed_server_by_id(&state.pool, id)
            .await?
            .and_then(|server| server.connection_id),
    )
}
