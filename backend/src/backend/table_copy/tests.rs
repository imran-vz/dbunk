use super::*;
use crate::{
    backend::profile,
    postgres::native_table_copy::{self, Execution},
};
use futures_util::FutureExt;
use std::time::Duration;
fn intent() -> TableCopyIntent {
    TableCopyIntent::new(
        TableCopyEndpoint {
            connection_id: profile::CONNECTION_ID.into(),
            schema: "public".into(),
            table: "source".into(),
        },
        TableCopyEndpoint {
            connection_id: profile::CONNECTION_ID.into(),
            schema: "public".into(),
            table: "destination".into(),
        },
    )
    .unwrap()
}
async fn backend() -> (tempfile::TempDir, Backend) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, profile::CONNECTION_ID)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!("pg")
    };
    pg.safe_mode = crate::SafeMode::Strict;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
    *backend.0.table_copy.inspector.lock().unwrap() = Some(Arc::new(|intent, targets| {
        async move { Ok(native_table_copy::tests::plan(intent, targets)) }.boxed()
    }));
    *backend.0.table_copy.runner.lock().unwrap() = Some(Arc::new(|control| {
        async move {
            assert!(control.admit_commit());
            Execution {
                outcome: TableCopyOutcome::Completed { rows: 2 },
                failure: None,
            }
        }
        .boxed()
    }));
    (directory, backend)
}
async fn wait(
    backend: &Backend,
    id: TableCopyAttemptId,
    test: impl Fn(&TableCopyObservation) -> bool,
) -> TableCopyObservation {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = backend.get_table_copy(id).unwrap();
            if test(&value) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}
async fn review(backend: &Backend, id: TableCopyAttemptId) -> TableCopyReview {
    backend.begin_table_copy(id, intent()).unwrap();
    wait(backend, id, |o| o.phase == TableCopyPhase::ReadyReview).await;
    backend.review_table_copy(id).unwrap()
}
fn confirm(backend: &Backend, review: TableCopyReview) {
    let TableCopySubmission::NeedsConfirmation(c) = backend.start_table_copy(review).unwrap()
    else {
        panic!("strict confirmation")
    };
    assert!(matches!(
        backend.confirm_table_copy(*c).unwrap(),
        TableCopySubmission::Accepted(_)
    ));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_submission_keeps_app_owned_receipt_and_exactly_once_success_audit() {
    let (_dir, backend) = backend().await;
    let id = TableCopyAttemptId::new();
    let review = review(&backend, id).await;
    let description = review.description().clone();
    confirm(&backend, review);
    let observed = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(observed.outcome, TableCopyOutcome::Completed { rows: 2 });
    assert_eq!(observed.change_revision, Some(1));
    let receipt = observed.receipt.unwrap();
    assert_eq!(receipt.attempt_id, id);
    assert!(receipt.description == description);
    assert!(receipt.checked_heap_bytes().is_some());
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM safety_overrides WHERE command='native_table_copy'",
    )
    .fetch_one(&backend.0.state.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    assert_eq!(
        backend.begin_table_copy(id, intent()).err(),
        Some(TableCopyError::DuplicateAttempt)
    );
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_is_retained_in_receipt_and_change_highwater_without_success_audit() {
    let (_dir, backend) = backend().await;
    *backend.0.table_copy.runner.lock().unwrap() = Some(Arc::new(|control| {
        async move {
            assert!(control.admit_commit());
            Execution {
                outcome: TableCopyOutcome::OutcomeUnknown,
                failure: Some(TableCopyError::Database.into()),
            }
        }
        .boxed()
    }));
    let id = TableCopyAttemptId::new();
    confirm(&backend, review(&backend, id).await);
    let result = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(result.outcome, TableCopyOutcome::OutcomeUnknown);
    assert_eq!(result.change_revision, Some(1));
    assert!(result.receipt.is_some());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_invalidates_confirmation_and_same_id_replacement_cannot_use_old_token() {
    let (_dir, backend) = backend().await;
    let id = TableCopyAttemptId::new();
    let TableCopySubmission::NeedsConfirmation(old) = backend
        .start_table_copy(review(&backend, id).await)
        .unwrap()
    else {
        panic!("confirmation")
    };
    let reacquired = backend.review_table_copy(id).unwrap();
    assert_eq!(reacquired.attempt_id(), id);
    backend.cancel_table_copy(id).unwrap();
    backend
        .0
        .table_copy
        .owner
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    backend.release_table_copy(id).unwrap();
    let current = review(&backend, id).await;
    assert_eq!(
        backend.confirm_table_copy(*old).err(),
        Some(TableCopyError::StaleReview)
    );
    assert_eq!(
        backend.get_table_copy(id).unwrap().phase,
        TableCopyPhase::ReadyReview
    );
    confirm(&backend, current);
    wait(&backend, id, |o| o.phase.terminal()).await;
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retirement_cancels_ready_tokens_and_latches_synchronous_registration() {
    let (_dir, backend) = backend().await;
    let id = TableCopyAttemptId::new();
    let old = review(&backend, id).await;
    let _gate = backend.0.development_gate.lock().await;
    let guard = retire_connection(
        &backend.0,
        Some(profile::CONNECTION_ID),
        tokio::time::Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(
        backend
            .begin_table_copy(TableCopyAttemptId::new(), intent())
            .err(),
        Some(TableCopyError::Closing)
    );
    assert_eq!(
        backend.start_table_copy(old).err(),
        Some(TableCopyError::Closing)
    );
    drop(guard);
    drop(_gate);
    assert_eq!(
        backend.get_table_copy(id).unwrap().phase,
        TableCopyPhase::Cancelled
    );
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_joins_an_admitted_worker_after_cancellation_not_just_driver_abort() {
    let (_dir, backend) = backend().await;
    let (started, ready) = tokio::sync::oneshot::channel();
    let started = Mutex::new(Some(started));
    let ended = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ended_worker = ended.clone();
    *backend.0.table_copy.runner.lock().unwrap() = Some(Arc::new(move |control| {
        started.lock().unwrap().take().unwrap().send(()).unwrap();
        let ended = ended_worker.clone();
        async move {
            control.cancelled().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            ended.store(true, std::sync::atomic::Ordering::Release);
            Execution {
                outcome: TableCopyOutcome::RolledBack,
                failure: Some(TableCopyError::Cancelled.into()),
            }
        }
        .boxed()
    }));
    let id = TableCopyAttemptId::new();
    confirm(&backend, review(&backend, id).await);
    ready.await.unwrap();
    backend.shutdown().await.unwrap();
    assert!(ended.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        backend.get_table_copy(id).unwrap().outcome,
        TableCopyOutcome::RolledBack
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_endpoint_authorities_and_destination_readonly_refuse_before_inspection() {
    let (_dir, backend) = backend().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    *backend.0.table_copy.inspector.lock().unwrap() = Some(Arc::new(move |_, _| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { panic!("unauthorized inspect") }.boxed()
    }));
    let mut invalid = intent();
    invalid.source.connection_id = "foreign".into();
    let id = TableCopyAttemptId::new();
    backend.begin_table_copy(id, invalid).unwrap();
    wait(&backend, id, |o| o.phase.terminal()).await;
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, profile::CONNECTION_ID)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!("pg")
    };
    pg.read_only = true;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
    backend
        .0
        .table_copy
        .owner
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    let id = TableCopyAttemptId::new();
    backend.begin_table_copy(id, intent()).unwrap();
    let failed = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(failed.failure, Some(TableCopyError::PolicyBlocked));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    backend.shutdown().await.unwrap();
}
#[test]
fn borrowed_bounds_refuse_spare_capacity_and_invalid_recovery_identity() {
    let mut input = intent();
    input.source.schema.reserve(10000);
    assert!(input.checked_heap_bytes().is_none());
    assert!(TableCopyAttemptId::parse("00000000-0000-0000-0000-000000000000").is_err());
    let json = r#"{"intent":{"source":{"connectionId":"c","schema":"s","table":"t"},"destination":{"connectionId":"d","schema":"s","table":"t"}},"sourceConnection":{"connectionName":"c","host":"h","port":5432,"database":"d","user":"u","environment":"Test","safeMode":"Strict","readOnly":false},"destinationConnection":{"connectionName":"c","host":"h","port":5432,"database":"d","user":"u","environment":"Test","safeMode":"Strict","readOnly":false},"sourceRelation":{"databaseOid":1,"relationOid":1,"kind":"r"},"destinationRelation":{"databaseOid":1,"relationOid":2,"kind":"r"},"mappingSha256":"bad","copiedColumns":1,"defaultedColumns":0,"generatedColumns":0,"identityColumns":0}"#;
    assert!(serde_json::from_str::<TableCopyDescription>(json).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_admitted_data_retirement_keeps_owner_until_existing_request_joins() {
    let (_dir, backend) = backend().await;
    let document = backend
        .0
        .documents
        .register(
            "copy-retirement-test".into(),
            "tab".into(),
            profile::CONNECTION_ID.into(),
        )
        .unwrap();
    let held = backend.0.documents.enter(&document).await.unwrap();
    let id = TableCopyAttemptId::new();
    confirm(&backend, review(&backend, id).await);
    tokio::time::timeout(Duration::from_secs(1), async {
        while document.0.check_open().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    backend.cancel_table_copy(id).unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let pending = backend.get_table_copy(id).unwrap();
    assert!(!pending.phase.terminal());
    assert_eq!(pending.cleanup, TableCopyCleanup::Pending);
    assert!(destination_write_in_progress(
        &backend.0,
        profile::CONNECTION_ID
    ));
    drop(held);
    let stopped = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(stopped.outcome, TableCopyOutcome::NotStarted);
    assert_eq!(stopped.cleanup, TableCopyCleanup::Complete);
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execution_budget_refusal_preserves_review_and_does_not_dispatch() {
    let (_dir, backend) = backend().await;
    let id = TableCopyAttemptId::new();
    let review = review(&backend, id).await;
    let permit = backend
        .0
        .table_copy
        .executions
        .clone()
        .acquire_many_owned(2)
        .await
        .unwrap();
    let TableCopySubmission::NeedsConfirmation(confirmation) =
        backend.start_table_copy(review).unwrap()
    else {
        panic!("confirmation")
    };
    assert_eq!(
        backend.confirm_table_copy(*confirmation).err(),
        Some(TableCopyError::Busy)
    );
    assert_eq!(
        backend.get_table_copy(id).unwrap().phase,
        TableCopyPhase::AwaitingConfirmation
    );
    drop(permit);
    confirm(&backend, backend.review_table_copy(id).unwrap());
    wait(&backend, id, |o| o.phase.terminal()).await;
    backend.shutdown().await.unwrap();
}
