use super::*;
use crate::postgres::{
    backup::native::Ownership,
    transfer::{
        manager::{Admission, TransferManager},
        native::IoContext,
    },
};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
pub(super) struct InspectionEntry {
    pub source: Option<Arc<artifacts::Artifacts>>,
    pub workbook: Option<Arc<CsvWorkbookData>>,
    pub workbook_permit: Option<Arc<OwnedSemaphorePermit>>,
    pub selected_sheet: Option<u16>,
    pub observation: CsvInspectionObservation,
    pub revision: uuid::Uuid,
    pub created: Instant,
    pub intent: Option<CsvInspectionIntent>,
    pub ready: Option<Arc<InspectionReady>>,
    pub token: Option<String>,
    pub cancel: watch::Sender<bool>,
    pub admission: Option<Admission>,
    pub permit: Option<OwnedSemaphorePermit>,
    pub io: IoContext,
}
pub(super) struct JobEntry {
    pub source: Option<Arc<artifacts::Artifacts>>,
    pub observation: CsvTransferObservation,
    pub review: Option<CsvTransferReview>,
    pub token: String,
    pub cancel: watch::Sender<bool>,
    pub admission: Option<Admission>,
    pub execution: Option<OwnedSemaphorePermit>,
    pub job_id: Option<String>,
    pub finished: Option<Instant>,
    pub import_reserved: bool,
}
#[derive(Default)]
pub(super) struct State {
    pub inspections: HashMap<CsvInspectionId, InspectionEntry>,
    pub jobs: HashMap<CsvTransferAttemptId, JobEntry>,
    pub revision: u64,
    pub retiring: Option<Option<String>>,
}
impl State {
    pub fn retiring(&self, connection: &str) -> bool {
        self.retiring
            .as_ref()
            .is_some_and(|target| target.as_ref().is_none_or(|id| id == connection))
    }
}
#[cfg(test)]
pub(super) type TestRunner = Arc<
    dyn Fn(
            crate::postgres::transfer::manager::JobContext,
            crate::StoredConnection,
            Arc<crate::postgres::transfer::runner::Review>,
            crate::postgres::transfer::runner::RunRequest,
        ) -> futures_util::future::BoxFuture<
            'static,
            Result<(), crate::postgres::transfer::protocol::TransferError>,
        > + Send
        + Sync,
>;
#[cfg(test)]
pub(super) type TestInspector = Arc<
    dyn Fn(
            crate::StoredConnection,
            crate::postgres::transfer::protocol::InspectPayload,
            IoContext,
        ) -> futures_util::future::BoxFuture<
            'static,
            Result<
                crate::postgres::transfer::runner::Review,
                crate::postgres::transfer::protocol::TransferError,
            >,
        > + Send
        + Sync,
>;
pub(in crate::backend) struct Registry {
    pub(super) state: Arc<Mutex<State>>,
    pub owner: Ownership,
    pub inspections: Arc<Semaphore>,
    pub executions: Arc<Semaphore>,
    pub parsers: Arc<Semaphore>,
    #[cfg(test)]
    pub test_runner: Mutex<Option<TestRunner>>,
    #[cfg(test)]
    pub test_inspector: Mutex<Option<TestInspector>>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            state: Default::default(),
            owner: Default::default(),
            parsers: Arc::new(Semaphore::new(
                64 * 1024 * 1024 / crate::xlsx_native::WORKING_BYTES,
            )),
            inspections: Arc::new(Semaphore::new(MAX_CSV_INSPECTIONS)),
            executions: Arc::new(Semaphore::new(
                MAX_CSV_EXECUTION_POOL_BYTES / MAX_CSV_EXECUTION_BYTES,
            )),
            #[cfg(test)]
            test_runner: Default::default(),
            #[cfg(test)]
            test_inspector: Default::default(),
        }
    }
}
impl Registry {
    pub fn begin_inspection(
        &self,
        backend: &Backend,
        id: CsvInspectionId,
        connection: String,
        intent: CsvInspectionIntent,
    ) -> Result<CsvInspectionObservation, CsvError> {
        let _submit = backend.0.submission.lock().unwrap();
        if backend.0.closing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(CsvError::Closing);
        }
        if connection.is_empty()
            || connection.len() > 128
            || connection.capacity() > 256
            || connection.chars().any(char::is_control)
            || intent.checked_heap_bytes().is_none_or(|n| n > 16 * 1024)
        {
            return Err(CsvError::InvalidRequest);
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(CsvError::InvalidRequest);
        }
        let mut state = self.state.lock().unwrap();
        self.prune(&mut state, &backend.0.state.pg_transfers);
        if state.retiring(&connection) {
            return Err(CsvError::Closing);
        }
        if state.inspections.contains_key(&id) {
            return Err(CsvError::DuplicateAttempt);
        }
        if state.inspections.len() >= MAX_CSV_INSPECTIONS
            || state.jobs.values().any(|entry| {
                entry.observation.connection_id == connection
                    && entry.observation.cleanup != CsvCleanup::Complete
            })
        {
            return Err(CsvError::Busy);
        }
        let permit = self
            .inspections
            .clone()
            .try_acquire_owned()
            .map_err(|_| CsvError::Busy)?;
        let admission = backend
            .0
            .state
            .pg_transfers
            .admission(&connection)
            .map_err(prepare::error)?;
        let observation = CsvInspectionObservation {
            inspection_id: id,
            connection_id: connection,
            target: intent.target.clone(),
            direction: intent.direction,
            phase: CsvInspectionPhase::Preparing,
            cleanup: CsvCleanup::Pending,
            expires_at: None,
            failure: None,
            diagnostic: None,
        };
        let (cancel, _) = watch::channel(false);
        let io = IoContext::new(self.owner.child(), backend.0.tasks.child());
        state.inspections.insert(
            id,
            InspectionEntry {
                source: intent
                    .xlsx
                    .then(|| artifacts::Artifacts::new(self.owner.child())),
                workbook: None,
                workbook_permit: None,
                selected_sheet: None,
                observation: observation.clone(),
                revision: uuid::Uuid::new_v4(),
                created: Instant::now(),
                intent: Some(intent),
                ready: None,
                token: None,
                cancel,
                admission: Some(admission),
                permit: Some(permit),
                io,
            },
        );
        let inner = backend.0.clone();
        if self
            .owner
            .spawn(async move { prepare::inspect(inner, id).await })
            .is_err()
        {
            state.inspections.remove(&id);
            return Err(CsvError::Busy);
        }
        Ok(observation)
    }
    fn prune(&self, state: &mut State, manager: &TransferManager) {
        let expired = state
            .inspections
            .iter()
            .filter(|(_, entry)| {
                entry.observation.cleanup == CsvCleanup::Complete
                    && (entry.created.elapsed() >= Duration::from_secs(CSV_INSPECTION_TTL_SECONDS)
                        || (entry.observation.phase == CsvInspectionPhase::Ready
                            && entry
                                .token
                                .as_ref()
                                .is_some_and(|token| manager.review(token).is_err())))
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in expired {
            if let Some(entry) = state.inspections.get_mut(&id) {
                if entry.source.is_some() {
                    entry.cancel.send_replace(true);
                    entry.observation.phase = CsvInspectionPhase::Cancelled;
                    entry.ready.take();
                    if let Some(token) = entry.token.take() {
                        manager.release_review(&token);
                    }
                    self.cleanup_inspection(entry, id);
                    continue;
                }
            }
            if let Some(entry) = state.inspections.remove(&id) {
                if let Some(token) = entry.token {
                    manager.release_review(&token);
                }
            }
        }
        let mut terminal = state
            .jobs
            .iter()
            .filter(|(_, e)| e.observation.cleanup == CsvCleanup::Complete && e.finished.is_some())
            .map(|(id, e)| (*id, e.finished.unwrap()))
            .collect::<Vec<_>>();
        terminal.sort_by_key(|(_, at)| *at);
        let excess = terminal.len().saturating_sub(MAX_CSV_TERMINAL);
        for (i, (id, at)) in terminal.into_iter().enumerate() {
            if i < excess || at.elapsed() >= Duration::from_secs(3600) {
                if let Some(entry) = state.jobs.remove(&id) {
                    if let Some(job) = entry.job_id {
                        let _ = manager.release(&job);
                    }
                }
            }
        }
    }
    pub fn get_inspection(
        &self,
        manager: &TransferManager,
        id: CsvInspectionId,
    ) -> Result<CsvInspectionObservation, CsvError> {
        let mut state = self.state.lock().unwrap();
        self.prune(&mut state, manager);
        state
            .inspections
            .get(&id)
            .map(|e| e.observation.clone())
            .ok_or(CsvError::Missing)
    }
    pub fn list_inspections(
        &self,
        manager: &TransferManager,
        connection: Option<&str>,
    ) -> Result<CsvInspectionList, CsvError> {
        let mut state = self.state.lock().unwrap();
        self.prune(&mut state, manager);
        let mut value = CsvInspectionList {
            inspections: state
                .inspections
                .values()
                .filter(|e| connection.is_none_or(|c| c == e.observation.connection_id))
                .map(|e| e.observation.clone())
                .collect(),
        };
        value
            .inspections
            .sort_by_key(|row| row.inspection_id.to_string());
        value.checked_heap_bytes().ok_or(CsvError::Limit)?;
        Ok(value)
    }
    pub fn inspection(
        &self,
        backend: &Backend,
        id: CsvInspectionId,
    ) -> Result<CsvInspection, CsvError> {
        let state = self.state.lock().unwrap();
        let entry = state.inspections.get(&id).ok_or(CsvError::Missing)?;
        if entry.observation.phase != CsvInspectionPhase::Ready
            || entry.created.elapsed() >= Duration::from_secs(CSV_INSPECTION_TTL_SECONDS)
        {
            return Err(CsvError::InspectionExpired);
        }
        backend
            .0
            .state
            .pg_transfers
            .review(entry.token.as_deref().ok_or(CsvError::InspectionExpired)?)
            .map_err(prepare::error)?;
        Ok(CsvInspection {
            owner: Arc::downgrade(&backend.0),
            id,
            revision: entry.revision,
            ready: entry.ready.clone().ok_or(CsvError::InspectionExpired)?,
        })
    }
    pub fn cancel_inspection(
        &self,
        manager: &TransferManager,
        id: CsvInspectionId,
    ) -> Result<CsvInspectionObservation, CsvError> {
        let mut state = self.state.lock().unwrap();
        let entry = state.inspections.get_mut(&id).ok_or(CsvError::Missing)?;
        entry.cancel.send_replace(true);
        if entry.observation.phase == CsvInspectionPhase::Preparing {
            entry.observation.phase = CsvInspectionPhase::Cancelling;
        } else if matches!(
            entry.observation.phase,
            CsvInspectionPhase::Ready | CsvInspectionPhase::WorkbookReady
        ) {
            entry.observation.phase = CsvInspectionPhase::Cancelled;
            entry.ready.take();
            if let Some(source) = &entry.source {
                source.cancel();
            }
            if let Some(token) = &entry.token {
                manager.release_review(token);
            }
            self.cleanup_inspection(entry, id);
        }
        if let Some(source) = &entry.source {
            source.cancel();
        }
        Ok(entry.observation.clone())
    }
    pub fn release_inspection(
        &self,
        manager: &TransferManager,
        id: CsvInspectionId,
    ) -> Result<(), CsvError> {
        let mut state = self.state.lock().unwrap();
        if state
            .inspections
            .get(&id)
            .is_some_and(|e| e.observation.cleanup != CsvCleanup::Complete)
        {
            return Err(CsvError::Active);
        }
        if let Some(entry) = state.inspections.get_mut(&id) {
            if entry.source.is_some() {
                entry.cancel.send_replace(true);
                entry.observation.phase = CsvInspectionPhase::Cancelled;
                entry.ready.take();
                if let Some(token) = entry.token.take() {
                    manager.release_review(&token);
                }
                self.cleanup_inspection(entry, id);
                return Err(CsvError::Active);
            }
        }
        // Running transfers own their consumed inspection independently; dropping
        // the setup view can never cancel or invalidate an accepted transfer.
        if let Some(entry) = state.inspections.remove(&id) {
            if let Some(token) = entry.token {
                manager.release_review(&token);
            }
        }
        Ok(())
    }
    fn validate_handle(
        &self,
        backend: &Backend,
        inspection: &CsvInspection,
    ) -> Result<(), CsvError> {
        if !inspection.belongs_to(backend) {
            return Err(CsvError::ForeignReview);
        }
        let fresh = self.inspection(backend, inspection.id)?;
        if fresh.revision != inspection.revision || !Arc::ptr_eq(&fresh.ready, &inspection.ready) {
            return Err(CsvError::StaleReview);
        }
        Ok(())
    }
    pub fn review_import(
        &self,
        backend: &Backend,
        inspection: CsvInspection,
        mapping: Vec<CsvMapping>,
    ) -> Result<CsvTransferReview, CsvError> {
        self.validate_handle(backend, &inspection)?;
        if inspection.data().direction != CsvDirection::Import
            || mapping.is_empty()
            || mapping.len() > MAX_CSV_COLUMNS
            || mapping.capacity() > MAX_CSV_COLUMNS
            || mapping.iter().any(|m| {
                m.source_index >= MAX_CSV_COLUMNS
                    || !types::identifier(&m.target_column)
                    || m.target_column.capacity() > 128
            })
        {
            return Err(CsvError::InvalidMapping);
        }
        let legacy = mapping
            .iter()
            .map(|m| crate::postgres::transfer::protocol::ColumnMapping {
                source_index: m.source_index,
                target_column: m.target_column.clone(),
            })
            .collect::<Vec<_>>();
        crate::postgres::transfer::runner::validate_mapping(&inspection.ready.core, &legacy)
            .map_err(|_| CsvError::InvalidMapping)?;
        Ok(CsvTransferReview {
            data: Arc::new(ReviewData {
                inspection,
                mapping,
                destination: None,
            }),
            attempt: None,
        })
    }
    pub fn review_export(
        &self,
        backend: &Backend,
        inspection: CsvInspection,
        destination: PathBuf,
    ) -> Result<CsvTransferReview, CsvError> {
        self.validate_handle(backend, &inspection)?;
        types::path(&destination)?;
        if inspection.data().direction != CsvDirection::Export
            || destination.capacity() > MAX_CSV_PATH_BYTES
        {
            return Err(CsvError::InvalidRequest);
        }
        Ok(CsvTransferReview {
            data: Arc::new(ReviewData {
                inspection,
                mapping: Vec::new(),
                destination: Some(destination),
            }),
            attempt: None,
        })
    }
    pub fn begin_transfer(
        &self,
        backend: &Backend,
        id: CsvTransferAttemptId,
        mut review: CsvTransferReview,
    ) -> Result<CsvTransferSubmission, CsvError> {
        if !review.belongs_to(backend) {
            return Err(CsvError::ForeignReview);
        }
        if let Some(existing) = review.attempt {
            if existing != id {
                return Err(CsvError::StaleReview);
            }
            let current = self.review_transfer(id)?;
            if !Arc::ptr_eq(&current.data, &review.data) {
                return Err(CsvError::StaleReview);
            }
            return Ok(CsvTransferSubmission::NeedsConfirmation(Box::new(
                CsvTransferConfirmation { review: current },
            )));
        }
        self.validate_handle(backend, review.inspection())?;
        let _submit = backend.0.submission.lock().unwrap();
        if backend.0.closing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(CsvError::Closing);
        }
        let mut state = self.state.lock().unwrap();
        self.prune(&mut state, &backend.0.state.pg_transfers);
        if state.retiring(&review.inspection().data().connection_id) {
            return Err(CsvError::Closing);
        }
        if state.jobs.contains_key(&id) {
            return Err(CsvError::DuplicateAttempt);
        }
        let connection = &review.inspection().data().connection_id;
        if state
            .jobs
            .values()
            .filter(|e| e.observation.cleanup != CsvCleanup::Complete)
            .count()
            >= MAX_CSV_ACTIVE
            || state.jobs.values().any(|e| {
                e.observation.connection_id == *connection
                    && e.observation.cleanup != CsvCleanup::Complete
            })
        {
            return Err(CsvError::Busy);
        }
        let admission = backend
            .0
            .state
            .pg_transfers
            .admission(connection)
            .map_err(prepare::error)?;
        let inspected = take_inspection(&mut state, review.inspection())?;
        let token = inspected.token.ok_or(CsvError::InspectionExpired)?;
        review.attempt = Some(id);
        let data = review.inspection().data();
        let needs = review.inspection().ready.requires_confirmation;
        let file_name = match &review.data.destination {
            Some(path) => path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(CsvError::InvalidRequest)?
                .to_owned(),
            None => data.file_name.clone().ok_or(CsvError::InvalidRequest)?,
        };
        let row = CsvTransferObservation {
            workbook: data.workbook.clone(),
            attempt_id: id,
            inspection_id: data.inspection_id,
            connection_id: data.connection_id.clone(),
            target: data.target.clone(),
            direction: data.direction,
            file_name,
            phase: if needs {
                CsvTransferPhase::AwaitingConfirmation
            } else {
                CsvTransferPhase::Preparing
            },
            effect: CsvEffect::NotApplied,
            cleanup: CsvCleanup::Pending,
            started_at: chrono::Utc::now().to_rfc3339(),
            finished_at: None,
            total_bytes: data.total_bytes,
            bytes_processed: 0,
            rows_processed: None,
            rows_committed: None,
            failure: None,
            diagnostic: None,
            import_change_revision: None,
        };
        let (cancel, _) = watch::channel(false);
        state.jobs.insert(
            id,
            JobEntry {
                source: inspected.source,
                observation: row.clone(),
                review: Some(review.clone()),
                token,
                cancel,
                admission: Some(admission),
                execution: None,
                job_id: None,
                finished: None,
                import_reserved: false,
            },
        );
        drop(state);
        if needs {
            Ok(CsvTransferSubmission::NeedsConfirmation(Box::new(
                CsvTransferConfirmation { review },
            )))
        } else {
            self.dispatch(backend, id, false, &review.data)?;
            Ok(CsvTransferSubmission::Accepted(row))
        }
    }
    pub(super) fn dispatch(
        &self,
        backend: &Backend,
        id: CsvTransferAttemptId,
        confirmed: bool,
        expected: &Arc<ReviewData>,
    ) -> Result<(), CsvError> {
        let permit = self
            .executions
            .clone()
            .try_acquire_owned()
            .map_err(|_| CsvError::WorkBudget);
        let mut state = self.state.lock().unwrap();
        let connection = &state
            .jobs
            .get(&id)
            .ok_or(CsvError::Missing)?
            .observation
            .connection_id;
        if state.retiring(connection) {
            return Err(CsvError::Closing);
        }
        let entry = state.jobs.get_mut(&id).ok_or(CsvError::Missing)?;
        if entry
            .review
            .as_ref()
            .is_none_or(|review| !Arc::ptr_eq(&review.data, expected))
            || entry.execution.is_some()
            || entry.job_id.is_some()
            || *entry.cancel.borrow()
            || !matches!(
                entry.observation.phase,
                CsvTransferPhase::Preparing | CsvTransferPhase::AwaitingConfirmation
            )
        {
            return Err(CsvError::StaleReview);
        }
        let permit = match permit {
            Ok(p) => p,
            Err(error) => {
                entry.observation.phase = CsvTransferPhase::Failed;
                entry.observation.failure = Some(error);
                entry.observation.finished_at = Some(chrono::Utc::now().to_rfc3339());
                entry.observation.cleanup = CsvCleanup::Complete;
                entry.finished = Some(Instant::now());
                if entry.source.is_none() {
                    entry.admission.take();
                }
                entry.review.take();
                backend.0.state.pg_transfers.release_review(&entry.token);
                self.cleanup_job(entry, id);
                return Err(error);
            }
        };
        entry.execution = Some(permit);
        entry.observation.phase = CsvTransferPhase::Preparing;
        let inner = backend.0.clone();
        if self
            .owner
            .spawn(async move { prepare::dispatch(inner, id, confirmed).await })
            .is_err()
        {
            if entry.source.is_none() {
                entry.execution.take();
                entry.admission.take();
            }
            entry.review.take();
            backend.0.state.pg_transfers.release_review(&entry.token);
            entry.observation.phase = CsvTransferPhase::Failed;
            entry.observation.failure = Some(CsvError::Busy);
            entry.observation.cleanup = CsvCleanup::Complete;
            entry.observation.finished_at = Some(chrono::Utc::now().to_rfc3339());
            entry.finished = Some(Instant::now());
            self.cleanup_job(entry, id);
            return Err(CsvError::Busy);
        }
        Ok(())
    }
    pub fn confirm(
        &self,
        backend: &Backend,
        confirmation: CsvTransferConfirmation,
    ) -> Result<CsvTransferSubmission, CsvError> {
        if !confirmation.belongs_to(backend) {
            return Err(CsvError::ForeignReview);
        }
        let id = confirmation.attempt_id();
        let current = self.review_transfer(id)?;
        if !Arc::ptr_eq(&current.data, &confirmation.review.data) {
            return Err(CsvError::StaleReview);
        }
        self.dispatch(backend, id, true, &confirmation.review.data)?;
        Ok(CsvTransferSubmission::Accepted(
            self.get(&backend.0.state.pg_transfers, id)?,
        ))
    }
    pub fn review_transfer(&self, id: CsvTransferAttemptId) -> Result<CsvTransferReview, CsvError> {
        let state = self.state.lock().unwrap();
        let entry = state.jobs.get(&id).ok_or(CsvError::Missing)?;
        if entry.observation.phase != CsvTransferPhase::AwaitingConfirmation
            || *entry.cancel.borrow()
            || !entry.admission.as_ref().is_some_and(Admission::current)
        {
            return Err(CsvError::StaleReview);
        }
        entry.review.clone().ok_or(CsvError::StaleReview)
    }
    pub fn get(
        &self,
        manager: &TransferManager,
        id: CsvTransferAttemptId,
    ) -> Result<CsvTransferObservation, CsvError> {
        let mut state = self.state.lock().unwrap();
        self.prune(&mut state, manager);
        state
            .jobs
            .get(&id)
            .map(|e| e.observation.clone())
            .ok_or(CsvError::Missing)
    }
    pub fn list(
        &self,
        manager: &TransferManager,
        connection: Option<&str>,
    ) -> Result<CsvTransferList, CsvError> {
        let mut state = self.state.lock().unwrap();
        self.prune(&mut state, manager);
        let mut value = CsvTransferList {
            jobs: state
                .jobs
                .values()
                .filter(|e| connection.is_none_or(|c| c == e.observation.connection_id))
                .map(|e| e.observation.clone())
                .collect(),
            import_change_revision: state.revision,
            execution_reserved_bytes: (MAX_CSV_EXECUTION_POOL_BYTES / MAX_CSV_EXECUTION_BYTES
                - self.executions.available_permits())
                * MAX_CSV_EXECUTION_BYTES,
        };
        value.jobs.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then_with(|| a.attempt_id.to_string().cmp(&b.attempt_id.to_string()))
        });
        value.checked_heap_bytes().ok_or(CsvError::Limit)?;
        Ok(value)
    }
    pub fn cancel(
        &self,
        manager: &TransferManager,
        id: CsvTransferAttemptId,
    ) -> Result<CsvTransferObservation, CsvError> {
        let mut state = self.state.lock().unwrap();
        let entry = state.jobs.get_mut(&id).ok_or(CsvError::Missing)?;
        if entry.observation.phase.terminal() {
            return Ok(entry.observation.clone());
        }
        if let Some(job) = &entry.job_id {
            let row = manager.cancel(job).map_err(prepare::error)?;
            if row.phase != crate::postgres::transfer::protocol::Phase::Cancelling {
                return Ok(entry.observation.clone());
            }
        }
        entry.cancel.send_replace(true);
        if entry.job_id.is_none() && entry.execution.is_none() {
            // Registration precedes automatic dispatch. If cancellation wins
            // before any worker is admitted, this call owns its terminal result.
            entry.observation.phase = CsvTransferPhase::Cancelled;
            entry.observation.cleanup = CsvCleanup::Complete;
            entry.observation.finished_at = Some(chrono::Utc::now().to_rfc3339());
            entry.finished = Some(Instant::now());
            if entry.source.is_none() {
                entry.admission.take();
            }
            entry.review.take();
            manager.release_review(&entry.token);
            self.cleanup_job(entry, id);
        } else {
            entry.observation.phase = CsvTransferPhase::Cancelling;
        }
        Ok(entry.observation.clone())
    }
    pub fn release(
        &self,
        manager: &TransferManager,
        id: CsvTransferAttemptId,
    ) -> Result<(), CsvError> {
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.jobs.get(&id) {
            if entry.observation.cleanup != CsvCleanup::Complete {
                return Err(CsvError::Active);
            }
            if let Some(job) = &entry.job_id {
                manager.release(job).map_err(prepare::error)?;
            }
        }
        state.jobs.remove(&id);
        Ok(())
    }
    pub(in crate::backend) fn import_in_progress(&self, connection: &str) -> bool {
        self.state.lock().unwrap().jobs.values().any(|e| {
            e.import_reserved
                && e.observation.connection_id == connection
                && e.observation.cleanup != CsvCleanup::Complete
        })
    }
    pub fn close(&self, manager: &TransferManager) {
        let (inspections, jobs) = {
            let s = self.state.lock().unwrap();
            (
                s.inspections.keys().copied().collect::<Vec<_>>(),
                s.jobs.keys().copied().collect::<Vec<_>>(),
            )
        };
        for id in inspections {
            let _ = self.cancel_inspection(manager, id);
        }
        for id in jobs {
            let _ = self.cancel(manager, id);
        }
    }
    pub async fn drain_until(&self, deadline: tokio::time::Instant) -> Result<(), String> {
        self.owner.drain_until(deadline).await.map_err(|_| {
            "CSV resource owners did not join before the shared deadline".to_owned()
        })?;
        let state = self.state.lock().unwrap();
        if state
            .inspections
            .values()
            .any(|e| e.observation.cleanup != CsvCleanup::Complete)
            || state
                .jobs
                .values()
                .any(|e| e.observation.cleanup != CsvCleanup::Complete)
        {
            return Err("CSV resource cleanup was not established".into());
        }
        Ok(())
    }
}

/// Validate and consume under one lock. Caller-supplied IDs may be reused only
/// after release, but an older opaque handle must never consume the replacement.
pub(super) fn take_inspection(
    state: &mut State,
    inspection: &CsvInspection,
) -> Result<InspectionEntry, CsvError> {
    let current = state
        .inspections
        .get(&inspection.id)
        .ok_or(CsvError::InspectionExpired)?;
    if current.observation.phase != CsvInspectionPhase::Ready
        || current.created.elapsed() >= Duration::from_secs(CSV_INSPECTION_TTL_SECONDS)
    {
        return Err(CsvError::InspectionExpired);
    }
    if current.revision != inspection.revision
        || current
            .ready
            .as_ref()
            .is_none_or(|ready| !Arc::ptr_eq(ready, &inspection.ready))
    {
        return Err(CsvError::StaleReview);
    }
    state
        .inspections
        .remove(&inspection.id)
        .ok_or(CsvError::InspectionExpired)
}
