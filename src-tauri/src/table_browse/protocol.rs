use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ComparisonOperator {
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TextMatchOperator {
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum BrowseFilter {
    Comparison {
        column: String,
        operator: ComparisonOperator,
        value: String,
    },
    TextMatch {
        column: String,
        operator: TextMatchOperator,
        value: String,
    },
    IsNull {
        column: String,
    },
    IsNotNull {
        column: String,
    },
    InList {
        column: String,
        values: Vec<String>,
    },
    RawSql {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BrowseSortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BrowseNulls {
    Default,
    First,
    Last,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseSortKey {
    pub column: String,
    pub direction: BrowseSortDirection,
    pub nulls: BrowseNulls,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseCursor {
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum BrowsePageRequest {
    Offset { page: u32 },
    Keyset { cursor: Option<BrowseCursor> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BrowseCountPolicy {
    None,
    Estimated,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseTableDataPayload {
    pub connection_id: String,
    pub tab_id: String,
    pub request_id: u64,
    pub schema: String,
    pub table: String,
    #[serde(default)]
    pub filters: Vec<BrowseFilter>,
    #[serde(default)]
    pub sort: Vec<BrowseSortKey>,
    pub page_request: BrowsePageRequest,
    pub page_size: u32,
    pub count_policy: BrowseCountPolicy,
    #[serde(default)]
    pub refresh_structure: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CountTableBrowseRowsPayload {
    pub connection_id: String,
    pub tab_id: String,
    pub request_id: u64,
    pub schema: String,
    pub table: String,
    #[serde(default)]
    pub filters: Vec<BrowseFilter>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableBrowseTabPayload {
    pub connection_id: String,
    pub tab_id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadTableGridPrefsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveTableGridPrefsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub prefs: TableGridPrefs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub(crate) struct TableGridPrefs(pub serde_json::Value);

pub(crate) const TABLE_GRID_PREFS_HISTORY_CAP: usize = 20;

pub(crate) fn validate_table_grid_prefs(prefs: TableGridPrefs) -> Result<TableGridPrefs, String> {
    let mut value = prefs.0;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "table grid prefs must be a JSON object".to_string())?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "table grid prefs.version is required".to_string())?;
    if version < 1 {
        return Err("table grid prefs.version must be >= 1".to_string());
    }
    for key in ["filterHistory", "sortHistory"] {
        if let Some(serde_json::Value::Array(entries)) = object.get_mut(key) {
            entries.truncate(TABLE_GRID_PREFS_HISTORY_CAP);
        }
    }
    Ok(TableGridPrefs(value))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BrowseIdentityKind {
    PrimaryKey,
    UniqueIndex,
    Virtual,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseIdentity {
    pub kind: BrowseIdentityKind,
    pub columns: Vec<String>,
}

impl BrowseIdentity {
    pub(crate) fn exists(&self) -> bool {
        self.kind != BrowseIdentityKind::None && !self.columns.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseColumn {
    pub name: String,
    pub cast_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BrowsePageMode {
    Offset,
    Keyset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowsePageInfo {
    pub mode: BrowsePageMode,
    pub page: Option<u32>,
    pub has_more: bool,
    pub next_cursor: Option<BrowseCursor>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BrowseCountKind {
    Exact,
    Estimated,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseCount {
    pub kind: BrowseCountKind,
    pub value: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum InspectionParam {
    Text { value: String },
    TextArray { values: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseInspection {
    pub sql: String,
    pub params: Vec<InspectionParam>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseTableResult {
    pub request_id: u64,
    pub columns: Vec<BrowseColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub identity: BrowseIdentity,
    pub row_identity: Option<Vec<Vec<String>>>,
    pub page_info: BrowsePageInfo,
    pub count: BrowseCount,
    pub inspection: BrowseInspection,
    pub omitted_rows: u64,
    pub truncated_cells: u64,
    pub runtime_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowseExactCountResult {
    pub kind: BrowseCountKind,
    pub value: u64,
    pub request_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelTableBrowseResult {
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub(crate) enum TableBrowseError {
    UnsupportedEngine,
    UnknownColumn {
        column: String,
    },
    InvalidFilter {
        reason: String,
    },
    InvalidSort {
        column: String,
    },
    InvalidCursor,
    Superseded,
    Cancelled,
    ConnectionClosing,
    ConnectionLost,
    /// TLS material or handshake failure while opening the socket (ADR-0025).
    TlsFailed {
        tls_kind: crate::TlsFailureKind,
        message: String,
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

impl TableBrowseError {
    pub(crate) fn sqlstate(&self) -> Option<&str> {
        match self {
            Self::Database { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes_are_tagged_and_camel_case() {
        let error = serde_json::to_value(TableBrowseError::Timeout {
            operation: "queueWait".into(),
        })
        .unwrap();
        assert_eq!(error["kind"], "timeout");
        assert_eq!(error["operation"], "queueWait");

        let database = serde_json::to_value(TableBrowseError::Database {
            code: Some("42703".into()),
            message: "missing".into(),
            severity: Some("ERROR".into()),
            position: Some(12),
        })
        .unwrap();
        assert_eq!(database["kind"], "database");
        assert_eq!(database["code"], "42703");
        assert_eq!(database["position"], 12);

        let filter = serde_json::to_value(BrowseFilter::TextMatch {
            column: "name".into(),
            operator: TextMatchOperator::NotContains,
            value: "x".into(),
        })
        .unwrap();
        assert_eq!(filter["kind"], "textMatch");
        assert_eq!(filter["operator"], "notContains");
        assert_eq!(filter["column"], "name");
        assert_eq!(filter["value"], "x");

        let raw = serde_json::to_value(BrowseFilter::RawSql {
            text: "id > 1".into(),
        })
        .unwrap();
        assert_eq!(raw["kind"], "rawSql");
        assert_eq!(raw["text"], "id > 1");

        let page = serde_json::to_value(BrowsePageRequest::Keyset {
            cursor: Some(BrowseCursor {
                values: vec!["1".into()],
            }),
        })
        .unwrap();
        assert_eq!(page["kind"], "keyset");
        assert_eq!(page["cursor"]["values"][0], "1");

        let offset = serde_json::to_value(BrowsePageRequest::Offset { page: 2 }).unwrap();
        assert_eq!(offset["kind"], "offset");
        assert_eq!(offset["page"], 2);

        let identity = serde_json::to_value(BrowseIdentity {
            kind: BrowseIdentityKind::PrimaryKey,
            columns: vec!["id".into()],
        })
        .unwrap();
        assert_eq!(identity["kind"], "primaryKey");
        assert_eq!(identity["columns"][0], "id");

        let result = serde_json::to_value(BrowseTableResult {
            request_id: 9,
            columns: vec![BrowseColumn {
                name: "id".into(),
                cast_type: "integer".into(),
                nullable: false,
            }],
            rows: vec![vec![Some("1".into()), None]],
            identity: BrowseIdentity {
                kind: BrowseIdentityKind::Virtual,
                columns: vec!["ctid".into()],
            },
            row_identity: Some(vec![vec!["(0,1)".into()]]),
            page_info: BrowsePageInfo {
                mode: BrowsePageMode::Keyset,
                page: None,
                has_more: true,
                next_cursor: Some(BrowseCursor {
                    values: vec!["(0,1)".into()],
                }),
            },
            count: BrowseCount {
                kind: BrowseCountKind::Estimated,
                value: Some(10),
            },
            inspection: BrowseInspection {
                sql: "SELECT 1".into(),
                params: vec![
                    InspectionParam::Text { value: "a".into() },
                    InspectionParam::TextArray {
                        values: vec!["1".into(), "2".into()],
                    },
                ],
            },
            omitted_rows: 1,
            truncated_cells: 2,
            runtime_ms: 3,
        })
        .unwrap();
        assert_eq!(result["requestId"], 9);
        assert_eq!(result["rowIdentity"][0][0], "(0,1)");
        assert_eq!(result["pageInfo"]["hasMore"], true);
        assert_eq!(result["pageInfo"]["nextCursor"]["values"][0], "(0,1)");
        assert_eq!(result["count"]["kind"], "estimated");
        assert_eq!(result["omittedRows"], 1);
        assert_eq!(result["truncatedCells"], 2);
        assert_eq!(result["runtimeMs"], 3);
        assert_eq!(result["columns"][0]["castType"], "integer");
        assert_eq!(result["inspection"]["params"][0]["kind"], "text");
        assert_eq!(result["inspection"]["params"][0]["value"], "a");
        assert_eq!(result["inspection"]["params"][1]["kind"], "textArray");
        assert_eq!(result["inspection"]["params"][1]["values"][1], "2");
    }

    #[test]
    fn payload_wire_shape_round_trips() {
        let json = serde_json::json!({
            "connectionId": "c1",
            "tabId": "t1",
            "requestId": 7,
            "schema": "public",
            "table": "users",
            "filters": [{
                "kind": "inList",
                "column": "id",
                "values": ["1", "2"]
            }],
            "sort": [{
                "column": "name",
                "direction": "desc",
                "nulls": "last"
            }],
            "pageRequest": { "kind": "offset", "page": 3 },
            "pageSize": 50,
            "countPolicy": "estimated",
            "refreshStructure": true
        });
        let payload: BrowseTableDataPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.connection_id, "c1");
        assert_eq!(payload.request_id, 7);
        assert_eq!(payload.page_size, 50);
        assert_eq!(payload.count_policy, BrowseCountPolicy::Estimated);
        assert!(payload.refresh_structure);
        match &payload.filters[0] {
            BrowseFilter::InList { values, .. } => {
                assert_eq!(values, &["1".to_string(), "2".into()]);
            }
            other => panic!("expected inList filter, got {other:?}"),
        }
    }

    #[test]
    fn comparison_filter_requires_value_and_rejects_bag_shape() {
        assert!(serde_json::from_value::<BrowseFilter>(serde_json::json!({
            "kind": "comparison",
            "column": "id",
            "operator": "eq"
        }))
        .is_err());
        assert!(serde_json::from_value::<BrowseFilter>(serde_json::json!({
            "kind": "column",
            "column": "id",
            "operator": "eq",
            "value": "1"
        }))
        .is_err());
    }

    #[test]
    fn table_grid_prefs_require_version_and_cap_history() {
        let too_many = (0..25).map(|i| serde_json::json!(i)).collect::<Vec<_>>();
        let prefs = validate_table_grid_prefs(TableGridPrefs(serde_json::json!({
            "version": 1,
            "filterHistory": too_many,
            "pageSize": 25
        })))
        .unwrap();
        assert_eq!(prefs.0["filterHistory"].as_array().unwrap().len(), 20);
        assert!(
            validate_table_grid_prefs(TableGridPrefs(serde_json::json!({"version": 0}))).is_err()
        );
        assert!(validate_table_grid_prefs(TableGridPrefs(serde_json::json!("nope"))).is_err());
    }
}
