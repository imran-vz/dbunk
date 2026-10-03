//! Opt-in native ownership. These joins are deliberately non-abortable: aborting
//! a future that holds Child is not proof that the operating-system child exited.
use futures_util::{
    future::{BoxFuture, Shared},
    FutureExt,
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::{sync::oneshot, time::Instant};

type Join = Shared<BoxFuture<'static, bool>>;
#[derive(Default)]
struct State {
    joins: Vec<Join>,
    failed: bool,
    parent: Option<Ownership>,
}
#[derive(Clone, Default)]
pub(crate) struct Ownership(Arc<Mutex<State>>);
impl Ownership {
    pub(crate) fn child(&self) -> Self {
        Self(Arc::new(Mutex::new(State {
            parent: Some(self.clone()),
            ..Default::default()
        })))
    }
    fn register(&self, join: Join) {
        let mut state = self.0.lock().unwrap();
        let results = state
            .joins
            .iter()
            .filter_map(|join| join.clone().now_or_never())
            .collect::<Vec<_>>();
        state.failed |= results.iter().any(|ok| !*ok);
        state.joins.retain(|join| join.peek().is_none());
        if let Some(parent) = &state.parent {
            parent.register(join.clone());
        }
        state.joins.push(join);
    }
    pub(crate) fn spawn<F, T>(&self, work: F) -> Result<oneshot::Receiver<T>, ()>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let mut state = self.0.lock().unwrap();
        let finished = state
            .joins
            .iter()
            .filter_map(|join| join.clone().now_or_never())
            .collect::<Vec<_>>();
        state.failed |= finished.iter().any(|ok| !*ok);
        state.joins.retain(|join| join.peek().is_none());
        if state.joins.len() >= 128 {
            return Err(());
        }
        let (send, receive) = oneshot::channel();
        let task = tokio::spawn(async move {
            let value = work.await;
            let _ = send.send(value);
        });
        let join = async move { task.await.is_ok() }.boxed().shared();
        if let Some(parent) = &state.parent {
            parent.register(join.clone());
        }
        state.joins.push(join);
        Ok(receive)
    }
    pub(crate) fn spawn_blocking<F, T>(
        &self,
        work: F,
    ) -> Result<oneshot::Receiver<Result<T, ()>>, ()>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // The parent owner joins this wrapper, which always waits for the actual
        // blocking operation. No dropped Tokio filesystem future can hide it.
        self.spawn(async move { tokio::task::spawn_blocking(work).await.map_err(|_| ()) })
    }
    pub(crate) async fn drain_until(&self, deadline: Instant) -> Result<(), ()> {
        tokio::time::timeout_at(deadline, async {
            loop {
                let joins = self.0.lock().unwrap().joins.clone();
                if joins.is_empty() {
                    return if self.0.lock().unwrap().failed {
                        Err(())
                    } else {
                        Ok(())
                    };
                }
                let mut failed = false;
                for join in joins {
                    failed |= !join.await;
                }
                let mut state = self.0.lock().unwrap();
                state.failed |= failed;
                state.joins.retain(|join| join.peek().is_none());
            }
        })
        .await
        .map_err(|_| ())?
    }
    pub(crate) fn settled(&self) -> bool {
        let mut state = self.0.lock().unwrap();
        let results = state
            .joins
            .iter()
            .filter_map(|join| join.clone().now_or_never())
            .collect::<Vec<_>>();
        state.failed |= results.iter().any(|ok| !*ok);
        state.joins.retain(|join| join.peek().is_none());
        state.joins.is_empty() && !state.failed
    }
}

pub(crate) mod archive;
pub(crate) mod source;

#[cfg(test)]
mod tests;
