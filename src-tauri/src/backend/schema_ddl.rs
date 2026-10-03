//! Native create-schema intent, pure review and owned transactional submission.
//! No legacy pooled execution or caller-supplied DDL is exposed.
mod types;
pub(crate) use super::data_documents::WritePermit;
use super::{data::DataDocument, Backend};
use crate::postgres::objects::{PgObjectKind, PgObjectRef};
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec,
    native_schema_ddl,
    object_ddl::{self, CreateSchemaOp, PgCommentTarget, PgObjectOp, SetCommentOp},
};
use crate::safety::{
    gate,
    policy::{assert_permitted, AuditDisposition, WriteIntent},
};
use std::{mem::size_of, sync::Arc};
pub use types::*;

pub struct CreateSchemaReview {
    document: DataDocument,
    intent: CreateSchemaIntent,
    attempt_id: CreateSchemaAttemptId,
    preview: CreateSchemaPreview,
}
impl CreateSchemaReview {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn intent(&self) -> &CreateSchemaIntent {
        &self.intent
    }
    pub fn attempt_id(&self) -> &CreateSchemaAttemptId {
        &self.attempt_id
    }
    pub fn preview(&self) -> &CreateSchemaPreview {
        &self.preview
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.intent.checked_heap_bytes().unwrap_or(usize::MAX))
            .saturating_add(self.preview.checked_heap_bytes().unwrap_or(usize::MAX))
            .saturating_add(self.attempt_id.as_str().len())
            .saturating_add(std::mem::size_of_val(&*self.document.0))
            .saturating_add(self.document.0.window.capacity())
            .saturating_add(self.document.0.tab.capacity())
            .saturating_add(self.document.0.connection.capacity())
            .saturating_add(self.document.0.manager_tab.capacity())
    }
}
pub struct CreateSchemaConfirmation {
    review: CreateSchemaReview,
}
impl CreateSchemaConfirmation {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn intent(&self) -> &CreateSchemaIntent {
        self.review.intent()
    }
    pub fn attempt_id(&self) -> &CreateSchemaAttemptId {
        self.review.attempt_id()
    }
    pub fn preview(&self) -> &CreateSchemaPreview {
        self.review.preview()
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum CreateSchemaSubmission {
    NeedsConfirmation(Box<CreateSchemaConfirmation>),
    Finished(CreateSchemaReceipt),
}

pub(crate) fn encoded_bytes(value: &impl serde::Serialize) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("encoded size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    if serde_json::to_writer(&mut count, value).is_ok() {
        count.0
    } else {
        usize::MAX
    }
}

fn preview(intent: &CreateSchemaIntent) -> Result<CreateSchemaPreview, CreateSchemaError> {
    // Count the actual JSON escape expansion of quoted input before allocating
    // operations or SQL. The fixed reserve covers SQL syntax, summaries, DTO
    // layout and JSON keys. Repeated identifiers are conservatively overcounted.
    let maximum = 512
        + quoted_json_bytes(intent.name(), true) * 4
        + intent
            .comment()
            .map_or(0, |text| quoted_json_bytes(text, false));
    if maximum > MAX_SCHEMA_PREVIEW_BYTES || intent.checked_heap_bytes().is_none() {
        return Err(CreateSchemaError::InvalidPreview);
    }
    let mut ops = Vec::with_capacity(MAX_SCHEMA_STATEMENTS);
    ops.push(PgObjectOp::CreateSchema(CreateSchemaOp {
        name: intent.name().into(),
    }));
    if let Some(comment) = intent.comment() {
        ops.push(PgObjectOp::SetComment(SetCommentOp {
            target: PgCommentTarget::Object {
                reference: PgObjectRef {
                    kind: PgObjectKind::Schema,
                    schema: None,
                    name: intent.name().into(),
                    identity_args: None,
                },
            },
            comment: Some(comment.into()),
        }));
    }
    let rendered =
        object_ddl::generate_object_ddl(&ops).map_err(|_| CreateSchemaError::InvalidPreview)?;
    let statements = rendered
        .statements
        .into_iter()
        .map(|statement| CreateSchemaStatement {
            sql: statement.sql.into_boxed_str().into_string(),
            summary: statement.summary.into_boxed_str().into_string(),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
        .into_vec();
    let preview = CreateSchemaPreview { statements };
    preview
        .checked_heap_bytes()
        .ok_or(CreateSchemaError::InvalidPreview)?;
    Ok(preview)
}

fn quoted_json_bytes(text: &str, identifier: bool) -> usize {
    let mut bytes = if identifier { 4 } else { 3 }; // JSON-escaped quotes or E''.
    for byte in text.bytes() {
        bytes += match byte {
            b'"' if identifier => 4,   // doubled SQL quotes, each JSON escaped
            b'\\' if !identifier => 4, // doubled E-string slash, each JSON escaped
            b'\'' if !identifier => 2,
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        };
    }
    bytes
}

impl Backend {
    /// Pure SQL generation against admitted stored metadata. Never hydrates a
    /// credential or opens a PostgreSQL socket. Opening a review is not Apply.
    pub async fn review_create_schema(
        &self,
        document: &DataDocument,
        intent: CreateSchemaIntent,
    ) -> Result<CreateSchemaReview, CreateSchemaError> {
        self.data_call(document, move |state, document, _admission| async move {
            let result = async {
                let connection =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| CreateSchemaError::Storage)?
                        .ok_or(CreateSchemaError::Unavailable)?;
                if connection.engine() != crate::DatabaseEngine::PostgreSQL {
                    return Err(CreateSchemaError::Unavailable);
                }
                let preview = preview(&intent)?;
                Ok(CreateSchemaReview {
                    document,
                    intent,
                    attempt_id: CreateSchemaAttemptId::new(),
                    preview,
                })
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| CreateSchemaError::Unavailable)?
    }

    /// Consumes exactly the reviewed intent. Persist its attempt as unknown and
    /// await the exact journal ACK before calling. All errors except
    /// OutcomeUnavailable are pre-execution refusals; lost task/delivery cannot
    /// establish whether COMMIT ran. Cancellation is settled by a receipt.
    pub async fn apply_create_schema(
        &self,
        review: CreateSchemaReview,
    ) -> Result<CreateSchemaSubmission, CreateSchemaError> {
        self.submit_create_schema(review, false).await
    }
    /// Rechecks the current profile-local policy; confirmation cannot change
    /// the reviewed intent, attempt or document. Same error contract as Apply.
    pub async fn confirm_create_schema(
        &self,
        confirmation: CreateSchemaConfirmation,
    ) -> Result<CreateSchemaSubmission, CreateSchemaError> {
        self.submit_create_schema(confirmation.review, true).await
    }
    async fn submit_create_schema(
        &self,
        review: CreateSchemaReview,
        confirmed: bool,
    ) -> Result<CreateSchemaSubmission, CreateSchemaError> {
        let document = review.document.clone();
        let inner = self.0.clone();
        self.data_call(&document, move |state, document, admission| async move {
            let result = async {
                let stored =
                    crate::storage::read_connection_by_id(&state.pool, &document.0.connection)
                        .await
                        .map_err(|_| CreateSchemaError::Storage)?
                        .ok_or(CreateSchemaError::Unavailable)?;
                let generated = preview(&review.intent)?;
                if generated != review.preview {
                    return Err(CreateSchemaError::InvalidPreview);
                }
                let authorization = match assert_permitted(
                    &gate::resolved_policy(&stored),
                    &WriteIntent::Ddl,
                    confirmed,
                ) {
                    Ok(authorization) => authorization,
                    Err(refusal) => {
                        return refusal.fold(
                            |_, _| Err(CreateSchemaError::PolicyBlocked),
                            |_| {
                                Ok(CreateSchemaSubmission::NeedsConfirmation(Box::new(
                                    CreateSchemaConfirmation { review },
                                )))
                            },
                        )
                    }
                };
                let permit = inner
                    .documents
                    .begin_write(&document)
                    .map_err(|_| CreateSchemaError::Busy)?;
                let cancellation = document.0.read_cancellation();
                let spec = crate::app::find_connection(&state, &document.0.connection)
                    .await
                    .ok()
                    .and_then(|connection| {
                        ResolvedPostgresConnectSpec::from_connection(&connection).ok()
                    });
                let drivers = inner.tasks.child();
                // The permit/cancellation fence owns startup and remains held
                // until all child drivers join, independent of the UI waiter.
                drop(admission);
                let outcome = match spec {
                    Some(spec) => {
                        native_schema_ddl::execute(
                            &spec,
                            &drivers,
                            &permit,
                            cancellation,
                            &generated,
                        )
                        .await
                    }
                    None => CreateSchemaOutcome::NotApplied {
                        reason: CreateSchemaFailure::Connection,
                    },
                };
                if matches!(outcome, CreateSchemaOutcome::Applied { .. }) {
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
                Ok(CreateSchemaSubmission::Finished(CreateSchemaReceipt {
                    attempt_id: review.attempt_id,
                    connection_id: document.0.connection.clone(),
                    intent: review.intent,
                    outcome,
                }))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| CreateSchemaError::OutcomeUnavailable)?
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
