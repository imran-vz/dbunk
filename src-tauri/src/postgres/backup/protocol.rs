use serde::{Deserialize, Serialize};

use crate::postgres::sql_class::StatementClassSummary;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum PgBackupScope {
    Database,
    Schema { schema: String },
    Table { schema: String, table: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PgBackupFormat {
    Plain,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PgToolJobKind {
    Backup,
    Restore,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PgToolJobPhase {
    Queued,
    Preflight,
    Running,
    Finalizing,
    Completed,
    Cancelling,
    Cancelled,
    Failed,
}
impl PgToolJobPhase {
    pub(crate) fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgToolJobError {
    UnsupportedEngine,
    InvalidRequest {
        field: String,
        reason: String,
    },
    ConnectionClosing,
    JobLimitReached,
    JobNotFound,
    JobActive,
    DestinationExists,
    ToolUnavailable {
        tool: String,
    },
    ToolFailed {
        tool: String,
        exit_code: Option<i32>,
        message: String,
    },
    Io {
        operation: String,
        message: String,
    },
    Timeout {
        operation: String,
    },
    PolicyBlocked {
        reason: String,
    },
    PolicyNeedsConfirmation {
        statements: Vec<StatementClassSummary>,
    },
    Cancelled,
}
impl PgToolJobError {
    pub(crate) fn invalid(field: &str, reason: &str) -> Self {
        Self::InvalidRequest {
            field: field.into(),
            reason: reason.into(),
        }
    }
    // OS error kinds convey the failure without embedding a path or database data.
    pub(crate) fn io(operation: &str, error: &std::io::Error) -> Self {
        Self::Io {
            operation: operation.into(),
            message: error.kind().to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartPgBackupPayload {
    pub connection_id: String,
    pub destination_path: String,
    pub format: PgBackupFormat,
    pub scope: PgBackupScope,
    pub clean: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartPgRestorePayload {
    pub connection_id: String,
    pub source_path: String,
    pub format: PgBackupFormat,
    pub clean: bool,
    #[serde(default)]
    pub confirmed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgToolJobSnapshot {
    pub job_id: String,
    pub connection_id: String,
    pub kind: PgToolJobKind,
    pub format: PgBackupFormat,
    pub file_name: String,
    pub phase: PgToolJobPhase,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub bytes_processed: Option<u64>,
    pub total_bytes: Option<u64>,
    pub tool_version: Option<String>,
    pub failure: Option<PgToolJobError>,
}
