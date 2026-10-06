use super::tests::{pool, raw, seed, snapshot};
use super::*;
use crate::backend::object_ddl::{
    render_preview,
    tests::{atomic_case, mixed_case},
    ObjectDdlAttemptId,
};

fn journal(mixed: bool) -> WorkspaceObjectDdl {
    let (target, operations) = if mixed { mixed_case() } else { atomic_case() };
    let preview = render_preview(&target, &operations, Some(0), mixed).unwrap();
    WorkspaceObjectDdl {
        attempt_id: ObjectDdlAttemptId::new(),
        target,
        operations,
        preview,
        apply_state: WorkspaceApplyState::Staged,
    }
}
fn workspace(record: WorkspaceObjectDdl) -> WorkspaceSnapshot {
    let mut value = snapshot();
    let doc = &mut value.documents[0];
    doc.sql.clear();
    doc.selection = WorkspaceSelection::default();
    doc.tool = Some(WorkspaceTool::Objects);
    doc.object_ddl = Some(record);
    value
}

#[tokio::test]
async fn object_ddl_exact_restore_is_read_only_and_written_as_version_15() {
    let pool = pool().await;
    let mut revision = None;
    for mixed in [false, true] {
        for state in [
            WorkspaceApplyState::Staged,
            WorkspaceApplyState::OutcomeUnknown,
        ] {
            let mut record = journal(mixed);
            record.apply_state = state;
            record.validate().unwrap();
            let expected = workspace(record);
            revision = Some(save(&pool, revision, expected.clone()).await.unwrap());
            let before = raw(&pool).await;
            assert_eq!(load(&pool).await.unwrap().snapshot, Some(expected));
            assert_eq!(raw(&pool).await, before, "loading never rewrites");
            let value: serde_json::Value = serde_json::from_str(&before).unwrap();
            assert_eq!(value["version"], 16);
            let record = &value["snapshot"]["documents"][0]["objectDdl"];
            assert_eq!(record["target"]["databaseOid"], 16384);
            assert!(record.get("impacts").is_none(), "impact is never journaled");
            assert!(record.get("confirmation").is_none());
        }
    }
}

#[tokio::test]
async fn object_ddl_invalid_recovery_cannot_overwrite_original_record() {
    let pool = pool().await;
    let valid = workspace(journal(true));
    let base: serde_json::Value = serde_json::from_str(&encode(valid.clone()).unwrap()).unwrap();
    for defect in 0..16 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 14.into(),
            1 => value["version"] = 17.into(),
            2 => value["snapshot"]["documents"][0]["connectionId"] = serde_json::Value::Null,
            3 => value["snapshot"]["documents"][0]["tool"] = "administration".into(),
            _ => {
                let record = &mut value["snapshot"]["documents"][0]["objectDdl"];
                match defect {
                    4 => record["confirmation"] = true.into(),
                    5 => record["attemptId"] = "invalid".into(),
                    6 => record["target"]["databaseOid"] = 0.into(),
                    7 => record["target"]["claims"][0][1]["name"] = "elsewhere".into(),
                    8 => record["operations"][0]["sqlBody"] = "SELECT 2".into(),
                    9 => record["preview"]["statements"][0]["sql"] = "DROP TABLE x".into(),
                    10 => record["preview"]["groups"][1]["statement"] = 2.into(),
                    11 => record["preview"]["operationTimeoutMs"] = 0.into(),
                    12 => record["applyState"] = "replay".into(),
                    13 => record["target"]["claims"][3][0]["address"]["rowVersion"] = "x".into(),
                    14 => record["preview"]["operationDigest"] = "fnv64:0".into(),
                    _ => record["operations"][0]["sqlBody"] = "x".repeat(17 * 1024).into(),
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
fn object_ddl_exact_matching_covers_attempt_identity_operations_and_preview() {
    let record = journal(false);
    assert!(record.matches(
        &record.attempt_id,
        &record.target,
        &record.operations,
        &record.preview
    ));
    for mutation in 0..6 {
        let mut changed = record.clone();
        match mutation {
            0 => changed.attempt_id = ObjectDdlAttemptId::new(),
            1 => changed.target.database_oid += 1,
            2 => {
                if let crate::backend::object_ddl::ObjectDdlClaim::Existing { address, .. } =
                    &mut changed.target.claims[0][0]
                {
                    address.object_oid += 1;
                }
            }
            3 => changed.operations.reverse(),
            4 => changed.preview.statement_timeout_ms = None,
            _ => changed.preview.statements[1].sql.push(' '),
        }
        assert!(
            !record.matches(
                &changed.attempt_id,
                &changed.target,
                &changed.operations,
                &changed.preview
            ),
            "mutation {mutation}"
        );
    }
    let loaded = decode(&encode(workspace(record.clone())).unwrap()).unwrap();
    assert_eq!(loaded.documents[0].object_ddl.as_ref(), Some(&record));
}

#[test]
fn object_ddl_legacy_workspaces_accept_absence_and_reject_even_null_new_field() {
    let legacy = snapshot();
    let mut value: serde_json::Value =
        serde_json::from_str(&encode(legacy.clone()).unwrap()).unwrap();
    for version in 1..=14 {
        value["version"] = version.into();
        assert_eq!(decode(&value.to_string()).unwrap(), legacy);
        value["snapshot"]["documents"][0]["objectDdl"] = serde_json::Value::Null;
        assert!(decode(&value.to_string()).is_err(), "version {version}");
        value["snapshot"]["documents"][0]
            .as_object_mut()
            .unwrap()
            .remove("objectDdl");
    }
}

#[tokio::test]
async fn object_ddl_capacity_and_other_objects_journals_refuse_without_overwrite() {
    let pool = pool().await;
    let valid = workspace(journal(false));
    let revision = save(&pool, None, valid.clone()).await.unwrap();
    let original = raw(&pool).await;
    let mut oversized = valid.clone();
    oversized.documents[0]
        .object_ddl
        .as_mut()
        .unwrap()
        .preview
        .statements[0]
        .sql
        .reserve(256 * 1024);
    assert!(save(&pool, Some(revision.clone()), oversized)
        .await
        .is_err());
    assert_eq!(raw(&pool).await, original);
    let mut conflicting = valid.clone();
    conflicting.documents[0].schema_changes = Some(WorkspaceSchemaChanges {
        attempt_id: ObjectDdlAttemptId::new(),
        intent: crate::backend::schema_ddl::CreateSchemaIntent::new("new_schema".into(), None)
            .unwrap(),
        apply_state: WorkspaceApplyState::Staged,
    });
    assert!(save(&pool, Some(revision.clone()), conflicting)
        .await
        .is_err());
    assert_eq!(raw(&pool).await, original);
    let mut unbound = valid;
    unbound.documents[0].connection_id = None;
    assert!(save(&pool, Some(revision), unbound).await.is_err());
    assert_eq!(raw(&pool).await, original);
}
