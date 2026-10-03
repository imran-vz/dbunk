use super::*;
use dbunk_lib::backend::table_ddl::{
    TABLE_DDL_OPERATION_TIMEOUT_MS, TableDdlAttemptId, TableDdlColumn, TableDdlFailure,
    TableDdlPreview, TableIdentity,
};
fn journal() -> WorkspaceTableDdl {
    WorkspaceTableDdl {
        attempt_id: TableDdlAttemptId::new(),
        target: TableDdlDescription {
            identity: TableIdentity {
                database_oid: 1,
                relation_oid: 2,
            },
            schema_oid: 3,
            schema: "Exact".into(),
            namespace_xmin: "42".into(),
            namespace_ctid: "(0,1)".into(),
            table: " rows ".into(),
            column: Some(TableDdlColumn {
                attnum: 7,
                name: "値".into(),
            }),
            comment: Some(String::new()),
        },
        intent: TableDdlIntent::SetComment { comment: None },
        preview: TableDdlPreview {
            sql: "COMMENT ON COLUMN \"Exact\".\" rows \".\"値\" IS NULL;".into(),
            summary: "Clear comment on COLUMN \"Exact\".\" rows \".\"値\"".into(),
            statement_timeout_ms: None,
            operation_timeout_ms: TABLE_DDL_OPERATION_TIMEOUT_MS,
        },
        apply_state: WorkspaceApplyState::Staged,
    }
}
fn receipt(j: &WorkspaceTableDdl, outcome: TableDdlOutcome) -> TableDdlReceipt {
    TableDdlReceipt {
        attempt_id: j.attempt_id.clone(),
        connection_id: "owned".into(),
        target: j.target.clone(),
        intent: j.intent.clone(),
        outcome,
    }
}
#[test]
fn table_ddl_exact_selection_checks_column_ordinal_and_relation_identity() {
    let j = journal();
    j.validate().unwrap();
    let selection = Selection::new(j.target.request(), Some(7)).unwrap();
    assert!(selection.matches(&j.target));
    let mut wrong = j.target.clone();
    wrong.column.as_mut().unwrap().attnum = 8;
    assert!(!selection.matches(&wrong));
    wrong = j.target.clone();
    wrong.identity.relation_oid += 1;
    assert!(!selection.matches(&wrong));
    wrong = j.target.clone();
    wrong.table = "rows".into();
    assert!(!selection.matches(&wrong));
    assert!(Selection::new(j.target.request(), None).is_err());
}
#[test]
fn table_ddl_mismatched_receipt_never_clears_unknown_recovery() {
    let j = journal();
    for variation in 0..5 {
        let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
        r.mark_unknown();
        let mut got = receipt(&j, TableDdlOutcome::Applied { runtime_ms: 1 });
        match variation {
            0 => got.connection_id = "other".into(),
            1 => got.attempt_id = TableDdlAttemptId::new(),
            2 => got.target.identity.relation_oid += 1,
            3 => got.target.comment = None,
            _ => {
                got.intent = TableDdlIntent::SetComment {
                    comment: Some(String::new()),
                }
            }
        }
        assert!(matches!(r.receipt(&got), Settlement::Unknown));
        assert!(r.unknown());
        assert_eq!(r.journal().unwrap().attempt_id, j.attempt_id);
    }
}
#[test]
fn table_ddl_outcomes_keep_exact_intent_until_known_success_or_explicit_reconcile() {
    let j = journal();
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    r.mark_unknown();
    assert!(matches!(
        r.receipt(&receipt(
            &j,
            TableDdlOutcome::RolledBack {
                reason: TableDdlFailure::TargetChanged
            }
        )),
        Settlement::Staged
    ));
    assert_eq!(r.journal().unwrap().intent, j.intent);
    r.mark_unknown();
    r.submission_error(&TableDdlError::OutcomeUnavailable);
    assert!(r.unknown());
    assert!(!r.discard(false));
    assert!(r.discard(true));
    assert!(r.journal().is_none());
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    r.mark_unknown();
    assert!(matches!(
        r.receipt(&receipt(&j, TableDdlOutcome::Applied { runtime_ms: 0 })),
        Settlement::Applied
    ));
    assert!(r.journal().is_none());
}
#[test]
fn table_ddl_cancelled_save_and_confirmation_keep_no_late_dispatch_authority() {
    use crate::apply_flow::ApplyFlow;
    let j = journal();
    let mut r = Recovery::new("owned".into(), Some(j)).unwrap();
    let mut flow = ApplyFlow::new(11, "live review");
    r.mark_unknown();
    assert!(flow.cancel_before_dispatch());
    r.not_sent();
    assert_eq!(flow.saved(11), None);
    assert!(!r.unknown());
    let mut flow = ApplyFlow::new(12, "new observed review");
    r.mark_unknown();
    assert_eq!(flow.saved(12), Some("new observed review"));
    flow.needs_confirmation("same exact confirmation");
    r.not_sent();
    assert!(flow.confirm(13));
    r.mark_unknown();
    assert_eq!(flow.saved(12), None);
    assert!(flow.cancel_before_dispatch());
    r.not_sent();
    assert_eq!(flow.saved(13), None);
    assert!(r.journal().is_some());
}
#[test]
fn table_ddl_comment_null_empty_and_exact_identifier_bytes_remain_distinct() {
    assert_ne!(
        intent(false, true, String::new()).unwrap(),
        intent(false, false, String::new()).unwrap()
    );
    assert_eq!(
        intent(true, false, " \t".into()).unwrap(),
        TableDdlIntent::Rename {
            new_name: " \t".into()
        }
    );
    assert!(intent(true, false, "é".repeat(32)).is_err());
    assert!(intent(false, false, "x".repeat(4097)).is_err());
    let mut oversized = String::with_capacity(1024 * 1024);
    oversized.push('x');
    assert!(intent(false, false, oversized).is_err());
}
#[test]
fn table_ddl_presentation_refuses_atomically_and_retains_old_allowance() {
    let budget = Rc::new(Cell::new(SHARED_BYTES - ALLOWANCE));
    let old = Lease::admit(budget.clone()).unwrap();
    assert_eq!(budget.get(), SHARED_BYTES);
    assert!(Lease::admit(budget.clone()).is_none());
    assert_eq!(budget.get(), SHARED_BYTES);
    drop(old);
    assert_eq!(budget.get(), SHARED_BYTES - ALLOWANCE);
    let next = Lease::admit(budget.clone()).unwrap();
    drop(next);
    assert_eq!(budget.get(), SHARED_BYTES - ALLOWANCE);
}

#[test]
fn table_ddl_invalid_restored_journal_is_refused_without_consuming_caller_record() {
    let mut original = journal();
    original.preview.sql.push_str(" SELECT 1;");
    assert!(crate::table_ddl_view::TableDdlView::prepare("owned", None, Some(&original)).is_err());
    assert!(original.preview.sql.ends_with(" SELECT 1;"));
    let valid = journal();
    let restored = Recovery::new("owned".into(), Some(valid.clone())).unwrap();
    assert!(!restored.unknown());
    assert!(
        restored
            .selection()
            .unwrap()
            .unwrap()
            .matches(&valid.target)
    );
}
