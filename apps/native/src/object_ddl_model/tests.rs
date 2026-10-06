use super::*;
use dbunk_lib::backend::object_ddl::{
    ObjectAddress, ObjectDdlAttemptId, ObjectDdlClaim, ObjectDdlDescription, ObjectDdlFailure,
    ObjectDdlIndexColumn, ObjectDdlResidue, ObjectDdlStop, render_preview,
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

fn column(expression: &str, descending: bool) -> ObjectDdlIndexColumn {
    ObjectDdlIndexColumn {
        expression: expression.into(),
        descending,
    }
}

#[test]
fn index_columns_parse_in_order_with_direction_and_nested_commas() {
    let columns =
        parse_index_columns(" email , lower(name) DESC, coalesce(a, b) asc,\"Mixed, Case\" desc")
            .unwrap();
    assert_eq!(
        columns,
        vec![
            column("email", false),
            column("lower(name)", true),
            column("coalesce(a, b)", false),
            column("\"Mixed, Case\"", true),
        ]
    );
    assert_eq!(
        index_columns_text(&columns),
        "email, lower(name) DESC, coalesce(a, b), \"Mixed, Case\" DESC"
    );
    assert_eq!(
        parse_index_columns(&index_columns_text(&columns)).unwrap(),
        columns
    );
}

#[test]
fn index_columns_refuse_empty_duplicate_and_unbalanced_input() {
    for text in [
        "",
        "   ",
        "a,,b",
        "a,",
        "DESC",
        "lower(a",
        "a)",
        "\"open",
        "a, A",
        "a, \"a\"",
        "lower(a), LOWER(a)",
        "a  +  b, a + b",
    ] {
        assert!(parse_index_columns(text).is_err(), "{text:?}");
    }
    // Quoted mixed case is a different column from its folded form.
    assert!(parse_index_columns("a, \"A\"").is_ok());
    let many = (0..17)
        .map(|i| format!("c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(parse_index_columns(&many).is_err());
    assert!(parse_index_columns(&"x".repeat(1025)).is_err());
}

#[test]
fn create_index_draft_derives_name_and_enforces_method_rules() {
    let purpose = Purpose::create_index("app".into(), "users".into()).unwrap();
    assert!(purpose.uses_form());
    assert!(Purpose::create_index("app".into(), String::new()).is_err());
    let draft = Draft {
        unique: true,
        concurrently: true,
        ..Draft::default()
    };
    let operations = draft
        .operations(&purpose, "  ".into(), "email, created_at DESC".into())
        .unwrap();
    assert!(
        operations
            == vec![ObjectDdlOperation::CreateIndex {
                schema: "app".into(),
                table: "users".into(),
                name: "users_email_created_at_idx".into(),
                unique: true,
                method: "btree".into(),
                columns: vec![column("email", false), column("created_at", true)],
                concurrently: true,
            }]
    );
    assert_eq!(Purpose::from_operations(&operations), Some(purpose.clone()));
    let (restored, name, body) = Draft::from_operations(&operations);
    assert_eq!(restored, draft);
    assert_eq!(name, "users_email_created_at_idx");
    assert_eq!(body, "email, created_at DESC");

    let named = Draft::default()
        .operations(&purpose, " by_email ".into(), "email".into())
        .unwrap();
    assert!(matches!(
        &named[0],
        ObjectDdlOperation::CreateIndex { name, .. } if name == "by_email"
    ));
    assert!(
        Draft::default()
            .operations(&purpose, "n".repeat(64), "email".into())
            .is_err()
    );
    assert_eq!(
        derived_index_name(&"t".repeat(70), &[column("a", false)]).len(),
        63
    );

    let hash = Draft {
        method: IndexMethod::Hash,
        ..Draft::default()
    };
    assert!(hash.operations(&purpose, String::new(), "a".into()).is_ok());
    assert!(
        hash.operations(&purpose, String::new(), "a, b".into())
            .is_err()
    );
    assert!(
        hash.operations(&purpose, String::new(), "a DESC".into())
            .is_err()
    );
    let unique_gin = Draft {
        method: IndexMethod::Gin,
        unique: true,
        ..Draft::default()
    };
    assert!(
        unique_gin
            .operations(&purpose, String::new(), "a".into())
            .is_err()
    );
    let gist = Draft {
        method: IndexMethod::Gist,
        ..Draft::default()
    };
    assert!(
        gist.operations(&purpose, String::new(), "a, b".into())
            .is_ok()
    );
}

#[test]
fn index_methods_cycle_and_unknown_restored_methods_stay_inspect_only() {
    let mut method = IndexMethod::default();
    let mut seen = Vec::new();
    for _ in 0..IndexMethod::ALL.len() {
        assert_eq!(IndexMethod::parse(method.as_str()), Some(method));
        seen.push(method);
        method = method.next();
    }
    assert_eq!(method, IndexMethod::Btree);
    assert_eq!(seen, IndexMethod::ALL);
    assert_eq!(IndexMethod::parse("BTREE"), None);
    let operations = vec![ObjectDdlOperation::CreateIndex {
        schema: "app".into(),
        table: "t".into(),
        name: "i".into(),
        unique: false,
        method: "bloom".into(),
        columns: vec![column("a", false)],
        concurrently: false,
    }];
    assert_eq!(Purpose::from_operations(&operations), None);
}

#[test]
fn add_enum_value_draft_validates_label_and_position() {
    let purpose = Purpose::add_enum_value("app".into(), "mood".into()).unwrap();
    assert!(Purpose::add_enum_value(String::new(), "mood".into()).is_err());
    let at_end = Draft::default()
        .operations(&purpose, "calm".into(), "ignored".into())
        .unwrap();
    assert!(
        at_end
            == vec![ObjectDdlOperation::AddEnumValue {
                schema: "app".into(),
                name: "mood".into(),
                value: "calm".into(),
                position: None,
            }]
    );
    let after = Draft {
        placement: EnumPlacement::After,
        ..Draft::default()
    };
    let operations = after
        .operations(&purpose, "calm".into(), "happy".into())
        .unwrap();
    assert!(matches!(
        &operations[0],
        ObjectDdlOperation::AddEnumValue {
            position: Some(ObjectDdlEnumPosition::After { neighbor }),
            ..
        } if neighbor == "happy"
    ));
    assert_eq!(Purpose::from_operations(&operations), Some(purpose.clone()));
    let (restored, value, neighbor) = Draft::from_operations(&operations);
    assert_eq!(
        (restored, value.as_str(), neighbor.as_str()),
        (after.clone(), "calm", "happy")
    );
    let long_value = "v".repeat(64);
    let long_neighbor = "n".repeat(64);
    for (draft, value, neighbor) in [
        (Draft::default(), "", ""),
        (Draft::default(), "  ", ""),
        (Draft::default(), long_value.as_str(), ""),
        (after.clone(), "calm", ""),
        (after.clone(), "calm", "calm"),
        (after.clone(), "calm", long_neighbor.as_str()),
    ] {
        assert!(
            draft
                .operations(&purpose, value.into(), neighbor.into())
                .is_err(),
            "{value:?} {neighbor:?}"
        );
    }
    assert_eq!(EnumPlacement::End.next().next().next(), EnumPlacement::End);
}
