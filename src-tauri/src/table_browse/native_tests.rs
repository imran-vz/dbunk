use super::*;
use crate::table_browse::executor::{dummy_spec, TabSlot};

#[tokio::test]
async fn close_tab_waits_its_operation_and_preserves_peer() {
    let global = DriverJoins::default();
    let manager = TableBrowseManager::new().with_native_tasks(global.clone());
    let executor = manager
        .native_executor_for(dummy_spec("c"), "c")
        .await
        .unwrap();
    let group = executor.native.as_ref().unwrap();
    let operation = group.begin_tab("closing");
    let peer = group.begin_tab("peer");
    {
        let mut state = executor.inner.lock().await;
        state.tabs.insert(
            "closing".into(),
            TabSlot {
                in_flight_request_id: Some(1),
                ..Default::default()
            },
        );
        state.tabs.insert("peer".into(), TabSlot::default());
    }
    let closing = manager.clone();
    let mut close = tokio::spawn(async move { closing.close_tab("c", "closing").await });
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut close)
        .await
        .is_err());
    assert!(executor.inner.lock().await.closing_tabs.contains("closing"));
    drop(operation);
    tokio::time::timeout(Duration::from_secs(1), close)
        .await
        .unwrap()
        .unwrap();
    assert!(executor.inner.lock().await.tabs.contains_key("peer"));
    assert!(!executor.inner.lock().await.closed);
    drop(peer);
    manager.begin_global_teardown().await;
    global.drain().await;
}

#[tokio::test]
async fn admission_ticket_does_not_wait_for_a_page_and_close_settles_it() {
    let global = DriverJoins::default();
    let manager = TableBrowseManager::new().with_native_tasks(global.clone());
    let executor = manager
        .native_executor_for(dummy_spec("c"), "c")
        .await
        .unwrap();
    // Stop the worker before enqueueing so the test never opens a socket.
    executor.native.as_ref().unwrap().drivers().abort_all();
    executor.native.as_ref().unwrap().drivers().drain().await;
    let ticket = manager
        .start_count(
            dummy_spec("c"),
            CountTableBrowseRowsPayload {
                connection_id: "c".into(),
                tab_id: "tab".into(),
                request_id: 1,
                schema: "public".into(),
                table: "rows".into(),
                filters: vec![],
            },
        )
        .await
        .unwrap();
    manager.close_tab("c", "tab").await;
    assert_eq!(ticket.await, Err(TableBrowseError::Cancelled));
    manager.force_native_teardown(None).await;
    global.drain().await;
}

#[tokio::test]
async fn cancelled_connection_close_retains_join_ownership() {
    let global = DriverJoins::default();
    let manager = TableBrowseManager::new().with_native_tasks(global.clone());
    let executor = manager
        .native_executor_for(dummy_spec("c"), "c")
        .await
        .unwrap();
    let peer = manager
        .native_executor_for(dummy_spec("peer"), "peer")
        .await
        .unwrap();
    let (started, ready) = oneshot::channel();
    let (ended, finished) = oneshot::channel();
    struct End(Option<oneshot::Sender<()>>);
    impl Drop for End {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    executor
        .native
        .as_ref()
        .unwrap()
        .track(tokio::spawn(async move {
            let _end = End(Some(ended));
            let _ = started.send(());
            std::future::pending::<()>().await;
        }));
    ready.await.unwrap();
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
    finished.await.unwrap();
    assert!(!peer.inner.lock().await.closed);
    manager.begin_global_teardown().await;
    global.drain().await;
}

#[tokio::test]
async fn explicit_teardown_takes_over_idle_close_fence() {
    let global = DriverJoins::default();
    let manager = TableBrowseManager::new().with_native_tasks(global.clone());
    let executor = manager
        .native_executor_for(dummy_spec("c"), "c")
        .await
        .unwrap();
    executor.inner.lock().await.last_used = Instant::now() - IDLE_TIMEOUT;
    let (release, wait) = oneshot::channel();
    executor
        .native
        .as_ref()
        .unwrap()
        .track(tokio::spawn(async move {
            let _ = wait.await;
        }));
    let idle = manager.close_native_idle();
    tokio::pin!(idle);
    assert!(tokio::time::timeout(Duration::from_millis(10), &mut idle)
        .await
        .is_err());
    assert!(manager.inner.lock().await.native_idle_closing.contains("c"));
    let explicit = manager.begin_connection_teardown("c");
    tokio::pin!(explicit);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut explicit)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    tokio::join!(idle, explicit);
    assert!(manager.inner.lock().await.closing.contains("c"));
    manager.end_connection_teardown("c").await;
    global.drain().await;
}

#[tokio::test]
async fn idle_retirement_fences_stale_executor_before_cleanup_is_polled() {
    use futures_util::FutureExt;
    let global = DriverJoins::default();
    let manager = TableBrowseManager::new().with_native_tasks(global.clone());
    let stale = manager
        .native_executor_for(dummy_spec("c"), "c")
        .await
        .unwrap();
    stale.inner.lock().await.last_used = Instant::now() - IDLE_TIMEOUT;
    let close = manager.close_native_idle();
    tokio::pin!(close);
    // Poll retirement without giving the spawned cleanup worker a turn.
    assert!(close.as_mut().now_or_never().is_none());
    assert!(stale.inner.try_lock().unwrap().closed);
    assert!(!manager
        .inner
        .try_lock()
        .unwrap()
        .executors
        .contains_key("c"));
    close.await;
    global.drain().await;
}
