//! Seed facade calls only register or inspect bounded registry state. Database
//! work and its shutdown belong to Backend; no native worker or payload queue is
//! introduced. Only task-admitting calls enter the shared Tokio runtime.
use crate::controller::Host;
use dbunk_lib::backend::table_seed::*;

pub fn register(
    host: &Host,
    id: TableSeedAttemptId,
    intent: TableSeedIntent,
) -> Result<TableSeedObservation, TableSeedError> {
    intent.checked_heap_bytes().ok_or(TableSeedError::Limit)?;
    let _runtime = host.runtime.enter();
    host.backend.begin_table_seed(id, intent)
}

/// Exact authority is matched again here after the durable save fence. A stale
/// confirmation can never be consumed to dispatch the newly selected attempt.
pub fn dispatch(
    host: &Host,
    id: TableSeedAttemptId,
    description: &TableSeedDescription,
    review: Option<TableSeedReview>,
    confirmation: Option<TableSeedConfirmation>,
) -> Result<TableSeedSubmission, TableSeedError> {
    let _runtime = host.runtime.enter();
    if let Some(confirmation) = confirmation {
        if confirmation.attempt_id() != id || confirmation.review().description() != description {
            return Err(TableSeedError::StaleReview);
        }
        host.backend.confirm_table_seed(confirmation)
    } else {
        let review = review
            .filter(|review| review.attempt_id() == id && review.description() == description)
            .ok_or(TableSeedError::StaleReview)?;
        host.backend.start_table_seed(review)
    }
}
