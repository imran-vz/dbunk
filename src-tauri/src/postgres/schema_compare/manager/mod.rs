//! Two admitted comparisons, including resolution, with one reservation per
//! endpoint. Only the supervisor publishes terminal state, after worker joins.
mod reads;
mod runner;
#[cfg(test)]
mod tests;

use super::{
    budget::Budget,
    capture::CaptureControl,
    diff::Comparison,
    protocol::*,
    values::{ResponseOwnership, RESULT_TTL},
};
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub(crate) const JOB_DEADLINE: Duration = Duration::from_secs(60);
const CLEANUP_GRACE: Duration = Duration::from_secs(5);
const MAX_ACTIVE: usize = 2;
const MAX_TERMINAL: usize = 2;
const MAX_REQUESTS: usize = 64;
const MAX_FENCE_SCOPES: usize = 4;
const MAX_CONNECTION_ID_BYTES: usize = 128;
const MAX_TRANSPORTS: usize = 4;

struct DocumentTransport {
    window: String,
    token: String,
}

#[derive(Default)]
struct Scope {
    closing: usize,
    generation: u64,
}
struct Entry {
    status: Status,
    generations: [u64; 3],
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
    active: bool,
    invalidated: bool,
    stop_reason: Option<CompareError>,
    finished: Option<Instant>,
    result: Option<Comparison>,
}
struct RequestRecord {
    request: StartRequest,
    job_id: String,
    created: Instant,
}
#[derive(Default)]
struct State {
    jobs: Vec<Entry>,
    requests: Vec<RequestRecord>,
    connections: Vec<(String, Scope)>,
    global: Scope,
    responses: ResponseOwnership,
    transports: Vec<DocumentTransport>,
}
#[derive(Clone, Default)]
pub(crate) struct CompareManager {
    inner: Arc<Mutex<State>>,
    budget: Budget,
}

#[derive(Clone)]
pub(crate) struct JobContext {
    manager: CompareManager,
    pub(crate) identity: ResultIdentity,
    pub(crate) request: StartRequest,
    pub(crate) control: CaptureControl,
    pub(crate) budget: Budget,
}
impl JobContext {
    pub(crate) fn progress(&self, phase: StatusState, source: u32, target: u32) {
        let mut s = self.manager.inner.lock().unwrap();
        if let Some(e) = s
            .jobs
            .iter_mut()
            .find(|e| e.status.job_id == self.identity.job_id)
        {
            if e.active && !*e.cancel.borrow() && !e.invalidated {
                e.status.state = phase;
                e.status.source_objects = source;
                e.status.target_objects = target;
            }
        }
    }
}

impl CompareManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn start_native(
        &self,
        request: StartRequest,
        pool: sqlx::SqlitePool,
    ) -> Result<Status, CompareError> {
        self.start(request, move |ctx| runner::run(ctx, pool))
    }

    pub(crate) fn start<F, Fut>(
        &self,
        request: StartRequest,
        run: F,
    ) -> Result<Status, CompareError>
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Comparison, CompareError>> + Send + 'static,
    {
        self.start_with_timing(request, run, JOB_DEADLINE, CLEANUP_GRACE)
    }

    fn start_with_timing<F, Fut>(
        &self,
        mut request: StartRequest,
        run: F,
        job_timeout: Duration,
        cleanup_grace: Duration,
    ) -> Result<Status, CompareError>
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Comparison, CompareError>> + Send + 'static,
    {
        request.validate()?;
        compact_request(&mut request);
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        if let Some(record) = s
            .requests
            .iter()
            .find(|r| r.request.request_id == request.request_id)
        {
            if record.request != request {
                return Err(CompareError::InvalidRequest);
            }
            return s
                .jobs
                .iter()
                .find(|e| e.status.job_id == record.job_id && !e.invalidated)
                .map(|e| e.status.clone())
                .ok_or(CompareError::Unavailable);
        }
        if !request.fresh() {
            return Err(CompareError::Unavailable);
        }
        let ids = [&request.source.connection_id, &request.target.connection_id];
        if s.global.closing > 0
            || ids
                .iter()
                .any(|id| connection_scope(&s, id).is_some_and(|c| c.closing > 0))
        {
            return Err(CompareError::Unavailable);
        }
        if s.requests.len() >= MAX_REQUESTS
            || s.jobs.iter().filter(|e| e.active).count() >= MAX_ACTIVE
            || s.jobs
                .iter()
                .any(|e| e.active && ids.iter().any(|id| involves(e, id)))
        {
            return Err(CompareError::Busy);
        }
        let deadline = tokio::time::Instant::now() + job_timeout;
        let generations = [
            s.global.generation,
            generation(&s, ids[0]),
            generation(&s, ids[1]),
        ];
        let (cancel, rx) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        let job_id = uuid::Uuid::new_v4().to_string();
        let status = Status {
            job_id: job_id.clone(),
            request_id: request.request_id.clone(),
            source: request.source.clone(),
            target: request.target.clone(),
            source_objects: 0,
            target_objects: 0,
            state: StatusState::Resolving,
        };
        let ctx = JobContext {
            manager: self.clone(),
            identity: ResultIdentity {
                job_id: job_id.clone(),
                result_id: uuid::Uuid::new_v4().to_string(),
            },
            request: request.clone(),
            control: CaptureControl::new(deadline, rx.clone()),
            budget: self.budget.result_scope(),
        };
        // Reserve setup/credential/transport scratch through all joins, including
        // cancellation during post-connect options. Capture charges its own pages.
        let scratch = self.budget.scratch(32 * 1024 * 1024)?;
        s.requests.push(RequestRecord {
            request,
            job_id: job_id.clone(),
            created: Instant::now(),
        });
        s.jobs.push(Entry {
            status: status.clone(),
            generations,
            cancel,
            done: done_rx,
            active: true,
            invalidated: false,
            stop_reason: None,
            finished: None,
            result: None,
        });
        let owner = self.clone();
        let control = ctx.control.clone();
        let runtime = tokio::runtime::Handle::current();
        // OS keychain/SSH calls and CPU diff work cannot occupy the async runtime.
        // A running blocking worker cannot be forcibly stopped by Tokio; its join
        // continues to own admission even after the cleanup grace expires.
        let mut worker = tokio::task::spawn_blocking(move || runtime.block_on(run(ctx)));
        tokio::spawn(async move {
            let mut rx = rx;
            let interrupted = tokio::select! {
                biased;
                _ = async { let _ = rx.wait_for(|v| *v).await; } => Some(CompareError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => Some(CompareError::DeadlineExceeded),
                result = &mut worker => {
                    control.join_drivers().await;
                    drop(scratch);
                    owner.finish(&job_id, result.unwrap_or(Err(CompareError::Unavailable)));
                    done.send_replace(true);
                    return;
                }
            };
            owner.stop(&job_id, interrupted.clone().unwrap());
            if tokio::time::timeout(cleanup_grace, &mut worker)
                .await
                .is_err()
            {
                control.abort_drivers();
                worker.abort();
                let _ = worker.await;
            }
            control.join_drivers().await;
            drop(scratch);
            owner.finish(&job_id, Err(interrupted.unwrap()));
            done.send_replace(true);
        });
        Ok(status)
    }

    fn finish(&self, id: &str, result: Result<Comparison, CompareError>) {
        let mut s = self.inner.lock().unwrap();
        let Some(index) = s.jobs.iter().position(|e| e.status.job_id == id) else {
            return;
        };
        let e = &s.jobs[index];
        let stale = e.invalidated
            || e.generations
                != [
                    s.global.generation,
                    generation(&s, &e.status.source.connection_id),
                    generation(&s, &e.status.target.connection_id),
                ];
        let e = &mut s.jobs[index];
        let result = if stale {
            Err(CompareError::Cancelled)
        } else if let Some(reason) = &e.stop_reason {
            Err(reason.clone())
        } else {
            result
        };
        e.status.state = match result {
            Ok(result) => {
                let result_id = result.metadata().identity.result_id.clone();
                e.result = Some(result);
                StatusState::Completed { result_id }
            }
            Err(CompareError::Cancelled) => StatusState::Cancelled,
            Err(failure) => StatusState::Failed { failure },
        };
        e.active = false;
        e.finished = Some(Instant::now());
        prune(&mut s, Instant::now());
    }

    fn stop(&self, id: &str, reason: CompareError) {
        let mut s = self.inner.lock().unwrap();
        if let Some(e) = s
            .jobs
            .iter_mut()
            .find(|e| e.status.job_id == id && e.active)
        {
            cancel(e, reason);
        }
    }

    pub(crate) fn get(&self, id: &str) -> Result<Status, CompareError> {
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        s.jobs
            .iter()
            .find(|e| e.status.job_id == id && !e.invalidated)
            .map(|e| e.status.clone())
            .ok_or(CompareError::Unavailable)
    }
    pub(crate) fn list(&self) -> Vec<Status> {
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        s.jobs
            .iter()
            .filter(|e| !e.invalidated)
            .map(|e| e.status.clone())
            .collect()
    }
    pub(crate) fn cancel(&self, id: &str) -> Result<Status, CompareError> {
        self.stop(id, CompareError::Cancelled);
        self.get(id)
    }
    pub(crate) fn release(&self, id: &str) -> Result<(), CompareError> {
        let mut s = self.inner.lock().unwrap();
        if s.jobs.iter().any(|e| e.status.job_id == id && e.active) {
            return Err(CompareError::Busy);
        }
        s.jobs.retain(|e| e.status.job_id != id);
        Ok(())
    }

    pub(crate) async fn begin_connection_teardown(&self, id: &str) {
        self.teardown(Some(id)).await;
    }
    pub(crate) async fn begin_global_teardown(&self) {
        self.teardown(None).await;
    }
    pub(crate) async fn close_all(&self) {
        self.begin_global_teardown().await;
    }
    async fn teardown(&self, id: Option<&str>) {
        let pending = {
            let mut s = self.inner.lock().unwrap();
            let connection_index = id.and_then(|id| {
                s.connections
                    .iter()
                    .position(|(connection_id, _)| connection_id == id)
            });
            let use_global = id.is_none()
                || id.is_some_and(|id| {
                    id.is_empty()
                        || id.len() > MAX_CONNECTION_ID_BYTES
                        || (connection_index.is_none()
                            && (s.global.closing > 0 || s.connections.len() >= MAX_FENCE_SCOPES))
                });
            let scope = if use_global {
                &mut s.global
            } else if let Some(index) = connection_index {
                &mut s.connections[index].1
            } else {
                s.connections
                    .push((id.unwrap().to_string(), Scope::default()));
                &mut s.connections.last_mut().unwrap().1
            };
            scope.closing += 1;
            scope.generation += 1;
            let mut pending = Vec::with_capacity(MAX_ACTIVE);
            for e in &mut s.jobs {
                if use_global || id.is_some_and(|id| involves(e, id)) {
                    e.invalidated = true;
                    e.result = None;
                    if e.active {
                        cancel(e, CompareError::Cancelled);
                        pending.push(e.done.clone());
                    }
                }
            }
            prune(&mut s, Instant::now());
            pending
        };
        for mut done in pending {
            let _ = done.wait_for(|v| *v).await;
        }
    }
    pub(crate) async fn end_connection_teardown(&self, id: &str) {
        let mut s = self.inner.lock().unwrap();
        if let Some(index) = s
            .connections
            .iter()
            .position(|(connection_id, _)| connection_id == id)
        {
            s.connections[index].1.closing = s.connections[index].1.closing.saturating_sub(1);
            if s.connections[index].1.closing == 0 {
                s.connections.remove(index);
            }
        } else {
            s.global.closing = s.global.closing.saturating_sub(1);
        }
    }
    pub(crate) async fn end_global_teardown(&self) {
        let mut s = self.inner.lock().unwrap();
        s.global.closing = s.global.closing.saturating_sub(1);
    }
    pub(crate) fn start_monitor(&self) {
        let weak = Arc::downgrade(&self.inner);
        // Tauri setup calls this on the main thread without an ambient Tokio
        // context, so the monitor must use Tauri's global runtime.
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tick.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                prune(&mut inner.lock().unwrap(), Instant::now());
            }
        });
    }
}
fn compact_request(request: &mut StartRequest) {
    for value in [
        &mut request.request_id,
        &mut request.source.connection_id,
        &mut request.source.schema,
        &mut request.target.connection_id,
        &mut request.target.schema,
    ] {
        *value = std::mem::take(value).into_boxed_str().into_string();
    }
}
fn generation(s: &State, id: &str) -> u64 {
    connection_scope(s, id).map_or(0, |c| c.generation)
}
fn connection_scope<'a>(s: &'a State, id: &str) -> Option<&'a Scope> {
    s.connections
        .iter()
        .find(|(connection_id, _)| connection_id == id)
        .map(|(_, scope)| scope)
}
fn involves(e: &Entry, id: &str) -> bool {
    e.status.source.connection_id == id || e.status.target.connection_id == id
}
fn cancel(e: &mut Entry, reason: CompareError) {
    if e.stop_reason.is_none() {
        e.stop_reason = Some(reason);
    }
    e.cancel.send_replace(true);
    e.status.state = StatusState::Cancelling;
}
fn prune(s: &mut State, now: Instant) {
    s.jobs.retain(|e| {
        e.active
            || (!e.invalidated
                && e.finished
                    .is_some_and(|at| now.saturating_duration_since(at) < RESULT_TTL))
    });
    while s.jobs.iter().filter(|e| !e.active).count() > MAX_TERMINAL {
        let oldest = s
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.active)
            .min_by_key(|(_, e)| e.finished)
            .map(|(i, _)| i)
            .unwrap();
        s.jobs.remove(oldest);
    }
    s.requests
        .retain(|r| now.saturating_duration_since(r.created) < RESULT_TTL);
    s.jobs.shrink_to(MAX_ACTIVE + MAX_TERMINAL);
    s.connections.shrink_to(MAX_FENCE_SCOPES);
}
