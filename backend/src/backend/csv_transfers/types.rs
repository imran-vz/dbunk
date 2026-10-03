//! Native CSV values are bounded before admission; paths and preview rows never
//! appear in job observations, diagnostics, or durable workspace state.
pub use crate::backend::pg_tools::PgToolTarget as CsvConnectionTarget;
use serde::Serialize;
use std::{fmt, path::PathBuf};

pub const MAX_CSV_INSPECTIONS: usize = 8;
pub const MAX_CSV_ACTIVE: usize = 4;
pub const MAX_CSV_TERMINAL: usize = 32;
pub const MAX_CSV_INSPECTION_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CSV_INSPECTION_POOL_BYTES: usize = MAX_CSV_INSPECTIONS * MAX_CSV_INSPECTION_BYTES;
pub const MAX_CSV_LIST_BYTES: usize = 256 * 1024;
pub const MAX_CSV_EXECUTION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CSV_EXECUTION_POOL_BYTES: usize = 128 * 1024 * 1024;
pub const CSV_INSPECTION_TTL_SECONDS: u64 = 300;
pub const MAX_CSV_COLUMNS: usize = 1600;
pub const MAX_CSV_PATH_BYTES: usize = 4096;

macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(pub(super) uuid::Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4())
            }
            pub fn parse(value: &str) -> Result<Self, CsvError> {
                let id = uuid::Uuid::parse_str(value).map_err(|_| CsvError::InvalidRequest)?;
                if id.get_version_num() != 4 {
                    return Err(CsvError::InvalidRequest);
                }
                Ok(Self(id))
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
    };
}
identity!(CsvInspectionId);
identity!(CsvTransferAttemptId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvDirection {
    Import,
    Export,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CsvOptions {
    pub delimiter: String,
    pub quote: String,
    pub escape: String,
    pub null_token: String,
    pub header: bool,
}
impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ",".into(),
            quote: "\"".into(),
            escape: "\"".into(),
            null_token: "\\N".into(),
            header: true,
        }
    }
}
impl CsvOptions {
    pub fn validate(&self) -> Result<(), CsvError> {
        if self.delimiter.len() != 1
            || self.quote.len() != 1
            || self.escape.len() != 1
            || self.null_token.len() > 1024
        {
            return Err(CsvError::InvalidOptions);
        }
        self.legacy()
            .validate()
            .map_err(|_| CsvError::InvalidOptions)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.validate().ok()?;
        [
            self.delimiter.capacity(),
            self.quote.capacity(),
            self.escape.capacity(),
            self.null_token.capacity(),
        ]
        .into_iter()
        .try_fold(std::mem::size_of::<Self>(), usize::checked_add)
        .filter(|n| *n <= 2048)
    }
    pub(super) fn legacy(&self) -> crate::postgres::transfer::csv::CsvOptions {
        crate::postgres::transfer::csv::CsvOptions {
            delimiter: self.delimiter.clone(),
            quote: self.quote.clone(),
            escape: self.escape.clone(),
            null_token: self.null_token.clone(),
            header: self.header,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CsvTarget {
    pub schema: String,
    pub table: String,
}
impl CsvTarget {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !identifier(&self.schema) || !identifier(&self.table) {
            return None;
        }
        std::mem::size_of::<Self>()
            .checked_add(self.schema.capacity())?
            .checked_add(self.table.capacity())
    }
}
#[derive(Clone)]
pub struct CsvInspectionIntent {
    pub(super) direction: CsvDirection,
    pub(super) target: CsvTarget,
    pub(super) options: CsvOptions,
    pub(super) source: Option<PathBuf>,
    pub(super) xlsx: bool,
}
impl CsvInspectionIntent {
    pub fn import(
        source: PathBuf,
        target: CsvTarget,
        options: CsvOptions,
    ) -> Result<Self, CsvError> {
        path(&source)?;
        let value = Self {
            direction: CsvDirection::Import,
            target,
            options,
            source: Some(source),
            xlsx: false,
        };
        value.checked_heap_bytes().ok_or(CsvError::InvalidRequest)?;
        Ok(value)
    }
    pub fn xlsx(source: PathBuf, target: CsvTarget, null_token: String) -> Result<Self, CsvError> {
        let mut intent = Self::import(
            source,
            target,
            CsvOptions {
                null_token,
                ..CsvOptions::default()
            },
        )?;
        intent.xlsx = true;
        Ok(intent)
    }
    pub fn is_xlsx(&self) -> bool {
        self.xlsx
    }
    pub fn export(target: CsvTarget, options: CsvOptions) -> Result<Self, CsvError> {
        let value = Self {
            direction: CsvDirection::Export,
            target,
            options,
            source: None,
            xlsx: false,
        };
        value.checked_heap_bytes().ok_or(CsvError::InvalidRequest)?;
        Ok(value)
    }
    pub fn direction(&self) -> CsvDirection {
        self.direction
    }
    pub fn target(&self) -> &CsvTarget {
        &self.target
    }
    pub fn options(&self) -> &CsvOptions {
        &self.options
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        std::mem::size_of::<Self>()
            .checked_add(self.target.checked_heap_bytes()?)?
            .checked_add(self.options.checked_heap_bytes()?)?
            .checked_add(self.source.as_ref().map_or(0, PathBuf::capacity))
    }
}
impl fmt::Debug for CsvInspectionIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CsvInspectionIntent")
            .field("direction", &self.direction)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CsvSourceColumn {
    pub index: usize,
    pub name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CsvTargetColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub has_default: bool,
    pub generated: bool,
    pub identity: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CsvMapping {
    pub source_index: usize,
    pub target_column: String,
}
#[derive(Serialize)]
pub struct CsvInspectionData {
    pub workbook: Option<super::CsvWorkbookSource>,
    pub inspection_id: CsvInspectionId,
    pub connection_id: String,
    pub target: CsvTarget,
    pub direction: CsvDirection,
    pub file_name: Option<String>,
    pub total_bytes: Option<u64>,
    pub source_columns: Vec<CsvSourceColumn>,
    pub target_columns: Vec<CsvTargetColumn>,
    pub sample_rows: Vec<Vec<Option<String>>>,
    pub sample_truncated: bool,
    pub options: CsvOptions,
    pub connection: CsvConnectionTarget,
    pub expires_at: String,
}
impl fmt::Debug for CsvInspectionData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CsvInspectionData")
            .field("inspection_id", &self.inspection_id)
            .field("direction", &self.direction)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvInspectionPhase {
    WorkbookReady,
    Preparing,
    Ready,
    Cancelling,
    Cancelled,
    Failed,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvInspectionObservation {
    pub inspection_id: CsvInspectionId,
    pub connection_id: String,
    pub target: CsvTarget,
    pub direction: CsvDirection,
    pub phase: CsvInspectionPhase,
    pub cleanup: CsvCleanup,
    pub expires_at: Option<String>,
    pub failure: Option<CsvError>,
    pub diagnostic: Option<CsvDiagnostic>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvTransferPhase {
    AwaitingConfirmation,
    Preparing,
    Running,
    Cancelling,
    Finalizing,
    Completed,
    Cancelled,
    Failed,
    OutcomeUnknown,
}
impl CsvTransferPhase {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::OutcomeUnknown
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvEffect {
    NotApplied,
    Pending,
    Succeeded,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvCleanup {
    Pending,
    Complete,
    Failed,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvTransferObservation {
    pub workbook: Option<super::CsvWorkbookSource>,
    pub attempt_id: CsvTransferAttemptId,
    pub inspection_id: CsvInspectionId,
    pub connection_id: String,
    pub target: CsvTarget,
    pub direction: CsvDirection,
    pub file_name: String,
    pub phase: CsvTransferPhase,
    pub effect: CsvEffect,
    pub cleanup: CsvCleanup,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total_bytes: Option<u64>,
    pub bytes_processed: u64,
    pub rows_processed: Option<u64>,
    pub rows_committed: Option<u64>,
    pub failure: Option<CsvError>,
    pub diagnostic: Option<CsvDiagnostic>,
    pub import_change_revision: Option<u64>,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvTransferList {
    pub jobs: Vec<CsvTransferObservation>,
    pub import_change_revision: u64,
    pub execution_reserved_bytes: usize,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvInspectionList {
    pub inspections: Vec<CsvInspectionObservation>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvExportLimit {
    Field,
    Record,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvDiagnostic {
    pub record: Option<u64>,
    pub column: Option<usize>,
    pub sqlstate: Option<String>,
    pub operation: Option<String>,
    pub reason: String,
    pub export_limit: Option<CsvExportLimit>,
}
impl CsvDiagnostic {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.reason.len() > 512
            || self.operation.as_ref().is_some_and(|s| s.len() > 64)
            || self.sqlstate.as_ref().is_some_and(|s| {
                s.len() != 5
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            })
            || self.column.is_some_and(|c| c > MAX_CSV_COLUMNS + 1)
        {
            return None;
        }
        std::mem::size_of::<Self>()
            .checked_add(self.reason.capacity())?
            .checked_add(self.operation.as_ref().map_or(0, String::capacity))?
            .checked_add(self.sqlstate.as_ref().map_or(0, String::capacity))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvError {
    InvalidWorkbook,
    UnsupportedWorkbook,
    MissingFormulaCache,
    InvalidRequest,
    InvalidOptions,
    InvalidMapping,
    UnsupportedTarget,
    Busy,
    WorkBudget,
    Missing,
    InspectionExpired,
    SourceChanged,
    TargetChanged,
    ForeignReview,
    StaleReview,
    Closing,
    Cancelled,
    PolicyBlocked,
    Credentials,
    Database,
    Csv,
    FileIo,
    DestinationExists,
    Timeout,
    Cleanup,
    Active,
    DuplicateAttempt,
    Limit,
}
impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidWorkbook => "Invalid XLSX workbook; repair or resave the workbook and inspect again",
            Self::UnsupportedWorkbook => "Unsupported XLSX workbook or cell value; use an unencrypted UTF-8 XLSX workbook with supported cached values",
            Self::MissingFormulaCache => "XLSX formula has no cached value; recalculate and save the workbook, then inspect again",
            Self::InvalidRequest => "Invalid or oversized CSV request",
            Self::InvalidOptions => "Invalid CSV delimiter, quote, escape or NULL token",
            Self::InvalidMapping => {
                "CSV mapping is incomplete, duplicated or incompatible with the target"
            }
            Self::UnsupportedTarget => "This relation is not supported for this CSV operation",
            Self::Busy => "CSV inspection or transfer capacity is full",
            Self::WorkBudget => "CSV execution workspace is full; no execution was dispatched",
            Self::Missing => "CSV attempt is unavailable",
            Self::InspectionExpired => "CSV inspection expired; inspect again",
            Self::SourceChanged => "CSV source changed; inspect again",
            Self::TargetChanged => "CSV target changed; inspect again",
            Self::ForeignReview => "CSV review belongs to another profile",
            Self::StaleReview => "CSV review is no longer current",
            Self::Closing => "Connection or application is closing",
            Self::Cancelled => "CSV operation cancelled",
            Self::PolicyBlocked => "Stored connection policy blocks this import",
            Self::Credentials => "Connection credentials are unavailable",
            Self::Database => "PostgreSQL refused the CSV transfer",
            Self::Csv => "CSV data is malformed or exceeds parser bounds",
            Self::FileIo => "CSV file operation failed",
            Self::DestinationExists => "Destination already exists; it was not replaced",
            Self::Timeout => "CSV operation timed out",
            Self::Cleanup => "CSV resource cleanup was not established",
            Self::Active => "CSV work or cleanup is still active",
            Self::DuplicateAttempt => "CSV attempt already exists",
            Self::Limit => "Import/export data or metadata exceeds native bounds",
        })
    }
}
impl std::error::Error for CsvError {}
macro_rules! redacted {
    ($name:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}
redacted!(CsvDiagnostic);
redacted!(CsvInspectionObservation);
redacted!(CsvTransferObservation);
redacted!(CsvTransferList);
redacted!(CsvInspectionList);
pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}
pub(super) fn path(value: &std::path::Path) -> Result<(), CsvError> {
    let value = value.to_str().ok_or(CsvError::InvalidRequest)?;
    if value.is_empty()
        || value.len() > MAX_CSV_PATH_BYTES
        || value.contains('\0')
        || !std::path::Path::new(value).is_absolute()
        || std::path::Path::new(value)
            .file_name()
            .is_none_or(|name| name.len() > 1024)
    {
        return Err(CsvError::InvalidRequest);
    }
    Ok(())
}
