use super::tests::{pool, raw, seed, snapshot};
use super::*;
use crate::backend::table_ddl::{
    TableDdlAttemptId, TableDdlColumn, TableDdlDescription, TableDdlIntent, TableDdlPreview,
    TableIdentity, TABLE_DDL_OPERATION_TIMEOUT_MS,
};

fn journal(column: bool, comment: Option<&str>) -> WorkspaceTableDdl {
    let target = TableDdlDescription {
        identity: TableIdentity {
            database_oid: 12,
            relation_oid: 56,
        },
        schema_oid: 34,
        schema: "Exact \"資料".into(),
        namespace_xmin: "123".into(),
        namespace_ctid: "(2,3)".into(),
        table: " rows ".into(),
        column: column.then(|| TableDdlColumn {
            attnum: 7,
            name: " ".into(),
        }),
        comment: Some("old comment\n🙂".into()),
    };
    let qualified = format!(
        "{}.{}",
        crate::quote_double(&target.schema),
        crate::quote_double(&target.table)
    );
    let object = match &target.column {
        Some(column) => format!("COLUMN {qualified}.{}", crate::quote_double(&column.name)),
        None => format!("TABLE {qualified}"),
    };
    WorkspaceTableDdl {
        attempt_id: TableDdlAttemptId::new(),
        target,
        intent: TableDdlIntent::SetComment {
            comment: comment.map(str::to_owned),
        },
        preview: TableDdlPreview {
            sql: format!(
                "COMMENT ON {object} IS {};",
                comment
                    .map(crate::quote_literal)
                    .unwrap_or_else(|| "NULL".into())
            ),
            summary: format!(
                "{} comment on {object}",
                if comment.is_none_or(str::is_empty) {
                    "Clear"
                } else {
                    "Set"
                }
            ),
            statement_timeout_ms: Some(0),
            operation_timeout_ms: TABLE_DDL_OPERATION_TIMEOUT_MS,
        },
        apply_state: WorkspaceApplyState::Staged,
    }
}
fn workspace(record: WorkspaceTableDdl) -> WorkspaceSnapshot {
    let mut value = snapshot();
    let doc = &mut value.documents[0];
    doc.sql.clear();
    doc.selection = WorkspaceSelection::default();
    doc.tool = Some(WorkspaceTool::Objects);
    doc.table_ddl = Some(record);
    value
}

#[tokio::test]
async fn table_ddl_exact_restore_is_read_only_and_preserves_empty_comment_intent() {
    let pool = pool().await;
    let mut revision = None;
    for column in [false, true] {
        for comment in [None, Some(""), Some(" quote ' \\ newline\n資料🙂 ")] {
            for state in [
                WorkspaceApplyState::Staged,
                WorkspaceApplyState::OutcomeUnknown,
            ] {
                let mut record = journal(column, comment);
                record.apply_state = state;
                let expected = workspace(record);
                revision = Some(save(&pool, revision, expected.clone()).await.unwrap());
                let before = raw(&pool).await;
                assert_eq!(load(&pool).await.unwrap().snapshot, Some(expected));
                assert_eq!(raw(&pool).await, before);
                let value: serde_json::Value = serde_json::from_str(&before).unwrap();
                assert_eq!(value["version"], 13);
                let record = &value["snapshot"]["documents"][0]["tableDdl"];
                assert_eq!(record["target"]["table"], " rows ");
                assert_eq!(record["intent"]["comment"], serde_json::json!(comment));
                assert!(record.get("confirmation").is_none());
            }
        }
    }
}

#[tokio::test]
async fn table_ddl_invalid_recovery_cannot_overwrite_original_record() {
    let pool = pool().await;
    let valid = workspace(journal(true, Some("new")));
    let base: serde_json::Value = serde_json::from_str(&encode(valid.clone()).unwrap()).unwrap();
    for defect in 0..17 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 12.into(),
            1 => value["version"] = 14.into(),
            2 => value["snapshot"]["documents"][0]["connectionId"] = serde_json::Value::Null,
            3 => value["snapshot"]["documents"][0]["tool"] = "administration".into(),
            _ => {
                let record = &mut value["snapshot"]["documents"][0]["tableDdl"];
                match defect {
                    4 => record["confirmation"] = true.into(),
                    5 => record["attemptId"] = "invalid".into(),
                    6 => record["target"]["identity"]["relation_oid"] = 0.into(),
                    7 => record["target"]["column"]["attnum"] = 0.into(),
                    8 => record["target"]["schema"] = "x\0".into(),
                    9 => record["intent"]["comment"] = "x\0".into(),
                    10 => record["preview"]["sql"] = "SELECT wrong_target".into(),
                    11 => record["preview"]["summary"] = "different intent".into(),
                    12 => record["preview"]["operationTimeoutMs"] = 0.into(),
                    13 => record["applyState"] = "replay".into(),
                    14 => record["target"]["namespaceXmin"] = "no".into(),
                    15 => record["target"]["namespaceCtid"] = "no".into(),
                    _ => record["intent"]["comment"] = "x".repeat(4097).into(),
                }
            }
        }
        let original = value.to_string();
        seed(&pool, &original).await;
        assert!(load(&pool).await.is_err(), "defect {defect}");
        assert!(
            save(&pool, Some(WorkspaceRevision("seed".into())), valid.clone())
                .await
                .is_err()
        );
        assert_eq!(raw(&pool).await, original);
    }
}

#[test]
fn table_ddl_exact_matching_includes_old_comment_attnum_namespace_and_both_timeouts() {
    let record = journal(true, Some(""));
    assert!(record.matches(
        &record.attempt_id,
        &record.target,
        &record.intent,
        &record.preview
    ));
    for mutation in 0..13 {
        let mut changed = record.clone();
        match mutation {
            0 => changed.attempt_id = TableDdlAttemptId::new(),
            1 => changed.target.identity.database_oid += 1,
            2 => changed.target.identity.relation_oid += 1,
            3 => changed.target.schema_oid += 1,
            4 => changed.target.namespace_xmin.push('1'),
            5 => changed.target.namespace_ctid = "(2,4)".into(),
            6 => changed.target.column.as_mut().unwrap().attnum += 1,
            7 => changed.target.column.as_mut().unwrap().name.push(' '),
            8 => changed.target.comment = None,
            9 => changed.intent = TableDdlIntent::SetComment { comment: None },
            10 => changed.preview.statement_timeout_ms = None,
            11 => changed.preview.operation_timeout_ms += 1,
            _ => changed.preview.sql.push(' '),
        }
        assert!(
            !record.matches(
                &changed.attempt_id,
                &changed.target,
                &changed.intent,
                &changed.preview
            ),
            "mutation {mutation}"
        );
    }
}

#[test]
fn table_ddl_rename_forms_preserve_exact_whitespace_and_quote_spelling() {
    for column in [false, true] {
        let mut record = journal(column, None);
        let name = " \" new資料 ";
        let qualified = format!(
            "{}.{}",
            crate::quote_double(&record.target.schema),
            crate::quote_double(&record.target.table)
        );
        let quoted = crate::quote_double(name);
        record.intent = TableDdlIntent::Rename {
            new_name: name.into(),
        };
        let (sql, summary) = match &record.target.column {
            Some(column) => (
                format!(
                    "ALTER TABLE {qualified} RENAME COLUMN {} TO {quoted};",
                    crate::quote_double(&column.name)
                ),
                format!(
                    "Rename column {} on {qualified} to {quoted}",
                    crate::quote_double(&column.name)
                ),
            ),
            None => (
                format!("ALTER TABLE {qualified} RENAME TO {quoted};"),
                format!("Rename table {qualified} to {quoted}"),
            ),
        };
        record.preview.sql = sql;
        record.preview.summary = summary;
        assert!(record.validate().is_ok());
        let loaded = decode(&encode(workspace(record.clone())).unwrap()).unwrap();
        assert_eq!(loaded.documents[0].table_ddl.as_ref(), Some(&record));
    }
}

#[test]
fn table_ddl_legacy_workspaces_accept_absence_and_reject_even_null_new_fields() {
    let legacy = snapshot();
    let mut value: serde_json::Value =
        serde_json::from_str(&encode(legacy.clone()).unwrap()).unwrap();
    for version in 1..=12 {
        value["version"] = version.into();
        assert_eq!(decode(&value.to_string()).unwrap(), legacy);
        value["snapshot"]["documents"][0]["tableDdl"] = serde_json::Value::Null;
        assert!(decode(&value.to_string()).is_err());
        value["snapshot"]["documents"][0]
            .as_object_mut()
            .unwrap()
            .remove("tableDdl");
    }
}

#[tokio::test]
async fn table_ddl_capacity_envelope_and_other_journals_refuse_without_overwrite() {
    let pool = pool().await;
    let valid = workspace(journal(false, Some("comment")));
    let revision = save(&pool, None, valid.clone()).await.unwrap();
    let original = raw(&pool).await;
    let mut oversized = valid.clone();
    oversized.documents[0]
        .table_ddl
        .as_mut()
        .unwrap()
        .preview
        .sql
        .reserve(64 * 1024);
    assert!(save(&pool, Some(revision.clone()), oversized)
        .await
        .is_err());
    assert_eq!(raw(&pool).await, original);
    let mut envelope = valid.clone();
    envelope.documents[1].sql = "x".repeat(NATIVE_WORKSPACE_MAX_BYTES);
    assert!(save(&pool, Some(revision.clone()), envelope).await.is_err());
    assert_eq!(raw(&pool).await, original);
    let mut conflicting = valid;
    conflicting.documents[0].schema_changes = Some(WorkspaceSchemaChanges {
        attempt_id: TableDdlAttemptId::new(),
        intent: crate::backend::schema_ddl::CreateSchemaIntent::new("new_schema".into(), None)
            .unwrap(),
        apply_state: WorkspaceApplyState::Staged,
    });
    assert!(save(&pool, Some(revision), conflicting).await.is_err());
    assert_eq!(raw(&pool).await, original);
}
