//! App-owned seed reviews and durable intent. The facade performs only bounded
//! registry work synchronously; all database work belongs to its joined owner.
use crate::controller::Host;
use dbunk_lib::backend::{WorkspaceTableSeed, WorkspaceTableSeedState as State, table_seed::*};
use gpui::{Context, EventEmitter, Task};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

// One inspection and one review (4 MiB each), one bounded list (1 MiB),
// journal descriptions and synchronous working copies. No generated rows.
const STORE_BYTES: usize = 12 * 1024 * 1024;
const MAX_JOURNAL: usize = 16;
struct Lease(Rc<Cell<usize>>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(STORE_BYTES));
    }
}
pub enum SeedEvent {
    Changed,
    Persist(u64),
    DestinationChanged(Option<String>),
}
mod fence;
mod recovery;
use fence::SaveFence;

pub struct SeedStore {
    host: Arc<Host>,
    journal: Vec<WorkspaceTableSeed>,
    jobs: Vec<TableSeedObservation>,
    inspection: Option<TableSeedInspection>,
    review: Option<TableSeedReview>,
    confirmation: Option<TableSeedConfirmation>,
    fence: SaveFence,
    current: bool,
    restored: bool,
    revision: u64,
    message: Option<String>,
    _lease: Option<Lease>,
    _poll: Task<()>,
}
impl EventEmitter<SeedEvent> for SeedStore {}
impl SeedStore {
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
            inspection: None,
            review: None,
            confirmation: None,
            fence: SaveFence::default(),
            current: false,
            restored: false,
            revision: 0,
            message: (!admitted)
                .then(|| "Table seeding needs 12 MiB of shared retained allowance".into()),
            _lease: lease,
            _poll: poll,
        }
    }
    pub fn restore(&mut self, journal: Vec<WorkspaceTableSeed>, cx: &mut Context<Self>) {
        if self.restored {
            return;
        }
        self.journal = journal;
        self.restored = true;
        self.refresh(cx);
    }
    pub fn snapshot(&self) -> Vec<WorkspaceTableSeed> {
        self.journal.clone()
    }
    pub fn journal(&self) -> &[WorkspaceTableSeed] {
        &self.journal
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.journal.iter().fold(0usize, |n, job| {
            n.saturating_add(job.checked_heap_bytes().unwrap_or(usize::MAX))
        })
    }
    pub fn jobs(&self) -> &[TableSeedObservation] {
        &self.jobs
    }
    pub fn review_payload(&self) -> Option<&TableSeedReview> {
        self.review.as_ref()
    }
    pub fn inspection_payload(&self) -> Option<&TableSeedInspection> {
        self.inspection.as_ref()
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
    pub fn pending_apply(&self, id: TableSeedAttemptId) -> bool {
        self.fence.pending.is_some_and(|(_, pending)| pending == id)
    }
    pub fn confirmation_pending(&self, id: TableSeedAttemptId) -> bool {
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
        let list = match self.host.backend.list_table_seeds() {
            Ok(list) if list.checked_heap_bytes().is_some() => list,
            Ok(_) => {
                self.current = false;
                self.fail("Seed observations exceed their bounds", cx);
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
            cx.emit(SeedEvent::DestinationChanged(None));
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
                && let Some(state) = recovery::observed_state(record.state, job.phase, job.outcome)
                && (record.state != state
                    || record.failure != job.failure
                    || record.diagnostic != job.diagnostic)
            {
                record.state = state;
                record.failure = job.failure;
                record.diagnostic = job.diagnostic.clone();
                changed = true;
            }
            if !gap
                && job
                    .change_revision
                    .is_some_and(|revision| revision > self.revision)
            {
                cx.emit(SeedEvent::DestinationChanged(Some(
                    job.endpoint.connection_id.clone(),
                )));
            }
        }
        self.revision = list.change_revision;
        self.jobs = list.jobs;
        self.current = true;
        if changed {
            cx.emit(SeedEvent::Changed);
        }
        cx.notify();
    }
    pub fn begin(
        &mut self,
        intent: TableSeedIntent,
        cx: &mut Context<Self>,
    ) -> Option<TableSeedAttemptId> {
        self.refresh(cx);
        if self.journal.iter().any(|record| {
            matches!(record.state, State::Unknown | State::Applying)
                && record.description.endpoint.connection_id == intent.endpoint.connection_id
        }) {
            self.fail("Reconcile the previous uncertain seed on this destination before preparing another", cx);
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
                "Seed recovery capacity is full or unavailable; resolve existing attempts first",
                cx,
            );
            return None;
        }
        let id = TableSeedAttemptId::new();
        let result = crate::table_seed_runtime::register(&self.host, id, intent);
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
    pub fn inspect(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        if self.busy() {
            self.fail("Finish saving the current seed review first", cx);
            return;
        }
        // Drop the old capture before allocating the next bounded payload.
        self.inspection = None;
        match self.host.backend.inspect_table_seed(id) {
            Ok(inspection) if inspection.retained_bytes() <= MAX_TABLE_SEED_REVIEW_BYTES => {
                self.inspection = Some(inspection);
                self.message = None;
                cx.notify();
            }
            Ok(_) => self.fail("Seed inspection exceeds its retained bound", cx),
            Err(error) => self.fail(error, cx),
        }
    }
    pub fn review(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        if self.busy() {
            self.fail("Finish saving the current review first", cx);
            return;
        }
        if self.confirmation.is_some() {
            self.fail("Confirm or cancel the current seed first", cx);
            return;
        }
        self.review = None;
        match self.host.backend.review_table_seed(id) {
            Ok(review) if review.retained_bytes() <= MAX_TABLE_SEED_REVIEW_BYTES => {
                if self.journal.iter().any(|record| {
                    record.attempt_id == id && record.description != *review.description()
                }) {
                    self.fail("Seed recovery does not match the current review", cx);
                    return;
                }
                if !self.journal.iter().any(|record| record.attempt_id == id) {
                    if self.journal.len() >= MAX_JOURNAL {
                        self.fail("Seed recovery capacity is full", cx);
                        return;
                    }
                    self.journal.push(WorkspaceTableSeed {
                        attempt_id: id,
                        description: review.description().clone(),
                        state: State::Staged,
                        failure: None,
                        diagnostic: None,
                    });
                    cx.emit(SeedEvent::Changed);
                }
                self.review = Some(review);
                self.message = None;
                cx.notify();
            }
            Ok(_) => self.fail("Seed review exceeds its retained bound", cx),
            Err(error) => self.fail(error, cx),
        }
    }
    pub fn apply(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        self.refresh(cx);
        if self.busy()
            || !self
                .review
                .as_ref()
                .is_some_and(|review| review.attempt_id() == id)
        {
            self.fail("Select and inspect a current seed review first", cx);
            return;
        }
        let Some(record) = self
            .journal
            .iter_mut()
            .find(|r| r.attempt_id == id && r.state == State::Staged)
        else {
            self.fail(
                "Recovered seed intent cannot be executed; reconcile it explicitly",
                cx,
            );
            return;
        };
        let Some(request) = self.fence.begin(id) else {
            self.fail("Seed save identity is unavailable", cx);
            return;
        };
        record.state = State::Applying;
        self.message = Some("Saving exact seed recovery intent before dispatch".into());
        cx.emit(SeedEvent::Persist(request));
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
        let Some(description) = self
            .journal
            .iter()
            .find(|record| record.attempt_id == id && record.state == State::Applying)
            .map(|record| record.description.clone())
        else {
            self.fail("The saved seed attempt is no longer awaiting dispatch", cx);
            return;
        };
        let result = crate::table_seed_runtime::dispatch(
            &self.host,
            id,
            &description,
            self.review.clone(),
            self.confirmation.take(),
        );
        match result {
            Ok(TableSeedSubmission::NeedsConfirmation(confirmation)) => {
                self.confirmation = Some(*confirmation);
                self.revert_apply(id, cx);
                self.message = Some(
                    "Destination policy requires explicit confirmation of this exact review".into(),
                );
            }
            Ok(TableSeedSubmission::Accepted(_)) => {
                self.review = None;
                self.message = Some("Seed admitted. Check the attempt for its transaction outcome and cleanup status.".into());
            }
            Err(error) => {
                self.revert_apply(id, cx);
                self.fail(error, cx);
            }
        }
        self.refresh(cx);
    }
    fn set_state(&mut self, id: TableSeedAttemptId, state: State, cx: &mut Context<Self>) {
        if let Some(record) = self.journal.iter_mut().find(|r| r.attempt_id == id) {
            record.state = state;
            cx.emit(SeedEvent::Changed);
        }
    }
    fn revert_apply(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        if let Some(record) = self
            .journal
            .iter_mut()
            .find(|record| record.attempt_id == id && record.state == State::Applying)
        {
            record.state = State::Staged;
            record.failure = None;
            record.diagnostic = None;
            cx.emit(SeedEvent::Changed);
        }
    }
    pub fn cancel(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        self.fence.cancel(id);
        // Inspection is non-executable, bounded metadata. Preserve it so an
        // explicit recipe replacement cannot erase the user's editable form.
        if self.review.as_ref().is_some_and(|r| r.attempt_id() == id) {
            self.review = None;
        }
        if self.confirmation_pending(id) {
            self.confirmation = None;
        }
        match self.host.backend.cancel_table_seed(id) {
            Ok(_) => {
                self.message = Some("Cancellation requested. Check the attempt's transaction outcome and cleanup status.".into());
                self.refresh(cx);
            }
            Err(error) => self.fail(error, cx),
        }
    }
    /// Explicitly discard only an unstarted preparation. Backend cancellation
    /// and release remain authoritative; failed settlement retains the attempt.
    pub fn discard_preparation(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) -> bool {
        self.refresh(cx);
        if !self.observation_current()
            || self.pending_apply(id)
            || self.journal.iter().any(|record| {
                record.attempt_id == id
                    && !matches!(record.state, State::Staged | State::NotStarted)
            })
            || !self.jobs.iter().any(|job| {
                job.attempt_id == id
                    && (matches!(
                        job.phase,
                        TableSeedPhase::NeedsRecipe | TableSeedPhase::ReadyReview
                    ) || job.phase.terminal())
                    && job.outcome == TableSeedOutcome::NotStarted
            })
        {
            self.fail("Only an unstarted seed preparation can be discarded", cx);
            return false;
        }
        self.cancel(id, cx);
        if self.jobs.iter().any(|job| {
            job.attempt_id == id
                && (!job.phase.terminal() || job.cleanup != TableSeedCleanup::Complete)
        }) {
            self.fail(
                "Wait for preparation cleanup before replacing the recipe",
                cx,
            );
            return false;
        }
        self.dismiss(id, cx);
        !self.jobs.iter().any(|job| job.attempt_id == id)
            && !self.journal.iter().any(|record| record.attempt_id == id)
    }
    pub fn reconcile(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        self.refresh(cx);
        if !self.observation_current()
            || self.pending_apply(id)
            || self.jobs.iter().any(|job| {
                job.attempt_id == id
                    && (!job.phase.terminal() || job.cleanup != TableSeedCleanup::Complete)
            })
        {
            self.fail("Wait for seed cleanup before recording reconciliation", cx);
            return;
        }
        self.set_state(id, State::Reconciled, cx);
        self.message = Some("Reconciliation recorded. This attempt will never be replayed".into());
        cx.notify();
    }
    pub fn dismiss(&mut self, id: TableSeedAttemptId, cx: &mut Context<Self>) {
        self.refresh(cx);
        if !self.observation_current() {
            self.fail("Refresh seed observations before dismissing recovery", cx);
            return;
        }
        if self.journal.iter().any(|r| {
            r.attempt_id == id
                && matches!(r.state, State::Applying | State::Unknown | State::Staged)
        }) {
            self.fail("Reconcile recovered intent before dismissing it", cx);
            return;
        }
        if self.jobs.iter().any(|job| job.attempt_id == id)
            && let Err(error) = self.host.backend.release_table_seed(id)
        {
            self.fail(error, cx);
            return;
        }
        self.journal.retain(|r| r.attempt_id != id);
        cx.emit(SeedEvent::Changed);
        self.refresh(cx);
    }
}
