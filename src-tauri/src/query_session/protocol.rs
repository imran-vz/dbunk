use serde::{Deserialize, Serialize};

use crate::postgres::sql_class::StatementClassSummary;
use crate::postgres::sql_params::{ParameterRejectionReason, ParameterValue, PlanRefusal};

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
    /// Returned before the policy check; nothing was sent to the server.
    ParametersRejected {
        reason: ParameterRejectionReason,
        names: Vec<String>,
    },
    InvalidRowLimit,
}

impl From<PlanRefusal> for QuerySessionError {
    fn from(refusal: PlanRefusal) -> Self {
        match refusal {
            PlanRefusal::Parameters(rejection) => Self::ParametersRejected {
                reason: rejection.reason,
                names: rejection.names,
            },
            PlanRefusal::InvalidRowLimit => Self::InvalidRowLimit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RowLimitOutcome {
    /// The server stopped at the limit and more rows exist.
    Stopped,
    /// Every row was read and the limit withheld some.
    Drained,
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
    /// Present, even when empty, puts the execution in parameter mode.
    #[serde(default)]
    pub parameters: Option<Vec<ParameterValue>>,
    #[serde(default)]
    pub row_limit: Option<i64>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DescribeParametersPayload {
    pub sql: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DescribeParametersResult {
    pub names: Vec<String>,
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
        limit: Option<RowLimitOutcome>,
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
        refusal: Option<String>,
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
        assert!(payload.parameters.is_none());
        assert!(payload.row_limit.is_none());
    }

    #[test]
    fn execute_payload_distinguishes_parameter_mode_and_redacts_values() {
        let payload: ExecutePayload = serde_json::from_value(serde_json::json!({
            "sessionId": "s",
            "executionId": "e",
            "sql": "SELECT :a, :b",
            "parameters": [
                { "name": "a", "value": "hunter2-secret" },
                { "name": "b", "value": null }
            ],
            "rowLimit": 200
        }))
        .unwrap();
        let parameters = payload.parameters.as_deref().expect("parameter mode");
        assert_eq!(parameters[0].value.as_deref(), Some("hunter2-secret"));
        assert_eq!(parameters[1].value, None);
        assert_eq!(payload.row_limit, Some(200));
        let rendered = format!("{payload:?}");
        assert!(!rendered.contains("hunter2-secret"), "{rendered}");

        let empty: ExecutePayload = serde_json::from_value(serde_json::json!({
            "sessionId": "s",
            "executionId": "e",
            "sql": "SELECT 1",
            "parameters": []
        }))
        .unwrap();
        assert_eq!(empty.parameters.map(|parameters| parameters.len()), Some(0));
    }

    #[test]
    fn refusals_and_new_event_fields_have_stable_wire_shapes() {
        let refusal = serde_json::to_value(QuerySessionError::from(PlanRefusal::Parameters(
            crate::postgres::sql_params::ParameterRejection {
                reason: ParameterRejectionReason::MissingValue,
                names: vec!["a".into()],
            },
        )))
        .unwrap();
        assert_eq!(
            refusal,
            serde_json::json!({
                "kind": "parametersRejected",
                "reason": "missingValue",
                "names": ["a"]
            })
        );
        assert_eq!(
            serde_json::to_value(QuerySessionError::from(PlanRefusal::InvalidRowLimit)).unwrap(),
            serde_json::json!({ "kind": "invalidRowLimit" })
        );
        for (reason, wire) in [
            (ParameterRejectionReason::Unlexable, "unlexable"),
            (
                ParameterRejectionReason::MultipleStatements,
                "multipleStatements",
            ),
            (
                ParameterRejectionReason::PositionalPlaceholder,
                "positionalPlaceholder",
            ),
            (ParameterRejectionReason::DuplicateName, "duplicateName"),
            (ParameterRejectionReason::NameTooLong, "nameTooLong"),
            (
                ParameterRejectionReason::TooManyParameters,
                "tooManyParameters",
            ),
            (ParameterRejectionReason::ValueTooLarge, "valueTooLarge"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), wire);
        }

        let completed = |limit| {
            serde_json::to_value(QueryEvent::ResultSetCompleted {
                result_set_index: 0,
                row_count: 3,
                partial: false,
                limit,
            })
            .unwrap()
        };
        // The field is always present; `null` means the limit withheld nothing.
        assert_eq!(completed(None)["limit"], serde_json::Value::Null);
        assert_eq!(
            completed(Some(RowLimitOutcome::Stopped))["limit"],
            "stopped"
        );
        assert_eq!(
            completed(Some(RowLimitOutcome::Drained))["limit"],
            "drained"
        );

        let terminal = serde_json::to_value(QueryEvent::ExecutionCompleted {
            status: "failed".into(),
            transaction: QueryTransactionSnapshot::default(),
            omitted_rows: 0,
            omitted_result_sets: 0,
            omitted_notices: 0,
            omitted_metadata_bytes: 0,
            truncation_reasons: Vec::new(),
            error: None,
            refusal: Some("parametersReturnRows".into()),
        })
        .unwrap();
        assert_eq!(terminal["refusal"], "parametersReturnRows");
        assert_eq!(terminal["error"], serde_json::Value::Null);
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
            limit: None,
        };
        let value = serde_json::to_value(event).unwrap();
        assert_eq!(value["kind"], "resultSetCompleted");
        assert_eq!(value["partial"], true);
    }
}
