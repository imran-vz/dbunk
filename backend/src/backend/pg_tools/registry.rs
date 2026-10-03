use super::*;
use crate::postgres::backup::{
    manager::{Admission, PgToolJobManager},
    native::{source, Ownership},
    protocol as legacy,
};
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub(super) struct Entry {
    pub observation: PgToolObservation,
    pub target: Option<PgToolTarget>,
    pub requires_confirmation: bool,
    pub revision: uuid::Uuid,
    pub admission: Option<Admission>,
    pub intent: Option<PgToolIntent>,
    pub source: Option<Arc<source::Snapshot>>,
    pub cancel: watch::Sender<bool>,
    pub copy_cancel: source::Cancellation,
    pub work: Ownership,
    pub job_id: Option<String>,
    pub finished: Option<Instant>,
    pub cancelled_at: Option<Instant>,
    pub data_retired: bool,
}
#[cfg(test)]
pub(super) type TestRunner = Arc<
    dyn Fn(
            crate::postgres::backup::manager::JobContext,
            crate::StoredConnection,
            crate::postgres::backup::runner::Request,
        ) -> futures_util::future::BoxFuture<
            'static,
            Result<crate::postgres::backup::runner::Ready, legacy::PgToolJobError>,
        > + Send
        + Sync,
>;
#[derive(Default)]
struct State {
    entries: HashMap<PgToolAttemptId, Entry>,
    restore_change_revision: u64,
}
#[derive(Clone, Default)]
pub(in crate::backend) struct Registry {
    state: Arc<Mutex<State>>,
    pub(super) owner: Ownership,
    #[cfg(test)]
    pub(super) test_runner: Arc<Mutex<Option<TestRunner>>>,
}
impl Registry {
    pub(super) fn require_connection_settled(
        &self,
        connection: Option<&str>,
    ) -> Result<(), String> {
        let state = self.state.lock().unwrap();
        if state.entries.values().any(|entry| {
            connection.is_none_or(|id| entry.observation.connection_id == id)
                && (!entry.observation.phase.terminal()
                    || entry.observation.cleanup != PgToolCleanup::Complete
                    || !entry.work.settled())
        }) {
            return Err("Cancel or finish PostgreSQL backup/restore jobs and wait for their cleanup before changing this connection or credential storage".into());
        }
        Ok(())
    }
    pub(in crate::backend) fn ownership(&self) -> Ownership {
        self.owner.clone()
    }
    fn prune(state: &mut State) {
        state.entries.retain(|_, entry| {
            entry
                .finished
                .is_none_or(|at| at.elapsed() < Duration::from_secs(3600))
                || entry.observation.cleanup != PgToolCleanup::Complete
                || !entry.work.settled()
        });
        let mut terminal = state
            .entries
            .iter()
            .filter(|(_, e)| {
                e.finished.is_some()
                    && e.observation.cleanup == PgToolCleanup::Complete
                    && e.work.settled()
            })
            .map(|(id, e)| (*id, e.finished))
            .collect::<Vec<_>>();
        terminal.sort_by_key(|(_, at)| *at);
        for (id, _) in terminal.into_iter().take(
            state
                .entries
                .values()
                .filter(|e| {
                    e.finished.is_some()
                        && e.observation.cleanup == PgToolCleanup::Complete
                        && e.work.settled()
                })
                .count()
                .saturating_sub(MAX_PG_TOOL_TERMINAL),
        ) {
            state.entries.remove(&id);
        }
    }
    pub(super) fn begin(
        &self,
        backend: &Backend,
        id: PgToolAttemptId,
        connection: String,
        intent: PgToolIntent,
    ) -> Result<PgToolObservation, PgToolError> {
        if connection.is_empty()
            || connection.len() > 256
            || connection.chars().any(char::is_control)
            || intent.checked_heap_bytes().is_none()
        {
            return Err(PgToolError::InvalidRequest);
        }
        let _runtime = tokio::runtime::Handle::try_current().map_err(|_| PgToolError::Closing)?;
        let _submission = backend.0.submission.lock().unwrap();
        if backend.0.closing.load(Ordering::SeqCst) {
            return Err(PgToolError::Closing);
        }
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        if state.entries.contains_key(&id) {
            return Err(PgToolError::DuplicateAttempt);
        }
        if state
            .entries
            .values()
            .filter(|e| {
                e.observation.cleanup != PgToolCleanup::Complete || !e.observation.phase.terminal()
            })
            .count()
            >= MAX_PG_TOOL_ACTIVE
        {
            return Err(PgToolError::Busy);
        }
        if state.entries.values().any(|entry| {
            entry.observation.connection_id == connection
                && (entry.observation.cleanup != PgToolCleanup::Complete
                    || !entry.observation.phase.terminal())
        }) {
            return Err(PgToolError::Busy);
        }
        let admission = backend
            .0
            .state
            .pg_tool_jobs
            .admission(&connection)
            .map_err(map_error)?;
        let lifecycle = admission.cancellation();
        let (cancel, _) = watch::channel(false);
        let work = self.owner.child();
        let observation = PgToolObservation {
            attempt_id: id,
            connection_id: connection.into_boxed_str().into_string(),
            kind: intent.kind,
            format: intent.format,
            scope: intent.scope.clone(),
            clean: intent.clean,
            file_name: intent
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(PgToolError::InvalidPath)?
                .into(),
            phase: PgToolPhase::Preparing,
            effect: PgToolEffect::NotStarted,
            cleanup: PgToolCleanup::Pending,
            source_bytes: None,
            bytes_processed: None,
            tool_version: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            failure: None,
            diagnostic: None,
            restore_change_revision: None,
        };
        observation
            .checked_heap_bytes()
            .ok_or(PgToolError::InvalidRequest)?;
        state.entries.insert(
            id,
            Entry {
                observation: observation.clone(),
                target: None,
                requires_confirmation: false,
                revision: uuid::Uuid::new_v4(),
                admission: Some(admission),
                intent: Some(intent),
                source: None,
                cancel,
                copy_cancel: Default::default(),
                work: work.clone(),
                job_id: None,
                finished: None,
                cancelled_at: None,
                data_retired: false,
            },
        );
        drop(state);
        let registry = self.clone();
        let inner = backend.0.clone();
        work.spawn(async move {
            prepare::prepare(inner, registry, id).await;
        })
        .map_err(|_| PgToolError::Busy)?;
        let registry = self.clone();
        let manager = backend.0.state.pg_tool_jobs.clone();
        let inner = backend.0.clone();
        self.owner
            .spawn(async move {
                registry.monitor(inner, manager, id, lifecycle).await;
            })
            .map_err(|_| PgToolError::Busy)?;
        Ok(observation)
    }
    pub(super) fn update<T>(
        &self,
        id: PgToolAttemptId,
        f: impl FnOnce(&mut Entry) -> T,
    ) -> Result<T, PgToolError> {
        let mut state = self.state.lock().unwrap();
        let entry = state.entries.get_mut(&id).ok_or(PgToolError::Missing)?;
        Ok(f(entry))
    }
    pub(super) fn get(&self, id: PgToolAttemptId) -> Result<PgToolObservation, PgToolError> {
        self.update(id, |entry| entry.observation.clone())
    }
    pub(super) fn list(&self, connection: Option<&str>) -> Result<PgToolJobList, PgToolError> {
        if connection.is_some_and(|id| id.is_empty() || id.len() > 256) {
            return Err(PgToolError::InvalidRequest);
        }
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        let mut jobs = Vec::with_capacity(MAX_PG_TOOL_JOBS);
        for entry in state
            .entries
            .values()
            .filter(|e| connection.is_none_or(|id| e.observation.connection_id == id))
        {
            jobs.push(entry.observation.clone());
        }
        jobs.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.attempt_id.to_string().cmp(&a.attempt_id.to_string()))
        });
        let list = PgToolJobList {
            jobs,
            restore_change_revision: state.restore_change_revision,
        };
        list.checked_heap_bytes()
            .ok_or(PgToolError::InvalidRequest)?;
        Ok(list)
    }
    pub(super) fn review(
        &self,
        backend: &Backend,
        id: PgToolAttemptId,
    ) -> Result<PgToolReview, PgToolError> {
        self.update(id, |e| {
            if !matches!(
                e.observation.phase,
                PgToolPhase::ReadyReview | PgToolPhase::AwaitingConfirmation
            ) || !e.admission.as_ref().is_some_and(Admission::current)
            {
                return Err(PgToolError::StaleReview);
            }
            let review = PgToolReview {
                owner: Arc::downgrade(&backend.0),
                attempt: id,
                revision: e.revision,
                observation: e.observation.clone(),
                target: e.target.clone().ok_or(PgToolError::StaleReview)?,
                requires_confirmation: e.requires_confirmation,
            };
            if review.retained_bytes() > MAX_PG_TOOL_REVIEW_BYTES {
                return Err(PgToolError::InvalidRequest);
            }
            Ok(review)
        })?
    }
    pub(super) fn await_confirmation(&self, review: &PgToolReview) -> Result<(), PgToolError> {
        self.update(review.attempt, |e| {
            if e.revision != review.revision
                || !matches!(
                    e.observation.phase,
                    PgToolPhase::ReadyReview | PgToolPhase::AwaitingConfirmation
                )
                || !e.admission.as_ref().is_some_and(Admission::current)
            {
                return Err(PgToolError::StaleReview);
            }
            e.observation.phase = PgToolPhase::AwaitingConfirmation;
            Ok(())
        })?
    }
    pub(super) fn dispatch(
        &self,
        backend: &Backend,
        review: PgToolReview,
        confirmed: bool,
    ) -> Result<PgToolObservation, PgToolError> {
        let _submission = backend.0.submission.lock().unwrap();
        if backend.0.closing.load(Ordering::SeqCst) {
            return Err(PgToolError::Closing);
        }
        let (observation, work) = self.update(review.attempt, |e| {
            if e.revision != review.revision
                || e.observation.phase
                    != if confirmed {
                        PgToolPhase::AwaitingConfirmation
                    } else {
                        PgToolPhase::ReadyReview
                    }
                || !e.admission.as_ref().is_some_and(Admission::current)
            {
                return Err(PgToolError::StaleReview);
            }
            e.observation.phase = PgToolPhase::Queued;
            Ok((e.observation.clone(), e.work.clone()))
        })??;
        let registry = self.clone();
        let inner = backend.0.clone();
        let id = review.attempt;
        work.spawn(async move {
            prepare::dispatch(inner, registry, id, confirmed).await;
        })
        .map_err(|_| PgToolError::Busy)?;
        Ok(observation)
    }
    pub(super) fn cancel(
        &self,
        manager: &PgToolJobManager,
        id: PgToolAttemptId,
    ) -> Result<PgToolObservation, PgToolError> {
        self.update(id, |e| {
            if !e.observation.phase.terminal() {
                if let Some(job) = &e.job_id {
                    // The manager's atomic publication claim decides whether
                    // cancellation won. Do not invent Cancelling after it lost.
                    if manager
                        .cancel(job)
                        .is_ok_and(|snapshot| snapshot.phase != legacy::PgToolJobPhase::Cancelling)
                    {
                        return e.observation.clone();
                    }
                }
                e.cancel.send_replace(true);
                e.copy_cancel.cancel();
                e.cancelled_at.get_or_insert_with(Instant::now);
                e.observation.phase = PgToolPhase::Cancelling;
            }
            e.observation.clone()
        })
    }
    pub(super) fn release(
        &self,
        manager: &PgToolJobManager,
        id: PgToolAttemptId,
    ) -> Result<(), PgToolError> {
        let mut state = self.state.lock().unwrap();
        if let Some(e) = state.entries.get(&id) {
            if !e.observation.phase.terminal()
                || e.observation.cleanup != PgToolCleanup::Complete
                || !e.work.settled()
            {
                return Err(PgToolError::Active);
            }
            if let Some(job) = &e.job_id {
                manager.release(job).map_err(map_error)?;
            }
        }
        state.entries.remove(&id);
        Ok(())
    }
    // Admission stays in the registry until admitted, joined filesystem cleanup
    // succeeds. Refusal, panic and unlink failure preserve the cleanup fence.
    async fn cleanup_resources(
        &self,
        id: PgToolAttemptId,
        manager: Option<PgToolJobManager>,
    ) -> bool {
        let registry = self.clone();
        let work = self.owner.spawn_blocking(move || {
            if let Some(manager) = manager {
                if let Some(job) = registry
                    .update(id, |entry| entry.job_id.clone())
                    .map_err(|_| ())?
                {
                    manager.native_cleanup_archive(&job)?;
                }
            }
            let source = registry
                .update(id, |entry| {
                    if entry
                        .source
                        .as_ref()
                        .is_some_and(|s| Arc::strong_count(s) != 1)
                    {
                        return Err(());
                    }
                    Ok(entry.source.take())
                })
                .map_err(|_| ())??;
            if let Some(source) = source {
                if source.remove().is_err() {
                    let _ = registry.update(id, |entry| entry.source = Some(source));
                    return Err(());
                }
                drop(source);
            }
            let resources = registry
                .update(id, |entry| (entry.intent.take(), entry.admission.take()))
                .map_err(|_| ())?;
            drop(resources);
            Ok::<(), ()>(())
        });
        match work {
            Ok(done) => matches!(done.await, Ok(Ok(Ok(())))),
            Err(()) => false,
        }
    }
    pub(super) async fn finish(&self, id: PgToolAttemptId, error: PgToolError) {
        let cleaned = error != PgToolError::Cleanup && self.cleanup_resources(id, None).await;
        let _ = self.update(id, |e| {
            e.observation.phase = if error == PgToolError::Cancelled {
                PgToolPhase::Cancelled
            } else {
                PgToolPhase::Failed
            };
            e.observation.failure = Some(error);
            e.observation.finished_at = Some(chrono::Utc::now().to_rfc3339());
            e.observation.cleanup = if cleaned {
                PgToolCleanup::Complete
            } else {
                PgToolCleanup::Failed
            };
            e.finished = Some(Instant::now());
        });
    }
    async fn monitor(
        &self,
        inner: Arc<crate::backend::Inner>,
        manager: PgToolJobManager,
        id: PgToolAttemptId,
        mut lifecycle: watch::Receiver<bool>,
    ) {
        loop {
            let Ok((job, cancel, phase, settled)) = self.update(id, |e| {
                (
                    e.job_id.clone(),
                    *e.cancel.borrow() || *lifecycle.borrow(),
                    e.observation.phase,
                    e.work.settled(),
                )
            }) else {
                return;
            };
            if *lifecycle.borrow() {
                let _ = self.cancel(&manager, id);
            }
            if let Some(job) = job {
                if let Some((snapshot, dispatched, joined)) = manager.native_observe(&job) {
                    let terminal = snapshot.phase.terminal();
                    if terminal {
                        let needs_retirement = self
                            .update(id, |entry| {
                                entry.observation.kind == PgToolKind::Restore
                                    && !entry.data_retired
                                    && (dispatched
                                        || snapshot.phase == legacy::PgToolJobPhase::Completed)
                            })
                            .unwrap_or(false);
                        if needs_retirement {
                            let connection = snapshot.connection_id.clone();
                            let _gate = inner.development_gate.lock().await;
                            let retired = crate::backend::data::retire_data(
                                &inner,
                                &inner.state,
                                Some(&connection),
                            )
                            .await
                            .is_ok();
                            let _ = self.update(id, |entry| {
                                entry.data_retired = retired;
                                if !retired {
                                    entry.observation.cleanup = PgToolCleanup::Failed;
                                }
                            });
                        } else {
                            let _ = self.update(id, |entry| entry.data_retired = true);
                        }
                    }
                    {
                        let mut state = self.state.lock().unwrap();
                        let Some(entry) = state.entries.get_mut(&id) else {
                            return;
                        };
                        entry.observation.phase = phase_from(snapshot.phase);
                        entry.observation.bytes_processed = snapshot.bytes_processed;
                        entry.observation.tool_version = snapshot.tool_version;
                        entry.observation.failure = snapshot.failure.clone().map(map_error);
                        entry.observation.diagnostic =
                            snapshot.failure.as_ref().and_then(diagnostic);
                        entry.observation.effect =
                            if snapshot.phase == legacy::PgToolJobPhase::Completed {
                                PgToolEffect::Succeeded
                            } else if dispatched {
                                if terminal {
                                    PgToolEffect::Unknown
                                } else {
                                    PgToolEffect::Pending
                                }
                            } else {
                                PgToolEffect::NotStarted
                            };
                        if terminal {
                            entry.observation.finished_at = snapshot.finished_at;
                            entry.finished.get_or_insert_with(Instant::now);
                        }
                        let change = terminal
                            && entry.observation.kind == PgToolKind::Restore
                            && matches!(
                                entry.observation.effect,
                                PgToolEffect::Succeeded | PgToolEffect::Unknown
                            )
                            && entry.observation.restore_change_revision.is_none();
                        if change {
                            state.restore_change_revision = state
                                .restore_change_revision
                                .checked_add(1)
                                .expect("bounded process lifetime restore revision");
                            let revision = state.restore_change_revision;
                            state
                                .entries
                                .get_mut(&id)
                                .unwrap()
                                .observation
                                .restore_change_revision = Some(revision);
                        }
                    }
                    if terminal
                        && joined
                        && settled
                        && self.update(id, |entry| entry.data_retired).unwrap_or(false)
                    {
                        let cleaned = self.cleanup_resources(id, Some(manager.clone())).await;
                        let _ = self.update(id, |e| {
                            e.observation.cleanup = if cleaned {
                                PgToolCleanup::Complete
                            } else {
                                PgToolCleanup::Failed
                            };
                        });
                        return;
                    }
                }
            } else if cancel && settled && !phase.terminal() {
                self.finish(id, PgToolError::Cancelled).await;
                return;
            } else if phase.terminal() && settled {
                return;
            }
            let _ = self.update(id, |e| {
                if e.cancelled_at
                    .is_some_and(|at| at.elapsed() > Duration::from_secs(5))
                    && e.observation.cleanup != PgToolCleanup::Complete
                {
                    e.observation.cleanup = PgToolCleanup::Failed;
                }
            });
            tokio::select! {_=tokio::time::sleep(Duration::from_millis(50))=>{},_=wait_lifecycle_cancel(&mut lifecycle),if !*lifecycle.borrow()=>{}}
        }
    }
    /// Caller holds the startup gate. Restore reserves the data-source fence
    /// before dispatch and retains it until terminal resource cleanup is proven.
    pub(in crate::backend) fn restore_in_progress(&self, connection: &str) -> bool {
        self.state.lock().unwrap().entries.values().any(|entry| {
            entry.observation.connection_id == connection
                && entry.observation.kind == PgToolKind::Restore
                && entry.observation.cleanup != PgToolCleanup::Complete
                && !matches!(
                    entry.observation.phase,
                    PgToolPhase::Preparing
                        | PgToolPhase::ReadyReview
                        | PgToolPhase::AwaitingConfirmation
                )
        })
    }
    pub(in crate::backend) fn close(&self, manager: &PgToolJobManager) {
        let ids = self
            .state
            .lock()
            .unwrap()
            .entries
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            let _ = self.cancel(manager, id);
        }
    }
    pub(in crate::backend) async fn drain_until(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<(), String> {
        self.owner.drain_until(deadline).await.map_err(|_| {
            "PostgreSQL file-job cleanup did not join every process and filesystem owner before the shared deadline".to_owned()
        })?;
        if self
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .any(|entry| entry.observation.cleanup != PgToolCleanup::Complete)
        {
            return Err("PostgreSQL file-job resource cleanup was not established".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "connection_settled_tests.rs"]
mod connection_settled_tests;
pub(super) fn map_error(error: legacy::PgToolJobError) -> PgToolError {
    use legacy::PgToolJobError as E;
    match error {
        E::UnsupportedEngine | E::InvalidRequest { .. } => PgToolError::InvalidRequest,
        E::ConnectionClosing => PgToolError::Closing,
        E::JobLimitReached => PgToolError::Busy,
        E::JobNotFound => PgToolError::Missing,
        E::JobActive => PgToolError::Active,
        E::DestinationExists => PgToolError::DestinationExists,
        E::ToolUnavailable { .. } => PgToolError::ToolUnavailable,
        E::ToolFailed { .. } => PgToolError::ToolFailed,
        E::Io { .. } => PgToolError::FileIo,
        E::Timeout { .. } => PgToolError::Timeout,
        E::PolicyBlocked { .. } | E::PolicyNeedsConfirmation { .. } => PgToolError::PolicyBlocked,
        E::Cancelled => PgToolError::Cancelled,
    }
}
fn phase_from(phase: legacy::PgToolJobPhase) -> PgToolPhase {
    use legacy::PgToolJobPhase as P;
    match phase {
        P::Queued => PgToolPhase::Queued,
        P::Preflight => PgToolPhase::Preflight,
        P::Running => PgToolPhase::Running,
        P::Finalizing => PgToolPhase::Finalizing,
        P::Completed => PgToolPhase::Completed,
        P::Cancelling => PgToolPhase::Cancelling,
        P::Cancelled => PgToolPhase::Cancelled,
        P::Failed => PgToolPhase::Failed,
    }
}
fn diagnostic(error: &legacy::PgToolJobError) -> Option<PgToolDiagnostic> {
    let detail = match error {
        legacy::PgToolJobError::ToolFailed {
            tool,
            exit_code,
            message,
        } => PgToolDiagnostic {
            tool: Some(tool.clone()),
            exit_code: *exit_code,
            operation: None,
            message: message.clone(),
        },
        legacy::PgToolJobError::ToolUnavailable { tool } => PgToolDiagnostic {
            tool: Some(tool.clone()),
            exit_code: None,
            operation: None,
            message: "A supported patched PostgreSQL client is required".into(),
        },
        legacy::PgToolJobError::Io { operation, message } => PgToolDiagnostic {
            tool: None,
            exit_code: None,
            operation: Some(operation.clone()),
            message: message.clone(),
        },
        legacy::PgToolJobError::Timeout { operation } => PgToolDiagnostic {
            tool: None,
            exit_code: None,
            operation: Some(operation.clone()),
            message: "The operation exceeded its deadline".into(),
        },
        _ => return None,
    };
    detail.checked_heap_bytes().map(|_| detail)
}

// Manager start consumes the pending admission sender. A closed false watch
// means ownership transferred, not cancellation and not an immediately ready
// polling wakeup. Execution cancellation now belongs to the manager's watch.
pub(super) async fn wait_lifecycle_cancel(receiver: &mut watch::Receiver<bool>) {
    if receiver.wait_for(|cancelled| *cancelled).await.is_err() {
        std::future::pending::<()>().await;
    }
}
