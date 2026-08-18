use super::{find_connection, touch_connection_activity};
use crate::query_session::protocol::*;
use crate::{AppState, DatabaseEngine};
use tauri::{ipc::Channel, State, Window};

#[tauri::command]
pub async fn register_query_session_owner(
    state: State<'_, AppState>,
    window: Window,
    payload: RegisterOwnerPayload,
) -> Result<RegisterOwnerResult, QuerySessionError> {
    Ok(state
        .query_sessions
        .register_owner(window.label(), payload.owner_id)
        .await)
}
#[tauri::command]
pub async fn open_query_session(
    state: State<'_, AppState>,
    window: Window,
    payload: OpenSessionPayload,
    on_event: Channel<QueryEventEnvelope>,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    let connection = find_connection(state.inner(), &payload.connection_id)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)?;
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(QuerySessionError::UnsupportedEngine);
    }
    let spec =
        crate::postgres::connect_spec::ResolvedPostgresConnectSpec::from_connection(&connection)
            .map_err(|_| QuerySessionError::UnsupportedEngine)?;
    let connection_id = payload.connection_id.clone();
    let result = state
        .query_sessions
        .open(window.label(), payload, on_event, spec)
        .await?;
    touch_connection_activity(state.inner(), &connection_id).await;
    Ok(result)
}
#[tauri::command]
pub async fn execute_query_session(
    state: State<'_, AppState>,
    window: Window,
    payload: ExecutePayload,
) -> Result<AcceptedResult, QuerySessionError> {
    state
        .query_sessions
        .execute(
            &payload.session_id,
            payload.execution_id,
            payload.sql,
            window.label(),
        )
        .await
}
#[tauri::command]
pub async fn ack_query_session_events(
    state: State<'_, AppState>,
    window: Window,
    payload: AckPayload,
) -> Result<(), QuerySessionError> {
    state.query_sessions.ack(payload, window.label()).await
}
#[tauri::command]
pub async fn heartbeat_query_sessions(
    state: State<'_, AppState>,
    window: Window,
    payload: HeartbeatPayload,
) -> Result<HeartbeatResult, QuerySessionError> {
    state
        .query_sessions
        .heartbeat(window.label(), payload)
        .await
}
#[tauri::command]
pub async fn cancel_query_execution(
    state: State<'_, AppState>,
    window: Window,
    payload: ExecutionPayload,
) -> Result<CancelResult, QuerySessionError> {
    state.query_sessions.cancel(payload, window.label()).await
}
#[tauri::command]
pub async fn refresh_query_transaction_state(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    let connection_id = state
        .query_sessions
        .connection_id(&payload.session_id, window.label())
        .await?;
    let connection = find_connection(state.inner(), &connection_id)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)?;
    let spec =
        crate::postgres::connect_spec::ResolvedPostgresConnectSpec::from_connection(&connection)
            .map_err(|_| QuerySessionError::UnsupportedEngine)?;
    state
        .query_sessions
        .refresh(&payload.session_id, window.label(), spec)
        .await
}
#[tauri::command]
pub async fn set_query_transaction_mode(
    state: State<'_, AppState>,
    window: Window,
    payload: SetModePayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .set_mode(&payload.session_id, window.label(), payload.mode)
        .await
}
#[tauri::command]
pub async fn set_query_transaction_isolation(
    state: State<'_, AppState>,
    window: Window,
    payload: SetIsolationPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .set_isolation(
            &payload.session_id,
            window.label(),
            payload.manual_isolation,
        )
        .await
}
#[tauri::command]
pub async fn commit_query_transaction(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .transaction_action(&payload.session_id, window.label(), true)
        .await
}
#[tauri::command]
pub async fn rollback_query_transaction(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .transaction_action(&payload.session_id, window.label(), false)
        .await
}
#[tauri::command]
pub async fn close_query_session(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<(), QuerySessionError> {
    state
        .query_sessions
        .close(&payload.session_id, window.label())
        .await
}
