//! Admission returns immediately; successful activity/audit belongs to completion.
use tauri::State;

use crate::postgres::backup::{protocol::*, runner::Request};
#[cfg(test)]
use crate::storage;
use crate::AppState;

#[tauri::command]
pub(crate) async fn start_pg_backup(
    state: State<'_, AppState>,
    payload: StartPgBackupPayload,
) -> Result<PgToolJobSnapshot, PgToolJobError> {
    start(state.inner(), Request::Backup(payload)).await
}
#[tauri::command]
pub(crate) async fn start_pg_restore(
    state: State<'_, AppState>,
    payload: StartPgRestorePayload,
) -> Result<PgToolJobSnapshot, PgToolJobError> {
    start(state.inner(), Request::Restore(payload)).await
}

pub(crate) use crate::postgres::backup::service::start;
#[cfg(test)]
use crate::postgres::backup::service::start_with;

#[tauri::command]
pub(crate) async fn get_pg_tool_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<PgToolJobSnapshot, PgToolJobError> {
    state.pg_tool_jobs.get(&job_id)
}
#[tauri::command]
pub(crate) async fn list_pg_tool_jobs(
    state: State<'_, AppState>,
    connection_id: Option<String>,
) -> Result<Vec<PgToolJobSnapshot>, PgToolJobError> {
    Ok(state.pg_tool_jobs.list(connection_id.as_deref()))
}
#[tauri::command]
pub(crate) async fn cancel_pg_tool_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<PgToolJobSnapshot, PgToolJobError> {
    state.pg_tool_jobs.cancel(&job_id)
}
#[tauri::command]
pub(crate) async fn release_pg_tool_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), PgToolJobError> {
    state.pg_tool_jobs.release(&job_id)
}

#[cfg(test)]
mod tests;

/// Native path selection only. The job runner remains the sole file-I/O owner.
#[tauri::command]
pub(crate) async fn pick_pg_tool_file(
    app: tauri::AppHandle,
    kind: PgToolJobKind,
    format: PgBackupFormat,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    match kind {
        PgToolJobKind::Backup => dialog
            .set_title("Choose a new backup destination")
            .set_file_name(match format {
                PgBackupFormat::Plain => "archive.sql",
                PgBackupFormat::Custom => "archive.dump",
            })
            .save_file(move |path| {
                let _ = sender.send(path);
            }),
        PgToolJobKind::Restore => dialog
            .set_title("Choose a PostgreSQL restore source")
            .pick_file(move |path| {
                let _ = sender.send(path);
            }),
    }
    let path = receiver
        .await
        .map_err(|_| "File selection was interrupted".to_string())?;
    path.map(|file| {
        let path = file
            .into_path()
            .map_err(|_| "Select a local file".to_string())?;
        if !path.is_absolute() {
            return Err("Select an absolute local path".to_string());
        }
        path.into_os_string()
            .into_string()
            .map_err(|_| "The selected path is not valid Unicode".to_string())
    })
    .transpose()
}
