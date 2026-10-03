use super::*;
use dbunk_lib::backend::{WorkspaceApplyState, WorkspaceStagedChange};

fn state() -> WorkspaceTableState {
    WorkspaceTableState {
        schema: "s".into(),
        table: "t".into(),
        filters: vec![],
        sort: vec![],
        page_size: 100,
        draft: None,
    }
}
fn saved() -> WorkspaceMutationDraft {
    let mut text = String::with_capacity(1024);
    text.push_str("exact\0雪");
    WorkspaceMutationDraft {
        apply_state: WorkspaceApplyState::OutcomeUnknown,
        changes: vec![WorkspaceStagedChange {
            id: uuid::Uuid::new_v4().to_string(),
            included: false,
            identity_kind: None,
            originals: vec![],
            operation: MutationOp::Insert {
                table: MutationTable {
                    schema: "s".into(),
                    table: "t".into(),
                },
                values: vec![MutationValue {
                    column: "value".into(),
                    value: Some(text),
                }],
            },
        }],
    }
}

#[test]
fn unadmitted_recovery_keeps_moved_intent_and_retries_without_losing_unknown_state() {
    let saved = saved();
    let expected = serde_json::to_string(&saved).unwrap();
    let retained = MutationDraft::recovery_bytes(&saved);
    assert!(
        retained > 1024,
        "capacity, not merely text length, is charged"
    );
    let budget = Rc::new(Cell::new(128 * 1024 * 1024 - retained));
    let mut view = TableChanges::new(&state(), Some(saved), budget.clone());
    assert_eq!(budget.get(), 128 * 1024 * 1024);
    assert!(view.draft.is_none());
    assert_eq!(
        serde_json::to_string(&view.snapshot().unwrap()).unwrap(),
        expected
    );
    assert_eq!(view.work_bytes, 0);
    // Other result owners release capacity. Retry decodes under a transient lease.
    budget.set(retained);
    assert!(view.restore_intent());
    assert!(view.unrestored.is_none());
    assert!(view.draft.as_ref().unwrap().outcome_unknown());
    assert_eq!(
        serde_json::to_string(&view.snapshot().unwrap()).unwrap(),
        expected
    );
    assert_eq!(budget.get(), view.draft_bytes);
    assert_eq!(view.work_bytes, 0);
    drop(view);
    assert_eq!(budget.get(), 0);
}

#[test]
fn malformed_recovery_and_refused_work_never_drop_or_double_charge_intent() {
    let mut saved = saved();
    saved.changes[0].id = "invalid id".into();
    let expected = serde_json::to_string(&saved).unwrap();
    let budget = Rc::new(Cell::new(0));
    let mut view = TableChanges::new(&state(), Some(saved), budget.clone());
    let retained = budget.get();
    assert!(view.draft.is_none() && view.unrestored.is_some());
    assert_eq!(view.work_bytes, 0);
    assert!(!view.restore_intent());
    assert_eq!(budget.get(), retained);
    assert_eq!(
        serde_json::to_string(&view.snapshot().unwrap()).unwrap(),
        expected
    );
    budget.set(128 * 1024 * 1024);
    assert!(!view.admit_work());
    assert_eq!(view.work_bytes, 0);
    budget.set(retained);
    drop(view);
    assert_eq!(budget.get(), 0);
}

#[test]
fn idle_work_releases_its_allowance_without_charging_every_empty_table() {
    let budget = Rc::new(Cell::new(123));
    let mut view = TableChanges::new(&state(), None, budget.clone());
    assert_eq!(budget.get(), 123);
    assert!(view.admit_work());
    assert_eq!(budget.get(), 123 + WORK_BYTES);
    assert!(view.admit_work());
    assert_eq!(budget.get(), 123 + WORK_BYTES);
    view.finish_work();
    assert_eq!(budget.get(), 123);
}

#[test]
fn query_recovery_measures_exact_journal_and_keeps_original_source_and_unknown_state() {
    let budget = Rc::new(Cell::new(0));
    let provenance = crate::query_result::Provenance::restore(
        dbunk_lib::backend::QueryMutationSource::new(
            "SELECT id, value FROM s.t WHERE value = :value".into(),
            true,
        )
        .unwrap(),
        budget.clone(),
    );
    let original = MutationValue {
        column: "value".into(),
        value: Some("exact\0雪".into()),
    };
    let key = MutationValue {
        column: "id".into(),
        value: Some("1".into()),
    };
    let saved = WorkspaceMutationDraft {
        apply_state: WorkspaceApplyState::OutcomeUnknown,
        changes: vec![WorkspaceStagedChange {
            id: uuid::Uuid::new_v4().to_string(),
            included: false,
            identity_kind: Some(MutationIdentityKind::PrimaryKey),
            originals: vec![key.clone(), original.clone()],
            operation: MutationOp::Update {
                table: MutationTable {
                    schema: "s".into(),
                    table: "t".into(),
                },
                identity: vec![key],
                guards: vec![original],
                set: vec![MutationValue {
                    column: "value".into(),
                    value: None,
                }],
            },
        }],
    };
    let mut view = TableChanges::new_query(provenance, None, Some(saved), budget.clone());
    assert!(view.has_intent());
    let snapshot = view.query_snapshot().unwrap();
    snapshot.validate().unwrap();
    assert_eq!(
        view.query_snapshot_bytes(),
        crate::results::encoded_size(&snapshot)
    );
    assert!(view.draft.as_ref().unwrap().outcome_unknown());
    assert!(snapshot.source.statement_sql().contains("$1"));
    view.draft = None;
    view.finish_work();
    assert!(view.query_snapshot().is_none());
    assert_eq!(view.query_snapshot_bytes(), 0);
    drop(view);
    assert_eq!(budget.get(), 0);
}
