use super::*;
use dbunk_lib::backend::object_ddl::{
    ObjectAddress, ObjectDdlAttemptId, ObjectDdlClaim, ObjectDdlDescription, ObjectDdlFailure,
    ObjectDdlResidue, ObjectDdlStop, render_preview,
};
fn view() -> PgObjectRef {
    PgObjectRef {
        kind: PgObjectKind::View,
        schema: Some("app".into()),
        name: "v\"1".into(),
        identity_args: None,
    }
}
fn journal() -> WorkspaceObjectDdl {
    let operations = vec![ObjectDdlOperation::DropObject {
        reference: view(),
        cascade: true,
    }];
    let target = ObjectDdlDescription {
        database_oid: 1,
        claims: vec![vec![ObjectDdlClaim::Existing {
            reference: view(),
            address: ObjectAddress {
                class_oid: 1259,
                object_oid: 9,
                row_version: "3".into(),
            },
        }]],
    };
    let preview = render_preview(&target, &operations, None, true).unwrap();
    WorkspaceObjectDdl {
        attempt_id: ObjectDdlAttemptId::new(),
        target,
        operations,
        preview,
        apply_state: WorkspaceApplyState::Staged,
    }
}
fn receipt(j: &WorkspaceObjectDdl, outcome: ObjectDdlOutcome) -> ObjectDdlReceipt {
    ObjectDdlReceipt {
        attempt_id: j.attempt_id.clone(),
        connection_id: "owned".into(),
        target: j.target.clone(),
        operations: j.operations.clone(),
        outcome,
    }
}
fn unknown() -> (Recovery, WorkspaceObjectDdl) {
    let j = journal();
    let mut r = Recovery::new("owned".into(), Some(j.clone())).unwrap();
    assert!(r.mark_unknown());
    (r, j)
}

#[test]
fn purposes_refuse_unsupported_targets_and_drafts_round_trip() {
    let mut extension = view();
    extension.kind = PgObjectKind::Extension;
    extension.schema = Some("public".into());
    assert!(Purpose::drop(extension).is_err());
    let mut routine = view();
    routine.kind = PgObjectKind::Function;
    assert!(Purpose::drop(routine.clone()).is_err(), "overload identity");
    routine.identity_args = Some("integer".into());
    assert!(Purpose::drop(routine).is_ok());
    assert!(Purpose::create_view(String::new()).is_err());

    let purpose = Purpose::create_view("app".into()).unwrap();
    let draft = Draft {
        materialized: true,
        with_data: false,
        ..Draft::default()
    };
    let operations = draft
        .operations(&purpose, "mv".into(), "SELECT 1\nFROM t".into())
        .unwrap();
    assert!(matches!(
        &operations[0],
        ObjectDdlOperation::CreateMaterializedView {
            with_data: false,
            ..
        }
    ));
    assert_eq!(Purpose::from_operations(&operations), Some(purpose.clone()));
    let (restored, name, body) = Draft::from_operations(&operations);
    assert_eq!((restored, name.as_str()), (draft.clone(), "mv"));
    assert_eq!(body, "SELECT 1\nFROM t");
    for (name, body) in [
        (String::new(), "SELECT 1".to_owned()),
        ("n".repeat(64), "SELECT 1".to_owned()),
        ("v".to_owned(), String::new()),
        ("v".to_owned(), "x".repeat(16 * 1024 + 1)),
    ] {
        assert!(draft.operations(&purpose, name, body).is_err());
    }
}

#[test]
fn mismatched_receipt_never_clears_unknown_recovery() {
    for variation in 0..5 {
        let (mut r, j) = unknown();
        let mut got = receipt(&j, ObjectDdlOutcome::Applied { runtime_ms: 1 });
        match variation {
            0 => got.connection_id = "other".into(),
            1 => got.attempt_id = ObjectDdlAttemptId::new(),
            2 => got.target.database_oid += 1,
            3 => {
                if let ObjectDdlOperation::DropObject { cascade, .. } = &mut got.operations[0] {
                    *cascade = false;
                }
            }
            _ => {
                let mut staged = Recovery::new("owned".into(), Some(j.clone())).unwrap();
                // A receipt can only settle a persisted uncertain attempt.
                assert!(matches!(staged.receipt(&got), Settlement::Unknown));
                assert!(staged.unknown());
                continue;
            }
        }
        assert!(matches!(r.receipt(&got), Settlement::Unknown));
        assert!(r.unknown());
        assert!(r.journal().is_some());
    }
}

#[test]
fn receipts_settle_truthfully_including_committed_prefix_and_residue() {
    let stopped = |committed, stop, residue| ObjectDdlOutcome::Stopped {
        committed,
        stopped_at: committed,
        stop,
        reason: ObjectDdlFailure::TargetChanged,
        residue,
    };
    let cases = [
        (ObjectDdlOutcome::Applied { runtime_ms: 4 }, "applied"),
        (
            ObjectDdlOutcome::NotDispatched {
                reason: ObjectDdlFailure::Cancelled,
            },
            "staged",
        ),
        (stopped(0, ObjectDdlStop::RolledBack, None), "staged"),
        (stopped(0, ObjectDdlStop::Rejected, None), "staged"),
        (stopped(1, ObjectDdlStop::NotDispatched, None), "partial"),
        (
            stopped(
                0,
                ObjectDdlStop::Rejected,
                Some(ObjectDdlResidue::InvalidIndex {
                    schema: "app".into(),
                    name: "i".into(),
                }),
            ),
            "partial",
        ),
        (
            ObjectDdlOutcome::OutcomeUnknown {
                committed: 0,
                uncertain_end: 1,
                reason: ObjectDdlFailure::Connection,
            },
            "unknown",
        ),
    ];
    for (outcome, expected) in cases {
        let (mut r, j) = unknown();
        let settled = r.receipt(&receipt(&j, outcome));
        let actual = match settled {
            Settlement::Applied => "applied",
            Settlement::Partial => "partial",
            Settlement::Staged => "staged",
            Settlement::Unknown => "unknown",
        };
        assert_eq!(actual, expected);
        match expected {
            "applied" | "partial" => assert!(r.journal().is_none()),
            "staged" => assert!(r.journal().is_some() && !r.unknown()),
            _ => assert!(r.unknown()),
        }
    }
}

#[test]
fn unknown_recovery_requires_explicit_reconciliation_and_never_restages() {
    let (mut r, _) = unknown();
    assert!(!r.discard(false));
    assert!(r.unknown());
    r.submission_error(&ObjectDdlError::OutcomeUnavailable);
    assert!(r.unknown(), "a lost delivery is never a refusal");
    r.submission_error(&ObjectDdlError::Busy);
    assert!(!r.unknown(), "a pre-dispatch refusal is staged again");
    assert!(r.mark_unknown());
    assert!(r.discard(true));
    assert!(r.journal().is_none());
    let mut tampered = journal();
    tampered.preview.statements[0].sql = "DROP VIEW other RESTRICT;".into();
    assert!(Recovery::new("owned".into(), Some(tampered)).is_err());
    assert!(Recovery::new(String::new(), None).is_err());
}

#[test]
fn lease_reserves_and_releases_the_shared_allowance() {
    let budget = Rc::new(Cell::new(SHARED_BYTES - ALLOWANCE + 1));
    assert!(Lease::admit(budget.clone()).is_none());
    budget.set(SHARED_BYTES - ALLOWANCE);
    let lease = Lease::admit(budget.clone()).unwrap();
    assert_eq!(budget.get(), SHARED_BYTES);
    drop(lease);
    assert_eq!(budget.get(), SHARED_BYTES - ALLOWANCE);
}
