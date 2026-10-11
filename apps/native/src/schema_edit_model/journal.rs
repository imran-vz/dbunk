//! The schema edit recovery record: the draft while it is edited and, from
//! Apply until the backend settles it, the exact attempt marked
//! `OutcomeUnknown`. Descriptive recovery only: restoring a record restores
//! the draft for editing, never authority, and an unknown attempt is
//! surfaced for reconciliation, never replayed. PostgreSQL journals it with
//! the workspace document; MySQL and SQLite keep it in memory only.
use dbunk_lib::backend::WorkspaceApplyState;
use dbunk_lib::backend::schema_edit::{
    SchemaBaseline, SchemaDraft, SchemaEditError, SchemaEditOutcome, SchemaEditReceipt,
    SchemaEditRequest, SchemaEditReview, WorkspaceSchemaEdit,
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

#[derive(Default)]
pub struct Recovery {
    record: Option<WorkspaceSchemaEdit>,
}

impl Recovery {
    pub fn new(restored: Option<WorkspaceSchemaEdit>) -> Self {
        Self { record: restored }
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

    /// The journaled draft for this table, when it was edited against a
    /// baseline whose table equals `baseline`'s (the drift token may move).
    pub fn restorable(
        &self,
        request: &SchemaEditRequest,
        baseline: &SchemaBaseline,
    ) -> Option<SchemaDraft> {
        self.record
            .as_ref()
            .filter(|record| {
                record.request == *request
                    && record.engine == baseline.engine
                    && record.baseline.table == baseline.table
            })
            .map(|record| record.draft.clone())
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
        let mut record = WorkspaceSchemaEdit::from_review(review)
            .map_err(|_| "The reviewed change cannot be saved for recovery; nothing was sent")?;
        record.mark_unknown();
        self.record = Some(record);
        Ok(())
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
