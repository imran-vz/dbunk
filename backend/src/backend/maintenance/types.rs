use super::super::data::DataDocument;
use serde::Serialize;
use std::{fmt, mem::size_of, sync::Arc};

pub const MAX_MAINTENANCE_REVIEW_BYTES: usize = 32 * 1024;
pub const MAX_MAINTENANCE_RECEIPT_BYTES: usize = 16 * 1024;
pub const MAINTENANCE_OPERATION_TIMEOUT_MS: u32 = 300_000;
pub const MAINTENANCE_IDENTITY_LIMIT: &str = "The observed object identity is rechecked before execution. PostgreSQL maintenance SQL resolves names again; concurrent schema or object changes can still retarget it. This is not an atomic OID-bound operation.";
pub const MAINTENANCE_EFFECT_LIMIT: &str = "Lock waits are capped at 10 seconds. Completion acknowledges the command, not a measured amount of work. Warnings may report skipped work. VACUUM, ANALYZE and partitioned REINDEX can leave effects after interruption. Transaction rollback covers transactional database changes, not sequence or external effects from user-defined functions.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MaintenanceIntent {
    Vacuum,
    Analyze,
    ReindexTable,
    RefreshMaterializedView { concurrently: bool },
}
impl MaintenanceIntent {
    pub(super) fn policy(self) -> crate::safety::policy::WriteIntent {
        match self {
            Self::RefreshMaterializedView { .. } => {
                crate::safety::policy::WriteIntent::RefreshMatView
            }
            _ => crate::safety::policy::WriteIntent::Maintenance,
        }
    }
    pub(super) fn command(self) -> &'static str {
        match self {
            Self::RefreshMaterializedView { .. } => "refresh_materialized_view",
            _ => "run_pg_maintenance",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MaintenanceRelationKind {
    Table,
    PartitionedTable,
    MaterializedView,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MaintenanceSemantics {
    Transactional,
    PotentiallyPartial,
}
/// Read-only identity for display/recovery. Constructing a description never
/// grants execution authority; only a backend observation can mint a review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaintenanceTarget {
    pub(crate) database_oid: u32,
    pub(crate) database: String,
    pub(crate) namespace_oid: u32,
    pub(crate) schema: String,
    pub(crate) relation_oid: u32,
    pub(crate) name: String,
    pub(crate) kind: MaintenanceRelationKind,
}
impl MaintenanceTarget {
    pub fn database_oid(&self) -> u32 {
        self.database_oid
    }
    pub fn database(&self) -> &str {
        &self.database
    }
    pub fn namespace_oid(&self) -> u32 {
        self.namespace_oid
    }
    pub fn schema(&self) -> &str {
        &self.schema
    }
    pub fn relation_oid(&self) -> u32 {
        self.relation_oid
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn kind(&self) -> MaintenanceRelationKind {
        self.kind
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.database.capacity() + self.schema.capacity() + self.name.capacity()
    }
    pub(crate) fn valid(&self) -> bool {
        self.database_oid != 0
            && self.namespace_oid != 0
            && self.relation_oid != 0
            && [&self.database, &self.schema, &self.name]
                .into_iter()
                .all(|s| valid_name(s))
    }
}
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 63 && !name.contains('\0')
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaintenancePreview {
    pub sql: String,
    pub semantics: MaintenanceSemantics,
    pub operation_timeout_ms: u32,
    pub statement_timeout_ms: Option<u32>,
}
impl MaintenancePreview {
    pub fn identity_limit(&self) -> &'static str {
        MAINTENANCE_IDENTITY_LIMIT
    }
    pub fn effect_limit(&self) -> &'static str {
        MAINTENANCE_EFFECT_LIMIT
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.sql.capacity()
    }
}
pub struct ObservedMaintenanceTarget {
    pub(super) document: DataDocument,
    pub(super) target: MaintenanceTarget,
    pub(super) statement_timeout_ms: Option<u32>,
}
impl ObservedMaintenanceTarget {
    pub fn target(&self) -> &MaintenanceTarget {
        &self.target
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.target.retained_bytes() + document_bytes(&self.document)
    }
    pub fn review(&self, intent: MaintenanceIntent) -> Result<MaintenanceReview, MaintenanceError> {
        self.document
            .0
            .check_open()
            .map_err(|_| MaintenanceError::Unavailable)?;
        let review = MaintenanceReview {
            document: self.document.clone(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
            intent,
            target: self.target.clone(),
            preview: super::preview(&self.target, intent, self.statement_timeout_ms)?,
        };
        if review.retained_bytes() > MAX_MAINTENANCE_REVIEW_BYTES {
            return Err(MaintenanceError::InvalidTarget);
        }
        Ok(review)
    }
}
pub struct MaintenanceReview {
    pub(super) document: DataDocument,
    pub(super) attempt_id: String,
    pub(super) intent: MaintenanceIntent,
    pub(super) target: MaintenanceTarget,
    pub(super) preview: MaintenancePreview,
}
impl MaintenanceReview {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn intent(&self) -> MaintenanceIntent {
        self.intent
    }
    pub fn target(&self) -> &MaintenanceTarget {
        &self.target
    }
    pub fn preview(&self) -> &MaintenancePreview {
        &self.preview
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.attempt_id.capacity()
            + self.target.retained_bytes()
            + self.preview.retained_bytes()
            + document_bytes(&self.document)
    }
}
pub struct MaintenanceConfirmation {
    pub(super) review: MaintenanceReview,
}
impl MaintenanceConfirmation {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn attempt_id(&self) -> &str {
        self.review.attempt_id()
    }
    pub fn intent(&self) -> MaintenanceIntent {
        self.review.intent()
    }
    pub fn target(&self) -> &MaintenanceTarget {
        self.review.target()
    }
    pub fn preview(&self) -> &MaintenancePreview {
        self.review.preview()
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum MaintenanceSubmission {
    NeedsConfirmation(Box<MaintenanceConfirmation>),
    Finished(Box<MaintenanceReceipt>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaintenanceReceipt {
    pub attempt_id: String,
    pub intent: MaintenanceIntent,
    pub target: MaintenanceTarget,
    pub preview: MaintenancePreview,
    pub outcome: MaintenanceOutcome,
    pub notices: Vec<MaintenanceNotice>,
    pub notices_truncated: bool,
    pub runtime_ms: u64,
}
impl MaintenanceReceipt {
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.attempt_id.capacity()
            + self.target.retained_bytes()
            + self.preview.retained_bytes()
            + self.outcome.heap_bytes()
            + self.notices.capacity() * size_of::<MaintenanceNotice>()
            + self
                .notices
                .iter()
                .map(|n| n.severity.capacity() + n.message.capacity())
                .sum::<usize>()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaintenanceNotice {
    pub severity: String,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum MaintenanceOutcome {
    NotDispatched {
        reason: MaintenanceFailure,
    },
    TargetChanged,
    Completed,
    /// Transactional target changes did not commit; user-defined external or
    /// sequence effects are not covered by this claim.
    RolledBack {
        reason: MaintenanceFailure,
    },
    InterruptedEffectsPossible {
        reason: MaintenanceFailure,
    },
    OutcomeUnknown {
        reason: MaintenanceFailure,
    },
}
impl MaintenanceOutcome {
    fn heap_bytes(&self) -> usize {
        let reason = match self {
            Self::NotDispatched { reason }
            | Self::RolledBack { reason }
            | Self::InterruptedEffectsPossible { reason }
            | Self::OutcomeUnknown { reason } => reason,
            _ => return 0,
        };
        match reason {
            MaintenanceFailure::Database { code } => code.as_ref().map_or(0, String::capacity),
            _ => 0,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum MaintenanceFailure {
    Cancelled,
    Timeout,
    Connection,
    Database { code: Option<String> },
}
impl fmt::Display for MaintenanceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Maintenance interrupted"),
            Self::Timeout => f.write_str("Maintenance deadline reached"),
            Self::Connection => f.write_str("Maintenance connection failed"),
            Self::Database { code } => write!(
                f,
                "PostgreSQL maintenance error{}",
                code.as_ref().map(|c| format!(" [{c}]")).unwrap_or_default()
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaintenanceError {
    ForeignDocument,
    Unavailable,
    InvalidTarget,
    Storage,
    Busy,
    PolicyBlocked,
    OutcomeUnavailable,
}
impl fmt::Display for MaintenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ForeignDocument => "Maintenance target belongs to another workspace",
            Self::Unavailable => "Maintenance target or document is unavailable",
            Self::InvalidTarget => {
                "Maintenance requires a bounded table or materialized-view identity"
            }
            Self::Storage => "Stored connection metadata is unavailable",
            Self::Busy => "A schema or maintenance operation is still settling on this connection",
            Self::PolicyBlocked => "Stored read-only policy blocks maintenance",
            Self::OutcomeUnavailable => {
                "Maintenance outcome unavailable; reconcile explicitly before another attempt"
            }
        })
    }
}
impl std::error::Error for MaintenanceError {}
fn document_bytes(document: &DataDocument) -> usize {
    std::mem::size_of_val(&*document.0)
        + document.0.window.capacity()
        + document.0.tab.capacity()
        + document.0.connection.capacity()
        + document.0.manager_tab.capacity()
}
