//! Maps the baseline React `ui.v1.session` record into the native workspace.
//!
//! Only query tabs carry restorable SQL in a form the native workspace models.
//! Table, designer and object tabs, tabs bound to non-PostgreSQL connections and
//! tabs beyond the native document limit stay only in the preserved React key.
//! Bindings to missing connections are kept so their SQL remains recoverable.

use super::super::development::encode_workspace_record;
use crate::backend::{
    WorkspaceDocument, WorkspaceError, WorkspaceSelection, WorkspaceSnapshot,
    NATIVE_WORKSPACE_MAX_DOCUMENTS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMappingStatus {
    /// No baseline session record existed.
    Absent,
    /// The baseline record could not be parsed; it is preserved unchanged.
    Unreadable,
    /// The record held no query tab the native workspace can represent.
    NoQueryTabs,
    /// Mapped query tabs exceed the native byte budget; nothing was mapped.
    OverBudget,
    Mapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceMapping {
    pub status: WorkspaceMappingStatus,
    pub mapped_documents: u64,
    pub unmapped_tabs: u64,
}

pub(super) struct Mapped {
    pub mapping: WorkspaceMapping,
    /// Native key and encoded record when status is `Mapped`.
    pub record: Option<(&'static str, String)>,
}

/// `unsupported` holds IDs of stored connections the native workspace cannot
/// bind (non-PostgreSQL engines).
pub(super) fn map_session(raw: Option<&str>, unsupported: &HashSet<String>) -> Mapped {
    let empty = |status, unmapped| Mapped {
        mapping: WorkspaceMapping {
            status,
            mapped_documents: 0,
            unmapped_tabs: unmapped,
        },
        record: None,
    };
    let Some(raw) = raw else {
        return empty(WorkspaceMappingStatus::Absent, 0);
    };
    let Ok(Value::Object(session)) = serde_json::from_str::<Value>(raw) else {
        return empty(WorkspaceMappingStatus::Unreadable, 0);
    };
    let tabs = match session.get("tabs") {
        Some(Value::Array(tabs)) => tabs.as_slice(),
        None => &[],
        Some(_) => return empty(WorkspaceMappingStatus::Unreadable, 0),
    };
    let mut documents = Vec::new();
    let mut ids = HashSet::new();
    let mut unmapped = 0u64;
    for tab in tabs {
        match document(tab, unsupported) {
            Some(document)
                if documents.len() < NATIVE_WORKSPACE_MAX_DOCUMENTS
                    && ids.insert(document.id.clone()) =>
            {
                documents.push(document)
            }
            _ => unmapped += 1,
        }
    }
    if documents.is_empty() {
        return empty(WorkspaceMappingStatus::NoQueryTabs, unmapped);
    }
    let active = session
        .get("activeTabId")
        .and_then(Value::as_str)
        .filter(|id| ids.contains(*id))
        .map_or_else(|| documents[0].id.clone(), str::to_owned);
    let mapped = documents.len() as u64;
    let snapshot = WorkspaceSnapshot {
        documents,
        active_document_id: Some(active),
        ..Default::default()
    };
    match encode_workspace_record(snapshot) {
        Ok(record) => Mapped {
            mapping: WorkspaceMapping {
                status: WorkspaceMappingStatus::Mapped,
                mapped_documents: mapped,
                unmapped_tabs: unmapped,
            },
            record: Some(record),
        },
        Err(WorkspaceError::TooLarge) => {
            empty(WorkspaceMappingStatus::OverBudget, unmapped + mapped)
        }
        Err(_) => empty(WorkspaceMappingStatus::Unreadable, unmapped + mapped),
    }
}

fn document(tab: &Value, unsupported: &HashSet<String>) -> Option<WorkspaceDocument> {
    let tab = tab.as_object()?;
    if tab.get("kind")?.as_str()? != "query" {
        return None;
    }
    let id = tab.get("id")?.as_str()?;
    let name = tab.get("label")?.as_str()?;
    let connection = tab.get("connectionId")?.as_str()?;
    if id.is_empty()
        || id.len() > 128
        || name.is_empty()
        || name.len() > 512
        || connection.is_empty()
        || connection.len() > 128
        || unsupported.contains(connection)
    {
        return None;
    }
    let sql = match tab.get("query") {
        Some(Value::String(sql)) => sql.clone(),
        None => String::new(),
        Some(_) => return None,
    };
    let selection = tab
        .get("caret")
        .and_then(|caret| caret_selection(&sql, caret))
        .unwrap_or_default();
    Some(WorkspaceDocument {
        id: id.into(),
        name: name.into(),
        connection_id: Some(connection.into()),
        sql,
        pinned: tab.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        selection,
        table: None,
        query_changes: None,
        schema_changes: None,
        table_ddl: None,
        schema_alter: None,
        object_ddl: None,
        admin_control: None,
        maintenance: None,
        tool: None,
        saved_query_id: None,
    })
}

/// Baseline carets are 1-based Monaco line/column pairs whose columns count
/// UTF-16 code units; native selections are UTF-8 byte offsets.
fn caret_selection(sql: &str, caret: &Value) -> Option<WorkspaceSelection> {
    let number = |key: &str| caret.get(key).and_then(Value::as_u64);
    let head = offset(sql, number("line")?, number("column")?);
    let anchor = match (number("anchorLine"), number("anchorColumn")) {
        (Some(line), Some(column)) => offset(sql, line, column),
        _ => head,
    };
    Some(WorkspaceSelection { anchor, head })
}

pub(super) fn offset(sql: &str, line: u64, column: u64) -> usize {
    let mut start = 0;
    for _ in 1..line.max(1) {
        match sql[start..].find('\n') {
            Some(index) => start += index + 1,
            None => return sql.len(),
        }
    }
    let end = sql[start..]
        .find('\n')
        .map_or(sql.len(), |index| start + index);
    let content = sql[start..end]
        .strip_suffix('\r')
        .unwrap_or(&sql[start..end]);
    let mut units = 0u64;
    for (index, character) in content.char_indices() {
        if units >= column.saturating_sub(1) {
            return start + index;
        }
        units += character.len_utf16() as u64;
    }
    start + content.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monaco_utf16_carets_become_utf8_boundaries() {
        let sql = "select 1;\r\nselect '😀é' -- x\nend";
        assert_eq!(offset(sql, 1, 1), 0);
        assert_eq!(offset(sql, 1, 8), 7);
        // Line 2 starts after CRLF; the emoji is two UTF-16 units, four bytes.
        let line2 = sql.find("select '").unwrap();
        assert_eq!(offset(sql, 2, 9), line2 + 8);
        assert_eq!(offset(sql, 2, 11), line2 + 12);
        assert_eq!(offset(sql, 2, 12), line2 + 14);
        // A column inside the surrogate pair clamps to the next boundary.
        assert!(sql.is_char_boundary(offset(sql, 2, 10)));
        // Past line end stops before CR; past the last line clamps to the end.
        assert_eq!(offset(sql, 1, 99), 9);
        assert_eq!(offset(sql, 9, 1), sql.len());
        assert_eq!(offset(sql, 3, 4), sql.len());
        assert_eq!(offset("", 0, 0), 0);
    }

    #[test]
    fn only_bindable_query_tabs_map_and_the_rest_are_counted() {
        let unsupported = HashSet::from(["mysql".to_string()]);
        let session = serde_json::json!({
            "tabs": [
                {"id": "q1", "kind": "query", "label": "One", "connectionId": "pg", "schema": "public",
                 "query": "select 1", "pinned": true, "caret": {"line": 1, "column": 3, "anchorLine": 1, "anchorColumn": 1},
                 "futureTabField": {"kept": "in the React key"}},
                {"id": "q2", "kind": "query", "label": "Missing binding", "connectionId": "deleted", "schema": ""},
                {"id": "q3", "kind": "query", "label": "MySQL", "connectionId": "mysql", "schema": "", "query": "select 3"},
                {"id": "t1", "kind": "table", "label": "users", "connectionId": "pg", "schema": "public", "table": "users"},
                {"id": "q1", "kind": "query", "label": "Duplicate", "connectionId": "pg", "schema": ""}
            ],
            "activeTabId": "t1"
        })
        .to_string();
        let mapped = map_session(Some(&session), &unsupported);
        assert_eq!(
            mapped.mapping,
            WorkspaceMapping {
                status: WorkspaceMappingStatus::Mapped,
                mapped_documents: 2,
                unmapped_tabs: 3
            }
        );
        let (key, record) = mapped.record.unwrap();
        assert_eq!(key, "ui.v1.native.workspace");
        let value: Value = serde_json::from_str(&record).unwrap();
        let documents = value["snapshot"]["documents"].as_array().unwrap();
        assert_eq!(
            documents[0]["selection"],
            serde_json::json!({"anchor": 0, "head": 2})
        );
        assert_eq!(documents[0]["pinned"], true);
        assert_eq!(documents[1]["connectionId"], "deleted");
        assert_eq!(documents[1]["sql"], "");
        // The active table tab is not representable, so the first document is.
        assert_eq!(value["snapshot"]["activeDocumentId"], "q1");

        for (raw, status) in [
            (None, WorkspaceMappingStatus::Absent),
            (Some("{not json"), WorkspaceMappingStatus::Unreadable),
            (Some(r#"{"tabs": 3}"#), WorkspaceMappingStatus::Unreadable),
            (Some(r#"{"tabs": []}"#), WorkspaceMappingStatus::NoQueryTabs),
        ] {
            let mapped = map_session(raw, &unsupported);
            assert_eq!(mapped.mapping.status, status);
            assert!(mapped.record.is_none());
        }
    }

    #[test]
    fn over_budget_sessions_map_nothing() {
        let tabs: Vec<_> = (0..4)
            .map(|index| {
                serde_json::json!({"id": format!("q{index}"), "kind": "query", "label": "Large",
                    "connectionId": "pg", "schema": "", "query": "x".repeat(200 * 1024)})
            })
            .collect();
        let session = serde_json::json!({ "tabs": tabs }).to_string();
        let mapped = map_session(Some(&session), &HashSet::new());
        assert_eq!(mapped.mapping.status, WorkspaceMappingStatus::OverBudget);
        assert_eq!(mapped.mapping.unmapped_tabs, 4);
        assert!(mapped.record.is_none());
    }
}
