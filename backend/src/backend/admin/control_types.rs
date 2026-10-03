use super::super::data::DataDocument;
use serde::Serialize;
use std::{fmt, mem::size_of, sync::Arc};

pub const MAX_ADMIN_CONTROL_BYTES: usize = 8 * 1024;
/// Matching observed identity narrows stale-target exposure. PostgreSQL cannot
/// atomically lock a backend's lifetime/current query until signal delivery.
pub const ADMIN_CONTROL_LIMIT: &str = "Identity is checked before signaling, but PostgreSQL cannot lock backend lifetime or query changes through signal delivery. Success means signal sent, not proof the query or session stopped.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminSection {
    Sessions,
    Locks,
    Pending,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdminRow {
    pub section: AdminSection,
    pub index: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum AdminControlAction {
    CancelQuery,
    TerminateSession,
}
impl AdminControlAction {
    pub(super) fn intent(self) -> crate::safety::policy::WriteIntent {
        match self {
            Self::CancelQuery => crate::safety::policy::WriteIntent::CancelBackend,
            Self::TerminateSession => crate::safety::policy::WriteIntent::TerminateBackend,
        }
    }
    pub(super) fn command(self) -> &'static str {
        match self {
            Self::CancelQuery => "cancel_pg_backend",
            Self::TerminateSession => "terminate_pg_backend",
        }
    }
}

/// Display/receipt data only. There is deliberately no deserializer or public
/// constructor; executable authority additionally requires a consumed review.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct AdminControlTarget {
    pub(super) pid: i32,
    pub(super) backend_start: String,
    pub(super) query_start: Option<String>,
    pub(super) database: Option<String>,
}
impl AdminControlTarget {
    #[cfg(test)]
    pub(crate) fn test_target() -> Self {
        Self {
            pid: 12345,
            backend_start: "2026-10-03T01:02:03.123456Z".into(),
            query_start: Some("2026-10-03T01:03:03.123456Z".into()),
            database: Some("owned-test".into()),
        }
    }
    pub fn pid(&self) -> i32 {
        self.pid
    }
    pub fn backend_start(&self) -> &str {
        &self.backend_start
    }
    pub fn query_start(&self) -> Option<&str> {
        self.query_start.as_deref()
    }
    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.backend_start.capacity()
            + self.query_start.as_ref().map_or(0, String::capacity)
            + self.database.as_ref().map_or(0, String::capacity)
    }
}
impl fmt::Debug for AdminControlTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdminControlTarget")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

pub struct AdminControlReview {
    pub(super) document: DataDocument,
    pub(super) attempt_id: String,
    pub(super) action: AdminControlAction,
    pub(super) target: AdminControlTarget,
}
impl AdminControlReview {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn action(&self) -> AdminControlAction {
        self.action
    }
    pub fn target(&self) -> &AdminControlTarget {
        &self.target
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.attempt_id.capacity()
            + self.target.retained_bytes()
            + document_bytes(&self.document)
    }
}
pub struct AdminControlConfirmation {
    pub(super) review: AdminControlReview,
}
impl AdminControlConfirmation {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn attempt_id(&self) -> &str {
        self.review.attempt_id()
    }
    pub fn action(&self) -> AdminControlAction {
        self.review.action()
    }
    pub fn target(&self) -> &AdminControlTarget {
        self.review.target()
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum AdminControlSubmission {
    NeedsConfirmation(Box<AdminControlConfirmation>),
    Finished(AdminControlReceipt),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdminControlReceipt {
    pub attempt_id: String,
    pub action: AdminControlAction,
    pub target: AdminControlTarget,
    pub outcome: AdminControlOutcome,
}
impl AdminControlReceipt {
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.attempt_id.capacity()
            + self.target.retained_bytes()
            + match &self.outcome {
                AdminControlOutcome::NotDispatched { reason }
                | AdminControlOutcome::OutcomeUnknown { reason } => reason.retained_bytes(),
                _ => 0,
            }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum AdminControlOutcome {
    NotDispatched { reason: AdminControlFailure },
    TargetChanged,
    SignalNotSent,
    SignalSent,
    OutcomeUnknown { reason: AdminControlFailure },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum AdminControlFailure {
    Cancelled,
    Timeout,
    Connection,
    Database { code: Option<String> },
}
impl AdminControlFailure {
    fn retained_bytes(&self) -> usize {
        match self {
            Self::Database { code } => code.as_ref().map_or(0, String::capacity),
            _ => 0,
        }
    }
}
impl fmt::Display for AdminControlFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Control interrupted"),
            Self::Timeout => f.write_str("Control exceeded its deadline"),
            Self::Connection => f.write_str("Control connection failed"),
            Self::Database { code } => write!(
                f,
                "Control database error{}",
                code.as_ref().map(|c| format!(" [{c}]")).unwrap_or_default()
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminControlError {
    Cancelled,
    ForeignDocument,
    OutcomeUnavailable,
    Unavailable,
    InvalidTarget,
    Storage,
    Busy,
    PolicyBlocked,
}
impl fmt::Display for AdminControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Administration control cancelled before dispatch",
            Self::ForeignDocument => "Administration review belongs to another document",
            Self::OutcomeUnavailable => {
                "Administration outcome unavailable; do not retry this attempt"
            }
            Self::Unavailable => "Administration capture or document is no longer available",
            Self::InvalidTarget => "This captured row has no complete observed backend identity",
            Self::Storage => "Stored connection metadata is unavailable",
            Self::Busy => "An administration control is still settling in this document",
            Self::PolicyBlocked => "This read-only connection does not allow session termination",
        })
    }
}
impl std::error::Error for AdminControlError {}

pub(super) fn document_bytes(document: &DataDocument) -> usize {
    std::mem::size_of_val(&*document.0)
        + document.0.window.capacity()
        + document.0.tab.capacity()
        + document.0.connection.capacity()
        + document.0.manager_tab.capacity()
}
