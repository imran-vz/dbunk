//! No sockets: task-drop signals prove tab-scoped termination, including
//! sessions removed before cleanup is requested.
use super::*;
use crate::postgres::dedicated::DriverJoins;

async fn tracked_task(group: &DriverJoins) -> tokio::sync::oneshot::Receiver<()> {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (stopped, done) = tokio::sync::oneshot::channel();
    struct OnDrop(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let stopped = OnDrop(Some(stopped));
    group.track_task(tokio::spawn(async move {
        let _stopped = stopped;
        let _ = started.send(());
        std::future::pending::<()>().await;
    }));
    // A group may already be aborted; its late registration must still join.
    let _ = ready.await;
    done
}

#[tokio::test]
async fn native_close_joins_only_its_session() {
    let directory = tempfile::tempdir().unwrap();
    let pool = crate::storage::open_pool(&crate::storage::Paths::from_dir(directory.path().into()))
        .await
        .unwrap();
    let parent = DriverJoins::default();
    let manager = QuerySessionManager::new(pool.clone()).with_native_tasks(parent.clone());
    let first = parent.child();
    let second = parent.child();
    let first_done = tracked_task(&first).await;
    let mut second_done = tracked_task(&second).await;
    {
        let mut state = manager.inner.lock().await;
        for (id, tasks) in [("one", first.clone()), ("two", second.clone())] {
            state.native_sessions.insert(
                id.into(),
                NativeSessionTasks {
                    window: "window".into(),
                    connection_id: "same-connection".into(),
                    tasks,
                },
            );
        }
    }
    assert!(matches!(
        manager.close_native("one", "different-window").await,
        Err(QuerySessionError::OwnerMismatch)
    ));
    manager.close_native("one", "window").await.unwrap();
    first_done.await.unwrap();
    assert!(matches!(
        second_done.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    // Late descendants inherit the closed session's abort without fencing the
    // parent or another session. Their join remains owned by the global host.
    let late = tracked_task(&first).await;
    first.drain().await;
    late.await.unwrap();
    manager.close_native("two", "window").await.unwrap();
    second_done.await.unwrap();
    manager.close_native("two", "window").await.unwrap();
    assert!(manager.inner.lock().await.native_sessions.is_empty());
    parent.drain().await;
    pool.close().await;
}

#[tokio::test]
async fn retiring_window_joins_failed_startups_that_never_entered_live_session_map() {
    let directory = tempfile::tempdir().unwrap();
    let pool = crate::storage::open_pool(&crate::storage::Paths::from_dir(directory.path().into()))
        .await
        .unwrap();
    let parent = DriverJoins::default();
    let manager = QuerySessionManager::new(pool.clone()).with_native_tasks(parent.clone());
    let startup = parent.child();
    let ended = tracked_task(&startup).await;
    manager.inner.lock().await.native_sessions.insert(
        "failed-startup".into(),
        NativeSessionTasks {
            window: "window".into(),
            connection_id: "fixture".into(),
            tasks: startup,
        },
    );
    manager.retire_window("window").await;
    ended.await.unwrap();
    assert!(manager.inner.lock().await.native_sessions.is_empty());
    parent.drain().await;
    pool.close().await;
}
