use super::*;

fn budget() -> Rc<Cell<usize>> {
    Rc::new(Cell::new(0))
}
fn setup(budget: Rc<Cell<usize>>) -> Setup {
    Setup::new(
        "connection".into(),
        7,
        Operation::Backup,
        Some(("Case Sensitive".into(), "a.b".into())),
        budget,
    )
    .unwrap()
}
fn job() -> PgToolObservation {
    PgToolObservation {
        attempt_id: PgToolAttemptId::new(),
        connection_id: "connection".into(),
        kind: Operation::Restore,
        format: Format::Custom,
        scope: Scope::Database,
        clean: false,
        file_name: "source.dump".into(),
        phase: PgToolPhase::Completed,
        effect: PgToolEffect::Succeeded,
        cleanup: PgToolCleanup::Complete,
        source_bytes: Some(0),
        bytes_processed: None,
        tool_version: None,
        started_at: "2026-10-03T10:00:00Z".into(),
        finished_at: Some("2026-10-03T10:01:00Z".into()),
        failure: None,
        diagnostic: None,
        restore_change_revision: Some(1),
    }
}

#[test]
fn setup_captures_exact_scope_and_fences_picker_and_review_changes() {
    let shared = budget();
    let mut setup = setup(shared.clone());
    assert_eq!(
        setup.scope(),
        &Scope::Table {
            schema: "Case Sensitive".into(),
            table: "a.b".into()
        }
    );
    let picker = setup.token();
    setup
        .accept_path(&picker, PathBuf::from("/tmp/../tmp/source 東京.dump"))
        .unwrap();
    assert_eq!(
        setup.path().unwrap().as_os_str(),
        "/tmp/../tmp/source 東京.dump"
    );
    assert!(!setup.is_current(&picker));
    let review = setup.token();
    assert!(setup.set_clean(true).is_err()); // Custom backup has no clean option.
    assert!(setup.is_current(&review));
    setup.set_format(Format::Plain).unwrap();
    assert!(setup.path().is_none());
    assert!(!setup.is_current(&review));
    assert!(
        setup
            .accept_path(&picker, PathBuf::from("/tmp/stale.sql"))
            .is_err()
    );
    setup.set_clean(true).unwrap();
    setup.set_operation(Operation::Restore).unwrap();
    assert!(!setup.clean());
    assert!(!setup.clean_enabled());
    setup.set_format(Format::Custom).unwrap();
    setup.set_clean(true).unwrap();
    let picker = setup.token();
    setup
        .accept_path(&picker, PathBuf::from("/tmp/archive.dump"))
        .unwrap();
    let intent = setup.intent().unwrap();
    assert_eq!(intent.scope(), &Scope::Database); // Table context cannot narrow restore.
    assert_eq!(intent.path(), Path::new("/tmp/archive.dump"));
    let review = setup.token();
    setup.retarget("connection".into(), 8).unwrap();
    assert!(!setup.is_current(&review));
    assert_eq!(setup.scope(), &Scope::Database);
    assert!(setup.path().is_none());
    drop(setup);
    assert_eq!(shared.get(), 0);
}

#[test]
fn refused_path_preserves_previous_selection_and_tokens_are_setup_local() {
    let mut first = setup(budget());
    let second = setup(budget());
    assert!(!second.is_current(&first.token()));
    first
        .accept_path(&first.token(), PathBuf::from("/tmp/good.dump"))
        .unwrap();
    let review = first.token();
    assert!(
        first
            .accept_path(&review, PathBuf::from("relative.dump"))
            .is_err()
    );
    let mut oversized = PathBuf::from("/tmp/small.dump");
    oversized.reserve(MAX_PATH_BYTES * 2);
    assert!(first.accept_path(&review, oversized).is_err());
    assert!(first.is_current(&review));
    assert_eq!(first.path(), Some(Path::new("/tmp/good.dump")));
}

#[test]
fn shared_capture_replacement_charges_both_and_refusal_retains_original() {
    let shared = budget();
    let first = Capture::new(
        PgToolJobList {
            restore_change_revision: 1,
            jobs: vec![job()],
        },
        shared.clone(),
    )
    .unwrap();
    let selected = first.key(0).unwrap();
    assert_eq!(shared.get(), CAPTURE_BYTES);
    shared.set(WORKSPACE_BYTES);
    assert!(
        Capture::new(
            PgToolJobList {
                restore_change_revision: 1,
                jobs: vec![job()]
            },
            shared.clone()
        )
        .is_err()
    );
    assert_eq!(first.key(0), Some(selected));
    assert_eq!(shared.get(), WORKSPACE_BYTES);
    shared.set(CAPTURE_BYTES);
    let replacement = Capture::new(
        PgToolJobList {
            restore_change_revision: 1,
            jobs: vec![job()],
        },
        shared.clone(),
    )
    .unwrap();
    assert_eq!(shared.get(), CAPTURE_BYTES * 2);
    assert_eq!(replacement.index_for_key(selected), None);
    drop(first);
    assert_eq!(shared.get(), CAPTURE_BYTES);
    drop(replacement);
    assert_eq!(shared.get(), 0);
}

#[test]
fn capture_refuses_duplicate_ids_and_unbounded_capacity_before_admission() {
    let shared = budget();
    let row = job();
    assert!(
        Capture::new(
            PgToolJobList {
                restore_change_revision: 1,
                jobs: vec![row.clone(), row]
            },
            shared.clone()
        )
        .is_err()
    );
    let mut row = job();
    row.file_name.reserve(MAX_PG_TOOL_LIST_BYTES);
    assert!(
        Capture::new(
            PgToolJobList {
                restore_change_revision: 1,
                jobs: vec![row]
            },
            shared.clone()
        )
        .is_err()
    );
    let mut a = job();
    a.phase = PgToolPhase::Failed;
    a.effect = PgToolEffect::Unknown;
    a.cleanup = PgToolCleanup::Pending;
    let mut b = a.clone();
    b.attempt_id = PgToolAttemptId::new();
    assert!(
        Capture::new(
            PgToolJobList {
                restore_change_revision: 1,
                jobs: vec![a, b]
            },
            shared.clone()
        )
        .is_err()
    );
    assert_eq!(shared.get(), 0);
}

#[test]
fn retained_identity_and_terminal_cleanup_are_independent_of_row_positions() {
    let a = job();
    let mut b = job();
    b.restore_change_revision = Some(2);
    let selected = a.attempt_id;
    b.phase = PgToolPhase::Cancelled;
    b.effect = PgToolEffect::Unknown;
    b.cleanup = PgToolCleanup::Failed;
    assert!(!releasable(&b));
    assert!(!cancellable(&b));
    let capture = Capture::new(
        PgToolJobList {
            restore_change_revision: 2,
            jobs: vec![b, a],
        },
        budget(),
    )
    .unwrap();
    assert_eq!(capture.index_for_key(selected), Some(1));
    assert!(capture.has_active());
    let details = capture.details(0).unwrap();
    assert!(details.contains("unknown"));
    assert!(details.contains("admission remains held"));
    assert!(details.contains("Written: Unavailable"));
    assert!(details.contains("Source size: 0 B"));
    assert!(details.contains("cannot undo a committed restore"));
    let empty = Capture::new(
        PgToolJobList {
            restore_change_revision: 1,
            jobs: vec![],
        },
        budget(),
    )
    .unwrap();
    assert_eq!(empty.index_for_key(selected), None);
    assert!(empty.limits().contains("do not prove"));
}

#[test]
fn release_fence_rejects_old_lists_even_after_new_requests_are_issued() {
    let mut order = ObservationOrder::default();
    let before_release = order.issue().unwrap();
    order.fence().unwrap();
    let after_release = order.issue().unwrap();
    assert!(!order.accept(before_release));
    assert!(order.accept(after_release));
    assert!(!order.accept(0));
    assert!(!order.accept(after_release + 1));
    assert!(!order.accept(before_release));
}

#[test]
fn restore_revision_acknowledges_only_applied_plans_and_handles_expired_records() {
    let mut changes = RestoreChanges::default();
    let revisions = [
        (2, "connection-b"),
        (1, "connection-a"),
        (3, "connection-a"),
    ];
    let plan = changes.plan(3, &revisions).unwrap();
    assert_eq!(
        plan.connections().unwrap().collect::<Vec<_>>(),
        vec!["connection-a", "connection-b"]
    );
    assert_eq!(changes.acknowledged(), 0); // Merely observing is not acknowledgement.
    let other = changes.plan(3, &revisions).unwrap();
    changes.acknowledge(plan).unwrap();
    assert!(changes.acknowledge(other).is_err());
    assert!(changes.plan(3, &revisions).is_none());
    assert!(changes.plan(2, &revisions).is_none());
    let expired = changes.plan(5, &[(5, "connection-b")]).unwrap();
    assert!(expired.connections().is_none());
    changes.acknowledge(expired).unwrap();
    let long_gap = changes.plan(1000, &[]).unwrap();
    assert!(long_gap.connections().is_none());
    changes.acknowledge(long_gap).unwrap();
    assert_eq!(changes.acknowledged(), 1000);
}

#[test]
fn full_capture_revision_survives_release_and_reports_safe_diagnostics() {
    let mut row = job();
    row.phase = PgToolPhase::Failed;
    row.effect = PgToolEffect::Unknown;
    row.failure = Some(dbunk_lib::backend::pg_tools::PgToolError::ToolFailed);
    row.diagnostic = Some(dbunk_lib::backend::pg_tools::PgToolDiagnostic {
        tool: Some("pg_restore".into()),
        exit_code: Some(1),
        operation: Some("restore".into()),
        message: "Permission denied".into(),
    });
    let capture = Capture::new(
        PgToolJobList {
            jobs: vec![row],
            restore_change_revision: 1,
        },
        budget(),
    )
    .unwrap();
    let mut changes = RestoreChanges::default();
    let plan = capture.restore_invalidation(&changes).unwrap();
    assert_eq!(
        plan.connections().unwrap().collect::<Vec<_>>(),
        vec!["connection"]
    );
    changes.acknowledge(plan).unwrap();
    assert!(capture.details(0).unwrap().contains("Exit code: 1"));
    let released = Capture::new(
        PgToolJobList {
            jobs: vec![],
            restore_change_revision: 1,
        },
        budget(),
    )
    .unwrap();
    assert!(released.restore_invalidation(&changes).is_none());
    let unseen = RestoreChanges::default();
    assert!(
        released
            .restore_invalidation(&unseen)
            .unwrap()
            .connections()
            .is_none()
    );
}
