use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use super::manager::{transition, JobContext};
use super::protocol::*;
use super::runner::{self, Ready, Request};
use super::PgToolJobManager;

pub(crate) fn backup(path: &std::path::Path) -> StartPgBackupPayload {
    StartPgBackupPayload {
        connection_id: "connection".into(),
        destination_path: path.to_string_lossy().into_owned(),
        format: PgBackupFormat::Plain,
        scope: PgBackupScope::Database,
        clean: false,
    }
}
fn snapshot(id: &str) -> PgToolJobSnapshot {
    let mut p = backup(std::path::Path::new("/private/archive.sql"));
    p.connection_id = id.into();
    Request::Backup(p).snapshot()
}
pub(crate) async fn terminal(manager: &PgToolJobManager, id: &str) -> PgToolJobSnapshot {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let snapshot = manager.get(id).unwrap();
            if snapshot.phase.terminal() {
                // Completion effects precede release/admission becoming available.
                if manager.inner_finished_for_test(id) {
                    return snapshot;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("terminal job")
}
async fn successful(context: JobContext) -> Result<Ready, PgToolJobError> {
    context.phase(PgToolJobPhase::Preflight)?;
    context.phase(PgToolJobPhase::Running)?;
    context.phase(PgToolJobPhase::Finalizing)?;
    Ok(Ready::Restore)
}
async fn cancellable(context: JobContext) -> Result<Ready, PgToolJobError> {
    context.phase(PgToolJobPhase::Preflight)?;
    context.phase(PgToolJobPhase::Running)?;
    context.cancelled().await;
    Err(PgToolJobError::Cancelled)
}

#[test]
fn wire_names_and_all_variants_round_trip() {
    fn round_trip<
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    >(
        v: T,
    ) {
        assert_eq!(
            serde_json::from_value::<T>(serde_json::to_value(&v).unwrap()).unwrap(),
            v
        );
    }
    for scope in [
        PgBackupScope::Database,
        PgBackupScope::Schema {
            schema: "Mixed*".into(),
        },
        PgBackupScope::Table {
            schema: "a".into(),
            table: "b".into(),
        },
    ] {
        round_trip(scope);
    }
    for format in [PgBackupFormat::Plain, PgBackupFormat::Custom] {
        round_trip(format);
    }
    for phase in [
        PgToolJobPhase::Queued,
        PgToolJobPhase::Preflight,
        PgToolJobPhase::Running,
        PgToolJobPhase::Finalizing,
        PgToolJobPhase::Completed,
        PgToolJobPhase::Cancelling,
        PgToolJobPhase::Cancelled,
        PgToolJobPhase::Failed,
    ] {
        round_trip(phase);
    }
    for e in [
        PgToolJobError::UnsupportedEngine,
        PgToolJobError::invalid("clean", "invalid"),
        PgToolJobError::ConnectionClosing,
        PgToolJobError::JobLimitReached,
        PgToolJobError::JobNotFound,
        PgToolJobError::JobActive,
        PgToolJobError::DestinationExists,
        PgToolJobError::ToolUnavailable {
            tool: "psql".into(),
        },
        PgToolJobError::ToolFailed {
            tool: "psql".into(),
            exit_code: Some(1),
            message: "failed".into(),
        },
        PgToolJobError::Io {
            operation: "read".into(),
            message: "failed".into(),
        },
        PgToolJobError::Timeout {
            operation: "reap".into(),
        },
        PgToolJobError::PolicyBlocked {
            reason: "read-only".into(),
        },
        PgToolJobError::PolicyNeedsConfirmation { statements: vec![] },
        PgToolJobError::Cancelled,
    ] {
        round_trip(e);
    }
    let value = serde_json::to_value(PgToolJobError::ToolFailed {
        tool: "psql".into(),
        exit_code: Some(1),
        message: "failed".into(),
    })
    .unwrap();
    assert_eq!(value["kind"], "toolFailed");
    assert_eq!(value["exitCode"], 1);
    round_trip(backup(std::path::Path::new("/archive")));
    round_trip(StartPgRestorePayload {
        connection_id: "id".into(),
        source_path: "/archive".into(),
        format: PgBackupFormat::Custom,
        clean: true,
        confirmed: true,
    });
    round_trip(snapshot("id"));
}

#[test]
fn transitions_reject_illegal_and_late_updates() {
    let mut s = snapshot("id");
    assert!(transition(&mut s, PgToolJobPhase::Completed).is_err());
    for phase in [
        PgToolJobPhase::Preflight,
        PgToolJobPhase::Running,
        PgToolJobPhase::Finalizing,
        PgToolJobPhase::Completed,
    ] {
        transition(&mut s, phase).unwrap();
    }
    assert!(transition(&mut s, PgToolJobPhase::Cancelling).is_err());
    assert!(transition(&mut s, PgToolJobPhase::Failed).is_err());
    for phase in [
        PgToolJobPhase::Queued,
        PgToolJobPhase::Preflight,
        PgToolJobPhase::Running,
        PgToolJobPhase::Finalizing,
    ] {
        s.phase = phase;
        transition(&mut s, PgToolJobPhase::Cancelling).unwrap();
        transition(&mut s, PgToolJobPhase::Cancelled).unwrap();
        assert!(transition(&mut s, PgToolJobPhase::Completed).is_err());
    }
}

#[tokio::test]
async fn admission_limits_listing_cancel_release_and_fences() {
    let manager = PgToolJobManager::new();
    let mut ids = Vec::new();
    for i in 0..4 {
        let id = i.to_string();
        let s = manager
            .start(
                manager.admission(&id).unwrap(),
                snapshot(&id),
                cancellable,
                Box::pin(async {}),
            )
            .unwrap();
        assert!(matches!(
            manager.admission(&id),
            Err(PgToolJobError::JobLimitReached)
        ));
        ids.push(s.job_id);
    }
    assert!(matches!(
        manager.admission("fifth"),
        Err(PgToolJobError::JobLimitReached)
    ));
    assert_eq!(manager.list(None).len(), 4);
    assert_eq!(manager.list(Some("1")).len(), 1);
    let wire = serde_json::to_string(&manager.list(None)).unwrap();
    assert!(!wire.contains("/private"));
    assert!(!wire.contains("password"));
    assert_eq!(manager.release(&ids[0]), Err(PgToolJobError::JobActive));
    assert_eq!(
        manager.cancel(&ids[0]).unwrap().phase,
        PgToolJobPhase::Cancelling
    );
    assert_eq!(
        terminal(&manager, &ids[0]).await.phase,
        PgToolJobPhase::Cancelled
    );
    assert_eq!(
        manager.cancel(&ids[0]).unwrap().phase,
        PgToolJobPhase::Cancelled
    );
    manager.release(&ids[0]).unwrap();
    manager.release(&ids[0]).unwrap();
    let stale = manager.admission("stale").unwrap();
    let teardown_manager = manager.clone();
    let teardown = tokio::spawn(async move { teardown_manager.begin_global_teardown().await });
    while !matches!(
        manager.admission("new"),
        Err(PgToolJobError::ConnectionClosing)
    ) {
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        manager.admission("new"),
        Err(PgToolJobError::ConnectionClosing)
    ));
    assert_eq!(
        manager
            .start(stale, snapshot("stale"), successful, Box::pin(async {}))
            .unwrap_err(),
        PgToolJobError::ConnectionClosing
    );
    teardown.await.unwrap();
    manager.end_global_teardown().await;
    manager.begin_connection_teardown("new").await;
    assert!(matches!(
        manager.admission("new"),
        Err(PgToolJobError::ConnectionClosing)
    ));
    manager.end_connection_teardown("new").await;
    assert!(manager.admission("new").is_ok());
}

#[tokio::test]
async fn terminal_cap_expiry_and_exactly_once_success_effects() {
    let manager = PgToolJobManager::new();
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..36 {
        let count = count.clone();
        let s = manager
            .start(
                manager.admission("id").unwrap(),
                snapshot("id"),
                successful,
                Box::pin(async move {
                    count.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        assert_eq!(
            terminal(&manager, &s.job_id).await.phase,
            PgToolJobPhase::Completed
        );
        manager.cancel(&s.job_id).unwrap();
    }
    assert_eq!(manager.list(None).len(), 32);
    assert_eq!(count.load(Ordering::SeqCst), 36);
    let active = manager
        .start(
            manager.admission("active").unwrap(),
            snapshot("active"),
            cancellable,
            Box::pin(async {}),
        )
        .unwrap();
    manager.expire_for_test();
    assert_eq!(manager.list(None).len(), 1);
    assert_eq!(manager.get(&active.job_id).unwrap().job_id, active.job_id);
    manager.close_all().await;
}

#[tokio::test]
async fn failed_cancelled_and_panicking_workers_do_not_record_success() {
    let manager = PgToolJobManager::new();
    let count = Arc::new(AtomicUsize::new(0));
    for mode in 0..3 {
        let count = count.clone();
        let s = manager
            .start(
                manager.admission("id").unwrap(),
                snapshot("id"),
                move |ctx| async move {
                    match mode {
                        0 => Err(PgToolJobError::ToolUnavailable {
                            tool: "pg_dump".into(),
                        }),
                        1 => cancellable(ctx).await,
                        _ => panic!("injected worker panic"),
                    }
                },
                Box::pin(async move {
                    count.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        if mode == 1 {
            manager.begin_connection_teardown("id").await;
            manager.end_connection_teardown("id").await;
        }
        terminal(&manager, &s.job_id).await;
    }
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stalled_preflight_and_finalization_are_aborted_at_the_teardown_bound() {
    for (finalizing, through_teardown) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let manager = PgToolJobManager::new();
        let completion_count = Arc::new(AtomicUsize::new(0));
        let hook_count = completion_count.clone();
        let dir = tempfile::tempdir().unwrap();
        let partial = Arc::new(tempfile::NamedTempFile::new_in(dir.path()).unwrap());
        let partial_path = partial.path().to_owned();
        let observed_partial_path = partial_path.clone();
        let (entered, entered_rx) = tokio::sync::oneshot::channel();
        let job = manager
            .start(
                manager.admission("stalled").unwrap(),
                snapshot("stalled"),
                move |context| async move {
                    context.phase(PgToolJobPhase::Preflight)?;
                    if finalizing {
                        context.phase(PgToolJobPhase::Running)?;
                        context.phase(PgToolJobPhase::Finalizing)?;
                    }
                    entered.send(()).unwrap();
                    std::future::pending::<()>().await;
                    Ok(Ready::Backup {
                        partial,
                        destination: partial_path,
                    })
                },
                Box::pin(async move {
                    hook_count.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .unwrap();
        entered_rx.await.unwrap();

        if through_teardown {
            manager
                .begin_connection_teardown_with_timeout_for_test(
                    "stalled",
                    Duration::from_millis(30),
                )
                .await;
        } else {
            manager
                .cancel_with_timeout_for_test(&job.job_id, Duration::from_millis(30))
                .unwrap();
            let _ = terminal(&manager, &job.job_id).await;
        }
        let terminal = manager.get(&job.job_id).unwrap();
        assert_eq!(terminal.phase, PgToolJobPhase::Cancelled);
        assert_eq!(terminal.failure, Some(PgToolJobError::Cancelled));
        assert!(manager.inner_finished_for_test(&job.job_id));
        assert!(!observed_partial_path.exists());
        assert_eq!(completion_count.load(Ordering::SeqCst), 0);
        if through_teardown {
            manager.end_connection_teardown("stalled").await;
        }
        assert!(manager.admission("stalled").is_ok());
    }
}

#[tokio::test]
async fn terminal_success_is_published_only_after_completion_is_releasable() {
    let manager = PgToolJobManager::new();
    let (hook_started, hook_started_rx) = tokio::sync::oneshot::channel();
    let (finish_hook, finish_hook_rx) = tokio::sync::oneshot::channel();
    let job = manager
        .start(
            manager.admission("id").unwrap(),
            snapshot("id"),
            |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            },
            Box::pin(async move {
                hook_started.send(()).unwrap();
                let _ = finish_hook_rx.await;
            }),
        )
        .unwrap();
    hook_started_rx.await.unwrap();
    assert_eq!(
        manager.get(&job.job_id).unwrap().phase,
        PgToolJobPhase::Finalizing
    );
    assert_eq!(manager.release(&job.job_id), Err(PgToolJobError::JobActive));
    assert!(matches!(
        manager.admission("id"),
        Err(PgToolJobError::JobLimitReached)
    ));

    finish_hook.send(()).unwrap();
    assert_eq!(
        terminal(&manager, &job.job_id).await.phase,
        PgToolJobPhase::Completed
    );
    manager.release(&job.job_id).unwrap();
    assert!(manager.admission("id").is_ok());
}

#[tokio::test]
async fn stalled_completion_attempt_is_bounded_and_runs_exactly_once() {
    let manager = PgToolJobManager::new();
    let count = Arc::new(AtomicUsize::new(0));
    let hook_count = count.clone();
    let job = manager
        .start(
            manager.admission("id").unwrap(),
            snapshot("id"),
            |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            },
            Box::pin(async move {
                hook_count.fetch_add(1, Ordering::SeqCst);
                std::future::pending::<()>().await;
            }),
        )
        .unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        terminal(&manager, &job.job_id).await.phase,
        PgToolJobPhase::Completed
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    manager.release(&job.job_id).unwrap();
    assert!(manager.admission("id").is_ok());
}

#[tokio::test]
async fn observed_restore_success_wins_a_concurrent_cancellation() {
    let manager = PgToolJobManager::new();
    let count = Arc::new(AtomicUsize::new(0));
    let hook_count = count.clone();
    let job = manager
        .start(
            manager.admission("restore").unwrap(),
            snapshot("restore"),
            |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.cancelled().await;
                // Models a successful psql/pg_restore exit observed by try_wait
                // when cancellation and child readiness become visible together.
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            },
            Box::pin(async move {
                hook_count.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    while manager.get(&job.job_id).unwrap().phase != PgToolJobPhase::Running {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        manager.cancel(&job.job_id).unwrap().phase,
        PgToolJobPhase::Cancelling
    );
    let terminal = terminal(&manager, &job.job_id).await;
    assert_eq!(terminal.phase, PgToolJobPhase::Completed);
    assert_eq!(terminal.failure, None);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pending_reservations_are_bounded_cancelled_and_do_not_grow_scopes() {
    let manager = PgToolJobManager::new();
    for i in 0..100 {
        drop(manager.admission(&format!("invalid-{i}")).unwrap());
    }
    assert_eq!(manager.pending_for_test(), 0);
    assert_eq!(manager.connection_scopes_for_test(), 0);

    let mut pending = manager.admission("pending").unwrap();
    let teardown_manager = manager.clone();
    let teardown =
        tokio::spawn(async move { teardown_manager.begin_connection_teardown("pending").await });
    tokio::time::timeout(Duration::from_secs(1), pending.cancelled())
        .await
        .unwrap();
    drop(pending);
    teardown.await.unwrap();
    manager.end_connection_teardown("pending").await;
    assert_eq!(manager.pending_for_test(), 0);
    assert!(manager.admission("pending").is_ok());
}

#[tokio::test]
async fn detached_reapers_have_a_separate_bounded_capacity() {
    let manager = PgToolJobManager::new();
    let mut exits = Vec::new();
    for i in 0..4 {
        let (exit, exited) = tokio::sync::oneshot::channel();
        exits.push(exit);
        let id = format!("orphan-{i}");
        let job = manager
            .start(
                manager.admission(&id).unwrap(),
                snapshot(&id),
                move |context| async move {
                    context.phase(PgToolJobPhase::Preflight)?;
                    context.phase(PgToolJobPhase::Running)?;
                    context.cancelled().await;
                    assert!(runner::reap(
                        &context,
                        async move {
                            let _ = exited.await;
                        },
                        Duration::from_millis(5),
                    )
                    .await
                    .is_err());
                    Err(PgToolJobError::Timeout {
                        operation: "reap".into(),
                    })
                },
                Box::pin(async {}),
            )
            .unwrap();
        while manager.get(&job.job_id).unwrap().phase != PgToolJobPhase::Running {
            tokio::task::yield_now().await;
        }
        manager.cancel(&job.job_id).unwrap();
        assert_eq!(
            terminal(&manager, &job.job_id).await.failure,
            Some(PgToolJobError::Timeout {
                operation: "reap".into()
            })
        );
    }
    assert!(matches!(
        manager.admission("blocked"),
        Err(PgToolJobError::JobLimitReached)
    ));
    exits.remove(0).send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if manager.admission("available").is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for exit in exits {
        exit.send(()).unwrap();
    }
}

#[test]
fn argument_contract_and_literal_pg_dump_patterns() {
    let mut p = backup(std::path::Path::new("/output"));
    assert_eq!(
        runner::backup_args(&p, std::path::Path::new("/partial")),
        vec!["--format=plain", "--file", "/partial"]
    );
    p.clean = true;
    p.scope = PgBackupScope::Table {
        schema: "Mixed.*?[x]".into(),
        table: "a\"b.c".into(),
    };
    assert_eq!(
        runner::backup_args(&p, std::path::Path::new("/partial")),
        vec![
            "--format=plain",
            "--file",
            "/partial",
            "--clean",
            "--if-exists",
            "--table",
            "\"Mixed.*?[x]\".\"a\"\"b.c\""
        ]
    );
    p.scope = PgBackupScope::Schema {
        schema: "a\"b".into(),
    };
    p.clean = false;
    p.format = PgBackupFormat::Custom;
    assert_eq!(
        runner::backup_args(&p, std::path::Path::new("/partial")),
        vec![
            "--format=custom",
            "--file",
            "/partial",
            "--schema",
            "\"a\"\"b\""
        ]
    );
    let mut r = StartPgRestorePayload {
        connection_id: "id".into(),
        source_path: "/source".into(),
        format: PgBackupFormat::Plain,
        clean: false,
        confirmed: false,
    };
    assert_eq!(
        runner::plain_restore_args("DBUNKprivatekey"),
        vec![
            "--single-transaction",
            "--no-psqlrc",
            "--set=ON_ERROR_STOP=on",
            "--command",
            r"\restrict DBUNKprivatekey",
            "--file",
            "-"
        ]
    );
    r.format = PgBackupFormat::Custom;
    r.clean = true;
    assert_eq!(
        runner::custom_restore_args(&r),
        vec!["--single-transaction", "--clean", "--if-exists", "/source"]
    );
}

#[test]
fn binary_resolution_orders_path_fallbacks_then_bare_name() {
    let dirs: Vec<_> = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/Applications/Postgres.app/Contents/Versions/latest/bin",
        "/usr/lib/postgresql/18/bin",
        "/usr/lib/postgresql/16/bin",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .collect();
    let path = std::env::join_paths(["/first", "/second"]).unwrap();
    let name = if cfg!(windows) {
        "pg_dump.exe"
    } else {
        "pg_dump"
    };
    assert_eq!(
        runner::resolve_tool_with("pg_dump", Some(&path), &dirs, |_| true),
        std::path::Path::new("/first").join(name)
    );
    for (i, dir) in dirs.iter().enumerate() {
        assert_eq!(
            runner::resolve_tool_with("pg_dump", Some(&path), &dirs, |p| dirs[i..]
                .iter()
                .any(|d| p == d.join(name))),
            dir.join(name)
        );
    }
    assert_eq!(
        runner::resolve_tool_with("pg_dump", None, &dirs, |_| false),
        std::path::PathBuf::from(name)
    );
}

#[tokio::test]
async fn validation_and_atomic_publication_never_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("backup.sql");
    Request::Backup(backup(&target)).validate().await.unwrap();
    assert!(Request::Backup(backup(std::path::Path::new("relative")))
        .validate()
        .await
        .is_err());
    assert!(Request::Backup(backup(&dir.path().join("absent/file")))
        .validate()
        .await
        .is_err());
    let partial = Arc::new(tempfile::NamedTempFile::new_in(dir.path()).unwrap());
    std::fs::write(partial.path(), b"archive").unwrap();
    let partial_path = partial.path().to_owned();
    Ready::Backup {
        partial,
        destination: target.clone(),
    }
    .publish()
    .unwrap();
    assert!(!partial_path.exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"archive");
    assert_eq!(
        Request::Backup(backup(&target))
            .validate()
            .await
            .unwrap_err(),
        PgToolJobError::DestinationExists
    );
    let partial = Arc::new(tempfile::NamedTempFile::new_in(dir.path()).unwrap());
    let partial_path = partial.path().to_owned();
    assert_eq!(
        Ready::Backup {
            partial,
            destination: target.clone()
        }
        .publish()
        .unwrap_err(),
        PgToolJobError::DestinationExists
    );
    assert!(!partial_path.exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"archive");
    for source in [
        dir.path().to_owned(),
        dir.path().join("empty"),
        dir.path().join("absent"),
    ] {
        if source.ends_with("empty") {
            std::fs::write(&source, []).unwrap();
        }
        let p = StartPgRestorePayload {
            connection_id: "id".into(),
            source_path: source.to_string_lossy().into(),
            format: PgBackupFormat::Plain,
            clean: false,
            confirmed: false,
        };
        assert!(Request::Restore(p).validate().await.is_err());
    }
}

#[tokio::test]
async fn stderr_drain_is_bounded_while_producer_exceeds_pipe_capacity() {
    use tokio::io::AsyncWriteExt;
    let (mut write, read) = tokio::io::duplex(1024);
    let producer = tokio::spawn(async move {
        write.write_all(&vec![b'x'; 300_000]).await.unwrap();
        write.write_all(b"tail").await.unwrap();
    });
    let tail = runner::drain_tail(read, 64 * 1024).await.unwrap();
    producer.await.unwrap();
    assert_eq!(tail.len(), 64 * 1024);
    assert!(tail.ends_with(b"tail"));
}

#[tokio::test]
async fn reap_timeout_releases_slots_but_detached_cleanup_waits_for_exit() {
    let manager = PgToolJobManager::new();
    let dir = tempfile::tempdir().unwrap();
    let partial = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let partial_path = partial.path().to_owned();
    let (exit, exited) = tokio::sync::oneshot::channel();
    let s = manager
        .start(
            manager.admission("id").unwrap(),
            snapshot("id"),
            move |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.cancelled().await;
                if runner::reap(
                    &context,
                    async move {
                        let _ = exited.await;
                        drop(partial);
                    },
                    Duration::from_millis(10),
                )
                .await
                .is_ok()
                {
                    panic!("fake child unexpectedly exited");
                }
                Err(PgToolJobError::Timeout {
                    operation: "reap".into(),
                })
            },
            Box::pin(async {}),
        )
        .unwrap();
    // Ensure the injected child has started before cancelling.
    while manager.get(&s.job_id).unwrap().phase == PgToolJobPhase::Queued {
        tokio::task::yield_now().await;
    }
    manager.begin_connection_teardown("id").await;
    let ended = terminal(&manager, &s.job_id).await;
    assert_eq!(ended.phase, PgToolJobPhase::Cancelled);
    assert_eq!(
        ended.failure,
        Some(PgToolJobError::Timeout {
            operation: "reap".into()
        })
    );
    assert!(partial_path.exists());
    manager.end_connection_teardown("id").await;
    let next = manager
        .start(
            manager.admission("id").unwrap(),
            snapshot("id"),
            successful,
            Box::pin(async {}),
        )
        .unwrap();
    assert_eq!(
        terminal(&manager, &next.job_id).await.phase,
        PgToolJobPhase::Completed
    );
    exit.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while partial_path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn teardown_watchdog_preserves_in_flight_reap_timeout_and_cleanup_owner() {
    let manager = PgToolJobManager::new();
    let dir = tempfile::tempdir().unwrap();
    let partial = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let partial_path = partial.path().to_owned();
    let (exit, exited) = tokio::sync::oneshot::channel();
    let job = manager
        .start(
            manager.admission("id").unwrap(),
            snapshot("id"),
            move |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.cancelled().await;
                // Scaled production ordering: cleanup starts after cancellation,
                // but its reap deadline extends beyond the manager watchdog.
                tokio::time::sleep(Duration::from_millis(30)).await;
                let _ = runner::reap(
                    &context,
                    async move {
                        let _ = exited.await;
                        drop(partial);
                    },
                    Duration::from_millis(80),
                )
                .await;
                unreachable!("watchdog aborts the worker first")
            },
            Box::pin(async {}),
        )
        .unwrap();
    while manager.get(&job.job_id).unwrap().phase != PgToolJobPhase::Running {
        tokio::task::yield_now().await;
    }
    manager
        .begin_connection_teardown_with_timeout_for_test("id", Duration::from_millis(100))
        .await;
    let ended = manager.get(&job.job_id).unwrap();
    assert_eq!(ended.phase, PgToolJobPhase::Cancelled);
    assert_eq!(
        ended.failure,
        Some(PgToolJobError::Timeout {
            operation: "reap".into()
        })
    );
    assert!(partial_path.exists());
    manager.end_connection_teardown("id").await;
    assert!(manager.admission("id").is_ok());

    exit.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while partial_path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[test]
#[ignore = "native subprocess helper, invoked only by the process tests"]
fn native_child_helper() {
    use std::io::Write;
    let mode = std::env::var("DBUNK_PG_TOOL_HELPER_MODE").unwrap();
    if let Ok(pid_file) = std::env::var("DBUNK_PG_TOOL_HELPER_PID") {
        std::fs::write(pid_file, std::process::id().to_string()).unwrap();
    }
    match mode.as_str() {
        "sleep" => std::thread::sleep(Duration::from_secs(60)),
        "failure" => {
            std::io::stderr().write_all(&vec![b'x'; 300_000]).unwrap();
            eprintln!("password=secret /private/key.pem: role secret does not exist");
            std::process::exit(2);
        }
        "success" => {}
        _ => panic!("unknown helper mode"),
    }
}
fn helper(mode: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "postgres::backup::tests::native_child_helper",
        "--ignored",
        "--nocapture",
    ])
    .env("DBUNK_PG_TOOL_HELPER_MODE", mode);
    cmd
}

#[tokio::test]
async fn native_child_failure_spawn_failure_timeout_and_cancel_clean_partials() {
    for mode in ["failure", "missing", "timeout", "cancel"] {
        let manager = PgToolJobManager::new();
        let dir = tempfile::tempdir().unwrap();
        let partial = Arc::new(tempfile::NamedTempFile::new_in(dir.path()).unwrap());
        let partial_path = partial.path().to_owned();
        let pid_file = dir.path().join("child.pid");
        let stdin_source = (mode == "cancel").then(|| {
            let path = dir.path().join("large-restore.sql");
            std::fs::write(&path, vec![b'x'; 2 * 1024 * 1024]).unwrap();
            path
        });
        let mut command = match mode {
            "missing" => tokio::process::Command::new(dir.path().join("nonexistent-tool")),
            "failure" => helper("failure"),
            _ => helper("sleep"),
        };
        command.env("DBUNK_PG_TOOL_HELPER_PID", &pid_file);
        let s = manager
            .start(
                manager.admission("id").unwrap(),
                snapshot("id"),
                move |ctx| async move {
                    ctx.phase(PgToolJobPhase::Preflight)?;
                    runner::process(
                        command,
                        "pg_dump",
                        &ctx,
                        Some(partial),
                        None,
                        (mode == "timeout").then_some(Duration::from_millis(100)),
                        false,
                        false,
                        stdin_source,
                    )
                    .await?;
                    unreachable!()
                },
                Box::pin(async {}),
            )
            .unwrap();
        if mode == "cancel" {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !pid_file.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            let start = std::time::Instant::now();
            manager.close_all().await;
            assert!(start.elapsed() < Duration::from_secs(3));
        }
        let ended = terminal(&manager, &s.job_id).await;
        assert!(!partial_path.exists());
        match mode {
            "missing" => assert!(matches!(
                ended.failure,
                Some(PgToolJobError::ToolUnavailable { .. })
            )),
            "failure" => {
                let Some(PgToolJobError::ToolFailed {
                    exit_code, message, ..
                }) = ended.failure
                else {
                    panic!("tool failure")
                };
                assert_eq!(exit_code, Some(2));
                assert!(!message.contains("secret"));
                assert!(!message.contains("/private"));
            }
            "timeout" => assert_eq!(
                ended.failure,
                Some(PgToolJobError::Timeout {
                    operation: "preflight".into()
                })
            ),
            "cancel" => assert_eq!(ended.phase, PgToolJobPhase::Cancelled),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn final_publication_and_cancel_have_one_winner() {
    for cancel_first in [true, false] {
        let manager = PgToolJobManager::new();
        let dir = tempfile::tempdir().unwrap();
        let partial = Arc::new(tempfile::NamedTempFile::new_in(dir.path()).unwrap());
        std::fs::write(partial.path(), b"archive").unwrap();
        let destination = dir.path().join("final");
        let output = destination.clone();
        let (ready, ready_rx) = tokio::sync::oneshot::channel();
        let (go, go_rx) = tokio::sync::oneshot::channel();
        let s = manager
            .start(
                manager.admission("id").unwrap(),
                snapshot("id"),
                move |ctx| async move {
                    successful(ctx).await?;
                    ready.send(()).unwrap();
                    let _ = go_rx.await;
                    Ok(Ready::Backup {
                        partial,
                        destination: output,
                    })
                },
                Box::pin(async {}),
            )
            .unwrap();
        ready_rx.await.unwrap();
        if cancel_first {
            manager.cancel(&s.job_id).unwrap();
        }
        go.send(()).unwrap();
        let ended = terminal(&manager, &s.job_id).await;
        assert_eq!(
            ended.phase,
            if cancel_first {
                PgToolJobPhase::Cancelled
            } else {
                PgToolJobPhase::Completed
            }
        );
        assert_eq!(destination.exists(), !cancel_first);
        assert_eq!(manager.cancel(&s.job_id).unwrap().phase, ended.phase);
    }
}

#[tokio::test]
async fn publication_does_not_hold_manager_lock_or_publish_late_cancellation() {
    let manager = PgToolJobManager::new();
    let (publication_started, publication_started_rx) = tokio::sync::oneshot::channel();
    let (finish_publication, finish_publication_rx) = tokio::sync::oneshot::channel();
    let job = manager
        .start(
            manager.admission("publishing").unwrap(),
            snapshot("publishing"),
            |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.phase(PgToolJobPhase::Finalizing)?;
                Ok(Ready::PublicationTest {
                    started: publication_started,
                    finish: finish_publication_rx,
                })
            },
            Box::pin(async {}),
        )
        .unwrap();
    publication_started_rx.await.unwrap();

    tokio::time::timeout(Duration::from_millis(100), async {
        assert_eq!(
            manager.cancel(&job.job_id).unwrap().phase,
            PgToolJobPhase::Finalizing
        );
        assert_eq!(
            manager.get(&job.job_id).unwrap().phase,
            PgToolJobPhase::Finalizing
        );
        assert!(manager.admission("unrelated").is_ok());
        assert_eq!(manager.release(&job.job_id), Err(PgToolJobError::JobActive));
    })
    .await
    .expect("publication must not hold the manager mutex");

    manager
        .begin_connection_teardown_with_timeout_for_test("publishing", Duration::from_millis(30))
        .await;
    assert_eq!(
        manager.get(&job.job_id).unwrap().phase,
        PgToolJobPhase::Finalizing
    );
    finish_publication.send(()).unwrap();
    let terminal = terminal(&manager, &job.job_id).await;
    assert_eq!(terminal.phase, PgToolJobPhase::Completed);
    assert_eq!(terminal.failure, None);
    manager.end_connection_teardown("publishing").await;
}
