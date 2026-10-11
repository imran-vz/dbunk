//! Plan 033 journal record: one table's schema draft and, once applied, the
//! exact attempt. Descriptive recovery, never executable authority: a
//! restored draft is re-based on a fresh observation before review, and an
//! `OutcomeUnknown` attempt is surfaced for reconciliation, never replayed.
//! The host persists the attempt as `OutcomeUnknown` and awaits that exact
//! workspace revision before dispatch.
use super::{WorkspaceApplyState, WorkspaceError};
use crate::backend::schema_edit::{
    plan_review, SchemaBaseline, SchemaDraft, SchemaEditAttemptId, SchemaEditPreview,
    SchemaEditRequest, SchemaEditReview, SchemaEngine,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Per-record allowance; the 448 KiB whole-workspace limit still wins. A
/// draft of a very wide table may not fit; the host then keeps it in memory
/// only and says so.
pub const WORKSPACE_SCHEMA_EDIT_MAX_BYTES: usize = 192 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSchemaEditAttempt {
    pub attempt_id: SchemaEditAttemptId,
    pub preview: SchemaEditPreview,
    pub apply_state: WorkspaceApplyState,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSchemaEdit {
    pub engine: SchemaEngine,
    pub request: SchemaEditRequest,
    /// The baseline the draft was edited against.
    pub baseline: SchemaBaseline,
    pub draft: SchemaDraft,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<WorkspaceSchemaEditAttempt>,
}

impl WorkspaceSchemaEdit {
    /// A draft in progress, before any review.
    pub fn draft(
        request: SchemaEditRequest,
        baseline: SchemaBaseline,
        draft: SchemaDraft,
    ) -> Result<Self, WorkspaceError> {
        let record = Self {
            engine: baseline.engine,
            request,
            baseline,
            draft,
            attempt: None,
        };
        record.validate()?;
        Ok(record)
    }

    /// The reviewed attempt, `Staged`. Mark it `OutcomeUnknown` and persist
    /// before dispatch.
    pub fn from_review(review: &SchemaEditReview) -> Result<Self, WorkspaceError> {
        let record = Self {
            engine: review.baseline().engine,
            request: review.target().request().clone(),
            baseline: review.baseline().clone(),
            draft: review.draft().clone(),
            attempt: Some(WorkspaceSchemaEditAttempt {
                attempt_id: review.attempt_id().clone(),
                preview: review.preview().clone(),
                apply_state: WorkspaceApplyState::Staged,
            }),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn mark_unknown(&mut self) {
        if let Some(attempt) = &mut self.attempt {
            attempt.apply_state = WorkspaceApplyState::OutcomeUnknown;
        }
    }

    pub fn outcome_unknown(&self) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|attempt| attempt.apply_state == WorkspaceApplyState::OutcomeUnknown)
    }

    /// Equality is descriptive only. The host must additionally bind the
    /// live review to its source and wait for the persisted revision.
    pub fn matches_review(&self, review: &SchemaEditReview) -> bool {
        self.attempt.as_ref().is_some_and(|attempt| {
            attempt.attempt_id == *review.attempt_id() && attempt.preview == *review.preview()
        }) && self.baseline == *review.baseline()
            && self.draft == *review.draft()
            && self.request == *review.target().request()
    }

    pub fn validate(&self) -> Result<(), WorkspaceError> {
        let bytes = crate::backend::schema_ddl::encoded_bytes(self);
        let consistent = self.engine == self.baseline.engine
            && self.request.validate().is_ok()
            && self.baseline.table.schema == self.request.schema
            && self.baseline.table.name == self.request.table;
        // A journaled preview must be exactly what the planner renders for
        // the journaled baseline and draft.
        let preview_ok = self.attempt.as_ref().is_none_or(|attempt| {
            plan_review(&self.baseline, &self.draft, &attempt.preview.options)
                .is_ok_and(|expected| expected == attempt.preview)
        });
        if bytes > WORKSPACE_SCHEMA_EDIT_MAX_BYTES || !consistent || !preview_ok {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        Ok(())
    }
}

impl fmt::Debug for WorkspaceSchemaEdit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceSchemaEdit")
            .field("engine", &self.engine)
            .field("attempt", &self.attempt.as_ref().map(|a| a.apply_state))
            .finish_non_exhaustive()
    }
}
