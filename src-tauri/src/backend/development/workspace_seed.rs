//! Application-owned recovery journal. Descriptions retain reviewed identity,
//! never a review token, connection, executable SQL, or permission to replay.
use super::WorkspaceError;
use crate::backend::table_seed::{
    TableSeedAttemptId, TableSeedDescription, TableSeedDiagnostic, TableSeedError,
    MAX_TABLE_SEED_DESCRIPTION_BYTES,
};
use serde::{Deserialize, Serialize};
use std::{fmt, io::Write, mem::size_of};

pub const WORKSPACE_SEED_MAX_JOBS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum WorkspaceTableSeedState {
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
pub struct WorkspaceTableSeed {
    pub attempt_id: TableSeedAttemptId,
    pub description: TableSeedDescription,
    pub state: WorkspaceTableSeedState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<TableSeedError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<TableSeedDiagnostic>,
}

impl WorkspaceTableSeed {
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
            WorkspaceTableSeedState::Staged | WorkspaceTableSeedState::Applying
        ) && (self.failure.is_some() || self.diagnostic.is_some())
        {
            return None;
        }
        // A known COMMIT can be followed by failed cleanup. Preserve that
        // distinction instead of replacing a known success with uncertainty.
        if matches!(self.state, WorkspaceTableSeedState::Completed { .. })
            && (self
                .failure
                .is_some_and(|failure| failure != TableSeedError::Cleanup)
                || (self.failure.is_none() && self.diagnostic.is_some()))
        {
            return None;
        }
        if matches!(self.state, WorkspaceTableSeedState::Completed { rows } if rows > u64::from(self.description.row_count))
        {
            return None;
        }
        let description_bytes = self.description.checked_heap_bytes()?;
        let mut encoded = DescriptionLimit(0);
        serde_json::to_writer(&mut encoded, &self.description).ok()?;
        let diagnostic_bytes = self
            .diagnostic
            .as_ref()
            .map_or(Some(0), |value| value.checked_heap_bytes())?;
        size_of::<Self>()
            .checked_add(description_bytes)?
            .checked_add(diagnostic_bytes)
    }

    pub(super) fn restore(&mut self) {
        if self.state == WorkspaceTableSeedState::Applying {
            self.state = WorkspaceTableSeedState::Unknown;
        }
    }
}

impl fmt::Debug for WorkspaceTableSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceTableSeed")
            .field("attempt_id", &self.attempt_id)
            .field("state", &self.state)
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}

struct DescriptionLimit(usize);
impl Write for DescriptionLimit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_TABLE_SEED_DESCRIPTION_BYTES.saturating_sub(self.0) {
            return Err(std::io::Error::other("seed description limit"));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "workspace_seed_tests.rs"]
mod tests;
