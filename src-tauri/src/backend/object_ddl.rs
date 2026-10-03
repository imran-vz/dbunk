//! Typed object DDL lifecycle on one owned connection, never caller SQL.
//!
//! Observation captures exact catalog identities (and, for drops, the bounded
//! impact walk) in one read-only snapshot. Review regenerates statements from
//! typed operations and binds them to the observed identities, the stored
//! policy and the statement timeout. Apply rechecks all of them, then runs
//! atomic groups in one transaction each and standalone statements alone,
//! reporting a committed prefix, residue, or an unknown outcome truthfully.
mod preview;
mod types;
pub(crate) use super::data_documents::WritePermit;
use super::{data::DataDocument, Backend};
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins, native_object_ddl,
};
use crate::safety::{
    gate,
    policy::{assert_permitted, requires_confirmation, AuditDisposition, WriteIntent},
};
use futures_util::future::BoxFuture;
use std::{
    future::Future,
    mem::{size_of, size_of_val},
    sync::Arc,
};
use tokio::sync::watch;
pub use types::*;

impl ObjectDdlPreview {
    /// Validates a read-only recovery description with the same renderer as a
    /// live review. This never observes a target or creates execution authority.
    pub fn matches_typed_description(
        &self,
        target: &ObjectDdlDescription,
        operations: &[ObjectDdlOperation],
    ) -> bool {
        self.checked_heap_bytes().is_some()
            && preview::render(
                target,
                operations,
                self.statement_timeout_ms,
                self.confirmation_required,
            )
            .is_ok_and(|expected| expected == *self)
    }
}

fn impact_bytes(impacts: &[Option<PgDropImpact>]) -> usize {
    impacts
        .iter()
        .flatten()
        .fold(size_of_val(impacts), |total, impact| {
            impact.dependents.iter().fold(
                total.saturating_add(impact.dependents.capacity() * size_of::<PgDropDependent>()),
                |total, dependent| {
                    total
                        .saturating_add(dependent.object_type.capacity())
                        .saturating_add(dependent.identity.capacity())
                },
            )
        })
}

pub struct ObjectDdlTarget {
    document: DataDocument,
    generation: u64,
    operations: Vec<ObjectDdlOperation>,
    description: ObjectDdlDescription,
    impacts: Vec<Option<PgDropImpact>>,
    statement_timeout_ms: Option<u32>,
}
impl ObjectDdlTarget {
    pub fn description(&self) -> &ObjectDdlDescription {
        &self.description
    }
    pub fn operations(&self) -> &[ObjectDdlOperation] {
        &self.operations
    }
    /// Aligned with operations; `Some` only for drops. Review evidence only.
    pub fn impacts(&self) -> &[Option<PgDropImpact>] {
        &self.impacts
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(
                self.description
                    .checked_heap_bytes()
                    .unwrap_or(usize::MAX / 4),
            )
            .saturating_add(operations_heap_bytes(&self.operations).unwrap_or(usize::MAX / 4))
            .saturating_add(impact_bytes(&self.impacts))
            .saturating_add(size_of_val(&*self.document.0))
            .saturating_add(self.document.0.window.capacity())
            .saturating_add(self.document.0.tab.capacity())
            .saturating_add(self.document.0.connection.capacity())
            .saturating_add(self.document.0.manager_tab.capacity())
    }
    pub fn encoded_bytes(&self) -> usize {
        self.description
            .encoded_bytes()
            .saturating_add(crate::backend::schema_ddl::encoded_bytes(&self.operations))
            .saturating_add(crate::backend::schema_ddl::encoded_bytes(&self.impacts))
    }
    fn current(&self) -> bool {
        self.document.0.check_open().is_ok()
            && *self.document.0.read_cancellation().borrow() == self.generation
    }
}
pub struct ObjectDdlReview {
    target: ObjectDdlTarget,
    preview: ObjectDdlPreview,
    attempt_id: ObjectDdlAttemptId,
}
impl ObjectDdlReview {
    pub fn target(&self) -> &ObjectDdlDescription {
        self.target.description()
    }
    pub fn operations(&self) -> &[ObjectDdlOperation] {
        self.target.operations()
    }
    pub fn impacts(&self) -> &[Option<PgDropImpact>] {
        self.target.impacts()
    }
    pub fn preview(&self) -> &ObjectDdlPreview {
        &self.preview
    }
    pub fn attempt_id(&self) -> &ObjectDdlAttemptId {
        &self.attempt_id
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.target.belongs_to(document)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.target.retained_bytes())
            .saturating_add(self.preview.checked_heap_bytes().unwrap_or(usize::MAX / 4))
            .saturating_add(self.attempt_id.as_str().len())
    }
    /// Upper bound of this token's encoded delivery size.
    pub fn encoded_bytes(&self) -> usize {
        self.target
            .encoded_bytes()
            .saturating_add(crate::backend::schema_ddl::encoded_bytes(&self.preview))
            .saturating_add(128)
    }
}
pub struct ObjectDdlConfirmation {
    review: ObjectDdlReview,
}
impl ObjectDdlConfirmation {
    pub fn review(&self) -> &ObjectDdlReview {
        &self.review
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum ObjectDdlSubmission {
    NeedsConfirmation(Box<ObjectDdlConfirmation>),
    Finished(Box<ObjectDdlReceipt>),
}

fn observation_error(error: super::data::DataError) -> ObjectDdlError {
    use crate::postgres::native_catalog::CatalogError;
    match error {
        super::data::DataError::Catalog(
            CatalogError::ObjectNotFound | CatalogError::StructureIdentityChanged,
        ) => ObjectDdlError::TargetMismatch,
        super::data::DataError::Catalog(
            CatalogError::DropImpactLimit | CatalogError::DescriptionLimit,
        ) => ObjectDdlError::Limit,
        super::data::DataError::Catalog(
            CatalogError::UnsupportedObjectKind | CatalogError::InvalidReference,
        ) => ObjectDdlError::InvalidRequest,
        _ => ObjectDdlError::Unavailable,
    }
}

impl Backend {
    /// One bounded capture on a read-only dedicated connection: every claimed
    /// identity, absence and (for drops) the impact walk share one snapshot.
    /// Authority remains tied to this document and its cancellation generation.
    pub async fn observe_object_ddl(
        &self,
        document: &DataDocument,
        request: ObjectDdlRequest,
    ) -> Result<ObjectDdlTarget, ObjectDdlError> {
        request.validate()?;
        // The shared renderer refuses unsupported or invalid operations before
        // any socket is opened; observed claims are not needed for that check.
        let typed = request
            .operations
            .iter()
            .map(ObjectDdlOperation::to_pg)
            .collect::<Vec<_>>();
        crate::postgres::object_ddl::generate_object_ddl(&typed)
            .map_err(|_| ObjectDdlError::InvalidRequest)?;
        let generation = *document.0.read_cancellation().borrow();
        let operations = request.operations.clone();
        let ((description, impacts), statement_timeout_ms) = self
            .object_read(document, move |spec, drivers, cancellation| async move {
                let timeout = spec.driver_options.statement_timeout_ms;
                native_object_ddl::observe(&spec, &drivers, cancellation, operations)
                    .await
                    .map(|observed| (observed, timeout))
            })
            .await
            .map_err(observation_error)?;
        if !description.answers(&request.operations) || description.checked_heap_bytes().is_none() {
            return Err(ObjectDdlError::TargetMismatch);
        }
        let target = ObjectDdlTarget {
            document: document.clone(),
            generation,
            operations: request.operations,
            description,
            impacts,
            statement_timeout_ms,
        };
        if !target.current() {
            return Err(ObjectDdlError::Unavailable);
        }
        Ok(target)
    }

    /// Consumes the observation. Reads only the stored policy; no credentials
    /// or socket are used. A blocking policy refuses before any review exists.
    pub async fn review_object_ddl(
        &self,
        target: ObjectDdlTarget,
    ) -> Result<ObjectDdlReview, ObjectDdlError> {
        if !self.0.documents.owns(&target.document) {
            return Err(ObjectDdlError::ForeignDocument);
        }
        let document = target.document.clone();
        self.data_call(&document, move |state, document, _| async move {
            Ok(async {
                if !target.current() {
                    return Err(ObjectDdlError::Unavailable);
                }
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| ObjectDdlError::Storage)?
                        .ok_or(ObjectDdlError::Unavailable)?;
                let policy = gate::resolved_policy(&stored);
                if assert_permitted(&policy, &WriteIntent::Ddl, true).is_err() {
                    return Err(ObjectDdlError::PolicyBlocked);
                }
                let preview = preview::render(
                    &target.description,
                    &target.operations,
                    target.statement_timeout_ms,
                    requires_confirmation(&policy, &WriteIntent::Ddl),
                )?;
                Ok(ObjectDdlReview {
                    target,
                    preview,
                    attempt_id: ObjectDdlAttemptId::new(),
                })
            }
            .await)
        })
        .await
        .map_err(|_| ObjectDdlError::Unavailable)?
    }

    /// The caller MUST durably save this exact attempt, target, operations and
    /// preview as uncertain and await the exact revision ACK before submission.
    /// Consumes authority once. Delivery loss is unknown, never a replay.
    pub async fn apply_object_ddl(
        &self,
        review: ObjectDdlReview,
    ) -> Result<ObjectDdlSubmission, ObjectDdlError> {
        self.submit_object_ddl(review, false, native_object_ddl::execute, load)
            .await
    }

    /// Same durable exact-attempt barrier as Apply, with a fresh policy check.
    pub async fn confirm_object_ddl(
        &self,
        confirmation: ObjectDdlConfirmation,
    ) -> Result<ObjectDdlSubmission, ObjectDdlError> {
        self.submit_object_ddl(confirmation.review, true, native_object_ddl::execute, load)
            .await
    }

    async fn submit_object_ddl<F, Fut>(
        &self,
        review: ObjectDdlReview,
        confirmed: bool,
        execute: F,
        loader: impl FnOnce(
                Arc<crate::app::AppState>,
                String,
            ) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
            + Send
            + 'static,
    ) -> Result<ObjectDdlSubmission, ObjectDdlError>
    where
        F: FnOnce(
                ResolvedPostgresConnectSpec,
                DriverJoins,
                WritePermit,
                watch::Receiver<u64>,
                ObjectDdlDescription,
                Vec<ObjectDdlOperation>,
                ObjectDdlPreview,
            ) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = ObjectDdlOutcome> + Send + 'static,
    {
        let document = review.target.document.clone();
        if !self.0.documents.owns(&document) {
            return Err(ObjectDdlError::ForeignDocument);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                if !review.target.current() {
                    return Err(ObjectDdlError::Unavailable);
                }
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| ObjectDdlError::Storage)?
                        .ok_or(ObjectDdlError::Unavailable)?;
                let crate::StoredConnection::PostgreSQL(pg) = &stored else {
                    return Err(ObjectDdlError::Unavailable);
                };
                let timeout = pg
                    .driver_options
                    .as_ref()
                    .and_then(|o| o.statement_timeout_ms);
                let policy = gate::resolved_policy(&stored);
                if assert_permitted(&policy, &WriteIntent::Ddl, true).is_err() {
                    return Err(ObjectDdlError::PolicyBlocked);
                }
                if requires_confirmation(&policy, &WriteIntent::Ddl)
                    != review.preview.confirmation_required
                {
                    return Err(ObjectDdlError::PolicyChanged);
                }
                let generated = preview::render(
                    &review.target.description,
                    &review.target.operations,
                    timeout,
                    review.preview.confirmation_required,
                )?;
                if generated != review.preview {
                    return Err(ObjectDdlError::InvalidRequest);
                }
                let authorization = match assert_permitted(&policy, &WriteIntent::Ddl, confirmed) {
                    Ok(value) => value,
                    Err(refusal) => {
                        return refusal.fold(
                            |_, _| Err(ObjectDdlError::PolicyBlocked),
                            |_| {
                                Ok(ObjectDdlSubmission::NeedsConfirmation(Box::new(
                                    ObjectDdlConfirmation { review },
                                )))
                            },
                        )
                    }
                };
                let permit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| ObjectDdlError::Busy)?;
                let cancellation = document.0.read_cancellation();
                // Cancellation after policy and before this lease must not be lost.
                if !review.target.current() || !permit.check_preparing() {
                    return Err(ObjectDdlError::Unavailable);
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
                            review.target.operations.clone(),
                            generated,
                        )
                        .await
                    }
                    _ => ObjectDdlOutcome::NotDispatched {
                        reason: ObjectDdlFailure::Connection,
                    },
                };
                // Groups commit independently: audit any committed or possibly
                // committed statement, not only a complete success.
                if outcome.may_have_changed() {
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
                Ok(ObjectDdlSubmission::Finished(Box::new(ObjectDdlReceipt {
                    attempt_id: review.attempt_id,
                    connection_id: document.0.connection.clone(),
                    target: review.target.description,
                    operations: review.target.operations,
                    outcome,
                })))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| ObjectDdlError::OutcomeUnavailable)?
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
/// The pure renderer used by review and recovery validation. A description
/// is never authority: only a live [`ObjectDdlReview`] can be submitted.
pub fn render_preview(
    target: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    timeout: Option<u32>,
    confirmation_required: bool,
) -> Result<ObjectDdlPreview, ObjectDdlError> {
    preview::render(target, operations, timeout, confirmation_required)
}
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod live_tests;
