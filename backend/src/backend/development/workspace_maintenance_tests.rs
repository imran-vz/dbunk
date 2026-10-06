use super::tests::{pool, raw, seed, snapshot};
use super::*;
use crate::backend::maintenance::{MaintenanceIntent, MaintenanceRelationKind, MaintenanceTarget};

fn target() -> MaintenanceTarget {
    MaintenanceTarget {
        database_oid: 12,
        database: "owned".into(),
        namespace_oid: 34,
        schema: "Exact \"資料".into(),
        relation_oid: 56,
        name: "rows".into(),
        kind: MaintenanceRelationKind::Table,
    }
}
fn journal() -> WorkspaceMaintenance {
    let target = target();
    let preview =
        crate::backend::maintenance::preview(&target, MaintenanceIntent::Vacuum, Some(0)).unwrap();
    WorkspaceMaintenance {
        attempt_id: uuid::Uuid::new_v4().to_string(),
        action: WorkspaceMaintenanceAction::Vacuum,
        database_oid: target.database_oid(),
        database: target.database().into(),
        namespace_oid: target.namespace_oid(),
        schema: target.schema().into(),
        relation_oid: target.relation_oid(),
        name: target.name().into(),
        kind: WorkspaceMaintenanceKind::Table,
        sql: preview.sql,
        potentially_partial: true,
        operation_timeout_ms: preview.operation_timeout_ms,
        statement_timeout_ms: preview.statement_timeout_ms,
        state: WorkspaceMaintenanceState::OutcomeUnknown,
    }
}
#[tokio::test]
async fn maintenance_recovery_preserves_exact_identity_and_partial_outcomes_without_replay() {
    let pool = pool().await;
    let mut input = snapshot();
    let document = &mut input.documents[0];
    document.sql.clear();
    document.selection = WorkspaceSelection::default();
    document.tool = Some(WorkspaceTool::Objects);
    document.maintenance = Some(journal());
    let mut revision = None;
    for state in [
        WorkspaceMaintenanceState::Staged,
        WorkspaceMaintenanceState::OutcomeUnknown,
        WorkspaceMaintenanceState::EffectsPossible,
    ] {
        input.documents[0].maintenance.as_mut().unwrap().state = state;
        revision = Some(save(&pool, revision, input.clone()).await.unwrap());
        let original = raw(&pool).await;
        assert_eq!(load(&pool).await.unwrap().snapshot, Some(input.clone()));
        assert_eq!(raw(&pool).await, original);
        let value: serde_json::Value = serde_json::from_str(&original).unwrap();
        assert_eq!(value["version"], 16);
        assert_eq!(
            value["snapshot"]["documents"][0]["maintenance"]["state"],
            match state {
                WorkspaceMaintenanceState::Staged => "staged",
                WorkspaceMaintenanceState::OutcomeUnknown => "outcomeUnknown",
                WorkspaceMaintenanceState::EffectsPossible => "effectsPossible",
            }
        );
        assert!(!original.contains("confirmation"));
    }
    let base: serde_json::Value = serde_json::from_str(&raw(&pool).await).unwrap();
    for defect in 0..12 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 8.into(),
            1 => value["version"] = 17.into(),
            2 => value["snapshot"]["documents"][0]["connectionId"] = serde_json::Value::Null,
            3 => value["snapshot"]["documents"][0]["tool"] = "administration".into(),
            4 => value["snapshot"]["documents"][0]["maintenance"]["confirmation"] = true.into(),
            5 => value["snapshot"]["documents"][0]["maintenance"]["attemptId"] = "invalid".into(),
            6 => value["snapshot"]["documents"][0]["maintenance"]["relationOid"] = 0.into(),
            7 => {
                value["snapshot"]["documents"][0]["maintenance"]["schema"] = "界".repeat(22).into()
            }
            8 => {
                value["snapshot"]["documents"][0]["maintenance"]["sql"] =
                    "VACUUM wrong_target".into()
            }
            9 => {
                value["snapshot"]["documents"][0]["maintenance"]["potentiallyPartial"] =
                    false.into()
            }
            10 => value["snapshot"]["documents"][0]["maintenance"]["operationTimeoutMs"] = 0.into(),
            _ => value["snapshot"]["documents"][0]["maintenance"]["state"] = "replay".into(),
        }
        let original = value.to_string();
        seed(&pool, &original).await;
        assert!(load(&pool).await.is_err());
        assert!(
            save(&pool, Some(WorkspaceRevision("seed".into())), input.clone())
                .await
                .is_err()
        );
        assert_eq!(raw(&pool).await, original);
    }
    let mut old = base;
    old["version"] = 8.into();
    old["snapshot"]["documents"][0]
        .as_object_mut()
        .unwrap()
        .remove("maintenance");
    assert!(decode(&old.to_string()).unwrap().documents[0]
        .maintenance
        .is_none());
}
#[test]
fn maintenance_receipts_require_exact_target_action_sql_and_deadlines() {
    let journal = journal();
    let target = target();
    let preview =
        crate::backend::maintenance::preview(&target, MaintenanceIntent::Vacuum, Some(0)).unwrap();
    assert!(journal.matches(
        &journal.attempt_id,
        MaintenanceIntent::Vacuum,
        &target,
        &preview
    ));
    for defect in 0..13 {
        let mut changed = journal.clone();
        match defect {
            0 => changed.attempt_id = uuid::Uuid::new_v4().to_string(),
            1 => changed.action = WorkspaceMaintenanceAction::Analyze,
            2 => changed.database_oid += 1,
            3 => changed.database.push('x'),
            4 => changed.namespace_oid += 1,
            5 => changed.schema.push('x'),
            6 => changed.relation_oid += 1,
            7 => changed.name.push('x'),
            8 => changed.kind = WorkspaceMaintenanceKind::PartitionedTable,
            9 => changed.sql.push(' '),
            10 => changed.potentially_partial = false,
            11 => changed.operation_timeout_ms += 1,
            _ => changed.statement_timeout_ms = None,
        }
        assert!(
            !changed.matches(
                &journal.attempt_id,
                MaintenanceIntent::Vacuum,
                &target,
                &preview
            ),
            "defect {defect}"
        );
    }
}
