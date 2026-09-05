use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures_util::FutureExt;
use tokio::sync::watch;

use super::protocol::*;
use super::runner::Ready;

const MAX_ACTIVE: usize = 4;
const MAX_REAPER_PERMITS: usize = 4;
const MAX_TERMINAL: usize = 32;
const RETENTION: Duration = Duration::from_secs(3600);
// Teardown reserves its final 100ms for aborting and joining work stalled
// outside the process supervisor. The reaper permit preserves child cleanup
// independently when the native reap wait consumes the foreground deadline.
const TEARDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(2);

type Completion = futures_util::future::BoxFuture<'static, ()>;

struct Entry {
    snapshot: PgToolJobSnapshot,
    active: Option<Control>,
    finished: Option<Instant>,
}
struct Control {
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
    outcome: Arc<OutcomeClaim>,
    task: Option<tokio::task::JoinHandle<()>>,
    reaper_permit: bool,
    reaper_in_flight: bool,
}
struct Pending {
    connection_id: String,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
#[derive(Default)]
struct Scope {
    closing: usize,
    generation: u64,
}
#[derive(Default)]
struct State {
    jobs: HashMap<String, Entry>,
    pending: HashMap<String, Pending>,
    connections: HashMap<String, Scope>,
    global: Scope,
    // A permit starts at admission and transfers to a detached reaper on a
    // reap timeout. This bounds orphan cleanup without retaining a normal job slot.
    reaper_permits: usize,
}

const OUTCOME_OPEN: u8 = 0;
const OUTCOME_CANCELLED: u8 = 1;
const OUTCOME_PUBLISHING: u8 = 2;
const OUTCOME_IRREVERSIBLE: u8 = 3;
const OUTCOME_CANCELLED_FINAL: u8 = 4;

struct OutcomeClaim(AtomicU8);
impl OutcomeClaim {
    fn new() -> Self {
        Self(AtomicU8::new(OUTCOME_OPEN))
    }
    fn cancel(&self) -> bool {
        self.0
            .compare_exchange(
                OUTCOME_OPEN,
                OUTCOME_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    fn begin_publication(&self) -> bool {
        self.0
            .compare_exchange(
                OUTCOME_OPEN,
                OUTCOME_PUBLISHING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
    fn irreversible_success(&self) {
        let mut current = self.0.load(Ordering::Acquire);
        while matches!(current, OUTCOME_OPEN | OUTCOME_CANCELLED) {
            match self.0.compare_exchange_weak(
                current,
                OUTCOME_IRREVERSIBLE,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(next) => current = next,
            }
        }
    }
    fn cancelled(&self) -> bool {
        matches!(
            self.0.load(Ordering::Acquire),
            OUTCOME_CANCELLED | OUTCOME_CANCELLED_FINAL
        )
    }
    fn irreversible(&self) -> bool {
        self.0.load(Ordering::Acquire) == OUTCOME_IRREVERSIBLE
    }
    fn finalize_cancellation(&self) -> bool {
        matches!(
            self.0.compare_exchange(
                OUTCOME_CANCELLED,
                OUTCOME_CANCELLED_FINAL,
                Ordering::AcqRel,
                Ordering::Acquire,
            ),
            Ok(_) | Err(OUTCOME_CANCELLED_FINAL)
        )
    }
}

/// A lifecycle-owned reservation for connection resolution and job admission.
/// Dropping it removes the pending work and wakes any teardown fence.
pub(crate) struct Admission {
    id: String,
    connection: u64,
    global: u64,
    manager: Weak<Mutex<State>>,
    cancel: watch::Receiver<bool>,
    done: watch::Sender<bool>,
    reserved: bool,
}
impl Admission {
    pub(crate) async fn cancelled(&mut self) {
        let _ = self.cancel.wait_for(|cancelled| *cancelled).await;
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        if !self.reserved {
            return;
        }
        if let Some(inner) = self.manager.upgrade() {
            let mut state = inner.lock().unwrap();
            if state.pending.remove(&self.id).is_some() {
                state.reaper_permits = state.reaper_permits.saturating_sub(1);
            }
        }
        self.done.send_replace(true);
    }
}

#[derive(Clone, Default)]
pub(crate) struct PgToolJobManager {
    inner: Arc<Mutex<State>>,
}

#[derive(Clone)]
pub(crate) struct JobContext {
    manager: PgToolJobManager,
    pub(crate) job_id: String,
    cancel: watch::Receiver<bool>,
    outcome: Arc<OutcomeClaim>,
}

impl PgToolJobManager {
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

    pub(crate) fn admission(&self, connection_id: &str) -> Result<Admission, PgToolJobError> {
        let mut state = self.inner.lock().unwrap();
        if state.global.closing > 0
            || state
                .connections
                .get(connection_id)
                .is_some_and(|scope| scope.closing > 0)
        {
            return Err(PgToolJobError::ConnectionClosing);
        }
        let active_for_connection = state
            .jobs
            .values()
            .any(|entry| entry.active.is_some() && entry.snapshot.connection_id == connection_id);
        let pending_for_connection = state
            .pending
            .values()
            .any(|pending| pending.connection_id == connection_id);
        let active = state
            .jobs
            .values()
            .filter(|entry| entry.active.is_some())
            .count();
        if active + state.pending.len() >= MAX_ACTIVE
            || active_for_connection
            || pending_for_connection
            || state.reaper_permits >= MAX_REAPER_PERMITS
        {
            return Err(PgToolJobError::JobLimitReached);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (cancel, cancel_rx) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        state.pending.insert(
            id.clone(),
            Pending {
                connection_id: connection_id.into(),
                cancel,
                done: done_rx,
            },
        );
        state.reaper_permits += 1;
        Ok(Admission {
            id,
            connection: state
                .connections
                .get(connection_id)
                .map_or(0, |scope| scope.generation),
            global: state.global.generation,
            manager: Arc::downgrade(&self.inner),
            cancel: cancel_rx,
            done,
            reserved: true,
        })
    }

    pub(crate) fn start<R, F>(
        &self,
        mut admission: Admission,
        mut snapshot: PgToolJobSnapshot,
        run: R,
        completion: Completion,
    ) -> Result<PgToolJobSnapshot, PgToolJobError>
    where
        R: FnOnce(JobContext) -> F + Send + 'static,
        F: Future<Output = Result<Ready, PgToolJobError>> + Send + 'static,
    {
        let mut state = self.inner.lock().unwrap();
        let reservation_matches = state.pending.get(&admission.id).is_some_and(|pending| {
            pending.connection_id == snapshot.connection_id && !*pending.cancel.borrow()
        });
        let generation = state
            .connections
            .get(&snapshot.connection_id)
            .map_or(0, |scope| scope.generation);
        let closing = state.global.closing > 0
            || state
                .connections
                .get(&snapshot.connection_id)
                .is_some_and(|scope| scope.closing > 0);
        if !reservation_matches
            || closing
            || generation != admission.connection
            || state.global.generation != admission.global
        {
            return Err(PgToolJobError::ConnectionClosing);
        }
        state.pending.remove(&admission.id);
        admission.reserved = false;
        admission.done.send_replace(true);

        prune(&mut state, Instant::now());
        snapshot.job_id = uuid::Uuid::new_v4().to_string();
        let (cancel, receiver) = watch::channel(false);
        let (done, _) = watch::channel(false);
        let outcome = Arc::new(OutcomeClaim::new());
        let context = JobContext {
            manager: self.clone(),
            job_id: snapshot.job_id.clone(),
            cancel: receiver,
            outcome: outcome.clone(),
        };
        let manager = self.clone();
        let task_context = context.clone();
        let task_done = done.clone();
        let task = tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(async { run(task_context.clone()).await })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(PgToolJobError::Io {
                        operation: "worker".into(),
                        message: "Worker stopped unexpectedly".into(),
                    })
                });
            manager
                .complete(&task_context.job_id, result, completion)
                .await;
            task_done.send_replace(true);
        });
        state.jobs.insert(
            snapshot.job_id.clone(),
            Entry {
                snapshot: snapshot.clone(),
                active: Some(Control {
                    cancel,
                    done,
                    outcome,
                    task: Some(task),
                    reaper_permit: true,
                    reaper_in_flight: false,
                }),
                finished: None,
            },
        );
        drop(state);
        Ok(snapshot)
    }

    async fn complete(
        &self,
        job_id: &str,
        result: Result<Ready, PgToolJobError>,
        completion: Completion,
    ) {
        let publication = match result {
            Ok(ready) => {
                let outcome = {
                    let state = self.inner.lock().unwrap();
                    let Some(entry) = state.jobs.get(job_id) else {
                        return;
                    };
                    let Some(active) = &entry.active else { return };
                    active.outcome.clone()
                };
                if ready.requires_publication_claim() && !outcome.begin_publication() {
                    drop(ready);
                    Err(PgToolJobError::Cancelled)
                } else {
                    tokio::task::spawn_blocking(move || ready.publish())
                        .await
                        .unwrap_or_else(|_| {
                            Err(PgToolJobError::Io {
                                operation: "publish".into(),
                                message: "File worker stopped".into(),
                            })
                        })
                }
            }
            Err(error) => Err(error),
        };
        let outcome = {
            let state = self.inner.lock().unwrap();
            state
                .jobs
                .get(job_id)
                .and_then(|entry| entry.active.as_ref())
                .map(|active| active.outcome.clone())
        };
        let cancelled = outcome
            .as_ref()
            .is_some_and(|outcome| outcome.cancelled() && outcome.finalize_cancellation());
        let irreversible = outcome.is_some_and(|outcome| outcome.irreversible());
        let result = if cancelled {
            match publication {
                Err(error @ PgToolJobError::Timeout { .. }) => Err(error),
                _ => Err(PgToolJobError::Cancelled),
            }
        } else if irreversible {
            Ok(())
        } else {
            publication
        };
        if result.is_ok() {
            // Success is terminal only after its audit/activity effects complete.
            if tokio::time::timeout(
                COMPLETION_TIMEOUT,
                std::panic::AssertUnwindSafe(completion).catch_unwind(),
            )
            .await
            .is_err()
            {
                log::warn!("PostgreSQL tool job completion effects timed out");
            }
        }
        self.publish_terminal(job_id, result);
    }

    fn publish_terminal(&self, job_id: &str, result: Result<(), PgToolJobError>) {
        let mut state = self.inner.lock().unwrap();
        let Some(entry) = state.jobs.get_mut(job_id) else {
            return;
        };
        let Some(active) = entry.active.take() else {
            return;
        };
        let cancelled = active.outcome.cancelled() && active.outcome.finalize_cancellation();
        let phase = if cancelled {
            PgToolJobPhase::Cancelled
        } else if result.is_ok() {
            PgToolJobPhase::Completed
        } else {
            PgToolJobPhase::Failed
        };
        if active.reaper_permit {
            state.reaper_permits = state.reaper_permits.saturating_sub(1);
        }
        let entry = state.jobs.get_mut(job_id).expect("active job retained");
        transition(&mut entry.snapshot, phase).expect("worker terminal transition");
        entry.snapshot.failure = result.err();
        entry.snapshot.finished_at = Some(chrono::Utc::now().to_rfc3339());
        entry.finished = Some(Instant::now());
        active.done.send_replace(true);
        prune(&mut state, Instant::now());
    }

    pub(crate) fn get(&self, job_id: &str) -> Result<PgToolJobSnapshot, PgToolJobError> {
        self.inner
            .lock()
            .unwrap()
            .jobs
            .get(job_id)
            .map(|e| e.snapshot.clone())
            .ok_or(PgToolJobError::JobNotFound)
    }
    pub(crate) fn list(&self, connection_id: Option<&str>) -> Vec<PgToolJobSnapshot> {
        let state = self.inner.lock().unwrap();
        let mut jobs: Vec<_> = state
            .jobs
            .values()
            .filter(|e| connection_id.is_none_or(|id| e.snapshot.connection_id == id))
            .map(|e| e.snapshot.clone())
            .collect();
        jobs.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then(a.job_id.cmp(&b.job_id))
        });
        jobs
    }
    pub(crate) fn cancel(&self, job_id: &str) -> Result<PgToolJobSnapshot, PgToolJobError> {
        self.cancel_with_timeout(job_id, TEARDOWN_TIMEOUT)
    }
    fn cancel_with_timeout(
        &self,
        job_id: &str,
        timeout: Duration,
    ) -> Result<PgToolJobSnapshot, PgToolJobError> {
        let mut state = self.inner.lock().unwrap();
        let entry = state
            .jobs
            .get_mut(job_id)
            .ok_or(PgToolJobError::JobNotFound)?;
        let accepted = cancel_entry(entry);
        let snapshot = entry.snapshot.clone();
        drop(state);
        if accepted {
            let manager = self.clone();
            let job_id = job_id.to_string();
            tokio::spawn(async move {
                let cleanup_timeout = (timeout / 50).min(Duration::from_millis(100));
                tokio::time::sleep(timeout.saturating_sub(cleanup_timeout)).await;
                manager
                    .abort_stalled_cancellations(None, Some(&job_id), cleanup_timeout)
                    .await;
            });
        }
        Ok(snapshot)
    }
    pub(crate) fn release(&self, job_id: &str) -> Result<(), PgToolJobError> {
        let mut state = self.inner.lock().unwrap();
        if let Some(entry) = state.jobs.get(job_id) {
            if !entry.snapshot.phase.terminal() || entry.active.is_some() {
                return Err(PgToolJobError::JobActive);
            }
        }
        state.jobs.remove(job_id);
        Ok(())
    }

    async fn begin_teardown(&self, connection_id: Option<&str>, timeout: Duration) {
        let cleanup_timeout = (timeout / 50).min(Duration::from_millis(100));
        let cancellation_timeout = timeout.saturating_sub(cleanup_timeout);
        let waits =
            {
                let mut state = self.inner.lock().unwrap();
                let scope = match connection_id {
                    Some(id) => state.connections.entry(id.into()).or_default(),
                    None => &mut state.global,
                };
                scope.closing += 1;
                scope.generation += 1;
                let mut waits = Vec::new();
                for entry in state.jobs.values_mut().filter(|entry| {
                    connection_id.is_none_or(|id| entry.snapshot.connection_id == id)
                }) {
                    let _ = cancel_entry(entry);
                    if let Some(active) = &entry.active {
                        waits.push(active.done.subscribe());
                    }
                }
                for pending in state
                    .pending
                    .values()
                    .filter(|pending| connection_id.is_none_or(|id| pending.connection_id == id))
                {
                    pending.cancel.send_replace(true);
                    waits.push(pending.done.clone());
                }
                waits
            };
        let completed = tokio::time::timeout(cancellation_timeout, async {
            futures_util::future::join_all(waits.into_iter().map(|mut done| async move {
                let _ = done.wait_for(|finished| *finished).await;
            }))
            .await;
        })
        .await
        .is_ok();
        if !completed {
            self.abort_stalled_cancellations(connection_id, None, cleanup_timeout)
                .await;
        }
    }

    async fn abort_stalled_cancellations(
        &self,
        connection_id: Option<&str>,
        job_id: Option<&str>,
        cleanup_timeout: Duration,
    ) {
        let tasks = {
            let mut state = self.inner.lock().unwrap();
            state
                .jobs
                .iter_mut()
                .filter(|(_, entry)| {
                    connection_id.is_none_or(|id| entry.snapshot.connection_id == id)
                        && entry
                            .active
                            .as_ref()
                            .is_some_and(|active| active.outcome.cancelled())
                })
                .filter_map(|(id, entry)| {
                    if job_id.is_some_and(|job_id| id != job_id) {
                        return None;
                    }
                    let active = entry.active.as_mut()?;
                    // Freeze the cancellation outcome before aborting the only
                    // worker that can run success effects. A concurrently
                    // observed restore success wins by promoting first.
                    if !active.outcome.finalize_cancellation() {
                        return None;
                    }
                    let task = active.task.take()?;
                    task.abort();
                    Some((id.clone(), task))
                })
                .collect::<Vec<_>>()
        };
        let (finished, mut finished_rx) = tokio::sync::mpsc::unbounded_channel();
        let task_count = tasks.len();
        let owned_ids: std::collections::HashSet<_> =
            tasks.iter().map(|(id, _)| id.clone()).collect();
        for (id, task) in tasks {
            let finished = finished.clone();
            tokio::spawn(async move {
                let _ = task.await;
                let _ = finished.send(id);
            });
        }
        drop(finished);
        let mut cleaned = std::collections::HashSet::new();
        let _ = tokio::time::timeout(cleanup_timeout, async {
            while cleaned.len() < task_count {
                let Some(id) = finished_rx.recv().await else {
                    break;
                };
                cleaned.insert(id);
            }
        })
        .await;

        let mut state = self.inner.lock().unwrap();
        for id in owned_ids {
            let Some(entry) = state.jobs.get_mut(&id) else {
                continue;
            };
            let Some(active) = entry.active.take() else {
                continue;
            };
            let reaper_in_flight = active.reaper_in_flight;
            if active.reaper_permit {
                state.reaper_permits = state.reaper_permits.saturating_sub(1);
            }
            let entry = state.jobs.get_mut(&id).expect("active job retained");
            transition(&mut entry.snapshot, PgToolJobPhase::Cancelled)
                .expect("stalled cancellation terminal transition");
            entry.snapshot.failure = Some(if reaper_in_flight {
                PgToolJobError::Timeout {
                    operation: "reap".into(),
                }
            } else if cleaned.contains(&id) {
                PgToolJobError::Cancelled
            } else {
                PgToolJobError::Timeout {
                    operation: "cleanup".into(),
                }
            });
            entry.snapshot.finished_at = Some(chrono::Utc::now().to_rfc3339());
            entry.finished = Some(Instant::now());
            active.done.send_replace(true);
        }
        prune(&mut state, Instant::now());
    }

    pub(crate) async fn begin_connection_teardown(&self, id: &str) {
        self.begin_teardown(Some(id), TEARDOWN_TIMEOUT).await;
    }
    pub(crate) async fn end_connection_teardown(&self, id: &str) {
        let mut state = self.inner.lock().unwrap();
        let scope = state.connections.entry(id.into()).or_default();
        scope.closing = scope.closing.saturating_sub(1);
    }
    pub(crate) async fn begin_global_teardown(&self) {
        self.begin_teardown(None, TEARDOWN_TIMEOUT).await;
    }
    pub(crate) async fn end_global_teardown(&self) {
        let mut state = self.inner.lock().unwrap();
        state.global.closing = state.global.closing.saturating_sub(1);
    }
    pub(crate) async fn close_all(&self) {
        self.begin_global_teardown().await;
    }
}

fn cancel_entry(entry: &mut Entry) -> bool {
    let Some(active) = &entry.active else {
        return false;
    };
    if !entry.snapshot.phase.terminal()
        && entry.snapshot.phase != PgToolJobPhase::Cancelling
        && active.outcome.cancel()
    {
        transition(&mut entry.snapshot, PgToolJobPhase::Cancelling).expect("cancel active job");
        active.cancel.send_replace(true);
        true
    } else {
        false
    }
}

pub(super) fn transition(
    snapshot: &mut PgToolJobSnapshot,
    next: PgToolJobPhase,
) -> Result<(), PgToolJobError> {
    use PgToolJobPhase::*;
    let current = snapshot.phase;
    let legal =
        matches!(
            (current, next),
            (Queued, Preflight)
                | (Preflight, Running)
                | (Running, Finalizing)
                | (Finalizing, Completed)
                | (Cancelling, Cancelled)
                | (Cancelling, Finalizing)
        ) || (!current.terminal() && current != Cancelling && matches!(next, Cancelling | Failed));
    if !legal {
        return Err(PgToolJobError::invalid("phase", "Invalid job transition"));
    }
    snapshot.phase = next;
    Ok(())
}

impl JobContext {
    pub(crate) async fn cancelled(&self) {
        let mut cancel = self.cancel.clone();
        let _ = cancel.wait_for(|cancelled| *cancelled).await;
    }
    pub(crate) fn check_cancelled(&self) -> Result<(), PgToolJobError> {
        if self.outcome.cancelled() {
            Err(PgToolJobError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub(crate) fn phase(&self, phase: PgToolJobPhase) -> Result<(), PgToolJobError> {
        self.check_cancelled()?;
        let mut state = self.manager.inner.lock().unwrap();
        let snapshot = &mut state
            .jobs
            .get_mut(&self.job_id)
            .expect("active job retained")
            .snapshot;
        if snapshot.phase == PgToolJobPhase::Cancelling {
            return Err(PgToolJobError::Cancelled);
        }
        transition(snapshot, phase)
    }
    pub(crate) fn irreversible_success(&self) {
        self.outcome.irreversible_success();
    }
    pub(crate) fn phase_after_irreversible_success(
        &self,
        phase: PgToolJobPhase,
    ) -> Result<(), PgToolJobError> {
        self.irreversible_success();
        let mut state = self.manager.inner.lock().unwrap();
        let snapshot = &mut state
            .jobs
            .get_mut(&self.job_id)
            .expect("active job retained")
            .snapshot;
        transition(snapshot, phase)
    }
    pub(crate) fn spawn_reaper<T, F>(
        &self,
        work: F,
    ) -> tokio::sync::oneshot::Receiver<Result<T, tokio::task::JoinError>>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let mut state = self.manager.inner.lock().unwrap();
        let active = state
            .jobs
            .get_mut(&self.job_id)
            .and_then(|entry| entry.active.as_mut())
            .expect("active job owns reaper permit");
        let transferred = active.reaper_permit;
        active.reaper_permit = false;
        active.reaper_in_flight = true;
        // Create the cleanup owner before releasing the manager lock, so a
        // teardown watchdog cannot free its permit between spawn and transfer.
        let task = tokio::spawn(work);
        drop(state);
        let weak = Arc::downgrade(&self.manager.inner);
        let job_id = self.job_id.clone();
        let (finished, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = task.await;
            if transferred {
                if let Some(inner) = weak.upgrade() {
                    let mut state = inner.lock().unwrap();
                    if let Some(active) = state
                        .jobs
                        .get_mut(&job_id)
                        .and_then(|entry| entry.active.as_mut())
                    {
                        active.reaper_in_flight = false;
                    }
                    state.reaper_permits = state.reaper_permits.saturating_sub(1);
                }
            }
            let _ = finished.send(result);
        });
        receiver
    }
    pub(crate) fn progress(&self, bytes: Option<u64>, total: Option<u64>, version: Option<String>) {
        let mut state = self.manager.inner.lock().unwrap();
        let Some(entry) = state.jobs.get_mut(&self.job_id) else {
            return;
        };
        let snapshot = &mut entry.snapshot;
        if !snapshot.phase.terminal() {
            if bytes.is_some() {
                snapshot.bytes_processed = bytes;
            }
            if total.is_some() {
                snapshot.total_bytes = total;
            }
            if version.is_some() {
                snapshot.tool_version = version;
            }
        }
    }
}

fn prune(state: &mut State, now: Instant) {
    state.jobs.retain(|_, e| {
        e.active.is_some()
            || e.finished
                .is_none_or(|at| now.duration_since(at) < RETENTION)
    });
    let mut terminal: Vec<_> = state
        .jobs
        .iter()
        .filter(|(_, e)| e.active.is_none())
        .map(|(id, e)| (id.clone(), e.finished))
        .collect();
    terminal.sort_by_key(|(_, at)| *at);
    let excess = terminal.len().saturating_sub(MAX_TERMINAL);
    for (id, _) in terminal.into_iter().take(excess) {
        state.jobs.remove(&id);
    }
}

#[cfg(test)]
impl PgToolJobManager {
    pub(crate) fn inner_finished_for_test(&self, id: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .jobs
            .get(id)
            .is_none_or(|e| e.active.is_none())
    }
    pub(super) fn expire_for_test(&self) {
        prune(&mut self.inner.lock().unwrap(), Instant::now() + RETENTION);
    }
    pub(crate) async fn begin_connection_teardown_with_timeout_for_test(
        &self,
        id: &str,
        timeout: Duration,
    ) {
        self.begin_teardown(Some(id), timeout).await;
    }
    pub(super) fn pending_for_test(&self) -> usize {
        self.inner.lock().unwrap().pending.len()
    }
    pub(super) fn connection_scopes_for_test(&self) -> usize {
        self.inner.lock().unwrap().connections.len()
    }
    pub(super) fn cancel_with_timeout_for_test(
        &self,
        id: &str,
        timeout: Duration,
    ) -> Result<PgToolJobSnapshot, PgToolJobError> {
        self.cancel_with_timeout(id, timeout)
    }
}
