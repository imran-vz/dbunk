use super::super::data_documents::ControlPermit;
use super::*;
use crate::{
    postgres::{
        connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins, native_admin_control,
    },
    safety::{
        gate,
        policy::{assert_permitted, AuditDisposition},
    },
};
use std::future::Future;
use tokio::sync::watch;

impl Backend {
    pub async fn apply_admin_control(
        &self,
        review: AdminControlReview,
    ) -> Result<AdminControlSubmission, AdminControlError> {
        self.submit_admin_control(review, false, native_admin_control::execute)
            .await
    }
    pub async fn confirm_admin_control(
        &self,
        confirmation: AdminControlConfirmation,
    ) -> Result<AdminControlSubmission, AdminControlError> {
        self.submit_admin_control(confirmation.review, true, native_admin_control::execute)
            .await
    }
    pub(super) async fn submit_admin_control<F, Fut>(
        &self,
        review: AdminControlReview,
        confirmed: bool,
        execute: F,
    ) -> Result<AdminControlSubmission, AdminControlError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                ControlPermit,
                watch::Receiver<u64>,
                AdminControlTarget,
                AdminControlAction,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = AdminControlOutcome> + Send + 'static,
    {
        let document = review.document.clone();
        if !self.0.documents.owns(&document) {
            return Err(AdminControlError::ForeignDocument);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| AdminControlError::Storage)?
                        .ok_or(AdminControlError::Unavailable)?;
                let intent = review.action.intent();
                let authorization =
                    match assert_permitted(&gate::resolved_policy(&stored), &intent, confirmed) {
                        Ok(value) => value,
                        Err(refusal) => {
                            return refusal.fold(
                                |_, _| Err(AdminControlError::PolicyBlocked),
                                |_| {
                                    Ok(AdminControlSubmission::NeedsConfirmation(Box::new(
                                        AdminControlConfirmation { review },
                                    )))
                                },
                            )
                        }
                    };
                // Policy and document admission precede secret hydration/socket
                // creation. Separate control admission does not block a signal
                // merely because another document owns a schema write.
                let permit = inner
                    .documents
                    .begin_control(&document)
                    .map_err(|_| AdminControlError::Busy)?;
                let cancellation = document.0.read_cancellation();
                let spec = crate::app::find_connection(&state, &document.0.connection)
                    .await
                    .ok()
                    .and_then(|connection| {
                        ResolvedPostgresConnectSpec::from_connection(&connection).ok()
                    });
                let drivers = inner.tasks.child();
                drop(admission);
                let outcome = match spec {
                    Some(spec) => {
                        execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            review.target.clone(),
                            review.action,
                        )
                        .await
                    }
                    None => AdminControlOutcome::NotDispatched {
                        reason: AdminControlFailure::Connection,
                    },
                };
                if outcome == AdminControlOutcome::SignalSent {
                    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
                        gate::record_override(
                            &state.pool,
                            &document.0.connection,
                            review.action.command(),
                            &intent,
                        )
                        .await;
                    }
                    crate::app::touch_connection_activity(&state, &document.0.connection).await;
                }
                // Cancellation/retirement after acknowledgement cannot replace
                // the signal receipt. There is no automatic retry of an attempt.
                Ok(AdminControlSubmission::Finished(AdminControlReceipt {
                    attempt_id: review.attempt_id,
                    action: review.action,
                    target: review.target,
                    outcome,
                }))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|error| match error {
            DataError::Document(_) => AdminControlError::Unavailable,
            _ => AdminControlError::OutcomeUnavailable,
        })?
    }
}
