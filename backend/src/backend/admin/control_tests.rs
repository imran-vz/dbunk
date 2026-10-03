use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use tokio::sync::oneshot;

fn snapshot() -> AdminSnapshot {
    let target = AdminControlTarget::test_target();
    AdminSnapshot {
        database: "owned-test".into(),
        reader_pid: 1,
        activity_restricted: false,
        sessions: vec![AdminSession {
            pid: target.pid(),
            user: Some("owned-user".into()),
            database: target.database().map(str::to_owned),
            application_name: None,
            client_addr: None,
            state: Some("active".into()),
            wait_event_type: None,
            wait_event: None,
            query_age_seconds: None,
            transaction_age_seconds: None,
            query: Some("sensitive statement".into()),
            query_clipped: false,
            details_restricted: false,
            backend_start: Some(target.backend_start().into()),
            query_start: target.query_start().map(str::to_owned),
            xact_start: None,
        }],
        locks: vec![],
        pending_transactions: vec![],
        stats: AdminStats {
            database_size_bytes: AdminMetric::Unavailable,
            cache_hit_ratio: AdminMetric::Unavailable,
            active_sessions: AdminMetric::Unavailable,
            idle_in_transaction: AdminMetric::Unavailable,
            blocked_locks: AdminMetric::Unavailable,
        },
        collected_start: "start".into(),
        collected_end: "end".into(),
        scope_note: "test".into(),
        sessions_truncated: false,
        locks_truncated: false,
        pending_transactions_truncated: false,
    }
}
fn review(document: &DataDocument, action: AdminControlAction) -> AdminControlReview {
    AdminCapture::new(document.clone(), snapshot())
        .unwrap()
        .review(
            AdminRow {
                section: AdminSection::Sessions,
                index: 0,
            },
            action,
        )
        .unwrap()
}
async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    // Synthetic temporary SQLite only. Injected execution never opens a socket.
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("admin-control-test", "one", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
async fn policy(backend: &Backend, read_only: bool, mode: crate::SafeMode) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        panic!("PostgreSQL")
    };
    pg.read_only = read_only;
    pg.safe_mode = mode;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_mints_exact_targets_refuses_missing_identity_and_bounds_retention() {
    let (_directory, backend, document) = fixture().await;
    let data = snapshot();
    let capture = AdminCapture::new(document.clone(), data.clone()).unwrap();
    let reviewed = capture
        .review(
            AdminRow {
                section: AdminSection::Sessions,
                index: 0,
            },
            AdminControlAction::CancelQuery,
        )
        .unwrap();
    assert_eq!(reviewed.target(), &AdminControlTarget::test_target());
    assert!(reviewed.belongs_to(&document));
    assert!(reviewed.retained_bytes() < MAX_ADMIN_CONTROL_BYTES);
    assert!(!format!("{:?}", reviewed.target()).contains("owned-test"));
    assert!(!serde_json::to_string(reviewed.target())
        .unwrap()
        .contains("sensitive"));
    for invalid in [None, Some("2026-10-03".into())] {
        let mut data = data.clone();
        data.sessions[0].backend_start = invalid;
        assert!(AdminCapture::new(document.clone(), data)
            .unwrap()
            .review(
                AdminRow {
                    section: AdminSection::Sessions,
                    index: 0
                },
                AdminControlAction::TerminateSession
            )
            .is_err());
    }
    let mut background = data.clone();
    background.sessions[0].database = None;
    assert_eq!(
        AdminCapture::new(document.clone(), background)
            .unwrap()
            .review(
                AdminRow {
                    section: AdminSection::Sessions,
                    index: 0
                },
                AdminControlAction::CancelQuery
            )
            .unwrap()
            .target()
            .database(),
        None
    );
    let mut reader = data.clone();
    reader.sessions[0].pid = reader.reader_pid;
    assert!(AdminCapture::new(document.clone(), reader)
        .unwrap()
        .review(
            AdminRow {
                section: AdminSection::Sessions,
                index: 0
            },
            AdminControlAction::CancelQuery
        )
        .is_err());
    let mut oversized = data;
    oversized.scope_note = String::with_capacity(MAX_ADMIN_BYTES + 1);
    assert!(AdminCapture::new(document.clone(), oversized).is_err());
    backend.close_data_document(&document).await.unwrap();
    assert!(matches!(
        capture.review(
            AdminRow {
                section: AdminSection::Sessions,
                index: 0
            },
            AdminControlAction::CancelQuery
        ),
        Err(AdminControlError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn confirmation_preserves_exact_attempt_and_rechecks_current_readonly_policy() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Strict).await;
    let reviewed = review(&document, AdminControlAction::TerminateSession);
    let attempt = reviewed.attempt_id().to_owned();
    let target = reviewed.target().clone();
    let confirmation = match backend
        .submit_admin_control(reviewed, false, |_, _, _, _, _, _| async {
            panic!("policy precedes execution")
        })
        .await
        .unwrap()
    {
        AdminControlSubmission::NeedsConfirmation(value) => value,
        _ => panic!("strict termination needs confirmation"),
    };
    assert_eq!(confirmation.attempt_id(), attempt);
    assert_eq!(confirmation.target(), &target);
    assert_eq!(confirmation.action(), AdminControlAction::TerminateSession);
    policy(&backend, true, crate::SafeMode::Strict).await;
    assert!(matches!(
        backend.confirm_admin_control(*confirmation).await,
        Err(AdminControlError::PolicyBlocked)
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_document_and_retired_confirmation_never_reach_execution() {
    let (_directory, backend, document) = fixture().await;
    let foreign = super::super::data_documents::Documents::default()
        .register("foreign".into(), "one".into(), backend.fixture().id.clone())
        .unwrap();
    assert!(matches!(
        backend
            .apply_admin_control(review(&foreign, AdminControlAction::CancelQuery))
            .await,
        Err(AdminControlError::ForeignDocument)
    ));
    policy(&backend, false, crate::SafeMode::Strict).await;
    let confirmation = match backend
        .apply_admin_control(review(&document, AdminControlAction::TerminateSession))
        .await
        .unwrap()
    {
        AdminControlSubmission::NeedsConfirmation(value) => value,
        _ => panic!("strict needs confirmation"),
    };
    backend.close_data_document(&document).await.unwrap();
    assert!(matches!(
        backend.confirm_admin_control(*confirmation).await,
        Err(AdminControlError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}

#[test]
fn lock_control_requires_the_same_observed_holder_not_a_blocker_pid() {
    let documents = super::super::data_documents::Documents::default();
    let document = documents
        .register("test".into(), "locks".into(), "connection".into())
        .unwrap();
    let mut data = snapshot();
    let session = &data.sessions[0];
    data.locks.push(AdminLock {
        pid: Some(session.pid),
        lock_type: "relation".into(),
        relation: None,
        mode: "AccessExclusiveLock".into(),
        granted: false,
        blocked_by: vec![77],
        blocked_by_clipped: false,
        blocked_by_unavailable: false,
        query: session.query.clone(),
        query_clipped: false,
        details_restricted: false,
        backend_start: session.backend_start.clone(),
        query_start: session.query_start.clone(),
    });
    let row = AdminRow {
        section: AdminSection::Locks,
        index: 0,
    };
    let capture = AdminCapture::new(document.clone(), data.clone()).unwrap();
    assert_eq!(
        capture
            .review(row, AdminControlAction::CancelQuery)
            .unwrap()
            .target()
            .pid(),
        session.pid
    );
    data.locks[0].pid = Some(77);
    assert!(AdminCapture::new(document.clone(), data.clone())
        .unwrap()
        .review(row, AdminControlAction::CancelQuery)
        .is_err());
    data.locks[0].pid = None;
    assert!(AdminCapture::new(document.clone(), data.clone())
        .unwrap()
        .review(row, AdminControlAction::TerminateSession)
        .is_err());
    data.locks[0].pid = Some(data.sessions[0].pid);
    data.locks[0].query_start = None;
    assert!(AdminCapture::new(document, data)
        .unwrap()
        .review(row, AdminControlAction::CancelQuery)
        .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readonly_cancel_is_allowed_during_schema_write_without_confirmation_or_audit() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, true, crate::SafeMode::Strict).await;
    let schema = backend.0.documents.begin_write(&document).unwrap();
    let reviewed = review(&document, AdminControlAction::CancelQuery);
    let result = backend
        .submit_admin_control(
            reviewed,
            false,
            |_, _, permit, _, target, action| async move {
                assert_eq!(target, AdminControlTarget::test_target());
                assert_eq!(action, AdminControlAction::CancelQuery);
                assert!(permit.admit_dispatch());
                AdminControlOutcome::SignalSent
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        AdminControlSubmission::Finished(AdminControlReceipt {
            outcome: AdminControlOutcome::SignalSent,
            ..
        })
    ));
    assert!(
        crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .is_empty()
    );
    drop(schema);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_known_sent_required_overrides_are_audited_once() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Protected).await;
    for outcome in [
        AdminControlOutcome::SignalNotSent,
        AdminControlOutcome::TargetChanged,
        AdminControlOutcome::OutcomeUnknown {
            reason: AdminControlFailure::Connection,
        },
        AdminControlOutcome::SignalSent,
    ] {
        let confirmation = match backend
            .apply_admin_control(review(&document, AdminControlAction::TerminateSession))
            .await
            .unwrap()
        {
            AdminControlSubmission::NeedsConfirmation(value) => value,
            _ => panic!("protected needs confirmation"),
        };
        let expected = outcome.clone();
        let result = backend
            .submit_admin_control(
                confirmation.review,
                true,
                |_, _, permit, _, _, _| async move {
                    assert!(permit.admit_dispatch());
                    outcome
                },
            )
            .await
            .unwrap();
        let AdminControlSubmission::Finished(receipt) = result else {
            panic!("confirmed")
        };
        assert_eq!(receipt.outcome, expected);
        assert!(receipt.retained_bytes() < MAX_ADMIN_CONTROL_BYTES);
        let audit =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        assert_eq!(
            audit.len(),
            usize::from(expected == AdminControlOutcome::SignalSent)
        );
        if let Some(entry) = audit.first() {
            assert_eq!(entry.command, "terminate_pg_backend");
        }
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_waiter_keeps_control_slot_and_document_owned_through_cleanup() {
    let (_directory, backend, document) = fixture().await;
    let reviewed = review(&document, AdminControlAction::CancelQuery);
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let owner = backend.clone();
    let task = tokio::spawn(async move {
        owner
            .submit_admin_control(reviewed, false, |_, drivers, permit, _, _, _| async move {
                assert!(permit.admit_dispatch());
                let child = tokio::spawn(async move {
                    let _ = released.await;
                });
                drivers.track_task(child);
                started.send(()).unwrap();
                drivers.drain().await;
                drop(permit);
                AdminControlOutcome::SignalSent
            })
            .await
    });
    ready.await.unwrap();
    task.abort();
    let _ = task.await;
    backend.cancel_data(&document).await.unwrap();
    assert!(backend.0.documents.begin_control(&document).is_err());
    let closing = backend.clone();
    let closed = document.clone();
    let close = tokio::spawn(async move { closing.close_data_document(&closed).await });
    tokio::task::yield_now().await;
    assert!(!close.is_finished());
    release.send(()).unwrap();
    assert_eq!(close.await.unwrap().unwrap(), DataCloseOutcome::Closed);
    assert!(backend.0.documents.begin_control(&document).is_err());
    backend.shutdown().await.unwrap();
}
