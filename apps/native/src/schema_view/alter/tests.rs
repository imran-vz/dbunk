use super::model::*;
use super::*;
use dbunk_lib::backend::WorkspaceApplyState;
fn target() -> SchemaAlterDescription {
    SchemaAlterDescription {
        identity: SchemaIdentity {
            database_oid: 1,
            schema_oid: 2,
        },
        schema: " Exact \"資料".into(),
        namespace_xmin: "42".into(),
        namespace_ctid: "(0,1)".into(),
        comment: Some(String::new()),
    }
}
fn journal() -> WorkspaceSchemaAlter {
    let target = target();
    WorkspaceSchemaAlter {
        attempt_id: SchemaAlterAttemptId::new(),
        intent: SchemaAlterIntent::SetComment { comment: None },
        preview: SchemaAlterPreview {
            sql: "COMMENT ON SCHEMA \" Exact \"\"資料\" IS NULL;".into(),
            summary: "Clear comment on SCHEMA \" Exact \"\"資料\"".into(),
            statement_timeout_ms: None,
            operation_timeout_ms: SCHEMA_ALTER_OPERATION_TIMEOUT_MS,
        },
        target,
        apply_state: WorkspaceApplyState::Staged,
    }
}
fn receipt(j: &WorkspaceSchemaAlter, outcome: SchemaAlterOutcome) -> SchemaAlterReceipt {
    SchemaAlterReceipt {
        attempt_id: j.attempt_id.clone(),
        connection_id: "owned".into(),
        target: j.target.clone(),
        intent: j.intent.clone(),
        outcome,
    }
}
#[test]
fn schema_alter_selection_pins_first_observed_oid_and_refuses_recreated_schema() {
    let mut selection = Selection::new(SchemaAlterRequest {
        schema: " Exact \"資料".into(),
        expected: None,
    })
    .unwrap();
    assert!(!selection.pinned());
    let observed = target();
    assert!(selection.pin(&observed));
    assert!(selection.pinned());
    assert_eq!(selection.request().expected, Some(observed.identity));
    // Same name, different OID: dropped and recreated between observations.
    let mut recreated = observed.clone();
    recreated.identity.schema_oid = 9;
    assert!(!selection.matches(&recreated));
    assert!(!selection.pin(&recreated));
    assert_eq!(selection.request().expected, Some(observed.identity));
    // Exact bytes: no trimming or case folding of quoted identifiers.
    let mut renamed = observed.clone();
    renamed.schema = "Exact \"資料".into();
    assert!(!selection.matches(&renamed));
    assert!(
        Selection::new(SchemaAlterRequest {
            schema: String::new(),
            expected: None
        })
        .is_err()
    );
}
#[test]
fn schema_alter_mismatched_receipt_never_clears_unknown_recovery() {
    let j = journal();
    j.validate().unwrap();
    for variation in 0..5 {
        let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
        r.mark_unknown();
        let mut got = receipt(&j, SchemaAlterOutcome::Applied { runtime_ms: 1 });
        match variation {
            0 => got.connection_id = "other".into(),
            1 => got.attempt_id = SchemaAlterAttemptId::new(),
            2 => got.target.identity.schema_oid += 1,
            3 => got.target.comment = None,
            _ => {
                got.intent = SchemaAlterIntent::SetComment {
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
fn schema_alter_outcomes_keep_exact_intent_until_known_success_or_explicit_reconcile() {
    let j = journal();
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    r.mark_unknown();
    assert!(matches!(
        r.receipt(&receipt(
            &j,
            SchemaAlterOutcome::RolledBack {
                reason: SchemaAlterFailure::TargetChanged
            }
        )),
        Settlement::Staged
    ));
    assert_eq!(r.journal().unwrap().intent, j.intent);
    assert!(!r.unknown());
    r.mark_unknown();
    // Lost delivery is unknown: never treated as not-sent and never retried.
    r.submission_error(&SchemaAlterError::OutcomeUnavailable);
    assert!(r.unknown());
    assert!(!r.discard(false));
    assert!(r.unknown());
    assert!(r.discard(true));
    assert!(r.journal().is_none());
    // A pre-dispatch refusal returns to staged.
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    r.mark_unknown();
    r.submission_error(&SchemaAlterError::PolicyBlocked);
    assert!(!r.unknown());
    r.mark_unknown();
    assert!(matches!(
        r.receipt(&receipt(
            &j,
            SchemaAlterOutcome::OutcomeUnknown {
                reason: SchemaAlterFailure::Timeout
            }
        )),
        Settlement::Unknown
    ));
    assert!(r.unknown());
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    r.mark_unknown();
    assert!(matches!(
        r.receipt(&receipt(&j, SchemaAlterOutcome::Applied { runtime_ms: 0 })),
        Settlement::Applied
    ));
    assert!(r.journal().is_none());
}
#[test]
fn schema_alter_receipt_for_a_staged_journal_is_not_accepted_as_success() {
    // A receipt can only settle the attempt that was durably marked uncertain.
    let j = journal();
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    assert!(matches!(
        r.receipt(&receipt(&j, SchemaAlterOutcome::Applied { runtime_ms: 0 })),
        Settlement::Unknown
    ));
    assert!(r.unknown());
    assert!(r.journal().is_some());
}
#[test]
fn schema_alter_cancelled_save_and_confirmation_keep_no_late_dispatch_authority() {
    use crate::apply_flow::ApplyFlow;
    let mut r = Recovery::new("owned".into(), Some(journal())).unwrap();
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
fn schema_alter_comment_null_empty_and_exact_identifier_bytes_remain_distinct() {
    assert_ne!(
        model::intent(false, true, String::new()).unwrap(),
        model::intent(false, false, String::new()).unwrap()
    );
    assert_eq!(
        model::intent(true, false, " \t".into()).unwrap(),
        SchemaAlterIntent::Rename {
            new_name: " \t".into()
        }
    );
    assert!(model::intent(true, false, String::new()).is_err());
    assert!(model::intent(true, false, "é".repeat(32)).is_err());
    assert!(model::intent(false, false, "x".repeat(4097)).is_err());
    let mut oversized = String::with_capacity(1024 * 1024);
    oversized.push('x');
    assert!(model::intent(false, false, oversized).is_err());
}
#[test]
fn schema_alter_invalid_restored_journal_is_refused_without_consuming_caller_record() {
    let mut original = journal();
    original.preview.sql.push_str(" DROP SCHEMA x;");
    assert!(SchemaAlterView::prepare("owned", None, Some(&original)).is_err());
    assert!(original.preview.sql.ends_with(" DROP SCHEMA x;"));
    let valid = journal();
    let restored = Recovery::new("owned".into(), Some(valid.clone())).unwrap();
    assert!(!restored.unknown());
    // Restoration yields a pinned selection, never a review or token.
    let selection = restored.selection().unwrap().unwrap();
    assert!(selection.pinned());
    assert!(selection.matches(&valid.target));
    let mut other = valid.target.clone();
    other.identity.schema_oid += 1;
    assert!(!selection.matches(&other));
    // A restored identity wins over a newly selected row.
    let fresh = Selection::new(SchemaAlterRequest {
        schema: "other".into(),
        expected: None,
    })
    .unwrap();
    let prepared = SchemaAlterView::prepare("owned", Some(&fresh), Some(&valid)).unwrap();
    assert_eq!(prepared.selection.unwrap().schema(), valid.target.schema);
    assert!(SchemaAlterView::prepare("", Some(&fresh), None).is_err());
}
#[test]
fn schema_alter_actions_expose_unique_accessible_labels() {
    let mut labels = ACTIONS.iter().map(|(_, label)| *label).collect::<Vec<_>>();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), ACTIONS.len());
}
