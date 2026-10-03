//! Table DDL adapter for the existing document worker. No persistence authority.
use super::*;
use dbunk_lib::backend::table_ddl::{TableDdlDescription, TableDdlError, TableDdlRequest};

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
            TableMessage::TableDdlObserved(id, result.map_err(Arc::new))
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
            TableMessage::TableDdlReviewed(id, result.map_err(Arc::new))
        }
        TableCommand::TableDdlApply(id, review) => {
            let result = if !review.belongs_to(document) {
                Err(TableDdlError::ForeignDocument)
            } else if review.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(TableDdlError::InvalidTarget)
            } else {
                backend.apply_table_ddl(*review).await.map(Box::new)
            };
            TableMessage::TableDdlApplied(id, result.map_err(Arc::new))
        }
        TableCommand::TableDdlConfirm(id, confirmation) => {
            let result = if !confirmation.belongs_to(document) {
                Err(TableDdlError::ForeignDocument)
            } else if confirmation.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(TableDdlError::InvalidTarget)
            } else {
                backend.confirm_table_ddl(*confirmation).await.map(Box::new)
            };
            TableMessage::TableDdlApplied(id, result.map_err(Arc::new))
        }
        _ => unreachable!("Only typed table DDL commands enter this adapter"),
    }
}

pub(super) fn message_bytes(message: &TableMessage) -> usize {
    match message {
        TableMessage::TableDdlObserved(_, Ok(target)) => {
            target.retained_bytes().max(target.encoded_bytes())
        }
        TableMessage::TableDdlReviewed(_, Ok(review)) => review.retained_bytes().max(
            review
                .target()
                .encoded_bytes()
                .saturating_add(encoded_size(review.intent()))
                .saturating_add(encoded_size(review.preview()))
                .saturating_add(128),
        ),
        TableMessage::TableDdlApplied(_, Ok(submission)) => match submission.as_ref() {
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
        },
        TableMessage::TableDdlObserved(_, Err(_))
        | TableMessage::TableDdlReviewed(_, Err(_))
        | TableMessage::TableDdlApplied(_, Err(_)) => std::mem::size_of::<TableDdlError>(),
        _ => unreachable!("Only typed table DDL messages enter this adapter"),
    }
}

#[cfg(test)]
#[path = "table_runtime_ddl_tests.rs"]
mod tests;
