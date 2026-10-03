//! One observed ordinary-table or column comment/rename, never caller SQL.
mod preview;
mod types;
pub(crate) use super::data_documents::WritePermit;
use super::{data::DataDocument, Backend};
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins, native_table_ddl,
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

impl TableDdlPreview {
    /// Validates a read-only recovery description with the same renderer as a
    /// live review. This never observes a target or creates execution authority.
    pub(crate) fn matches_typed_description(
        &self,
        target: &TableDdlDescription,
        intent: &TableDdlIntent,
    ) -> bool {
        self.checked_heap_bytes().is_some()
            && preview::render(target, intent, self.statement_timeout_ms)
                .is_ok_and(|expected| expected == *self)
    }
}

pub struct TableDdlTarget {
    document: DataDocument,
    generation: u64,
    description: TableDdlDescription,
    statement_timeout_ms: Option<u32>,
}
impl TableDdlTarget {
    pub fn description(&self) -> &TableDdlDescription {
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
pub struct TableDdlReview {
    target: TableDdlTarget,
    intent: TableDdlIntent,
    preview: TableDdlPreview,
    attempt_id: TableDdlAttemptId,
}
impl TableDdlReview {
    pub fn target(&self) -> &TableDdlDescription {
        self.target.description()
    }
    pub fn intent(&self) -> &TableDdlIntent {
        &self.intent
    }
    pub fn preview(&self) -> &TableDdlPreview {
        &self.preview
    }
    pub fn attempt_id(&self) -> &TableDdlAttemptId {
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
pub struct TableDdlConfirmation {
    review: TableDdlReview,
}
impl TableDdlConfirmation {
    pub fn review(&self) -> &TableDdlReview {
        &self.review
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum TableDdlSubmission {
    NeedsConfirmation(Box<TableDdlConfirmation>),
    Finished(Box<TableDdlReceipt>),
}

impl Backend {
    /// A complete bounded capture on one read-only dedicated connection. Authority
    /// remains tied to this document and its cancellation generation.
    pub async fn observe_table_ddl(
        &self,
        document: &DataDocument,
        request: TableDdlRequest,
    ) -> Result<TableDdlTarget, TableDdlError> {
        request.validate()?;
        let generation = *document.0.read_cancellation().borrow();
        let (description, statement_timeout_ms) = self
            .object_read(document, move |spec, drivers, cancellation| async move {
                let timeout = spec.driver_options.statement_timeout_ms;
                native_table_ddl::observe(&spec, &drivers, cancellation, request)
                    .await
                    .map(|description| (description, timeout))
            })
            .await
            .map_err(|_| TableDdlError::Unavailable)?;
        let target = TableDdlTarget {
            document: document.clone(),
            generation,
            description,
            statement_timeout_ms,
        };
        if !target.current() {
            return Err(TableDdlError::Unavailable);
        }
        Ok(target)
    }
    /// Consumes the observation. No credentials or socket are used for review.
    pub async fn review_table_ddl(
        &self,
        target: TableDdlTarget,
        intent: TableDdlIntent,
    ) -> Result<TableDdlReview, TableDdlError> {
        if !self.0.documents.owns(&target.document) {
            return Err(TableDdlError::ForeignDocument);
        }
        let document = target.document.clone();
        self.data_call(&document, move |_, _, _| async move {
            Ok(if target.current() {
                preview::render(&target.description, &intent, target.statement_timeout_ms).map(
                    |preview| TableDdlReview {
                        target,
                        intent,
                        preview,
                        attempt_id: TableDdlAttemptId::new(),
                    },
                )
            } else {
                Err(TableDdlError::Unavailable)
            })
        })
        .await
        .map_err(|_| TableDdlError::Unavailable)?
    }
    /// The caller MUST durably save this exact attempt, target, intent and preview
    /// as uncertain and await the exact revision ACK before submission. Consumes
    /// authority once. Delivery loss is unknown, never permission to replay.
    pub async fn apply_table_ddl(
        &self,
        review: TableDdlReview,
    ) -> Result<TableDdlSubmission, TableDdlError> {
        self.submit(review, false, native_table_ddl::execute, load)
            .await
    }
    /// Same durable exact-attempt barrier as Apply, with a fresh stored policy check.
    pub async fn confirm_table_ddl(
        &self,
        confirmation: TableDdlConfirmation,
    ) -> Result<TableDdlSubmission, TableDdlError> {
        self.submit(confirmation.review, true, native_table_ddl::execute, load)
            .await
    }
    async fn submit<F, Fut>(
        &self,
        review: TableDdlReview,
        confirmed: bool,
        execute: F,
        loader: impl FnOnce(
                Arc<crate::app::AppState>,
                String,
            ) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
            + Send
            + 'static,
    ) -> Result<TableDdlSubmission, TableDdlError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                TableDdlDescription,
                TableDdlIntent,
                TableDdlPreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = TableDdlOutcome> + Send + 'static,
    {
        let document = review.target.document.clone();
        if !self.0.documents.owns(&document) {
            return Err(TableDdlError::ForeignDocument);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                if !review.target.current() {
                    return Err(TableDdlError::Unavailable);
                }
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| TableDdlError::Storage)?
                        .ok_or(TableDdlError::Unavailable)?;
                let crate::StoredConnection::PostgreSQL(pg) = &stored else {
                    return Err(TableDdlError::Unavailable);
                };
                let timeout = pg
                    .driver_options
                    .as_ref()
                    .and_then(|o| o.statement_timeout_ms);
                let generated =
                    preview::render(&review.target.description, &review.intent, timeout)?;
                if generated != review.preview {
                    return Err(TableDdlError::InvalidTarget);
                }
                let authorization = match assert_permitted(
                    &gate::resolved_policy(&stored),
                    &WriteIntent::Ddl,
                    confirmed,
                ) {
                    Ok(value) => value,
                    Err(refusal) => {
                        return refusal.fold(
                            |_, _| Err(TableDdlError::PolicyBlocked),
                            |_| {
                                Ok(TableDdlSubmission::NeedsConfirmation(Box::new(
                                    TableDdlConfirmation { review },
                                )))
                            },
                        )
                    }
                };
                let permit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| TableDdlError::Busy)?;
                let cancellation = document.0.read_cancellation();
                // Cancellation after policy and before this lease must not be lost.
                if !review.target.current() || !permit.check_preparing() {
                    return Err(TableDdlError::Unavailable);
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
                    _ => TableDdlOutcome::NotDispatched {
                        reason: TableDdlFailure::Connection,
                    },
                };
                if matches!(outcome, TableDdlOutcome::Applied { .. }) {
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
                Ok(TableDdlSubmission::Finished(Box::new(TableDdlReceipt {
                    attempt_id: review.attempt_id,
                    connection_id: document.0.connection.clone(),
                    target: review.target.description,
                    intent: review.intent,
                    outcome,
                })))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| TableDdlError::OutcomeUnavailable)?
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
