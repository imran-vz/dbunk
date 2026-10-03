use serde::Serialize;
use std::{
    fmt,
    mem::size_of,
    path::{Path, PathBuf},
};

pub const MAX_PG_TOOL_ACTIVE: usize = 4;
pub const MAX_PG_TOOL_TERMINAL: usize = 32;
pub const MAX_PG_TOOL_JOBS: usize = MAX_PG_TOOL_ACTIVE + MAX_PG_TOOL_TERMINAL;
pub const MAX_PG_TOOL_LIST_BYTES: usize = 256 * 1024;
pub const MAX_PG_TOOL_PATH_BYTES: usize = 16 * 1024;
pub const MAX_PG_TOOL_REVIEW_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PgToolAttemptId(uuid::Uuid);
impl Serialize for PgToolAttemptId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl PgToolAttemptId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    pub fn parse(value: &str) -> Result<Self, PgToolError> {
        let id = uuid::Uuid::parse_str(value).map_err(|_| PgToolError::InvalidRequest)?;
        if id.get_version_num() != 4 || id.to_string() != value {
            return Err(PgToolError::InvalidRequest);
        }
        Ok(Self(id))
    }
}
impl Default for PgToolAttemptId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for PgToolAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl fmt::Debug for PgToolAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PgToolKind {
    Backup,
    Restore,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PgToolFormat {
    Plain,
    Custom,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PgToolScope {
    Database,
    Schema { schema: String },
    Table { schema: String, table: String },
}
impl PgToolScope {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let identifiers: &[&String] = match self {
            Self::Database => &[],
            Self::Schema { schema } => &[schema],
            Self::Table { schema, table } => &[schema, table],
        };
        let mut bytes = size_of::<Self>();
        for value in identifiers {
            if value.is_empty() || value.len() > 63 || value.contains('\0') || value.capacity() > 63
            {
                return None;
            }
            bytes = bytes.checked_add(value.capacity())?;
        }
        Some(bytes)
    }
}

/// An ephemeral file selection. Never serialized or included in diagnostics.
pub struct PgToolIntent {
    pub(super) path: PathBuf,
    pub(super) kind: PgToolKind,
    pub(super) format: PgToolFormat,
    pub(super) scope: PgToolScope,
    pub(super) clean: bool,
}
impl fmt::Debug for PgToolIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PgToolIntent(<redacted file selection>)")
    }
}
impl PgToolIntent {
    pub fn backup(
        destination: PathBuf,
        format: PgToolFormat,
        scope: PgToolScope,
        clean: bool,
    ) -> Result<Self, PgToolError> {
        Self::new(destination, PgToolKind::Backup, format, scope, clean)
    }
    pub fn restore(
        source: PathBuf,
        format: PgToolFormat,
        clean: bool,
    ) -> Result<Self, PgToolError> {
        Self::new(
            source,
            PgToolKind::Restore,
            format,
            PgToolScope::Database,
            clean,
        )
    }
    fn new(
        path: PathBuf,
        kind: PgToolKind,
        format: PgToolFormat,
        scope: PgToolScope,
        clean: bool,
    ) -> Result<Self, PgToolError> {
        let text = path.to_str().ok_or(PgToolError::InvalidPath)?;
        if !path.is_absolute()
            || path.file_name().is_none()
            || text.len() > MAX_PG_TOOL_PATH_BYTES
            || text.contains('\0')
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| name.len() > 1024)
            || scope.checked_heap_bytes().is_none()
        {
            return Err(PgToolError::InvalidRequest);
        }
        if clean
            && matches!(
                (kind, format),
                (PgToolKind::Backup, PgToolFormat::Custom)
                    | (PgToolKind::Restore, PgToolFormat::Plain)
            )
        {
            return Err(PgToolError::InvalidRequest);
        }
        let path = PathBuf::from(text);
        Ok(Self {
            path,
            kind,
            format,
            scope,
            clean,
        })
    }
    pub fn kind(&self) -> PgToolKind {
        self.kind
    }
    pub fn format(&self) -> PgToolFormat {
        self.format
    }
    pub fn scope(&self) -> &PgToolScope {
        &self.scope
    }
    pub fn clean(&self) -> bool {
        self.clean
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let bytes = size_of::<Self>()
            .checked_add(self.path.capacity())?
            .checked_add(self.scope.checked_heap_bytes()?)?;
        (self.path.capacity() <= MAX_PG_TOOL_PATH_BYTES && bytes <= MAX_PG_TOOL_PATH_BYTES + 1024)
            .then_some(bytes)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PgToolPhase {
    Preparing,
    ReadyReview,
    AwaitingConfirmation,
    Queued,
    Preflight,
    Running,
    Finalizing,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}
impl PgToolPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PgToolEffect {
    NotStarted,
    Pending,
    Succeeded,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PgToolCleanup {
    Pending,
    Complete,
    Failed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PgToolError {
    InvalidRequest,
    InvalidPath,
    Busy,
    Missing,
    ForeignReview,
    StaleReview,
    Closing,
    Cancelled,
    PolicyBlocked,
    SourceChanged,
    FileIo,
    DestinationExists,
    Credentials,
    ToolUnavailable,
    ToolFailed,
    Timeout,
    Cleanup,
    Active,
    DuplicateAttempt,
}
impl fmt::Display for PgToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
Self::InvalidRequest=>"The file job request exceeds its supported bounds",Self::InvalidPath=>"Select an absolute local Unicode file path",Self::Busy=>"Four jobs are admitted or this connection already has a job",Self::Missing=>"The file job is missing or expired",Self::ForeignReview=>"Review belongs to another native profile",Self::StaleReview=>"Connection or review changed; prepare a new job",Self::Closing=>"The native profile or connection is closing",Self::Cancelled=>"Cancellation requested; this cannot undo a committed restore",Self::PolicyBlocked=>"Current connection policy blocks restore",Self::SourceChanged=>"Restore source changed during preparation; select it again",Self::FileIo=>"The selected file could not be read, copied or written; check space and permissions",Self::DestinationExists=>"Backup destination exists; choose a new file",Self::Credentials=>"Connection credentials could not be resolved",Self::ToolUnavailable=>"A supported patched PostgreSQL client is unavailable",Self::ToolFailed=>"The PostgreSQL client failed; database outcome may require inspection",Self::Timeout=>"The client or cleanup deadline expired",Self::Cleanup=>"Cleanup has not been proven complete",Self::Active=>"Active work or pending cleanup cannot be released",Self::DuplicateAttempt=>"This attempt was already registered; observe it instead of retrying" })
    }
}
impl std::error::Error for PgToolError {}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgToolObservation {
    pub attempt_id: PgToolAttemptId,
    pub connection_id: String,
    pub kind: PgToolKind,
    pub format: PgToolFormat,
    pub scope: PgToolScope,
    pub clean: bool,
    pub file_name: String,
    pub phase: PgToolPhase,
    pub effect: PgToolEffect,
    pub cleanup: PgToolCleanup,
    pub source_bytes: Option<u64>,
    pub bytes_processed: Option<u64>,
    pub tool_version: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub failure: Option<PgToolError>,
    pub diagnostic: Option<PgToolDiagnostic>,
    pub restore_change_revision: Option<u64>,
}
impl fmt::Debug for PgToolObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgToolObservation")
            .field("attempt_id", &self.attempt_id)
            .field("phase", &self.phase)
            .field("effect", &self.effect)
            .field("cleanup", &self.cleanup)
            .finish_non_exhaustive()
    }
}
impl PgToolObservation {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut bytes = size_of::<Self>().checked_add(self.scope.checked_heap_bytes()?)?;
        for (value, cap) in [
            (&self.connection_id, 256),
            (&self.file_name, 1024),
            (&self.started_at, 64),
        ] {
            if value.is_empty()
                || value.len() > cap
                || value.capacity() > cap
                || value.contains('\0')
            {
                return None;
            }
            bytes = bytes.checked_add(value.capacity())?;
        }
        for (value, cap) in [(&self.tool_version, 1024), (&self.finished_at, 64)] {
            if let Some(value) = value {
                if value.len() > cap || value.capacity() > cap || value.contains('\0') {
                    return None;
                }
                bytes = bytes.checked_add(value.capacity())?;
            }
        }
        if chrono::DateTime::parse_from_rfc3339(&self.started_at).is_err()
            || self
                .finished_at
                .as_ref()
                .is_some_and(|time| chrono::DateTime::parse_from_rfc3339(time).is_err())
        {
            return None;
        }
        if self.phase.terminal() != self.finished_at.is_some()
            || (self.phase == PgToolPhase::Completed && self.effect != PgToolEffect::Succeeded)
        {
            return None;
        }
        if let Some(diagnostic) = &self.diagnostic {
            bytes = bytes.checked_add(diagnostic.checked_heap_bytes()?)?;
        }
        let requires_revision = self.kind == PgToolKind::Restore
            && self.phase.terminal()
            && matches!(self.effect, PgToolEffect::Succeeded | PgToolEffect::Unknown);
        if requires_revision != self.restore_change_revision.is_some()
            || self.restore_change_revision == Some(0)
        {
            return None;
        }
        Some(bytes)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgToolJobList {
    pub jobs: Vec<PgToolObservation>,
    pub restore_change_revision: u64,
}
impl fmt::Debug for PgToolJobList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgToolJobList")
            .field("count", &self.jobs.len())
            .finish()
    }
}
impl PgToolJobList {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.jobs.len() > MAX_PG_TOOL_JOBS || self.jobs.capacity() > MAX_PG_TOOL_JOBS {
            return None;
        }
        let mut bytes = size_of::<Self>().checked_add(
            self.jobs
                .capacity()
                .checked_mul(size_of::<PgToolObservation>())?,
        )?;
        for (i, job) in self.jobs.iter().enumerate() {
            bytes = bytes.checked_add(job.checked_heap_bytes()?)?;
            if let Some(revision) = job.restore_change_revision {
                if revision > self.restore_change_revision
                    || self.jobs[..i]
                        .iter()
                        .any(|other| other.restore_change_revision == Some(revision))
                {
                    return None;
                }
            }
            if self.jobs[..i]
                .iter()
                .any(|other| other.attempt_id == job.attempt_id)
            {
                return None;
            }
        }
        self.encoded_bytes()?;
        (bytes <= MAX_PG_TOOL_LIST_BYTES).then_some(bytes)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded_bytes(self, MAX_PG_TOOL_LIST_BYTES)
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgToolTarget {
    pub connection_name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub environment: String,
    pub safe_mode: String,
    pub read_only: bool,
}
impl PgToolTarget {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut bytes = size_of::<Self>();
        for text in [
            &self.connection_name,
            &self.host,
            &self.database,
            &self.user,
            &self.environment,
            &self.safe_mode,
        ] {
            if text.len() > 256 || text.capacity() > 256 || text.contains('\0') {
                return None;
            }
            bytes = bytes.checked_add(text.capacity())?;
        }
        Some(bytes)
    }
}

pub(super) fn encoded_bytes(value: &impl Serialize, limit: usize) -> Option<usize> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(input.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("tool snapshot bound"))?;
            Ok(input.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.bytes)
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PgToolDiagnostic {
    pub tool: Option<String>,
    pub exit_code: Option<i32>,
    pub operation: Option<String>,
    pub message: String,
}
impl fmt::Debug for PgToolDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PgToolDiagnostic(<classified>)")
    }
}
impl PgToolDiagnostic {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut bytes = size_of::<Self>();
        for (value, cap) in [(&self.tool, 32), (&self.operation, 64)] {
            if let Some(value) = value {
                if value.len() > cap || value.capacity() > cap {
                    return None;
                }
                bytes = bytes.checked_add(value.capacity())?;
            }
        }
        if self.message.len() > 1024 || self.message.capacity() > 1024 {
            return None;
        }
        bytes.checked_add(self.message.capacity())
    }
}
