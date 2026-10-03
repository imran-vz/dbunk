//! Sequence adapter for the Objects document worker. Tokens are single-use and
//! document-bound; nothing here persists or retries a write.
use crate::controller::TableMessage;
use crate::results::encoded_size;
use dbunk_lib::backend::Backend;
use dbunk_lib::backend::data::DataDocument;
use dbunk_lib::backend::objects::PgObjectRef;
use dbunk_lib::backend::sequences::{
    MAX_SEQUENCE_REVIEW_BYTES, ObservedSequence, SequenceConfirmation, SequenceError,
    SequenceReview, SequenceSubmission,
};
use std::sync::Arc;

/// Both a confirmation token and a receipt fit, including envelope overhead.
pub const RESPONSE_BYTES: usize = 64 * 1024;

pub enum SequenceRequest {
    Inspect(PgObjectRef),
    Apply(Box<SequenceReview>),
    Confirm(Box<SequenceConfirmation>),
}
pub enum SequenceReply {
    Inspected(Result<Box<ObservedSequence>, Arc<SequenceError>>),
    Applied(Result<SequenceSubmission, Arc<SequenceError>>),
}
impl SequenceRequest {
    pub fn writes(&self) -> bool {
        !matches!(self, Self::Inspect(_))
    }
    /// Used only before the backend future is polled: nothing was dispatched.
    pub fn cancelled(self) -> SequenceReply {
        match self {
            Self::Inspect(_) => SequenceReply::Inspected(Err(Arc::new(SequenceError::Unavailable))),
            Self::Apply(_) | Self::Confirm(_) => {
                SequenceReply::Applied(Err(Arc::new(SequenceError::Unavailable)))
            }
        }
    }
}

pub async fn request(
    backend: &Backend,
    document: &DataDocument,
    request: SequenceRequest,
) -> SequenceReply {
    match request {
        SequenceRequest::Inspect(reference) => SequenceReply::Inspected(
            backend
                .observe_sequence(document, reference)
                .await
                .and_then(|observed| {
                    if observed.belongs_to(document)
                        && observed.retained_bytes() <= MAX_SEQUENCE_REVIEW_BYTES
                    {
                        Ok(Box::new(observed))
                    } else {
                        Err(SequenceError::InvalidTarget)
                    }
                })
                .map_err(Arc::new),
        ),
        SequenceRequest::Apply(review) => SequenceReply::Applied(
            if !review.belongs_to(document) {
                Err(SequenceError::ForeignDocument)
            } else if review.retained_bytes() > MAX_SEQUENCE_REVIEW_BYTES {
                Err(SequenceError::InvalidTarget)
            } else {
                backend.apply_sequence(*review).await
            }
            .map_err(Arc::new),
        ),
        SequenceRequest::Confirm(confirmation) => SequenceReply::Applied(
            if !confirmation.belongs_to(document) {
                Err(SequenceError::ForeignDocument)
            } else if confirmation.retained_bytes() > MAX_SEQUENCE_REVIEW_BYTES {
                Err(SequenceError::InvalidTarget)
            } else {
                backend.confirm_sequence(*confirmation).await
            }
            .map_err(Arc::new),
        ),
    }
}

pub fn message_bytes(message: &TableMessage) -> usize {
    let TableMessage::Sequence(_, reply) = message else {
        unreachable!("Only sequence messages enter this adapter")
    };
    match reply {
        SequenceReply::Inspected(Ok(observed)) => observed
            .retained_bytes()
            .max(encoded_size(observed.observation())),
        SequenceReply::Applied(Ok(SequenceSubmission::NeedsConfirmation(confirmation))) => {
            let review = confirmation.review();
            confirmation.retained_bytes().max(
                encoded_size(review.observation())
                    .saturating_add(encoded_size(review.preview()))
                    .saturating_add(256),
            )
        }
        SequenceReply::Applied(Ok(SequenceSubmission::Finished(receipt))) => {
            receipt.retained_bytes().max(encoded_size(receipt.as_ref()))
        }
        SequenceReply::Inspected(Err(_)) | SequenceReply::Applied(Err(_)) => {
            std::mem::size_of::<SequenceError>()
        }
    }
}
