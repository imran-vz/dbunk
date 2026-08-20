use std::sync::Arc;

use tauri::State;

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::result_mutation::protocol::*;
use crate::result_mutation::VirtualKeyLookup;
use crate::{storage, AppState, DatabaseEngine};

use super::{find_connection, touch_connection_activity};

#[tauri::command]
pub async fn analyze_result_set(
    state: State<'_, AppState>,
    payload: AnalyzeResultSetPayload,
) -> Result<AnalyzeResultSetResult, ResultMutationError> {
    let spec = postgres_spec(state.inner(), &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let pool = state.inner().pool.clone();
    let lookup: VirtualKeyLookup = Arc::new(move |connection_id, schema, table| {
        let pool = pool.clone();
        Box::pin(async move {
            storage::read_virtual_key(&pool, &connection_id, &schema, &table)
                .await
                .map_err(virtual_key_storage_error)
        })
    });
    let result = state
        .inner()
        .result_mutations
        .analyze(spec, payload, lookup)
        .await?;
    touch_connection_activity(state.inner(), &connection_id).await;
    Ok(result)
}

#[tauri::command]
pub async fn preview_result_mutations(
    state: State<'_, AppState>,
    payload: PreviewResultMutationsPayload,
) -> Result<PreviewResult, ResultMutationError> {
    state.inner().result_mutations.preview(payload).await
}

#[tauri::command]
pub async fn apply_result_mutations(
    state: State<'_, AppState>,
    payload: ApplyResultMutationsPayload,
) -> Result<ApplyResult, ResultMutationError> {
    let spec = postgres_spec(state.inner(), &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let result = state.inner().result_mutations.apply(spec, payload).await?;
    touch_connection_activity(state.inner(), &connection_id).await;
    Ok(result)
}

#[tauri::command]
pub async fn cancel_result_mutation(
    state: State<'_, AppState>,
    payload: CancelResultMutationPayload,
) -> Result<CancelResultMutationResult, ResultMutationError> {
    Ok(state
        .inner()
        .result_mutations
        .cancel_tab(&payload.connection_id, &payload.tab_id)
        .await)
}

#[tauri::command]
pub async fn close_result_mutation_for_connection(
    state: State<'_, AppState>,
    payload: CloseResultMutationPayload,
) -> Result<(), ResultMutationError> {
    state
        .inner()
        .result_mutations
        .close_connection(&payload.connection_id)
        .await;
    Ok(())
}

#[tauri::command]
pub async fn load_virtual_key(
    state: State<'_, AppState>,
    payload: LoadVirtualKeyPayload,
) -> Result<Option<VirtualKey>, ResultMutationError> {
    storage::read_postgres_virtual_key(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
    )
    .await
    .map_err(virtual_key_storage_error)
}

#[tauri::command]
pub async fn save_virtual_key(
    state: State<'_, AppState>,
    payload: SaveVirtualKeyPayload,
) -> Result<(), ResultMutationError> {
    storage::upsert_postgres_virtual_key(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
        &VirtualKey {
            version: storage::VIRTUAL_KEY_VERSION,
            columns: payload.columns,
        },
    )
    .await
    .map_err(virtual_key_storage_error)?;
    state
        .inner()
        .result_mutations
        .invalidate_virtual_key(&payload.connection_id, &payload.schema, &payload.table)
        .await;
    Ok(())
}

#[tauri::command]
pub async fn clear_virtual_key(
    state: State<'_, AppState>,
    payload: ClearVirtualKeyPayload,
) -> Result<(), ResultMutationError> {
    storage::clear_postgres_virtual_key(
        &state.inner().pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
    )
    .await
    .map_err(virtual_key_storage_error)?;
    state
        .inner()
        .result_mutations
        .invalidate_virtual_key(&payload.connection_id, &payload.schema, &payload.table)
        .await;
    Ok(())
}

async fn postgres_spec(
    state: &AppState,
    connection_id: &str,
) -> Result<ResolvedPostgresConnectSpec, ResultMutationError> {
    let connection = find_connection(state, connection_id)
        .await
        .map_err(|_| ResultMutationError::ConnectionLost)?;
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(ResultMutationError::UnsupportedEngine);
    }
    ResolvedPostgresConnectSpec::from_connection(&connection)
        .map_err(|_| ResultMutationError::UnsupportedEngine)
}

fn virtual_key_storage_error(error: storage::VirtualKeyStorageError) -> ResultMutationError {
    match error {
        storage::VirtualKeyStorageError::ConnectionNotFound => ResultMutationError::ConnectionLost,
        storage::VirtualKeyStorageError::UnsupportedEngine => {
            ResultMutationError::UnsupportedEngine
        }
        storage::VirtualKeyStorageError::InvalidInput(
            storage::VirtualKeyValidationError::EmptyIdentity,
        ) => ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::EmptyIdentity,
        },
        storage::VirtualKeyStorageError::InvalidInput(
            storage::VirtualKeyValidationError::DuplicateColumn,
        ) => ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::DuplicateColumn,
        },
        storage::VirtualKeyStorageError::InvalidInput(
            storage::VirtualKeyValidationError::UnsupportedVersion(_),
        )
        | storage::VirtualKeyStorageError::CorruptDocument(_)
        | storage::VirtualKeyStorageError::Database(_) => ResultMutationError::Database {
            code: None,
            message: "Virtual key storage failed".to_string(),
            severity: None,
            position: None,
            op_index: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_key_storage_errors_preserve_the_command_contract() {
        assert_eq!(
            virtual_key_storage_error(storage::VirtualKeyStorageError::UnsupportedEngine),
            ResultMutationError::UnsupportedEngine
        );
        assert_eq!(
            virtual_key_storage_error(storage::VirtualKeyStorageError::ConnectionNotFound),
            ResultMutationError::ConnectionLost
        );
        assert_eq!(
            virtual_key_storage_error(storage::VirtualKeyStorageError::InvalidInput(
                storage::VirtualKeyValidationError::EmptyIdentity,
            )),
            ResultMutationError::InvalidPlan {
                reason: InvalidPlanReason::EmptyIdentity,
            }
        );
        assert_eq!(
            virtual_key_storage_error(storage::VirtualKeyStorageError::InvalidInput(
                storage::VirtualKeyValidationError::DuplicateColumn,
            )),
            ResultMutationError::InvalidPlan {
                reason: InvalidPlanReason::DuplicateColumn,
            }
        );
        assert!(matches!(
            virtual_key_storage_error(storage::VirtualKeyStorageError::CorruptDocument(
                "stale JSON".to_string(),
            )),
            ResultMutationError::Database { .. }
        ));
    }
}
