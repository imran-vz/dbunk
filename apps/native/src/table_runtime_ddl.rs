//! Table and existing-schema DDL adapter for the existing document worker. No
//! persistence authority. Both families share the three reply variants, so
//! exhaustive matches elsewhere route them as one Objects-owned lane.
use super::*;
use dbunk_lib::backend::object_ddl::{
    ObjectDdlError, ObjectDdlReview, ObjectDdlSubmission as ObjectSubmission, ObjectDdlTarget,
};
use dbunk_lib::backend::schema_alter::{
    SchemaAlterConfirmation, SchemaAlterDescription, SchemaAlterError, SchemaAlterRequest,
    SchemaAlterReview, SchemaAlterSubmission, SchemaAlterTarget,
};
use dbunk_lib::backend::table_ddl::{TableDdlDescription, TableDdlError, TableDdlRequest};

/// Objects-owned DDL replies share these three message variants; each consumer
/// matches only its own payload family.
pub enum DdlObserved {
    Table(Result<Box<TableDdlTarget>, Arc<TableDdlError>>),
    Schema(Result<Box<SchemaAlterTarget>, Arc<SchemaAlterError>>),
    Object(Result<Box<ObjectDdlTarget>, Arc<ObjectDdlError>>),
}
pub enum DdlReviewed {
    Table(Result<Box<TableDdlReview>, Arc<TableDdlError>>),
    Schema(Result<Box<SchemaAlterReview>, Arc<SchemaAlterError>>),
    Object(Result<Box<ObjectDdlReview>, Arc<ObjectDdlError>>),
}
pub enum DdlApplied {
    Table(Result<Box<TableDdlSubmission>, Arc<TableDdlError>>),
    Schema(Result<Box<SchemaAlterSubmission>, Arc<SchemaAlterError>>),
    Object(Result<Box<ObjectSubmission>, Arc<ObjectDdlError>>),
}

pub(super) const RESPONSE_BYTES: usize = 64 * 1024;

/// An attnum must come from the selected Structure capture, never a display row
/// index. Reusing a dropped column's name does not reuse its identity.
fn selected_matches(
    request: &TableDdlRequest,
    expected_attnum: Option<i16>,
    observed: &TableDdlDescription,
) -> bool {
    observed.checked_heap_bytes().is_some()
        && observed.schema == request.schema
        && observed.table == request.table
        && request
            .expected
            .is_none_or(|identity| identity == observed.identity)
        && match (&request.column, &observed.column, expected_attnum) {
            (None, None, None) => true,
            (Some(name), Some(column), expected) => {
                *name == column.name
                    && expected.is_none_or(|attnum| attnum > 0 && attnum == column.attnum)
            }
            _ => false,
        }
}

pub(super) async fn request(
    backend: &Backend,
    document: &DataDocument,
    command: TableCommand,
) -> TableMessage {
    match command {
        TableCommand::TableDdlObserve(id, request, expected_attnum) => {
            let result = async {
                request.validate()?;
                if expected_attnum.is_some_and(|attnum| attnum <= 0 || request.column.is_none()) {
                    return Err(TableDdlError::InvalidTarget);
                }
                // Request is already bounded to three identifiers before cloning.
                let target = backend.observe_table_ddl(document, request.clone()).await?;
                if !target.belongs_to(document)
                    || !selected_matches(&request, expected_attnum, target.description())
                    || target.retained_bytes().max(target.encoded_bytes()) > RESPONSE_BYTES / 2
                {
                    return Err(TableDdlError::InvalidTarget);
                }
                Ok(Box::new(target))
            }
            .await;
            TableMessage::TableDdlObserved(id, DdlObserved::Table(result.map_err(Arc::new)))
        }
        TableCommand::TableDdlReview(id, target, intent) => {
            let result = if !target.belongs_to(document) {
                Err(TableDdlError::ForeignDocument)
            } else if intent.checked_heap_bytes().is_none()
                || target.retained_bytes() > RESPONSE_BYTES / 2
            {
                Err(TableDdlError::InvalidTarget)
            } else {
                backend
                    .review_table_ddl(*target, intent)
                    .await
                    .map(Box::new)
            };
            TableMessage::TableDdlReviewed(id, DdlReviewed::Table(result.map_err(Arc::new)))
        }
        TableCommand::TableDdlApply(id, review) => {
            let result = if !review.belongs_to(document) {
                Err(TableDdlError::ForeignDocument)
            } else if review.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(TableDdlError::InvalidTarget)
            } else {
                backend.apply_table_ddl(*review).await.map(Box::new)
            };
            TableMessage::TableDdlApplied(id, DdlApplied::Table(result.map_err(Arc::new)))
        }
        TableCommand::TableDdlConfirm(id, confirmation) => {
            let result = if !confirmation.belongs_to(document) {
                Err(TableDdlError::ForeignDocument)
            } else if confirmation.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(TableDdlError::InvalidTarget)
            } else {
                backend.confirm_table_ddl(*confirmation).await.map(Box::new)
            };
            TableMessage::TableDdlApplied(id, DdlApplied::Table(result.map_err(Arc::new)))
        }
        TableCommand::SchemaAlterObserve(id, request) => {
            let result = async {
                request.validate()?;
                let target = backend
                    .observe_schema_alter(document, request.clone())
                    .await?;
                if !target.belongs_to(document)
                    || !schema_matches(&request, target.description())
                    || target.retained_bytes().max(target.encoded_bytes()) > RESPONSE_BYTES / 2
                {
                    return Err(SchemaAlterError::InvalidTarget);
                }
                Ok(Box::new(target))
            }
            .await;
            TableMessage::TableDdlObserved(id, DdlObserved::Schema(result.map_err(Arc::new)))
        }
        TableCommand::SchemaAlterReview(id, target, intent) => {
            let result = if !target.belongs_to(document) {
                Err(SchemaAlterError::ForeignDocument)
            } else if intent.checked_heap_bytes().is_none()
                || target.retained_bytes() > RESPONSE_BYTES / 2
            {
                Err(SchemaAlterError::InvalidTarget)
            } else {
                backend
                    .review_schema_alter(*target, intent)
                    .await
                    .map(Box::new)
            };
            TableMessage::TableDdlReviewed(id, DdlReviewed::Schema(result.map_err(Arc::new)))
        }
        TableCommand::SchemaAlterApply(id, review) => {
            let result = if !review.belongs_to(document) {
                Err(SchemaAlterError::ForeignDocument)
            } else if review.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(SchemaAlterError::InvalidTarget)
            } else {
                backend.apply_schema_alter(*review).await.map(Box::new)
            };
            TableMessage::TableDdlApplied(id, DdlApplied::Schema(result.map_err(Arc::new)))
        }
        TableCommand::SchemaAlterConfirm(id, confirmation) => {
            let result = if !confirmation.belongs_to(document) {
                Err(SchemaAlterError::ForeignDocument)
            } else if confirmation.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(SchemaAlterError::InvalidTarget)
            } else {
                backend
                    .confirm_schema_alter(*confirmation)
                    .await
                    .map(Box::new)
            };
            TableMessage::TableDdlApplied(id, DdlApplied::Schema(result.map_err(Arc::new)))
        }
        _ => unreachable!("Only typed table or schema DDL commands enter this adapter"),
    }
}

/// The observed name must be the selected name; once pinned, the OID must
/// match. A recreated schema reusing the name is a different identity.
fn schema_matches(request: &SchemaAlterRequest, observed: &SchemaAlterDescription) -> bool {
    observed.checked_heap_bytes().is_some()
        && observed.schema == request.schema
        && request
            .expected
            .is_none_or(|identity| identity == observed.identity)
}

fn schema_review_bytes(review: &SchemaAlterReview) -> usize {
    review.retained_bytes().max(
        review
            .target()
            .encoded_bytes()
            .saturating_add(encoded_size(review.intent()))
            .saturating_add(encoded_size(review.preview()))
            .saturating_add(128),
    )
}
fn schema_confirmation_bytes(confirmation: &SchemaAlterConfirmation) -> usize {
    confirmation
        .retained_bytes()
        .max(schema_review_bytes(confirmation.review()))
}

pub(super) fn message_bytes(message: &TableMessage) -> usize {
    match message {
        TableMessage::TableDdlObserved(_, DdlObserved::Table(Ok(target))) => {
            target.retained_bytes().max(target.encoded_bytes())
        }
        TableMessage::TableDdlObserved(_, DdlObserved::Schema(Ok(target))) => {
            target.retained_bytes().max(target.encoded_bytes())
        }
        TableMessage::TableDdlReviewed(_, DdlReviewed::Table(Ok(review))) => {
            review.retained_bytes().max(
                review
                    .target()
                    .encoded_bytes()
                    .saturating_add(encoded_size(review.intent()))
                    .saturating_add(encoded_size(review.preview()))
                    .saturating_add(128),
            )
        }
        TableMessage::TableDdlReviewed(_, DdlReviewed::Schema(Ok(review))) => {
            schema_review_bytes(review)
        }
        TableMessage::TableDdlApplied(_, DdlApplied::Table(Ok(submission))) => {
            match submission.as_ref() {
                TableDdlSubmission::NeedsConfirmation(confirmation) => {
                    let review = confirmation.review();
                    confirmation.retained_bytes().max(
                        review
                            .target()
                            .encoded_bytes()
                            .saturating_add(encoded_size(review.intent()))
                            .saturating_add(encoded_size(review.preview()))
                            .saturating_add(128),
                    )
                }
                TableDdlSubmission::Finished(receipt) => {
                    receipt.retained_bytes().max(receipt.encoded_bytes())
                }
            }
        }
        TableMessage::TableDdlApplied(_, DdlApplied::Schema(Ok(submission))) => {
            match submission.as_ref() {
                SchemaAlterSubmission::NeedsConfirmation(confirmation) => {
                    schema_confirmation_bytes(confirmation)
                }
                SchemaAlterSubmission::Finished(receipt) => {
                    receipt.retained_bytes().max(receipt.encoded_bytes())
                }
            }
        }
        TableMessage::TableDdlObserved(_, DdlObserved::Table(Err(_)))
        | TableMessage::TableDdlReviewed(_, DdlReviewed::Table(Err(_)))
        | TableMessage::TableDdlApplied(_, DdlApplied::Table(Err(_))) => {
            std::mem::size_of::<TableDdlError>()
        }
        TableMessage::TableDdlObserved(_, DdlObserved::Schema(Err(_)))
        | TableMessage::TableDdlReviewed(_, DdlReviewed::Schema(Err(_)))
        | TableMessage::TableDdlApplied(_, DdlApplied::Schema(Err(_))) => {
            std::mem::size_of::<SchemaAlterError>()
        }
        _ => unreachable!("Only typed table or schema DDL messages enter this adapter"),
    }
}

#[cfg(test)]
#[path = "table_runtime_ddl_tests.rs"]
mod tests;
