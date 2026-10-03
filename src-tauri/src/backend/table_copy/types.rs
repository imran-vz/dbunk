//! Recovery values describe reviewed intent; they never grant execution authority.
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of};
pub const MAX_TABLE_COPY_ACTIVE: usize = 4;
pub const MAX_TABLE_COPY_TERMINAL: usize = 32;
pub const MAX_TABLE_COPY_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_TABLE_COPY_REVIEW_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TABLE_COPY_LIST_BYTES: usize = 1024 * 1024;
pub const MAX_TABLE_COPY_EXECUTION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TABLE_COPY_EXECUTION_POOL_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_TABLE_COPY_COLUMNS: usize = 1600;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableCopyAttemptId(pub(super) uuid::Uuid);
impl TableCopyAttemptId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    pub fn parse(text: &str) -> Result<Self, TableCopyError> {
        let id = uuid::Uuid::parse_str(text).map_err(|_| TableCopyError::InvalidRequest)?;
        if id.get_version_num() != 4 || id.to_string() != text {
            return Err(TableCopyError::InvalidRequest);
        }
        Ok(Self(id))
    }
}
impl Default for TableCopyAttemptId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for TableCopyAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl fmt::Debug for TableCopyAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Serialize for TableCopyAttemptId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for TableCopyAttemptId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableCopyEndpoint {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}
impl TableCopyEndpoint {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut n = size_of::<Self>();
        for (s, max) in [
            (&self.connection_id, 128),
            (&self.schema, 63),
            (&self.table, 63),
        ] {
            if s.is_empty() || s.len() > max || s.capacity() > max * 2 || s.contains('\0') {
                return None;
            }
            n = n.checked_add(s.capacity())?;
        }
        Some(n)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableCopyIntent {
    pub source: TableCopyEndpoint,
    pub destination: TableCopyEndpoint,
}
impl TableCopyIntent {
    pub fn new(
        source: TableCopyEndpoint,
        destination: TableCopyEndpoint,
    ) -> Result<Self, TableCopyError> {
        let value = Self {
            source,
            destination,
        };
        value
            .checked_heap_bytes()
            .ok_or(TableCopyError::InvalidRequest)?;
        Ok(value)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.source
            .checked_heap_bytes()?
            .checked_add(self.destination.checked_heap_bytes()?)?
            .checked_add(size_of::<Self>())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableCopyConnection {
    pub connection_name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub environment: String,
    pub safe_mode: String,
    pub read_only: bool,
}
impl TableCopyConnection {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.port == 0
            || !matches!(
                self.environment.as_str(),
                "Development" | "Test" | "Staging" | "Production"
            )
            || !matches!(
                self.safe_mode.as_str(),
                "Inherit" | "Disabled" | "Protected" | "Strict"
            )
        {
            return None;
        }
        let mut n = size_of::<Self>();
        for s in [
            &self.connection_name,
            &self.host,
            &self.database,
            &self.user,
            &self.environment,
            &self.safe_mode,
        ] {
            if s.len() > 256 || s.capacity() > 512 || s.contains('\0') {
                return None;
            }
            n = n.checked_add(s.capacity())?;
        }
        Some(n)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableCopyRelation {
    pub database_oid: u32,
    pub relation_oid: u32,
    pub kind: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "DescriptionFields")]
pub struct TableCopyDescription {
    pub intent: TableCopyIntent,
    pub source_connection: TableCopyConnection,
    pub destination_connection: TableCopyConnection,
    pub source_relation: TableCopyRelation,
    pub destination_relation: TableCopyRelation,
    pub mapping_sha256: String,
    pub copied_columns: u16,
    pub defaulted_columns: u16,
    pub generated_columns: u16,
    pub identity_columns: u16,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DescriptionFields {
    intent: TableCopyIntent,
    source_connection: TableCopyConnection,
    destination_connection: TableCopyConnection,
    source_relation: TableCopyRelation,
    destination_relation: TableCopyRelation,
    mapping_sha256: String,
    copied_columns: u16,
    defaulted_columns: u16,
    generated_columns: u16,
    identity_columns: u16,
}
impl From<TableCopyDescription> for DescriptionFields {
    fn from(d: TableCopyDescription) -> Self {
        Self {
            intent: d.intent,
            source_connection: d.source_connection,
            destination_connection: d.destination_connection,
            source_relation: d.source_relation,
            destination_relation: d.destination_relation,
            mapping_sha256: d.mapping_sha256,
            copied_columns: d.copied_columns,
            defaulted_columns: d.defaulted_columns,
            generated_columns: d.generated_columns,
            identity_columns: d.identity_columns,
        }
    }
}
impl TryFrom<DescriptionFields> for TableCopyDescription {
    type Error = TableCopyError;
    fn try_from(d: DescriptionFields) -> Result<Self, Self::Error> {
        let value = Self {
            intent: d.intent,
            source_connection: d.source_connection,
            destination_connection: d.destination_connection,
            source_relation: d.source_relation,
            destination_relation: d.destination_relation,
            mapping_sha256: d.mapping_sha256,
            copied_columns: d.copied_columns,
            defaulted_columns: d.defaulted_columns,
            generated_columns: d.generated_columns,
            identity_columns: d.identity_columns,
        };
        value.validate()?;
        Ok(value)
    }
}
impl TableCopyDescription {
    pub fn validate(&self) -> Result<(), TableCopyError> {
        self.checked_heap_bytes()
            .map(|_| ())
            .ok_or(TableCopyError::InvalidRequest)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.mapping_sha256.len() != 64
            || !self.mapping_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.mapping_sha256.capacity() > 128
            || self.identity_columns > self.copied_columns
            || usize::from(self.copied_columns)
                + usize::from(self.defaulted_columns)
                + usize::from(self.generated_columns)
                > MAX_TABLE_COPY_COLUMNS
            || self.copied_columns == 0
        {
            return None;
        }
        let mut n = size_of::<Self>()
            .checked_add(self.intent.checked_heap_bytes()?)?
            .checked_add(self.source_connection.checked_heap_bytes()?)?
            .checked_add(self.destination_connection.checked_heap_bytes()?)?
            .checked_add(self.mapping_sha256.capacity())?;
        for relation in [&self.source_relation, &self.destination_relation] {
            if relation.database_oid == 0
                || relation.relation_oid == 0
                || !matches!(relation.kind.as_str(), "r" | "p")
                || relation.kind.capacity() > 8
            {
                return None;
            }
            n = n.checked_add(relation.kind.capacity())?;
        }
        (n <= MAX_TABLE_COPY_DESCRIPTION_BYTES
            && encoded_size(self, MAX_TABLE_COPY_DESCRIPTION_BYTES).is_some())
        .then_some(n)
    }
}
impl fmt::Debug for TableCopyDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableCopyDescription")
            .field("copied_columns", &self.copied_columns)
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for TableCopyIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TableCopyIntent { .. }")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableCopyColumnAction {
    Copy,
    CopyIdentity,
    DefaultOrNull,
    Generated,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TableCopyColumn {
    pub name: String,
    pub source_type: Option<String>,
    pub destination_type: String,
    pub action: TableCopyColumnAction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableCopyPhase {
    Preparing,
    ReadyReview,
    AwaitingConfirmation,
    Running,
    Committing,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}
impl TableCopyPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableCopyOutcome {
    NotStarted,
    Pending,
    Completed { rows: u64 },
    RolledBack,
    OutcomeUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableCopyCleanup {
    Pending,
    Complete,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableCopyError {
    InvalidRequest,
    Limit,
    Busy,
    Closing,
    Missing,
    DuplicateAttempt,
    StaleReview,
    PolicyBlocked,
    Credentials,
    UnsupportedTarget,
    MissingRequiredColumn,
    TargetChanged,
    Cancelled,
    Timeout,
    Database,
    Cleanup,
}
impl fmt::Display for TableCopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "Invalid table copy request",
            Self::Limit => "Table copy exceeds its bounded capacity",
            Self::Busy => "An endpoint or table copy workspace is busy",
            Self::Closing => "Table copy owner is closing",
            Self::Missing => "Table copy attempt is unavailable",
            Self::DuplicateAttempt => "This table copy attempt was already registered",
            Self::StaleReview => "Table copy review is stale; review again",
            Self::PolicyBlocked => "Destination policy prohibits table copy",
            Self::Credentials => "Table copy credentials are unavailable",
            Self::UnsupportedTarget => {
                "Table copy requires ordinary or partitioned tables with a common insertable column"
            }
            Self::MissingRequiredColumn => "A required destination column has no matching source",
            Self::TargetChanged => "Reviewed table metadata changed",
            Self::Cancelled => "Table copy was cancelled",
            Self::Timeout => "Table copy exceeded its operation deadline",
            Self::Database => "PostgreSQL refused or disconnected table copy",
            Self::Cleanup => "Table copy cleanup could not be established",
        })
    }
}
impl std::error::Error for TableCopyError {}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCopyDiagnostic {
    pub side: TableCopySide,
    pub sqlstate: Option<String>,
    pub field_limit: bool,
    pub record_limit: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableCopySide {
    Source,
    Destination,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCopyReceipt {
    pub attempt_id: TableCopyAttemptId,
    pub description: TableCopyDescription,
    pub outcome: TableCopyOutcome,
    pub failure: Option<TableCopyError>,
    pub diagnostic: Option<TableCopyDiagnostic>,
}
impl TableCopyReceipt {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if matches!(self.outcome, TableCopyOutcome::Pending)
            || matches!(self.outcome, TableCopyOutcome::Completed { .. })
                && (self.failure.is_some() || self.diagnostic.is_some())
            || matches!(
                self.outcome,
                TableCopyOutcome::OutcomeUnknown | TableCopyOutcome::RolledBack
            ) && self.failure.is_none()
        {
            return None;
        }
        diagnostic_bytes(self.diagnostic.as_ref())?
            .checked_add(self.description.checked_heap_bytes()?)?
            .checked_add(size_of::<Self>())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TableCopyObservation {
    pub attempt_id: TableCopyAttemptId,
    pub intent: TableCopyIntent,
    pub phase: TableCopyPhase,
    pub outcome: TableCopyOutcome,
    pub cleanup: TableCopyCleanup,
    pub bytes_processed: u64,
    pub failure: Option<TableCopyError>,
    pub diagnostic: Option<TableCopyDiagnostic>,
    pub receipt: Option<TableCopyReceipt>,
    pub change_revision: Option<u64>,
}
impl TableCopyObservation {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.change_revision == Some(0)
            || self.change_revision.is_some()
                != matches!(
                    self.outcome,
                    TableCopyOutcome::Completed { .. } | TableCopyOutcome::OutcomeUnknown
                )
            || matches!(self.outcome, TableCopyOutcome::Completed { .. })
                != (self.phase == TableCopyPhase::Completed)
            || matches!(
                self.outcome,
                TableCopyOutcome::OutcomeUnknown | TableCopyOutcome::RolledBack
            ) && !self.phase.terminal()
            || !self.phase.terminal()
                && (self.receipt.is_some() || self.failure.is_some() || self.diagnostic.is_some())
            || self.receipt.as_ref().is_some_and(|r| {
                r.attempt_id != self.attempt_id
                    || r.description.intent != self.intent
                    || r.outcome != self.outcome
                    || r.failure != self.failure
                    || r.diagnostic != self.diagnostic
            })
        {
            return None;
        }
        size_of::<Self>()
            .checked_add(self.intent.checked_heap_bytes()?)?
            .checked_add(diagnostic_bytes(self.diagnostic.as_ref())?)?
            .checked_add(match &self.receipt {
                Some(r) => r.checked_heap_bytes()?,
                None => 0,
            })
    }
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TableCopyList {
    pub jobs: Vec<TableCopyObservation>,
    pub change_revision: u64,
}
impl TableCopyList {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.jobs.len() > MAX_TABLE_COPY_ACTIVE + MAX_TABLE_COPY_TERMINAL
            || self.jobs.capacity() > MAX_TABLE_COPY_ACTIVE + MAX_TABLE_COPY_TERMINAL
        {
            return None;
        }
        let mut n = size_of::<Self>().checked_add(
            self.jobs
                .capacity()
                .checked_mul(size_of::<TableCopyObservation>())?,
        )?;
        for (i, j) in self.jobs.iter().enumerate() {
            if self.jobs[..i].iter().any(|p| {
                p.attempt_id == j.attempt_id
                    || p.change_revision.is_some() && p.change_revision == j.change_revision
            }) || j.change_revision.is_some_and(|r| r > self.change_revision)
            {
                return None;
            }
            n = n.checked_add(j.checked_heap_bytes()?)?;
        }
        (n <= MAX_TABLE_COPY_LIST_BYTES && encoded_size(self, MAX_TABLE_COPY_LIST_BYTES).is_some())
            .then_some(n)
    }
}
fn diagnostic_bytes(value: Option<&TableCopyDiagnostic>) -> Option<usize> {
    match value {
        None => Some(0),
        Some(d) => {
            if d.sqlstate.as_ref().is_some_and(|s| {
                s.len() != 5 || s.capacity() > 8 || !s.bytes().all(|b| b.is_ascii_alphanumeric())
            }) {
                return None;
            }
            Some(size_of::<TableCopyDiagnostic>() + d.sqlstate.as_ref().map_or(0, String::capacity))
        }
    }
}

/// Bounded serialized count without materializing escaped JSON.
fn encoded_size(value: &impl Serialize, limit: usize) -> Option<usize> {
    struct Count {
        used: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.used = self
                .used
                .checked_add(bytes.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("table copy encoding bound"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { used: 0, limit };
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.used)
}

impl fmt::Debug for TableCopyObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableCopyObservation")
            .field("attempt_id", &self.attempt_id)
            .field("phase", &self.phase)
            .field("outcome", &self.outcome)
            .field("cleanup", &self.cleanup)
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for TableCopyList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableCopyList")
            .field("jobs", &self.jobs.len())
            .field("change_revision", &self.change_revision)
            .finish()
    }
}
impl fmt::Debug for TableCopyReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableCopyReceipt")
            .field("attempt_id", &self.attempt_id)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}
