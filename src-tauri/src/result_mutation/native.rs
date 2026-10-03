//! Native cleanup waits for a retired tab's operation, including an admitted
//! COMMIT, without closing the shared socket or another tab's analysis.

use super::*;

impl ResultMutationManager {
    #[cfg(feature = "isolated-profile")]
    pub(crate) fn with_native_tasks(
        mut self,
        tasks: crate::postgres::dedicated::DriverJoins,
    ) -> Self {
        self.native = Some(ConnectionTasks::new(tasks));
        self
    }

    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn close_native_tab(&self, id: &str, tab: &str) {
        let Some(executor) = self.existing_executor(id).await else {
            return;
        };
        let Some(group) = executor.native.clone() else {
            return;
        };
        let cancel = {
            let mut state = executor.state.lock().await;
            state.closing_tabs.insert(tab.into());
            cancel_tab_state(&mut state, tab).1
        };
        let tab = tab.to_owned();
        let pending = group.clone();
        let (done, joined) = oneshot::channel();
        group.cleanup(async move {
            perform_cancel(&executor, cancel).await;
            executor.notify.notify_one();
            pending.wait_tab(&tab).await;
            let mut state = executor.state.lock().await;
            state.tabs.remove(&tab);
            state
                .snapshots
                .values
                .retain(|_, snapshot| snapshot.tab_id != tab);
            let snapshots = &mut state.snapshots;
            let values = &snapshots.values;
            snapshots.order.retain(|id| values.contains_key(id));
            state.closing_tabs.remove(&tab);
            let _ = done.send(());
        });
        if joined.await.is_err() {
            group.drivers().drain().await;
        }
    }

    pub(super) async fn close_native_connection(&self, id: &str) {
        let executor = {
            let mut state = self.inner.lock().await;
            state.native_idle_closing.remove(id);
            state.closing.insert(id.into());
            state.executors.remove(id)
        };
        self.changed.notify_waiters();
        if let Some(executor) = executor {
            start_close(executor, true);
        }
        self.native.as_ref().unwrap().join(Some(id), false).await;
    }

    pub(super) async fn close_native_all(&self) {
        let (executors, restore) = {
            let mut state = self.inner.lock().await;
            let restore = !state.global_closing;
            state.global_closing = true;
            (
                state
                    .executors
                    .drain()
                    .map(|(_, executor)| executor)
                    .collect::<Vec<_>>(),
                restore,
            )
        };
        self.changed.notify_waiters();
        for executor in executors {
            start_close(executor, true);
        }
        self.native.as_ref().unwrap().join(None, false).await;
        if restore {
            self.inner.lock().await.global_closing = false;
        }
        self.changed.notify_waiters();
    }

    #[cfg(feature = "isolated-profile")]
    pub(crate) async fn force_native_teardown(&self, connection: Option<&str>) {
        {
            let mut state = self.inner.lock().await;
            match connection {
                Some(id) => {
                    state.native_idle_closing.remove(id);
                    state.closing.insert(id.into());
                    state.executors.remove(id);
                }
                None => {
                    state.global_closing = true;
                    state.executors.clear();
                }
            }
        }
        self.changed.notify_waiters();
        if let Some(native) = &self.native {
            native.join(connection, true).await;
        }
    }

    pub(super) async fn close_native_idle(&self) {
        let candidates = {
            let state = self.inner.lock().await;
            state
                .executors
                .iter()
                .map(|(id, executor)| (id.clone(), executor.clone()))
                .collect::<Vec<_>>()
        };
        for (id, executor) in candidates {
            let removed = {
                let mut state = self.inner.lock().await;
                if state
                    .executors
                    .get(&id)
                    .is_some_and(|current| Arc::ptr_eq(current, &executor))
                {
                    let mut inner = executor.state.lock().await;
                    if is_idle(&inner) {
                        // Fence callers that already obtained this executor Arc
                        // before the owned close worker gets its first poll.
                        inner.closed = true;
                        state.native_idle_closing.insert(id.clone());
                        state.closing.insert(id.clone());
                        state.executors.remove(&id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(executor) = removed {
                start_close(executor, false);
                self.native.as_ref().unwrap().join(Some(&id), false).await;
                let mut state = self.inner.lock().await;
                // An explicit connection teardown can take over this fence
                // while the idle close is awaiting its worker.
                if state.native_idle_closing.remove(&id) {
                    state.closing.remove(&id);
                }
                drop(state);
                self.changed.notify_waiters();
            }
        }
    }
}

fn start_close(executor: Arc<Executor>, fence: bool) {
    if let Some(group) = &executor.native {
        let group = group.clone();
        group.cleanup(async move {
            close_executor(&executor, fence).await;
            executor.notify.notify_one();
        });
    }
}

#[cfg(all(test, feature = "isolated-profile"))]
#[path = "native_tests.rs"]
mod tests;
