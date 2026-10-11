//! Plan 033 facade: observe, review, apply and confirm one table's schema
//! edit on PostgreSQL, MySQL or SQLite (ADR-0016).
//!
//! The data model, `diff` and the preview are shared ([`crate::schema_edit`]);
//! capture, planning and execution belong to each engine and are routed here
//! with a `match` on the source. Lifecycle, per engine:
//!
//! 1. `observe_schema_edit` captures a [`SchemaBaseline`] (authority stays
//!    tied to the source: the PostgreSQL document generation, the SQLite or
//!    MySQL session).
//! 2. `review_schema_edit` validates and plans the draft into a
//!    [`SchemaEditPreview`]. No SQL comes from the caller (ADR-0011).
//! 3. The host journals the attempt durably ([`WorkspaceSchemaEdit`]) and
//!    awaits the exact revision before calling `apply_schema_edit`.
//! 4. Apply re-plans and refuses unless the preview is identical, asserts
//!    `WriteIntent::Ddl`, then the engine rechecks drift, executes and
//!    verifies by re-introspecting.
use super::data_documents::WritePermit;
use super::{
    data::DataDocument, mysql_sessions::MySqlSession, sqlite_session::SqliteSession,
    sqlite_session::SqliteSessionError, Backend,
};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::safety::{
    gate,
    policy::{assert_permitted, AuditDisposition, WriteIntent},
};
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

pub use super::mysql_sessions::MySqlSessionError;
pub use crate::backend::schema_ddl::CreateSchemaAttemptId as SchemaEditAttemptId;
pub use crate::schema_edit::*;

/// Where a schema edit runs. Cloning shares the same document or session.
#[derive(Clone)]
pub enum SchemaEditSource {
    Postgres(DataDocument),
    Sqlite(Arc<SqliteSession>),
    MySql(MySqlSession),
}

impl SchemaEditSource {
    pub fn engine(&self) -> SchemaEngine {
        match self {
            Self::Postgres(_) => SchemaEngine::Postgres,
            Self::Sqlite(_) => SchemaEngine::Sqlite,
            Self::MySql(_) => SchemaEngine::MySql,
        }
    }

    pub fn connection_id(&self) -> String {
        match self {
            Self::Postgres(document) => document.0.connection.clone(),
            Self::Sqlite(session) => session.info().connection_id.clone(),
            Self::MySql(session) => session.connection_id().to_owned(),
        }
    }

    /// True when both name the same document or session.
    pub fn same(&self, other: &SchemaEditSource) -> bool {
        match (self, other) {
            (Self::Postgres(a), Self::Postgres(b)) => Arc::ptr_eq(&a.0, &b.0),
            (Self::Sqlite(a), Self::Sqlite(b)) => Arc::ptr_eq(a, b),
            (Self::MySql(a), Self::MySql(b)) => a.same_session(b),
            _ => false,
        }
    }
}

impl fmt::Debug for SchemaEditSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SchemaEditSource({:?})", self.engine())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaEditError {
    /// The draft cannot be planned (invalid, read-only, unsupported, no
    /// changes).
    Plan(PlanError),
    /// Capture or the source failed; user-facing text.
    Unavailable(String),
    Busy,
    Storage,
    /// The stored policy refuses DDL on this connection.
    PolicyBlocked(String),
    /// Apply re-planned a different preview than the one reviewed (the
    /// timeout, options or planner changed). Review again.
    PreviewChanged,
    /// The review belongs to another backend.
    ForeignSource,
    /// The outcome could not be delivered; reconcile before another attempt.
    OutcomeUnavailable,
}

impl fmt::Display for SchemaEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(f),
            Self::Unavailable(reason) | Self::PolicyBlocked(reason) => f.write_str(reason),
            Self::Busy => {
                f.write_str("Another schema or maintenance operation owns this connection")
            }
            Self::Storage => f.write_str("Stored connection is unavailable"),
            Self::PreviewChanged => {
                f.write_str("The planned change differs from the reviewed one; review again")
            }
            Self::ForeignSource => f.write_str("The schema edit belongs to a different backend"),
            Self::OutcomeUnavailable => f.write_str(
                "Schema change outcome unavailable; reload the structure before another attempt",
            ),
        }
    }
}

impl std::error::Error for SchemaEditError {}

impl From<PlanError> for SchemaEditError {
    fn from(error: PlanError) -> Self {
        Self::Plan(error)
    }
}

/// A captured baseline, bound to its source. Not serializable: restoring a
/// journal never restores authority.
pub struct SchemaEditTarget {
    source: SchemaEditSource,
    request: SchemaEditRequest,
    baseline: SchemaBaseline,
    /// PostgreSQL only: the document cancellation generation at capture.
    generation: Option<u64>,
}

impl SchemaEditTarget {
    pub fn source(&self) -> &SchemaEditSource {
        &self.source
    }
    pub fn request(&self) -> &SchemaEditRequest {
        &self.request
    }
    pub fn baseline(&self) -> &SchemaBaseline {
        &self.baseline
    }
    /// A fresh draft of this baseline.
    pub fn draft(&self) -> SchemaDraft {
        SchemaDraft::from_baseline(&self.baseline)
    }
    fn current(&self) -> bool {
        match (&self.source, self.generation) {
            (SchemaEditSource::Postgres(document), Some(generation)) => {
                document.0.check_open().is_ok()
                    && *document.0.read_cancellation().borrow() == generation
            }
            (SchemaEditSource::Postgres(_), None) => false,
            (SchemaEditSource::Sqlite(session), _) => !session.is_closed(),
            (SchemaEditSource::MySql(session), _) => {
                matches!(
                    session.status(),
                    super::mysql_sessions::MySqlSessionStatus::Open
                )
            }
        }
    }
}

impl fmt::Debug for SchemaEditTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaEditTarget")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

pub struct SchemaEditReview {
    target: SchemaEditTarget,
    draft: SchemaDraft,
    preview: SchemaEditPreview,
    attempt_id: SchemaEditAttemptId,
}

impl SchemaEditReview {
    pub fn target(&self) -> &SchemaEditTarget {
        &self.target
    }
    pub fn baseline(&self) -> &SchemaBaseline {
        &self.target.baseline
    }
    pub fn draft(&self) -> &SchemaDraft {
        &self.draft
    }
    pub fn preview(&self) -> &SchemaEditPreview {
        &self.preview
    }
    pub fn attempt_id(&self) -> &SchemaEditAttemptId {
        &self.attempt_id
    }
    /// Gives the target back (to edit the draft further and review again).
    pub fn into_target(self) -> SchemaEditTarget {
        self.target
    }
}

/// A review that could not be planned. The target comes back so the host
/// can keep editing without capturing again.
#[derive(Debug)]
pub struct ReviewRefusal {
    pub target: SchemaEditTarget,
    pub error: SchemaEditError,
}

impl ReviewRefusal {
    fn new(target: SchemaEditTarget, error: SchemaEditError) -> Box<Self> {
        Box::new(Self { target, error })
    }
}

pub struct SchemaEditConfirmation {
    review: SchemaEditReview,
}

impl SchemaEditConfirmation {
    pub fn review(&self) -> &SchemaEditReview {
        &self.review
    }
    pub fn into_review(self) -> SchemaEditReview {
        self.review
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaEditReceipt {
    pub attempt_id: SchemaEditAttemptId,
    pub engine: SchemaEngine,
    pub connection_id: String,
    pub request: SchemaEditRequest,
    pub outcome: SchemaEditOutcome,
}

pub enum SchemaEditSubmission {
    NeedsConfirmation(Box<SchemaEditConfirmation>),
    Finished(Box<SchemaEditReceipt>),
}

fn sqlite_error(error: SqliteSessionError) -> SchemaEditError {
    match error {
        SqliteSessionError::Busy => SchemaEditError::Busy,
        other => SchemaEditError::Unavailable(other.to_string()),
    }
}

fn mysql_error(error: MySqlSessionError) -> SchemaEditError {
    match error {
        MySqlSessionError::Busy => SchemaEditError::Busy,
        MySqlSessionError::ReadOnly(reason) => SchemaEditError::PolicyBlocked(reason),
        other => SchemaEditError::Unavailable(other.to_string()),
    }
}

/// Session-level outcome failures that mean "nothing was sent".
fn session_outcome(
    review: SchemaEditReview,
    connection_id: String,
    result: crate::schema_edit::SessionApply,
) -> Result<SchemaEditSubmission, SchemaEditError> {
    use crate::schema_edit::SessionApply;
    match result {
        SessionApply::NeedsConfirmation => Ok(SchemaEditSubmission::NeedsConfirmation(Box::new(
            SchemaEditConfirmation { review },
        ))),
        SessionApply::Blocked(reason) => Err(SchemaEditError::PolicyBlocked(reason)),
        SessionApply::Finished(outcome) => Ok(SchemaEditSubmission::Finished(Box::new(
            SchemaEditReceipt {
                attempt_id: review.attempt_id,
                engine: review.target.baseline.engine,
                connection_id,
                request: review.target.request,
                outcome,
            },
        ))),
    }
}

impl Backend {
    /// Captures one table's complete structure as a baseline. A baseline that
    /// cannot be edited (old server, unparseable definition, catalog
    /// mismatch) is returned `ReadOnly`, not as an error.
    pub async fn observe_schema_edit(
        &self,
        source: SchemaEditSource,
        request: SchemaEditRequest,
    ) -> Result<SchemaEditTarget, SchemaEditError> {
        request.validate()?;
        match &source {
            SchemaEditSource::Postgres(document) => {
                let generation = *document.0.read_cancellation().borrow();
                let captured = request.clone();
                let baseline = self
                    .object_read(document, move |spec, drivers, cancellation| async move {
                        crate::schema_edit::postgres::capture(
                            &spec,
                            &drivers,
                            cancellation,
                            captured,
                        )
                        .await
                    })
                    .await
                    .map_err(|error| SchemaEditError::Unavailable(capture_message(&error)))?;
                let target = SchemaEditTarget {
                    source,
                    request,
                    baseline,
                    generation: Some(generation),
                };
                if !target.current() {
                    return Err(SchemaEditError::Unavailable(
                        "The structure changed while it was read; reload".into(),
                    ));
                }
                Ok(target)
            }
            SchemaEditSource::Sqlite(session) => {
                let baseline = session
                    .schema_baseline(
                        session.ticket(),
                        request.schema.clone(),
                        request.table.clone(),
                    )
                    .await
                    .map_err(sqlite_error)?;
                Ok(SchemaEditTarget {
                    source,
                    request,
                    baseline,
                    generation: None,
                })
            }
            SchemaEditSource::MySql(session) => {
                let baseline = session
                    .schema_baseline(request.schema.clone(), request.table.clone())
                    .await
                    .map_err(mysql_error)?;
                Ok(SchemaEditTarget {
                    source,
                    request,
                    baseline,
                    generation: None,
                })
            }
        }
    }

    /// Validates and plans `draft`. Consumes the target; a refusal returns
    /// it with the error so the host can keep editing.
    pub fn review_schema_edit(
        &self,
        target: SchemaEditTarget,
        draft: SchemaDraft,
        options: PlanOptions,
    ) -> Result<SchemaEditReview, Box<ReviewRefusal>> {
        if let SchemaEditSource::Postgres(document) = &target.source {
            if !self.0.documents.owns(document) {
                return Err(ReviewRefusal::new(target, SchemaEditError::ForeignSource));
            }
        }
        if !target.current() {
            return Err(ReviewRefusal::new(
                target,
                SchemaEditError::Unavailable("The structure is no longer current; reload".into()),
            ));
        }
        match plan_review(&target.baseline, &draft, &options) {
            Ok(preview) => Ok(SchemaEditReview {
                target,
                draft,
                preview,
                attempt_id: SchemaEditAttemptId::new(),
            }),
            Err(error) => Err(ReviewRefusal::new(target, SchemaEditError::Plan(error))),
        }
    }

    /// The caller MUST durably journal this exact attempt as uncertain and
    /// await that revision before calling. Consumes authority once; a lost
    /// reply is unknown, never permission to replay.
    pub async fn apply_schema_edit(
        &self,
        review: SchemaEditReview,
    ) -> Result<SchemaEditSubmission, SchemaEditError> {
        self.submit_schema_edit(review, false).await
    }

    /// Same journal barrier as apply, with the policy confirmation given.
    pub async fn confirm_schema_edit(
        &self,
        confirmation: SchemaEditConfirmation,
    ) -> Result<SchemaEditSubmission, SchemaEditError> {
        self.submit_schema_edit(confirmation.review, true).await
    }

    async fn submit_schema_edit(
        &self,
        review: SchemaEditReview,
        confirmed: bool,
    ) -> Result<SchemaEditSubmission, SchemaEditError> {
        if !review.target.current() {
            return Err(SchemaEditError::Unavailable(
                "The structure is no longer current; reload".into(),
            ));
        }
        // Re-render and compare: the reviewed SQL is exactly what runs.
        let replanned = plan_review(
            &review.target.baseline,
            &review.draft,
            &review.preview.options,
        )?;
        if replanned != review.preview {
            return Err(SchemaEditError::PreviewChanged);
        }
        let connection_id = review.target.source.connection_id();
        match review.target.source.clone() {
            SchemaEditSource::Postgres(document) => {
                self.submit_postgres(document, review, confirmed, load_postgres)
                    .await
            }
            SchemaEditSource::Sqlite(session) => {
                let result = session
                    .apply_schema_edit(
                        session.ticket(),
                        review.target.baseline.clone(),
                        review.draft.clone(),
                        review.preview.clone(),
                        confirmed,
                    )
                    .await
                    .map_err(sqlite_error)?;
                session_outcome(review, connection_id, result)
            }
            SchemaEditSource::MySql(session) => {
                let result = session
                    .apply_schema_edit(
                        review.target.baseline.clone(),
                        review.draft.clone(),
                        review.preview.clone(),
                        confirmed,
                    )
                    .await
                    .map_err(mysql_error)?;
                session_outcome(review, connection_id, result)
            }
        }
    }

    async fn submit_postgres(
        &self,
        document: DataDocument,
        review: SchemaEditReview,
        confirmed: bool,
        loader: impl FnOnce(
                Arc<crate::app::AppState>,
                String,
            ) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
            + Send
            + 'static,
    ) -> Result<SchemaEditSubmission, SchemaEditError> {
        if !self.0.documents.owns(&document) {
            return Err(SchemaEditError::ForeignSource);
        }
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                if !review.target.current() {
                    return Err(SchemaEditError::Unavailable(
                        "The structure is no longer current; reload".into(),
                    ));
                }
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| SchemaEditError::Storage)?
                        .ok_or(SchemaEditError::Storage)?;
                if !matches!(stored, crate::StoredConnection::PostgreSQL(_)) {
                    return Err(SchemaEditError::Storage);
                }
                let authorization = match assert_permitted(
                    &gate::resolved_policy(&stored),
                    &WriteIntent::Ddl,
                    confirmed,
                ) {
                    Ok(value) => value,
                    Err(refusal) => {
                        return refusal.fold(
                            |reason, _| Err(SchemaEditError::PolicyBlocked(reason.to_string())),
                            |_| {
                                Ok(SchemaEditSubmission::NeedsConfirmation(Box::new(
                                    SchemaEditConfirmation { review },
                                )))
                            },
                        )
                    }
                };
                let permit: WritePermit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| SchemaEditError::Busy)?;
                let cancellation = document.0.read_cancellation();
                if !review.target.current() || !permit.check_preparing() {
                    return Err(SchemaEditError::Unavailable(
                        "The structure is no longer current; reload".into(),
                    ));
                }
                let spec = loader(state.clone(), document.0.connection.clone()).await;
                let drivers = inner.tasks.child();
                drop(admission);
                let outcome = match spec {
                    Some(spec) => {
                        crate::schema_edit::postgres::execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            review.target.baseline.clone(),
                            review.draft.clone(),
                            review.preview.clone(),
                        )
                        .await
                    }
                    None => SchemaEditOutcome::NotDispatched {
                        reason: SchemaEditFailure::Connection,
                    },
                };
                if outcome.may_have_changed() {
                    if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
                        gate::record_override(
                            &state.pool,
                            &document.0.connection,
                            "apply_schema_edit",
                            &WriteIntent::Ddl,
                        )
                        .await;
                    }
                    crate::app::touch_connection_activity(&state, &document.0.connection).await;
                }
                Ok(SchemaEditSubmission::Finished(Box::new(
                    SchemaEditReceipt {
                        attempt_id: review.attempt_id,
                        engine: SchemaEngine::Postgres,
                        connection_id: document.0.connection.clone(),
                        request: review.target.request,
                        outcome,
                    },
                )))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| SchemaEditError::OutcomeUnavailable)?
    }
}

fn capture_message(error: &super::data::DataError) -> String {
    use super::data::DataError;
    use super::objects::CatalogError;
    match error {
        DataError::Catalog(CatalogError::ObjectNotFound) => "The table no longer exists".into(),
        DataError::Catalog(CatalogError::StructurePermission) => {
            "This account cannot read the table's structure".into()
        }
        DataError::Catalog(CatalogError::UnsupportedObjectKind) => {
            "Schema editing is not available for this table".into()
        }
        DataError::Catalog(CatalogError::Cancelled) => "Cancelled".into(),
        DataError::Catalog(CatalogError::Timeout) => "Reading the structure timed out".into(),
        _ => "The table structure could not be read".into(),
    }
}

fn load_postgres(
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

/// Journal record for one schema edit (ADR-0016). Descriptive recovery
/// only: restoring it restores the draft for editing, never authority. The
/// attempt is saved `OutcomeUnknown` before dispatch.
pub use super::development::{WorkspaceSchemaEdit, WorkspaceSchemaEditAttempt};

#[cfg(test)]
mod tests;
