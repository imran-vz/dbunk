use super::*;
use crate::{
    backend::profile,
    postgres::native_table_seed::{self, Execution},
};
use futures_util::FutureExt;
use std::time::Duration;
fn intent() -> TableSeedIntent {
    TableSeedIntent::new(
        TableSeedEndpoint {
            connection_id: profile::CONNECTION_ID.into(),
            schema: "public".into(),
            table: "items".into(),
        },
        100,
        Some(u64::MAX),
        vec![],
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
    *backend.0.table_seed.inspector.lock().unwrap() = Some(Arc::new(|intent, targets| {
        async move { Ok(native_table_seed::tests::plan(intent, targets)) }.boxed()
    }));
    *backend.0.table_seed.runner.lock().unwrap() = Some(Arc::new(|control| {
        async move {
            assert!(control.admit_commit());
            Execution {
                outcome: TableSeedOutcome::Completed { rows: 2 },
                failure: None,
            }
        }
        .boxed()
    }));
    (directory, backend)
}
async fn wait(
    backend: &Backend,
    id: TableSeedAttemptId,
    test: impl Fn(&TableSeedObservation) -> bool,
) -> TableSeedObservation {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let value = backend.get_table_seed(id).unwrap();
            if test(&value) {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}
async fn review(backend: &Backend, id: TableSeedAttemptId) -> TableSeedReview {
    backend.begin_table_seed(id, intent()).unwrap();
    wait(backend, id, |o| o.phase == TableSeedPhase::ReadyReview).await;
    backend.review_table_seed(id).unwrap()
}
fn confirm(backend: &Backend, review: TableSeedReview) {
    let TableSeedSubmission::NeedsConfirmation(c) = backend.start_table_seed(review).unwrap()
    else {
        panic!("strict confirmation")
    };
    assert!(matches!(
        backend.confirm_table_seed(*c).unwrap(),
        TableSeedSubmission::Accepted(_)
    ));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_submission_keeps_app_owned_receipt_and_exactly_once_success_audit() {
    let (_dir, backend) = backend().await;
    let id = TableSeedAttemptId::new();
    let review = review(&backend, id).await;
    let description = review.description().clone();
    confirm(&backend, review);
    let observed = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(observed.outcome, TableSeedOutcome::Completed { rows: 2 });
    assert_eq!(observed.change_revision, Some(1));
    let receipt = observed.receipt.unwrap();
    assert_eq!(receipt.attempt_id, id);
    assert!(receipt.description == description);
    assert!(receipt.checked_heap_bytes().is_some());
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM safety_overrides WHERE command='seed_table'")
            .fetch_one(&backend.0.state.pool)
            .await
            .unwrap();
    assert_eq!(audits, 1);
    assert_eq!(
        backend.begin_table_seed(id, intent()).err(),
        Some(TableSeedError::DuplicateAttempt)
    );
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_is_retained_in_receipt_and_change_highwater_without_success_audit() {
    let (_dir, backend) = backend().await;
    *backend.0.table_seed.runner.lock().unwrap() = Some(Arc::new(|control| {
        async move {
            assert!(control.admit_commit());
            Execution {
                outcome: TableSeedOutcome::OutcomeUnknown,
                failure: Some(TableSeedError::Database.into()),
            }
        }
        .boxed()
    }));
    let id = TableSeedAttemptId::new();
    confirm(&backend, review(&backend, id).await);
    let result = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(result.outcome, TableSeedOutcome::OutcomeUnknown);
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
async fn metadata_retention_does_not_hold_admission_after_explicit_release() {
    let (_dir, backend) = backend().await;
    let mut retained = Vec::new();
    for _ in 0..6 {
        let id = TableSeedAttemptId::new();
        let review = review(&backend, id).await;
        retained.push(backend.inspect_table_seed(id).unwrap());
        drop(review);
        backend.cancel_table_seed(id).unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if backend.release_table_seed(id).is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(retained.len(), 6);
    assert_eq!(retained[0].intent().seed, Some(u64::MAX));
    assert!(backend.list_table_seeds().unwrap().jobs.is_empty());
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn needs_recipe_exposes_metadata_without_executable_review() {
    let (_dir, backend) = backend().await;
    *backend.0.table_seed.inspector.lock().unwrap() = Some(Arc::new(|intent, target| {
        async move {
            let mut p = native_table_seed::tests::plan(intent, target);
            p.description = None;
            p.issue = Some(TableSeedError::UnsupportedColumn);
            Ok(p)
        }
        .boxed()
    }));
    let id = TableSeedAttemptId::new();
    backend.begin_table_seed(id, intent()).unwrap();
    wait(&backend, id, |o| o.phase == TableSeedPhase::NeedsRecipe).await;
    assert_eq!(
        backend.inspect_table_seed(id).unwrap().issue(),
        Some(TableSeedError::UnsupportedColumn)
    );
    assert_eq!(
        backend.review_table_seed(id).err(),
        Some(TableSeedError::StaleReview)
    );
    backend.cancel_table_seed(id).unwrap();
    assert!(backend.get_table_seed(id).unwrap().receipt.is_none());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_invalidates_confirmation_and_same_id_replacement_cannot_use_old_token() {
    let (_dir, backend) = backend().await;
    let id = TableSeedAttemptId::new();
    let TableSeedSubmission::NeedsConfirmation(old) = backend
        .start_table_seed(review(&backend, id).await)
        .unwrap()
    else {
        panic!("confirmation")
    };
    let reacquired = backend.review_table_seed(id).unwrap();
    assert_eq!(reacquired.attempt_id(), id);
    backend.cancel_table_seed(id).unwrap();
    backend
        .0
        .table_seed
        .owner
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    backend.release_table_seed(id).unwrap();
    let current = review(&backend, id).await;
    assert_eq!(
        backend.confirm_table_seed(*old).err(),
        Some(TableSeedError::StaleReview)
    );
    assert_eq!(
        backend.get_table_seed(id).unwrap().phase,
        TableSeedPhase::ReadyReview
    );
    confirm(&backend, current);
    wait(&backend, id, |o| o.phase.terminal()).await;
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retirement_cancels_ready_tokens_and_latches_synchronous_registration() {
    let (_dir, backend) = backend().await;
    let id = TableSeedAttemptId::new();
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
            .begin_table_seed(TableSeedAttemptId::new(), intent())
            .err(),
        Some(TableSeedError::Closing)
    );
    assert_eq!(
        backend.start_table_seed(old).err(),
        Some(TableSeedError::Closing)
    );
    drop(guard);
    drop(_gate);
    assert_eq!(
        backend.get_table_seed(id).unwrap().phase,
        TableSeedPhase::Cancelled
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
    *backend.0.table_seed.runner.lock().unwrap() = Some(Arc::new(move |control| {
        started.lock().unwrap().take().unwrap().send(()).unwrap();
        let ended = ended_worker.clone();
        async move {
            control.cancelled().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            ended.store(true, std::sync::atomic::Ordering::Release);
            Execution {
                outcome: TableSeedOutcome::RolledBack,
                failure: Some(TableSeedError::Cancelled.into()),
            }
        }
        .boxed()
    }));
    let id = TableSeedAttemptId::new();
    confirm(&backend, review(&backend, id).await);
    ready.await.unwrap();
    backend.shutdown().await.unwrap();
    assert!(ended.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        backend.get_table_seed(id).unwrap().outcome,
        TableSeedOutcome::RolledBack
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn target_authority_and_readonly_refuse_before_inspection() {
    let (_dir, backend) = backend().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    *backend.0.table_seed.inspector.lock().unwrap() = Some(Arc::new(move |_, _| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { panic!("unauthorized inspect") }.boxed()
    }));
    let mut invalid = intent();
    invalid.endpoint.connection_id = "foreign".into();
    let id = TableSeedAttemptId::new();
    backend.begin_table_seed(id, invalid).unwrap();
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
        .table_seed
        .owner
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    let id = TableSeedAttemptId::new();
    backend.begin_table_seed(id, intent()).unwrap();
    let failed = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(failed.failure, Some(TableSeedError::PolicyBlocked));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_refuses_oversized_retained_capacity_before_publication() {
    let (_dir, backend) = backend().await;
    *backend.0.table_seed.inspector.lock().unwrap() = Some(Arc::new(|intent, target| {
        async move {
            let mut p = native_table_seed::tests::plan(intent, target);
            p.columns[0].name.reserve(MAX_TABLE_SEED_REVIEW_BYTES);
            Ok(p)
        }
        .boxed()
    }));
    let id = TableSeedAttemptId::new();
    backend.begin_table_seed(id, intent()).unwrap();
    let failure = wait(&backend, id, |o| o.phase.terminal()).await;
    assert_eq!(failure.failure, Some(TableSeedError::Limit));
    assert!(backend.inspect_table_seed(id).is_err());
    backend.shutdown().await.unwrap();
}
