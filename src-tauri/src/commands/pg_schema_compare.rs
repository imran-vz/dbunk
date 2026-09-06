//! Dark native command boundary. Starts return admission; only completed jobs
//! expose immutable results. The caller request ID reconciles uncertain starts.
use crate::{postgres::schema_compare::protocol::*, AppState};
use tauri::{State, WebviewWindow};

#[tauri::command]
pub(crate) async fn start_pg_schema_compare(
    state: State<'_, AppState>,
    payload: StartRequest,
) -> Result<Status, CompareError> {
    state
        .pg_schema_compare
        .start_native(payload, state.pool.clone())
}
#[tauri::command]
pub(crate) async fn list_pg_schema_compares(
    state: State<'_, AppState>,
) -> Result<Vec<Status>, CompareError> {
    Ok(state.pg_schema_compare.list())
}
#[tauri::command]
pub(crate) async fn get_pg_schema_compare(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<Status, CompareError> {
    state.pg_schema_compare.get(&job_id)
}
#[tauri::command]
pub(crate) async fn cancel_pg_schema_compare(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<Status, CompareError> {
    state.pg_schema_compare.cancel(&job_id)
}
#[tauri::command]
pub(crate) async fn release_pg_schema_compare(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), CompareError> {
    state.pg_schema_compare.release(&job_id)
}

#[tauri::command]
pub(crate) fn get_pg_schema_compare_transport(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<String, CompareError> {
    state.pg_schema_compare.transport(window.label())
}

#[tauri::command]
pub(crate) async fn read_pg_schema_compare(
    state: State<'_, AppState>,
    window: WebviewWindow,
    transport: String,
    response_id: String,
    request: ResultRequest,
    read: ReadRequest,
) -> Result<tauri::ipc::Response, CompareError> {
    let mut body = None;
    state.pg_schema_compare.read(
        window.label(),
        &transport,
        &response_id,
        &request,
        read,
        |json| body = Some(json),
    )?;
    // Response::new moves already-capped JSON directly into Tauri's body.
    // The manager retains the serializer lease after this command returns.
    Ok(tauri::ipc::Response::new(
        body.ok_or(CompareError::Unavailable)?,
    ))
}
#[tauri::command]
pub(crate) async fn acknowledge_pg_schema_compare(
    state: State<'_, AppState>,
    window: WebviewWindow,
    transport: String,
    response_id: String,
) -> Result<(), CompareError> {
    state
        .pg_schema_compare
        .acknowledge(window.label(), &transport, &response_id)
}
