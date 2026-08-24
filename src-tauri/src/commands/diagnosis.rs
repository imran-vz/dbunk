//! Staged connection diagnosis command (ADR-0025).

use tauri::State;

use crate::diagnosis::{self, ConnectionDiagnosis, DiagnoseConnectionPayload};
use crate::{credentials, storage, tunnel, AppState};

use super::current_credential_mode;

/// Run the connect ladder for a (possibly unsaved) connection. The outer
/// `Err` covers validation and credential-store failures only; probe
/// failures are inside the report.
#[tauri::command]
pub async fn diagnose_connection(
    state: State<'_, AppState>,
    payload: DiagnoseConnectionPayload,
) -> Result<ConnectionDiagnosis, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    tunnel::validate_connection_tunnel(&payload.connection)?;
    let mut connection = payload.connection;
    if let Some(id) = payload.hydrate_credential_from {
        if connection.password().is_empty() {
            let mut stored = storage::read_connection_by_id(&state.pool, &id)
                .await?
                .ok_or_else(|| "Connection not found".to_string())?;
            if !credentials::destination_matches(&connection, &stored) {
                return Err(
                    "Enter the password again after changing the connection endpoint or transport security settings."
                        .to_string(),
                );
            }
            credentials::hydrate(&state.pool, mode, &mut stored).await?;
            connection.set_password(stored.password().to_string());
        }
    }
    diagnosis::run(&state.pool, mode, &connection).await
}
