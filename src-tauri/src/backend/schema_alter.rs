//! One observed existing-schema comment or rename, never caller SQL.
//! Mirrors table_ddl: observation pins the schema OID, review is pure, and
//! Apply rechecks stored policy and the observed identity inside its transaction.
mod preview;
mod types;
pub(crate) use super::data_documents::WritePermit;
use super::{data::DataDocument, Backend};
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins, native_schema_alter,
};
use crate::safety::{
    gate,
    policy::{assert_permitted, AuditDisposition, WriteIntent},
};
use futures_util::future::BoxFuture;
use std::{
    future::Future,
    mem::{size_of, size_of_val},
    sync::Arc,
};
use tokio::sync::watch;
pub use types::*;

impl SchemaAlterPreview {
    /// Validates a read-only recovery description with the same renderer as a
    /// live review. This never observes a target or creates execution authority.
    pub(crate) fn matches_typed_description(
        &self,
        target: &SchemaAlterDescription,
        intent: &SchemaAlterIntent,
    ) -> bool {
        self.checked_heap_bytes().is_some()
            && preview::render(target, intent, self.statement_timeout_ms)
                .is_ok_and(|expected| expected == *self)
    }
}

pub struct SchemaAlterTarget {
    document: DataDocument,
    generation: u64,
    description: SchemaAlterDescription,
    statement_timeout_ms: Option<u32>,
}
impl SchemaAlterTarget {
    pub fn description(&self) -> &SchemaAlterDescription {
        &self.description
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .description
                .checked_heap_bytes()
                .unwrap_or(usize::MAX / 2)
            + size_of_val(&*self.document.0)
            + self.document.0.window.capacity()
            + self.document.0.tab.capacity()
            + self.document.0.connection.capacity()
            + self.document.0.manager_tab.capacity()
    }
    pub fn encoded_bytes(&self) -> usize {
        self.description.encoded_bytes()
    }
    fn current(&self) -> bool {
        self.document.0.check_open().is_ok()
            && *self.document.0.read_cancellation().borrow() == self.generation
    }
}
pub struct SchemaAlterReview {
    target: SchemaAlterTarget,
    intent: SchemaAlterIntent,
    preview: SchemaAlterPreview,
    attempt_id: SchemaAlterAttemptId,
}
impl SchemaAlterReview {
    pub fn target(&self) -> &SchemaAlterDescription {
        self.target.description()
    }
    pub fn intent(&self) -> &SchemaAlterIntent {
        &self.intent
    }
    pub fn preview(&self) -> &SchemaAlterPreview {
        &self.preview
    }
    pub fn attempt_id(&self) -> &SchemaAlterAttemptId {
        &self.attempt_id
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.target.belongs_to(document)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.target.retained_bytes()
            + self.intent.checked_heap_bytes().unwrap_or(usize::MAX / 4)
            + self.preview.checked_heap_bytes().unwrap_or(usize::MAX / 4)
            + self.attempt_id.as_str().len()
    }
}
pub struct SchemaAlterConfirmation {
    review: SchemaAlterReview,
}
impl SchemaAlterConfirmation {
    pub fn review(&self) -> &SchemaAlterReview {
        &self.review
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum SchemaAlterSubmission {
    NeedsConfirmation(Box<SchemaAlterConfirmation>),
    Finished(Box<SchemaAlterReceipt>),
}

impl Backend {
    /// A complete bounded capture on one read-only dedicated connection. Authority
    /// remains tied to this document and its cancellation generation.
    pub async fn observe_schema_alter(
        &self,
        document: &DataDocument,
        request: SchemaAlterRequest,
    ) -> Result<SchemaAlterTarget, SchemaAlterError> {
        request.validate()?;
        let generation = *document.0.read_cancellation().borrow();
        let (description, statement_timeout_ms) = self
            .object_read(document, move |spec, drivers, cancellation| async move {
                let timeout = spec.driver_options.statement_timeout_ms;
                native_schema_alter::observe(&spec, &drivers, cancellation, request)
                    .await
                    .map(|description| (description, timeout))
            })
            .await
            .map_err(|_| SchemaAlterError::Unavailable)?;
        let target = SchemaAlterTarget {
            document: document.clone(),
            generation,
            description,
            statement_timeout_ms,
        };
        if !target.current() {
            return Err(SchemaAlterError::Unavailable);
        }
        Ok(target)
    }
    /// Consumes the observation. No credentials or socket are used for review.
    pub async fn review_schema_alter(
        &self,
        target: SchemaAlterTarget,
        intent: SchemaAlterIntent,
    ) -> Result<SchemaAlterReview, SchemaAlterError> {
        if !self.0.documents.owns(&target.document) {
            return Err(SchemaAlterError::ForeignDocument);
        }
        let document = target.document.clone();
        self.data_call(&document, move |_, _, _| async move {
            Ok(if target.current() {
                preview::render(&target.description, &intent, target.statement_timeout_ms).map(
                    |preview| SchemaAlterReview {
                        target,
                        intent,
                        preview,
                        attempt_id: SchemaAlterAttemptId::new(),
                    },
                )
            } else {
                Err(SchemaAlterError::Unavailable)
            })
        })
        .await
        .map_err(|_| SchemaAlterError::Unavailable)?
    }
    /// The caller MUST durably save this exact attempt, target, intent and preview
    /// as uncertain and await the exact revision ACK before submission. Consumes
    /// authority once. Delivery loss is unknown, never permission to replay.
    pub async fn apply_schema_alter(
        &self,
        review: SchemaAlterReview,
    ) -> Result<SchemaAlterSubmission, SchemaAlterError> {
        self.submit_schema_alter(review, false, native_schema_alter::execute, load)
            .await
    }
    /// Same durable exact-attempt barrier as Apply, with a fresh stored policy check.
    pub async fn confirm_schema_alter(
        &self,
        confirmation: SchemaAlterConfirmation,
    ) -> Result<SchemaAlterSubmission, SchemaAlterError> {
        self.submit_schema_alter(
            confirmation.review,
            true,
            native_schema_alter::execute,
            load,
        )
        .await
    }
    async fn submit_schema_alter<F, Fut>(
        &self,
        review: SchemaAlterReview,
        confirmed: bool,
        execute: F,
        loader: impl FnOnce(
                Arc<crate::app::AppState>,
                String,
            ) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
            + Send
            + 'static,
    ) -> Result<SchemaAlterSubmission, SchemaAlterError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                SchemaAlterDescription,
                SchemaAlterIntent,
                SchemaAlterPreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = SchemaAlterOutcome> + Send + 'static,
    {
        let document = review.target.document.clone();
        if !self.0.documents.owns(&document) {
            return Err(SchemaAlterError::ForeignDocument);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                if !review.target.current() {
                    return Err(SchemaAlterError::Unavailable);
                }
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| SchemaAlterError::Storage)?
                        .ok_or(SchemaAlterError::Unavailable)?;
                let crate::StoredConnection::PostgreSQL(pg) = &stored else {
                    return Err(SchemaAlterError::Unavailable);
                };
                let timeout = pg
                    .driver_options
                    .as_ref()
                    .and_then(|o| o.statement_timeout_ms);
                let generated =
                    preview::render(&review.target.description, &review.intent, timeout)?;
                if generated != review.preview {
                    return Err(SchemaAlterError::InvalidTarget);
                }
                let authorization = match assert_permitted(
                    &gate::resolved_policy(&stored),
                    &WriteIntent::Ddl,
                    confirmed,
                ) {
                    Ok(value) => value,
                    Err(refusal) => {
                        return refusal.fold(
                            |_, _| Err(SchemaAlterError::PolicyBlocked),
                            |_| {
                                Ok(SchemaAlterSubmission::NeedsConfirmation(Box::new(
                                    SchemaAlterConfirmation { review },
                                )))
                            },
                        )
                    }
                };
                let permit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| SchemaAlterError::Busy)?;
                let cancellation = document.0.read_cancellation();
                // Cancellation after policy and before this lease must not be lost.
                if !review.target.current() || !permit.check_preparing() {
                    return Err(SchemaAlterError::Unavailable);
                }
                let spec = loader(state.clone(), document.0.connection.clone()).await;
                let drivers = inner.tasks.child();
                drop(admission);
                let outcome = match spec {
                    Some(spec) if spec.driver_options.statement_timeout_ms == timeout => {
                        execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            review.target.description.clone(),
                            review.intent.clone(),
                            generated,
                        )
                        .await
                    }
                    _ => SchemaAlterOutcome::NotDispatched {
                        reason: SchemaAlterFailure::Connection,
                    },
                };
                if matches!(outcome, SchemaAlterOutcome::Applied { .. }) {
                    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
                        gate::record_override(
                            &state.pool,
                            &document.0.connection,
                            "apply_object_ddl",
                            &WriteIntent::Ddl,
                        )
                        .await;
                    }
                    crate::app::touch_connection_activity(&state, &document.0.connection).await;
                }
                // No late document/cancellation check may overwrite COMMIT.
                Ok(SchemaAlterSubmission::Finished(Box::new(
                    SchemaAlterReceipt {
                        attempt_id: review.attempt_id,
                        connection_id: document.0.connection.clone(),
                        target: review.target.description,
                        intent: review.intent,
                        outcome,
                    },
                )))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| SchemaAlterError::OutcomeUnavailable)?
    }
}
fn load(
    state: Arc<crate::app::AppState>,
    connection: String,
) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>> {
    Box::pin(async move {
        crate::app::find_connection(&state, &connection)
            .await
            .ok()
            .and_then(|c| ResolvedPostgresConnectSpec::from_connection(&c).ok())
    })
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
