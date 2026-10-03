use tauri::State;

use crate::postgres::object_ddl::{DdlApplyResult, DdlPlanPreview, PgObjectError, PgObjectOp};
use crate::postgres::object_service;
pub(crate) use crate::postgres::object_service::{
    ApplyObjectDdlPayload, PgObjectReferencePayload, PreviewObjectDdlPayload,
};
use crate::postgres::objects::{PgDropImpact, PgObjectCatalog, PgObjectDescription};
use crate::{AppState, ConnectionPayload};

#[tauri::command]
pub async fn load_pg_object_catalog(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<PgObjectCatalog, PgObjectError> {
    load_pg_object_catalog_inner(state.inner(), &payload.connection_id).await
}

pub(crate) async fn load_pg_object_catalog_inner(
    state: &AppState,
    connection_id: &str,
) -> Result<PgObjectCatalog, PgObjectError> {
    object_service::load_catalog(state, connection_id).await
}

#[tauri::command]
pub async fn describe_pg_object(
    state: State<'_, AppState>,
    payload: PgObjectReferencePayload,
) -> Result<PgObjectDescription, PgObjectError> {
    object_service::describe(state.inner(), payload).await
}

#[tauri::command]
pub async fn load_pg_drop_impact(
    state: State<'_, AppState>,
    payload: PgObjectReferencePayload,
) -> Result<PgDropImpact, PgObjectError> {
    object_service::drop_impact(state.inner(), payload).await
}

#[tauri::command]
pub async fn preview_object_ddl(
    state: State<'_, AppState>,
    payload: PreviewObjectDdlPayload,
) -> Result<DdlPlanPreview, PgObjectError> {
    preview_object_ddl_inner(state.inner(), &payload.connection_id, &payload.ops).await
}

pub(crate) async fn preview_object_ddl_inner(
    state: &AppState,
    connection_id: &str,
    ops: &[PgObjectOp],
) -> Result<DdlPlanPreview, PgObjectError> {
    object_service::preview(state, connection_id, ops).await
}

#[tauri::command]
pub async fn apply_object_ddl(
    state: State<'_, AppState>,
    payload: ApplyObjectDdlPayload,
) -> Result<DdlApplyResult, PgObjectError> {
    apply_object_ddl_inner(state.inner(), payload).await
}

pub(crate) async fn apply_object_ddl_inner(
    state: &AppState,
    payload: ApplyObjectDdlPayload,
) -> Result<DdlApplyResult, PgObjectError> {
    object_service::apply(state, payload).await
}

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use crate::app::test_postgres_connection as connection;
}
