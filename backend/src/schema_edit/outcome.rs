//! What an apply did. Shared by every engine executor; the facade wraps it
//! in a receipt.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchemaEditFailure {
    Cancelled,
    Timeout,
    /// PostgreSQL `lock_timeout`, MySQL `lock_wait_timeout`.
    LockTimeout,
    Connection,
    /// The table changed since capture; nothing was dispatched.
    BaselineChanged,
    /// The applied structure does not match the draft (rolled back where the
    /// engine is transactional).
    VerificationFailed {
        detail: String,
    },
    /// SQLite: the session has an open transaction. Commit or roll back first.
    OpenTransaction,
    /// SQLite rebuild: `PRAGMA foreign_key_check` found violations.
    ForeignKeyViolation,
    /// The policy refused at dispatch (read-only connection).
    PolicyBlocked {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
    /// The server rejected a statement. `message` is the server text.
    Database {
        code: Option<String>,
        message: String,
    },
}

impl std::fmt::Display for SchemaEditFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Timeout => f.write_str("Timed out"),
            Self::LockTimeout => f.write_str("Timed out waiting for a lock on the table"),
            Self::Connection => f.write_str("The connection failed"),
            Self::BaselineChanged => {
                f.write_str("The table changed since it was captured; reload and review again")
            }
            Self::VerificationFailed { detail } => {
                write!(
                    f,
                    "The applied structure does not match the draft: {detail}"
                )
            }
            Self::OpenTransaction => {
                f.write_str("The session has an open transaction; commit or roll back first")
            }
            Self::ForeignKeyViolation => {
                f.write_str("Existing rows violate a foreign key of the new table")
            }
            Self::PolicyBlocked { reason } | Self::Unsupported { reason } => f.write_str(reason),
            Self::Database {
                code: Some(code),
                message,
            } => write!(f, "{message} ({code})"),
            Self::Database {
                code: None,
                message,
            } => f.write_str(message),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SchemaEditOutcome {
    /// Nothing reached the server.
    NotDispatched { reason: SchemaEditFailure },
    /// The engine acknowledged a rollback; the table is unchanged.
    RolledBack { reason: SchemaEditFailure },
    /// Every statement ran and the re-captured structure matches the draft.
    Applied { runtime_ms: u64 },
    /// Non-transactional engines: statements `0..committed` committed, then
    /// `failed_statement` failed. The table is partly changed.
    Stopped {
        committed: usize,
        failed_statement: usize,
        reason: SchemaEditFailure,
    },
    /// The result could not be established: a lost connection after
    /// dispatch, or committed statements whose re-capture failed or did not
    /// match the draft. Reconcile before another attempt; never replay.
    OutcomeUnknown { reason: SchemaEditFailure },
}

impl SchemaEditOutcome {
    pub fn applied(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }
    /// True when the table may have changed.
    pub fn may_have_changed(&self) -> bool {
        matches!(
            self,
            Self::Applied { .. } | Self::Stopped { .. } | Self::OutcomeUnknown { .. }
        )
    }
}

/// What a session worker (SQLite, MySQL) answers to an apply request. The
/// policy is checked on the worker against the session's own resolved
/// connection, so a refusal never dispatches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionApply {
    NeedsConfirmation,
    Blocked(String),
    Finished(SchemaEditOutcome),
}
