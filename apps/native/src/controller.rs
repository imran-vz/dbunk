//! Owns Tokio work. GPUI sends bounded commands and consumes typed messages.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dbunk_lib::backend::{
    AckPayload, Backend, ExecutePayload, ExecutionPayload, HeartbeatPayload, Layout,
    OpenSessionPayload, QueryEventEnvelope, QuerySessionError, RegisterOwnerPayload,
    TransactionControl,
};
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::mailbox::{self, Failure, Message};
#[path = "table_runtime.rs"]
mod table_runtime;
use table_runtime::TableRuntime;
#[path = "diagnosis_runtime.rs"]
mod diagnosis_runtime;
pub use table_runtime::{
    DdlApplied, DdlObserved, DdlReviewed, TableCommand, TableControls, TableMessage, TableReceiver,
};
#[path = "query_controls.rs"]
mod query_controls;
use query_controls::QueryControls;
pub use query_controls::transaction_allowed;
#[path = "workspace_runtime.rs"]
mod workspace_runtime;

#[path = "query_library_runtime.rs"]
mod query_library_runtime;
use query_library_runtime::LibraryRuntime;
pub use query_library_runtime::{LibraryCommand, LibraryControls, LibraryDelivery, LibraryReply};

#[path = "pg_tool_runtime.rs"]
mod pg_tool_runtime;
use pg_tool_runtime::PgToolRuntime;
pub use pg_tool_runtime::{ToolCommand, ToolControls, ToolDelivery, ToolReply};

#[path = "csv_transfer_runtime.rs"]
mod csv_transfer_runtime;
use csv_transfer_runtime::CsvRuntime;
#[path = "schema_compare_runtime.rs"]
mod schema_compare_runtime;
pub use csv_transfer_runtime::{CsvCommand, CsvControls, CsvDelivery, CsvReply};
use schema_compare_runtime::CompareRuntime;
pub use schema_compare_runtime::{
    CompareCommand, CompareControls, CompareFailure, CompareReceiver, CompareReply,
};

pub const WINDOW: &str = "native-fixture";
const GRACE: Duration = Duration::from_secs(3);
const TOTAL: Duration = Duration::from_secs(5);
/// One document's cleanup budget outside app shutdown. It covers the
/// backend's default 10 s connect deadline plus margin, so a document closed
/// while its connect stalls is joined, not aborted. Shutdown keeps the
/// shared `GRACE`/`TOTAL` budget.
const DOCUMENT_GRACE: Duration = Duration::from_secs(12);
const DOCUMENT_TOTAL: Duration = Duration::from_secs(14);

type WorkerResult = Result<(), String>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum WorkerError {
    Cancelled,
    Failed(String),
}
impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Native worker was cancelled"),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

pub enum Command {
    Run(ExecutePayload),
    Confirm(String),
    Transaction(TransactionControl),
    Cancel(ExecutionPayload),
    Layout(Layout),
}

#[derive(Clone)]
pub struct Controls {
    commands: mpsc::Sender<Command>,
    transactions: mpsc::Sender<TransactionControl>,
    discarded_review: watch::Sender<Option<String>>,
    cancellation: Option<watch::Sender<Option<ExecutionPayload>>>,
    layout: watch::Sender<Option<Layout>>,
    acknowledgements: watch::Sender<Option<AckPayload>>,
    focused: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    closing: Arc<AtomicBool>,
    submission: Arc<Mutex<()>>,
}
impl Controls {
    pub fn send(&self, command: Command) -> Result<(), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        if let Command::Transaction(control) = command {
            return self
                .transactions
                .try_send(control)
                .map_err(|_| "Transaction control is already pending");
        }
        if let Command::Layout(layout) = command {
            // The preference belongs to the profile, including while disconnected.
            return self
                .layout
                .send(Some(layout))
                .map_err(|_| "Layout storage is unavailable");
        }
        if let (Command::Cancel(payload), Some(cancellation)) = (&command, &self.cancellation) {
            cancellation.send_replace(Some(payload.clone()));
            return Ok(());
        }
        self.commands
            .try_send(command)
            .map_err(|_| "Native command queue is unavailable")
    }
    pub fn discard_confirmation(&self, execution: String) {
        self.discarded_review.send_replace(Some(execution));
    }
    pub fn acknowledge(&self, payload: AckPayload) {
        self.acknowledgements.send_replace(Some(payload));
    }
    pub fn focus(&self, focused: bool) {
        self.focused.send_replace(focused);
    }
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

/// Shared joins retain ownership when a reconnect waiter itself is cancelled.
/// Host keeps every unfinished worker until it has been joined or abort-joined.
#[derive(Clone)]
struct Worker {
    abort: tokio::task::AbortHandle,
    join: Shared<BoxFuture<'static, Result<(), WorkerError>>>,
}
impl Worker {
    fn new(task: tokio::task::JoinHandle<WorkerResult>) -> Self {
        let abort = task.abort_handle();
        let join = async move {
            match task.await {
                Ok(result) => result.map_err(WorkerError::Failed),
                Err(error) if error.is_cancelled() => Err(WorkerError::Cancelled),
                Err(error) => Err(WorkerError::Failed(format!(
                    "Native worker failed: {error}"
                ))),
            }
        }
        .boxed()
        .shared();
        Self { abort, join }
    }
}
struct Active {
    controls: Controls,
    worker: Worker,
    opening: Arc<AtomicBool>,
}
#[derive(Default)]
struct Sessions {
    active: Option<Active>,
    workers: Vec<Worker>,
    failure: Option<String>,
}
pub struct Host {
    pub files: crate::file_runtime::FileRuntime,
    pub backend: Backend,
    pub runtime: tokio::runtime::Handle,
    sessions: Mutex<Sessions>,
    closing: Arc<AtomicBool>,
    submission: Arc<Mutex<()>>,
    layout: watch::Sender<Option<Layout>>,
    layout_stop: watch::Sender<bool>,
    layout_worker: Worker,
    shutdown: tokio::sync::Mutex<Option<WorkerResult>>,
    workspace: Option<workspace_runtime::WorkspaceRuntime>,
    tables: Option<TableRuntime>,
    library: Option<LibraryRuntime>,
    pg_tools: Option<PgToolRuntime>,
    csv_transfers: Option<CsvRuntime>,
    comparisons: Option<CompareRuntime>,
}
impl Host {
    pub fn new(backend: Backend, runtime: tokio::runtime::Handle) -> Arc<Self> {
        let (layout, preferences) = watch::channel(None);
        let (layout_stop, stop) = watch::channel(false);
        let storage = backend.clone();
        let layout_worker = Worker::new(
            runtime.spawn(async move { persist_layout(storage, preferences, stop).await }),
        );
        Arc::new(Self {
            files: crate::file_runtime::FileRuntime::default(),
            backend,
            runtime,
            sessions: Mutex::new(Sessions::default()),
            closing: Arc::new(AtomicBool::new(false)),
            submission: Arc::new(Mutex::new(())),
            layout,
            layout_stop,
            layout_worker,
            shutdown: tokio::sync::Mutex::new(None),
            workspace: None,
            tables: None,
            library: None,
            pg_tools: None,
            csv_transfers: None,
            comparisons: None,
        })
    }

    /// Stage04 owns one window registration and heartbeat for all documents.
    pub fn new_workspace(
        backend: Backend,
        runtime: tokio::runtime::Handle,
        owner: String,
    ) -> Arc<Self> {
        let mut host = Self::new(backend, runtime);
        let value = Arc::get_mut(&mut host).expect("new host has one owner");
        value.workspace = Some(workspace_runtime::WorkspaceRuntime::new(
            value.backend.clone(),
            value.runtime.clone(),
            owner,
            value.layout.clone(),
            value.closing.clone(),
            value.submission.clone(),
        ));
        value.tables = Some(TableRuntime::new(
            value.backend.clone(),
            value.runtime.clone(),
            value
                .workspace
                .as_ref()
                .expect("workspace initialized")
                .queue_budget(),
        ));
        value.library = Some(LibraryRuntime::new(
            value.backend.clone(),
            value.runtime.clone(),
            value.workspace.as_ref().unwrap().queue_budget(),
        ));
        value.pg_tools = Some(PgToolRuntime::new(
            value.backend.clone(),
            value.runtime.clone(),
            value.workspace.as_ref().unwrap().queue_budget(),
        ));
        value.csv_transfers = Some(CsvRuntime::new(
            value.backend.clone(),
            value.runtime.clone(),
            value.workspace.as_ref().unwrap().queue_budget(),
        ));
        value.comparisons = Some(CompareRuntime::new(
            value.backend.clone(),
            value.runtime.clone(),
            value.workspace.as_ref().unwrap().queue_budget(),
            value.owner().expect("workspace initialized").to_owned(),
        ));
        host
    }

    pub fn open_comparisons(
        &self,
        wake: async_channel::Sender<()>,
    ) -> Result<(CompareControls, CompareReceiver), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        self.comparisons
            .as_ref()
            .ok_or("A workspace host is required")?
            .open(None, wake)
    }
    pub fn open_comparison_reader(
        &self,
        tab: String,
        wake: async_channel::Sender<()>,
    ) -> Result<(CompareControls, CompareReceiver), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        self.comparisons
            .as_ref()
            .ok_or("A workspace host is required")?
            .open(Some(tab), wake)
    }

    pub fn open_csv_transfers(
        &self,
        wake: async_channel::Sender<()>,
    ) -> Result<(CsvControls, async_channel::Receiver<CsvDelivery>), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        self.csv_transfers
            .as_ref()
            .ok_or("A workspace host is required")?
            .open(wake)
    }

    pub fn open_pg_tools(
        &self,
        wake: async_channel::Sender<()>,
    ) -> Result<(ToolControls, async_channel::Receiver<ToolDelivery>), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        self.pg_tools
            .as_ref()
            .ok_or("A workspace host is required")?
            .open(wake)
    }

    pub fn open_library(
        &self,
        id: String,
        wake: async_channel::Sender<()>,
    ) -> Result<(LibraryControls, async_channel::Receiver<LibraryDelivery>), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        self.library
            .as_ref()
            .ok_or("A workspace host is required")?
            .open(id, wake)
    }

    pub fn owner(&self) -> Option<&str> {
        self.workspace.as_ref().map(|workspace| workspace.owner())
    }

    pub fn mailbox(&self) -> (mailbox::Sender, mailbox::Receiver) {
        match &self.workspace {
            Some(workspace) => workspace.mailbox(),
            None => mailbox::channel(mailbox::QUEUE_CAPACITY, mailbox::QUEUE_BYTES),
        }
    }

    pub fn connect_document(
        &self,
        tab: String,
        connection: String,
        session: String,
        events: mailbox::Sender,
    ) -> Result<Controls, &'static str> {
        self.workspace
            .as_ref()
            .ok_or("A workspace host is required")?
            .connect(tab, connection, session, events)
    }

    pub fn open_table_document(
        &self,
        id: String,
        connection: String,
        wake: async_channel::Sender<()>,
    ) -> Result<(TableControls, TableReceiver), &'static str> {
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        self.tables
            .as_ref()
            .ok_or("A workspace host is required")?
            .open(WINDOW.into(), id, connection, wake)
    }

    /// Saved connections the user explicitly connected in some document.
    pub fn connected(&self) -> std::collections::BTreeSet<String> {
        let mut connected = std::collections::BTreeSet::new();
        let mut visit = |id: &str| {
            connected.insert(id.to_owned());
        };
        if let Some(workspace) = &self.workspace {
            workspace.visit_connections(&mut visit);
        }
        if let Some(tables) = &self.tables {
            tables.visit_connections(&mut visit);
        }
        connected
    }

    pub async fn close_document(&self, tab: &str) -> WorkerResult {
        if let Some(comparisons) = &self.comparisons {
            comparisons.close(tab).await?;
        }
        self.disconnect_document(tab).await
    }

    pub async fn disconnect_document(&self, tab: &str) -> WorkerResult {
        if let Some(library) = &self.library {
            library.close(tab).await?;
        }
        let workspace = self
            .workspace
            .as_ref()
            .ok_or("A workspace host is required")?;
        let (query, table) = tokio::join!(workspace.disconnect(tab), async {
            match &self.tables {
                Some(tables) => tables
                    .close(tab)
                    .await
                    .map_err(|error| format!("Table cleanup failed: {error}")),
                None => Ok(dbunk_lib::backend::data::DataCloseOutcome::Closed),
            }
        });
        query?;
        match table? {
            dbunk_lib::backend::data::DataCloseOutcome::Closed => Ok(()),
            dbunk_lib::backend::data::DataCloseOutcome::ConnectionDataClosed => {
                Err("All table documents on this connection were closed during cleanup".into())
            }
        }
    }

    pub async fn disconnect_connection_documents(&self, connection: &str) -> WorkerResult {
        self.disconnect_matching(Some(connection)).await
    }

    pub async fn disconnect_all_documents(&self) -> WorkerResult {
        self.disconnect_matching(None).await
    }

    async fn disconnect_matching(&self, connection: Option<&str>) -> WorkerResult {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or("A workspace host is required")?;
        let (query, table) = tokio::join!(workspace.disconnect_matching(connection), async {
            match &self.tables {
                Some(tables) => tables
                    .close_matching(connection)
                    .await
                    .map_err(|error| format!("Table cleanup failed: {error}")),
                None => Ok(Vec::new()),
            }
        });
        query?;
        if table?.iter().any(|(_, outcome)| {
            *outcome == dbunk_lib::backend::data::DataCloseOutcome::ConnectionDataClosed
        }) {
            return Err("All table documents on a connection were closed during cleanup".into());
        }
        Ok(())
    }

    /// A new owner is admitted only after the previous owner's full cleanup
    /// succeeds. A cleanup failure stays latched until this process exits.
    pub fn connect(
        self: &Arc<Self>,
        owner: String,
        session: String,
        events: mailbox::Sender,
    ) -> Result<Controls, &'static str> {
        if self.workspace.is_some() {
            return Err("Use document connections in a workspace");
        }
        let mut sessions = self.sessions.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        // Poll only completed joins, retaining all pending ones. This bounds
        // history across repeated reconnects and also observes worker panics.
        let mut failure = None;
        sessions
            .workers
            .retain(|worker| match worker.join.clone().now_or_never() {
                Some(Err(error)) => {
                    failure = Some(error.to_string());
                    false
                }
                Some(Ok(())) => false,
                None => true,
            });
        if sessions.failure.is_none() {
            sessions.failure = failure;
        }
        if sessions.failure.is_some() {
            return Err("Previous connection cleanup failed. Close this fixture window.");
        }
        if sessions
            .active
            .as_ref()
            .is_some_and(|active| active.opening.load(Ordering::Acquire))
        {
            return Err("Connection is already pending");
        }
        let (commands, command_rx) = mpsc::channel(16);
        let (transactions, transaction_rx) = mpsc::channel(1);
        let (discarded_review, review_rx) = watch::channel(None);
        let (acknowledgements, ack_rx) = watch::channel(None);
        let (focused, focus_rx) = watch::channel(true);
        let (stop, stop_rx) = watch::channel(false);
        let controls = Controls {
            commands,
            transactions,
            discarded_review,
            cancellation: None,
            acknowledgements,
            focused,
            stop,
            layout: self.layout.clone(),
            closing: self.closing.clone(),
            submission: self.submission.clone(),
        };
        let previous = sessions.active.take();
        if let Some(previous) = &previous {
            previous.controls.stop();
        }
        let host = self.clone();
        let opening = Arc::new(AtomicBool::new(true));
        let worker_opening = opening.clone();
        let worker = Worker::new(self.runtime.spawn(async move {
            let result = async {
                if let Some(previous) = previous {
                    if host.closing.load(Ordering::Acquire) || *stop_rx.borrow() { return Ok(()); }
                    let started = Instant::now();
                    let mut waiting_stop = stop_rx.clone();
                    tokio::select! {
                        biased;
                        _ = waiting_stop.changed() => return Ok(()),
                        result = join_worker(&previous.worker, started + GRACE, started + TOTAL, false) => result?,
                    }
                }
                if host.closing.load(Ordering::Acquire) || *stop_rx.borrow() {
                    return Ok(());
                }
                let result = session_worker(
                    &host.backend,
                    owner,
                    session,
                    events.clone(),
                    command_rx,
                    transaction_rx,
                    review_rx,
                    ack_rx,
                    focus_rx,
                    stop_rx,
                    worker_opening.clone(),
                )
                .await;
                if let Err(error) = result {
                    events.fail(Failure::Backend(error));
                }
                if !host.closing.load(Ordering::Acquire)
                    && let Err(error) = host.backend.retire_window(WINDOW).await
                    && !(host.closing.load(Ordering::Acquire) && matches!(error, QuerySessionError::ConnectionClosing))
                {
                    return Err(format!("Connection cleanup failed: {error:?}"));
                }
                Ok(())
            }
            .await;
            worker_opening.store(false, Ordering::Release);
            if let Err(error) = &result {
                events.fail(Failure::Backend(error.clone()));
                host.sessions
                    .lock()
                    .unwrap()
                    .failure
                    .get_or_insert_with(|| error.clone());
            }
            result
        }));
        sessions.workers.push(worker.clone());
        sessions.active = Some(Active {
            controls: controls.clone(),
            worker,
            opening,
        });
        Ok(controls)
    }

    /// The host and backend share one three-second grace / five-second total
    /// deadline. Results are latched for the window-close and app-quit callers.
    pub async fn shutdown(&self) -> WorkerResult {
        let started = Instant::now();
        let (workers, prior_failure) = {
            let mut sessions = self.sessions.lock().unwrap();
            // Same locks as connect/command submission: no worker or preference
            // can be admitted after this snapshot and the final layout flush.
            let _submission = self.submission.lock().unwrap();
            self.closing.store(true, Ordering::Release);
            self.files.stop();
            if let Some(workspace) = &self.workspace {
                workspace.stop();
            }
            if let Some(tables) = &self.tables {
                tables.stop();
            }
            if let Some(library) = &self.library {
                library.stop();
            }
            if let Some(tools) = &self.pg_tools {
                tools.stop();
            }
            if let Some(comparisons) = &self.comparisons {
                comparisons.stop();
            }
            if let Some(csv) = &self.csv_transfers {
                csv.stop();
            }
            if let Some(active) = sessions.active.take() {
                active.controls.stop();
            }
            (sessions.workers.clone(), sessions.failure.clone())
        };
        let mut shutdown = self.shutdown.lock().await;
        if let Some(result) = &*shutdown {
            return result.clone();
        }
        self.layout_stop.send_replace(true);
        let grace = started + GRACE;
        let final_deadline = started + TOTAL;
        // Flush the last preference before fencing backend storage. Its work
        // uses the same deadlines, so slow storage cannot extend shutdown.
        let layout_result = join_worker(&self.layout_worker, grace, final_deadline, false).await;
        // Local storage acknowledgements finish before Backend fences its call
        // lane. Sessions/sockets remain owned until the backend shutdown below.
        let (workspace_result, library_result, tools_result, csv_result, comparisons_result) = tokio::join!(
            async {
                match &self.workspace {
                    Some(workspace) => workspace.join(grace, final_deadline).await,
                    None => Ok(()),
                }
            },
            async {
                match &self.library {
                    Some(library) => library.join(grace, final_deadline).await,
                    None => Ok(()),
                }
            },
            async {
                match &self.pg_tools {
                    Some(tools) => tools.join(grace, final_deadline).await,
                    None => Ok(()),
                }
            },
            async {
                match &self.csv_transfers {
                    Some(csv) => csv.join(grace, final_deadline).await,
                    None => Ok(()),
                }
            },
            async {
                match &self.comparisons {
                    Some(comparisons) => comparisons.join(grace, final_deadline).await,
                    None => Ok(()),
                }
            },
        );
        let (worker_result, backend_result, table_result) = tokio::join!(
            async {
                let results = futures_util::future::join_all(
                    workers
                        .iter()
                        .map(|worker| join_worker(worker, grace, final_deadline, true)),
                )
                .await;
                results
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()
                    .map(|_| ())
            },
            self.backend.shutdown_with_deadlines(grace, final_deadline),
            async {
                match &self.tables {
                    Some(tables) => tables.join(grace, final_deadline).await,
                    None => Ok(()),
                }
            },
        );
        let file_result = self.files.join(final_deadline).await;
        let result = prior_failure
            .map_or(Ok(()), Err)
            .and(layout_result)
            .and(worker_result)
            .and(backend_result)
            .and(workspace_result)
            .and(table_result)
            .and(library_result)
            .and(tools_result)
            .and(csv_result)
            .and(comparisons_result)
            .and(file_result);
        *shutdown = Some(result.clone());
        result
    }
}

async fn join_worker(
    worker: &Worker,
    grace: Instant,
    final_deadline: Instant,
    allow_abort: bool,
) -> WorkerResult {
    match tokio::time::timeout_at(grace, worker.join.clone()).await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => {
            worker.abort.abort();
            let result = tokio::time::timeout_at(final_deadline, worker.join.clone())
                .await
                .map_err(|_| {
                    "Native worker did not terminate within the shutdown budget".to_string()
                })?;
            match result {
                Ok(()) | Err(WorkerError::Cancelled) if allow_abort => Ok(()),
                Err(error @ WorkerError::Failed(_)) => Err(error.to_string()),
                _ => Err("Previous connection cleanup exceeded its deadline".into()),
            }
        }
    }
}

async fn persist_layout(
    backend: Backend,
    mut layouts: watch::Receiver<Option<Layout>>,
    mut stop: watch::Receiver<bool>,
) -> WorkerResult {
    loop {
        // Persist the latest value on normal changes and on the final stop.
        let layout = *layouts.borrow_and_update();
        if let Some(layout) = layout {
            backend.set_layout(layout).await?;
        }
        if *stop.borrow() {
            return Ok(());
        }
        tokio::select! {
            changed = layouts.changed() => if changed.is_err() { return Ok(()); },
            changed = stop.changed() => if changed.is_err() { return Ok(()); },
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn session_worker(
    backend: &Backend,
    owner: String,
    session: String,
    events: mailbox::Sender,
    mut commands: mpsc::Receiver<Command>,
    mut transactions: mpsc::Receiver<TransactionControl>,
    mut discarded_review: watch::Receiver<Option<String>>,
    mut acknowledgements: watch::Receiver<Option<AckPayload>>,
    mut focused: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
    opening: Arc<AtomicBool>,
) -> WorkerResult {
    if *stop.borrow() {
        return Ok(());
    }
    let open = async {
        backend
            .register_owner(
                WINDOW,
                RegisterOwnerPayload {
                    owner_id: owner.clone(),
                },
            )
            .await?;
        let sink = events.clone();
        let opening = backend.open(
            WINDOW,
            OpenSessionPayload {
                owner_id: owner.clone(),
                session_id: session.clone(),
                tab_id: "query".into(),
                connection_id: backend.fixture().id,
            },
            Arc::new(move |event: QueryEventEnvelope| sink.send(Message::Event(event))),
        );
        #[cfg(feature = "fixture-verification")]
        let opening = crate::verification::open(opening);
        opening.await
    };
    tokio::select! {
        biased;
        _ = stop.changed() => return Ok(()),
        result = open => { result.map_err(error_message)?; }
    }
    if *stop.borrow() {
        return Ok(());
    }
    opening.store(false, Ordering::Release);
    events
        .send(Message::Ready)
        .map_err(|_| "Connection event delivery failed".to_string())?;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if events.is_failed() || *stop.borrow() {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            changed = discarded_review.changed() => {
                if changed.is_err() { return Ok(()); }
                discarded_review.borrow_and_update();
            }
            changed = acknowledgements.changed() => {
                if changed.is_err() { return Ok(()); }
                let payload = acknowledgements.borrow_and_update().clone();
                if let Some(payload) = payload {
                    let execution = payload.execution_id.clone();
                    let sequence = payload.ack_through_sequence;
                    backend.ack(WINDOW, payload).await.map_err(error_message)?;
                    events.send(Message::Acked { execution, sequence }).map_err(|_| "ACK reply delivery failed".to_string())?;
                }
            }
            changed = focused.changed() => {
                if changed.is_err() { return Ok(()); }
                let focused = *focused.borrow_and_update();
                backend.set_focus(WINDOW, focused).await.map_err(error_message)?;
            }
            control = transactions.recv() => {
                let Some(control) = control else { return Ok(()); };
                let result = backend.control_transaction(WINDOW, &session, control).await;
                events.send(Message::Transaction { session: session.clone(), result }).map_err(|_| "Transaction reply delivery failed")?;
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                match command {
                    Command::Run(payload) => {
                        let execution = payload.execution_id.clone();
                        // The stage03 fixture keeps its original refusal-only
                        // behavior; service-issued review belongs to stage04.
                        if let Err(error) = backend.execute(WINDOW, payload).await {
                            events.send(Message::Rejected { execution, message: error_message(error) }).map_err(|_| "Query refusal delivery failed")?;
                        }
                    }
                    Command::Confirm(execution) => {
                        events.send(Message::Rejected { execution, message: "Query review requires a workspace".into() }).map_err(|_| "Query refusal delivery failed")?;
                    }
                    Command::Cancel(payload) => {
                        let execution = payload.execution_id.clone();
                        let result = backend.cancel(WINDOW, payload).await;
                        #[cfg(feature = "fixture-verification")]
                        if crate::verification::enabled() && result.is_ok() {
                            eprintln!("VERIFY cancel-ok {execution}");
                        }
                        if let Err(error) = result {
                            let _ = events.send(Message::CancelFailed { execution, message: error_message(error) });
                        }
                    }
                    Command::Layout(_) | Command::Transaction(_) => unreachable!("separate control channels"),
                }
            }
            _ = heartbeat.tick() => {
                if !backend.session_alive(WINDOW, &session).await.map_err(error_message)? { return Err("Connection lost. Reconnect to run again.".into()); }
                if *focused.borrow() {
                    backend.heartbeat(WINDOW, HeartbeatPayload { owner_id: owner.clone(), session_ids: vec![session.clone()] }).await.map_err(error_message)?;
                }
            }
        }
    }
}

pub fn error_message(error: QuerySessionError) -> String {
    match error {
        QuerySessionError::Database {
            message,
            code,
            position,
            ..
        } => format!(
            "{}{}{}",
            code.map_or(String::new(), |code| format!("{code}: ")),
            message,
            position.map_or(String::new(), |position| format!(" (position {position})"))
        ),
        QuerySessionError::PolicyBlocked { reason } => format!("Not run: {reason}"),
        QuerySessionError::PolicyNeedsConfirmation { .. } => {
            "Not run: Safe Mode requires confirmation".into()
        }
        error => format!("{error:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_worker() -> (
        Worker,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (started, ready) = tokio::sync::oneshot::channel();
        let (stopped, done) = tokio::sync::oneshot::channel();
        struct Stopped(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Stopped {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let stopped = Stopped(Some(stopped));
        let worker = Worker::new(tokio::spawn(async move {
            let _stopped = stopped;
            started.send(()).unwrap();
            std::future::pending::<WorkerResult>().await
        }));
        (worker, ready, done)
    }

    #[tokio::test]
    async fn failed_reconnect_cleanup_never_becomes_success_after_abort_join() {
        let (worker, ready, done) = pending_worker();
        ready.await.unwrap();
        let now = Instant::now();
        assert!(
            join_worker(&worker, now, now + Duration::from_secs(1), false)
                .await
                .is_err()
        );
        done.await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_waiter_leaves_termination_join_owned_by_host() {
        let (worker, ready, done) = pending_worker();
        ready.await.unwrap();
        let mut waiter = Box::pin(worker.join.clone());
        assert!(futures_util::poll!(&mut waiter).is_pending());
        drop(waiter);
        let now = Instant::now();
        join_worker(&worker, now, now + Duration::from_secs(1), true)
            .await
            .unwrap();
        done.await.unwrap();
    }

    #[tokio::test]
    async fn actual_cleanup_error_is_not_swallowed_by_shutdown() {
        let worker = Worker::new(tokio::spawn(async {
            Err("observer cleanup failed".into())
        }));
        let now = Instant::now();
        let error = join_worker(
            &worker,
            now + Duration::from_secs(1),
            now + Duration::from_secs(2),
            true,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "observer cleanup failed");
    }

    #[tokio::test]
    async fn disconnected_controls_still_coalesce_layout_preference() {
        let (commands, command_rx) = mpsc::channel(1);
        drop(command_rx);
        let (layout, mut layout_rx) = watch::channel(None);
        let (acknowledgements, _) = watch::channel(None);
        let (focused, _) = watch::channel(true);
        let (stop, _) = watch::channel(false);
        let controls = Controls {
            commands,
            transactions: mpsc::channel(1).0,
            discarded_review: watch::channel(None).0,
            cancellation: None,
            layout,
            acknowledgements,
            focused,
            stop,
            closing: Arc::new(AtomicBool::new(false)),
            submission: Arc::new(Mutex::new(())),
        };
        controls.send(Command::Layout(Layout::SideBySide)).unwrap();
        controls
            .send(Command::Layout(Layout::ResultsFirst))
            .unwrap();
        layout_rx.changed().await.unwrap();
        assert_eq!(*layout_rx.borrow_and_update(), Some(Layout::ResultsFirst));
        let submission = controls.submission.lock().unwrap();
        let sender = controls.clone();
        let attempted = std::sync::Arc::new(std::sync::Barrier::new(2));
        let attempting = attempted.clone();
        let pending = std::thread::spawn(move || {
            attempting.wait();
            sender.send(Command::Layout(Layout::Stacked))
        });
        attempted.wait();
        controls.closing.store(true, Ordering::Release);
        drop(submission);
        assert!(pending.join().unwrap().is_err());
        assert_eq!(*layout_rx.borrow(), Some(Layout::ResultsFirst));
    }
}
