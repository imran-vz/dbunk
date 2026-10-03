//! Fixed native maintenance actions with consumed review authority and owned
//! execution. Recovery descriptions cannot be deserialized into executable tokens.
mod types;
pub(crate) use super::data_documents::WritePermit;
use super::{
    data::{DataDocument, DataError},
    objects::{PgObjectKind, PgObjectRef},
    Backend,
};
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins, native_maintenance,
};
use crate::safety::{
    gate,
    policy::{assert_permitted, AuditDisposition},
};
use futures_util::future::BoxFuture;
use std::{future::Future, sync::Arc};
use tokio::sync::watch;
pub use types::*;

pub(crate) fn preview(
    target: &MaintenanceTarget,
    intent: MaintenanceIntent,
    statement_timeout_ms: Option<u32>,
) -> Result<MaintenancePreview, MaintenanceError> {
    if !target.valid() {
        return Err(MaintenanceError::InvalidTarget);
    }
    let qualified = format!(
        "\"{}\".\"{}\"",
        target.schema().replace('"', "\"\""),
        target.name().replace('"', "\"\"")
    );
    let (command, semantics) = match intent {
        MaintenanceIntent::Vacuum => ("VACUUM", MaintenanceSemantics::PotentiallyPartial),
        MaintenanceIntent::Analyze => ("ANALYZE", MaintenanceSemantics::PotentiallyPartial),
        MaintenanceIntent::ReindexTable => (
            "REINDEX TABLE",
            if target.kind() == MaintenanceRelationKind::PartitionedTable {
                MaintenanceSemantics::PotentiallyPartial
            } else {
                MaintenanceSemantics::Transactional
            },
        ),
        MaintenanceIntent::RefreshMaterializedView { concurrently } => {
            if target.kind() != MaintenanceRelationKind::MaterializedView {
                return Err(MaintenanceError::InvalidTarget);
            }
            (
                if concurrently {
                    "REFRESH MATERIALIZED VIEW CONCURRENTLY"
                } else {
                    "REFRESH MATERIALIZED VIEW"
                },
                MaintenanceSemantics::Transactional,
            )
        }
    };
    Ok(MaintenancePreview {
        sql: format!("{command} {qualified}"),
        semantics,
        operation_timeout_ms: MAINTENANCE_OPERATION_TIMEOUT_MS,
        statement_timeout_ms,
    })
}
impl Backend {
    /// Dedicated, bounded read. The returned observation, not a caller-supplied
    /// OID or SQL string, is authority to review an operation on this document.
    pub async fn observe_maintenance_target(
        &self,
        document: &DataDocument,
        reference: PgObjectRef,
    ) -> Result<ObservedMaintenanceTarget, MaintenanceError> {
        if !matches!(
            reference.kind,
            PgObjectKind::Table | PgObjectKind::MaterializedView
        ) || !reference
            .schema
            .as_ref()
            .is_some_and(|s| types::valid_name(s))
            || !types::valid_name(&reference.name)
            || reference.identity_args.is_some()
        {
            return Err(MaintenanceError::InvalidTarget);
        }
        let result = self
            .object_read(document, move |spec, drivers, cancellation| async move {
                let timeout = spec.driver_options.statement_timeout_ms;
                native_maintenance::observe(&spec, &drivers, cancellation, &reference)
                    .await
                    .map(|target| (target, timeout))
            })
            .await
            .map_err(|_| MaintenanceError::Unavailable)?;
        Ok(ObservedMaintenanceTarget {
            document: document.clone(),
            target: result.0,
            statement_timeout_ms: result.1,
        })
    }
    /// Persist the exact attempt/review and await its save ACK before submitting.
    /// Delivery loss is uncertainty; it never authorizes an automatic retry.
    pub async fn apply_maintenance(
        &self,
        review: MaintenanceReview,
    ) -> Result<MaintenanceSubmission, MaintenanceError> {
        self.submit_maintenance(review, false, native_maintenance::execute)
            .await
    }
    pub async fn confirm_maintenance(
        &self,
        confirmation: MaintenanceConfirmation,
    ) -> Result<MaintenanceSubmission, MaintenanceError> {
        self.submit_maintenance(confirmation.review, true, native_maintenance::execute)
            .await
    }
    async fn submit_maintenance<F, Fut>(
        &self,
        review: MaintenanceReview,
        confirmed: bool,
        execute: F,
    ) -> Result<MaintenanceSubmission, MaintenanceError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                MaintenanceTarget,
                MaintenancePreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = native_maintenance::Execution> + Send + 'static,
    {
        self.submit_maintenance_with_loader(review, confirmed, execute, |state, connection| {
            Box::pin(async move {
                crate::app::find_connection(&state, &connection)
                    .await
                    .ok()
                    .and_then(|connection| {
                        ResolvedPostgresConnectSpec::from_connection(&connection).ok()
                    })
            })
        })
        .await
    }

    // Injectable credential boundary: tests can prove stale reviews never reach
    // hydration, rather than merely observing that SQL execution did not run.
    async fn submit_maintenance_with_loader<F, Fut>(
        &self,
        review: MaintenanceReview,
        confirmed: bool,
        execute: F,
        load: impl FnOnce(
                Arc<crate::app::AppState>,
                String,
            ) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
            + Send
            + 'static,
    ) -> Result<MaintenanceSubmission, MaintenanceError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                MaintenanceTarget,
                MaintenancePreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = native_maintenance::Execution> + Send + 'static,
    {
        let document = review.document.clone();
        if !self.0.documents.owns(&document) {
            return Err(MaintenanceError::ForeignDocument);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| MaintenanceError::Storage)?
                        .ok_or(MaintenanceError::Unavailable)?;
                let crate::StoredConnection::PostgreSQL(postgres) = &stored else {
                    return Err(MaintenanceError::Unavailable);
                };
                // Regenerate from current stored options before policy can mint
                // a confirmation, acquire a write slot or hydrate credentials.
                let generated = preview(
                    &review.target,
                    review.intent,
                    postgres
                        .driver_options
                        .as_ref()
                        .and_then(|options| options.statement_timeout_ms),
                )?;
                if generated != review.preview {
                    return Err(MaintenanceError::InvalidTarget);
                }
                let intent = review.intent.policy();
                let authorization =
                    match assert_permitted(&gate::resolved_policy(&stored), &intent, confirmed) {
                        Ok(value) => value,
                        Err(refusal) => {
                            return refusal.fold(
                                |_, _| Err(MaintenanceError::PolicyBlocked),
                                |_| {
                                    Ok(MaintenanceSubmission::NeedsConfirmation(Box::new(
                                        MaintenanceConfirmation { review },
                                    )))
                                },
                            )
                        }
                    };
                let permit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| MaintenanceError::Busy)?;
                let cancellation = document.0.read_cancellation();
                let spec = load(state.clone(), document.0.connection.clone()).await;
                let drivers = inner.tasks.child();
                drop(admission);
                let execution = match spec {
                    Some(spec)
                        if spec.driver_options.statement_timeout_ms
                            == generated.statement_timeout_ms =>
                    {
                        execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            review.target.clone(),
                            generated,
                        )
                        .await
                    }
                    _ => native_maintenance::Execution::not_dispatched(
                        MaintenanceFailure::Connection,
                    ),
                };
                if execution.outcome == MaintenanceOutcome::Completed {
                    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
                        gate::record_override(
                            &state.pool,
                            &document.0.connection,
                            review.intent.command(),
                            &intent,
                        )
                        .await;
                    }
                    crate::app::touch_connection_activity(&state, &document.0.connection).await;
                }
                Ok(MaintenanceSubmission::Finished(Box::new(
                    MaintenanceReceipt {
                        attempt_id: review.attempt_id,
                        intent: review.intent,
                        target: review.target,
                        preview: review.preview,
                        outcome: execution.outcome,
                        notices: execution.notices,
                        notices_truncated: execution.notices_truncated,
                        runtime_ms: execution.runtime_ms,
                    },
                )))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|error| match error {
            DataError::Document(_) => MaintenanceError::Unavailable,
            _ => MaintenanceError::OutcomeUnavailable,
        })?
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
