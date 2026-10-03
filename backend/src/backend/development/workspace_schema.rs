//! Exact create-schema intent and recovery identity, never a runtime review,
//! confirmation, SQL string, or permission to replay a restored attempt.
use super::{WorkspaceApplyState, WorkspaceError};
use crate::backend::schema_ddl::{CreateSchemaAttemptId, CreateSchemaIntent};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSchemaChanges {
    pub attempt_id: CreateSchemaAttemptId,
    pub intent: CreateSchemaIntent,
    pub apply_state: WorkspaceApplyState,
}

impl WorkspaceSchemaChanges {
    /// Validated types enforce exact names, comments and canonical UUIDv4 IDs.
    /// Retained capacity must also fit the intent's existing bounded contract.
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        self.intent
            .checked_heap_bytes()
            .ok_or(WorkspaceError::InvalidSnapshot)?;
        Ok(())
    }
}

impl fmt::Debug for WorkspaceSchemaChanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceSchemaChanges")
            .field("apply_state", &self.apply_state)
            .finish_non_exhaustive()
    }
}
