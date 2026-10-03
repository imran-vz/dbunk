//! Host-neutral Table Browse operations. Connection resolution, engine checks
//! and activity updates stay here; hosts do not call the manager directly.

use crate::app::{find_connection, touch_connection_activity};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::table_browse::protocol::*;
use crate::{AppState, DatabaseEngine};
use futures_util::future::BoxFuture;

pub(crate) async fn browse(
    state: &AppState,
    payload: BrowseTableDataPayload,
) -> Result<BrowseTableResult, TableBrowseError> {
    start_browse(state, payload).await?.await
}

/// Admit before returning the result wait, allowing native startup admission to
/// be released while database work remains independently cancelable.
pub(crate) async fn start_browse(
    state: &AppState,
    payload: BrowseTableDataPayload,
) -> Result<BoxFuture<'_, Result<BrowseTableResult, TableBrowseError>>, TableBrowseError> {
    let spec = postgres_spec(state, &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let pending = state.table_browse.start_browse(spec, payload).await?;
    Ok(Box::pin(async move {
        let result = pending.await?;
        touch_connection_activity(state, &connection_id).await;
        Ok(result)
    }))
}

pub(crate) async fn cancel(
    state: &AppState,
    payload: TableBrowseTabPayload,
) -> Result<CancelTableBrowseResult, TableBrowseError> {
    Ok(state
        .table_browse
        .cancel_tab(&payload.connection_id, &payload.tab_id)
        .await)
}

pub(crate) async fn count(
    state: &AppState,
    payload: CountTableBrowseRowsPayload,
) -> Result<BrowseExactCountResult, TableBrowseError> {
    start_count(state, payload).await?.await
}

pub(crate) async fn start_count(
    state: &AppState,
    payload: CountTableBrowseRowsPayload,
) -> Result<BoxFuture<'_, Result<BrowseExactCountResult, TableBrowseError>>, TableBrowseError> {
    let spec = postgres_spec(state, &payload.connection_id).await?;
    let connection_id = payload.connection_id.clone();
    let pending = state.table_browse.start_count(spec, payload).await?;
    Ok(Box::pin(async move {
        let result = pending.await?;
        touch_connection_activity(state, &connection_id).await;
        Ok(result)
    }))
}

pub(crate) async fn close_tab(
    state: &AppState,
    payload: TableBrowseTabPayload,
) -> Result<(), TableBrowseError> {
    state
        .table_browse
        .close_tab(&payload.connection_id, &payload.tab_id)
        .await;
    Ok(())
}

pub(crate) async fn load_grid_prefs(
    state: &AppState,
    payload: LoadTableGridPrefsPayload,
) -> Result<Option<TableGridPrefs>, String> {
    crate::storage::read_table_grid_prefs(
        &state.pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
    )
    .await
}

pub(crate) async fn save_grid_prefs(
    state: &AppState,
    payload: SaveTableGridPrefsPayload,
) -> Result<(), String> {
    let prefs = validate_table_grid_prefs(payload.prefs)?;
    crate::storage::upsert_table_grid_prefs(
        &state.pool,
        &payload.connection_id,
        &payload.schema,
        &payload.table,
        &prefs,
    )
    .await
}

async fn postgres_spec(
    state: &AppState,
    connection_id: &str,
) -> Result<ResolvedPostgresConnectSpec, TableBrowseError> {
    let connection = find_connection(state, connection_id)
        .await
        .map_err(|_| TableBrowseError::ConnectionLost)?;
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(TableBrowseError::UnsupportedEngine);
    }
    ResolvedPostgresConnectSpec::from_connection(&connection)
        .map_err(|_| TableBrowseError::UnsupportedEngine)
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
