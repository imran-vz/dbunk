//! Connection CRUD and health-check commands.

use tauri::State;

use crate::dispatch;
use crate::managed;
use crate::socket_lifecycle;
use crate::storage;
use crate::tunnel;
use crate::{
    AppState, ConnectResult, ConnectionPayload, HealthCheckResult, StoredConnection,
    TestConnectionPayload,
};

use super::{current_credential_mode, find_connection, public_connections, with_active_connection};

#[tauri::command]
pub async fn load_connections(state: State<'_, AppState>) -> Result<Vec<StoredConnection>, String> {
    public_connections(state.inner()).await
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    tunnel::validate_connection_tunnel(&connection)?;
    let engine = connection.engine();
    let connection_id = connection.id().to_string();
    let save_result = socket_lifecycle::with_connection_fence(state, &connection_id, async {
        storage::upsert_connection(&state.pool, &connection).await?;
        crate::credentials::upsert(&state.pool, mode, &connection).await?;
        socket_lifecycle::invalidate_connection_caches(&connection_id, Some(engine));
        Ok::<_, String>(())
    })
    .await;
    save_result?;
    public_connections(state).await
}

#[tauri::command]
pub async fn delete_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    let delete_result =
        socket_lifecycle::with_connection_fence(state, &payload.connection_id, async {
            if !storage::delete_connection(&state.pool, &payload.connection_id).await? {
                return Err(format!("Connection '{}' not found", payload.connection_id));
            }
            if let Err(error) =
                crate::credentials::delete(&state.pool, mode, &payload.connection_id).await
            {
                log::warn!(
                    "Failed to delete credential for {}: {error}",
                    payload.connection_id
                );
            }
            socket_lifecycle::invalidate_connection_caches(&payload.connection_id, None);
            Ok(())
        })
        .await;
    delete_result?;
    public_connections(state).await
}

#[tauri::command]
pub async fn disconnect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<(), String> {
    socket_lifecycle::with_connection_fence(state.inner(), &payload.connection_id, async {
        socket_lifecycle::invalidate_connection_caches(&payload.connection_id, None);
    })
    .await;
    Ok(())
}

#[tauri::command]
pub async fn connect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<ConnectResult, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    with_active_connection(state, &payload.connection_id, |connection| async move {
        managed::ensure_running_for_connection(&state.pool, mode, &connection).await?;
        dispatch::ping_connection(&connection).await
    })
    .await
}

/// Validate credentials without saving them. Used by the New Connection
/// side panel's `Test Connection` button — connects, runs `SELECT 1`,
/// disconnects, returns latency or surfaces the underlying driver error.
#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    payload: TestConnectionPayload,
) -> Result<ConnectResult, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    tunnel::validate_connection_tunnel(&payload.connection)?;
    let route_key = format!("test-{}", uuid::Uuid::new_v4());
    let connection =
        tunnel::resolve_connection(&state.pool, mode, &route_key, &payload.connection).await?;
    let result = dispatch::ping_connection(&connection).await;
    tunnel::drop_connection(&route_key);
    result
}

/// Periodic poll: returns "healthy" + latency or "error" + message. Designed
/// for a frontend interval that fans out across all stored connections — the
/// caller decides cadence and concurrency.
/// Health check is a probe, not a use of the connection — we deliberately do
/// NOT route through `with_active_connection` so a 30 s tick doesn't keep
/// `lastActivityAt` artificially fresh.
#[tauri::command]
pub async fn health_check_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<HealthCheckResult, String> {
    let connection = find_connection(state.inner(), &payload.connection_id).await?;
    match dispatch::ping_connection(&connection).await {
        Ok(result) => Ok(HealthCheckResult::Healthy {
            latency_ms: result.latency_ms,
        }),
        Err(error) => Ok(HealthCheckResult::Error { error }),
    }
}
