//! Tauri adapters for `query_session::service`. Each command supplies the
//! window label and, for `open`, wraps the IPC channel as the event sink.

use crate::query_session::protocol::*;
use crate::query_session::service;
use crate::AppState;
use tauri::{ipc::Channel, State, Window};

#[tauri::command]
pub async fn register_query_session_owner(
    state: State<'_, AppState>,
    window: Window,
    payload: RegisterOwnerPayload,
) -> Result<RegisterOwnerResult, QuerySessionError> {
    Ok(service::register_owner(state.inner(), window.label(), payload).await)
}
#[tauri::command]
pub async fn open_query_session(
    state: State<'_, AppState>,
    window: Window,
    payload: OpenSessionPayload,
    on_event: Channel<QueryEventEnvelope>,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    let sink = super::channel_sink(on_event);
    service::open(state.inner(), window.label(), payload, sink).await
}
#[tauri::command]
pub async fn execute_query_session(
    state: State<'_, AppState>,
    window: Window,
    payload: ExecutePayload,
) -> Result<AcceptedResult, QuerySessionError> {
    service::execute(state.inner(), window.label(), payload).await
}
/// Pure: the same scan execution binds with, and no database access.
#[tauri::command]
pub fn describe_query_parameters(
    payload: DescribeParametersPayload,
) -> Result<DescribeParametersResult, QuerySessionError> {
    service::describe_parameters(&payload.sql)
}
#[tauri::command]
pub async fn ack_query_session_events(
    state: State<'_, AppState>,
    window: Window,
    payload: AckPayload,
) -> Result<(), QuerySessionError> {
    service::ack(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn heartbeat_query_sessions(
    state: State<'_, AppState>,
    window: Window,
    payload: HeartbeatPayload,
) -> Result<HeartbeatResult, QuerySessionError> {
    service::heartbeat(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn cancel_query_execution(
    state: State<'_, AppState>,
    window: Window,
    payload: ExecutionPayload,
) -> Result<CancelResult, QuerySessionError> {
    service::cancel(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn refresh_query_transaction_state(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    service::refresh_transaction_state(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn set_query_transaction_mode(
    state: State<'_, AppState>,
    window: Window,
    payload: SetModePayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    service::set_mode(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn set_query_transaction_isolation(
    state: State<'_, AppState>,
    window: Window,
    payload: SetIsolationPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    service::set_isolation(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn commit_query_transaction(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    service::commit(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn rollback_query_transaction(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    service::rollback(state.inner(), window.label(), payload).await
}
#[tauri::command]
pub async fn close_query_session(
    state: State<'_, AppState>,
    window: Window,
    payload: SessionPayload,
) -> Result<(), QuerySessionError> {
    service::close(state.inner(), window.label(), payload).await
}
