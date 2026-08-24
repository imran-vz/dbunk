use serde::{Deserialize, Serialize};

use crate::postgres::sql_class::StatementClassSummary;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QueryTransactionMode {
    Autocommit,
    Manual,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QueryTransactionStatus {
    Idle,
    Active,
    Failed,
    Unknown,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum QueryTransactionIsolation {
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryTransactionSnapshot {
    pub mode: QueryTransactionMode,
    pub status: QueryTransactionStatus,
    pub manual_isolation: QueryTransactionIsolation,
}
impl Default for QueryTransactionSnapshot {
    fn default() -> Self {
        Self {
            mode: QueryTransactionMode::Autocommit,
            status: QueryTransactionStatus::Idle,
            manual_isolation: QueryTransactionIsolation::ReadCommitted,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum QuerySessionError {
    UnsupportedEngine,
    ConnectionClosing,
    SessionLimitReached {
        limit: String,
    },
    SessionNotFound,
    OwnerMismatch,
    ExecutionInProgress,
    InvalidSequence,
    InvalidTransactionTransition {
        status: QueryTransactionStatus,
        attempted_action: String,
        allowed_actions: Vec<String>,
    },
    TransactionStateUnknown {
        can_recheck: bool,
    },
    // Kept in the wire contract for observer failures that occur after admission.
    #[allow(dead_code)]
    TransactionObserverUnavailable,
    ConnectionLost,
    /// TLS material or handshake failure while opening the socket (ADR-0025).
    TlsFailed {
        tls_kind: crate::TlsFailureKind,
        message: String,
    },
    PolicyBlocked {
        reason: String,
    },
    PolicyNeedsConfirmation {
        statements: Vec<StatementClassSummary>,
    },
    Timeout {
        operation: String,
    },
    Database {
        code: Option<String>,
        message: String,
        severity: Option<String>,
        position: Option<u32>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterOwnerPayload {
    pub owner_id: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterOwnerResult {
    pub replaced_session_count: usize,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenSessionPayload {
    pub owner_id: String,
    pub session_id: String,
    pub tab_id: String,
    pub connection_id: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionPayload {
    pub session_id: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutePayload {
    pub session_id: String,
    pub execution_id: String,
    pub sql: String,
    #[serde(default)]
    pub confirmed: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionPayload {
    pub session_id: String,
    pub execution_id: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AckPayload {
    pub session_id: String,
    pub execution_id: String,
    pub ack_through_sequence: u64,
    pub retain_more_rows: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeartbeatPayload {
    pub owner_id: String,
    pub session_ids: Vec<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HeartbeatResult {
    pub refreshed_session_ids: Vec<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetModePayload {
    pub session_id: String,
    pub mode: QueryTransactionMode,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetIsolationPayload {
    pub session_id: String,
    pub manual_isolation: QueryTransactionIsolation,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedResult {
    pub accepted: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelResult {
    pub requested: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryEventEnvelope {
    pub session_id: String,
    pub tab_id: String,
    pub connection_id: String,
    pub generation: u64,
    pub sequence: u64,
    pub execution_id: Option<String>,
    pub requires_ack: bool,
    pub event: QueryEvent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum QueryEvent {
    SessionState {
        transaction: QueryTransactionSnapshot,
    },
    ExecutionStarted,
    ResultSetStarted {
        result_set_index: u32,
        columns: Vec<Option<String>>,
    },
    RowBatch {
        result_set_index: u32,
        rows: Vec<Vec<Option<String>>>,
    },
    ResultSetCompleted {
        result_set_index: u32,
        row_count: u64,
        partial: bool,
    },
    Notice {
        severity: String,
        message: String,
    },
    ExecutionCompleted {
        status: String,
        transaction: QueryTransactionSnapshot,
        omitted_rows: u64,
        omitted_result_sets: u32,
        omitted_notices: u32,
        omitted_metadata_bytes: u64,
        truncation_reasons: Vec<String>,
        error: Option<QueryDatabaseError>,
    },
    SessionLost {
        reason: String,
    },
    SessionClosed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryDatabaseError {
    pub code: Option<String>,
    pub message: String,
    pub severity: Option<String>,
    pub position: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_shapes_are_tagged_and_camel_case() {
        let error =
            serde_json::to_value(QuerySessionError::TransactionStateUnknown { can_recheck: true })
                .unwrap();
        assert_eq!(error["kind"], "transactionStateUnknown");
        assert_eq!(error["canRecheck"], true);
        assert_eq!(
            serde_json::to_value(QueryTransactionSnapshot::default()).unwrap()["manualIsolation"],
            "readCommitted"
        );
        let confirmation = serde_json::to_value(QuerySessionError::PolicyNeedsConfirmation {
            statements: vec![crate::postgres::sql_class::StatementClass::Dml {
                unbounded: true,
                destructive: false,
            }
            .summary(0)],
        })
        .unwrap();
        assert_eq!(confirmation["kind"], "policyNeedsConfirmation");
        assert_eq!(confirmation["statements"][0]["class"], "dml");
        assert_eq!(confirmation["statements"][0]["unbounded"], true);

        let payload: ExecutePayload = serde_json::from_value(serde_json::json!({
            "sessionId": "s",
            "executionId": "e",
            "sql": "SELECT 1"
        }))
        .unwrap();
        assert!(!payload.confirmed);
    }
    #[test]
    fn nullable_column_names_keep_positions() {
        let event = QueryEvent::ResultSetStarted {
            result_set_index: 0,
            columns: vec![Some("a".into()), None],
        };
        assert_eq!(
            serde_json::to_value(event).unwrap()["columns"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn completed_result_sets_report_whether_they_are_partial() {
        let event = QueryEvent::ResultSetCompleted {
            result_set_index: 2,
            row_count: 7,
            partial: true,
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["kind"], "resultSetCompleted");
        assert_eq!(value["partial"], true);
    }
}
