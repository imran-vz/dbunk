//! Tauri adapters for `crate::connections`.

use tauri::State;

use crate::connections::{self, ConnectionOrganizationPayload};
use crate::{AppState, ConnectResult, ConnectionPayload, HealthCheckResult, StoredConnection};

#[tauri::command]
pub async fn load_connections(state: State<'_, AppState>) -> Result<Vec<StoredConnection>, String> {
    connections::list(state.inner()).await
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    connections::save(state.inner(), connection).await
}

#[tauri::command]
pub async fn delete_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    connections::delete(state.inner(), &payload.connection_id).await
}

#[tauri::command]
pub async fn duplicate_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    connections::duplicate(state.inner(), &payload.connection_id).await
}

#[tauri::command]
pub async fn update_connection_organization(
    state: State<'_, AppState>,
    payload: ConnectionOrganizationPayload,
) -> Result<Vec<StoredConnection>, String> {
    connections::update_organization(state.inner(), payload).await
}

#[tauri::command]
pub async fn disconnect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<(), String> {
    connections::disconnect(state.inner(), &payload.connection_id).await
}

#[tauri::command]
pub async fn connect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<ConnectResult, String> {
    connections::connect(state.inner(), &payload.connection_id).await
}

#[tauri::command]
pub async fn health_check_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<HealthCheckResult, String> {
    connections::health_check(state.inner(), &payload.connection_id).await
}
