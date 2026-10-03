//! Bounded local file jobs. Blocking filesystem calls are joined, never detached
//! by aborting their waiter. Shutdown cancels before publication admission.
use dbunk_lib::backend::result_files::Cancellation;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

type Output<T> = Result<Arc<T>, String>;
type JobJoin<T> = Shared<BoxFuture<'static, Result<Output<T>, String>>>;
type Join = Shared<BoxFuture<'static, Result<(), String>>>;
struct Entry {
    cancellation: Cancellation,
    join: Join,
}
#[derive(Default)]
struct State {
    closing: bool,
    entries: Vec<Entry>,
    failure: Option<String>,
}
#[derive(Default)]
pub struct FileRuntime {
    state: Mutex<State>,
}
pub struct FileJob<T> {
    join: JobJoin<T>,
}
impl<T: Send + Sync + 'static> FileJob<T> {
    pub async fn finish(self) -> Output<T> {
        self.join.await?
    }
}
impl FileRuntime {
    pub fn start<T: Send + Sync + 'static>(
        &self,
        runtime: &tokio::runtime::Handle,
        cancellation: Cancellation,
        operation: impl FnOnce(&Cancellation) -> Result<T, String> + Send + 'static,
    ) -> Result<FileJob<T>, &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.closing {
            return Err("Application is closing");
        }
        let mut pending = Vec::new();
        for entry in std::mem::take(&mut state.entries) {
            match entry.join.clone().now_or_never() {
                None => pending.push(entry),
                Some(Err(error)) => state.failure = Some(error),
                Some(Ok(_)) => {}
            }
        }
        state.entries = pending;
        if state.entries.len() >= 2 {
            return Err("Two file exports are already active");
        }
        let token = cancellation.clone();
        let task = runtime.spawn_blocking(move || operation(&token).map(Arc::new));
        let join = async move {
            task.await
                .map_err(|_| "File worker failed while joining".to_owned())
        }
        .boxed()
        .shared();
        let retained = join.clone();
        state.entries.push(Entry {
            cancellation: cancellation.clone(),
            join: async move { retained.await.map(|_| ()) }.boxed().shared(),
        });
        Ok(FileJob { join })
    }
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.closing = true;
        for entry in &state.entries {
            entry.cancellation.cancel();
        }
    }
    pub async fn join(&self, deadline: Instant) -> Result<(), String> {
        let joins = self
            .state
            .lock()
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.join.clone())
            .collect::<Vec<_>>();
        for join in joins {
            // Ordinary preparation/publication refusals are already displayed in
            // the owning tool. A panic or unfinished worker is a cleanup failure.
            tokio::time::timeout_at(deadline, join)
                .await
                .map_err(|_| {
                    "File worker did not terminate within the shutdown budget".to_owned()
                })??;
        }
        self.state
            .lock()
            .unwrap()
            .failure
            .clone()
            .map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bounded_jobs_remain_owned_after_ui_waiter_drop_and_shutdown_timeout() {
        let files = FileRuntime::default();
        let (release_a, wait_a) = std::sync::mpsc::channel();
        let (release_b, wait_b) = std::sync::mpsc::channel();
        let first = files
            .start::<()>(
                &tokio::runtime::Handle::current(),
                Cancellation::default(),
                move |token| {
                    wait_a.recv().unwrap();
                    assert!(token.is_cancelled());
                    Err("cancelled before publication".into())
                },
            )
            .unwrap();
        let second = files
            .start::<()>(
                &tokio::runtime::Handle::current(),
                Cancellation::default(),
                move |token| {
                    wait_b.recv().unwrap();
                    assert!(token.is_cancelled());
                    Err("cancelled before publication".into())
                },
            )
            .unwrap();
        assert!(
            files
                .start::<()>(
                    &tokio::runtime::Handle::current(),
                    Cancellation::default(),
                    |_| unreachable!()
                )
                .is_err()
        );
        drop(first);
        drop(second);
        files.stop();
        assert!(
            files
                .start::<()>(
                    &tokio::runtime::Handle::current(),
                    Cancellation::default(),
                    |_| unreachable!()
                )
                .is_err()
        );
        assert!(
            files
                .join(Instant::now() + Duration::from_millis(5))
                .await
                .is_err()
        );
        release_a.send(()).unwrap();
        release_b.send(()).unwrap();
        files
            .join(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reaped_worker_panic_remains_a_cleanup_failure() {
        let files = FileRuntime::default();
        let job = files
            .start::<()>(
                &tokio::runtime::Handle::current(),
                Cancellation::default(),
                |_| panic!("injected export worker panic"),
            )
            .unwrap();
        assert!(job.finish().await.is_err());
        let next = files
            .start::<()>(
                &tokio::runtime::Handle::current(),
                Cancellation::default(),
                |_| Err("ordinary refusal".into()),
            )
            .unwrap();
        assert!(next.finish().await.is_err());
        files.stop();
        assert!(
            files
                .join(Instant::now() + Duration::from_secs(2))
                .await
                .unwrap_err()
                .contains("joining")
        );
    }
}
