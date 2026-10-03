//! Exact existing-schema comment/rename recovery description, never executable authority.
//! Staged records require a fresh observation/review after restoration. Persist
//! OutcomeUnknown and await that exact workspace revision before dispatch.
use super::{WorkspaceApplyState, WorkspaceError};
use crate::backend::schema_alter::{
    SchemaAlterAttemptId, SchemaAlterDescription, SchemaAlterIntent, SchemaAlterPreview,
    SchemaAlterReview,
};
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of};

/// Per-record allowance; the existing 448 KiB whole-workspace limit still wins.
pub const WORKSPACE_SCHEMA_ALTER_MAX_BYTES: usize = 48 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSchemaAlter {
    pub attempt_id: SchemaAlterAttemptId,
    pub target: SchemaAlterDescription,
    pub intent: SchemaAlterIntent,
    pub preview: SchemaAlterPreview,
    pub apply_state: WorkspaceApplyState,
}

impl WorkspaceSchemaAlter {
    pub fn from_review(review: &SchemaAlterReview) -> Result<Self, WorkspaceError> {
        let record = Self {
            attempt_id: review.attempt_id().clone(),
            target: review.target().clone(),
            intent: review.intent().clone(),
            preview: review.preview().clone(),
            apply_state: WorkspaceApplyState::Staged,
        };
        record.validate()?;
        Ok(record)
    }

    /// Equality is descriptive only. The host must additionally bind the live
    /// review to its document and wait for its exact persisted revision.
    pub fn matches_review(&self, review: &SchemaAlterReview) -> bool {
        self.matches(
            review.attempt_id(),
            review.target(),
            review.intent(),
            review.preview(),
        )
    }

    pub fn matches(
        &self,
        attempt_id: &SchemaAlterAttemptId,
        target: &SchemaAlterDescription,
        intent: &SchemaAlterIntent,
        preview: &SchemaAlterPreview,
    ) -> bool {
        self.attempt_id == *attempt_id
            && self.target == *target
            && self.intent == *intent
            && self.preview == *preview
    }

    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.retained_bytes() > WORKSPACE_SCHEMA_ALTER_MAX_BYTES
            || crate::backend::schema_ddl::encoded_bytes(self) > WORKSPACE_SCHEMA_ALTER_MAX_BYTES
            || !self
                .preview
                .matches_typed_description(&self.target, &self.intent)
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        Ok(())
    }

    /// Conservative actual-capacity accounting; nested inline sizes are counted
    /// twice intentionally. Invalid component bounds return a refusal sentinel.
    pub fn retained_bytes(&self) -> usize {
        let checked = || {
            size_of::<Self>()
                .checked_add(self.attempt_id.as_str().len())?
                .checked_add(self.target.checked_heap_bytes()?)?
                .checked_add(self.intent.checked_heap_bytes()?)?
                .checked_add(self.preview.checked_heap_bytes()?)
        };
        checked().unwrap_or(usize::MAX)
    }
}

impl fmt::Debug for WorkspaceSchemaAlter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceSchemaAlter")
            .field("apply_state", &self.apply_state)
            .finish_non_exhaustive()
    }
}
