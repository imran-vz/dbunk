//! Owned seed attempts. Preparing only inspects metadata; an immutable review and
//! explicit start are required. Recovery descriptions never grant replay authority.
mod bounds;
mod registry;
mod service;
mod types;
use super::Backend;
use crate::postgres::native_table_seed::Plan;
pub(super) use registry::retire_connection;
pub(super) use registry::Registry;
use std::sync::{Arc, Mutex, Weak};
pub use types::*;
struct Ready {
    plan: Arc<Plan>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    requires_confirmation: bool,
}
#[derive(Clone)]
pub struct TableSeedInspection {
    owner: Weak<super::Inner>,
    id: TableSeedAttemptId,
    plan: Arc<Plan>,
}
impl TableSeedInspection {
    pub fn attempt_id(&self) -> TableSeedAttemptId {
        self.id
    }
    pub fn columns(&self) -> &[TableSeedColumn] {
        &self.plan.columns
    }
    pub fn specs(&self) -> &[TableSeedColumnSpec] {
        &self.plan.intent.columns
    }
    pub fn intent(&self) -> &TableSeedIntent {
        &self.plan.intent
    }
    pub fn issue(&self) -> Option<TableSeedError> {
        self.plan.issue
    }
    pub fn retained_bytes(&self) -> usize {
        MAX_TABLE_SEED_REVIEW_BYTES
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(&backend.0))
    }
}
#[derive(Clone)]
pub struct TableSeedReview {
    owner: Weak<super::Inner>,
    id: TableSeedAttemptId,
    ready: Arc<Ready>,
}
impl TableSeedReview {
    pub fn attempt_id(&self) -> TableSeedAttemptId {
        self.id
    }
    pub fn description(&self) -> &TableSeedDescription {
        self.ready
            .plan
            .description
            .as_ref()
            .expect("validated runnable seed review")
    }
    pub fn columns(&self) -> &[TableSeedColumn] {
        &self.ready.plan.columns
    }
    pub fn specs(&self) -> &[TableSeedColumnSpec] {
        &self.ready.plan.intent.columns
    }
    pub fn retained_bytes(&self) -> usize {
        MAX_TABLE_SEED_REVIEW_BYTES
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(&backend.0))
    }
}
pub struct TableSeedConfirmation {
    review: TableSeedReview,
}
impl TableSeedConfirmation {
    pub fn review(&self) -> &TableSeedReview {
        &self.review
    }
    pub fn attempt_id(&self) -> TableSeedAttemptId {
        self.review.id
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.review.belongs_to(backend)
    }
}
pub enum TableSeedSubmission {
    NeedsConfirmation(Box<TableSeedConfirmation>),
    Accepted(Box<TableSeedObservation>),
}
impl Backend {
    pub fn begin_table_seed(
        &self,
        id: TableSeedAttemptId,
        intent: TableSeedIntent,
    ) -> Result<TableSeedObservation, TableSeedError> {
        self.0.table_seed.begin(self, id, intent)
    }
    pub fn inspect_table_seed(
        &self,
        id: TableSeedAttemptId,
    ) -> Result<TableSeedInspection, TableSeedError> {
        let state = self.0.table_seed.state.lock().unwrap();
        let e = state.jobs.get(&id).ok_or(TableSeedError::Missing)?;
        if !matches!(
            e.observation.phase,
            TableSeedPhase::NeedsRecipe
                | TableSeedPhase::ReadyReview
                | TableSeedPhase::AwaitingConfirmation
        ) || e.created.elapsed() > std::time::Duration::from_secs(300)
            || e.control.is_cancelled()
        {
            return Err(TableSeedError::StaleReview);
        }
        Ok(TableSeedInspection {
            owner: Arc::downgrade(&self.0),
            id,
            plan: e
                .ready
                .as_ref()
                .ok_or(TableSeedError::StaleReview)?
                .plan
                .clone(),
        })
    }
    pub fn review_table_seed(
        &self,
        id: TableSeedAttemptId,
    ) -> Result<TableSeedReview, TableSeedError> {
        self.0.table_seed.review(self, id)
    }
    /// Caller must persist this exact attempt and description before dispatch.
    pub fn start_table_seed(
        &self,
        review: TableSeedReview,
    ) -> Result<TableSeedSubmission, TableSeedError> {
        self.0.table_seed.submit(self, review, false)
    }
    pub fn confirm_table_seed(
        &self,
        confirmation: TableSeedConfirmation,
    ) -> Result<TableSeedSubmission, TableSeedError> {
        self.0.table_seed.submit(self, confirmation.review, true)
    }
    pub fn get_table_seed(
        &self,
        id: TableSeedAttemptId,
    ) -> Result<TableSeedObservation, TableSeedError> {
        self.0.table_seed.get(id)
    }
    pub fn list_table_seeds(&self) -> Result<TableSeedList, TableSeedError> {
        self.0.table_seed.list()
    }
    pub fn cancel_table_seed(
        &self,
        id: TableSeedAttemptId,
    ) -> Result<TableSeedObservation, TableSeedError> {
        self.0.table_seed.cancel(id)
    }
    pub fn release_table_seed(&self, id: TableSeedAttemptId) -> Result<(), TableSeedError> {
        self.0.table_seed.release(id)
    }
}
pub(crate) struct Control {
    state: Mutex<ControlState>,
    cancel: tokio::sync::watch::Sender<bool>,
    rows: std::sync::atomic::AtomicU64,
}
#[derive(Default)]
struct ControlState {
    cancelled: bool,
    commit: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            cancel: tokio::sync::watch::channel(false).0,
            rows: Default::default(),
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
    pub(crate) fn is_cancelled(&self) -> bool {
        self.state.lock().unwrap().cancelled
    }
    pub(crate) fn admit_commit(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.cancelled || s.commit {
            return false;
        }
        s.commit = true;
        true
    }
    pub(crate) fn progress(&self, rows: u64) {
        self.rows.store(rows, std::sync::atomic::Ordering::Release);
    }
}
pub(super) fn destination_write_in_progress(inner: &super::Inner, connection: &str) -> bool {
    inner.table_seed.involves(connection)
}

#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;
