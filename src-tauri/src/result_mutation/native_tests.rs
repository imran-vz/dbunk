use super::*;
use crate::postgres::dedicated::DriverJoins;

fn spec(id: &str) -> ResolvedPostgresConnectSpec {
    ResolvedPostgresConnectSpec::from_connection(&crate::app::test_postgres_connection(
        id,
        crate::SafeMode::Disabled,
        false,
    ))
    .unwrap()
}

#[tokio::test]
async fn tab_close_waits_admitted_commit_and_preserves_peer_analysis() {
    let global = DriverJoins::default();
    let manager = ResultMutationManager::new().with_native_tasks(global.clone());
    let executor = manager.executor_for(spec("c"), "c").await.unwrap();
    let operation = executor.native.as_ref().unwrap().begin_tab("closing");
    let peer = {
        let mut state = executor.state.lock().await;
        let mut active = ActiveRequest::new("closing", 1, ActiveKind::Apply);
        active.apply_phase = Some(ApplyPhase::CommitAdmitted);
        state.active = Some(active);
        state.snapshots.insert(AnalysisSnapshot {
            tab_id: "closing".into(),
            descriptors: vec![],
        });
        state.snapshots.insert(AnalysisSnapshot {
            tab_id: "peer".into(),
            descriptors: vec![],
        })
    };
    let closing = manager.clone();
    let mut close = tokio::spawn(async move { closing.close_native_tab("c", "closing").await });
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut close)
        .await
        .is_err());
    assert_eq!(
        executor
            .state
            .lock()
            .await
            .active
            .as_ref()
            .unwrap()
            .interrupt,
        Interrupt::None
    );
    executor.state.lock().await.active = None;
    drop(operation);
    tokio::time::timeout(Duration::from_secs(1), close)
        .await
        .unwrap()
        .unwrap();
    assert!(executor
        .state
        .lock()
        .await
        .snapshots
        .values
        .contains_key(&peer));
    assert_eq!(executor.state.lock().await.snapshots.values.len(), 1);
    manager.begin_global_teardown().await;
    global.drain().await;
}

#[tokio::test]
async fn queued_analysis_ticket_is_cancellable_without_waiting_for_io() {
    let global = DriverJoins::default();
    let manager = ResultMutationManager::new().with_native_tasks(global.clone());
    let executor = manager.executor_for(spec("c"), "c").await.unwrap();
    executor.native.as_ref().unwrap().drivers().abort_all();
    executor.native.as_ref().unwrap().drivers().drain().await;
    let ticket = manager
        .start_analyze(
            spec("c"),
            AnalyzeResultSetPayload {
                connection_id: "c".into(),
                tab_id: "tab".into(),
                request_id: 1,
                source: AnalyzeSource::Relation {
                    schema: "public".into(),
                    table: "rows".into(),
                },
                refresh_structure: false,
            },
            Arc::new(|_, _, _| Box::pin(async { Ok(None) })),
        )
        .await
        .unwrap();
    manager.close_native_tab("c", "tab").await;
    assert_eq!(ticket.await, Err(ResultMutationError::Cancelled));
    manager.force_native_teardown(None).await;
    global.drain().await;
}

#[tokio::test]
async fn forced_close_joins_worker_and_cleanup_after_grace_future_is_dropped() {
    let global = DriverJoins::default();
    let manager = ResultMutationManager::new().with_native_tasks(global.clone());
    let executor = manager.executor_for(spec("c"), "c").await.unwrap();
    executor.state.lock().await.active = Some(ActiveRequest::new("tab", 1, ActiveKind::Analysis));
    let closing = manager.clone();
    let mut close = tokio::spawn(async move { closing.begin_connection_teardown("c").await });
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut close)
        .await
        .is_err());
    close.abort();
    let _ = close.await;
    tokio::time::timeout(
        Duration::from_secs(1),
        manager.force_native_teardown(Some("c")),
    )
    .await
    .unwrap();
    global.drain().await;
    assert!(manager.inner.lock().await.closing.contains("c"));
    manager.end_connection_teardown("c").await;
    let reopened = manager.executor_for(spec("c"), "c").await.unwrap();
    assert!(!Arc::ptr_eq(&executor, &reopened));
    manager.begin_global_teardown().await;
    global.drain().await;
}

#[tokio::test]
async fn apply_ticket_claims_exclusivity_before_waiting_and_can_be_cancelled() {
    let global = DriverJoins::default();
    let manager = ResultMutationManager::new().with_native_tasks(global.clone());
    let executor = manager.executor_for(spec("c"), "c").await.unwrap();
    let analysis_id = executor
        .state
        .lock()
        .await
        .snapshots
        .insert(AnalysisSnapshot {
            tab_id: "tab".into(),
            descriptors: vec![],
        });
    let payload = ApplyResultMutationsPayload {
        connection_id: "c".into(),
        tab_id: "tab".into(),
        request_id: 1,
        analysis_id,
        confirmed: false,
        plan: MutationPlan { operations: vec![] },
    };
    // On the current-thread runtime, admission does not yield after spawning:
    // cancellation below precedes the apply worker's first poll and socket open.
    let ticket = manager
        .start_apply(spec("c"), payload.clone())
        .await
        .unwrap();
    assert!(matches!(
        manager.start_apply(spec("c"), payload).await,
        Err(ResultMutationError::Busy)
    ));
    assert!(manager.cancel_tab("c", "tab").await.cancel_requested);
    assert_eq!(ticket.await, Err(ResultMutationError::Cancelled));
    assert!(executor.state.lock().await.active.is_none());
    manager.close_native_tab("c", "tab").await;
    manager.begin_global_teardown().await;
    global.drain().await;
}

#[tokio::test]
async fn idle_retirement_fences_stale_executor_before_cleanup_is_polled() {
    use futures_util::FutureExt;
    let global = DriverJoins::default();
    let manager = ResultMutationManager::new().with_native_tasks(global.clone());
    let stale = manager.executor_for(spec("c"), "c").await.unwrap();
    stale.state.lock().await.last_used = Instant::now() - IDLE_TIMEOUT;
    let close = manager.close_native_idle();
    tokio::pin!(close);
    // Poll retirement without giving the spawned cleanup worker a turn.
    assert!(close.as_mut().now_or_never().is_none());
    assert!(stale.state.try_lock().unwrap().closed);
    assert!(!manager
        .inner
        .try_lock()
        .unwrap()
        .executors
        .contains_key("c"));
    close.await;
    global.drain().await;
}
