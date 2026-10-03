use super::tests::{pool, raw, seed, snapshot};
use super::*;
use crate::backend::schema_alter::{
    SchemaAlterAttemptId, SchemaAlterDescription, SchemaAlterIntent, SchemaAlterPreview,
    SchemaIdentity, SCHEMA_ALTER_OPERATION_TIMEOUT_MS,
};

fn journal(intent: SchemaAlterIntent) -> WorkspaceSchemaAlter {
    let target = SchemaAlterDescription {
        identity: SchemaIdentity {
            database_oid: 12,
            schema_oid: 34,
        },
        schema: " Exact \"資料".into(),
        namespace_xmin: "123".into(),
        namespace_ctid: "(2,3)".into(),
        comment: Some("old comment\n🙂".into()),
    };
    let schema = crate::quote_double(&target.schema);
    let (sql, summary) = match &intent {
        SchemaAlterIntent::SetComment { comment } => (
            format!(
                "COMMENT ON SCHEMA {schema} IS {};",
                comment
                    .as_deref()
                    .map(crate::quote_literal)
                    .unwrap_or_else(|| "NULL".into())
            ),
            format!(
                "{} comment on SCHEMA {schema}",
                if comment.as_deref().is_none_or(str::is_empty) {
                    "Clear"
                } else {
                    "Set"
                }
            ),
        ),
        SchemaAlterIntent::Rename { new_name } => (
            format!(
                "ALTER SCHEMA {schema} RENAME TO {};",
                crate::quote_double(new_name)
            ),
            format!(
                "Rename schema {schema} to {}",
                crate::quote_double(new_name)
            ),
        ),
    };
    WorkspaceSchemaAlter {
        attempt_id: SchemaAlterAttemptId::new(),
        target,
        intent,
        preview: SchemaAlterPreview {
            sql,
            summary,
            statement_timeout_ms: Some(0),
            operation_timeout_ms: SCHEMA_ALTER_OPERATION_TIMEOUT_MS,
        },
        apply_state: WorkspaceApplyState::Staged,
    }
}
fn intents() -> [SchemaAlterIntent; 4] {
    [
        SchemaAlterIntent::SetComment { comment: None },
        SchemaAlterIntent::SetComment {
            comment: Some(String::new()),
        },
        SchemaAlterIntent::SetComment {
            comment: Some(" quote ' \\ newline\n資料🙂 ".into()),
        },
        SchemaAlterIntent::Rename {
            new_name: " \" new資料 ".into(),
        },
    ]
}
fn workspace(record: WorkspaceSchemaAlter) -> WorkspaceSnapshot {
    let mut value = snapshot();
    let doc = &mut value.documents[0];
    doc.sql.clear();
    doc.selection = WorkspaceSelection::default();
    doc.tool = Some(WorkspaceTool::Objects);
    doc.schema_alter = Some(record);
    value
}

#[tokio::test]
async fn schema_alter_exact_restore_is_read_only_and_written_as_current_version_15() {
    let pool = pool().await;
    let mut revision = None;
    for intent in intents() {
        for state in [
            WorkspaceApplyState::Staged,
            WorkspaceApplyState::OutcomeUnknown,
        ] {
            let mut record = journal(intent.clone());
            record.apply_state = state;
            record.validate().unwrap();
            let expected = workspace(record);
            revision = Some(save(&pool, revision, expected.clone()).await.unwrap());
            let before = raw(&pool).await;
            assert_eq!(load(&pool).await.unwrap().snapshot, Some(expected));
            assert_eq!(raw(&pool).await, before);
            let value: serde_json::Value = serde_json::from_str(&before).unwrap();
            assert_eq!(value["version"], 15);
            let record = &value["snapshot"]["documents"][0]["schemaAlter"];
            assert_eq!(record["target"]["schema"], " Exact \"資料");
            assert_eq!(record["target"]["identity"]["schemaOid"], 34);
            assert!(record.get("confirmation").is_none());
        }
    }
}

#[tokio::test]
async fn schema_alter_invalid_recovery_cannot_overwrite_original_record() {
    let pool = pool().await;
    let valid = workspace(journal(SchemaAlterIntent::SetComment {
        comment: Some("new".into()),
    }));
    let base: serde_json::Value = serde_json::from_str(&encode(valid.clone()).unwrap()).unwrap();
    for defect in 0..16 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 13.into(),
            1 => value["version"] = 16.into(),
            2 => value["snapshot"]["documents"][0]["connectionId"] = serde_json::Value::Null,
            3 => value["snapshot"]["documents"][0]["tool"] = "administration".into(),
            _ => {
                let record = &mut value["snapshot"]["documents"][0]["schemaAlter"];
                match defect {
                    4 => record["confirmation"] = true.into(),
                    5 => record["attemptId"] = "invalid".into(),
                    6 => record["target"]["identity"]["schemaOid"] = 0.into(),
                    7 => record["target"]["schema"] = "x\0".into(),
                    8 => record["intent"]["comment"] = "x\0".into(),
                    9 => record["preview"]["sql"] = "DROP SCHEMA wrong_target".into(),
                    10 => record["preview"]["summary"] = "different intent".into(),
                    11 => record["preview"]["operationTimeoutMs"] = 0.into(),
                    12 => record["applyState"] = "replay".into(),
                    13 => record["target"]["namespaceXmin"] = "no".into(),
                    14 => record["target"]["schema"] = "renamed elsewhere".into(),
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
fn schema_alter_exact_matching_includes_identity_row_version_comment_and_timeouts() {
    let record = journal(SchemaAlterIntent::Rename {
        new_name: "next".into(),
    });
    assert!(record.matches(
        &record.attempt_id,
        &record.target,
        &record.intent,
        &record.preview
    ));
    for mutation in 0..10 {
        let mut changed = record.clone();
        match mutation {
            0 => changed.attempt_id = SchemaAlterAttemptId::new(),
            1 => changed.target.identity.database_oid += 1,
            2 => changed.target.identity.schema_oid += 1,
            3 => changed.target.namespace_xmin.push('1'),
            4 => changed.target.namespace_ctid = "(2,4)".into(),
            5 => changed.target.comment = None,
            6 => changed.intent = SchemaAlterIntent::SetComment { comment: None },
            7 => changed.preview.statement_timeout_ms = None,
            8 => changed.preview.operation_timeout_ms += 1,
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
    for intent in intents() {
        let record = journal(intent);
        let loaded = decode(&encode(workspace(record.clone())).unwrap()).unwrap();
        assert_eq!(loaded.documents[0].schema_alter.as_ref(), Some(&record));
    }
}

#[test]
fn schema_alter_legacy_workspaces_accept_absence_and_reject_even_null_new_field() {
    let legacy = snapshot();
    let mut value: serde_json::Value =
        serde_json::from_str(&encode(legacy.clone()).unwrap()).unwrap();
    for version in 1..=13 {
        value["version"] = version.into();
        assert_eq!(decode(&value.to_string()).unwrap(), legacy);
        value["snapshot"]["documents"][0]["schemaAlter"] = serde_json::Value::Null;
        assert!(decode(&value.to_string()).is_err(), "version {version}");
        value["snapshot"]["documents"][0]
            .as_object_mut()
            .unwrap()
            .remove("schemaAlter");
    }
}

#[tokio::test]
async fn schema_alter_capacity_and_other_objects_journals_refuse_without_overwrite() {
    let pool = pool().await;
    let valid = workspace(journal(SchemaAlterIntent::SetComment {
        comment: Some("comment".into()),
    }));
    let revision = save(&pool, None, valid.clone()).await.unwrap();
    let original = raw(&pool).await;
    let mut oversized = valid.clone();
    oversized.documents[0]
        .schema_alter
        .as_mut()
        .unwrap()
        .preview
        .sql
        .reserve(64 * 1024);
    assert!(save(&pool, Some(revision.clone()), oversized)
        .await
        .is_err());
    assert_eq!(raw(&pool).await, original);
    let mut conflicting = valid.clone();
    conflicting.documents[0].schema_changes = Some(WorkspaceSchemaChanges {
        attempt_id: SchemaAlterAttemptId::new(),
        intent: crate::backend::schema_ddl::CreateSchemaIntent::new("new_schema".into(), None)
            .unwrap(),
        apply_state: WorkspaceApplyState::Staged,
    });
    assert!(save(&pool, Some(revision.clone()), conflicting)
        .await
        .is_err());
    assert_eq!(raw(&pool).await, original);
    let mut both = valid;
    both.documents[0].table_ddl = Some(
        serde_json::from_value(serde_json::json!({
            "attemptId": SchemaAlterAttemptId::new(),
            "target": {"identity": {"database_oid": 1, "relation_oid": 2}, "schemaOid": 3,
                "schema": "s", "namespaceXmin": "1", "namespaceCtid": "(0,1)", "table": "t",
                "column": null, "comment": null},
            "intent": {"kind": "setComment", "comment": null},
            "preview": {"sql": "COMMENT ON TABLE \"s\".\"t\" IS NULL;",
                "summary": "Clear comment on TABLE \"s\".\"t\"",
                "statementTimeoutMs": null, "operationTimeoutMs": 30000},
            "applyState": "staged"
        }))
        .unwrap(),
    );
    both.documents[0]
        .table_ddl
        .as_ref()
        .unwrap()
        .validate()
        .unwrap();
    assert!(save(&pool, Some(revision), both).await.is_err());
    assert_eq!(raw(&pool).await, original);
}
