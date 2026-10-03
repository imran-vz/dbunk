//! Object DDL adapter for the existing document worker. No persistence authority.
use super::*;
use dbunk_lib::backend::object_ddl::{
    ObjectDdlError, ObjectDdlOperation, ObjectDdlSubmission, ObjectDdlTarget,
};

/// Covers a review carrying the bounded (256 KiB encoded) drop-impact evidence.
pub(super) const RESPONSE_BYTES: usize = 1024 * 1024;

/// The observation must answer exactly the requested operations; a target for
/// different operations can never be reviewed under this request's identity.
fn observed_matches(expected: &[ObjectDdlOperation], target: &ObjectDdlTarget) -> bool {
    target.operations() == expected
        && target.description().checked_heap_bytes().is_some()
        && target.impacts().len() == expected.len()
}

pub(super) async fn request(
    backend: &Backend,
    document: &DataDocument,
    command: TableCommand,
) -> TableMessage {
    match command {
        TableCommand::ObjectDdlObserve(id, request) => {
            let result = async {
                request.validate()?;
                // Request is already bounded to eight operations before cloning.
                let expected = request.operations.clone();
                let target = backend.observe_object_ddl(document, request).await?;
                if !target.belongs_to(document)
                    || !observed_matches(&expected, &target)
                    || target.retained_bytes().max(target.encoded_bytes()) > RESPONSE_BYTES / 2
                {
                    return Err(ObjectDdlError::Limit);
                }
                Ok(Box::new(target))
            }
            .await;
            TableMessage::TableDdlObserved(id, DdlObserved::Object(result.map_err(Arc::new)))
        }
        TableCommand::ObjectDdlReview(id, target) => {
            let result = if !target.belongs_to(document) {
                Err(ObjectDdlError::ForeignDocument)
            } else if target.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(ObjectDdlError::Limit)
            } else {
                backend.review_object_ddl(*target).await.map(Box::new)
            };
            TableMessage::TableDdlReviewed(id, DdlReviewed::Object(result.map_err(Arc::new)))
        }
        TableCommand::ObjectDdlApply(id, review) => {
            let result = if !review.belongs_to(document) {
                Err(ObjectDdlError::ForeignDocument)
            } else if review.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(ObjectDdlError::Limit)
            } else {
                backend.apply_object_ddl(*review).await.map(Box::new)
            };
            TableMessage::TableDdlApplied(id, DdlApplied::Object(result.map_err(Arc::new)))
        }
        TableCommand::ObjectDdlConfirm(id, confirmation) => {
            let result = if !confirmation.belongs_to(document) {
                Err(ObjectDdlError::ForeignDocument)
            } else if confirmation.retained_bytes() > RESPONSE_BYTES / 2 {
                Err(ObjectDdlError::Limit)
            } else {
                backend
                    .confirm_object_ddl(*confirmation)
                    .await
                    .map(Box::new)
            };
            TableMessage::TableDdlApplied(id, DdlApplied::Object(result.map_err(Arc::new)))
        }
        _ => unreachable!("Only typed object DDL commands enter this adapter"),
    }
}

pub(super) fn message_bytes(message: &TableMessage) -> usize {
    match message {
        TableMessage::TableDdlObserved(_, DdlObserved::Object(Ok(target))) => {
            target.retained_bytes().max(target.encoded_bytes())
        }
        TableMessage::TableDdlReviewed(_, DdlReviewed::Object(Ok(review))) => {
            review.retained_bytes().max(review.encoded_bytes())
        }
        TableMessage::TableDdlApplied(_, DdlApplied::Object(Ok(submission))) => {
            match submission.as_ref() {
                ObjectDdlSubmission::NeedsConfirmation(confirmation) => confirmation
                    .retained_bytes()
                    .max(confirmation.review().encoded_bytes()),
                ObjectDdlSubmission::Finished(receipt) => {
                    receipt.retained_bytes().max(receipt.encoded_bytes())
                }
            }
        }
        TableMessage::TableDdlObserved(_, DdlObserved::Object(Err(_)))
        | TableMessage::TableDdlReviewed(_, DdlReviewed::Object(Err(_)))
        | TableMessage::TableDdlApplied(_, DdlApplied::Object(Err(_))) => {
            std::mem::size_of::<ObjectDdlError>()
        }
        _ => unreachable!("Only typed object DDL messages enter this adapter"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::object_ddl::ObjectDdlRequest;

    // A queued object command that never reached the backend settles in its own
    // reply family as a pre-dispatch refusal, never as a table reply or unknown.
    #[test]
    fn queued_object_command_cancels_as_its_own_pre_dispatch_refusal() {
        let message = super::super::cancelled(TableCommand::ObjectDdlObserve(
            5,
            ObjectDdlRequest {
                operations: Vec::new(),
            },
        ));
        assert!(matches!(
            &message,
            TableMessage::TableDdlObserved(5, DdlObserved::Object(Err(error)))
                if **error == ObjectDdlError::Unavailable
        ));
        assert_eq!(
            message_bytes(&message),
            std::mem::size_of::<ObjectDdlError>()
        );
        assert!(super::super::message_bytes(&message) < RESPONSE_BYTES);
    }
}
