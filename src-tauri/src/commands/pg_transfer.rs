//! CSV commands return admission, never claim database or file success at start.
use crate::postgres::transfer::{
    protocol::*,
    runner::{self, RunRequest},
};
use crate::safety::policy::{assert_permitted, AuditDisposition, WriteIntent};
use crate::{storage, AppState, DatabaseEngine, StoredConnection};
use std::time::Duration;
use tauri::State;
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(30);

fn identifier(value: &str, field: &str) -> Result<(), TransferError> {
    if value.is_empty() || value.len() > 1024 || value.contains('\0') {
        return Err(TransferError::invalid(field, "Invalid identifier"));
    }
    Ok(())
}
fn token(value: &str) -> Result<(), TransferError> {
    if value.is_empty() || value.len() > 128 {
        return Err(TransferError::InspectionExpired);
    }
    Ok(())
}
fn path(value: &str) -> Result<(), TransferError> {
    if value.len() > 4096 || value.contains('\0') || !std::path::Path::new(value).is_absolute() {
        return Err(TransferError::invalid(
            "path",
            "Choose an absolute local file path",
        ));
    }
    Ok(())
}
async fn connection(state: &AppState, id: &str) -> Result<StoredConnection, TransferError> {
    let stored = storage::read_connection_by_id(&state.pool, id)
        .await
        .map_err(|_| TransferError::invalid("connectionId", "Unable to load connection"))?
        .ok_or_else(|| TransferError::invalid("connectionId", "Connection not found"))?;
    if stored.engine() != DatabaseEngine::PostgreSQL {
        return Err(TransferError::UnsupportedEngine);
    }
    super::find_connection(state, id).await.map_err(|_| {
        TransferError::invalid("connectionId", "Unable to resolve credentials or tunnel")
    })
}
#[tauri::command]
pub(crate) async fn inspect_pg_transfer(
    state: State<'_, AppState>,
    payload: InspectPayload,
) -> Result<Inspection, TransferError> {
    inspect(state.inner(), payload).await
}
pub(crate) async fn inspect(
    state: &AppState,
    payload: InspectPayload,
) -> Result<Inspection, TransferError> {
    identifier(&payload.schema, "schema")?;
    identifier(&payload.table, "table")?;
    payload.options.validate().map_err(|_| {
        TransferError::invalid(
            "options",
            "Invalid CSV delimiter, quote, escape or NULL token",
        )
    })?;
    match (&payload.source_path, payload.direction) {
        (Some(p), Direction::Import) => path(p)?,
        (None, Direction::Export) => {}
        _ => {
            return Err(TransferError::invalid(
                "sourcePath",
                "Import requires a source file; export does not",
            ))
        }
    }
    let mut admission = state.pg_transfers.admission(&payload.connection_id)?;
    let work = async {
        let conn = connection(state, &payload.connection_id).await?;
        runner::inspect(conn, payload).await
    };
    let mut work = Box::pin(tokio::time::timeout(PREPARATION_TIMEOUT, work));
    let reviewed = tokio::select! {biased;_=admission.cancelled()=>Err(TransferError::ConnectionClosing),result=&mut work=>result.map_err(|_|TransferError::Timeout{operation:"inspection".into()})?};
    drop(work);
    state.pg_transfers.insert_review(&admission, reviewed?)
}
#[tauri::command]
pub(crate) async fn release_pg_transfer_inspection(
    state: State<'_, AppState>,
    inspection_token: String,
) -> Result<(), TransferError> {
    token(&inspection_token)?;
    state.pg_transfers.release_review(&inspection_token);
    Ok(())
}
#[tauri::command]
pub(crate) async fn start_pg_csv_import(
    state: State<'_, AppState>,
    payload: StartImportPayload,
) -> Result<Snapshot, TransferError> {
    start_import(state.inner(), payload).await
}
pub(crate) async fn start_import(
    state: &AppState,
    payload: StartImportPayload,
) -> Result<Snapshot, TransferError> {
    if payload.mapping.is_empty() || payload.mapping.len() > 1600 {
        return Err(TransferError::invalid(
            "mapping",
            "Map between one and 1,600 columns",
        ));
    }
    for m in &payload.mapping {
        identifier(&m.target_column, "targetColumn")?;
        if m.source_index >= 1600 {
            return Err(TransferError::invalid(
                "sourceIndex",
                "Source column index is out of range",
            ));
        }
    }
    start(
        state,
        payload.inspection_token,
        RunRequest::Import {
            mapping: payload.mapping,
        },
        payload.confirmed,
    )
    .await
}
#[tauri::command]
pub(crate) async fn start_pg_csv_export(
    state: State<'_, AppState>,
    payload: StartExportPayload,
) -> Result<Snapshot, TransferError> {
    start_export(state.inner(), payload).await
}
pub(crate) async fn start_export(
    state: &AppState,
    payload: StartExportPayload,
) -> Result<Snapshot, TransferError> {
    path(&payload.destination_path)?;
    start(
        state,
        payload.inspection_token,
        RunRequest::Export {
            destination_path: payload.destination_path,
        },
        false,
    )
    .await
}
async fn start(
    state: &AppState,
    inspection_token: String,
    request: RunRequest,
    confirmed: bool,
) -> Result<Snapshot, TransferError> {
    start_with(state, inspection_token, request, confirmed, runner::run).await
}

async fn start_with<R, F>(
    state: &AppState,
    inspection_token: String,
    request: RunRequest,
    confirmed: bool,
    run: R,
) -> Result<Snapshot, TransferError>
where
    R: FnOnce(
            crate::postgres::transfer::manager::JobContext,
            StoredConnection,
            runner::Review,
            RunRequest,
        ) -> F
        + Send
        + 'static,
    F: std::future::Future<Output = Result<(), TransferError>> + Send + 'static,
{
    token(&inspection_token)?;
    let review = state.pg_transfers.review(&inspection_token)?;
    let direction = match &request {
        RunRequest::Import { .. } => Direction::Import,
        RunRequest::Export { .. } => Direction::Export,
    };
    if review.payload.direction != direction {
        return Err(TransferError::invalid(
            "inspectionToken",
            "The review direction does not match the request",
        ));
    }
    let connection_id = review.payload.connection_id.clone();
    let mut admission = state.pg_transfers.admission(&connection_id)?;
    let mut preparation = Box::pin(tokio::time::timeout(
        PREPARATION_TIMEOUT,
        connection(state, &connection_id),
    ));
    let prepared = tokio::select! {biased;_=admission.cancelled()=>Err(TransferError::ConnectionClosing),result=&mut preparation=>result.map_err(|_|TransferError::Timeout{operation:"connection".into()})?};
    drop(preparation);
    let conn = prepared?;
    let audit = if direction == Direction::Import {
        assert_permitted(
            &super::safety::resolved_policy(&conn),
            &WriteIntent::Import,
            confirmed,
        )
        .map_err(|r| {
            r.fold(
                |reason, _| TransferError::PolicyBlocked {
                    reason: reason.into(),
                },
                |mut statements| {
                    if statements.is_empty() {
                        statements.push(
                            crate::postgres::sql_class::StatementClass::Dml {
                                unbounded: false,
                                destructive: false,
                            }
                            .summary(0),
                        );
                    }
                    TransferError::PolicyNeedsConfirmation { statements }
                },
            )
        })?
        .audit_disposition()
    } else {
        AuditDisposition::NotRequired
    };
    let file_name = match &request {
        RunRequest::Import { .. } => review
            .inspection
            .file_name
            .clone()
            .unwrap_or_else(|| "CSV file".into()),
        RunRequest::Export { destination_path } => std::path::Path::new(destination_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("CSV file")
            .to_owned(),
    };
    let snapshot = Snapshot {
        job_id: String::new(),
        connection_id: connection_id.clone(),
        schema: review.payload.schema.clone(),
        table: review.payload.table.clone(),
        direction,
        file_name,
        phase: Phase::Preparing,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        total_bytes: review.inspection.total_bytes,
        bytes_processed: 0,
        rows_processed: None,
        rows_committed: None,
        failure: None,
    };
    let pool = state.pool.clone();
    let reviewed = review.as_ref().clone();
    state.pg_transfers.start(
        admission,
        &inspection_token,
        snapshot,
        move |ctx| run(ctx, conn, reviewed, request),
        Box::pin(async move {
            if audit == AuditDisposition::RequiredAfterSuccess {
                super::safety::record_override(
                    &pool,
                    &connection_id,
                    "start_pg_csv_import",
                    &WriteIntent::Import,
                )
                .await;
            }
            if storage::touch_connection_activity(&pool, &connection_id)
                .await
                .is_err()
            {
                log::warn!("Failed to record CSV transfer activity");
            }
        }),
    )
}

#[cfg(test)]
mod tests;
#[tauri::command]
pub(crate) async fn get_pg_transfer_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<Snapshot, TransferError> {
    state.pg_transfers.get(&job_id)
}
#[tauri::command]
pub(crate) async fn list_pg_transfer_jobs(
    state: State<'_, AppState>,
    connection_id: Option<String>,
) -> Result<Vec<Snapshot>, TransferError> {
    Ok(state.pg_transfers.list(connection_id.as_deref()))
}
#[tauri::command]
pub(crate) async fn cancel_pg_transfer_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<Snapshot, TransferError> {
    state.pg_transfers.cancel(&job_id)
}
#[tauri::command]
pub(crate) async fn release_pg_transfer_job(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<(), TransferError> {
    state.pg_transfers.release(&job_id)
}
#[tauri::command]
pub(crate) async fn pick_pg_transfer_file(
    app: tauri::AppHandle,
    direction: Direction,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file().add_filter("CSV", &["csv"]);
    let (tx, rx) = tokio::sync::oneshot::channel();
    match direction {
        Direction::Import => dialog.set_title("Choose a CSV source").pick_file(move |p| {
            let _ = tx.send(p);
        }),
        Direction::Export => dialog
            .set_title("Choose a new CSV destination")
            .set_file_name("export.csv")
            .save_file(move |p| {
                let _ = tx.send(p);
            }),
    }
    rx.await
        .map_err(|_| "File selection was interrupted".to_string())?
        .map(|f| {
            let p = f
                .into_path()
                .map_err(|_| "Select a local file".to_string())?;
            if !p.is_absolute() {
                return Err("Select an absolute local path".into());
            }
            p.into_os_string()
                .into_string()
                .map_err(|_| "The selected path is not Unicode".into())
        })
        .transpose()
}
