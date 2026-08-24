use serde::{Deserialize, Serialize};

use crate::postgres::sql_class::StatementClassSummary;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum AnalyzeSource {
    Statement { sql: String },
    Relation { schema: String, table: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyzeResultSetPayload {
    pub connection_id: String,
    pub tab_id: String,
    pub request_id: u64,
    pub source: AnalyzeSource,
    #[serde(default)]
    pub refresh_structure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum ColumnOrigin {
    Table {
        schema: String,
        table: String,
        column: String,
        attnum: i16,
    },
    Expression,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum ColumnWritability {
    Writable,
    Generated,
    IdentityAlways,
    SystemColumn,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyzedColumn {
    pub name: String,
    pub origin: ColumnOrigin,
    pub cast_type: String,
    pub nullable: bool,
    pub writability: ColumnWritability,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MutationIdentityKind {
    PrimaryKey,
    UniqueIndex,
    VirtualKey,
    CtidFallback,
    None,
}

impl MutationIdentityKind {
    pub(crate) fn is_keyed(self) -> bool {
        matches!(self, Self::PrimaryKey | Self::UniqueIndex)
    }

    pub(crate) fn requires_full_row_guards(self) -> bool {
        matches!(self, Self::VirtualKey | Self::CtidFallback)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationIdentity {
    pub kind: MutationIdentityKind,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CapabilityReason {
    NotAnalyzable,
    NoIdentity,
    IdentityNotProjected,
    MultipleOriginTables,
    NoWritableColumns,
    CtidInsertUnsupported,
    InvalidVirtualKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilityVerdict {
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CapabilityReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyzedTable {
    pub schema: String,
    pub table: String,
    pub identity: MutationIdentity,
    pub identity_projected: bool,
    pub identity_projection_indexes: Vec<usize>,
    pub updatable: CapabilityVerdict,
    pub deletable: CapabilityVerdict,
    pub insertable: CapabilityVerdict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum NotAnalyzableReason {
    MultiStatement,
    NoProjectedColumns,
    NoTableOrigins,
    PossibleTempShadowing,
    Database {
        code: Option<String>,
        message: String,
        severity: Option<String>,
        position: Option<u32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum AnalysisStatement {
    Analyzed,
    NotAnalyzable { reason: NotAnalyzableReason },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyzeResultSetResult {
    pub request_id: u64,
    pub analysis_id: u64,
    pub columns: Vec<AnalyzedColumn>,
    pub tables: Vec<AnalyzedTable>,
    pub statement: AnalysisStatement,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationTable {
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationValue {
    pub column: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum MutationOp {
    Update {
        table: MutationTable,
        identity: Vec<MutationValue>,
        guards: Vec<MutationValue>,
        set: Vec<MutationValue>,
    },
    Delete {
        table: MutationTable,
        identity: Vec<MutationValue>,
        guards: Vec<MutationValue>,
    },
    Insert {
        table: MutationTable,
        values: Vec<MutationValue>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationPlan {
    pub operations: Vec<MutationOp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewResultMutationsPayload {
    pub connection_id: String,
    pub tab_id: String,
    pub analysis_id: u64,
    pub plan: MutationPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyResultMutationsPayload {
    pub connection_id: String,
    pub tab_id: String,
    pub request_id: u64,
    pub analysis_id: u64,
    pub plan: MutationPlan,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelResultMutationPayload {
    pub connection_id: String,
    pub tab_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseResultMutationPayload {
    pub connection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadVirtualKeyPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveVirtualKeyPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClearVirtualKeyPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VirtualKey {
    pub version: u32,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum DmlParam {
    Text { value: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewStatement {
    pub op_index: usize,
    pub sql: String,
    pub params: Vec<DmlParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewResult {
    pub statements: Vec<PreviewStatement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppliedOperation {
    pub op_index: usize,
    pub rows_affected: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyResult {
    pub operations: Vec<AppliedOperation>,
    pub runtime_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelResultMutationResult {
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InvalidPlanReason {
    EmptySet,
    EmptyIdentity,
    NullKeyedIdentity,
    MissingGuard,
    IdentityMismatch,
    TableMismatch,
    DuplicateColumn,
    GeneratedColumn,
    IdentityAlwaysColumn,
    SystemColumn,
    NoIdentity,
    MultipleOriginTables,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum ResultMutationError {
    UnsupportedEngine,
    NotAnalyzable {
        reason: NotAnalyzableReason,
    },
    UnknownColumn {
        column: String,
    },
    InvalidPlan {
        reason: InvalidPlanReason,
    },
    AnalysisExpired,
    Conflict {
        op_index: usize,
    },
    IdentityNotUnique {
        op_index: usize,
    },
    LockTimeout {
        op_index: usize,
    },
    Busy,
    Superseded,
    Cancelled,
    ConnectionClosing,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        op_index: Option<usize>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn table() -> MutationTable {
        MutationTable {
            schema: "public".into(),
            table: "users".into(),
        }
    }

    fn plan() -> MutationPlan {
        MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(),
                identity: vec![MutationValue {
                    column: "id".into(),
                    value: Some("1".into()),
                }],
                guards: vec![MutationValue {
                    column: "name".into(),
                    value: None,
                }],
                set: vec![MutationValue {
                    column: "name".into(),
                    value: Some("Ada".into()),
                }],
            }],
        }
    }

    #[test]
    fn command_payloads_are_camel_case_and_tagged() {
        let statement = serde_json::to_value(AnalyzeResultSetPayload {
            connection_id: "c1".into(),
            tab_id: "t1".into(),
            request_id: 7,
            source: AnalyzeSource::Statement {
                sql: "SELECT * FROM users".into(),
            },
            refresh_structure: true,
        })
        .unwrap();
        assert_eq!(
            statement,
            json!({
                "connectionId": "c1",
                "tabId": "t1",
                "requestId": 7,
                "source": { "kind": "statement", "sql": "SELECT * FROM users" },
                "refreshStructure": true
            })
        );

        let relation = serde_json::to_value(AnalyzeSource::Relation {
            schema: "public".into(),
            table: "users".into(),
        })
        .unwrap();
        assert_eq!(
            relation,
            json!({ "kind": "relation", "schema": "public", "table": "users" })
        );

        let preview = serde_json::to_value(PreviewResultMutationsPayload {
            connection_id: "c1".into(),
            tab_id: "t1".into(),
            analysis_id: 8,
            plan: plan(),
        })
        .unwrap();
        assert_eq!(preview["analysisId"], 8);
        assert_eq!(preview["plan"]["operations"][0]["kind"], "update");
        assert_eq!(preview["plan"]["operations"][0]["set"][0]["value"], "Ada");
        assert!(preview["plan"]["operations"][0]["guards"][0]["value"].is_null());

        let apply = serde_json::to_value(ApplyResultMutationsPayload {
            connection_id: "c1".into(),
            tab_id: "t1".into(),
            request_id: 9,
            analysis_id: 8,
            confirmed: false,
            plan: plan(),
        })
        .unwrap();
        assert_eq!(apply["requestId"], 9);
        assert_eq!(apply["analysisId"], 8);
        assert_eq!(apply["confirmed"], false);

        let legacy_apply: ApplyResultMutationsPayload = serde_json::from_value(json!({
            "connectionId": "c1",
            "tabId": "t1",
            "requestId": 9,
            "analysisId": 8,
            "plan": { "operations": [] }
        }))
        .unwrap();
        assert!(!legacy_apply.confirmed);

        assert_eq!(
            serde_json::to_value(CancelResultMutationPayload {
                connection_id: "c1".into(),
                tab_id: "t1".into(),
            })
            .unwrap(),
            json!({ "connectionId": "c1", "tabId": "t1" })
        );
        assert_eq!(
            serde_json::to_value(CloseResultMutationPayload {
                connection_id: "c1".into(),
            })
            .unwrap(),
            json!({ "connectionId": "c1" })
        );
        assert_eq!(
            serde_json::to_value(LoadVirtualKeyPayload {
                connection_id: "c1".into(),
                schema: "public".into(),
                table: "users".into(),
            })
            .unwrap(),
            json!({ "connectionId": "c1", "schema": "public", "table": "users" })
        );
        assert_eq!(
            serde_json::to_value(SaveVirtualKeyPayload {
                connection_id: "c1".into(),
                schema: "public".into(),
                table: "users".into(),
                columns: vec!["email".into()],
            })
            .unwrap(),
            json!({
                "connectionId": "c1",
                "schema": "public",
                "table": "users",
                "columns": ["email"]
            })
        );
        assert_eq!(
            serde_json::to_value(ClearVirtualKeyPayload {
                connection_id: "c1".into(),
                schema: "public".into(),
                table: "users".into(),
            })
            .unwrap(),
            json!({ "connectionId": "c1", "schema": "public", "table": "users" })
        );
    }

    #[test]
    fn analysis_result_covers_all_tagged_shapes() {
        let result = AnalyzeResultSetResult {
            request_id: 7,
            analysis_id: 8,
            columns: vec![
                AnalyzedColumn {
                    name: "id".into(),
                    origin: ColumnOrigin::Table {
                        schema: "public".into(),
                        table: "users".into(),
                        column: "id".into(),
                        attnum: 1,
                    },
                    cast_type: "integer".into(),
                    nullable: false,
                    writability: ColumnWritability::Writable,
                },
                AnalyzedColumn {
                    name: "count".into(),
                    origin: ColumnOrigin::Expression,
                    cast_type: "bigint".into(),
                    nullable: true,
                    writability: ColumnWritability::SystemColumn,
                },
            ],
            tables: vec![AnalyzedTable {
                schema: "public".into(),
                table: "users".into(),
                identity: MutationIdentity {
                    kind: MutationIdentityKind::PrimaryKey,
                    columns: vec!["id".into()],
                },
                identity_projected: true,
                identity_projection_indexes: vec![0],
                updatable: CapabilityVerdict {
                    allowed: true,
                    reason: None,
                },
                deletable: CapabilityVerdict {
                    allowed: false,
                    reason: Some(CapabilityReason::IdentityNotProjected),
                },
                insertable: CapabilityVerdict {
                    allowed: false,
                    reason: Some(CapabilityReason::MultipleOriginTables),
                },
            }],
            statement: AnalysisStatement::Analyzed,
        };
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["requestId"], 7);
        assert_eq!(value["analysisId"], 8);
        assert_eq!(value["columns"][0]["origin"]["kind"], "table");
        assert_eq!(value["columns"][0]["origin"]["attnum"], 1);
        assert_eq!(value["columns"][0]["castType"], "integer");
        assert_eq!(value["columns"][0]["writability"]["kind"], "writable");
        assert_eq!(value["columns"][1]["origin"]["kind"], "expression");
        assert_eq!(value["tables"][0]["identity"]["kind"], "primaryKey");
        assert_eq!(value["tables"][0]["identityProjected"], true);
        assert_eq!(value["tables"][0]["identityProjectionIndexes"], json!([0]));
        assert!(value["tables"][0]["updatable"].get("reason").is_none());
        assert_eq!(
            value["tables"][0]["deletable"]["reason"],
            "identityNotProjected"
        );
        assert_eq!(value["statement"]["kind"], "analyzed");

        for (writability, expected) in [
            (ColumnWritability::Writable, "writable"),
            (ColumnWritability::Generated, "generated"),
            (ColumnWritability::IdentityAlways, "identityAlways"),
            (ColumnWritability::SystemColumn, "systemColumn"),
        ] {
            assert_eq!(serde_json::to_value(writability).unwrap()["kind"], expected);
        }
        for (kind, expected) in [
            (MutationIdentityKind::PrimaryKey, "primaryKey"),
            (MutationIdentityKind::UniqueIndex, "uniqueIndex"),
            (MutationIdentityKind::VirtualKey, "virtualKey"),
            (MutationIdentityKind::CtidFallback, "ctidFallback"),
            (MutationIdentityKind::None, "none"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), expected);
        }
    }

    #[test]
    fn not_analyzable_reason_shapes_are_typed() {
        let reasons = [
            (NotAnalyzableReason::MultiStatement, "multiStatement"),
            (
                NotAnalyzableReason::NoProjectedColumns,
                "noProjectedColumns",
            ),
            (NotAnalyzableReason::NoTableOrigins, "noTableOrigins"),
            (
                NotAnalyzableReason::PossibleTempShadowing,
                "possibleTempShadowing",
            ),
        ];
        for (reason, expected) in reasons {
            assert_eq!(serde_json::to_value(reason).unwrap()["kind"], expected);
        }
        let database = NotAnalyzableReason::Database {
            code: Some("42601".into()),
            message: "syntax error".into(),
            severity: Some("ERROR".into()),
            position: Some(4),
        };
        let statement =
            serde_json::to_value(AnalysisStatement::NotAnalyzable { reason: database }).unwrap();
        assert_eq!(statement["kind"], "notAnalyzable");
        assert_eq!(statement["reason"]["kind"], "database");
        assert_eq!(statement["reason"]["position"], 4);
    }

    #[test]
    fn preview_apply_cancel_and_virtual_key_results_are_camel_case() {
        let preview = serde_json::to_value(PreviewResult {
            statements: vec![PreviewStatement {
                op_index: 2,
                sql: "UPDATE x".into(),
                params: vec![
                    DmlParam::Text {
                        value: Some("value".into()),
                    },
                    DmlParam::Text { value: None },
                ],
            }],
        })
        .unwrap();
        assert_eq!(preview["statements"][0]["opIndex"], 2);
        assert_eq!(preview["statements"][0]["params"][0]["kind"], "text");
        assert_eq!(preview["statements"][0]["params"][0]["value"], "value");
        assert!(preview["statements"][0]["params"][1]["value"].is_null());

        let apply = serde_json::to_value(ApplyResult {
            operations: vec![AppliedOperation {
                op_index: 2,
                rows_affected: 1,
            }],
            runtime_ms: 12,
        })
        .unwrap();
        assert_eq!(apply["operations"][0]["opIndex"], 2);
        assert_eq!(apply["operations"][0]["rowsAffected"], 1);
        assert_eq!(apply["runtimeMs"], 12);

        assert_eq!(
            serde_json::to_value(CancelResultMutationResult {
                cancel_requested: true,
            })
            .unwrap(),
            json!({ "cancelRequested": true })
        );
        assert_eq!(
            serde_json::to_value(VirtualKey {
                version: 1,
                columns: vec!["email".into(), "tenant_id".into()],
            })
            .unwrap(),
            json!({ "version": 1, "columns": ["email", "tenant_id"] })
        );
        assert_eq!(
            serde_json::to_value(Option::<VirtualKey>::None).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn every_error_variant_has_the_stable_wire_shape() {
        let errors = [
            (ResultMutationError::UnsupportedEngine, "unsupportedEngine"),
            (
                ResultMutationError::NotAnalyzable {
                    reason: NotAnalyzableReason::NoTableOrigins,
                },
                "notAnalyzable",
            ),
            (
                ResultMutationError::UnknownColumn {
                    column: "missing".into(),
                },
                "unknownColumn",
            ),
            (
                ResultMutationError::InvalidPlan {
                    reason: InvalidPlanReason::EmptySet,
                },
                "invalidPlan",
            ),
            (ResultMutationError::AnalysisExpired, "analysisExpired"),
            (ResultMutationError::Conflict { op_index: 1 }, "conflict"),
            (
                ResultMutationError::IdentityNotUnique { op_index: 1 },
                "identityNotUnique",
            ),
            (
                ResultMutationError::LockTimeout { op_index: 1 },
                "lockTimeout",
            ),
            (ResultMutationError::Busy, "busy"),
            (ResultMutationError::Superseded, "superseded"),
            (ResultMutationError::Cancelled, "cancelled"),
            (ResultMutationError::ConnectionClosing, "connectionClosing"),
            (ResultMutationError::ConnectionLost, "connectionLost"),
            (
                ResultMutationError::PolicyBlocked {
                    reason: "read-only".into(),
                },
                "policyBlocked",
            ),
            (
                ResultMutationError::PolicyNeedsConfirmation {
                    statements: vec![crate::postgres::sql_class::StatementClass::Dml {
                        unbounded: false,
                        destructive: false,
                    }
                    .summary(0)],
                },
                "policyNeedsConfirmation",
            ),
            (
                ResultMutationError::Timeout {
                    operation: "queueWait".into(),
                },
                "timeout",
            ),
            (
                ResultMutationError::Database {
                    code: Some("23505".into()),
                    message: "duplicate".into(),
                    severity: Some("ERROR".into()),
                    position: Some(3),
                    op_index: Some(4),
                },
                "database",
            ),
        ];

        for (error, expected_kind) in errors {
            let value = serde_json::to_value(error).unwrap();
            assert_eq!(value["kind"], expected_kind);
        }

        let invalid = serde_json::to_value(ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::IdentityAlwaysColumn,
        })
        .unwrap();
        assert_eq!(invalid["reason"], "identityAlwaysColumn");
        let conflict = serde_json::to_value(ResultMutationError::Conflict { op_index: 6 }).unwrap();
        assert_eq!(conflict["opIndex"], 6);
        let database = serde_json::to_value(ResultMutationError::Database {
            code: None,
            message: "closed".into(),
            severity: None,
            position: None,
            op_index: None,
        })
        .unwrap();
        assert!(database.get("opIndex").is_none());
    }

    #[test]
    fn payload_round_trip_rejects_untagged_operations() {
        let payload = serde_json::to_value(PreviewResultMutationsPayload {
            connection_id: "c1".into(),
            tab_id: "t1".into(),
            analysis_id: 1,
            plan: plan(),
        })
        .unwrap();
        let decoded: PreviewResultMutationsPayload = serde_json::from_value(payload).unwrap();
        assert_eq!(decoded.plan, plan());

        assert!(serde_json::from_value::<MutationOp>(json!({
            "table": { "schema": "public", "table": "users" },
            "identity": [],
            "guards": [],
            "set": []
        }))
        .is_err());
    }
}
