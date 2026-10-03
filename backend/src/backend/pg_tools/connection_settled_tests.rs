//! Connection mutation refuses unresolved native work without waiting for a
//! monitor that may need the caller's development gate. No real clients run.
use super::*;
use crate::{
    backend::profile,
    postgres::backup::runner::{Ready, Request},
};
use futures_util::FutureExt;

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
            let row = backend.get_pg_tool_job(id).unwrap();
            if predicate(&row) {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}

fn require(backend: &Backend, connection: Option<&str>) -> Result<(), String> {
    crate::backend::pg_tools::require_connection_settled(&backend.0, connection)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparing_and_confirmation_refuse_edits_until_cancelled_and_joined() {
    let (_profile, backend) = backend().await;
    let files = tempfile::tempdir().unwrap();
    let source = files.path().join("owned.sql");
    std::fs::write(&source, b"SELECT 'owned';\n").unwrap();
    let gate = backend.0.development_gate.lock().await;
    require(&backend, None).unwrap();
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(source, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    assert!(require(&backend, Some(profile::CONNECTION_ID)).is_err());
    assert!(require(&backend, None).is_err());
    require(&backend, Some("unrelated-connection")).unwrap();
    assert!(backend
        .0
        .tool_jobs
        .update(id, |entry| entry.target.is_none())
        .unwrap());
    drop(gate);
    wait(&backend, id, |row| row.phase == PgToolPhase::ReadyReview).await;
    let review = backend.review_pg_tool_job(id).unwrap();
    let PgToolSubmission::NeedsConfirmation(confirmation) =
        backend.start_pg_tool_job(review).unwrap()
    else {
        panic!("restore requires explicit confirmation")
    };
    let gate = backend.0.development_gate.lock().await;
    assert!(require(&backend, Some(profile::CONNECTION_ID)).is_err());
    backend.cancel_pg_tool_job(id).unwrap();
    wait(&backend, id, |row| row.cleanup == PgToolCleanup::Complete).await;
    require(&backend, Some(profile::CONNECTION_ID)).unwrap();
    assert!(matches!(
        backend.confirm_pg_tool_job(*confirmation),
        Err(PgToolError::StaleReview)
    ));
    drop(gate);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_registered_after_settled_check_cannot_cross_generation_fence() {
    let (_profile, backend) = backend().await;
    let files = tempfile::tempdir().unwrap();
    let gate = backend.0.development_gate.lock().await;
    require(&backend, Some(profile::CONNECTION_ID)).unwrap();
    // A synchronous registration can arrive after the check. It cannot hydrate
    // while this gate is held, and the following mutation fence retires its
    // captured generation before the gate permits preparation to continue.
    let id = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            id,
            profile::CONNECTION_ID.into(),
            PgToolIntent::backup(
                files.path().join("late.sql"),
                PgToolFormat::Plain,
                PgToolScope::Database,
                false,
            )
            .unwrap(),
        )
        .unwrap();
    backend
        .0
        .state
        .pg_tool_jobs
        .begin_connection_teardown(profile::CONNECTION_ID)
        .await;
    let row = wait(&backend, id, |row| row.cleanup == PgToolCleanup::Complete).await;
    assert_eq!(row.phase, PgToolPhase::Cancelled);
    assert_eq!(row.effect, PgToolEffect::NotStarted);
    assert!(backend
        .0
        .tool_jobs
        .update(id, |entry| entry.target.is_none() && entry.job_id.is_none())
        .unwrap());
    backend
        .0
        .state
        .pg_tool_jobs
        .end_connection_teardown(profile::CONNECTION_ID)
        .await;
    require(&backend, Some(profile::CONNECTION_ID)).unwrap();
    assert!(matches!(
        backend.review_pg_tool_job(id),
        Err(PgToolError::StaleReview)
    ));
    drop(gate);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_and_published_backup_with_failed_cleanup_refuse_connection_mutation() {
    let (_profile, backend) = backend().await;
    let files = tempfile::tempdir().unwrap();
    let destination = files.path().join("owned.sql");
    let (running, entered) = tokio::sync::oneshot::channel();
    let (finish, resume) = tokio::sync::oneshot::channel();
    let (archive_sender, archive_receiver) = tokio::sync::oneshot::channel();
    let inputs = Arc::new(Mutex::new(Some((running, resume, archive_sender))));
    *backend.0.tool_jobs.test_runner.lock().unwrap() =
        Some(Arc::new(move |context, _, request| {
            let (running, resume, archive_sender) = inputs.lock().unwrap().take().unwrap();
            async move {
                context.phase(legacy::PgToolJobPhase::Preflight)?;
                context.phase(legacy::PgToolJobPhase::Running)?;
                running.send(()).unwrap();
                resume.await.unwrap();
                let Request::Backup(request) = request else {
                    panic!("backup")
                };
                let destination = std::path::PathBuf::from(request.destination_path);
                let partial = Arc::new(
                    tempfile::NamedTempFile::new_in(destination.parent().unwrap()).unwrap(),
                );
                std::fs::write(partial.path(), b"owned published bytes").unwrap();
                let archive = context.retain_native_archive(partial.clone()).unwrap();
                archive.fail_cleanup.store(true, Ordering::Release);
                drop(partial);
                archive_sender.send(archive.clone()).ok().unwrap();
                context.phase(legacy::PgToolJobPhase::Finalizing)?;
                Ok(Ready::NativeBackup {
                    archive,
                    destination,
                })
            }
            .boxed()
        }));
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
    wait(&backend, id, |row| row.phase == PgToolPhase::ReadyReview).await;
    assert!(matches!(
        backend
            .start_pg_tool_job(backend.review_pg_tool_job(id).unwrap())
            .unwrap(),
        PgToolSubmission::Accepted(_)
    ));
    tokio::time::timeout(Duration::from_secs(3), entered)
        .await
        .unwrap()
        .unwrap();
    let gate = backend.0.development_gate.lock().await;
    assert!(require(&backend, Some(profile::CONNECTION_ID)).is_err());
    require(&backend, Some("unrelated-connection")).unwrap();
    finish.send(()).unwrap();
    let archive = tokio::time::timeout(Duration::from_secs(3), archive_receiver)
        .await
        .unwrap()
        .unwrap();
    let row = wait(&backend, id, |row| row.cleanup == PgToolCleanup::Failed).await;
    assert_eq!(row.phase, PgToolPhase::Completed);
    assert_eq!(row.effect, PgToolEffect::Succeeded);
    assert!(require(&backend, Some(profile::CONNECTION_ID)).is_err());
    assert!(require(&backend, None).is_err());
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"owned published bytes"
    );
    // Resolve only this injected failure through the same checked resource
    // cleanup path. Published output is never removed by that cleanup.
    archive.fail_cleanup.store(false, Ordering::Release);
    assert!(
        backend
            .0
            .tool_jobs
            .cleanup_resources(id, Some(backend.0.state.pg_tool_jobs.clone()))
            .await
    );
    backend
        .0
        .tool_jobs
        .update(id, |entry| {
            entry.observation.cleanup = PgToolCleanup::Complete
        })
        .unwrap();
    require(&backend, Some(profile::CONNECTION_ID)).unwrap();
    require(&backend, None).unwrap();
    drop(gate);
    backend.shutdown().await.unwrap();
    assert_eq!(
        std::fs::read(destination).unwrap(),
        b"owned published bytes"
    );
}
