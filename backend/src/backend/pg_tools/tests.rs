use super::*;
use crate::{
    backend::profile,
    postgres::backup::{
        protocol::{PgToolJobError, PgToolJobPhase},
        runner::{Ready, Request},
    },
};
use futures_util::FutureExt;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
async fn backend() -> (tempfile::TempDir, Backend) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    (directory, backend)
}
async fn wait(
    backend: &Backend,
    id: PgToolAttemptId,
    predicate: impl Fn(&PgToolObservation) -> bool,
) -> PgToolObservation {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snapshot = backend.get_pg_tool_job(id).unwrap();
            if predicate(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn source(directory: &tempfile::TempDir) -> std::path::PathBuf {
    let path = directory.path().join("selected.sql");
    std::fs::write(&path, b"SELECT 'exact original';\n").unwrap();
    path
}
fn stub(backend: &Backend, run: registry::TestRunner) {
    *backend.0.tool_jobs.test_runner.lock().unwrap() = Some(run);
}
fn submit(backend: &Backend, id: PgToolAttemptId) {
    let review = backend.review_pg_tool_job(id).unwrap();
    let submission = backend.start_pg_tool_job(review).unwrap();
    if let PgToolSubmission::NeedsConfirmation(confirmation) = submission {
        assert!(matches!(
            backend.confirm_pg_tool_job(*confirmation).unwrap(),
            PgToolSubmission::Accepted(_)
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_attempt_survives_lost_reply_and_restore_uses_immutable_snapshot() {
    let (directory, backend) = backend().await;
    let path = source(&directory);
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();
    stub(
        &backend,
        Arc::new(move |context, _, request| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                let Request::Restore(payload) = request else {
                    panic!("restore")
                };
                assert_eq!(
                    std::fs::read(payload.source_path).unwrap(),
                    b"SELECT 'exact original';\n"
                );
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.mark_restore_dispatch();
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    let _lost = backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(path.clone(), PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    assert_eq!(
        backend.list_pg_tool_jobs(None).unwrap().jobs[0].attempt_id,
        id
    );
    assert_eq!(
        backend
            .begin_pg_tool_job(
                id,
                profile::CONNECTION_ID.into(),
                PgToolIntent::restore(path.clone(), PgToolFormat::Plain, false).unwrap()
            )
            .unwrap_err(),
        PgToolError::DuplicateAttempt
    );
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    std::fs::write(&path, b"SELECT 'replaced';").unwrap();
    let review = backend.review_pg_tool_job(id).unwrap();
    assert_eq!(review.observation().source_bytes, Some(25));
    assert_eq!(review.target().database, "dbunk_demo");
    assert!(review.retained_bytes() <= MAX_PG_TOOL_REVIEW_BYTES);
    submit(&backend, id);
    let done = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.phase, PgToolPhase::Completed);
    assert_eq!(done.effect, PgToolEffect::Succeeded);
    assert_eq!(done.restore_change_revision, Some(1));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let list = backend.list_pg_tool_jobs(None).unwrap();
    assert_eq!(list.restore_change_revision, 1);
    assert!(list.checked_heap_bytes().is_some());
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(audits, 1);
    backend.release_pg_tool_job(id).unwrap();
    assert_eq!(
        backend
            .list_pg_tool_jobs(None)
            .unwrap()
            .restore_change_revision,
        1
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_review_releases_source_and_foreign_or_stale_review_never_dispatches() {
    let (directory, backend) = backend().await;
    let path = source(&directory);
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(path, PgToolFormat::Custom, true).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    let review = backend.review_pg_tool_job(id).unwrap();
    let (_foreign, other) = self::backend().await;
    assert!(matches!(
        other.start_pg_tool_job(review),
        Err(PgToolError::ForeignReview)
    ));
    let stale = backend.review_pg_tool_job(id).unwrap();
    assert_eq!(backend.release_pg_tool_job(id), Err(PgToolError::Active));
    backend.cancel_pg_tool_job(id).unwrap();
    assert!(matches!(
        backend.start_pg_tool_job(stale),
        Err(PgToolError::StaleReview)
    ));
    let done = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.effect, PgToolEffect::NotStarted);
    assert!(done.restore_change_revision.is_none());
    assert_eq!(
        backend
            .list_pg_tool_jobs(None)
            .unwrap()
            .restore_change_revision,
        0
    );
    backend.release_pg_tool_job(id).unwrap();
    other.shutdown().await.unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_failure_after_dispatch_is_unknown_and_invalidates_once_without_audit() {
    let (directory, backend) = backend().await;
    let path = source(&directory);
    stub(
        &backend,
        Arc::new(|context, _, _| {
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.mark_restore_dispatch();
                Err(PgToolJobError::ToolFailed {
                    tool: "psql".into(),
                    exit_code: Some(2),
                    message: "Connection failed".into(),
                })
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(path, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    let done = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.effect, PgToolEffect::Unknown);
    assert_eq!(done.restore_change_revision, Some(1));
    let diagnostic = done.diagnostic.unwrap();
    assert_eq!(diagnostic.exit_code, Some(2));
    assert_eq!(diagnostic.tool.as_deref(), Some("psql"));
    for _ in 0..3 {
        assert_eq!(
            backend
                .list_pg_tool_jobs(None)
                .unwrap()
                .restore_change_revision,
            1
        );
    }
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(audits, 0);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_fence_cancels_ready_review_and_admission_is_bounded() {
    let (directory, backend) = backend().await;
    let path = source(&directory);
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(path.clone(), PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    let review = backend.review_pg_tool_job(id).unwrap();
    assert_eq!(
        backend
            .begin_pg_tool_job(
                PgToolAttemptId::new(),
                profile::CONNECTION_ID.into(),
                PgToolIntent::restore(path, PgToolFormat::Plain, false).unwrap()
            )
            .unwrap_err(),
        PgToolError::Busy
    );
    backend
        .0
        .state
        .pg_tool_jobs
        .begin_connection_teardown(profile::CONNECTION_ID)
        .await;
    assert!(matches!(
        backend.start_pg_tool_job(review),
        Err(PgToolError::StaleReview)
    ));
    let done = wait(&backend, id, |s| s.phase.terminal()).await;
    assert_eq!(done.effect, PgToolEffect::NotStarted);
    backend
        .0
        .state
        .pg_tool_jobs
        .end_connection_teardown(profile::CONNECTION_ID)
        .await;
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_restore_cancellation_remains_owned_until_success_reconciliation() {
    let (directory, backend) = backend().await;
    let path = source(&directory);
    stub(
        &backend,
        Arc::new(|context, _, _| {
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.mark_restore_dispatch();
                context.cancelled().await;
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(path, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    wait(&backend, id, |s| s.phase == PgToolPhase::Running).await;
    backend.cancel_pg_tool_job(id).unwrap();
    let done = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.phase, PgToolPhase::Completed);
    assert_eq!(done.effect, PgToolEffect::Succeeded);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unjoined_cleanup_is_reported_at_absolute_deadline_without_aborting_owner() {
    let owner = crate::postgres::backup::native::Ownership::default();
    let (send, receive) = tokio::sync::oneshot::channel();
    let joined = owner
        .spawn(async move {
            let _ = receive.await;
        })
        .unwrap();
    assert!(owner
        .drain_until(tokio::time::Instant::now() + Duration::from_millis(10))
        .await
        .is_err());
    assert!(!owner.settled());
    send.send(()).unwrap();
    joined.await.unwrap();
    owner
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
}

#[test]
fn intent_bounds_and_source_snapshot_reject_symlinks_and_keep_selected_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let selected = directory.path().join("source.sql");
    std::fs::write(&selected, b"original").unwrap();
    let cancel = crate::postgres::backup::native::source::Cancellation::default();
    let snapshot = crate::postgres::backup::native::source::copy(&selected, &cancel).unwrap();
    std::fs::write(&selected, b"replacement").unwrap();
    assert_eq!(std::fs::read(snapshot.file.path()).unwrap(), b"original");
    #[cfg(unix)]
    {
        let link = directory.path().join("link.sql");
        std::os::unix::fs::symlink(&selected, &link).unwrap();
        assert!(crate::postgres::backup::native::source::copy(&link, &cancel).is_err());
    }
    cancel.cancel();
    assert!(crate::postgres::backup::native::source::copy(&selected, &cancel).is_err());
    assert!(PgToolIntent::restore("relative.sql".into(), PgToolFormat::Plain, false).is_err());
    assert!(PgToolIntent::restore(selected.clone(), PgToolFormat::Plain, true).is_err());
    assert!(PgToolIntent::backup(
        selected,
        PgToolFormat::Custom,
        PgToolScope::Schema {
            schema: "x".repeat(64)
        },
        false
    )
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_publication_preserves_existing_destination_and_never_audits() {
    let (directory, backend) = backend().await;
    let destination = directory.path().join("archive.dump");
    stub(
        &backend,
        Arc::new(|context, _, request| {
            async move {
                let Request::Backup(payload) = request else {
                    panic!("backup")
                };
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                let mut partial = tempfile::NamedTempFile::new_in(
                    std::path::Path::new(&payload.destination_path)
                        .parent()
                        .unwrap(),
                )
                .unwrap();
                std::io::Write::write_all(&mut partial, b"complete archive").unwrap();
                std::fs::write(&payload.destination_path, b"racing existing file").unwrap();
                context.phase(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Backup {
                    partial: Arc::new(partial),
                    destination: payload.destination_path.into(),
                })
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::backup(
                destination.clone(),
                PgToolFormat::Custom,
                PgToolScope::Table {
                    schema: "odd.schema".into(),
                    table: "*quote\"".into(),
                },
                false,
            )
            .unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    let done = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.failure, Some(PgToolError::DestinationExists));
    assert_eq!(std::fs::read(destination).unwrap(), b"racing existing file");
    assert_eq!(
        backend
            .list_pg_tool_jobs(None)
            .unwrap()
            .restore_change_revision,
        0
    );
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(audits, 0);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_interrupts_preparation_waiting_for_lifecycle_gate() {
    let (directory, backend) = backend().await;
    let selected = source(&directory);
    let guard = backend.0.development_gate.lock().await;
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(selected, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    backend.cancel_pg_tool_job(id).unwrap();
    let done = wait(&backend, id, |s| s.phase.terminal()).await;
    assert_eq!(done.effect, PgToolEffect::NotStarted);
    drop(guard);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsupported_connection_and_read_only_policy_refuse_before_source_work() {
    let (directory, backend) = backend().await;
    let missing = directory.path().join("missing.sql");
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            "foreign-endpoint".into(),
            PgToolIntent::restore(missing.clone(), PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    let done = wait(&backend, id, |s| s.phase.terminal()).await;
    assert_eq!(done.failure, Some(PgToolError::StaleReview));
    sqlx::query("UPDATE connections SET read_only=1 WHERE id=?")
        .bind(profile::CONNECTION_ID)
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(missing, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    let done = wait(&backend, id, |s| s.phase.terminal()).await;
    assert_eq!(done.failure, Some(PgToolError::PolicyBlocked));
    assert_eq!(done.effect, PgToolEffect::NotStarted);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_fences_existing_and_new_data_documents_until_cleanup() {
    let (directory, backend) = backend().await;
    let selected = source(&directory);
    let document = backend
        .open_data_document("window", "table", profile::CONNECTION_ID)
        .await
        .unwrap();
    stub(
        &backend,
        Arc::new(|context, _, _| {
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.mark_restore_dispatch();
                context.cancelled().await;
                Err(PgToolJobError::Cancelled)
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(selected, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    wait(&backend, id, |s| s.phase == PgToolPhase::Running).await;
    assert!(backend
        .0
        .tool_jobs
        .restore_in_progress(profile::CONNECTION_ID));
    assert!(backend
        .open_data_document("window", "new", profile::CONNECTION_ID)
        .await
        .is_err());
    let calls = Arc::new(AtomicUsize::new(0));
    let executed = calls.clone();
    let result = backend
        .data_call(&document, move |_, _, _| async move {
            executed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await;
    assert!(result.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    backend.cancel_pg_tool_job(id).unwrap();
    wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert!(!backend
        .0
        .tool_jobs
        .restore_in_progress(profile::CONNECTION_ID));
    let fresh = backend
        .open_data_document("window", "new", profile::CONNECTION_ID)
        .await
        .unwrap();
    assert_eq!(fresh.connection_id(), profile::CONNECTION_ID);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_confirmation_can_be_explicitly_reacquired_but_cancelled_tokens_refuse() {
    let (directory, backend) = backend().await;
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(source(&directory), PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    let PgToolSubmission::NeedsConfirmation(first) = backend
        .start_pg_tool_job(backend.review_pg_tool_job(id).unwrap())
        .unwrap()
    else {
        panic!("strict fixture confirmation")
    };
    drop(first);
    let reviewed = backend.review_pg_tool_job(id).unwrap();
    assert_eq!(reviewed.attempt_id(), id);
    let PgToolSubmission::NeedsConfirmation(second) = backend.start_pg_tool_job(reviewed).unwrap()
    else {
        panic!("reissue confirmation only")
    };
    assert_eq!(
        backend.get_pg_tool_job(id).unwrap().effect,
        PgToolEffect::NotStarted
    );
    backend.cancel_pg_tool_job(id).unwrap();
    assert!(matches!(
        backend.confirm_pg_tool_job(*second),
        Err(PgToolError::StaleReview)
    ));
    wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_finalizing_obeys_the_actual_publication_claim() {
    let (directory, backend) = backend().await;
    let destination = directory.path().join("before-claim.dump");
    let (finish, ready) = tokio::sync::oneshot::channel();
    let ready = Arc::new(std::sync::Mutex::new(Some(ready)));
    stub(
        &backend,
        Arc::new(move |context, _, request| {
            let ready = ready.lock().unwrap().take().unwrap();
            async move {
                let Request::Backup(payload) = request else {
                    panic!("backup")
                };
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                let mut partial = tempfile::NamedTempFile::new_in(
                    std::path::Path::new(&payload.destination_path)
                        .parent()
                        .unwrap(),
                )
                .unwrap();
                std::io::Write::write_all(&mut partial, b"archive").unwrap();
                context.phase(PgToolJobPhase::Finalizing)?;
                let _ = ready.await;
                Ok(Ready::Backup {
                    partial: Arc::new(partial),
                    destination: payload.destination_path.into(),
                })
            }
            .boxed()
        }),
    );
    let first = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            first,
            profile::CONNECTION_ID.into(),
            PgToolIntent::backup(
                destination.clone(),
                PgToolFormat::Custom,
                PgToolScope::Database,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    wait(&backend, first, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, first);
    wait(&backend, first, |s| s.phase == PgToolPhase::Finalizing).await;
    assert_eq!(
        backend.cancel_pg_tool_job(first).unwrap().phase,
        PgToolPhase::Cancelling
    );
    finish.send(()).unwrap();
    let done = wait(&backend, first, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.phase, PgToolPhase::Cancelled);
    assert!(!destination.exists());
    backend.release_pg_tool_job(first).unwrap();
    let (publication_started, started) = tokio::sync::oneshot::channel();
    let (finish, ready) = tokio::sync::oneshot::channel();
    let signals = Arc::new(std::sync::Mutex::new(Some((publication_started, ready))));
    stub(
        &backend,
        Arc::new(move |context, _, _| {
            let (started, finish) = signals.lock().unwrap().take().unwrap();
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.phase(PgToolJobPhase::Finalizing)?;
                Ok(Ready::PublicationTest { started, finish })
            }
            .boxed()
        }),
    );
    let second = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            second,
            profile::CONNECTION_ID.into(),
            PgToolIntent::backup(
                destination,
                PgToolFormat::Custom,
                PgToolScope::Database,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    wait(&backend, second, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, second);
    started.await.unwrap();
    wait(&backend, second, |s| s.phase == PgToolPhase::Finalizing).await;
    assert_eq!(
        backend.cancel_pg_tool_job(second).unwrap().phase,
        PgToolPhase::Finalizing
    );
    finish.send(()).unwrap();
    let done = wait(&backend, second, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Complete
    })
    .await;
    assert_eq!(done.phase, PgToolPhase::Completed);
    backend.shutdown().await.unwrap();
}

#[tokio::test]
async fn consumed_preparation_watch_does_not_turn_into_a_ready_poll_loop() {
    let (send, mut receive) = tokio::sync::watch::channel(false);
    drop(send);
    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        registry::wait_lifecycle_cancel(&mut receive)
    )
    .await
    .is_err());
    let (send, mut receive) = tokio::sync::watch::channel(false);
    send.send_replace(true);
    tokio::time::timeout(
        Duration::from_secs(1),
        registry::wait_lifecycle_cancel(&mut receive),
    )
    .await
    .unwrap();
}

#[test]
fn source_replacement_during_copy_and_changed_length_are_refused() {
    use crate::postgres::backup::native::source;
    let directory = tempfile::tempdir().unwrap();
    let selected = directory.path().join("selected.sql");
    for mutation in 0..3 {
        std::fs::write(&selected, vec![b'x'; 128 * 1024]).unwrap();
        let cancel = source::Cancellation::default();
        let result = source::copy_observed(&selected, &cancel, |copied| {
            if copied != 64 * 1024 {
                return;
            }
            match mutation {
                0 => {
                    std::fs::rename(&selected, directory.path().join("original.sql")).unwrap();
                    std::fs::write(&selected, b"replacement").unwrap();
                }
                1 => {
                    let mut file = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&selected)
                        .unwrap();
                    std::io::Write::write_all(&mut file, b"extra").unwrap();
                }
                _ => {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&selected)
                        .unwrap()
                        .set_len(64 * 1024)
                        .unwrap();
                }
            }
        });
        assert!(matches!(
            result,
            Err(source::Failure {
                error: source::Error::Changed,
                partial: None
            })
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backend_shutdown_reports_unjoined_registered_job_and_keeps_its_owner() {
    let (directory, backend) = backend().await;
    let selected = source(&directory);
    let (finish, blocked) = tokio::sync::oneshot::channel();
    let blocked = Arc::new(std::sync::Mutex::new(Some(blocked)));
    stub(
        &backend,
        Arc::new(move |context, _, _| {
            let blocked = blocked.lock().unwrap().take().unwrap();
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.mark_restore_dispatch();
                let _ = blocked.await;
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(selected, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    wait(&backend, id, |s| s.phase == PgToolPhase::Running).await;
    let started = tokio::time::Instant::now();
    assert!(backend
        .shutdown_with_deadlines(
            started + Duration::from_millis(10),
            started + Duration::from_millis(20)
        )
        .await
        .is_err());
    assert!(!backend.0.tool_jobs.owner.settled());
    assert_eq!(backend.release_pg_tool_job(id), Err(PgToolError::Active));
    finish
        .send(())
        .expect("shutdown must retain, not abort, the registered runner");
    backend
        .0
        .tool_jobs
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    let done = backend.get_pg_tool_job(id).unwrap();
    assert_eq!(done.effect, PgToolEffect::Succeeded);
    assert_eq!(done.cleanup, PgToolCleanup::Complete);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(
        audits, 1,
        "storage remains open for late known-success effects"
    );
    backend.0.state.pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_snapshot_unlink_retains_admission_and_refuses_successful_drain() {
    let (directory, backend) = backend().await;
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(source(&directory), PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    let private = backend
        .0
        .tool_jobs
        .update(id, |entry| {
            entry.source.as_ref().unwrap().file.path().to_owned()
        })
        .unwrap();
    // Only this test's private snapshot path: make unlink deterministically fail.
    std::fs::remove_file(&private).unwrap();
    std::fs::create_dir(&private).unwrap();
    backend.cancel_pg_tool_job(id).unwrap();
    let failed = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Failed
    })
    .await;
    assert_eq!(failed.effect, PgToolEffect::NotStarted);
    assert!(backend
        .0
        .tool_jobs
        .update(id, |entry| entry.source.is_some()
            && entry.admission.is_some())
        .unwrap());
    assert_eq!(backend.release_pg_tool_job(id), Err(PgToolError::Active));
    assert!(backend
        .0
        .tool_jobs
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .is_err());
    assert!(matches!(
        backend.begin_pg_tool_job(
            PgToolAttemptId::new(),
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(source(&directory), PgToolFormat::Plain, false).unwrap()
        ),
        Err(PgToolError::Busy)
    ));
    // Repair only the injected obstruction and exercise the same explicit cleanup.
    std::fs::remove_dir(&private).unwrap();
    std::fs::write(&private, b"owned cleanup fixture").unwrap();
    backend.0.tool_jobs.finish(id, PgToolError::Cancelled).await;
    assert!(!private.exists());
    assert_eq!(
        backend.get_pg_tool_job(id).unwrap().cleanup,
        PgToolCleanup::Complete
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_partial_unlink_failure_keeps_cleanup_fence_and_publication_transfers_ownership() {
    let (directory, backend) = backend().await;
    let (send_path, path) = tokio::sync::oneshot::channel();
    let send_path = Arc::new(std::sync::Mutex::new(Some(send_path)));
    stub(
        &backend,
        Arc::new(move |context, _, _| {
            let send_path = send_path.lock().unwrap().take().unwrap();
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                let partial = Arc::new(tempfile::NamedTempFile::new().unwrap());
                let path = partial.path().to_owned();
                let _owner = context.retain_native_archive(partial.clone()).unwrap();
                std::fs::remove_file(&path).unwrap();
                std::fs::create_dir(&path).unwrap();
                send_path.send(path).unwrap();
                Err(PgToolJobError::Cancelled)
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::backup(
                directory.path().join("archive.sql"),
                PgToolFormat::Plain,
                PgToolScope::Database,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    let private = path.await.unwrap();
    wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Failed
    })
    .await;
    assert_eq!(backend.release_pg_tool_job(id), Err(PgToolError::Active));
    assert!(backend
        .0
        .tool_jobs
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .is_err());
    std::fs::remove_dir(&private).unwrap();
    std::fs::write(&private, b"owned obstruction repair").unwrap();
    let job = backend
        .0
        .tool_jobs
        .update(id, |entry| entry.job_id.clone().unwrap())
        .unwrap();
    backend.0.state.pg_tool_jobs.expire_for_test();
    assert!(
        backend.0.state.pg_tool_jobs.native_observe(&job).is_some(),
        "retention expiry cannot drop failed cleanup owners"
    );
    backend
        .0
        .state
        .pg_tool_jobs
        .native_cleanup_archive(&job)
        .unwrap();
    backend.0.tool_jobs.finish(id, PgToolError::Cancelled).await;
    backend.shutdown().await.unwrap();

    // Successful no-clobber publication consumes only the private file owner.
    let partial = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    std::fs::write(partial.path(), b"exact archive").unwrap();
    let archive = crate::postgres::backup::native::archive::Archive::new(Arc::new(partial));
    let destination = directory.path().join("published.dump");
    archive.publish(destination.clone()).unwrap();
    archive.cleanup().unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"exact archive");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn published_backup_stays_succeeded_when_private_cleanup_fails() {
    let (directory, backend) = backend().await;
    let destination = directory.path().join("published.sql");
    let (send, archive) = tokio::sync::oneshot::channel();
    let send = Arc::new(std::sync::Mutex::new(Some(send)));
    stub(
        &backend,
        Arc::new(move |context, _, request| {
            let send = send.lock().unwrap().take().unwrap();
            async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                let Request::Backup(request) = request else {
                    panic!("backup")
                };
                let destination = std::path::PathBuf::from(request.destination_path);
                let partial = Arc::new(
                    tempfile::NamedTempFile::new_in(destination.parent().unwrap()).unwrap(),
                );
                std::fs::write(partial.path(), b"exact published archive").unwrap();
                let archive = context.retain_native_archive(partial.clone()).unwrap();
                archive.fail_cleanup.store(true, Ordering::Release);
                drop(partial);
                send.send(archive.clone()).ok().unwrap();
                context.phase(PgToolJobPhase::Finalizing)?;
                Ok(Ready::NativeBackup {
                    archive,
                    destination,
                })
            }
            .boxed()
        }),
    );
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::backup(
                destination.clone(),
                PgToolFormat::Plain,
                PgToolScope::Database,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    wait(&backend, id, |s| s.phase == PgToolPhase::ReadyReview).await;
    submit(&backend, id);
    let archive = archive.await.unwrap();
    let done = wait(&backend, id, |s| {
        s.phase.terminal() && s.cleanup == PgToolCleanup::Failed
    })
    .await;
    assert_eq!(done.phase, PgToolPhase::Completed);
    assert_eq!(done.effect, PgToolEffect::Succeeded);
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"exact published archive"
    );
    assert_eq!(backend.release_pg_tool_job(id), Err(PgToolError::Active));
    archive.fail_cleanup.store(false, Ordering::Release);
    let job = backend
        .0
        .tool_jobs
        .update(id, |entry| entry.job_id.clone().unwrap())
        .unwrap();
    backend
        .0
        .state
        .pg_tool_jobs
        .native_cleanup_archive(&job)
        .unwrap();
    // Restore the successful terminal observation after the injected cleanup fault.
    backend
        .0
        .tool_jobs
        .update(id, |entry| {
            entry.admission.take();
            entry.intent.take();
            entry.observation.cleanup = PgToolCleanup::Complete;
        })
        .unwrap();
    backend.shutdown().await.unwrap();
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"exact published archive"
    );
}
