//! Native-only cleanup. The facade retires document admission before tab close
//! and supplies the shared deadline around graceful and forced teardown.

use super::*;
#[cfg(feature = "isolated-profile")]
use crate::postgres::dedicated::DriverJoins;
use crate::table_browse::executor::spawn_executor_tracked;

impl TableBrowseManager {
    #[cfg(feature = "isolated-profile")]
    pub(crate) fn with_native_tasks(mut self, tasks: DriverJoins) -> Self {
        self.native = Some(ConnectionTasks::new(tasks));
        self
    }

    pub(super) async fn native_executor_for(
        &self,
        spec: ResolvedPostgresConnectSpec,
        id: &str,
    ) -> Result<Arc<Executor>, TableBrowseError> {
        let mut state = self.inner.lock().await;
        if state.global_closing || state.closing.contains(id) {
            return Err(TableBrowseError::ConnectionClosing);
        }
        if let Some(executor) = state.executors.get(id) {
            return Ok(executor.clone());
        }
        check_admission(&state)?;
        if self
            .native
            .as_ref()
            .is_some_and(|tasks| tasks.count() >= MAX_EXECUTORS)
        {
            return Err(TableBrowseError::Timeout {
                operation: "admission".into(),
            });
        }
        let group = self.native.as_ref().and_then(|native| native.register(id));
        let executor = spawn_executor_tracked(spec, group);
        state.executors.insert(id.into(), executor.clone());
        Ok(executor)
    }

    pub(super) async fn close_native_tab(&self, id: &str, tab: &str) {
        let executor = self.inner.lock().await.executors.get(id).cloned();
        let Some(executor) = executor else {
            return;
        };
        let Some(group) = executor.native.clone() else {
            return;
        };
        {
            let mut state = executor.inner.lock().await;
            state.closing_tabs.insert(tab.into());
            apply_tab_cancel(&mut state, tab);
        }
        executor.notify.notify_one();
        let tab = tab.to_owned();
        let (done, joined) = oneshot::channel();
        let pending = group.clone();
        group.cleanup(async move {
            pending.wait_tab(&tab).await;
            let mut state = executor.inner.lock().await;
            state.tabs.remove(&tab);
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
        for executor in executors {
            start_close(executor, true);
        }
        self.native.as_ref().unwrap().join(None, false).await;
        if restore {
            self.inner.lock().await.global_closing = false;
        }
    }

    /// Called after the facade's shared graceful deadline expires. Admission
    /// remains fenced until its explicit end-teardown operation.
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
                    let mut inner = executor.inner.lock().await;
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
            }
        }
    }
}

fn start_close(executor: Arc<Executor>, fence: bool) {
    if let Some(group) = &executor.native {
        let group = group.clone();
        group.cleanup(async move {
            close_executor(&executor, fence).await;
            // The worker may not yet be waiting; keep a wake permit for it.
            executor.notify.notify_one();
        });
    }
}

#[cfg(all(test, feature = "isolated-profile"))]
#[path = "native_tests.rs"]
mod tests;
