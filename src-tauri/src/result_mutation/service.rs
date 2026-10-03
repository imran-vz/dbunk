//! Host-neutral Result Mutation operations. The stored connection supplies the
//! policy; successful required overrides keep the existing audit command name.

use futures_util::future::BoxFuture;
use std::sync::Arc;

use crate::app::{find_connection, touch_connection_activity};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::result_mutation::protocol::*;
use crate::result_mutation::VirtualKeyLookup;
use crate::{storage, AppState, DatabaseEngine};

pub(crate) async fn analyze(
    state: &AppState,
    payload: AnalyzeResultSetPayload,
) -> Result<AnalyzeResultSetResult, ResultMutationError> {
    start_analyze(state, payload).await?.await
}

pub(crate) async fn start_analyze(
    state: &AppState,
    payload: AnalyzeResultSetPayload,
) -> Result<BoxFuture<'_, Result<AnalyzeResultSetResult, ResultMutationError>>, ResultMutationError>
{
    let spec = postgres_spec(state, &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let pool = state.pool.clone();
    let lookup: VirtualKeyLookup = Arc::new(move |connection_id, schema, table| {
        let pool = pool.clone();
        Box::pin(async move {
            storage::read_virtual_key(&pool, &connection_id, &schema, &table)
                .await
                .map_err(virtual_key_storage_error)
        })
    });
    let pending = state
        .result_mutations
        .start_analyze(spec, payload, lookup)
        .await?;
    Ok(Box::pin(async move {
        let result = pending.await?;
        touch_connection_activity(state, &connection_id).await;
        Ok(result)
    }))
}

pub(crate) async fn preview(
    state: &AppState,
    payload: PreviewResultMutationsPayload,
) -> Result<PreviewResult, ResultMutationError> {
    state.result_mutations.preview(payload).await
}

pub(crate) async fn apply(
    state: &AppState,
    payload: ApplyResultMutationsPayload,
) -> Result<ApplyResult, ResultMutationError> {
    start_apply(state, payload).await?.await
}

pub(crate) async fn start_apply(
    state: &AppState,
    payload: ApplyResultMutationsPayload,
) -> Result<BoxFuture<'_, Result<ApplyResult, ResultMutationError>>, ResultMutationError> {
    let connection = find_connection(state, &payload.connection_id)
        .await
        .map_err(|_| ResultMutationError::ConnectionLost)?;
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(ResultMutationError::UnsupportedEngine);
    }
    let spec = ResolvedPostgresConnectSpec::from_connection(&connection)
        .map_err(|_| ResultMutationError::UnsupportedEngine)?;
    let connection_id = payload.connection_id.clone();
    let pending = state.result_mutations.start_apply(spec, payload).await?;
    Ok(Box::pin(async move {
        let outcome = pending.await?;
        let (result, intent, authorization) = outcome.into_parts();
        if matches!(
            authorization.audit_disposition(),
            crate::safety::policy::AuditDisposition::RequiredAfterSuccess
        ) {
            crate::safety::gate::record_override(
                &state.pool,
                &connection_id,
                "apply_result_mutations",
                &intent,
            )
            .await;
        }
        touch_connection_activity(state, &connection_id).await;
        Ok(result)
    }))
}

pub(crate) async fn cancel(
    state: &AppState,
    payload: CancelResultMutationPayload,
) -> Result<CancelResultMutationResult, ResultMutationError> {
    Ok(state
        .result_mutations
        .cancel_tab(&payload.connection_id, &payload.tab_id)
        .await)
}

pub(crate) async fn close_connection(
    state: &AppState,
    payload: CloseResultMutationPayload,
) -> Result<(), ResultMutationError> {
    state
        .result_mutations
        .close_connection(&payload.connection_id)
        .await;
    Ok(())
}

pub(crate) async fn load_virtual_key(
    state: &AppState,
    payload: LoadVirtualKeyPayload,
) -> Result<Option<VirtualKey>, ResultMutationError> {
    storage::read_postgres_virtual_key(
        &state.pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
    )
    .await
    .map_err(virtual_key_storage_error)
}

pub(crate) async fn save_virtual_key(
    state: &AppState,
    payload: SaveVirtualKeyPayload,
) -> Result<(), ResultMutationError> {
    storage::upsert_postgres_virtual_key(
        &state.pool,
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
        .result_mutations
        .invalidate_virtual_key(&payload.connection_id, &payload.schema, &payload.table)
        .await;
    Ok(())
}

pub(crate) async fn clear_virtual_key(
    state: &AppState,
    payload: ClearVirtualKeyPayload,
) -> Result<(), ResultMutationError> {
    storage::clear_postgres_virtual_key(
        &state.pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
    )
    .await
    .map_err(virtual_key_storage_error)?;
    state
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
#[path = "service_tests.rs"]
mod tests;
