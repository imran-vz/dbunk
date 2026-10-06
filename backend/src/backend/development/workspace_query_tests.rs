use super::tests::{pool, raw, seed, snapshot, table_state};
use super::*;
use crate::backend::QueryMutationSource;
use crate::result_mutation::protocol::{MutationIdentityKind, MutationOp};

fn query() -> WorkspaceQueryChanges {
    let mut draft = table_state().draft.unwrap();
    let mut second = draft.changes[0].clone();
    second.id = uuid::Uuid::new_v4().to_string();
    second.included = false;
    if let MutationOp::Update { table, .. } = &mut second.operation {
        table.schema = "other schema".into();
        table.table = "joined_table".into();
    }
    draft.changes.push(second);
    WorkspaceQueryChanges {
        source: QueryMutationSource::new(
            "SELECT a.*, b.* FROM public.a JOIN public.b USING(id) WHERE a.id=:id".into(),
            true,
        )
        .unwrap(),
        draft,
    }
}

#[tokio::test]
async fn multi_origin_unknown_outcome_journal_preserves_source_separate_from_editor() {
    let pool = pool().await;
    let mut input = snapshot();
    input.documents[0].query_changes = Some(query());
    input.documents[0].sql = "SELECT 'new unsent editor text'".into();
    let expected = validate(input.clone()).unwrap();
    save(&pool, None, input).await.unwrap();
    let encoded = raw(&pool).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&encoded).unwrap()["version"],
        16
    );
    assert_eq!(load(&pool).await.unwrap().snapshot, Some(expected));
    assert!(!encoded.contains("analysisId"));
    assert!(!encoded.contains("boundValues"));
}

#[test]
fn query_operations_require_updates_with_original_identity_and_exact_guards() {
    assert!(query().validate().is_ok());
    for defect in 0..7 {
        let mut query = query();
        let change = &mut query.draft.changes[0];
        if let MutationOp::Update {
            table,
            identity,
            guards,
            set,
        } = &mut change.operation
        {
            match defect {
                0 => guards[0].value = Some("forged".into()),
                1 => identity[0].value = Some("new key".into()),
                2 => change.identity_kind = Some(MutationIdentityKind::None),
                3 => set[0].column = "not projected".into(),
                4 => table.schema.clear(),
                5 => {
                    change.operation = MutationOp::Insert {
                        table: table.clone(),
                        values: vec![],
                    }
                }
                _ => {
                    change.operation = MutationOp::Delete {
                        table: table.clone(),
                        identity: identity.clone(),
                        guards: guards.clone(),
                    }
                }
            }
        }
        assert!(query.validate().is_err(), "defect {defect}");
    }
    let mut query = query();
    query.draft.changes[1].id = query.draft.changes[0].id.clone();
    assert!(query.validate().is_err());
}

#[test]
fn virtual_key_requires_full_original_row_and_limits_cover_escape_expansion() {
    let mut query = query();
    let change = &mut query.draft.changes[0];
    change.identity_kind = Some(MutationIdentityKind::VirtualKey);
    assert!(query.validate().is_err());
    let change = &mut query.draft.changes[0];
    if let MutationOp::Update { guards, .. } = &mut change.operation {
        *guards = change.originals.clone();
    }
    assert!(query.validate().is_ok());
    if let MutationOp::Update { set, .. } = &mut query.draft.changes[0].operation {
        set[0].value = Some("\0".repeat(WORKSPACE_MUTATION_MAX_BYTES / 6));
    }
    assert_eq!(query.validate(), Err(WorkspaceError::TooLarge));
    let mut query = self::query();
    let change = query.draft.changes[0].clone();
    query.draft.changes = (0..129)
        .map(|_| {
            let mut copy = change.clone();
            copy.id = uuid::Uuid::new_v4().to_string();
            copy
        })
        .collect();
    assert_eq!(query.validate(), Err(WorkspaceError::InvalidSnapshot));
}

#[tokio::test]
async fn invalid_versions_provenance_and_extra_intent_are_preserved_without_overwrite() {
    let pool = pool().await;
    let mut input = snapshot();
    input.documents[0].query_changes = Some(query());
    let base: serde_json::Value = serde_json::from_str(&encode(input).unwrap()).unwrap();
    for defect in 0..5 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 1.into(),
            1 => value["version"] = 2.into(),
            2 => {
                value["snapshot"]["documents"][0]["queryChanges"]["source"]["statementSql"] =
                    "SELECT changed".into()
            }
            3 => {
                value["snapshot"]["documents"][0]["queryChanges"]["draft"]["changes"][0]
                    ["operation"]["futureAuthority"] = true.into()
            }
            _ => value["version"] = 17.into(),
        }
        let encoded = serde_json::to_string(&value).unwrap();
        seed(&pool, &encoded).await;
        assert!(load(&pool).await.is_err());
        assert!(
            save(&pool, Some(WorkspaceRevision("seed".into())), snapshot())
                .await
                .is_err()
        );
        assert_eq!(raw(&pool).await, encoded);
    }
}

#[tokio::test]
async fn v2_load_does_not_write_and_explicit_save_upgrades_to_v16() {
    let pool = pool().await;
    let mut input = snapshot();
    input.documents[0].table = Some(table_state());
    let mut value: serde_json::Value = serde_json::from_str(&encode(input).unwrap()).unwrap();
    value["version"] = 2.into();
    let encoded = serde_json::to_string(&value).unwrap();
    seed(&pool, &encoded).await;
    let loaded = load(&pool).await.unwrap();
    assert_eq!(raw(&pool).await, encoded);
    save(&pool, loaded.revision, loaded.snapshot.unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw(&pool).await).unwrap()["version"],
        16
    );
}

#[test]
fn query_journal_excludes_table_tools_and_obeys_smaller_workspace_budget() {
    for tool in [false, true] {
        let mut input = snapshot();
        input.documents[0].query_changes = Some(query());
        if tool {
            input.documents[0].tool = Some(WorkspaceTool::Objects);
            input.documents[0].sql.clear();
        } else {
            input.documents[0].table = Some(table_state());
        }
        assert_eq!(validate(input), Err(WorkspaceError::InvalidSnapshot));
    }
    let mut input = snapshot();
    let mut query = query();
    if let MutationOp::Update { set, .. } = &mut query.draft.changes[0].operation {
        set[0].value = Some("\0".repeat(NATIVE_WORKSPACE_MAX_BYTES / 6));
    }
    assert!(query.validate().is_ok());
    input.documents[0].query_changes = Some(query);
    assert_eq!(encode(input), Err(WorkspaceError::TooLarge));
}

#[test]
fn query_nullable_virtual_identity_refused_without_changing_table_contract() {
    let mut table = table_state();
    let change = &mut table.draft.as_mut().unwrap().changes[0];
    change.identity_kind = Some(MutationIdentityKind::VirtualKey);
    change.originals[0].value = None;
    if let MutationOp::Update {
        identity, guards, ..
    } = &mut change.operation
    {
        identity[0].value = None;
        *guards = change.originals.clone();
    }
    assert!(table.validate().is_ok());
    let query = WorkspaceQueryChanges {
        source: QueryMutationSource::new("SELECT * FROM public.example".into(), false).unwrap(),
        draft: table.draft.unwrap(),
    };
    assert_eq!(query.validate(), Err(WorkspaceError::InvalidSnapshot));
}
