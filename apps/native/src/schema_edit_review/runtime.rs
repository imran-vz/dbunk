//! Backend calls of the schema editor. MySQL and SQLite call the facade on
//! the host runtime with their session; PostgreSQL routes through the Objects
//! document worker, which owns the `DataDocument` (like `sequence_runtime`).
//! Review is synchronous and runs on the UI thread. Tokens are single-use;
//! nothing here persists or retries a write.
use crate::controller::TableMessage;
use crate::results::encoded_size;
use dbunk_lib::backend::Backend;
use dbunk_lib::backend::data::DataDocument;
use dbunk_lib::backend::schema_edit::{
    SchemaEditConfirmation, SchemaEditError, SchemaEditRequest, SchemaEditReview, SchemaEditSource,
    SchemaEditSubmission, SchemaEditTarget,
};

/// Reserved for a write reply: a confirmation carries the baseline, the
/// draft and the preview (at most 1 MiB), a receipt is small.
pub const RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// A baseline or review larger than this is refused before it is queued.
const PAYLOAD_BYTES: usize = RESPONSE_BYTES / 2;

pub enum SchemaEditCall {
    Observe(SchemaEditRequest),
    Apply(Box<SchemaEditReview>),
    Confirm(Box<SchemaEditConfirmation>),
}

pub enum SchemaEditReply {
    Observed(Result<Box<SchemaEditTarget>, SchemaEditError>),
    Applied(Result<SchemaEditSubmission, SchemaEditError>),
}

impl SchemaEditCall {
    pub fn writes(&self) -> bool {
        !matches!(self, Self::Observe(_))
    }
    /// Used only before the backend future is polled: nothing was sent.
    pub fn cancelled(self) -> SchemaEditReply {
        match self {
            Self::Observe(_) => SchemaEditReply::Observed(Err(SchemaEditError::Unavailable(
                "Cancelled before the structure was read".into(),
            ))),
            Self::Apply(_) | Self::Confirm(_) => SchemaEditReply::Applied(Err(
                SchemaEditError::Unavailable("Cancelled before dispatch; nothing was sent".into()),
            )),
        }
    }
    /// The reply when the task carrying the call ended without one. A write
    /// may have reached the server, so its outcome is unknown.
    pub fn lost(writes: bool) -> SchemaEditReply {
        if writes {
            SchemaEditReply::Applied(Err(SchemaEditError::OutcomeUnavailable))
        } else {
            SchemaEditReply::Observed(Err(SchemaEditError::Unavailable(
                "The structure read ended unexpectedly".into(),
            )))
        }
    }
}

fn review_bytes(review: &SchemaEditReview) -> usize {
    encoded_size(review.baseline())
        .saturating_add(encoded_size(review.draft()))
        .saturating_add(encoded_size(review.preview()))
}

/// One facade call against `source`.
pub async fn call(
    backend: &Backend,
    source: SchemaEditSource,
    call: SchemaEditCall,
) -> SchemaEditReply {
    match call {
        SchemaEditCall::Observe(request) => {
            SchemaEditReply::Observed(backend.observe_schema_edit(source, request).await.and_then(
                |target| {
                    if encoded_size(target.baseline()) > PAYLOAD_BYTES {
                        Err(SchemaEditError::Unavailable(
                            "The table structure is too large to edit here".into(),
                        ))
                    } else {
                        Ok(Box::new(target))
                    }
                },
            ))
        }
        SchemaEditCall::Apply(review) => {
            SchemaEditReply::Applied(if !review.target().source().same(&source) {
                Err(SchemaEditError::ForeignSource)
            } else if review_bytes(&review) > PAYLOAD_BYTES {
                Err(SchemaEditError::Unavailable(
                    "The reviewed change is too large to send; nothing was sent".into(),
                ))
            } else {
                backend.apply_schema_edit(*review).await
            })
        }
        SchemaEditCall::Confirm(confirmation) => {
            SchemaEditReply::Applied(if !confirmation.review().target().source().same(&source) {
                Err(SchemaEditError::ForeignSource)
            } else if review_bytes(confirmation.review()) > PAYLOAD_BYTES {
                Err(SchemaEditError::Unavailable(
                    "The reviewed change is too large to send; nothing was sent".into(),
                ))
            } else {
                backend.confirm_schema_edit(*confirmation).await
            })
        }
    }
}

/// The Objects document worker's entry point (PostgreSQL).
pub async fn request(
    backend: &Backend,
    document: &DataDocument,
    request: SchemaEditCall,
) -> SchemaEditReply {
    call(
        backend,
        SchemaEditSource::Postgres(document.clone()),
        request,
    )
    .await
}

pub fn message_bytes(message: &TableMessage) -> usize {
    let TableMessage::SchemaEdit(_, reply) = message else {
        unreachable!("Only schema edit messages enter this adapter")
    };
    match reply {
        SchemaEditReply::Observed(Ok(target)) => encoded_size(target.baseline()),
        SchemaEditReply::Applied(Ok(SchemaEditSubmission::NeedsConfirmation(confirmation))) => {
            review_bytes(confirmation.review()).saturating_add(256)
        }
        SchemaEditReply::Applied(Ok(SchemaEditSubmission::Finished(receipt))) => {
            encoded_size(receipt.as_ref())
        }
        SchemaEditReply::Observed(Err(error)) | SchemaEditReply::Applied(Err(error)) => {
            error.to_string().len()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_call_reports_nothing_sent_and_a_lost_write_is_unknown() {
        let observe = SchemaEditCall::Observe(SchemaEditRequest::new("public", "users"));
        assert!(!observe.writes());
        assert!(matches!(
            observe.cancelled(),
            SchemaEditReply::Observed(Err(SchemaEditError::Unavailable(_)))
        ));
        // A write whose task ended without a reply may have reached the
        // server: never report it as not sent.
        assert!(matches!(
            SchemaEditCall::lost(true),
            SchemaEditReply::Applied(Err(SchemaEditError::OutcomeUnavailable))
        ));
        assert!(matches!(
            SchemaEditCall::lost(false),
            SchemaEditReply::Observed(Err(SchemaEditError::Unavailable(_)))
        ));
    }
}
