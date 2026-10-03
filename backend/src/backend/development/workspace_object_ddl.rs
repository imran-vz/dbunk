//! Exact typed object-DDL recovery description, never executable authority.
//! Staged records require a fresh observation/review after restoration. Persist
//! OutcomeUnknown and await that exact workspace revision before dispatch.
//! Drop-impact evidence is review-only and is never journaled.
use super::{WorkspaceApplyState, WorkspaceError};
use crate::backend::object_ddl::{
    ObjectDdlAttemptId, ObjectDdlDescription, ObjectDdlOperation, ObjectDdlPreview, ObjectDdlReview,
};
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of};

/// Per-record allowance; the existing 448 KiB whole-workspace limit still wins.
pub const WORKSPACE_OBJECT_DDL_MAX_BYTES: usize = 96 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceObjectDdl {
    pub attempt_id: ObjectDdlAttemptId,
    pub target: ObjectDdlDescription,
    pub operations: Vec<ObjectDdlOperation>,
    pub preview: ObjectDdlPreview,
    pub apply_state: WorkspaceApplyState,
}

impl WorkspaceObjectDdl {
    pub fn from_review(review: &ObjectDdlReview) -> Result<Self, WorkspaceError> {
        let record = Self {
            attempt_id: review.attempt_id().clone(),
            target: review.target().clone(),
            operations: review.operations().to_vec(),
            preview: review.preview().clone(),
            apply_state: WorkspaceApplyState::Staged,
        };
        record.validate()?;
        Ok(record)
    }

    /// Equality is descriptive only. The host must additionally bind the live
    /// review to its document and wait for its exact persisted revision.
    pub fn matches_review(&self, review: &ObjectDdlReview) -> bool {
        self.matches(
            review.attempt_id(),
            review.target(),
            review.operations(),
            review.preview(),
        )
    }

    pub fn matches(
        &self,
        attempt_id: &ObjectDdlAttemptId,
        target: &ObjectDdlDescription,
        operations: &[ObjectDdlOperation],
        preview: &ObjectDdlPreview,
    ) -> bool {
        self.attempt_id == *attempt_id
            && self.target == *target
            && self.operations == operations
            && self.preview == *preview
    }

    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.retained_bytes() > WORKSPACE_OBJECT_DDL_MAX_BYTES
            || crate::backend::schema_ddl::encoded_bytes(self) > WORKSPACE_OBJECT_DDL_MAX_BYTES
            || !self
                .preview
                .matches_typed_description(&self.target, &self.operations)
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        Ok(())
    }

    /// Conservative actual-capacity accounting. Invalid component bounds
    /// return a refusal sentinel.
    pub fn retained_bytes(&self) -> usize {
        let checked = || {
            size_of::<Self>()
                .checked_add(self.attempt_id.as_str().len())?
                .checked_add(self.target.checked_heap_bytes()?)?
                .checked_add(crate::backend::object_ddl::operations_heap_bytes(
                    &self.operations,
                )?)?
                .checked_add(self.preview.checked_heap_bytes()?)
        };
        checked().unwrap_or(usize::MAX)
    }
}

impl fmt::Debug for WorkspaceObjectDdl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceObjectDdl")
            .field("apply_state", &self.apply_state)
            .field("operations", &self.operations.len())
            .finish_non_exhaustive()
    }
}
