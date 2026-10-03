//! Owned table workers with independent stop/cancel signals and bounded delivery.
use dbunk_lib::backend::schema_map::{
    SchemaMapRequest, SchemaMapSnapshot,
    preferences::{
        SchemaMapPreferenceScope, SchemaMapPreferences, SchemaMapPreferencesCapture,
        SchemaMapPreferencesError, SchemaMapPreferencesRevision,
    },
};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use dbunk_lib::backend::admin::{
    AdminCapture, AdminControlConfirmation, AdminControlError, AdminControlReview,
    AdminControlSubmission,
};
use dbunk_lib::backend::data::{
    AnalyzeResultSetPayload, AnalyzeResultSetResult, BrowseExactCountResult,
    BrowseTableDataPayload, BrowseTableResult, CountTableBrowseRowsPayload, DataCloseOutcome,
    DataDocument, DataError, MutationConfirmation, MutationPlan, MutationReview,
    MutationSubmission, ResultMutationError, TableBrowseError, TableGridPrefs, VirtualKey,
};
use dbunk_lib::backend::maintenance::{
    MaintenanceConfirmation, MaintenanceError, MaintenanceIntent, MaintenanceReview,
    MaintenanceSubmission,
};
use dbunk_lib::backend::schema_ddl::{
    CreateSchemaConfirmation, CreateSchemaError, CreateSchemaIntent, CreateSchemaReview,
    CreateSchemaSubmission,
};
use dbunk_lib::backend::{Backend, QuerySessionError};
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::browse_preferences::PreferencePatch;
use crate::data_model::RequestTicket;
use crate::mailbox::{ByteBudget, BytePermit, WORKSPACE_QUEUE_BYTES};
use crate::results::encoded_size;

#[path = "table_runtime_ddl.rs"]
mod table_ddl;
use dbunk_lib::backend::table_ddl::{
    TableDdlConfirmation, TableDdlError, TableDdlIntent, TableDdlRequest, TableDdlReview,
    TableDdlSubmission, TableDdlTarget,
};

const DOCUMENT_LIMIT: usize = 16;
const RESULT_CAPACITY: usize = 4;
const LOCAL_CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone, Debug)]
pub enum DeliveryFailure {
    Full,
    Oversize,
    ConsumerClosed,
}
#[derive(Clone, Debug)]
pub enum TableError {
    Backend(Arc<DataError>),
    Delivery(DeliveryFailure),
    Worker(String),
}
impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(error) => write!(f, "{error:?}"),
            Self::Delivery(DeliveryFailure::Full) => f.write_str("Table result queue is full"),
            Self::Delivery(DeliveryFailure::Oversize) => {
                f.write_str("Table result exceeds the 16 MiB delivery budget; use a smaller page")
            }
            Self::Delivery(DeliveryFailure::ConsumerClosed) => f.write_str("Table view is closed"),
            Self::Worker(error) => f.write_str(error),
        }
    }
}
pub type TableCloseResult = Result<DataCloseOutcome, TableError>;
type DataResult<T> = Result<T, Arc<DataError>>;

pub enum TableCommand {
    TableDdlObserve(u64, TableDdlRequest, Option<i16>),
    TableDdlReview(u64, Box<TableDdlTarget>, TableDdlIntent),
    TableDdlApply(u64, Box<TableDdlReview>),
    TableDdlConfirm(u64, Box<TableDdlConfirmation>),
    MaintenanceReview(
        u64,
        dbunk_lib::backend::objects::PgObjectRef,
        MaintenanceIntent,
    ),
    MaintenanceApply(u64, MaintenanceReview),
    MaintenanceConfirm(u64, MaintenanceConfirmation),
    AdminApply(u64, AdminControlReview),
    AdminConfirm(u64, AdminControlConfirmation),
    SchemaReview(u64, CreateSchemaIntent),
    SchemaApply(u64, CreateSchemaReview),
    SchemaConfirm(u64, CreateSchemaConfirmation),
    CompletionColumns(u64, String, String),
    Admin(u64),
    ServerDetails(u64),
    Overview(u64, dbunk_lib::backend::overview::RelationStatsRequest),
    WholeTableExport(u64, dbunk_lib::backend::table_export::TableExportRequest),
    DdlExport(u64, dbunk_lib::backend::ddl_export::DdlExportRequest),
    SchemaMap(u64, SchemaMapRequest),
    MapPreferencesLoad(u64, SchemaMapPreferenceScope),
    MapPreferencesSave(
        u64,
        SchemaMapPreferenceScope,
        SchemaMapPreferencesRevision,
        SchemaMapPreferences,
    ),
    MapPreferencesReset(u64, SchemaMapPreferenceScope, SchemaMapPreferencesRevision),
    Catalog(u64),
    DropImpact(u64, dbunk_lib::backend::objects::PgObjectRef),
    ForeignKeys(u64, String, String),
    Describe(u64, dbunk_lib::backend::objects::PgObjectRef),
    Structure(
        u64,
        dbunk_lib::backend::table_structure::TableStructureRequest,
    ),
    LoadVirtualKey(u64, String, String),
    WriteVirtualKey(u64, String, String, Option<Vec<String>>),
    LoadPreferences(String, String),
    SavePreferences(u64, String, String, PreferencePatch),
    Browse(RequestTicket, BrowseTableDataPayload),
    Count(RequestTicket, CountTableBrowseRowsPayload),
    Analyze(u64, AnalyzeResultSetPayload),
    Review(u64, u64, MutationPlan),
    Apply(u64, MutationReview),
    Confirm(u64, MutationConfirmation),
}
#[allow(clippy::large_enum_variant)]
pub enum TableMessage {
    TableDdlObserved(u64, Result<Box<TableDdlTarget>, Arc<TableDdlError>>),
    TableDdlReviewed(u64, Result<Box<TableDdlReview>, Arc<TableDdlError>>),
    TableDdlApplied(u64, Result<Box<TableDdlSubmission>, Arc<TableDdlError>>),
    WholeTableExport(
        u64,
        DataResult<dbunk_lib::backend::table_export::TableExportCapture>,
    ),
    SchemaMap(u64, DataResult<SchemaMapSnapshot>),
    MapPreferencesLoad(
        u64,
        Result<SchemaMapPreferencesCapture, Arc<SchemaMapPreferencesError>>,
    ),
    MapPreferencesSave(
        u64,
        Result<SchemaMapPreferencesCapture, Arc<SchemaMapPreferencesError>>,
    ),
    MapPreferencesReset(
        u64,
        Result<SchemaMapPreferencesCapture, Arc<SchemaMapPreferencesError>>,
    ),
    MaintenanceReviewed(u64, Result<MaintenanceReview, Arc<MaintenanceError>>),
    MaintenanceApplied(u64, Result<MaintenanceSubmission, Arc<MaintenanceError>>),
    AdminApplied(u64, Result<AdminControlSubmission, Arc<AdminControlError>>),
    SchemaReviewed(u64, Result<CreateSchemaReview, Arc<CreateSchemaError>>),
    SchemaApplied(u64, Result<CreateSchemaSubmission, Arc<CreateSchemaError>>),
    CompletionColumns(
        u64,
        DataResult<dbunk_lib::backend::completion::CompletionColumns>,
    ),
    Admin(u64, DataResult<AdminCapture>),
    ServerDetails(
        u64,
        DataResult<dbunk_lib::backend::server_details::ServerDetailsSnapshot>,
    ),
    Overview(
        u64,
        DataResult<dbunk_lib::backend::overview::OverviewSnapshot>,
    ),
    DdlExport(
        u64,
        DataResult<dbunk_lib::backend::ddl_export::DdlExportArtifact>,
    ),
    DropImpact(
        u64,
        dbunk_lib::backend::objects::PgObjectRef,
        DataResult<dbunk_lib::backend::objects::PgDropImpact>,
    ),
    ForeignKeys(
        u64,
        DataResult<Vec<dbunk_lib::backend::objects::ForeignKey>>,
    ),
    Structure(
        u64,
        DataResult<dbunk_lib::backend::table_structure::TableStructureSnapshot>,
    ),
    Description(
        u64,
        DataResult<dbunk_lib::backend::objects::PgObjectDescription>,
    ),
    Catalog(
        u64,
        DataResult<dbunk_lib::backend::objects::PgObjectCatalog>,
    ),
    Opened,
    VirtualKeyLoaded(u64, DataResult<Option<VirtualKey>>),
    VirtualKeySaved(u64, DataResult<Option<VirtualKey>>),
    Preferences(DataResult<Option<TableGridPrefs>>),
    PreferencesSaved(u64, DataResult<TableGridPrefs>),
    Page(RequestTicket, DataResult<BrowseTableResult>),
    Count(RequestTicket, DataResult<BrowseExactCountResult>),
    Analysis(u64, DataResult<AnalyzeResultSetResult>),
    Reviewed(u64, DataResult<MutationReview>),
    Applied(u64, DataResult<MutationSubmission>),
    Error(TableError),
    Closed(TableCloseResult),
}

/// Owns queued-byte accounting until consumed or dropped by the UI.
pub struct TableEnvelope {
    message: Option<TableMessage>,
    _permit: Option<BytePermit>,
}
impl TableEnvelope {
    pub fn into_message(mut self) -> TableMessage {
        self.message.take().unwrap()
    }
}
impl std::ops::Deref for TableEnvelope {
    type Target = TableMessage;
    fn deref(&self) -> &Self::Target {
        self.message.as_ref().unwrap()
    }
}

struct Status {
    failure: Mutex<Option<TableError>>,
    failure_pending: AtomicBool,
    closed: Mutex<Option<TableCloseResult>>,
    closed_pending: AtomicBool,
    stop: watch::Sender<bool>,
    wake: async_channel::Sender<()>,
}
impl Status {
    fn fail(&self, error: TableError) {
        let mut failure = self.failure.lock().unwrap();
        if failure.is_none() {
            *failure = Some(error);
            self.failure_pending.store(true, Ordering::Release);
        }
        drop(failure);
        self.stop.send_replace(true);
        let _ = self.wake.try_send(());
    }
    fn finish(&self, result: TableCloseResult) {
        let mut closed = self.closed.lock().unwrap();
        if closed.is_none() {
            *closed = Some(result);
            self.closed_pending.store(true, Ordering::Release);
        }
        drop(closed);
        self.stop.send_replace(true);
        let _ = self.wake.try_send(());
    }
}

struct ControlHandle {
    commands: mpsc::Sender<TableCommand>,
    cancellation: watch::Sender<u64>,
    next_cancel: AtomicU64,
    status: Arc<Status>,
}
impl Drop for ControlHandle {
    fn drop(&mut self) {
        self.status.stop.send_replace(true);
    }
}
#[derive(Clone)]
pub struct TableControls(Arc<ControlHandle>);
impl TableControls {
    pub fn send(&self, command: TableCommand) -> Result<(), &'static str> {
        if *self.0.status.stop.borrow() {
            return Err("Table document is closing");
        }
        self.0
            .commands
            .try_send(command)
            .map_err(|_| "A table request is already pending")
    }
    pub fn cancel(&self) {
        let next = self
            .0
            .next_cancel
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.0.cancellation.send_replace(next);
    }
    pub fn stop(&self) {
        self.0.status.stop.send_replace(true);
    }
}

pub struct TableReceiver {
    data: async_channel::Receiver<TableEnvelope>,
    status: Arc<Status>,
}
impl TableReceiver {
    pub fn try_recv(&self) -> Option<TableEnvelope> {
        if let Ok(message) = self.data.try_recv() {
            return Some(message);
        }
        // Failures and close results cannot be hidden behind a full data queue.
        let message = if self.status.failure_pending.swap(false, Ordering::AcqRel) {
            Some(TableMessage::Error(
                self.status.failure.lock().unwrap().clone().unwrap(),
            ))
        } else if self.status.closed_pending.swap(false, Ordering::AcqRel) {
            Some(TableMessage::Closed(
                self.status.closed.lock().unwrap().clone().unwrap(),
            ))
        } else {
            None
        };
        message.map(|message| TableEnvelope {
            message: Some(message),
            _permit: None,
        })
    }
    pub fn has_pending(&self) -> bool {
        !self.data.is_empty()
            || self.status.failure_pending.load(Ordering::Acquire)
            || self.status.closed_pending.load(Ordering::Acquire)
    }
    #[cfg(test)]
    pub fn close_result(&self) -> Option<TableCloseResult> {
        self.status.closed.lock().unwrap().clone()
    }
}
impl Drop for TableReceiver {
    fn drop(&mut self) {
        self.data.close();
        while self.data.try_recv().is_ok() {}
        self.status.stop.send_replace(true);
    }
}

struct Delivery {
    data: async_channel::Sender<TableEnvelope>,
    status: Arc<Status>,
    budget: ByteBudget,
}
impl Delivery {
    fn send(&self, message: TableMessage) -> bool {
        if self.data.is_closed() {
            self.status
                .fail(TableError::Delivery(DeliveryFailure::ConsumerClosed));
            return false;
        }
        let bytes = message_bytes(&message);
        if bytes > WORKSPACE_QUEUE_BYTES {
            self.status
                .fail(TableError::Delivery(DeliveryFailure::Oversize));
            return false;
        }
        let Some(permit) = self.budget.reserve(bytes) else {
            self.status
                .fail(TableError::Delivery(DeliveryFailure::Full));
            return false;
        };
        self.send_reserved(message, permit)
    }
    fn send_reserved(&self, message: TableMessage, permit: BytePermit) -> bool {
        if message_bytes(&message) > permit.bytes() {
            self.status
                .fail(TableError::Delivery(DeliveryFailure::Oversize));
            return false;
        }
        let envelope = TableEnvelope {
            message: Some(message),
            _permit: Some(permit),
        };
        if self.data.try_send(envelope).is_err() {
            self.status
                .fail(TableError::Delivery(DeliveryFailure::Full));
            return false;
        }
        let _ = self.status.wake.try_send(());
        true
    }
}
fn error_bytes(error: &DataError) -> usize {
    match error {
        DataError::Unavailable(error) => encoded_size(error),
        DataError::Document(error) => encoded_size(error),
        DataError::Browse(error) => encoded_size(error),
        DataError::Mutation(error) => encoded_size(error),
        DataError::Storage(error) => encoded_size(error),
        DataError::Catalog(_) => std::mem::size_of::<dbunk_lib::backend::objects::CatalogError>(),
    }
}
fn table_error_bytes(error: &TableError) -> usize {
    match error {
        TableError::Backend(error) => error_bytes(error),
        TableError::Delivery(_) => 0,
        TableError::Worker(error) => encoded_size(error),
    }
}
fn message_bytes(message: &TableMessage) -> usize {
    let payload = match message {
        TableMessage::TableDdlObserved(..)
        | TableMessage::TableDdlReviewed(..)
        | TableMessage::TableDdlApplied(..) => table_ddl::message_bytes(message),
        TableMessage::MaintenanceReviewed(_, Ok(value)) => value.retained_bytes(),
        TableMessage::MaintenanceApplied(
            _,
            Ok(MaintenanceSubmission::NeedsConfirmation(value)),
        ) => value.retained_bytes(),
        TableMessage::MaintenanceApplied(_, Ok(MaintenanceSubmission::Finished(value))) => {
            value.retained_bytes()
        }
        TableMessage::MaintenanceReviewed(_, Err(error))
        | TableMessage::MaintenanceApplied(_, Err(error)) => format!("{error:?}").len(),
        TableMessage::AdminApplied(_, Ok(AdminControlSubmission::NeedsConfirmation(value))) => {
            value.retained_bytes()
        }
        TableMessage::AdminApplied(_, Ok(AdminControlSubmission::Finished(value))) => {
            value.retained_bytes()
        }
        TableMessage::AdminApplied(_, Err(error)) => format!("{error:?}").len(),
        TableMessage::SchemaReviewed(_, Ok(review)) => review.retained_bytes(),
        TableMessage::SchemaApplied(
            _,
            Ok(CreateSchemaSubmission::NeedsConfirmation(confirmation)),
        ) => confirmation.retained_bytes(),
        TableMessage::SchemaApplied(_, Ok(CreateSchemaSubmission::Finished(receipt))) => {
            encoded_size(receipt)
        }
        TableMessage::SchemaReviewed(_, Err(error))
        | TableMessage::SchemaApplied(_, Err(error)) => format!("{error:?}").len(),
        TableMessage::DropImpact(_, reference, result) => {
            encoded_size(reference).saturating_add(match result {
                Ok(impact) => encoded_size(impact),
                Err(error) => format!("{error:?}").len(),
            })
        }
        TableMessage::ForeignKeys(_, Ok(keys)) => encoded_size(keys),
        TableMessage::CompletionColumns(_, Ok(columns)) => encoded_size(columns),
        TableMessage::Admin(_, Ok(snapshot)) => snapshot.retained_bytes(),
        TableMessage::ServerDetails(_, Ok(snapshot)) => encoded_size(snapshot),
        TableMessage::Overview(_, Ok(snapshot)) => snapshot
            .checked_heap_bytes()
            .unwrap_or(usize::MAX)
            .max(encoded_size(snapshot)),
        TableMessage::WholeTableExport(_, Ok(capture)) => capture
            .checked_heap_bytes()
            .unwrap_or(usize::MAX)
            .max(capture.encoded_bytes().unwrap_or(usize::MAX)),
        TableMessage::DdlExport(_, Ok(artifact)) => artifact
            .checked_heap_bytes()
            .unwrap_or(usize::MAX)
            .max(encoded_size(artifact)),
        TableMessage::SchemaMap(_, Ok(snapshot)) => snapshot
            .checked_heap_bytes()
            .unwrap_or(usize::MAX)
            .max(encoded_size(snapshot)),
        TableMessage::MapPreferencesLoad(_, Ok(capture))
        | TableMessage::MapPreferencesSave(_, Ok(capture))
        | TableMessage::MapPreferencesReset(_, Ok(capture)) => {
            capture.checked_heap_bytes().unwrap_or(usize::MAX)
        }
        TableMessage::MapPreferencesLoad(_, Err(error))
        | TableMessage::MapPreferencesSave(_, Err(error))
        | TableMessage::MapPreferencesReset(_, Err(error)) => error.to_string().len(),
        TableMessage::Catalog(_, Ok(catalog)) => encoded_size(catalog),
        TableMessage::Description(_, Ok(description)) => encoded_size(description),
        TableMessage::Structure(_, Ok(snapshot)) => snapshot
            .checked_heap_bytes()
            .unwrap_or(usize::MAX)
            .max(encoded_size(snapshot)),
        TableMessage::VirtualKeyLoaded(_, Ok(key)) | TableMessage::VirtualKeySaved(_, Ok(key)) => {
            encoded_size(key)
        }
        TableMessage::Preferences(Ok(prefs)) => encoded_size(prefs),
        TableMessage::PreferencesSaved(_, Ok(prefs)) => encoded_size(prefs),
        TableMessage::Page(_, Ok(page)) => encoded_size(page),
        TableMessage::Count(_, Ok(count)) => encoded_size(count),
        TableMessage::Analysis(_, Ok(analysis)) => encoded_size(analysis),
        TableMessage::Reviewed(_, Ok(review)) => review.retained_bytes(),
        TableMessage::Applied(_, Ok(MutationSubmission::Applied(applied))) => encoded_size(applied),
        TableMessage::Applied(_, Ok(MutationSubmission::NeedsConfirmation(confirmation))) => {
            confirmation.retained_bytes()
        }
        TableMessage::ForeignKeys(_, Err(error))
        | TableMessage::Structure(_, Err(error))
        | TableMessage::Description(_, Err(error))
        | TableMessage::CompletionColumns(_, Err(error))
        | TableMessage::Admin(_, Err(error))
        | TableMessage::ServerDetails(_, Err(error))
        | TableMessage::Overview(_, Err(error))
        | TableMessage::WholeTableExport(_, Err(error))
        | TableMessage::DdlExport(_, Err(error))
        | TableMessage::SchemaMap(_, Err(error))
        | TableMessage::Catalog(_, Err(error))
        | TableMessage::VirtualKeyLoaded(_, Err(error))
        | TableMessage::VirtualKeySaved(_, Err(error))
        | TableMessage::Page(_, Err(error))
        | TableMessage::Count(_, Err(error))
        | TableMessage::Analysis(_, Err(error))
        | TableMessage::Reviewed(_, Err(error))
        | TableMessage::Applied(_, Err(error)) => error_bytes(error),
        TableMessage::Preferences(Err(error)) | TableMessage::PreferencesSaved(_, Err(error)) => {
            error_bytes(error)
        }
        TableMessage::Error(error) | TableMessage::Closed(Err(error)) => table_error_bytes(error),
        TableMessage::Opened | TableMessage::Closed(Ok(_)) => 0,
    };
    payload.saturating_add(std::mem::size_of::<TableMessage>())
}

#[derive(Clone)]
struct Worker {
    abort: tokio::task::AbortHandle,
    join: Shared<BoxFuture<'static, TableCloseResult>>,
    status: Arc<Status>,
}
impl Worker {
    fn new(task: tokio::task::JoinHandle<TableCloseResult>, status: Arc<Status>) -> Self {
        let abort = task.abort_handle();
        let terminal = status.clone();
        let join = async move {
            let result = task.await.unwrap_or_else(|error| {
                Err(TableError::Worker(if error.is_cancelled() {
                    "Table worker was aborted during shutdown".into()
                } else {
                    "Table worker failed".into()
                }))
            });
            terminal.finish(result.clone());
            result
        }
        .boxed()
        .shared();
        Self {
            abort,
            join,
            status,
        }
    }
}
struct Record {
    connection: String,
    worker: Worker,
}
#[derive(Default)]
struct Registry {
    closing: bool,
    documents: HashMap<String, Record>,
}
pub struct TableRuntime {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    budget: ByteBudget,
    registry: Arc<Mutex<Registry>>,
    preferences: Arc<tokio::sync::Mutex<()>>,
}
impl TableRuntime {
    pub fn new(backend: Backend, runtime: tokio::runtime::Handle, budget: ByteBudget) -> Self {
        Self {
            backend,
            runtime,
            budget,
            registry: Default::default(),
            preferences: Default::default(),
        }
    }
    pub fn open(
        &self,
        window: String,
        tab: String,
        connection: String,
        wake: async_channel::Sender<()>,
    ) -> Result<(TableControls, TableReceiver), &'static str> {
        let mut registry = self.registry.lock().unwrap();
        if registry.closing {
            return Err("Application is closing");
        }
        // Observe completed joins before reclaiming their capacity. Failed
        // cleanup stays registered, preventing reuse of the same visible ID.
        registry
            .documents
            .retain(|_, record| !matches!(record.worker.join.clone().now_or_never(), Some(Ok(_))));
        if registry.documents.contains_key(&tab) {
            return Err("This table document is already open or still closing");
        }
        if registry.documents.len() >= DOCUMENT_LIMIT {
            return Err("Sixteen table documents are open. Close one first.");
        }
        let (controls, receiver, delivery, commands, cancellation, stop) =
            channels(self.budget.clone(), wake);
        let status = delivery.status.clone();
        let task_status = status.clone();
        let backend = self.backend.clone();
        let preferences = self.preferences.clone();
        let records = self.registry.clone();
        let task_tab = tab.clone();
        let task_connection = connection.clone();
        let task = self.runtime.spawn(async move {
            let result = document_worker(
                backend,
                window,
                task_tab.clone(),
                task_connection.clone(),
                delivery,
                commands,
                cancellation,
                stop,
                preferences,
            )
            .await;
            if matches!(result, Ok(DataCloseOutcome::ConnectionDataClosed)) {
                let records = records.lock().unwrap();
                for (id, record) in &records.documents {
                    if id != &task_tab && record.connection == task_connection {
                        record.worker.status.fail(TableError::Backend(Arc::new(
                            DataError::Document(
                                "Connection data was closed during another table's cleanup",
                            ),
                        )));
                    }
                }
            }
            task_status.finish(result.clone());
            result
        });
        registry.documents.insert(
            tab,
            Record {
                connection,
                worker: Worker::new(task, status),
            },
        );
        Ok((controls, receiver))
    }
    pub async fn close(&self, tab: &str) -> TableCloseResult {
        let worker = {
            let records = self.registry.lock().unwrap();
            records.documents.get(tab).map(|record| {
                record.worker.status.stop.send_replace(true);
                record.worker.clone()
            })
        };
        match worker {
            Some(worker) => wait_close(&worker, Instant::now() + LOCAL_CLOSE_TIMEOUT).await,
            None => Ok(DataCloseOutcome::Closed),
        }
    }
    pub async fn close_matching(
        &self,
        connection: Option<&str>,
    ) -> Result<Vec<(String, DataCloseOutcome)>, TableError> {
        let workers = {
            let records = self.registry.lock().unwrap();
            records
                .documents
                .iter()
                .filter(|(_, record)| connection.is_none_or(|id| id == record.connection))
                .map(|(id, record)| {
                    record.worker.status.stop.send_replace(true);
                    (id.clone(), record.worker.clone())
                })
                .collect::<Vec<_>>()
        };
        let deadline = Instant::now() + LOCAL_CLOSE_TIMEOUT;
        futures_util::future::join_all(workers.into_iter().map(|(id, worker)| async move {
            wait_close(&worker, deadline)
                .await
                .map(|outcome| (id, outcome))
        }))
        .await
        .into_iter()
        .collect()
    }
    pub fn stop(&self) {
        let mut records = self.registry.lock().unwrap();
        records.closing = true;
        for record in records.documents.values() {
            record.worker.status.stop.send_replace(true);
        }
    }
    /// Host runs backend shutdown concurrently with this UI-worker barrier.
    /// Its global cleanup owns any request whose UI waiter must be aborted.
    pub async fn join(&self, grace: Instant, final_deadline: Instant) -> Result<(), String> {
        let global_stop = self.registry.lock().unwrap().closing;
        self.stop();
        let workers = self
            .registry
            .lock()
            .unwrap()
            .documents
            .values()
            .map(|record| record.worker.clone())
            .collect::<Vec<_>>();
        let all =
            || futures_util::future::join_all(workers.iter().map(|worker| worker.join.clone()));
        match tokio::time::timeout_at(grace, all()).await {
            Ok(results) => results
                .into_iter()
                .map(|result| global_close_result(result, global_stop))
                .collect::<Result<Vec<_>, _>>()
                .map(|_| ()),
            Err(_) => {
                for worker in &workers {
                    worker.abort.abort();
                }
                tokio::time::timeout_at(final_deadline, all())
                    .await
                    .map_err(|_| {
                        "Table workers did not join before the shutdown deadline".to_owned()
                    })?;
                Err("Table workers required forced shutdown".into())
            }
        }
    }
}

fn global_close_result(result: TableCloseResult, global_stop: bool) -> Result<(), String> {
    match result {
        // Host has stopped UI admission and concurrently awaits backend global
        // cleanup. That cleanup owns the handle when per-tab close is refused.
        Err(TableError::Backend(error))
            if global_stop
                && matches!(
                    error.as_ref(),
                    DataError::Unavailable(QuerySessionError::ConnectionClosing)
                ) =>
        {
            Ok(())
        }
        result => result.map(|_| ()).map_err(|error| error.to_string()),
    }
}

/// A local waiter may time out; the registry still owns the unfinished join.
/// In particular, stopping during open must not discard its eventual handle.
async fn wait_close(worker: &Worker, deadline: Instant) -> TableCloseResult {
    tokio::time::timeout_at(deadline, worker.join.clone())
        .await
        .unwrap_or_else(|_| {
            Err(TableError::Worker(
                "Table cleanup is still pending. Keep this tab open and retry closing it.".into(),
            ))
        })
}

type Channels = (
    TableControls,
    TableReceiver,
    Delivery,
    mpsc::Receiver<TableCommand>,
    watch::Receiver<u64>,
    watch::Receiver<bool>,
);
fn channels(budget: ByteBudget, wake: async_channel::Sender<()>) -> Channels {
    let (commands, command_rx) = mpsc::channel(1);
    let (cancel, cancel_rx) = watch::channel(0);
    let (stop, stop_rx) = watch::channel(false);
    let (data, receive) = async_channel::bounded(RESULT_CAPACITY);
    let status = Arc::new(Status {
        failure: Mutex::new(None),
        failure_pending: AtomicBool::new(false),
        closed: Mutex::new(None),
        closed_pending: AtomicBool::new(false),
        stop,
        wake,
    });
    (
        TableControls(Arc::new(ControlHandle {
            commands,
            cancellation: cancel,
            next_cancel: AtomicU64::new(0),
            status: status.clone(),
        })),
        TableReceiver {
            data: receive,
            status: status.clone(),
        },
        Delivery {
            data,
            status,
            budget,
        },
        command_rx,
        cancel_rx,
        stop_rx,
    )
}

/// Internal backend seam lets ownership tests delay admission without a socket.
trait TableBackend: Send + Sync + 'static {
    type Document: Send + Sync;
    fn open<'a>(
        &'a self,
        window: &'a str,
        tab: &'a str,
        connection: &'a str,
    ) -> BoxFuture<'a, DataResult<Self::Document>>;
    fn request<'a>(
        &'a self,
        document: &'a Self::Document,
        command: TableCommand,
    ) -> BoxFuture<'a, TableMessage>;
    fn cancel<'a>(&'a self, document: &'a Self::Document) -> BoxFuture<'a, DataResult<()>>;
    fn close<'a>(&'a self, document: &'a Self::Document) -> BoxFuture<'a, TableCloseResult>;
}
impl TableBackend for Backend {
    type Document = DataDocument;
    fn open<'a>(
        &'a self,
        window: &'a str,
        tab: &'a str,
        connection: &'a str,
    ) -> BoxFuture<'a, DataResult<DataDocument>> {
        Box::pin(async move {
            self.open_data_document(window, tab, connection)
                .await
                .map_err(Arc::new)
        })
    }
    fn request<'a>(
        &'a self,
        document: &'a DataDocument,
        command: TableCommand,
    ) -> BoxFuture<'a, TableMessage> {
        Box::pin(async move {
            match command {
                command @ (TableCommand::TableDdlObserve(..)
                | TableCommand::TableDdlReview(..)
                | TableCommand::TableDdlApply(..)
                | TableCommand::TableDdlConfirm(..)) => {
                    table_ddl::request(self, document, command).await
                }
                TableCommand::AdminApply(id, review) => TableMessage::AdminApplied(
                    id,
                    if review.belongs_to(document) {
                        self.apply_admin_control(review).await
                    } else {
                        Err(AdminControlError::ForeignDocument)
                    }
                    .map_err(Arc::new),
                ),
                TableCommand::AdminConfirm(id, confirmation) => TableMessage::AdminApplied(
                    id,
                    if confirmation.belongs_to(document) {
                        self.confirm_admin_control(confirmation).await
                    } else {
                        Err(AdminControlError::ForeignDocument)
                    }
                    .map_err(Arc::new),
                ),
                TableCommand::CompletionColumns(id, schema, relation) => {
                    TableMessage::CompletionColumns(
                        id,
                        self.completion_columns(document, schema, relation)
                            .await
                            .map_err(Arc::new),
                    )
                }
                TableCommand::ForeignKeys(id, schema, table) => TableMessage::ForeignKeys(
                    id,
                    self.load_foreign_keys(document, schema, table)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::DropImpact(id, reference) => {
                    let result = self
                        .load_object_drop_impact(document, reference.clone())
                        .await
                        .map_err(Arc::new);
                    TableMessage::DropImpact(id, reference, result)
                }
                TableCommand::Structure(id, request) => TableMessage::Structure(
                    id,
                    self.table_structure(document, request)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::Describe(id, reference) => TableMessage::Description(
                    id,
                    self.describe_object(document, reference)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::Admin(id) => {
                    TableMessage::Admin(id, self.admin_capture(document).await.map_err(Arc::new))
                }
                TableCommand::ServerDetails(id) => TableMessage::ServerDetails(
                    id,
                    self.server_details(document).await.map_err(Arc::new),
                ),
                TableCommand::Overview(id, request) => TableMessage::Overview(
                    id,
                    self.overview(document, request).await.map_err(Arc::new),
                ),
                TableCommand::WholeTableExport(id, request) => TableMessage::WholeTableExport(
                    id,
                    self.capture_table_export(document, request)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::DdlExport(id, request) => TableMessage::DdlExport(
                    id,
                    self.export_ddl(document, request).await.map_err(Arc::new),
                ),
                TableCommand::SchemaMap(id, request) => TableMessage::SchemaMap(
                    id,
                    self.schema_map(document, request).await.map_err(Arc::new),
                ),
                TableCommand::MapPreferencesLoad(id, scope) => TableMessage::MapPreferencesLoad(
                    id,
                    self.load_schema_map_preferences(document, scope)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::MapPreferencesSave(id, scope, expected, value) => {
                    TableMessage::MapPreferencesSave(
                        id,
                        self.save_schema_map_preferences(document, scope, expected, value)
                            .await
                            .map_err(Arc::new),
                    )
                }
                TableCommand::MapPreferencesReset(id, scope, expected) => {
                    TableMessage::MapPreferencesReset(
                        id,
                        self.reset_schema_map_preferences(document, scope, expected)
                            .await
                            .map_err(Arc::new),
                    )
                }
                TableCommand::Catalog(id) => TableMessage::Catalog(
                    id,
                    self.load_object_catalog(document).await.map_err(Arc::new),
                ),
                TableCommand::LoadVirtualKey(id, schema, table) => TableMessage::VirtualKeyLoaded(
                    id,
                    self.load_virtual_key(document, schema, table)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::WriteVirtualKey(id, schema, table, columns) => {
                    let result = async {
                        match columns {
                            Some(columns) => {
                                let key = VirtualKey {
                                    version: 1,
                                    columns,
                                };
                                if key.columns.len() > 64 || encoded_size(&key) > 16 * 1024 {
                                    return Err(DataError::Storage(
                                        "Virtual key exceeds 64 columns or 16 KiB".into(),
                                    ));
                                }
                                self.save_virtual_key(document, schema, table, key.columns.clone())
                                    .await?;
                                Ok(Some(key))
                            }
                            None => {
                                self.clear_virtual_key(document, schema, table).await?;
                                Ok(None)
                            }
                        }
                    }
                    .await
                    .map_err(Arc::new);
                    TableMessage::VirtualKeySaved(id, result)
                }
                TableCommand::LoadPreferences(schema, table) => TableMessage::Preferences(
                    self.load_table_preferences(document, schema, table)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::SavePreferences(id, schema, table, patch) => {
                    let saved = async {
                        let current = self
                            .load_table_preferences(document, schema.clone(), table.clone())
                            .await?;
                        let prefs = patch
                            .apply(current, &dbunk_lib::backend::query_library::utc_timestamp())
                            .map_err(|error| DataError::Storage(error.into()))?;
                        self.save_table_preferences(document, schema, table, prefs.clone())
                            .await?;
                        Ok(prefs)
                    }
                    .await
                    .map_err(Arc::new);
                    TableMessage::PreferencesSaved(id, saved)
                }
                TableCommand::Browse(ticket, payload) => TableMessage::Page(
                    ticket,
                    self.browse_table(document, payload).await.map_err(Arc::new),
                ),
                TableCommand::Count(ticket, payload) => TableMessage::Count(
                    ticket,
                    self.count_table(document, payload).await.map_err(Arc::new),
                ),
                TableCommand::Analyze(id, payload) => TableMessage::Analysis(
                    id,
                    self.analyze_result(document, payload)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::MaintenanceReview(id, reference, intent) => {
                    TableMessage::MaintenanceReviewed(
                        id,
                        self.observe_maintenance_target(document, reference)
                            .await
                            .and_then(|capture| capture.review(intent))
                            .map_err(Arc::new),
                    )
                }
                TableCommand::MaintenanceApply(id, review) => TableMessage::MaintenanceApplied(
                    id,
                    if review.belongs_to(document) {
                        self.apply_maintenance(review).await.map_err(Arc::new)
                    } else {
                        Err(Arc::new(MaintenanceError::ForeignDocument))
                    },
                ),
                TableCommand::MaintenanceConfirm(id, confirmation) => {
                    TableMessage::MaintenanceApplied(
                        id,
                        if confirmation.belongs_to(document) {
                            self.confirm_maintenance(confirmation)
                                .await
                                .map_err(Arc::new)
                        } else {
                            Err(Arc::new(MaintenanceError::ForeignDocument))
                        },
                    )
                }
                TableCommand::SchemaReview(id, intent) => TableMessage::SchemaReviewed(
                    id,
                    self.review_create_schema(document, intent)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::SchemaApply(id, review) => TableMessage::SchemaApplied(
                    id,
                    if review.belongs_to(document) {
                        self.apply_create_schema(review).await.map_err(Arc::new)
                    } else {
                        Err(Arc::new(CreateSchemaError::ForeignDocument))
                    },
                ),
                TableCommand::SchemaConfirm(id, confirmation) => TableMessage::SchemaApplied(
                    id,
                    if confirmation.belongs_to(document) {
                        self.confirm_create_schema(confirmation)
                            .await
                            .map_err(Arc::new)
                    } else {
                        Err(Arc::new(CreateSchemaError::ForeignDocument))
                    },
                ),
                TableCommand::Review(id, analysis_id, plan) => TableMessage::Reviewed(
                    id,
                    self.review_mutations(document, analysis_id, plan)
                        .await
                        .map_err(Arc::new),
                ),
                TableCommand::Apply(id, review) => TableMessage::Applied(
                    id,
                    if review.belongs_to(document) {
                        self.apply_review(review, id).await.map_err(Arc::new)
                    } else {
                        Err(Arc::new(DataError::Document(
                            "Review belongs to another table document",
                        )))
                    },
                ),
                TableCommand::Confirm(id, confirmation) => TableMessage::Applied(
                    id,
                    if confirmation.belongs_to(document) {
                        self.confirm_mutations(confirmation).await.map_err(Arc::new)
                    } else {
                        Err(Arc::new(DataError::Document(
                            "Confirmation belongs to another table document",
                        )))
                    },
                ),
            }
        })
    }
    fn cancel<'a>(&'a self, document: &'a DataDocument) -> BoxFuture<'a, DataResult<()>> {
        Box::pin(async move { self.cancel_data(document).await.map_err(Arc::new) })
    }
    fn close<'a>(&'a self, document: &'a DataDocument) -> BoxFuture<'a, TableCloseResult> {
        Box::pin(async move {
            self.close_data_document(document)
                .await
                .map_err(|error| TableError::Backend(Arc::new(error)))
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn document_worker<B: TableBackend>(
    backend: B,
    window: String,
    tab: String,
    connection: String,
    delivery: Delivery,
    mut commands: mpsc::Receiver<TableCommand>,
    mut cancellation: watch::Receiver<u64>,
    mut stop: watch::Receiver<bool>,
    preferences: Arc<tokio::sync::Mutex<()>>,
) -> TableCloseResult {
    // Never select away an open: Backend may have registered its handle even
    // after the UI stops waiting. Acquire it, then close that exact handle.
    let document = match backend.open(&window, &tab, &connection).await {
        Ok(document) => document,
        Err(error) => {
            delivery.status.fail(TableError::Backend(error));
            return Ok(DataCloseOutcome::Closed);
        }
    };
    if !*stop.borrow() && delivery.send(TableMessage::Opened) {
        'requests: loop {
            let command = tokio::select! {
                biased;
                _ = stopped(&mut stop) => break,
                changed = cancellation.changed() => {
                    if changed.is_err() { break; }
                    discard_pending(&mut commands, &delivery);
                    if !cancel_request(&backend, &document, &delivery, &mut stop).await { break; }
                    continue;
                }
                command = commands.recv() => match command { Some(command) => command, None => break },
            };
            if !run_request(
                &backend,
                &document,
                command,
                &delivery,
                &mut commands,
                &mut cancellation,
                &mut stop,
                &preferences,
            )
            .await
            {
                break 'requests;
            }
        }
    }
    backend.close(&document).await
}
/// Keep ownership of a dequeued command until its backend future is first
/// polled. Cancellation before that boundary must not dispatch it afterwards.
#[allow(clippy::too_many_arguments)]
async fn run_request<B: TableBackend>(
    backend: &B,
    document: &B::Document,
    command: TableCommand,
    delivery: &Delivery,
    commands: &mut mpsc::Receiver<TableCommand>,
    cancellation: &mut watch::Receiver<u64>,
    stop: &mut watch::Receiver<bool>,
    preferences: &tokio::sync::Mutex<()>,
) -> bool {
    if matches!(
        command,
        TableCommand::LoadPreferences(..)
            | TableCommand::SavePreferences(..)
            | TableCommand::LoadVirtualKey(..)
            | TableCommand::WriteVirtualKey(..)
            | TableCommand::MapPreferencesLoad(..)
            | TableCommand::MapPreferencesSave(..)
            | TableCommand::MapPreferencesReset(..)
    ) {
        // One profile-local lane covers every read/merge/commit. Cancel/close
        // can remove a waiter; once dispatched we join its exact SQLite result
        // before releasing the lane or closing the owned backend document.
        let _guard = tokio::select! {
            biased;
            _ = stopped(stop) => return false,
            changed = cancellation.changed() => {
                if changed.is_err() { return false; }
                discard_pending(commands, delivery);
                return delivery.send(cancelled(command));
            },
            guard = preferences.lock() => guard,
        };
        let response = backend.request(document, command).await;
        return !*stop.borrow() && delivery.send(response);
    }
    // The single worker is this queue's only producer. Refuse a full/closed
    // queue before DDL dispatch; close/cancellation during execution can still
    // lose delivery, which the exact durable journal must retain as unknown.
    if matches!(
        command,
        TableCommand::TableDdlObserve(..)
            | TableCommand::TableDdlReview(..)
            | TableCommand::TableDdlApply(..)
            | TableCommand::TableDdlConfirm(..)
    ) && (delivery.data.is_closed() || delivery.data.is_full())
    {
        delivery
            .status
            .fail(TableError::Delivery(DeliveryFailure::Full));
        return false;
    }
    let response_reservation = match &command {
        TableCommand::TableDdlObserve(..)
        | TableCommand::TableDdlReview(..)
        | TableCommand::TableDdlApply(..)
        | TableCommand::TableDdlConfirm(..) => Some(table_ddl::RESPONSE_BYTES),
        TableCommand::AdminApply(..) | TableCommand::AdminConfirm(..) => Some(16 * 1024),
        // Both the 32 KiB confirmation token and 16 KiB receipt fit, including
        // envelope overhead. Reserve before dispatch, never after side effects.
        TableCommand::MaintenanceApply(..) | TableCommand::MaintenanceConfirm(..) => {
            Some(64 * 1024)
        }
        _ => None,
    };
    let permit = if let Some(bytes) = response_reservation {
        match delivery.budget.reserve(bytes) {
            Some(permit) => Some(permit),
            None => return delivery.send(cancelled(command)),
        }
    } else {
        None
    };
    let mut command = Some(command);
    let dispatched = AtomicBool::new(false);
    let mut pending = Box::pin(async {
        dispatched.store(true, Ordering::Relaxed);
        backend.request(document, command.take().unwrap()).await
    });
    loop {
        tokio::select! {
            biased;
            _ = stopped(stop) => return false,
            changed = cancellation.changed() => {
                if changed.is_err() { return false; }
                discard_pending(commands, delivery);
                if !dispatched.load(Ordering::Relaxed) {
                    drop(pending);
                    let response = cancelled(command.take().unwrap());
                    return match permit {
                        Some(permit) => delivery.send_reserved(response, permit),
                        None => delivery.send(response),
                    };
                }
                // Once dispatched, only the backend can settle apply outcome.
                if !cancel_request(backend, document, delivery, stop).await { return false; }
            }
            response = &mut pending => return match permit {
                Some(permit) => delivery.send_reserved(response, permit),
                None => delivery.send(response),
            },
        }
    }
}

async fn stopped(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}
async fn cancel_request<B: TableBackend>(
    backend: &B,
    document: &B::Document,
    delivery: &Delivery,
    stop: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        biased;
        _ = stopped(stop) => false,
        result = backend.cancel(document) => match result {
            Ok(()) => true,
            Err(error) => delivery.send(TableMessage::Error(TableError::Backend(error))),
        },
    }
}
/// A queued operation has not reached the backend. Cancelling it must retain
/// its family and correlation ID, including a consumed single-use review token.
fn cancelled(command: TableCommand) -> TableMessage {
    match command {
        // This mapping is used only before the backend future is polled. Once
        // dispatched, preserve its typed receipt or an unknown transport outcome.
        TableCommand::TableDdlObserve(id, ..) => {
            TableMessage::TableDdlObserved(id, Err(Arc::new(TableDdlError::Unavailable)))
        }
        TableCommand::TableDdlReview(id, ..) => {
            TableMessage::TableDdlReviewed(id, Err(Arc::new(TableDdlError::Unavailable)))
        }
        TableCommand::TableDdlApply(id, ..) | TableCommand::TableDdlConfirm(id, ..) => {
            TableMessage::TableDdlApplied(id, Err(Arc::new(TableDdlError::Unavailable)))
        }
        TableCommand::AdminApply(id, _) | TableCommand::AdminConfirm(id, _) => {
            TableMessage::AdminApplied(id, Err(Arc::new(AdminControlError::Cancelled)))
        }
        TableCommand::CompletionColumns(id, ..) => TableMessage::CompletionColumns(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::ForeignKeys(id, ..) => TableMessage::ForeignKeys(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::DropImpact(id, reference) => TableMessage::DropImpact(
            id,
            reference,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::Structure(id, _) => TableMessage::Structure(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::Describe(id, _) => TableMessage::Description(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::Admin(id) => TableMessage::Admin(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::ServerDetails(id) => TableMessage::ServerDetails(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::Overview(id, _) => TableMessage::Overview(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::WholeTableExport(id, _) => TableMessage::WholeTableExport(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::DdlExport(id, _) => TableMessage::DdlExport(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::SchemaMap(id, _) => TableMessage::SchemaMap(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::MapPreferencesLoad(id, _) => TableMessage::MapPreferencesLoad(
            id,
            Err(Arc::new(SchemaMapPreferencesError::Cancelled)),
        ),
        TableCommand::MapPreferencesSave(id, ..) => TableMessage::MapPreferencesSave(
            id,
            Err(Arc::new(SchemaMapPreferencesError::Cancelled)),
        ),
        TableCommand::MapPreferencesReset(id, ..) => TableMessage::MapPreferencesReset(
            id,
            Err(Arc::new(SchemaMapPreferencesError::Cancelled)),
        ),
        TableCommand::Catalog(id) => TableMessage::Catalog(
            id,
            Err(Arc::new(DataError::Catalog(
                dbunk_lib::backend::objects::CatalogError::Cancelled,
            ))),
        ),
        TableCommand::LoadVirtualKey(id, ..) => TableMessage::VirtualKeyLoaded(
            id,
            Err(Arc::new(DataError::Mutation(
                ResultMutationError::Cancelled,
            ))),
        ),
        TableCommand::WriteVirtualKey(id, ..) => TableMessage::VirtualKeySaved(
            id,
            Err(Arc::new(DataError::Mutation(
                ResultMutationError::Cancelled,
            ))),
        ),
        TableCommand::LoadPreferences(..) => TableMessage::Preferences(Err(Arc::new(
            DataError::Storage("Preference load cancelled".into()),
        ))),
        TableCommand::SavePreferences(id, ..) => TableMessage::PreferencesSaved(
            id,
            Err(Arc::new(DataError::Storage(
                "Preference save cancelled before dispatch".into(),
            ))),
        ),
        TableCommand::Browse(ticket, _) => TableMessage::Page(
            ticket,
            Err(Arc::new(DataError::Browse(TableBrowseError::Cancelled))),
        ),
        TableCommand::Count(ticket, _) => TableMessage::Count(
            ticket,
            Err(Arc::new(DataError::Browse(TableBrowseError::Cancelled))),
        ),
        TableCommand::Analyze(id, _) => TableMessage::Analysis(
            id,
            Err(Arc::new(DataError::Mutation(
                ResultMutationError::Cancelled,
            ))),
        ),
        TableCommand::MaintenanceReview(id, _, _) => {
            TableMessage::MaintenanceReviewed(id, Err(Arc::new(MaintenanceError::Unavailable)))
        }
        TableCommand::MaintenanceApply(id, _) | TableCommand::MaintenanceConfirm(id, _) => {
            TableMessage::MaintenanceApplied(id, Err(Arc::new(MaintenanceError::Unavailable)))
        }
        TableCommand::SchemaReview(id, _) => {
            TableMessage::SchemaReviewed(id, Err(Arc::new(CreateSchemaError::Cancelled)))
        }
        TableCommand::SchemaApply(id, _) | TableCommand::SchemaConfirm(id, _) => {
            TableMessage::SchemaApplied(id, Err(Arc::new(CreateSchemaError::Cancelled)))
        }
        TableCommand::Review(id, _, _) => TableMessage::Reviewed(
            id,
            Err(Arc::new(DataError::Mutation(
                ResultMutationError::Cancelled,
            ))),
        ),
        TableCommand::Apply(id, _) | TableCommand::Confirm(id, _) => TableMessage::Applied(
            id,
            Err(Arc::new(DataError::Mutation(
                ResultMutationError::Cancelled,
            ))),
        ),
    }
}
fn discard_pending(commands: &mut mpsc::Receiver<TableCommand>, delivery: &Delivery) {
    if let Ok(command) = commands.try_recv() {
        delivery.send(cancelled(command));
    }
}

#[cfg(test)]
#[path = "table_runtime_tests.rs"]
mod tests;
