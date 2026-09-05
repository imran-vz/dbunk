use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn snapshot(id: &str) -> Snapshot {
    Snapshot {
        job_id: String::new(),
        connection_id: id.into(),
        schema: "public".into(),
        table: "items".into(),
        direction: Direction::Import,
        file_name: "items.csv".into(),
        phase: Phase::Preparing,
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        total_bytes: Some(100),
        bytes_processed: 0,
        rows_processed: None,
        rows_committed: None,
        failure: None,
    }
}
fn prepared(manager: &TransferManager, id: &str) -> String {
    let a = manager.admission(id).unwrap();
    manager
        .insert_review(&a, super::super::runner::test_review(id))
        .unwrap()
        .inspection_token
}
async fn terminal(manager: &TransferManager, id: &str) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let s = manager.get(id).unwrap();
            if s.phase.terminal() {
                return s;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
fn launch<R, F>(manager: &TransferManager, id: &str, run: R, completion: Completion) -> Snapshot
where
    R: FnOnce(JobContext) -> F + Send + 'static,
    F: Future<Output = Result<(), TransferError>> + Send + 'static,
{
    let token = prepared(manager, id);
    manager
        .start(
            manager.admission(id).unwrap(),
            &token,
            snapshot(id),
            run,
            completion,
        )
        .unwrap()
}
pub(crate) fn start_waiting(
    manager: &TransferManager,
    id: &str,
    stopped: Arc<AtomicBool>,
) -> Snapshot {
    launch(
        manager,
        id,
        move |ctx| async move {
            ctx.cancelled().await;
            stopped.store(true, Ordering::SeqCst);
            Err(TransferError::Cancelled)
        },
        Box::pin(async {}),
    )
}

#[tokio::test]
async fn pending_work_counts_towards_capacity_and_same_connection_limit() {
    let m = TransferManager::new();
    let first = m.admission("one").unwrap();
    assert!(matches!(
        m.admission("one"),
        Err(TransferError::JobLimitReached)
    ));
    let rest: Vec<_> = (0..3)
        .map(|i| m.admission(&format!("other-{i}")).unwrap())
        .collect();
    assert!(matches!(
        m.admission("full"),
        Err(TransferError::JobLimitReached)
    ));
    drop(first);
    assert!(m.admission("one").is_ok());
    drop(rest);
}
#[tokio::test]
async fn reviews_have_a_cap_ttl_and_explicit_release() {
    let m = TransferManager::new();
    let tokens: Vec<_> = (0..MAX_REVIEWS)
        .map(|i| prepared(&m, &format!("c{i}")))
        .collect();
    let a = m.admission("overflow").unwrap();
    assert!(matches!(
        m.insert_review(&a, super::super::runner::test_review("overflow")),
        Err(TransferError::JobLimitReached)
    ));
    m.release_review(&tokens[0]);
    assert!(matches!(
        m.review(&tokens[0]),
        Err(TransferError::InspectionExpired)
    ));
    m.inner
        .lock()
        .unwrap()
        .reviews
        .get_mut(&tokens[1])
        .unwrap()
        .created = Instant::now() - REVIEW_TTL;
    assert!(matches!(
        m.review(&tokens[1]),
        Err(TransferError::InspectionExpired)
    ));
}
#[tokio::test]
async fn cancellation_cannot_commit_or_run_success_effects() {
    let m = TransferManager::new();
    let audit = Arc::new(AtomicBool::new(false));
    let audited = audit.clone();
    let s = launch(
        &m,
        "c",
        |ctx| async move {
            ctx.cancelled().await;
            assert!(!ctx.begin_finalizing());
            Err(TransferError::Cancelled)
        },
        Box::pin(async move {
            audited.store(true, Ordering::SeqCst);
        }),
    );
    assert!(matches!(
        m.release(&s.job_id),
        Err(TransferError::JobActive)
    ));
    assert_eq!(m.cancel(&s.job_id).unwrap().phase, Phase::Cancelling);
    assert_eq!(terminal(&m, &s.job_id).await.phase, Phase::Cancelled);
    assert!(!audit.load(Ordering::SeqCst));
    m.release(&s.job_id).unwrap();
    m.release(&s.job_id).unwrap();
}
#[tokio::test]
async fn finalization_wins_and_completion_effects_precede_terminal() {
    let m = TransferManager::new();
    let (entered, rx) = tokio::sync::oneshot::channel();
    let (release, wait) = tokio::sync::oneshot::channel();
    let s = launch(
        &m,
        "c",
        move |ctx| async move {
            ctx.progress(100, Some(2));
            assert!(ctx.begin_finalizing());
            ctx.succeeded(Some(2));
            Ok(())
        },
        Box::pin(async move {
            let _ = entered.send(());
            let _ = wait.await;
        }),
    );
    rx.await.unwrap();
    let finalizing = m.cancel(&s.job_id).unwrap();
    assert_eq!(finalizing.phase, Phase::Finalizing);
    assert_eq!(finalizing.rows_committed, Some(2));
    assert!(matches!(
        m.release(&s.job_id),
        Err(TransferError::JobActive)
    ));
    release.send(()).unwrap();
    let done = terminal(&m, &s.job_id).await;
    assert_eq!(done.phase, Phase::Completed);
    assert_eq!(done.rows_committed, Some(2));
}
#[tokio::test]
async fn unknown_commit_never_runs_success_audit() {
    let m = TransferManager::new();
    let audit = Arc::new(AtomicBool::new(false));
    let audited = audit.clone();
    let s = launch(
        &m,
        "c",
        |ctx| async move {
            assert!(ctx.begin_finalizing());
            Err(TransferError::OutcomeUnknown)
        },
        Box::pin(async move {
            audited.store(true, Ordering::SeqCst);
        }),
    );
    let done = terminal(&m, &s.job_id).await;
    assert_eq!(done.phase, Phase::OutcomeUnknown);
    assert_eq!(done.rows_committed, None);
    assert!(!audit.load(Ordering::SeqCst));
}
#[tokio::test]
async fn worker_panic_after_commit_claim_is_unknown() {
    let m = TransferManager::new();
    let s = launch(
        &m,
        "c",
        |ctx| async move {
            assert!(ctx.begin_finalizing());
            panic!("injected worker panic");
            #[allow(unreachable_code)]
            Ok(())
        },
        Box::pin(async {}),
    );
    assert_eq!(terminal(&m, &s.job_id).await.phase, Phase::OutcomeUnknown);
}
#[tokio::test]
async fn fences_invalidate_reviews_and_resolutions_with_nested_scopes() {
    let m = TransferManager::new();
    let token = prepared(&m, "c");
    let mut pending = m.admission("c").unwrap();
    let mm = m.clone();
    let fence = tokio::spawn(async move {
        mm.begin_connection_teardown("c").await;
    });
    pending.cancelled().await;
    assert!(matches!(
        m.admission("c"),
        Err(TransferError::ConnectionClosing)
    ));
    drop(pending);
    fence.await.unwrap();
    assert!(matches!(
        m.review(&token),
        Err(TransferError::InspectionExpired)
    ));
    m.begin_connection_teardown("c").await;
    m.end_connection_teardown("c").await;
    assert!(matches!(
        m.admission("c"),
        Err(TransferError::ConnectionClosing)
    ));
    m.end_connection_teardown("c").await;
    assert!(m.admission("c").is_ok());
}
#[tokio::test]
async fn global_fence_stops_all_workers_before_returning() {
    let m = TransferManager::new();
    let flags: Vec<_> = (0..4)
        .map(|i| {
            let stopped = Arc::new(AtomicBool::new(false));
            start_waiting(&m, &format!("c{i}"), stopped.clone());
            stopped
        })
        .collect();
    m.begin_global_teardown().await;
    assert!(flags.iter().all(|f| f.load(Ordering::SeqCst)));
    assert!(m.list(None).iter().all(|s| s.phase == Phase::Cancelled));
    assert!(matches!(
        m.admission("new"),
        Err(TransferError::ConnectionClosing)
    ));
    m.end_global_teardown().await;
    assert!(m.admission("new").is_ok());
}
#[tokio::test]
async fn watchdog_joins_before_releasing_capacity() {
    struct OnDrop(Arc<AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let m = TransferManager::new();
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let s = launch(
        &m,
        "c",
        move |_ctx| async move {
            let _guard = OnDrop(flag);
            let _ = ready.send(());
            std::future::pending::<()>().await;
            Ok(())
        },
        Box::pin(async {}),
    );
    wait.await.unwrap();
    m.cancel(&s.job_id).unwrap();
    m.abort_cancelled(None, Some(&s.job_id)).await;
    assert_eq!(terminal(&m, &s.job_id).await.phase, Phase::Cancelled);
    assert!(stopped.load(Ordering::SeqCst));
    assert!(m.admission("c").is_ok());
}
#[tokio::test]
async fn teardown_timeout_joins_aborted_worker_before_returning() {
    struct OnDrop(Arc<AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let m = TransferManager::new();
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let s = launch(
        &m,
        "c",
        move |_ctx| async move {
            let _owned_resource = OnDrop(flag);
            let _ = ready.send(());
            std::future::pending::<()>().await;
            Ok(())
        },
        Box::pin(async {}),
    );
    wait.await.unwrap();

    m.begin_teardown(Some("c"), Duration::ZERO).await;

    assert!(
        stopped.load(Ordering::SeqCst),
        "teardown returned before the worker released its owned resource"
    );
    let terminal = m.get(&s.job_id).unwrap();
    assert_eq!(terminal.phase, Phase::Cancelled);
    assert_eq!(
        terminal.failure,
        Some(TransferError::Timeout {
            operation: "cleanup".into()
        })
    );
}
#[tokio::test]
async fn terminal_retention_never_evicts_active_jobs() {
    let m = TransferManager::new();
    let stopped = Arc::new(AtomicBool::new(false));
    let active = start_waiting(&m, "active", stopped);
    for i in 0..MAX_TERMINAL + 3 {
        let s = launch(
            &m,
            &format!("c{i}"),
            |ctx| async move {
                assert!(ctx.begin_finalizing());
                ctx.succeeded(Some(0));
                Ok(())
            },
            Box::pin(async {}),
        );
        terminal(&m, &s.job_id).await;
    }
    assert_eq!(m.list(None).len(), MAX_TERMINAL + 1);
    assert!(!m.get(&active.job_id).unwrap().phase.terminal());
    m.close_all().await;
}
#[tokio::test]
async fn an_inspection_is_consumed_exactly_once() {
    let m = TransferManager::new();
    let token = prepared(&m, "c");
    let s = m
        .start(
            m.admission("c").unwrap(),
            &token,
            snapshot("c"),
            |ctx| async move {
                ctx.cancelled().await;
                Err(TransferError::Cancelled)
            },
            Box::pin(async {}),
        )
        .unwrap();
    assert!(matches!(
        m.review(&token),
        Err(TransferError::InspectionExpired)
    ));
    m.cancel(&s.job_id).unwrap();
    terminal(&m, &s.job_id).await;
    let a = m.admission("c").unwrap();
    assert!(matches!(
        m.start(
            a,
            &token,
            snapshot("c"),
            |_| async { Ok(()) },
            Box::pin(async {})
        ),
        Err(TransferError::InspectionExpired)
    ));
    assert!(m.admission("c").is_ok());
}
