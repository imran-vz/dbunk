//! Application-owned recovery journal. Descriptions retain reviewed identity,
//! never a review token, connection, executable SQL, or permission to replay.
use super::WorkspaceError;
use crate::backend::table_copy::{
    TableCopyAttemptId, TableCopyDescription, TableCopyDiagnostic, TableCopyError,
    MAX_TABLE_COPY_DESCRIPTION_BYTES,
};
use serde::{Deserialize, Serialize};
use std::{fmt, io::Write, mem::size_of};

pub const WORKSPACE_COPY_MAX_JOBS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum WorkspaceTableCopyState {
    Staged,
    /// Exact saved admission marker. Loading turns it into Unknown, never replay.
    Applying,
    Unknown,
    Completed {
        rows: u64,
    },
    RolledBack,
    NotStarted,
    /// Explicit user reconciliation, without claiming success or rollback.
    Reconciled,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceTableCopy {
    pub attempt_id: TableCopyAttemptId,
    pub description: TableCopyDescription,
    pub state: WorkspaceTableCopyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<TableCopyError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<TableCopyDiagnostic>,
}

impl WorkspaceTableCopy {
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        self.checked_heap_bytes()
            .map(|_| ())
            .ok_or(WorkspaceError::InvalidSnapshot)
    }

    /// Accounts actual retained capacities and checks description serialization
    /// without allocating an escaped copy. The workspace adds its 448 KiB cap.
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if matches!(
            self.state,
            WorkspaceTableCopyState::Staged | WorkspaceTableCopyState::Applying
        ) && (self.failure.is_some() || self.diagnostic.is_some())
        {
            return None;
        }
        // A known COMMIT can be followed by failed cleanup. Preserve that
        // distinction instead of replacing a known success with uncertainty.
        if matches!(self.state, WorkspaceTableCopyState::Completed { .. })
            && (self
                .failure
                .is_some_and(|failure| failure != TableCopyError::Cleanup)
                || (self.failure.is_none() && self.diagnostic.is_some()))
        {
            return None;
        }
        let description_bytes = self.description.checked_heap_bytes()?;
        let mut encoded = DescriptionLimit(0);
        serde_json::to_writer(&mut encoded, &self.description).ok()?;
        let diagnostic_bytes = match &self.diagnostic {
            None => 0,
            Some(diagnostic) => {
                let bytes = match &diagnostic.sqlstate {
                    None => 0,
                    Some(code) => {
                        if code.len() != 5
                            || code.capacity() > 8
                            || !code.bytes().all(|byte| byte.is_ascii_alphanumeric())
                        {
                            return None;
                        }
                        code.capacity()
                    }
                };
                size_of::<TableCopyDiagnostic>().checked_add(bytes)?
            }
        };
        size_of::<Self>()
            .checked_add(description_bytes)?
            .checked_add(diagnostic_bytes)
    }

    pub(super) fn restore(&mut self) {
        if self.state == WorkspaceTableCopyState::Applying {
            self.state = WorkspaceTableCopyState::Unknown;
        }
    }
}

impl fmt::Debug for WorkspaceTableCopy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceTableCopy")
            .field("attempt_id", &self.attempt_id)
            .field("state", &self.state)
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}

struct DescriptionLimit(usize);
impl Write for DescriptionLimit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_TABLE_COPY_DESCRIPTION_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("copy description limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "workspace_copy_tests.rs"]
mod tests;
