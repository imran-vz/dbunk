use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use tokio::sync::oneshot;
fn observed(document: &DataDocument, kind: MaintenanceRelationKind) -> ObservedMaintenanceTarget {
    ObservedMaintenanceTarget {
        document: document.clone(),
        statement_timeout_ms: None,
        target: MaintenanceTarget {
            database_oid: 1,
            database: "owned".into(),
            namespace_oid: 2,
            schema: "owned\"schema".into(),
            relation_oid: 3,
            name: "multi字\"table".into(),
            kind,
        },
    }
}
async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("maintenance-test", "one", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
async fn policy(backend: &Backend, readonly: bool, mode: crate::SafeMode) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        panic!("postgres")
    };
    pg.read_only = readonly;
    pg.safe_mode = mode;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}
fn completed() -> native_maintenance::Execution {
    native_maintenance::Execution {
        outcome: MaintenanceOutcome::Completed,
        notices: vec![],
        notices_truncated: false,
        runtime_ms: 1,
    }
}
#[test]
fn review_is_exact_quoted_bounded_and_classifies_partitioned_maintenance() {
    let documents = super::super::data_documents::Documents::default();
    let document = documents
        .register("w".into(), "t".into(), "connection".into())
        .unwrap();
    let capture = observed(&document, MaintenanceRelationKind::Table);
    let review = capture.review(MaintenanceIntent::Vacuum).unwrap();
    assert_eq!(
        review.preview().sql,
        "VACUUM \"owned\"\"schema\".\"multi字\"\"table\""
    );
    assert!(review.retained_bytes() < MAX_MAINTENANCE_REVIEW_BYTES);
    assert_eq!(review.preview().operation_timeout_ms, 300_000);
    assert!(review
        .preview()
        .identity_limit()
        .contains("not an atomic OID"));
    assert!(capture
        .review(MaintenanceIntent::RefreshMaterializedView {
            concurrently: false
        })
        .is_err());
    let partitioned = observed(&document, MaintenanceRelationKind::PartitionedTable);
    assert_eq!(
        partitioned
            .review(MaintenanceIntent::ReindexTable)
            .unwrap()
            .preview()
            .semantics,
        MaintenanceSemantics::PotentiallyPartial
    );
    let mut invalid = observed(&document, MaintenanceRelationKind::Table);
    invalid.target.name = "字".repeat(22);
    assert!(invalid.review(MaintenanceIntent::Vacuum).is_err());
    documents.retire(&document).unwrap();
    assert!(matches!(
        capture.review(MaintenanceIntent::Vacuum),
        Err(MaintenanceError::Unavailable)
    ));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_confirmation_preserves_authority_and_rechecks_policy_before_execution() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Strict).await;
    let review = observed(&document, MaintenanceRelationKind::Table)
        .review(MaintenanceIntent::Analyze)
        .unwrap();
    let attempt = review.attempt_id().to_owned();
    let target = review.target().clone();
    let confirmation = match backend
        .submit_maintenance(review, false, |_, _, _, _, _, _| async {
            panic!("no secret/execution before confirmation")
        })
        .await
        .unwrap()
    {
        MaintenanceSubmission::NeedsConfirmation(c) => c,
        _ => panic!("strict confirmation"),
    };
    assert_eq!(confirmation.attempt_id(), attempt);
    assert_eq!(confirmation.target(), &target);
    policy(&backend, true, crate::SafeMode::Strict).await;
    assert!(matches!(
        backend.confirm_maintenance(*confirmation).await,
        Err(MaintenanceError::PolicyBlocked)
    ));
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readonly_always_refuses_while_protected_maintenance_needs_no_confirmation() {
    let (_directory, backend, document) = fixture().await;
    for mode in [
        crate::SafeMode::Disabled,
        crate::SafeMode::Protected,
        crate::SafeMode::Strict,
    ] {
        policy(&backend, true, mode).await;
        let review = observed(&document, MaintenanceRelationKind::Table)
            .review(MaintenanceIntent::Vacuum)
            .unwrap();
        assert!(matches!(
            backend
                .submit_maintenance(review, true, |_, _, _, _, _, _| async {
                    panic!("readonly precedes executor")
                })
                .await,
            Err(MaintenanceError::PolicyBlocked)
        ));
    }
    policy(&backend, false, crate::SafeMode::Protected).await;
    let review = observed(&document, MaintenanceRelationKind::Table)
        .review(MaintenanceIntent::Vacuum)
        .unwrap();
    let result = backend
        .submit_maintenance(review, false, |_, _, permit, _, _, _| async move {
            assert!(permit.admit_dispatch());
            completed()
        })
        .await
        .unwrap();
    assert!(matches!(
        result,
        MaintenanceSubmission::Finished(receipt) if receipt.outcome == MaintenanceOutcome::Completed
    ));
    assert!(
        crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .is_empty()
    );
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_and_retired_documents_refuse_consumed_reviews_without_execution() {
    let (_directory, backend, document) = fixture().await;
    let documents = super::super::data_documents::Documents::default();
    let foreign = documents
        .register("foreign".into(), "t".into(), backend.fixture().id.clone())
        .unwrap();
    assert!(matches!(
        backend
            .apply_maintenance(
                observed(&foreign, MaintenanceRelationKind::Table)
                    .review(MaintenanceIntent::Vacuum)
                    .unwrap()
            )
            .await,
        Err(MaintenanceError::ForeignDocument)
    ));
    let review = observed(&document, MaintenanceRelationKind::Table)
        .review(MaintenanceIntent::Vacuum)
        .unwrap();
    backend.close_data_document(&document).await.unwrap();
    assert!(matches!(
        backend.apply_maintenance(review).await,
        Err(MaintenanceError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_waiter_keeps_connection_write_slot_until_driver_cleanup_joins() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Disabled).await;
    let review = observed(&document, MaintenanceRelationKind::Table)
        .review(MaintenanceIntent::Vacuum)
        .unwrap();
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let owner = backend.clone();
    let task = tokio::spawn(async move {
        owner
            .submit_maintenance(review, false, |_, drivers, permit, _, _, _| async move {
                assert!(permit.admit_dispatch());
                drivers.track_task(tokio::spawn(async move {
                    let _ = released.await;
                }));
                started.send(()).unwrap();
                drivers.drain().await;
                drop(permit);
                completed()
            })
            .await
    });
    ready.await.unwrap();
    task.abort();
    let _ = task.await;
    backend.cancel_data(&document).await.unwrap();
    assert!(backend.0.documents.begin_write(&document).is_err());
    let closing = backend.clone();
    let closed = document.clone();
    let close = tokio::spawn(async move { closing.close_data_document(&closed).await });
    tokio::task::yield_now().await;
    assert!(!close.is_finished());
    release.send(()).unwrap();
    assert_eq!(close.await.unwrap().unwrap(), DataCloseOutcome::Closed);
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_known_completed_required_overrides_are_audited() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Strict).await;
    for outcome in [
        MaintenanceOutcome::InterruptedEffectsPossible {
            reason: MaintenanceFailure::Cancelled,
        },
        MaintenanceOutcome::OutcomeUnknown {
            reason: MaintenanceFailure::Connection,
        },
        MaintenanceOutcome::Completed,
    ] {
        let review = observed(&document, MaintenanceRelationKind::MaterializedView)
            .review(MaintenanceIntent::RefreshMaterializedView { concurrently: true })
            .unwrap();
        let expected = outcome.clone();
        let result = backend
            .submit_maintenance(review, true, |_, _, permit, _, _, _| async move {
                assert!(permit.admit_commit());
                native_maintenance::Execution {
                    outcome,
                    ..completed()
                }
            })
            .await
            .unwrap();
        let MaintenanceSubmission::Finished(receipt) = result else {
            panic!("confirmed")
        };
        assert!(receipt.retained_bytes() < MAX_MAINTENANCE_RECEIPT_BYTES);
        let audit =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        assert_eq!(
            audit.len(),
            usize::from(expected == MaintenanceOutcome::Completed)
        );
        if let Some(entry) = audit.first() {
            assert_eq!(entry.command, "refresh_materialized_view");
        }
    }
    backend.shutdown().await.unwrap();
}
#[test]
fn worst_escaped_receipt_and_review_fit_native_reservations() {
    let documents = super::super::data_documents::Documents::default();
    let document = documents
        .register("w".repeat(256), "t".repeat(256), "c".repeat(256))
        .unwrap();
    let mut capture = observed(&document, MaintenanceRelationKind::MaterializedView);
    capture.target.database = "\u{1}".repeat(63);
    capture.target.schema = "\u{1}".repeat(63);
    capture.target.name = "\u{1}".repeat(63);
    let review = capture
        .review(MaintenanceIntent::RefreshMaterializedView { concurrently: true })
        .unwrap();
    assert!(review.retained_bytes() < MAX_MAINTENANCE_REVIEW_BYTES);
    let receipt = MaintenanceReceipt {
        attempt_id: review.attempt_id,
        intent: review.intent,
        target: review.target,
        preview: review.preview,
        outcome: MaintenanceOutcome::Completed,
        notices: (0..8)
            .map(|_| MaintenanceNotice {
                severity: "\u{1}".repeat(16),
                message: "\u{1}".repeat(128),
            })
            .collect(),
        notices_truncated: true,
        runtime_ms: u64::MAX,
    };
    assert!(receipt.retained_bytes() < MAX_MAINTENANCE_RECEIPT_BYTES);
    assert!(super::super::schema_ddl::encoded_bytes(&receipt) < MAX_MAINTENANCE_RECEIPT_BYTES);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_statement_timeout_refuses_before_confirmation_admission_and_credentials() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_directory, backend, document) = fixture().await;
    let calls = Arc::new(AtomicUsize::new(0));
    for (mode, confirmed) in [
        (crate::SafeMode::Disabled, false),
        (crate::SafeMode::Strict, false),
        (crate::SafeMode::Strict, true),
    ] {
        policy(&backend, false, mode).await;
        let mut capture = observed(&document, MaintenanceRelationKind::Table);
        // Simulate a stale review without retiring the lease. The refusal must
        // precede both Strict's confirmation challenge and write-slot admission.
        capture.statement_timeout_ms = Some(1234);
        let review = capture.review(MaintenanceIntent::Vacuum).unwrap();
        let occupied = backend.0.documents.begin_write(&document).unwrap();
        let loader_calls = calls.clone();
        let result = backend
            .submit_maintenance_with_loader(
                review,
                confirmed,
                |_, _, _, _, _, _| async { panic!("stale timeout cannot execute") },
                move |_, _| {
                    loader_calls.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { None })
                },
            )
            .await;
        assert!(matches!(result, Err(MaintenanceError::InvalidTarget)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "credential loader must remain untouched"
        );
        drop(occupied);
    }
    backend.shutdown().await.unwrap();
}
