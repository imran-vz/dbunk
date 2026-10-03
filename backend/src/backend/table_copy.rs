//! App-owned table copy attempts. Recovery descriptions cannot recreate authority.
//!
//! Source rows come from a repeatable-read snapshot established at execution,
//! before any destination write. Generated destination columns are recomputed;
//! explicit identity/serial values are copied without advancing their sequences.
//! Rollback describes the destination transaction, not nontransactional sequence
//! or trigger side effects. Relation locks/catalog checks cannot prevent another
//! session renaming/replacing a schema name between name resolution steps.
mod registry;
mod service;
mod types;
use super::Backend;
use crate::postgres::native_table_copy::Plan;
pub(super) use registry::Registry;
use std::sync::{Arc, Mutex, Weak};
pub use types::*;

struct Ready {
    plan: Arc<Plan>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    requires_confirmation: bool,
}
#[derive(Clone)]
pub struct TableCopyReview {
    owner: Weak<super::Inner>,
    id: TableCopyAttemptId,
    ready: Arc<Ready>,
}
impl TableCopyReview {
    pub fn attempt_id(&self) -> TableCopyAttemptId {
        self.id
    }
    pub fn description(&self) -> &TableCopyDescription {
        &self.ready.plan.description
    }
    pub fn columns(&self) -> &[TableCopyColumn] {
        &self.ready.plan.columns
    }
    pub fn retained_bytes(&self) -> usize {
        self.ready.plan.retained_bytes()
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(&backend.0))
    }
}
pub struct TableCopyConfirmation {
    review: TableCopyReview,
}
impl TableCopyConfirmation {
    pub fn review(&self) -> &TableCopyReview {
        &self.review
    }
    pub fn attempt_id(&self) -> TableCopyAttemptId {
        self.review.id
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.review.belongs_to(backend)
    }
}
#[allow(clippy::large_enum_variant)]
pub enum TableCopySubmission {
    NeedsConfirmation(Box<TableCopyConfirmation>),
    Accepted(TableCopyObservation),
}
impl Backend {
    /// Synchronous registration precedes policy checks, hydration and catalog I/O.
    /// Registration never starts destination COPY; an exact review must be started.
    pub fn begin_table_copy(
        &self,
        id: TableCopyAttemptId,
        intent: TableCopyIntent,
    ) -> Result<TableCopyObservation, TableCopyError> {
        self.0.table_copy.begin(self, id, intent)
    }
    pub fn review_table_copy(
        &self,
        id: TableCopyAttemptId,
    ) -> Result<TableCopyReview, TableCopyError> {
        self.0.table_copy.review(self, id)
    }
    /// Persist the exact attempt and description before calling. Accepted is an
    /// admission, not a success receipt. Never replay after an uncertain delivery.
    pub fn start_table_copy(
        &self,
        review: TableCopyReview,
    ) -> Result<TableCopySubmission, TableCopyError> {
        self.0.table_copy.submit(self, review, false)
    }
    pub fn confirm_table_copy(
        &self,
        confirmation: TableCopyConfirmation,
    ) -> Result<TableCopySubmission, TableCopyError> {
        self.0.table_copy.submit(self, confirmation.review, true)
    }
    pub fn get_table_copy(
        &self,
        id: TableCopyAttemptId,
    ) -> Result<TableCopyObservation, TableCopyError> {
        self.0.table_copy.get(id)
    }
    pub fn list_table_copies(&self) -> Result<TableCopyList, TableCopyError> {
        self.0.table_copy.list()
    }
    pub fn cancel_table_copy(
        &self,
        id: TableCopyAttemptId,
    ) -> Result<TableCopyObservation, TableCopyError> {
        self.0.table_copy.cancel(id)
    }
    pub fn release_table_copy(&self, id: TableCopyAttemptId) -> Result<(), TableCopyError> {
        self.0.table_copy.release(id)
    }
}
/// One short mutex linearizes COMMIT admission against cancellation/retirement.
/// Notification and state update happen under the same lock; no filesystem or
/// socket operation holds it. A known COMMIT success wins over subsequent cancel.
pub(crate) struct Control {
    state: Mutex<ControlState>,
    cancel: tokio::sync::watch::Sender<bool>,
    bytes: std::sync::atomic::AtomicU64,
}
#[derive(Default)]
struct ControlState {
    cancelled: bool,
    commit: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            state: Default::default(),
            cancel: tokio::sync::watch::channel(false).0,
            bytes: Default::default(),
        }
    }
}
impl Control {
    pub(crate) fn cancel(&self) {
        let mut s = self.state.lock().unwrap();
        s.cancelled = true;
        self.cancel.send_replace(true);
    }
    pub(crate) async fn cancelled(&self) {
        let mut r = self.cancel.subscribe();
        let _ = r.wait_for(|v| *v).await;
    }
    pub(crate) fn admit_commit(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.cancelled || s.commit {
            return false;
        }
        s.commit = true;
        true
    }
    fn is_cancelled(&self) -> bool {
        self.state.lock().unwrap().cancelled
    }
    pub(crate) fn add_bytes(&self, n: usize) -> Result<(), TableCopyError> {
        self.bytes
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |old| old.checked_add(n as u64),
            )
            .map(|_| ())
            .map_err(|_| TableCopyError::Limit)
    }
}
/// Called under the shared development gate. Reservations include review and
/// preparation, preventing reciprocal restore/import admission races.
pub(super) fn destination_write_in_progress(inner: &super::Inner, connection: &str) -> bool {
    inner.table_copy.involves(connection)
}
pub(super) use registry::retire_connection;
#[cfg(test)]
mod tests;
