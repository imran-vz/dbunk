//! Host-neutral PostgreSQL file-job orchestration; admission is not success.
use super::{
    protocol::*,
    runner::{self, Request},
};
use crate::safety::policy::{assert_permitted, AuditDisposition, WriteIntent};
use crate::{storage, AppState, DatabaseEngine};

pub(crate) async fn start(
    state: &AppState,
    request: Request,
) -> Result<PgToolJobSnapshot, PgToolJobError> {
    start_with(state, request, runner::run).await
}

pub(crate) async fn start_with<R, F>(
    state: &AppState,
    request: Request,
    run: R,
) -> Result<PgToolJobSnapshot, PgToolJobError>
where
    R: FnOnce(crate::postgres::backup::manager::JobContext, crate::StoredConnection, Request) -> F
        + Send
        + 'static,
    F: std::future::Future<Output = Result<runner::Ready, PgToolJobError>> + Send + 'static,
{
    let connection_id = request.connection_id().to_string();
    let mut admission = state.pg_tool_jobs.admission(&connection_id)?;
    let mut preparation = Box::pin(async {
        request.validate().await?;
        // Reject unsupported engines before hydration/tunnel creation.
        let stored = storage::read_connection_by_id(&state.pool, &connection_id)
            .await
            .map_err(|_| PgToolJobError::invalid("connectionId", "Unable to load connection"))?
            .ok_or_else(|| PgToolJobError::invalid("connectionId", "Connection not found"))?;
        if stored.engine() != DatabaseEngine::PostgreSQL {
            return Err(PgToolJobError::UnsupportedEngine);
        }
        let connection = crate::app::find_connection(state, &connection_id)
            .await
            .map_err(|_| {
                PgToolJobError::invalid(
                    "connectionId",
                    "Unable to resolve connection credentials or tunnel",
                )
            })?;
        let audit = match &request {
            Request::Backup(_) => AuditDisposition::NotRequired,
            Request::Restore(payload) => assert_permitted(
                &crate::safety::gate::resolved_policy(&connection),
                &WriteIntent::Restore,
                payload.confirmed,
            )
            .map_err(|refusal| {
                refusal.fold(
                    |reason, _| PgToolJobError::PolicyBlocked {
                        reason: reason.into(),
                    },
                    |statements| PgToolJobError::PolicyNeedsConfirmation { statements },
                )
            })?
            .audit_disposition(),
        };
        Ok((connection, audit))
    });
    let prepared = tokio::select! {
        biased;
        _ = admission.cancelled() => Err(PgToolJobError::ConnectionClosing),
        prepared = &mut preparation => prepared,
    };
    drop(preparation);
    let (connection, audit) = prepared?;
    let pool = state.pool.clone();
    let snapshot = request.snapshot();
    state.pg_tool_jobs.start(
        admission,
        snapshot,
        move |context| run(context, connection, request),
        Box::pin(async move {
            if audit == AuditDisposition::RequiredAfterSuccess {
                crate::safety::gate::record_override(
                    &pool,
                    &connection_id,
                    "start_pg_restore",
                    &WriteIntent::Restore,
                )
                .await;
            }
            if storage::touch_connection_activity(&pool, &connection_id)
                .await
                .is_err()
            {
                log::warn!("Failed to touch PostgreSQL tool job activity");
            }
        }),
    )
}
