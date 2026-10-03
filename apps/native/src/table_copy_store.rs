//! App-owned copy reviews and durable intent. The facade performs only bounded
//! registry work synchronously; all database work belongs to its joined owner.
use crate::controller::Host;
use dbunk_lib::backend::{WorkspaceTableCopy, WorkspaceTableCopyState as State, table_copy::*};
use gpui::{Context, EventEmitter, Task};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

const STORE_BYTES: usize = 7 * 1024 * 1024;
const MAX_JOURNAL: usize = 16;
struct Lease(Rc<Cell<usize>>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(STORE_BYTES));
    }
}
pub enum CopyEvent {
    Changed,
    Persist(u64),
    DestinationChanged(Option<String>),
}
/// An acknowledgement is executable only while its exact request remains live.
#[derive(Default)]
struct SaveFence {
    serial: u64,
    pending: Option<(u64, TableCopyAttemptId)>,
}
impl SaveFence {
    fn begin(&mut self, id: TableCopyAttemptId) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.serial = self.serial.checked_add(1)?;
        self.pending = Some((self.serial, id));
        Some(self.serial)
    }
    fn cancel(&mut self, id: TableCopyAttemptId) {
        if self.pending.is_some_and(|(_, pending)| pending == id) {
            self.pending = None;
        }
    }
    fn accept(&mut self, serial: u64) -> Option<TableCopyAttemptId> {
        if self.pending.is_some_and(|(pending, _)| pending == serial) {
            self.pending.take().map(|(_, id)| id)
        } else {
            None
        }
    }
}
pub struct CopyStore {
    host: Arc<Host>,
    journal: Vec<WorkspaceTableCopy>,
    jobs: Vec<TableCopyObservation>,
    review: Option<TableCopyReview>,
    confirmation: Option<TableCopyConfirmation>,
    fence: SaveFence,
    current: bool,
    restored: bool,
    revision: u64,
    message: Option<String>,
    _lease: Option<Lease>,
    _poll: Task<()>,
}
impl EventEmitter<CopyEvent> for CopyStore {}
impl CopyStore {
    pub fn new(host: Arc<Host>, budget: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
        let admitted = STORE_BYTES <= (128 * 1024 * 1024usize).saturating_sub(budget.get());
        let lease = admitted.then(|| {
            budget.set(budget.get() + STORE_BYTES);
            Lease(budget)
        });
        let poll = cx.spawn(async move |this, cx| {
            loop {
                let Ok(active) = this.update(cx, |this, cx| {
                    this.refresh(cx);
                    this.jobs.iter().any(|job| !job.phase.terminal())
                }) else {
                    break;
                };
                cx.background_executor()
                    .timer(Duration::from_secs(if active { 1 } else { 15 }))
                    .await;
            }
        });
        Self {
            host,
            journal: vec![],
            jobs: vec![],
            review: None,
            confirmation: None,
            fence: SaveFence::default(),
            current: false,
            restored: false,
            revision: 0,
            message: (!admitted)
                .then(|| "Table copy needs 7 MiB of shared retained allowance".into()),
            _lease: lease,
            _poll: poll,
        }
    }
    pub fn restore(&mut self, journal: Vec<WorkspaceTableCopy>, cx: &mut Context<Self>) {
        if self.restored {
            return;
        }
        self.journal = journal;
        self.restored = true;
        self.refresh(cx);
    }
    pub fn snapshot(&self) -> Vec<WorkspaceTableCopy> {
        self.journal.clone()
    }
    pub fn journal(&self) -> &[WorkspaceTableCopy] {
        &self.journal
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.journal.iter().fold(0usize, |n, job| {
            n.saturating_add(job.checked_heap_bytes().unwrap_or(usize::MAX))
        })
    }
    pub fn jobs(&self) -> &[TableCopyObservation] {
        &self.jobs
    }
    pub fn review_payload(&self) -> Option<&TableCopyReview> {
        self.review.as_ref()
    }
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub fn observation_current(&self) -> bool {
        self.current && self.restored && self._lease.is_some()
    }
    pub fn busy(&self) -> bool {
        !self.observation_current() || self.fence.pending.is_some()
    }
    pub fn pending_apply(&self, id: TableCopyAttemptId) -> bool {
        self.fence.pending.is_some_and(|(_, pending)| pending == id)
    }
    pub fn confirmation_pending(&self, id: TableCopyAttemptId) -> bool {
        self.confirmation
            .as_ref()
            .is_some_and(|value| value.attempt_id() == id)
    }
    fn fail(&mut self, error: impl ToString, cx: &mut Context<Self>) {
        self.message = Some(error.to_string());
        cx.notify();
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.restored || self._lease.is_none() {
            return;
        }
        // Before allocating the next bounded list, discard the previous one.
        // Journal and review remain separately covered by the fixed lease.
        self.jobs.clear();
        self.jobs.shrink_to_fit();
        let list = match self.host.backend.list_table_copies() {
            Ok(list) if list.checked_heap_bytes().is_some() => list,
            Ok(_) => {
                self.current = false;
                self.fail("Copy observations exceed their bounds", cx);
                return;
            }
            Err(error) => {
                self.current = false;
                self.fail(error, cx);
                return;
            }
        };
        let covered = list
            .jobs
            .iter()
            .filter(|job| {
                job.change_revision
                    .is_some_and(|revision| revision > self.revision)
            })
            .count() as u64;
        let gap = list.change_revision.saturating_sub(self.revision) != covered;
        if gap {
            cx.emit(CopyEvent::DestinationChanged(None));
        }
        let mut changed = false;
        for record in &mut self.journal {
            if record.state == State::Applying
                && !list
                    .jobs
                    .iter()
                    .any(|job| job.attempt_id == record.attempt_id)
                && !self
                    .fence
                    .pending
                    .is_some_and(|(_, id)| id == record.attempt_id)
            {
                record.state = State::Unknown;
                changed = true;
            }
        }
        for job in &list.jobs {
            if job.phase.terminal() {
                self.fence.cancel(job.attempt_id);
            }
            if let Some(record) = self
                .journal
                .iter_mut()
                .find(|record| record.attempt_id == job.attempt_id)
            {
                let state = match job.outcome {
                    TableCopyOutcome::Completed { rows } => Some(State::Completed { rows }),
                    TableCopyOutcome::RolledBack => Some(State::RolledBack),
                    TableCopyOutcome::OutcomeUnknown => Some(State::Unknown),
                    TableCopyOutcome::NotStarted if job.phase.terminal() => Some(State::NotStarted),
                    _ => None,
                };
                // Explicit reconciliation is durable and cannot be undone by polling.
                if record.state != State::Reconciled
                    && let Some(state) = state
                    && (record.state != state
                        || record.failure != job.failure
                        || record.diagnostic != job.diagnostic)
                {
                    record.state = state;
                    record.failure = job.failure;
                    record.diagnostic = job.diagnostic.clone();
                    changed = true;
                }
            }
            if !gap
                && job
                    .change_revision
                    .is_some_and(|revision| revision > self.revision)
            {
                cx.emit(CopyEvent::DestinationChanged(Some(
                    job.intent.destination.connection_id.clone(),
                )));
            }
        }
        self.revision = list.change_revision;
        self.jobs = list.jobs;
        self.current = true;
        if changed {
            cx.emit(CopyEvent::Changed);
        }
        cx.notify();
    }
    pub fn begin(
        &mut self,
        intent: TableCopyIntent,
        cx: &mut Context<Self>,
    ) -> Option<TableCopyAttemptId> {
        self.refresh(cx);
        if self.journal.iter().any(|record| {
            matches!(record.state, State::Unknown | State::Applying)
                && record.description.intent.destination.connection_id
                    == intent.destination.connection_id
        }) {
            self.fail("Reconcile the previous uncertain copy on this destination before preparing another", cx);
            return None;
        }
        let unstaged = self
            .jobs
            .iter()
            .filter(|job| {
                !job.phase.terminal()
                    && !self.journal.iter().any(|r| r.attempt_id == job.attempt_id)
            })
            .count();
        if self.busy() || self.journal.len() + unstaged >= MAX_JOURNAL {
            self.fail(
                "Copy recovery capacity is full or unavailable; resolve existing attempts first",
                cx,
            );
            return None;
        }
        let id = TableCopyAttemptId::new();
        let result = {
            let _runtime = self.host.runtime.enter();
            self.host.backend.begin_table_copy(id, intent)
        };
        match result {
            Ok(_) => {
                self.message = None;
                self.refresh(cx);
                Some(id)
            }
            Err(error) => {
                self.fail(error, cx);
                None
            }
        }
    }
    pub fn review(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        if self.busy() {
            self.fail("Finish saving the current review first", cx);
            return;
        }
        if self.confirmation.is_some() {
            self.fail("Confirm or cancel the current copy first", cx);
            return;
        }
        self.review = None;
        match self.host.backend.review_table_copy(id) {
            Ok(review) if review.retained_bytes() <= MAX_TABLE_COPY_REVIEW_BYTES => {
                if !self.journal.iter().any(|record| record.attempt_id == id) {
                    if self.journal.len() >= MAX_JOURNAL {
                        self.fail("Copy recovery capacity is full", cx);
                        return;
                    }
                    self.journal.push(WorkspaceTableCopy {
                        attempt_id: id,
                        description: review.description().clone(),
                        state: State::Staged,
                        failure: None,
                        diagnostic: None,
                    });
                    cx.emit(CopyEvent::Changed);
                }
                self.review = Some(review);
                self.message = None;
                cx.notify();
            }
            Ok(_) => self.fail("Copy review exceeds its retained bound", cx),
            Err(error) => self.fail(error, cx),
        }
    }
    pub fn apply(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        if self.busy()
            || !self
                .review
                .as_ref()
                .is_some_and(|review| review.attempt_id() == id)
        {
            self.fail("Select and inspect a current copy review first", cx);
            return;
        }
        let Some(record) = self
            .journal
            .iter_mut()
            .find(|r| r.attempt_id == id && r.state == State::Staged)
        else {
            self.fail(
                "Recovered copy intent cannot be executed; reconcile it explicitly",
                cx,
            );
            return;
        };
        let Some(request) = self.fence.begin(id) else {
            self.fail("Copy save identity is unavailable", cx);
            return;
        };
        record.state = State::Applying;
        self.message = Some("Saving exact copy recovery intent before dispatch".into());
        cx.emit(CopyEvent::Persist(request));
        cx.notify();
    }
    pub fn saved(&mut self, request: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        let Some(id) = self.fence.accept(request) else {
            return;
        };
        if let Err(error) = result {
            self.revert_apply(id, cx);
            self.fail(error, cx);
            return;
        }
        let result = {
            let _runtime = self.host.runtime.enter();
            if let Some(confirmation) = self.confirmation.take() {
                self.host.backend.confirm_table_copy(confirmation)
            } else if let Some(review) = self
                .review
                .clone()
                .filter(|review| review.attempt_id() == id)
            {
                self.host.backend.start_table_copy(review)
            } else {
                Err(TableCopyError::StaleReview)
            }
        };
        match result {
            Ok(TableCopySubmission::NeedsConfirmation(confirmation)) => {
                self.confirmation = Some(*confirmation);
                self.revert_apply(id, cx);
                self.message = Some(
                    "Destination policy requires explicit confirmation of this exact review".into(),
                );
            }
            Ok(TableCopySubmission::Accepted(_)) => {
                self.review = None;
                self.message = Some("Copy admitted. Check the attempt for its transaction outcome and cleanup status.".into());
            }
            Err(error) => {
                self.revert_apply(id, cx);
                self.fail(error, cx);
            }
        }
        self.refresh(cx);
    }
    fn set_state(&mut self, id: TableCopyAttemptId, state: State, cx: &mut Context<Self>) {
        if let Some(record) = self.journal.iter_mut().find(|r| r.attempt_id == id) {
            record.state = state;
            cx.emit(CopyEvent::Changed);
        }
    }
    fn revert_apply(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        if let Some(record) = self
            .journal
            .iter_mut()
            .find(|record| record.attempt_id == id && record.state == State::Applying)
        {
            record.state = State::Staged;
            record.failure = None;
            record.diagnostic = None;
            cx.emit(CopyEvent::Changed);
        }
    }
    pub fn cancel(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        self.fence.cancel(id);
        if self.review.as_ref().is_some_and(|r| r.attempt_id() == id) {
            self.review = None;
        }
        if self.confirmation_pending(id) {
            self.confirmation = None;
        }
        match self.host.backend.cancel_table_copy(id) {
            Ok(_) => {
                self.message = Some("Cancellation requested; waiting for joined cleanup".into());
                self.refresh(cx);
            }
            Err(error) => self.fail(error, cx),
        }
    }
    pub fn reconcile(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        if self.pending_apply(id)
            || self.jobs.iter().any(|job| {
                job.attempt_id == id
                    && (!job.phase.terminal() || job.cleanup != TableCopyCleanup::Complete)
            })
        {
            self.fail("Wait for copy cleanup before recording reconciliation", cx);
            return;
        }
        self.set_state(id, State::Reconciled, cx);
        self.message = Some("Reconciliation recorded. This attempt will never be replayed".into());
        cx.notify();
    }
    pub fn dismiss(&mut self, id: TableCopyAttemptId, cx: &mut Context<Self>) {
        if self.journal.iter().any(|r| {
            r.attempt_id == id
                && matches!(r.state, State::Applying | State::Unknown | State::Staged)
        }) {
            self.fail("Reconcile recovered intent before dismissing it", cx);
            return;
        }
        if self.jobs.iter().any(|job| job.attempt_id == id)
            && let Err(error) = self.host.backend.release_table_copy(id)
        {
            self.fail(error, cx);
            return;
        }
        self.journal.retain(|r| r.attempt_id != id);
        cx.emit(CopyEvent::Changed);
        self.refresh(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_save_ack_cannot_dispatch_or_consume_a_new_request() {
        let first = TableCopyAttemptId::new();
        let second = TableCopyAttemptId::new();
        let mut fence = SaveFence::default();
        let old = fence.begin(first).unwrap();
        fence.cancel(first);
        let new = fence.begin(second).unwrap();
        assert_eq!(fence.accept(old), None);
        assert_eq!(fence.accept(new), Some(second));
        assert_eq!(fence.accept(new), None);
    }
}
