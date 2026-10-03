use super::*;
use crate::{
    backend::profile,
    postgres::transfer::{protocol as legacy, runner},
};
use futures_util::FutureExt;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
pub(super) async fn backend() -> (tempfile::TempDir, Backend) {
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
        panic!("PostgreSQL")
    };
    pg.safe_mode = crate::SafeMode::Strict;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
    *backend.0.csv_transfers.test_inspector.lock().unwrap() = Some(Arc::new(|_, payload, _| {
        async move {
            let mut review = runner::test_review(&payload.connection_id);
            review.inspection.schema = payload.schema.clone();
            review.inspection.table = payload.table.clone();
            review.inspection.direction = payload.direction;
            review.inspection.options = payload.options.clone();
            if payload.direction == legacy::Direction::Import {
                review.inspection.file_name = Some("selected.csv".into());
                review.inspection.total_bytes = Some(32);
                review.inspection.source_columns = vec![
                    legacy::SourceColumn {
                        index: 0,
                        name: "duplicate".into(),
                    },
                    legacy::SourceColumn {
                        index: 1,
                        name: "duplicate".into(),
                    },
                ];
                review.inspection.sample_rows = vec![
                    vec![Some("9223372036854775807".into()), Some("日本語".into())],
                    vec![None, Some(String::new())],
                ];
            }
            review.payload = payload;
            Ok(review)
        }
        .boxed()
    }));
    (directory, backend)
}
pub(super) fn target() -> CsvTarget {
    CsvTarget {
        schema: "public".into(),
        table: "transfer_test".into(),
    }
}
fn intent(import: bool) -> CsvInspectionIntent {
    if import {
        CsvInspectionIntent::import(
            PathBuf::from("/tmp/owned-test-not-opened.csv"),
            target(),
            CsvOptions::default(),
        )
        .unwrap()
    } else {
        CsvInspectionIntent::export(target(), CsvOptions::default()).unwrap()
    }
}
async fn inspect(backend: &Backend, import: bool) -> CsvInspection {
    let id = CsvInspectionId::new();
    backend
        .begin_csv_inspection(id, profile::CONNECTION_ID.into(), intent(import))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = backend.get_csv_inspection(id).unwrap();
            if row.phase == CsvInspectionPhase::Ready {
                return backend.csv_inspection(id).unwrap();
            }
            assert!(
                !matches!(
                    row.phase,
                    CsvInspectionPhase::Failed | CsvInspectionPhase::Cancelled
                ),
                "{row:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn terminal(backend: &Backend, id: CsvTransferAttemptId) -> CsvTransferObservation {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let row = backend.get_csv_transfer(id).unwrap();
            if row.phase.terminal() && row.cleanup != CsvCleanup::Pending {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn reviewed(backend: &Backend, inspection: CsvInspection) -> CsvTransferReview {
    backend
        .review_csv_import(
            inspection,
            vec![CsvMapping {
                source_index: 1,
                target_column: "value".into(),
            }],
        )
        .unwrap()
}
fn submit(backend: &Backend, id: CsvTransferAttemptId, review: CsvTransferReview) {
    if let CsvTransferSubmission::NeedsConfirmation(confirmation) =
        backend.begin_csv_transfer(id, review).unwrap()
    {
        backend.confirm_csv_transfer(*confirmation).unwrap();
    }
}
fn runner(backend: &Backend, run: registry::TestRunner) {
    *backend.0.csv_transfers.test_runner.lock().unwrap() = Some(run);
}

#[test]
fn options_and_identity_refuse_before_unbounded_clone_and_keep_exact_dialect() {
    let options = CsvOptions::default();
    assert!(options.validate().is_ok());
    let mut large = options.clone();
    large.null_token = "x".repeat(1025);
    assert_eq!(large.validate(), Err(CsvError::InvalidOptions));
    let mut capacity = options.clone();
    capacity.null_token.reserve(8192);
    assert!(capacity.checked_heap_bytes().is_none());
    let mut distinct = options;
    distinct.escape = "\\".into();
    distinct.null_token = String::new();
    assert!(distinct.validate().is_ok());
    assert!(CsvInspectionId::parse("00000000-0000-0000-0000-000000000000").is_err());
    assert!(
        CsvInspectionIntent::import(PathBuf::from("relative.csv"), target(), distinct).is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn eight_ready_inspections_are_separate_from_active_admission_and_expire_on_observation() {
    let (_directory, backend) = backend().await;
    let mut ids = Vec::new();
    for _ in 0..MAX_CSV_INSPECTIONS {
        let handle = inspect(&backend, true).await;
        ids.push(handle.inspection_id());
    }
    assert!(backend
        .0
        .state
        .pg_transfers
        .admission(profile::CONNECTION_ID)
        .is_ok());
    assert_eq!(
        backend
            .begin_csv_inspection(
                CsvInspectionId::new(),
                profile::CONNECTION_ID.into(),
                intent(true)
            )
            .unwrap_err(),
        CsvError::Busy
    );
    let handle = backend.csv_inspection(ids[0]).unwrap();
    assert_eq!(
        handle.data().source_columns[0].name,
        handle.data().source_columns[1].name
    );
    assert_eq!(
        handle.data().sample_rows[0][0].as_deref(),
        Some("9223372036854775807")
    );
    assert!(handle.retained_bytes() <= MAX_CSV_INSPECTION_BYTES);
    assert!(!format!("{handle:?}").contains("922337"));
    drop(handle);
    backend
        .0
        .csv_transfers
        .state
        .lock()
        .unwrap()
        .inspections
        .get_mut(&ids[0])
        .unwrap()
        .created = std::time::Instant::now() - Duration::from_secs(301);
    assert_eq!(
        backend
            .list_csv_inspections(None)
            .unwrap()
            .inspections
            .len(),
        7
    );
    assert_eq!(
        backend.get_csv_inspection(ids[0]).unwrap_err(),
        CsvError::Missing
    );
    for id in ids {
        backend.release_csv_inspection(id).unwrap();
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_confirmation_reacquisition_and_consumed_inspection_release_do_not_replay() {
    let (_directory, backend) = backend().await;
    let inspection = inspect(&backend, true).await;
    let inspection_id = inspection.inspection_id();
    let review = reviewed(&backend, inspection);
    let id = CsvTransferAttemptId::new();
    let CsvTransferSubmission::NeedsConfirmation(old) =
        backend.begin_csv_transfer(id, review).unwrap()
    else {
        panic!("strict profile")
    };
    backend.release_csv_inspection(inspection_id).unwrap();
    let reacquired = backend.review_csv_transfer(id).unwrap();
    assert_eq!(reacquired.attempt_id(), Some(id));
    assert_eq!(
        backend
            .begin_csv_transfer(CsvTransferAttemptId::new(), reacquired.clone())
            .err(),
        Some(CsvError::StaleReview)
    );
    assert!(matches!(
        backend.begin_csv_transfer(id, reacquired).unwrap(),
        CsvTransferSubmission::NeedsConfirmation(_)
    ));
    backend.cancel_csv_transfer(id).unwrap();
    assert_eq!(
        backend.confirm_csv_transfer(*old).err(),
        Some(CsvError::StaleReview)
    );
    assert_eq!(
        backend.get_csv_transfer(id).unwrap().effect,
        CsvEffect::NotApplied
    );
    backend.release_csv_transfer(id).unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execution_workspace_refuses_before_dispatch_and_does_not_audit() {
    let (_directory, backend) = backend().await;
    let review = reviewed(&backend, inspect(&backend, true).await);
    let id = CsvTransferAttemptId::new();
    let CsvTransferSubmission::NeedsConfirmation(confirmation) =
        backend.begin_csv_transfer(id, review).unwrap()
    else {
        panic!("confirmation")
    };
    let reserved = backend
        .0
        .csv_transfers
        .executions
        .clone()
        .try_acquire_many_owned(2)
        .unwrap();
    assert_eq!(
        backend.confirm_csv_transfer(*confirmation).err(),
        Some(CsvError::WorkBudget)
    );
    let row = backend.get_csv_transfer(id).unwrap();
    assert_eq!(row.phase, CsvTransferPhase::Failed);
    assert_eq!(row.effect, CsvEffect::NotApplied);
    assert_eq!(row.cleanup, CsvCleanup::Complete);
    assert_eq!(
        backend
            .list_csv_transfers(None)
            .unwrap()
            .execution_reserved_bytes,
        MAX_CSV_EXECUTION_POOL_BYTES
    );
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(audits, 0);
    drop(reserved);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_and_unknown_imports_invalidate_once_and_only_known_success_audits() {
    let (_directory, backend) = backend().await;
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();
    runner(
        &backend,
        Arc::new(move |context, _, _, request| {
            let counter = counter.clone();
            async move {
                let runner::RunRequest::Import { mapping } = request else {
                    panic!("import")
                };
                assert_eq!(mapping[0].source_index, 1);
                context.progress(32, Some(2));
                assert!(context.begin_finalizing());
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    context.succeeded(Some(1));
                    Ok(())
                } else {
                    Err(legacy::TransferError::OutcomeUnknown)
                }
            }
            .boxed()
        }),
    );
    for revision in 1..=2 {
        let id = CsvTransferAttemptId::new();
        submit(
            &backend,
            id,
            reviewed(&backend, inspect(&backend, true).await),
        );
        let row = terminal(&backend, id).await;
        assert_eq!(row.import_change_revision, Some(revision));
        assert_eq!(row.cleanup, CsvCleanup::Complete);
        assert_eq!(
            row.effect,
            if revision == 1 {
                CsvEffect::Succeeded
            } else {
                CsvEffect::Unknown
            }
        );
        assert_eq!(
            row.rows_committed,
            if revision == 1 { Some(1) } else { None }
        );
        backend.release_csv_transfer(id).unwrap();
    }
    let list = backend.list_csv_transfers(None).unwrap();
    assert_eq!(list.import_change_revision, 2);
    assert!(list.jobs.is_empty());
    assert_eq!(list.execution_reserved_bytes, 0);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM safety_overrides WHERE command='start_pg_csv_import'",
    )
    .fetch_one(&backend.0.state.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_preserves_bounded_parser_diagnostics_without_sample_content() {
    let (_directory, backend) = backend().await;
    *backend.0.csv_transfers.test_inspector.lock().unwrap() = Some(Arc::new(|_, _, _| {
        async {
            Err(legacy::TransferError::Csv {
                record: 51,
                column: Some(3),
                reason: "field exceeds the 1 MiB limit".into(),
            })
        }
        .boxed()
    }));
    let id = CsvInspectionId::new();
    backend
        .begin_csv_inspection(id, profile::CONNECTION_ID.into(), intent(true))
        .unwrap();
    let row = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let row = backend.get_csv_inspection(id).unwrap();
            if row.phase == CsvInspectionPhase::Failed {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(row.failure, Some(CsvError::Csv));
    let d = row.diagnostic.unwrap();
    assert_eq!(d.record, Some(51));
    assert_eq!(d.column, Some(3));
    assert_eq!(d.reason, "field exceeds the 1 MiB limit");
    assert_eq!(row.cleanup, CsvCleanup::Complete);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_blocking_inspection_retains_admission_until_real_worker_join() {
    let (_directory, backend) = backend().await;
    let (release, held) = std::sync::mpsc::channel();
    let held = Arc::new(std::sync::Mutex::new(Some(held)));
    let started = Arc::new(tokio::sync::Notify::new());
    let notice = started.clone();
    *backend.0.csv_transfers.test_inspector.lock().unwrap() = Some(Arc::new(move |_, _, io| {
        let held = held.lock().unwrap().take().unwrap();
        let notice = notice.clone();
        async move {
            io.file_work(move || {
                notice.notify_one();
                held.recv().unwrap();
                Ok(())
            })
            .await?;
            Err(legacy::TransferError::Cancelled)
        }
        .boxed()
    }));
    let id = CsvInspectionId::new();
    backend
        .begin_csv_inspection(id, profile::CONNECTION_ID.into(), intent(true))
        .unwrap();
    started.notified().await;
    backend.cancel_csv_inspection(id).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(backend.release_csv_inspection(id), Err(CsvError::Active));
    assert!(!backend.0.csv_transfers.owner.settled());
    release.send(()).unwrap();
    let row = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = backend.get_csv_inspection(id).unwrap();
            if row.cleanup == CsvCleanup::Complete {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(row.phase, CsvInspectionPhase::Cancelled);
    backend.release_csv_inspection(id).unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn import_retires_old_data_and_refuses_new_data_and_restore_until_cleanup() {
    use crate::backend::pg_tools::{
        PgToolAttemptId, PgToolError, PgToolFormat, PgToolIntent, PgToolPhase,
    };
    let (directory, backend) = backend().await;
    let old = backend
        .open_data_document("window", "old", profile::CONNECTION_ID)
        .await
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let notice = started.clone();
    runner(
        &backend,
        Arc::new(move |context, _, _, _| {
            let notice = notice.clone();
            async move {
                context.progress(1, Some(0));
                notice.notify_one();
                context.cancelled().await;
                Err(legacy::TransferError::Cancelled)
            }
            .boxed()
        }),
    );
    let id = CsvTransferAttemptId::new();
    submit(
        &backend,
        id,
        reviewed(&backend, inspect(&backend, true).await),
    );
    started.notified().await;
    assert!(backend
        .0
        .csv_transfers
        .import_in_progress(profile::CONNECTION_ID));
    assert!(backend
        .open_data_document("window", "new", profile::CONNECTION_ID)
        .await
        .is_err());
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    assert!(backend
        .data_call(&old, move |_, _, _| async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let source = directory.path().join("owned.sql");
    std::fs::write(&source, b"SELECT 1;\n").unwrap();
    let restore = PgToolAttemptId::new();
    backend
        .begin_pg_tool_job(
            restore,
            profile::CONNECTION_ID.into(),
            PgToolIntent::restore(source, PgToolFormat::Plain, false).unwrap(),
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = backend.get_pg_tool_job(restore).unwrap();
            if row.phase == PgToolPhase::ReadyReview {
                break;
            }
            assert!(!row.phase.terminal(), "{row:?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let submission = backend
        .start_pg_tool_job(backend.review_pg_tool_job(restore).unwrap())
        .unwrap();
    if let crate::backend::pg_tools::PgToolSubmission::NeedsConfirmation(confirm) = submission {
        backend.confirm_pg_tool_job(*confirm).unwrap();
    }
    let refused = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = backend.get_pg_tool_job(restore).unwrap();
            if row.phase.terminal() {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(refused.failure, Some(PgToolError::Busy));
    backend.cancel_csv_transfer(id).unwrap();
    assert_eq!(terminal(&backend, id).await.effect, CsvEffect::NotApplied);
    assert!(!backend
        .0
        .csv_transfers
        .import_in_progress(profile::CONNECTION_ID));
    backend
        .open_data_document("window", "new", profile::CONNECTION_ID)
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_deadline_retains_registered_runner_and_execution_workspace_until_actual_join() {
    let (_directory, backend) = backend().await;
    let (release, held) = tokio::sync::oneshot::channel();
    let held = Arc::new(std::sync::Mutex::new(Some(held)));
    let started = Arc::new(tokio::sync::Notify::new());
    let notice = started.clone();
    runner(
        &backend,
        Arc::new(move |context, _, _, _| {
            let held = held.lock().unwrap().take().unwrap();
            let notice = notice.clone();
            async move {
                context.progress(1, Some(0));
                notice.notify_one();
                let _ = held.await;
                Err(legacy::TransferError::Cancelled)
            }
            .boxed()
        }),
    );
    let id = CsvTransferAttemptId::new();
    submit(
        &backend,
        id,
        reviewed(&backend, inspect(&backend, true).await),
    );
    started.notified().await;
    let now = tokio::time::Instant::now();
    assert!(backend
        .shutdown_with_deadlines(
            now + Duration::from_millis(10),
            now + Duration::from_millis(20)
        )
        .await
        .is_err());
    assert!(!backend.0.csv_transfers.owner.settled());
    assert_eq!(
        backend
            .list_csv_transfers(None)
            .unwrap()
            .execution_reserved_bytes,
        MAX_CSV_EXECUTION_BYTES
    );
    assert_eq!(backend.release_csv_transfer(id), Err(CsvError::Active));
    release.send(()).expect("owned runner must not be aborted");
    backend
        .0
        .csv_transfers
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(
        backend.get_csv_transfer(id).unwrap().cleanup,
        CsvCleanup::Complete
    );
    assert_eq!(
        backend
            .list_csv_transfers(None)
            .unwrap()
            .execution_reserved_bytes,
        0
    );
    backend.0.state.pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retirement_cancels_confirmation_and_blocks_synchronous_registration_through_mutation() {
    let (_directory, backend) = backend().await;
    let id = CsvTransferAttemptId::new();
    let CsvTransferSubmission::NeedsConfirmation(confirmation) = backend
        .begin_csv_transfer(id, reviewed(&backend, inspect(&backend, true).await))
        .unwrap()
    else {
        panic!("strict")
    };
    let gate = backend.0.development_gate.lock().await;
    let guard = retire_connection(
        &backend.0,
        Some(profile::CONNECTION_ID),
        tokio::time::Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(
        backend.get_csv_transfer(id).unwrap().phase,
        CsvTransferPhase::Cancelled
    );
    assert_eq!(
        backend
            .begin_csv_inspection(
                CsvInspectionId::new(),
                profile::CONNECTION_ID.into(),
                intent(true)
            )
            .unwrap_err(),
        CsvError::Closing
    );
    assert!(backend.confirm_csv_transfer(*confirmation).is_err());
    assert!(backend
        .0
        .state
        .pg_transfers
        .admission(profile::CONNECTION_ID)
        .is_ok());
    drop(guard);
    drop(gate);
    let next = inspect(&backend, true).await;
    backend
        .release_csv_inspection(next.inspection_id())
        .unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retirement_deadline_refuses_mutation_and_preserves_registered_worker() {
    let (_directory, backend) = backend().await;
    let (release, held) = tokio::sync::oneshot::channel();
    let held = Arc::new(std::sync::Mutex::new(Some(held)));
    let started = Arc::new(tokio::sync::Notify::new());
    let notice = started.clone();
    runner(
        &backend,
        Arc::new(move |context, _, _, _| {
            let held = held.lock().unwrap().take().unwrap();
            let notice = notice.clone();
            async move {
                context.progress(1, Some(0));
                notice.notify_one();
                let _ = held.await;
                Err(legacy::TransferError::Cancelled)
            }
            .boxed()
        }),
    );
    let id = CsvTransferAttemptId::new();
    submit(
        &backend,
        id,
        reviewed(&backend, inspect(&backend, true).await),
    );
    started.notified().await;
    let gate = backend.0.development_gate.lock().await;
    assert!(retire_connection(
        &backend.0,
        None,
        tokio::time::Instant::now() + Duration::from_millis(20)
    )
    .await
    .is_err());
    assert_eq!(
        backend
            .list_csv_transfers(None)
            .unwrap()
            .execution_reserved_bytes,
        MAX_CSV_EXECUTION_BYTES
    );
    assert_eq!(backend.release_csv_transfer(id), Err(CsvError::Active));
    release
        .send(())
        .expect("retirement preserves the real worker");
    let guard = retire_connection(
        &backend.0,
        None,
        tokio::time::Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    // This returned while the facade monitor still needs the held gate.
    assert!(backend
        .0
        .csv_transfers
        .import_in_progress(profile::CONNECTION_ID));
    drop(guard);
    drop(gate);
    assert_eq!(terminal(&backend, id).await.cleanup, CsvCleanup::Complete);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_connection_edit_retires_confirmation_and_refuses_unresolved_cleanup() {
    use crate::backend::{
        DevelopmentEnvironment, DevelopmentPostgresConnection, DevelopmentSafeMode,
        DevelopmentStorageMode,
    };
    const CHILD: &str = "DBUNK_CSV_CONNECTION_RETIRE_TEST";
    if std::env::var_os(CHILD).is_none() {
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","backend::csv_transfers::tests::native_connection_edit_retires_confirmation_and_refuses_unresolved_cleanup","--nocapture"]).env(CHILD,"1").output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let backend =
        Backend::create_native_profile(&directory.path().canonicalize().unwrap().join("profile"))
            .await
            .unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    let form = |name: &str| DevelopmentPostgresConnection {
        name: name.into(),
        host: "not-contacted.invalid".into(),
        port: 5432,
        database: "synthetic".into(),
        user: "synthetic".into(),
        environment: DevelopmentEnvironment::Test,
        safe_mode: DevelopmentSafeMode::Strict,
        read_only: false,
        tls: Default::default(),
        driver_options: Default::default(),
        ssh_tunnel: None,
    };
    let saved = backend
        .save_development_connection(None, form("original"), "synthetic secret".into())
        .await
        .unwrap();
    *backend.0.csv_transfers.test_inspector.lock().unwrap() = Some(Arc::new(|_, payload, _| {
        async move {
            let mut review = runner::test_review(&payload.connection_id);
            review.inspection.direction = payload.direction;
            review.inspection.file_name = Some("selected.csv".into());
            review.inspection.source_columns = vec![legacy::SourceColumn {
                index: 0,
                name: "value".into(),
            }];
            review.payload = payload;
            Ok(review)
        }
        .boxed()
    }));
    let inspection_id = CsvInspectionId::new();
    backend
        .begin_csv_inspection(inspection_id, saved.id.clone(), intent(true))
        .unwrap();
    let inspection = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(value) = backend.csv_inspection(inspection_id) {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let review = backend
        .review_csv_import(
            inspection,
            vec![CsvMapping {
                source_index: 0,
                target_column: "value".into(),
            }],
        )
        .unwrap();
    let id = CsvTransferAttemptId::new();
    let CsvTransferSubmission::NeedsConfirmation(confirmation) =
        backend.begin_csv_transfer(id, review).unwrap()
    else {
        panic!("strict")
    };
    let edited = backend
        .save_development_connection(Some(saved.id.clone()), form("renamed"), String::new())
        .await
        .unwrap();
    assert_eq!(edited.name, "renamed");
    assert_eq!(
        backend.get_csv_transfer(id).unwrap().phase,
        CsvTransferPhase::Cancelled
    );
    assert!(backend.confirm_csv_transfer(*confirmation).is_err());
    // A recorded failed cleanup must stop both credential and metadata paths,
    // even though the job has a terminal effect and no active SQL worker.
    backend
        .0
        .csv_transfers
        .state
        .lock()
        .unwrap()
        .jobs
        .get_mut(&id)
        .unwrap()
        .observation
        .cleanup = CsvCleanup::Failed;
    assert!(backend
        .save_development_connection(
            Some(saved.id.clone()),
            form("must not save"),
            "replacement".into()
        )
        .await
        .is_err());
    assert!(backend
        .delete_development_connection(saved.id.clone())
        .await
        .is_err());
    assert!(backend.reset_development_credentials(true).await.is_err());
    let stored = crate::storage::read_connection_by_id(&backend.0.state.pool, &saved.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.name(), "renamed");
    let hydrated = crate::app::find_connection(&backend.0.state, &saved.id)
        .await
        .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = hydrated else {
        panic!("PostgreSQL")
    };
    assert_eq!(pg.password, "synthetic secret");
    backend
        .0
        .csv_transfers
        .state
        .lock()
        .unwrap()
        .jobs
        .get_mut(&id)
        .unwrap()
        .observation
        .cleanup = CsvCleanup::Complete;
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consuming_lock_refuses_replaced_same_id_inspection_without_consuming_new_review() {
    let (_directory, backend) = backend().await;
    let old = inspect(&backend, true).await;
    let id = old.inspection_id();
    let old_review = reviewed(&backend, old.clone()); // preflight succeeded
    backend.release_csv_inspection(id).unwrap();
    backend
        .begin_csv_inspection(id, profile::CONNECTION_ID.into(), intent(true))
        .unwrap();
    let fresh = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(handle) = backend.csv_inspection(id) {
                break handle;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    {
        let mut state = backend.0.csv_transfers.state.lock().unwrap();
        assert!(matches!(
            registry::take_inspection(&mut state, old_review.inspection()),
            Err(CsvError::StaleReview)
        ));
        assert!(Arc::ptr_eq(
            state.inspections.get(&id).unwrap().ready.as_ref().unwrap(),
            &fresh.ready
        ));
    }
    let attempt = CsvTransferAttemptId::new();
    assert!(matches!(
        backend
            .begin_csv_transfer(attempt, reviewed(&backend, fresh))
            .unwrap(),
        CsvTransferSubmission::NeedsConfirmation(_)
    ));
    backend.cancel_csv_transfer(attempt).unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dispatch_lock_refuses_old_confirmation_after_same_attempt_id_replacement() {
    let (_directory, backend) = backend().await;
    let id = CsvTransferAttemptId::new();
    let CsvTransferSubmission::NeedsConfirmation(old) = backend
        .begin_csv_transfer(id, reviewed(&backend, inspect(&backend, true).await))
        .unwrap()
    else {
        panic!("strict")
    };
    let expected = old.review.data.clone(); // confirmation preflight already matched
    backend.cancel_csv_transfer(id).unwrap();
    backend.release_csv_transfer(id).unwrap();
    let CsvTransferSubmission::NeedsConfirmation(new) = backend
        .begin_csv_transfer(id, reviewed(&backend, inspect(&backend, true).await))
        .unwrap()
    else {
        panic!("strict")
    };
    assert_ne!(old.review.inspection_id(), new.review.inspection_id());
    assert_eq!(
        backend
            .0
            .csv_transfers
            .dispatch(&backend, id, true, &expected),
        Err(CsvError::StaleReview)
    );
    assert_eq!(
        backend.get_csv_transfer(id).unwrap().phase,
        CsvTransferPhase::AwaitingConfirmation
    );
    assert_eq!(
        backend
            .list_csv_transfers(None)
            .unwrap()
            .execution_reserved_bytes,
        0
    );
    assert!(backend.confirm_csv_transfer(*old).is_err());
    backend.cancel_csv_transfer(id).unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_between_registration_and_worker_admission_finishes_without_orphan() {
    let (_directory, backend) = backend().await;
    let id = CsvTransferAttemptId::new();
    let CsvTransferSubmission::NeedsConfirmation(review) = backend
        .begin_csv_transfer(id, reviewed(&backend, inspect(&backend, true).await))
        .unwrap()
    else {
        panic!("strict")
    };
    // Reproduce the auto-start interval: entry exists as Preparing, and no
    // execution permit or worker has yet been admitted.
    backend
        .0
        .csv_transfers
        .state
        .lock()
        .unwrap()
        .jobs
        .get_mut(&id)
        .unwrap()
        .observation
        .phase = CsvTransferPhase::Preparing;
    let cancelled = backend.cancel_csv_transfer(id).unwrap();
    assert_eq!(cancelled.phase, CsvTransferPhase::Cancelled);
    assert_eq!(cancelled.cleanup, CsvCleanup::Complete);
    assert_eq!(cancelled.effect, CsvEffect::NotApplied);
    assert_eq!(
        backend
            .0
            .csv_transfers
            .dispatch(&backend, id, false, &review.review.data),
        Err(CsvError::StaleReview)
    );
    backend.release_csv_transfer(id).unwrap();
    assert!(backend
        .0
        .state
        .pg_transfers
        .admission(profile::CONNECTION_ID)
        .is_ok());
    assert!(backend.0.csv_transfers.owner.settled());
    backend.shutdown().await.unwrap();
}
