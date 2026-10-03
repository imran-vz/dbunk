//! Profile-owned, transient PostgreSQL file jobs. Register before preparation;
//! views observe attempts and never own a subprocess or restore source snapshot.
mod prepare;
mod registry;
mod types;
use super::Backend;
pub(super) use registry::Registry;
use std::sync::{Arc, Weak};
pub use types::*;

/// Caller holds `development_gate` through the following connection/credential
/// mutation and its manager generation fence. Refuse instead of waiting for a
/// restore monitor that needs the same gate to complete data retirement.
pub(in crate::backend) fn require_connection_settled(
    inner: &super::Inner,
    connection: Option<&str>,
) -> Result<(), String> {
    inner.tool_jobs.require_connection_settled(connection)
}

pub struct PgToolReview {
    owner: Weak<super::Inner>,
    attempt: PgToolAttemptId,
    revision: uuid::Uuid,
    observation: PgToolObservation,
    target: PgToolTarget,
    requires_confirmation: bool,
}
impl PgToolReview {
    pub fn attempt_id(&self) -> PgToolAttemptId {
        self.attempt
    }
    pub fn observation(&self) -> &PgToolObservation {
        &self.observation
    }
    pub fn target(&self) -> &PgToolTarget {
        &self.target
    }
    pub fn requires_confirmation(&self) -> bool {
        self.requires_confirmation
    }
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.observation.checked_heap_bytes().unwrap_or(usize::MAX))
            .saturating_add(self.target.checked_heap_bytes().unwrap_or(usize::MAX))
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(&backend.0))
    }
}
pub struct PgToolConfirmation {
    review: PgToolReview,
}
impl PgToolConfirmation {
    pub fn review(&self) -> &PgToolReview {
        &self.review
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
// The ordinary accepted path keeps its bounded 320-byte observation inline.
#[allow(clippy::large_enum_variant)]
pub enum PgToolSubmission {
    NeedsConfirmation(Box<PgToolConfirmation>),
    Accepted(PgToolObservation),
}
impl Backend {
    /// Synchronously registers the attempt before owned preparation begins. A
    /// successful return never means backup publication or restore completion.
    /// A missing attempt before this call is dispatched is not a refusal proof.
    pub fn begin_pg_tool_job(
        &self,
        attempt_id: PgToolAttemptId,
        connection_id: String,
        intent: PgToolIntent,
    ) -> Result<PgToolObservation, PgToolError> {
        self.0
            .tool_jobs
            .begin(self, attempt_id, connection_id, intent)
    }
    pub fn get_pg_tool_job(
        &self,
        attempt: PgToolAttemptId,
    ) -> Result<PgToolObservation, PgToolError> {
        self.0.tool_jobs.get(attempt)
    }
    pub fn list_pg_tool_jobs(
        &self,
        connection_id: Option<&str>,
    ) -> Result<PgToolJobList, PgToolError> {
        self.0.tool_jobs.list(connection_id)
    }
    pub fn review_pg_tool_job(
        &self,
        attempt: PgToolAttemptId,
    ) -> Result<PgToolReview, PgToolError> {
        self.0.tool_jobs.review(self, attempt)
    }
    pub fn start_pg_tool_job(&self, review: PgToolReview) -> Result<PgToolSubmission, PgToolError> {
        if !review.belongs_to(self) {
            return Err(PgToolError::ForeignReview);
        }
        if review.requires_confirmation {
            self.0.tool_jobs.await_confirmation(&review)?;
            return Ok(PgToolSubmission::NeedsConfirmation(Box::new(
                PgToolConfirmation { review },
            )));
        }
        self.0
            .tool_jobs
            .dispatch(self, review, false)
            .map(PgToolSubmission::Accepted)
    }
    pub fn confirm_pg_tool_job(
        &self,
        confirmation: PgToolConfirmation,
    ) -> Result<PgToolSubmission, PgToolError> {
        if !confirmation.review.belongs_to(self) {
            return Err(PgToolError::ForeignReview);
        }
        self.0
            .tool_jobs
            .dispatch(self, confirmation.review, true)
            .map(PgToolSubmission::Accepted)
    }
    pub fn cancel_pg_tool_job(
        &self,
        attempt: PgToolAttemptId,
    ) -> Result<PgToolObservation, PgToolError> {
        self.0.tool_jobs.cancel(&self.0.state.pg_tool_jobs, attempt)
    }
    pub fn release_pg_tool_job(&self, attempt: PgToolAttemptId) -> Result<(), PgToolError> {
        self.0
            .tool_jobs
            .release(&self.0.state.pg_tool_jobs, attempt)
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
