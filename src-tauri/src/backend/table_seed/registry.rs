use super::*;
use crate::postgres::{backup::native::Ownership, dedicated::DriverJoins};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
pub(super) struct Entry {
    pub observation: TableSeedObservation,
    pub intent: TableSeedIntent,
    pub ready: Option<Arc<Ready>>,
    pub control: Arc<Control>,
    pub owner: Ownership,
    pub drivers: DriverJoins,
    pub permit: Option<OwnedSemaphorePermit>,
    pub execution: Option<OwnedSemaphorePermit>,
    pub created: Instant,
    pub finished: Option<Instant>,
}
#[derive(Default)]
pub(super) struct State {
    pub jobs: HashMap<TableSeedAttemptId, Entry>,
    pub revision: u64,
    pub closed: bool,
    pub retiring: Option<Option<String>>,
}
#[cfg(test)]
pub(super) type Inspector = Arc<
    dyn Fn(
            TableSeedIntent,
            TableSeedConnection,
        ) -> futures_util::future::BoxFuture<
            'static,
            Result<Plan, crate::postgres::native_table_seed::Failure>,
        > + Send
        + Sync,
>;
#[cfg(test)]
pub(super) type Runner = Arc<
    dyn Fn(
            Arc<Control>,
        ) -> futures_util::future::BoxFuture<
            'static,
            crate::postgres::native_table_seed::Execution,
        > + Send
        + Sync,
>;
pub(in crate::backend) struct Registry {
    #[cfg(test)]
    pub(super) inspector: Mutex<Option<Inspector>>,
    #[cfg(test)]
    pub(super) runner: Mutex<Option<Runner>>,
    pub(super) state: Mutex<State>,
    pub owner: Ownership,
    pub(super) reviews: Arc<Semaphore>,
    pub(super) executions: Arc<Semaphore>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            #[cfg(test)]
            inspector: Default::default(),
            #[cfg(test)]
            runner: Default::default(),
            state: Default::default(),
            owner: Default::default(),
            reviews: Arc::new(Semaphore::new(MAX_TABLE_SEED_ACTIVE)),
            executions: Arc::new(Semaphore::new(
                MAX_TABLE_SEED_EXECUTION_POOL_BYTES / MAX_TABLE_SEED_EXECUTION_BYTES,
            )),
        }
    }
}
fn touches(intent: &TableSeedIntent, connection: Option<&str>) -> bool {
    connection.is_none_or(|c| intent.endpoint.connection_id == c)
}
fn unresolved(entry: &Entry) -> bool {
    !entry.observation.phase.terminal()
        || entry.observation.cleanup != TableSeedCleanup::Complete
        || !entry.owner.settled()
}
impl Registry {
    fn prune(state: &mut State) {
        for entry in state.jobs.values_mut() {
            if matches!(
                entry.observation.phase,
                TableSeedPhase::NeedsRecipe
                    | TableSeedPhase::ReadyReview
                    | TableSeedPhase::AwaitingConfirmation
            ) && entry.created.elapsed() > Duration::from_secs(300)
            {
                entry.control.cancel();
                cancel_ready(entry);
            }
        }
        state.jobs.retain(|_, e| {
            unresolved(e)
                || e.finished
                    .is_none_or(|at| at.elapsed() < Duration::from_secs(3600))
        });
        while state.jobs.values().filter(|e| !unresolved(e)).count() > MAX_TABLE_SEED_TERMINAL {
            let oldest = state
                .jobs
                .iter()
                .filter(|(_, e)| !unresolved(e))
                .min_by_key(|(_, e)| e.finished)
                .map(|(id, _)| *id);
            if let Some(id) = oldest {
                state.jobs.remove(&id);
            } else {
                break;
            }
        }
    }
    pub fn begin(
        &self,
        backend: &Backend,
        id: TableSeedAttemptId,
        intent: TableSeedIntent,
    ) -> Result<TableSeedObservation, TableSeedError> {
        let _submission = backend.0.submission.lock().unwrap();
        intent
            .checked_heap_bytes()
            .ok_or(TableSeedError::InvalidRequest)?;
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(TableSeedError::InvalidRequest);
        }
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        if state.closed
            || backend.0.closing.load(std::sync::atomic::Ordering::Acquire)
            || state
                .retiring
                .as_ref()
                .is_some_and(|c| touches(&intent, c.as_deref()))
        {
            return Err(TableSeedError::Closing);
        }
        if state.jobs.contains_key(&id) {
            return Err(TableSeedError::DuplicateAttempt);
        }
        if state.jobs.values().filter(|e| unresolved(e)).count() >= MAX_TABLE_SEED_ACTIVE
            || state
                .jobs
                .values()
                .any(|e| unresolved(e) && touches(&e.intent, Some(&intent.endpoint.connection_id)))
        {
            return Err(TableSeedError::Busy);
        }
        let permit = self
            .reviews
            .clone()
            .try_acquire_owned()
            .map_err(|_| TableSeedError::Busy)?;
        let observation = TableSeedObservation {
            attempt_id: id,
            endpoint: intent.endpoint.clone(),
            row_count: intent.row_count,
            seed_used: None,
            phase: TableSeedPhase::Preparing,
            outcome: TableSeedOutcome::NotStarted,
            cleanup: TableSeedCleanup::Pending,
            rows_generated: 0,
            issue: None,
            failure: None,
            diagnostic: None,
            receipt: None,
            change_revision: None,
        };
        let owner = self.owner.child();
        state.jobs.insert(
            id,
            Entry {
                observation: observation.clone(),
                intent,
                ready: None,
                control: Arc::default(),
                owner: owner.clone(),
                drivers: backend.0.tasks.child(),
                permit: Some(permit),
                execution: None,
                created: Instant::now(),
                finished: None,
            },
        );
        let inner = backend.0.clone();
        if owner
            .spawn(async move { service::prepare(inner, id).await })
            .is_err()
        {
            state.jobs.remove(&id);
            return Err(TableSeedError::Busy);
        }
        Ok(observation)
    }
    pub fn review(
        &self,
        backend: &Backend,
        id: TableSeedAttemptId,
    ) -> Result<TableSeedReview, TableSeedError> {
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        let e = state.jobs.get(&id).ok_or(TableSeedError::Missing)?;
        if !matches!(
            e.observation.phase,
            TableSeedPhase::ReadyReview | TableSeedPhase::AwaitingConfirmation
        ) {
            return Err(TableSeedError::StaleReview);
        }
        Ok(TableSeedReview {
            owner: Arc::downgrade(&backend.0),
            id,
            ready: e.ready.clone().ok_or(TableSeedError::StaleReview)?,
        })
    }
    pub fn submit(
        &self,
        backend: &Backend,
        review: TableSeedReview,
        confirmed: bool,
    ) -> Result<TableSeedSubmission, TableSeedError> {
        let _submission = backend.0.submission.lock().unwrap();
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(TableSeedError::InvalidRequest);
        }
        if !review.belongs_to(backend) {
            return Err(TableSeedError::StaleReview);
        }
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        if state.closed
            || state.retiring.as_ref().is_some_and(|c| {
                c.as_ref()
                    .is_none_or(|id| id == &review.description().endpoint.connection_id)
            })
        {
            return Err(TableSeedError::Closing);
        }
        let e = state
            .jobs
            .get_mut(&review.id)
            .ok_or(TableSeedError::Missing)?;
        if !matches!(
            e.observation.phase,
            TableSeedPhase::ReadyReview | TableSeedPhase::AwaitingConfirmation
        ) || e.control.is_cancelled()
            || !e
                .ready
                .as_ref()
                .is_some_and(|r| Arc::ptr_eq(r, &review.ready))
        {
            return Err(TableSeedError::StaleReview);
        }
        if review.ready.requires_confirmation && !confirmed {
            e.observation.phase = TableSeedPhase::AwaitingConfirmation;
            return Ok(TableSeedSubmission::NeedsConfirmation(Box::new(
                TableSeedConfirmation { review },
            )));
        }
        let execution = self
            .executions
            .clone()
            .try_acquire_owned()
            .map_err(|_| TableSeedError::Busy)?;
        e.execution = Some(execution);
        e.observation.phase = TableSeedPhase::Running;
        e.observation.outcome = TableSeedOutcome::Pending;
        e.observation.cleanup = TableSeedCleanup::Pending;
        // Inspection's drained driver owner is latched aborted. Execution gets a
        // fresh child whose joins still belong to the host and this attempt.
        e.drivers = backend.0.tasks.child();
        let inner = backend.0.clone();
        let id = review.id;
        if e.owner
            .spawn(async move { service::execute(inner, id, review, confirmed).await })
            .is_err()
        {
            e.execution.take();
            e.observation.phase = TableSeedPhase::ReadyReview;
            e.observation.outcome = TableSeedOutcome::NotStarted;
            e.observation.cleanup = TableSeedCleanup::Complete;
            return Err(TableSeedError::Busy);
        }
        Ok(TableSeedSubmission::Accepted(Box::new(
            e.observation.clone(),
        )))
    }
    pub fn get(&self, id: TableSeedAttemptId) -> Result<TableSeedObservation, TableSeedError> {
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        let e = state.jobs.get(&id).ok_or(TableSeedError::Missing)?;
        Ok(observe(e))
    }
    pub fn list(&self) -> Result<TableSeedList, TableSeedError> {
        let mut state = self.state.lock().unwrap();
        Self::prune(&mut state);
        let mut jobs = state.jobs.values().map(observe).collect::<Vec<_>>();
        jobs.sort_by_key(|o| o.attempt_id.0);
        let list = TableSeedList {
            jobs,
            change_revision: state.revision,
        };
        list.checked_heap_bytes().ok_or(TableSeedError::Limit)?;
        Ok(list)
    }
    pub fn cancel(&self, id: TableSeedAttemptId) -> Result<TableSeedObservation, TableSeedError> {
        let mut state = self.state.lock().unwrap();
        let e = state.jobs.get_mut(&id).ok_or(TableSeedError::Missing)?;
        e.control.cancel();
        if matches!(
            e.observation.phase,
            TableSeedPhase::NeedsRecipe
                | TableSeedPhase::ReadyReview
                | TableSeedPhase::AwaitingConfirmation
        ) {
            cancel_ready(e);
        } else if !e.observation.phase.terminal() {
            e.observation.phase = TableSeedPhase::Cancelling;
        }
        Ok(observe(e))
    }
    pub fn release(&self, id: TableSeedAttemptId) -> Result<(), TableSeedError> {
        let mut state = self.state.lock().unwrap();
        if state.jobs.get(&id).is_some_and(unresolved) {
            return Err(TableSeedError::Busy);
        }
        state.jobs.remove(&id).ok_or(TableSeedError::Missing)?;
        Ok(())
    }
    pub fn involves(&self, connection: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .jobs
            .values()
            .any(|e| unresolved(e) && touches(&e.intent, Some(connection)))
    }
    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        for e in state.jobs.values_mut() {
            e.control.cancel();
            if matches!(
                e.observation.phase,
                TableSeedPhase::NeedsRecipe
                    | TableSeedPhase::ReadyReview
                    | TableSeedPhase::AwaitingConfirmation
            ) {
                cancel_ready(e);
            }
        }
    }
    pub async fn drain_until(&self, deadline: tokio::time::Instant) -> Result<(), String> {
        self.owner
            .drain_until(deadline)
            .await
            .map_err(|_| "Table seed owned cleanup did not join".to_string())?;
        if self
            .state
            .lock()
            .unwrap()
            .jobs
            .values()
            .any(|e| e.observation.cleanup == TableSeedCleanup::Failed)
        {
            return Err("Table seed cleanup remains unresolved".into());
        }
        Ok(())
    }
}
fn observe(e: &Entry) -> TableSeedObservation {
    let mut value = e.observation.clone();
    value.rows_generated = e.control.rows.load(std::sync::atomic::Ordering::Acquire);
    if matches!(value.phase, TableSeedPhase::Running) && e.control.state.lock().unwrap().commit {
        value.phase = TableSeedPhase::Committing;
    }
    value
}
fn cancel_ready(e: &mut Entry) {
    e.observation.phase = TableSeedPhase::Cancelled;
    e.observation.outcome = TableSeedOutcome::NotStarted;
    e.observation.cleanup = TableSeedCleanup::Complete;
    e.observation.failure = Some(TableSeedError::Cancelled);
    if let Some(ready) = e.ready.take().filter(|r| r.plan.description.is_some()) {
        e.observation.receipt = Some(TableSeedReceipt {
            attempt_id: e.observation.attempt_id,
            description: ready.plan.description.clone().expect("runnable review"),
            outcome: TableSeedOutcome::NotStarted,
            failure: Some(TableSeedError::Cancelled),
            diagnostic: None,
        });
    }
    e.permit.take();
    e.finished = Some(Instant::now());
}
pub(in crate::backend) struct RetirementGuard(Arc<super::super::Inner>);
impl Drop for RetirementGuard {
    fn drop(&mut self) {
        self.0.table_seed.state.lock().unwrap().retiring = None;
    }
}
/// Caller holds development_gate through mutation. The latch also blocks synchronous
/// registrations between joining the old owner and the manager generation fence.
pub(in crate::backend) async fn retire_connection(
    inner: &Arc<super::super::Inner>,
    connection: Option<&str>,
    deadline: tokio::time::Instant,
) -> Result<RetirementGuard, String> {
    let workers = {
        let mut s = inner.table_seed.state.lock().unwrap();
        if s.retiring.is_some() {
            return Err("Table seed retirement already in progress".into());
        }
        s.retiring = Some(connection.map(str::to_owned));
        let mut workers = Vec::new();
        for e in s
            .jobs
            .values_mut()
            .filter(|e| touches(&e.intent, connection))
        {
            e.control.cancel();
            if matches!(
                e.observation.phase,
                TableSeedPhase::NeedsRecipe
                    | TableSeedPhase::ReadyReview
                    | TableSeedPhase::AwaitingConfirmation
            ) {
                cancel_ready(e);
            }
            workers.push(e.owner.clone());
        }
        workers
    };
    let guard = RetirementGuard(inner.clone());
    for worker in workers {
        worker.drain_until(deadline).await.map_err(|_| {
            "Table seed cleanup could not join before connection change".to_string()
        })?;
    }
    if inner
        .table_seed
        .state
        .lock()
        .unwrap()
        .jobs
        .values()
        .any(|e| touches(&e.intent, connection) && unresolved(e))
    {
        return Err("Table seed cleanup remains unresolved".into());
    }
    Ok(guard)
}
