pub(crate) use super::csv::CsvOptions;
use crate::postgres::sql_class::StatementClassSummary;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Direction {
    Import,
    Export,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Phase {
    Preparing,
    Running,
    Cancelling,
    Finalizing,
    Completed,
    Cancelled,
    Failed,
    OutcomeUnknown,
}
impl Phase {
    #[cfg(test)]
    pub(crate) fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::OutcomeUnknown
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum TransferError {
    UnsupportedEngine,
    InvalidRequest {
        field: String,
        reason: String,
    },
    ConnectionClosing,
    JobLimitReached,
    JobNotFound,
    JobActive,
    InspectionExpired,
    SourceChanged,
    TargetChanged,
    DestinationExists,
    UnsupportedTarget {
        reason: String,
    },
    ExportLimitExceeded {
        limit: ExportLimit,
    },
    Csv {
        record: u64,
        column: Option<usize>,
        reason: String,
    },
    Database {
        code: Option<String>,
        reason: String,
    },
    Io {
        operation: String,
        reason: String,
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
    OutcomeUnknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ExportLimit {
    Field,
    Record,
}

impl TransferError {
    pub(crate) fn invalid(field: &str, reason: &str) -> Self {
        Self::InvalidRequest {
            field: field.into(),
            reason: reason.into(),
        }
    }
    pub(crate) fn io(operation: &str, error: &std::io::Error) -> Self {
        Self::Io {
            operation: operation.into(),
            reason: error.kind().to_string(),
        }
    }
    pub(crate) fn database(error: &tokio_postgres::Error) -> Self {
        let code = error.code().map(|c| c.code().to_owned());
        let reason = match code.as_deref() {
            Some("23505") => "A unique constraint rejected a row",
            Some("23503") => "A foreign key rejected a row",
            Some("23502") => "A required column has no value",
            Some("23514") => "A check constraint rejected a row",
            Some("42501") => "Insufficient privileges",
            Some("22P02") => "A value has an invalid input format",
            Some("22007" | "22008") => "A date or time value is invalid",
            Some("57014") => "The database cancelled the operation",
            _ => "The database could not complete the transfer",
        };
        Self::Database {
            code,
            reason: reason.into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InspectPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub direction: Direction,
    pub source_path: Option<String>,
    pub options: CsvOptions,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TargetColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub has_default: bool,
    pub generated: bool,
    pub identity: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceColumn {
    pub index: usize,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Inspection {
    pub inspection_token: String,
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub direction: Direction,
    pub file_name: Option<String>,
    pub total_bytes: Option<u64>,
    pub source_columns: Vec<SourceColumn>,
    pub target_columns: Vec<TargetColumn>,
    pub sample_rows: Vec<Vec<Option<String>>>,
    pub sample_truncated: bool,
    pub options: CsvOptions,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColumnMapping {
    pub source_index: usize,
    pub target_column: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartImportPayload {
    pub inspection_token: String,
    pub mapping: Vec<ColumnMapping>,
    #[serde(default)]
    pub confirmed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartExportPayload {
    pub inspection_token: String,
    pub destination_path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub job_id: String,
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub direction: Direction,
    pub file_name: String,
    pub phase: Phase,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total_bytes: Option<u64>,
    pub bytes_processed: u64,
    pub rows_processed: Option<u64>,
    pub rows_committed: Option<u64>,
    pub failure: Option<TransferError>,
}
