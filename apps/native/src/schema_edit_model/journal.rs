//! The schema edit recovery record: the draft while it is edited and, from
//! Apply until the backend settles it, the exact attempt marked
//! `OutcomeUnknown`. Descriptive recovery only: restoring a record restores
//! the draft for editing, never authority, and an unknown attempt is
//! surfaced for reconciliation, never replayed. PostgreSQL journals it with
//! the workspace document; MySQL and SQLite keep it in memory only.
use dbunk_lib::backend::WorkspaceApplyState;
use dbunk_lib::backend::schema_edit::{
    SchemaBaseline, SchemaDraft, SchemaEditError, SchemaEditOutcome, SchemaEditReceipt,
    SchemaEditRequest, SchemaEditReview, SchemaEngine, WorkspaceSchemaEdit,
    WorkspaceSchemaEditAttempt,
};

/// What a settled receipt means for the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settlement {
    /// Every statement ran and verified; the draft is consumed.
    Applied,
    /// Nothing changed on the server; the draft is kept for editing.
    Unchanged,
    /// Some statements committed; the draft is stale and dropped.
    Partial,
    /// The outcome is unknown, or the receipt does not match the journal.
    Unknown,
}

/// Why the draft could not be journaled. The draft stays in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftSave {
    Saved,
    /// No changes: nothing to keep.
    Cleared,
    /// Too large for the workspace record, or inconsistent.
    MemoryOnly,
    /// An unknown attempt is never overwritten by a draft.
    Blocked,
}

/// Why Apply refused before dispatch: the PostgreSQL journal must hold the
/// exact attempt before it is sent, and this one does not fit.
pub const JOURNAL_REFUSAL: &str = "This change cannot be applied: the workspace recovery journal cannot hold it, and it must be saved before dispatch. Nothing was sent; apply it in smaller steps.";

#[derive(Default)]
pub struct Recovery {
    record: Option<WorkspaceSchemaEdit>,
    /// PostgreSQL journals the record with the workspace document; MySQL
    /// and SQLite keep it in memory only, so no size limit applies.
    persisted: bool,
}

impl Recovery {
    pub fn new(restored: Option<WorkspaceSchemaEdit>, persisted: bool) -> Self {
        Self {
            record: restored,
            persisted,
        }
    }

    pub fn record(&self) -> Option<&WorkspaceSchemaEdit> {
        self.record.as_ref()
    }

    pub fn unknown(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(WorkspaceSchemaEdit::outcome_unknown)
    }

    /// The journaled attempt identity, for the reconcile notice.
    pub fn attempt_id(&self) -> Option<&str> {
        self.record
            .as_ref()
            .and_then(|record| record.attempt.as_ref())
            .map(|attempt| attempt.attempt_id.as_str())
    }

    /// The journaled baseline and draft for this table on this engine. The
    /// caller restores the draft as is when the captured table still equals
    /// the journaled baseline's, and re-bases it otherwise.
    pub fn restorable(
        &self,
        request: &SchemaEditRequest,
        engine: SchemaEngine,
    ) -> Option<(&SchemaBaseline, &SchemaDraft)> {
        self.record
            .as_ref()
            .filter(|record| record.request == *request && record.engine == engine)
            .map(|record| (&record.baseline, &record.draft))
    }

    /// Journals the draft in progress. An unchanged draft clears the record.
    pub fn save_draft(
        &mut self,
        request: &SchemaEditRequest,
        baseline: &SchemaBaseline,
        draft: &SchemaDraft,
    ) -> DraftSave {
        if self.unknown() {
            return DraftSave::Blocked;
        }
        if draft.table == baseline.table {
            self.record = None;
            return DraftSave::Cleared;
        }
        match WorkspaceSchemaEdit::draft(request.clone(), baseline.clone(), draft.clone()) {
            Ok(record) => {
                self.record = Some(record);
                DraftSave::Saved
            }
            Err(_) => {
                self.record = None;
                DraftSave::MemoryOnly
            }
        }
    }

    /// Records the exact reviewed attempt as `OutcomeUnknown`. The host
    /// persists it and awaits that revision before dispatch.
    pub fn stage(&mut self, review: &SchemaEditReview) -> Result<(), &'static str> {
        if self.unknown() {
            return Err("Reconcile the previous unknown outcome first");
        }
        self.stage_record(WorkspaceSchemaEdit {
            engine: review.baseline().engine,
            request: review.target().request().clone(),
            baseline: review.baseline().clone(),
            draft: review.draft().clone(),
            attempt: Some(WorkspaceSchemaEditAttempt {
                attempt_id: review.attempt_id().clone(),
                preview: review.preview().clone(),
                apply_state: WorkspaceApplyState::Staged,
            }),
        })
    }

    /// Marks a staged attempt record `OutcomeUnknown` and keeps it. Only a
    /// persisted journal enforces the workspace record limit: an in-memory
    /// record is never written, so a wide table still applies there.
    pub(super) fn stage_record(
        &mut self,
        mut record: WorkspaceSchemaEdit,
    ) -> Result<(), &'static str> {
        if self.unknown() {
            return Err("Reconcile the previous unknown outcome first");
        }
        if self.persisted && record.validate().is_err() {
            return Err(JOURNAL_REFUSAL);
        }
        record.mark_unknown();
        self.record = Some(record);
        Ok(())
    }

    /// MySQL and SQLite: nothing is journaled, so a draft record would only
    /// cost a copy per edit. Drops a plain draft record; an attempt stays.
    pub fn forget_draft(&mut self) {
        if self
            .record
            .as_ref()
            .is_some_and(|record| record.attempt.is_none())
        {
            self.record = None;
        }
    }

    pub fn matches_review(&self, review: &SchemaEditReview) -> bool {
        self.record
            .as_ref()
            .is_some_and(|record| record.matches_review(review))
    }

    /// Nothing was dispatched: drop the attempt, keep the draft.
    pub fn not_sent(&mut self) {
        if let Some(record) = &mut self.record {
            record.attempt = None;
        }
    }

    /// A refusal before dispatch keeps the draft; a lost reply stays unknown.
    pub fn submission_error(&mut self, error: &SchemaEditError) {
        if *error != SchemaEditError::OutcomeUnavailable {
            self.not_sent();
        }
    }

    /// Settles the journaled attempt from its receipt. A receipt for another
    /// attempt or table never settles anything: the record stays unknown.
    pub fn settle(
        &mut self,
        receipt: &SchemaEditReceipt,
        request: &SchemaEditRequest,
    ) -> Settlement {
        let matches = receipt.request == *request
            && self.record.as_ref().is_some_and(|record| {
                record.request == *request
                    && record.attempt.as_ref().is_some_and(|attempt| {
                        attempt.attempt_id == receipt.attempt_id
                            && attempt.apply_state == WorkspaceApplyState::OutcomeUnknown
                    })
            });
        if !matches {
            if let Some(record) = &mut self.record {
                record.mark_unknown();
            }
            return Settlement::Unknown;
        }
        match receipt.outcome {
            SchemaEditOutcome::Applied { .. } => {
                self.record = None;
                Settlement::Applied
            }
            SchemaEditOutcome::NotDispatched { .. } | SchemaEditOutcome::RolledBack { .. } => {
                self.not_sent();
                Settlement::Unchanged
            }
            SchemaEditOutcome::Stopped { .. } => {
                self.record = None;
                Settlement::Partial
            }
            SchemaEditOutcome::OutcomeUnknown { .. } => Settlement::Unknown,
        }
    }

    /// An unknown outcome needs an explicit reconciliation acknowledgement.
    pub fn discard(&mut self, reconciled: bool) -> bool {
        if self.unknown() && !reconciled {
            return false;
        }
        self.record = None;
        true
    }
}
