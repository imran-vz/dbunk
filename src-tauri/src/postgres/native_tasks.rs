//! Opt-in task ownership for connection-scoped native executors. Generations
//! remain registered until their shared joins have actually been observed.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::dedicated::DriverJoins;

#[derive(Clone)]
pub(crate) struct ConnectionTasks {
    parent: DriverJoins,
    registry: Arc<Mutex<Registry>>,
}

#[derive(Default)]
struct Registry {
    #[cfg(feature = "isolated-profile")]
    next: u64,
    groups: HashMap<String, Vec<TaskGroup>>,
}

#[derive(Clone)]
pub(crate) struct TaskGroup(Arc<Group>);

struct Group {
    generation: u64,
    work: DriverJoins,
    cleanup: DriverJoins,
    cleanup_open: Mutex<bool>,
    tabs: Mutex<HashMap<String, usize>>,
    changed: Notify,
}

impl ConnectionTasks {
    #[cfg(feature = "isolated-profile")]
    pub(crate) fn new(parent: DriverJoins) -> Self {
        Self {
            parent,
            registry: Default::default(),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.registry
            .lock()
            .unwrap()
            .groups
            .values()
            .map(Vec::len)
            .sum()
    }

    pub(crate) fn register(&self, connection: &str) -> Option<TaskGroup> {
        #[cfg(feature = "isolated-profile")]
        {
            let mut registry = self.registry.lock().unwrap();
            registry.next += 1;
            let group = TaskGroup(Arc::new(Group {
                generation: registry.next,
                work: self.parent.child(),
                cleanup: self.parent.child(),
                cleanup_open: Mutex::new(true),
                tabs: Default::default(),
                changed: Notify::new(),
            }));
            registry
                .groups
                .entry(connection.into())
                .or_default()
                .push(group.clone());
            Some(group)
        }
        #[cfg(not(feature = "isolated-profile"))]
        {
            let _ = (&self.parent, connection);
            None
        }
    }

    /// The caller first closes manager admission. Cancellation of this future
    /// leaves every group registered so a subsequent forced close can join it.
    pub(crate) async fn join(&self, connection: Option<&str>, abort: bool) {
        let groups = {
            let registry = self.registry.lock().unwrap();
            registry
                .groups
                .iter()
                .filter(|(id, _)| connection.is_none_or(|connection| connection == id.as_str()))
                .flat_map(|(id, groups)| groups.iter().map(|group| (id.clone(), group.clone())))
                .collect::<Vec<_>>()
        };
        if abort {
            for (_, group) in &groups {
                group.0.work.abort_all();
                group.0.cleanup.abort_all();
            }
        }
        futures_util::future::join_all(groups.iter().map(|(_, group)| group.join())).await;
        let mut registry = self.registry.lock().unwrap();
        for (id, joined) in groups {
            if let Some(current) = registry.groups.get_mut(&id) {
                current.retain(|group| group.0.generation != joined.0.generation);
                if current.is_empty() {
                    registry.groups.remove(&id);
                }
            }
        }
    }
}

impl TaskGroup {
    pub(crate) fn drivers(&self) -> &DriverJoins {
        &self.0.work
    }

    pub(crate) fn track(&self, task: tokio::task::JoinHandle<()>) {
        self.0.work.track_task(task);
    }

    /// Cleanup has its own child group: it can await executor work without
    /// joining itself. Both groups also remain owned by the global host.
    pub(crate) fn cleanup(&self, work: impl Future<Output = ()> + Send + 'static) {
        let open = self.0.cleanup_open.lock().unwrap();
        if *open {
            self.0.cleanup.track_task(tokio::spawn(work));
        }
    }

    async fn join(&self) {
        self.0.cleanup.drain().await;
        self.0.work.drain().await;
        // Work or a caller holding a stale executor Arc can register cleanup
        // after the first drain. Seal registration once all work is joined,
        // then observe those late joins before retiring this generation.
        *self.0.cleanup_open.lock().unwrap() = false;
        self.0.cleanup.drain().await;
    }

    pub(crate) fn begin_tab(&self, tab: &str) -> TabWork {
        *self.0.tabs.lock().unwrap().entry(tab.into()).or_default() += 1;
        TabWork {
            group: self.clone(),
            tab: tab.into(),
        }
    }

    pub(crate) async fn wait_tab(&self, tab: &str) {
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !self.0.tabs.lock().unwrap().contains_key(tab) {
                return;
            }
            changed.await;
        }
    }
}

pub(crate) struct TabWork {
    group: TaskGroup,
    tab: String,
}

impl Drop for TabWork {
    fn drop(&mut self) {
        let mut tabs = self.group.0.tabs.lock().unwrap();
        if let Some(count) = tabs.get_mut(&self.tab) {
            *count -= 1;
            if *count == 0 {
                tabs.remove(&self.tab);
            }
        }
        drop(tabs);
        self.group.0.changed.notify_waiters();
    }
}

#[cfg(all(test, feature = "isolated-profile"))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn cancelled_join_preserves_generation_until_forced_join() {
        let global = DriverJoins::default();
        let tasks = ConnectionTasks::new(global.clone());
        let group = tasks.register("connection").unwrap();
        group.track(tokio::spawn(std::future::pending()));
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            tasks.join(Some("connection"), false),
        )
        .await
        .is_err());
        assert_eq!(tasks.registry.lock().unwrap().groups["connection"].len(), 1);
        tasks.join(Some("connection"), true).await;
        assert!(tasks.registry.lock().unwrap().groups.is_empty());
        global.drain().await;
    }

    #[tokio::test]
    async fn joining_old_generation_does_not_retire_new_generation() {
        let global = DriverJoins::default();
        let tasks = ConnectionTasks::new(global.clone());
        let first = tasks.register("connection").unwrap();
        let (release, pending) = oneshot::channel();
        first.track(tokio::spawn(async move {
            let _ = pending.await;
        }));
        let joining = tasks.join(Some("connection"), false);
        tokio::pin!(joining);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut joining)
                .await
                .is_err()
        );
        let second = tasks.register("connection").unwrap();
        release.send(()).unwrap();
        joining.await;
        {
            let registry = tasks.registry.lock().unwrap();
            let remaining = &registry.groups["connection"];
            assert_eq!(remaining.len(), 1);
            assert_eq!(remaining[0].0.generation, second.0.generation);
        }
        tasks.join(None, true).await;
        global.drain().await;
    }

    #[tokio::test]
    async fn global_abort_owns_work_and_cleanup_even_without_local_join() {
        let global = DriverJoins::default();
        let tasks = ConnectionTasks::new(global.clone());
        let group = tasks.register("connection").unwrap();
        let work = group.begin_tab("tab");
        let cleanup = group.begin_tab("tab");
        group.track(tokio::spawn(async move {
            let _work = work;
            std::future::pending::<()>().await;
        }));
        group.cleanup(async move {
            let _cleanup = cleanup;
            std::future::pending::<()>().await;
        });
        global.abort_all();
        tokio::time::timeout(Duration::from_secs(1), global.drain())
            .await
            .unwrap();
        group.wait_tab("tab").await;
        tasks.join(None, true).await;
        assert!(tasks.registry.lock().unwrap().groups.is_empty());
    }

    #[tokio::test]
    async fn join_observes_cleanup_registered_by_work_and_seals_retired_group() {
        use futures_util::FutureExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        let global = DriverJoins::default();
        let tasks = ConnectionTasks::new(global.clone());
        let group = tasks.register("connection").unwrap();
        let late = group.clone();
        let (run, waiting) = oneshot::channel();
        let (started, cleanup_started) = oneshot::channel();
        let (release, blocked) = oneshot::channel();
        group.track(tokio::spawn(async move {
            let _ = waiting.await;
            late.cleanup(async move {
                let _ = started.send(());
                let _ = blocked.await;
            });
        }));
        let joining = tasks.join(None, false);
        tokio::pin!(joining);
        // The first cleanup drain is empty; the owned worker is still pending.
        assert!(joining.as_mut().now_or_never().is_none());
        run.send(()).unwrap();
        cleanup_started.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut joining)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        joining.await;
        assert_eq!(tasks.count(), 0);
        let ran = Arc::new(AtomicBool::new(false));
        let late_ran = ran.clone();
        group.cleanup(async move {
            late_ran.store(true, Ordering::Release);
        });
        global.drain().await;
        assert!(!ran.load(Ordering::Acquire));
    }
}
