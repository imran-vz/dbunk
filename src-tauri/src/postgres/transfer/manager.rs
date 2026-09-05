//! Bounded in-memory CSV jobs and expiring reviews, owned independently of UI tabs.
use super::{protocol::*, runner::Review};
use futures_util::FutureExt;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::watch;

const MAX_ACTIVE: usize = 4;
const MAX_REVIEWS: usize = 8;
const MAX_TERMINAL: usize = 32;
const REVIEW_TTL: Duration = Duration::from_secs(300);
const RETENTION: Duration = Duration::from_secs(3600);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const OPEN: u8 = 0;
const CANCEL: u8 = 1;
const FINALIZING: u8 = 2;
const SUCCEEDED: u8 = 3;
const SAFE_INTEGER: u64 = 9_007_199_254_740_991;
type Completion = futures_util::future::BoxFuture<'static, ()>;
#[derive(Default)]
struct Scope {
    closing: usize,
    generation: u64,
}
struct Pending {
    connection_id: String,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
struct ReviewEntry {
    review: Arc<Review>,
    created: Instant,
    connection: u64,
    global: u64,
}
struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
    claim: Arc<AtomicU8>,
    task: Option<tokio::task::JoinHandle<()>>,
}
struct Entry {
    snapshot: Snapshot,
    active: Option<Control>,
    finished: Option<Instant>,
}
#[derive(Default)]
struct State {
    jobs: HashMap<String, Entry>,
    pending: HashMap<String, Pending>,
    reviews: HashMap<String, ReviewEntry>,
    connections: HashMap<String, Scope>,
    global: Scope,
}
#[derive(Clone, Default)]
pub(crate) struct TransferManager {
    inner: Arc<Mutex<State>>,
}

/// A reservation covers credential/tunnel and file/catalog inspection work too.
pub(crate) struct Admission {
    id: String,
    connection_id: String,
    connection: u64,
    global: u64,
    manager: Weak<Mutex<State>>,
    cancel: watch::Receiver<bool>,
    done: watch::Sender<bool>,
    reserved: bool,
}
impl Admission {
    pub(crate) async fn cancelled(&mut self) {
        let _ = self.cancel.wait_for(|v| *v).await;
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        if self.reserved {
            if let Some(inner) = self.manager.upgrade() {
                inner.lock().unwrap().pending.remove(&self.id);
            }
            self.done.send_replace(true);
        }
    }
}
#[derive(Clone)]
pub(crate) struct JobContext {
    manager: TransferManager,
    pub(crate) job_id: String,
    cancel: watch::Receiver<bool>,
    claim: Arc<AtomicU8>,
}
impl JobContext {
    pub(crate) async fn cancelled(&self) {
        let mut rx = self.cancel.clone();
        let _ = rx.wait_for(|v| *v).await;
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.claim.load(Ordering::Acquire) == CANCEL
    }
    /// Cancellation and the irreversible commit/publication boundary have one winner.
    pub(crate) fn begin_finalizing(&self) -> bool {
        if self
            .claim
            .compare_exchange(OPEN, FINALIZING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        let mut state = self.manager.inner.lock().unwrap();
        if let Some(entry) = state.jobs.get_mut(&self.job_id) {
            entry.snapshot.phase = Phase::Finalizing;
        }
        true
    }
    pub(crate) fn progress(&self, bytes: u64, rows: Option<u64>) {
        let mut state = self.manager.inner.lock().unwrap();
        if let Some(entry) = state.jobs.get_mut(&self.job_id) {
            if entry.active.is_none() {
                return;
            }
            if entry.snapshot.phase == Phase::Preparing {
                entry.snapshot.phase = Phase::Running;
            }
            entry.snapshot.bytes_processed = bytes.min(SAFE_INTEGER);
            entry.snapshot.rows_processed = rows.map(|v| v.min(SAFE_INTEGER));
        }
    }
    /// Called immediately after acknowledged COMMIT or completed file publication.
    pub(crate) fn succeeded(&self, rows_committed: Option<u64>) {
        // A worker may only report success after winning finalization.
        if self
            .claim
            .compare_exchange(FINALIZING, SUCCEEDED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let mut state = self.manager.inner.lock().unwrap();
            if let Some(entry) = state.jobs.get_mut(&self.job_id) {
                entry.snapshot.rows_committed = rows_committed.map(|v| v.min(SAFE_INTEGER));
            }
        }
    }
}
impl TransferManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn start_monitor(&self) {
        let weak = Arc::downgrade(&self.inner);
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tick.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                prune(&mut inner.lock().unwrap(), Instant::now());
            }
        });
    }
    pub(crate) fn admission(&self, id: &str) -> Result<Admission, TransferError> {
        if id.is_empty() || id.len() > 128 {
            return Err(TransferError::invalid(
                "connectionId",
                "Invalid connection identifier",
            ));
        }
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        if s.global.closing > 0 || s.connections.get(id).is_some_and(|c| c.closing > 0) {
            return Err(TransferError::ConnectionClosing);
        }
        let active = s.jobs.values().filter(|e| e.active.is_some()).count();
        if active + s.pending.len() >= MAX_ACTIVE
            || s.jobs
                .values()
                .any(|e| e.active.is_some() && e.snapshot.connection_id == id)
            || s.pending.values().any(|p| p.connection_id == id)
        {
            return Err(TransferError::JobLimitReached);
        }
        let token = uuid::Uuid::new_v4().to_string();
        let (cancel, rx) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        s.pending.insert(
            token.clone(),
            Pending {
                connection_id: id.into(),
                cancel,
                done: done_rx,
            },
        );
        Ok(Admission {
            id: token,
            connection_id: id.into(),
            connection: s.connections.get(id).map_or(0, |c| c.generation),
            global: s.global.generation,
            manager: Arc::downgrade(&self.inner),
            cancel: rx,
            done,
            reserved: true,
        })
    }
    pub(crate) fn insert_review(
        &self,
        admission: &Admission,
        mut review: Review,
    ) -> Result<Inspection, TransferError> {
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        check_admission(&s, admission)?;
        if s.reviews.len() >= MAX_REVIEWS {
            return Err(TransferError::JobLimitReached);
        }
        let token = uuid::Uuid::new_v4().to_string();
        review.inspection.inspection_token = token.clone();
        let inspection = review.inspection.clone();
        s.reviews.insert(
            token,
            ReviewEntry {
                review: Arc::new(review),
                created: Instant::now(),
                connection: admission.connection,
                global: admission.global,
            },
        );
        Ok(inspection)
    }
    pub(crate) fn review(&self, token: &str) -> Result<Arc<Review>, TransferError> {
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        let entry = s
            .reviews
            .get(token)
            .ok_or(TransferError::InspectionExpired)?;
        let id = &entry.review.payload.connection_id;
        if entry.global != s.global.generation
            || entry.connection != s.connections.get(id).map_or(0, |c| c.generation)
        {
            return Err(TransferError::InspectionExpired);
        }
        Ok(entry.review.clone())
    }
    pub(crate) fn release_review(&self, token: &str) {
        self.inner.lock().unwrap().reviews.remove(token);
    }
    pub(crate) fn start<R, F>(
        &self,
        mut admission: Admission,
        token: &str,
        mut snapshot: Snapshot,
        run: R,
        completion: Completion,
    ) -> Result<Snapshot, TransferError>
    where
        R: FnOnce(JobContext) -> F + Send + 'static,
        F: Future<Output = Result<(), TransferError>> + Send + 'static,
    {
        let mut s = self.inner.lock().unwrap();
        if let Err(e) = check_admission(&s, &admission) {
            drop(s);
            return Err(e);
        }
        let valid_review = s.reviews.get(token).is_some_and(|e| {
            e.created.elapsed() < REVIEW_TTL
                && e.connection == admission.connection
                && e.global == admission.global
                && e.review.payload.connection_id == admission.connection_id
        });
        if !valid_review {
            drop(s);
            return Err(TransferError::InspectionExpired);
        }
        s.reviews.remove(token);
        s.pending.remove(&admission.id);
        admission.reserved = false;
        admission.done.send_replace(true);
        snapshot.job_id = uuid::Uuid::new_v4().to_string();
        let (cancel, rx) = watch::channel(false);
        let (done, _) = watch::channel(false);
        let claim = Arc::new(AtomicU8::new(OPEN));
        let context = JobContext {
            manager: self.clone(),
            job_id: snapshot.job_id.clone(),
            cancel: rx,
            claim: claim.clone(),
        };
        let manager = self.clone();
        let task = tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(run(context.clone()))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(TransferError::Io {
                        operation: "worker".into(),
                        reason: "Transfer worker stopped unexpectedly".into(),
                    })
                });
            manager.complete(context, result, completion).await;
        });
        s.jobs.insert(
            snapshot.job_id.clone(),
            Entry {
                snapshot: snapshot.clone(),
                active: Some(Control {
                    cancel,
                    done,
                    claim,
                    task: Some(task),
                }),
                finished: None,
            },
        );
        Ok(snapshot)
    }
    async fn complete(
        &self,
        ctx: JobContext,
        mut result: Result<(), TransferError>,
        completion: Completion,
    ) {
        let claim = ctx.claim.load(Ordering::Acquire);
        if claim == SUCCEEDED {
            result = Ok(());
        } else if result.is_ok() {
            result = Err(TransferError::Io {
                operation: "worker".into(),
                reason: "Transfer ended without acknowledged completion".into(),
            });
        }
        // Losing a worker after entering COMMIT cannot establish rollback.
        if claim == FINALIZING
            && matches!(&result,Err(TransferError::Io{operation,..}) if operation=="worker")
        {
            let import = self
                .get(&ctx.job_id)
                .is_ok_and(|s| s.direction == Direction::Import);
            if import {
                result = Err(TransferError::OutcomeUnknown);
            }
        }
        if result.is_ok()
            && tokio::time::timeout(
                Duration::from_secs(2),
                std::panic::AssertUnwindSafe(completion).catch_unwind(),
            )
            .await
            .is_err()
        {
            log::warn!("CSV transfer completion effects timed out");
        }
        self.terminal(&ctx.job_id, result);
    }
    fn terminal(&self, id: &str, result: Result<(), TransferError>) {
        let mut s = self.inner.lock().unwrap();
        let Some(e) = s.jobs.get_mut(id) else { return };
        let Some(active) = e.active.take() else {
            return;
        };
        let claim = active.claim.load(Ordering::Acquire);
        let phase = if claim == SUCCEEDED {
            Phase::Completed
        } else if matches!(result, Err(TransferError::OutcomeUnknown)) {
            Phase::OutcomeUnknown
        } else if claim == CANCEL || matches!(result, Err(TransferError::Cancelled)) {
            Phase::Cancelled
        } else {
            Phase::Failed
        };
        e.snapshot.phase = phase;
        e.snapshot.failure = if phase == Phase::Completed {
            None
        } else {
            result.err()
        };
        e.snapshot.finished_at = Some(chrono::Utc::now().to_rfc3339());
        e.finished = Some(Instant::now());
        active.done.send_replace(true);
        prune(&mut s, Instant::now());
    }
    pub(crate) fn get(&self, id: &str) -> Result<Snapshot, TransferError> {
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        s.jobs
            .get(id)
            .map(|e| e.snapshot.clone())
            .ok_or(TransferError::JobNotFound)
    }
    pub(crate) fn list(&self, id: Option<&str>) -> Vec<Snapshot> {
        let mut s = self.inner.lock().unwrap();
        prune(&mut s, Instant::now());
        let mut jobs: Vec<_> = s
            .jobs
            .values()
            .filter(|e| id.is_none_or(|id| e.snapshot.connection_id == id))
            .map(|e| e.snapshot.clone())
            .collect();
        jobs.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then(a.job_id.cmp(&b.job_id))
        });
        jobs
    }
    pub(crate) fn release(&self, id: &str) -> Result<(), TransferError> {
        let mut s = self.inner.lock().unwrap();
        if s.jobs.get(id).is_some_and(|e| e.active.is_some()) {
            return Err(TransferError::JobActive);
        }
        s.jobs.remove(id);
        Ok(())
    }
    pub(crate) fn cancel(&self, id: &str) -> Result<Snapshot, TransferError> {
        let mut s = self.inner.lock().unwrap();
        let e = s.jobs.get_mut(id).ok_or(TransferError::JobNotFound)?;
        let accepted = cancel_entry(e);
        let snapshot = e.snapshot.clone();
        let done = e.active.as_ref().map(|c| c.done.subscribe());
        drop(s);
        if let Some(mut done) = done.filter(|_| accepted) {
            let manager = self.clone();
            let id = id.to_owned();
            tokio::spawn(async move {
                if tokio::time::timeout(STOP_TIMEOUT, done.wait_for(|finished| *finished))
                    .await
                    .is_err()
                {
                    manager.abort_cancelled(None, Some(&id)).await;
                }
            });
        }
        Ok(snapshot)
    }
    async fn abort_cancelled(&self, connection: Option<&str>, job: Option<&str>) {
        let (tasks, waits) = {
            let mut s = self.inner.lock().unwrap();
            let mut tasks = Vec::new();
            let mut waits = Vec::new();
            for (id, e) in s.jobs.iter_mut().filter(|(id, e)| {
                job.is_none_or(|j| *id == j)
                    && connection.is_none_or(|c| e.snapshot.connection_id == c)
            }) {
                let Some(active) = e.active.as_mut() else {
                    continue;
                };
                if active.claim.load(Ordering::Acquire) != CANCEL {
                    continue;
                }
                waits.push(active.done.subscribe());
                if let Some(task) = active.task.take() {
                    task.abort();
                    e.snapshot.failure = Some(TransferError::Timeout {
                        operation: "cleanup".into(),
                    });
                    tasks.push((id.clone(), task));
                }
            }
            (tasks, waits)
        };
        // Keep the active slot until aborted work is actually joined. A stuck task
        // cannot free capacity for an unbounded number of replacement workers.
        futures_util::future::join_all(tasks.into_iter().map(|(id, task)| {
            let manager = self.clone();
            async move {
                let _ = task.await;
                manager.terminal(
                    &id,
                    Err(TransferError::Timeout {
                        operation: "cleanup".into(),
                    }),
                );
            }
        }))
        .await;
        // A concurrent cancel watchdog may already own the JoinHandle. Its done
        // signal still keeps this call behind that join and terminal transition.
        futures_util::future::join_all(waits.into_iter().map(|mut done| async move {
            let _ = done.wait_for(|finished| *finished).await;
        }))
        .await;
    }
    async fn begin_teardown(&self, id: Option<&str>, stop_timeout: Duration) {
        let waits = {
            let mut s = self.inner.lock().unwrap();
            let scope = match id {
                Some(id) => s.connections.entry(id.into()).or_default(),
                None => &mut s.global,
            };
            scope.closing += 1;
            scope.generation += 1;
            s.reviews
                .retain(|_, e| id.is_some_and(|id| e.review.payload.connection_id != id));
            let mut waits = Vec::new();
            for e in s
                .jobs
                .values_mut()
                .filter(|e| id.is_none_or(|id| e.snapshot.connection_id == id))
            {
                cancel_entry(e);
                if let Some(c) = &e.active {
                    waits.push(c.done.subscribe());
                }
            }
            for p in s
                .pending
                .values()
                .filter(|p| id.is_none_or(|id| p.connection_id == id))
            {
                p.cancel.send_replace(true);
                waits.push(p.done.clone());
            }
            waits
        };
        let stopped = tokio::time::timeout(
            stop_timeout.saturating_sub(Duration::from_millis(100)),
            futures_util::future::join_all(waits.into_iter().map(|mut rx| async move {
                let _ = rx.wait_for(|v| *v).await;
            })),
        )
        .await
        .is_ok();
        if !stopped {
            self.abort_cancelled(id, None).await;
        }
    }
    pub(crate) async fn begin_connection_teardown(&self, id: &str) {
        self.begin_teardown(Some(id), STOP_TIMEOUT).await;
    }
    pub(crate) async fn end_connection_teardown(&self, id: &str) {
        let mut s = self.inner.lock().unwrap();
        let c = s.connections.entry(id.into()).or_default();
        c.closing = c.closing.saturating_sub(1);
    }
    pub(crate) async fn begin_global_teardown(&self) {
        self.begin_teardown(None, STOP_TIMEOUT).await;
    }
    pub(crate) async fn end_global_teardown(&self) {
        let mut s = self.inner.lock().unwrap();
        s.global.closing = s.global.closing.saturating_sub(1);
    }
    pub(crate) async fn close_all(&self) {
        self.begin_global_teardown().await;
    }
}
fn check_admission(s: &State, a: &Admission) -> Result<(), TransferError> {
    if s.global.closing > 0
        || s.global.generation != a.global
        || s.connections
            .get(&a.connection_id)
            .is_some_and(|c| c.closing > 0 || c.generation != a.connection)
        || !s.pending.get(&a.id).is_some_and(|p| !*p.cancel.borrow())
    {
        Err(TransferError::ConnectionClosing)
    } else {
        Ok(())
    }
}
fn cancel_entry(e: &mut Entry) -> bool {
    let Some(c) = &e.active else { return false };
    if c.claim
        .compare_exchange(OPEN, CANCEL, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    e.snapshot.phase = Phase::Cancelling;
    c.cancel.send_replace(true);
    true
}
fn prune(s: &mut State, now: Instant) {
    s.reviews
        .retain(|_, e| now.duration_since(e.created) < REVIEW_TTL);
    s.jobs
        .retain(|_, e| e.finished.is_none_or(|t| now.duration_since(t) < RETENTION));
    let mut terminal: Vec<_> = s
        .jobs
        .iter()
        .filter_map(|(id, e)| e.finished.map(|t| (id.clone(), t)))
        .collect();
    terminal.sort_by_key(|(_, t)| *t);
    let excess = terminal.len().saturating_sub(MAX_TERMINAL);
    for (id, _) in terminal.into_iter().take(excess) {
        s.jobs.remove(&id);
    }
}

#[cfg(test)]
pub(crate) mod tests;
