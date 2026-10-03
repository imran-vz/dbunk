//! One window owner with independently owned document sessions. Reconnect waits
//! only for that document; owner registration and heartbeat outlive every tab.
use super::*;
use dbunk_lib::backend::QueryEvent;
use std::collections::HashMap;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const SESSION_LIMIT: usize = 4;
const EXECUTION_LIMIT: usize = 2;

struct Execution {
    id: String,
    _permit: OwnedSemaphorePermit,
}
type ExecutionState = Arc<Mutex<Option<Execution>>>;

#[derive(Clone)]
struct Document {
    controls: Controls,
    worker: Worker,
    session: String,
    connection: String,
    events: mailbox::Sender,
    opening: Arc<AtomicBool>,
}
#[derive(Default)]
struct Documents {
    active: HashMap<String, Document>,
    workers: Vec<(String, Worker)>,
    failure: Option<String>,
}
struct State {
    documents: Mutex<Documents>,
    closing: Arc<AtomicBool>,
    submission: Arc<Mutex<()>>,
    executions: Arc<Semaphore>,
}
pub(super) struct WorkspaceRuntime {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    owner: String,
    state: Arc<State>,
    layout: watch::Sender<Option<Layout>>,
    focused: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    registration: Worker,
    heartbeat: Worker,
    queue_budget: mailbox::ByteBudget,
}

impl WorkspaceRuntime {
    pub(super) fn new(
        backend: Backend,
        runtime: tokio::runtime::Handle,
        owner: String,
        layout: watch::Sender<Option<Layout>>,
        closing: Arc<AtomicBool>,
        submission: Arc<Mutex<()>>,
    ) -> Self {
        let state = Arc::new(State {
            documents: Mutex::new(Documents::default()),
            closing,
            submission,
            executions: Arc::new(Semaphore::new(EXECUTION_LIMIT)),
        });
        let registration_backend = backend.clone();
        let registered_owner = owner.clone();
        let registration = Worker::new(runtime.spawn(async move {
            registration_backend
                .register_owner(
                    WINDOW,
                    RegisterOwnerPayload {
                        owner_id: registered_owner,
                    },
                )
                .await
                .map(|_| ())
                .map_err(error_message)
        }));
        let (focused, focus_rx) = watch::channel(true);
        let (stop, stop_rx) = watch::channel(false);
        let heartbeat_backend = backend.clone();
        let heartbeat_owner = owner.clone();
        let heartbeat_state = state.clone();
        let heartbeat_registration = registration.clone();
        let heartbeat = Worker::new(runtime.spawn(async move {
            let result = heartbeat(
                heartbeat_backend,
                heartbeat_owner,
                heartbeat_state.clone(),
                heartbeat_registration,
                focus_rx,
                stop_rx,
            )
            .await;
            if let Err(error) = &result {
                let mut documents = heartbeat_state.documents.lock().unwrap();
                documents.failure.get_or_insert_with(|| error.clone());
                for document in documents.active.values() {
                    document.events.fail(Failure::Backend(error.clone()));
                    document.controls.stop();
                }
            }
            result
        }));
        Self {
            backend,
            runtime,
            owner,
            state,
            layout,
            focused,
            stop,
            registration,
            heartbeat,
            queue_budget: mailbox::ByteBudget::new(mailbox::WORKSPACE_QUEUE_BYTES),
        }
    }

    pub(super) fn owner(&self) -> &str {
        &self.owner
    }
    pub(super) fn queue_budget(&self) -> mailbox::ByteBudget {
        self.queue_budget.clone()
    }
    pub(super) fn mailbox(&self) -> (mailbox::Sender, mailbox::Receiver) {
        mailbox::channel_with_budget(
            mailbox::QUEUE_CAPACITY,
            mailbox::QUEUE_BYTES,
            self.queue_budget.clone(),
        )
    }

    pub(super) fn connect(
        &self,
        tab: String,
        connection: String,
        session: String,
        events: mailbox::Sender,
    ) -> Result<Controls, &'static str> {
        if !events.shares_budget(&self.queue_budget) {
            return Err("Document mailbox must use the workspace queue budget");
        }
        if tab.is_empty() || connection.is_empty() || session.is_empty() {
            return Err("Document, connection and session IDs are required");
        }
        let _submission = self.state.submission.lock().unwrap();
        let mut documents = self.state.documents.lock().unwrap();
        if self.state.closing.load(Ordering::Acquire) {
            return Err("Application is closing");
        }
        let mut failure = None;
        documents
            .workers
            .retain(|(_, worker)| match worker.join.clone().now_or_never() {
                Some(Err(error)) => {
                    failure = Some(error.to_string());
                    false
                }
                Some(Ok(())) => false,
                None => true,
            });
        if documents.failure.is_none() {
            documents.failure = failure;
        }
        if documents.failure.is_some() {
            return Err("Previous session cleanup failed. Close the workspace.");
        }
        if !documents.active.contains_key(&tab) && documents.active.len() >= SESSION_LIMIT {
            return Err("Four sessions are open. Disconnect a document before connecting another.");
        }
        if documents
            .active
            .get(&tab)
            .is_some_and(|document| document.opening.load(Ordering::Acquire))
        {
            return Err("Connection is already pending");
        }
        if documents
            .active
            .get(&tab)
            .is_some_and(|document| document.controls.stop.borrow().to_owned())
        {
            return Err("Document is still disconnecting");
        }
        if documents.workers.iter().any(|(id, _)| id == &session) {
            return Err("Reconnect requires a new session ID");
        }
        // Only one replacement may wait for a document's previous cleanup.
        if documents
            .active
            .get(&tab)
            .is_some_and(|document| document.controls.commands.is_closed())
        {
            return Err("Document is still disconnecting");
        }
        let (commands, command_rx) = mpsc::channel(16);
        let (transactions, transaction_rx) = mpsc::channel(1);
        let (discarded_review, review_rx) = watch::channel(None);
        let (cancellation, cancel_rx) = watch::channel(None);
        let (acknowledgements, ack_rx) = watch::channel(None);
        let (stop, stop_rx) = watch::channel(false);
        let controls = Controls {
            commands,
            transactions,
            discarded_review,
            cancellation: Some(cancellation),
            acknowledgements,
            focused: self.focused.clone(),
            stop,
            layout: self.layout.clone(),
            closing: self.state.closing.clone(),
            submission: self.state.submission.clone(),
        };
        let previous = documents.active.remove(&tab);
        if let Some(previous) = &previous {
            previous.controls.stop();
        }
        let state = self.state.clone();
        let backend = self.backend.clone();
        let owner = self.owner.clone();
        let worker_tab = tab.clone();
        let worker_session = session.clone();
        let worker_events = events.clone();
        let worker_connection = connection.clone();
        let registration = self.registration.clone();
        let opening = Arc::new(AtomicBool::new(true));
        let worker_opening = opening.clone();
        let focused = self.focused.subscribe();
        let worker = Worker::new(self.runtime.spawn(async move {
            let result: WorkerResult = async {
                if let Some(previous) = previous {
                    let now = Instant::now();
                    join_worker(&previous.worker, now + GRACE, now + TOTAL, false).await?;
                }
                if state.closing.load(Ordering::Acquire) || *stop_rx.borrow() { return Ok(()); }
                let mut registration_stop = stop_rx.clone();
                tokio::select! {
                    biased;
                    _ = registration_stop.changed() => return Ok(()),
                    result = registration.join.clone() => result.map_err(|error| error.to_string())?,
                }
                let execution = Arc::new(Mutex::new(None));
                let work = document_worker(
                    &backend, &owner, &worker_tab, &worker_session, worker_connection,
                    worker_events.clone(), command_rx, transaction_rx, review_rx, cancel_rx, ack_rx, stop_rx,
                    state.executions.clone(), execution.clone(), worker_opening.clone(), focused,
                ).await;
                // Backend admission waits any still-owned open before closing
                // this session. Never retire the window for one document.
                if !state.closing.load(Ordering::Acquire) {
                    backend.close_native_session(WINDOW, &worker_session).await.map_err(error_message)?;
                }
                execution.lock().unwrap().take();
                if let Err(error) = work { worker_events.fail(Failure::Backend(error)); }
                Ok(())
            }.await;
            worker_opening.store(false, Ordering::Release);
            let mut documents = state.documents.lock().unwrap();
            if documents.active.get(&worker_tab).is_some_and(|document| document.session == worker_session) {
                documents.active.remove(&worker_tab);
            }
            if let Err(error) = &result {
                documents.failure.get_or_insert_with(|| error.clone());
                worker_events.fail(Failure::Backend(error.clone()));
            }
            result
        }));
        documents.workers.push((session.clone(), worker.clone()));
        documents.active.insert(
            tab,
            Document {
                controls: controls.clone(),
                worker,
                session,
                connection,
                events,
                opening,
            },
        );
        Ok(controls)
    }

    pub(super) async fn disconnect(&self, tab: &str) -> WorkerResult {
        let document = self
            .state
            .documents
            .lock()
            .unwrap()
            .active
            .get(tab)
            .cloned();
        let Some(document) = document else {
            return Ok(());
        };
        document.controls.stop();
        let now = Instant::now();
        let result = join_worker(&document.worker, now + GRACE, now + TOTAL, false).await;
        if let Err(error) = &result {
            self.state
                .documents
                .lock()
                .unwrap()
                .failure
                .get_or_insert_with(|| error.clone());
        }
        result
    }

    /// Connections with an admitted query session; opening alone counts.
    pub(super) fn visit_connections(&self, visit: &mut dyn FnMut(&str)) {
        for document in self.state.documents.lock().unwrap().active.values() {
            visit(&document.connection);
        }
    }

    pub(super) async fn disconnect_matching(&self, connection: Option<&str>) -> WorkerResult {
        let documents = self
            .state
            .documents
            .lock()
            .unwrap()
            .active
            .values()
            .filter(|document| connection.is_none_or(|id| document.connection == id))
            .cloned()
            .collect::<Vec<_>>();
        for document in &documents {
            document.controls.stop();
        }
        let now = Instant::now();
        let results = futures_util::future::join_all(
            documents
                .iter()
                .map(|document| join_worker(&document.worker, now + GRACE, now + TOTAL, false)),
        )
        .await;
        let result = results
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map(|_| ());
        if let Err(error) = &result {
            self.state
                .documents
                .lock()
                .unwrap()
                .failure
                .get_or_insert_with(|| error.clone());
        }
        result
    }

    pub(super) fn stop(&self) {
        self.stop.send_replace(true);
        for document in self.state.documents.lock().unwrap().active.values() {
            document.controls.stop();
        }
    }

    pub(super) async fn join(&self, grace: Instant, deadline: Instant) -> WorkerResult {
        let (workers, failure) = {
            let documents = self.state.documents.lock().unwrap();
            (
                documents
                    .workers
                    .iter()
                    .map(|(_, worker)| worker.clone())
                    .collect::<Vec<_>>(),
                documents.failure.clone(),
            )
        };
        let results = futures_util::future::join_all(
            workers
                .iter()
                .chain([&self.registration, &self.heartbeat])
                .map(|worker| join_worker(worker, grace, deadline, true)),
        )
        .await;
        eprintln!(
            "Native workspace queue: high_water_bytes={} remaining_bytes={}",
            self.queue_budget.high_water(),
            self.queue_budget.used(),
        );
        if let Some(failure) = failure {
            return Err(failure);
        }
        results
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map(|_| ())
    }
}

#[allow(clippy::too_many_arguments)]
async fn document_worker(
    backend: &Backend,
    owner: &str,
    tab: &str,
    session: &str,
    connection: String,
    events: mailbox::Sender,
    mut commands: mpsc::Receiver<Command>,
    mut transactions: mpsc::Receiver<TransactionControl>,
    mut discarded_review: watch::Receiver<Option<String>>,
    mut cancellations: watch::Receiver<Option<ExecutionPayload>>,
    mut acknowledgements: watch::Receiver<Option<AckPayload>>,
    mut stop: watch::Receiver<bool>,
    executions: Arc<Semaphore>,
    execution: ExecutionState,
    opening_state: Arc<AtomicBool>,
    focused: watch::Receiver<bool>,
) -> WorkerResult {
    if *stop.borrow() {
        return Ok(());
    }
    let connection_record = backend
        .development_connections()
        .await?
        .into_iter()
        .find(|value| value.id == connection);
    let capture = Arc::new(Mutex::new(
        None::<(crate::query_library::Capture, mailbox::BytePermit)>,
    ));
    let capture_sink = capture.clone();
    let (history_send, mut history_receive) = mpsc::channel(1);
    let sink = events.clone();
    let completed = execution.clone();
    let opening = backend.open(
        WINDOW,
        OpenSessionPayload {
            owner_id: owner.into(),
            session_id: session.into(),
            tab_id: tab.into(),
            connection_id: connection.clone(),
        },
        Arc::new(move |event: QueryEventEnvelope| {
            let mut pending = capture_sink.lock().unwrap();
            if pending.as_ref().is_some_and(|(capture, _)| {
                Some(capture.record.id.as_str()) == event.execution_id.as_deref()
            }) {
                if let Some((capture, _)) = pending.as_mut() {
                    capture.observe(&event.event);
                }
                if matches!(event.event, QueryEvent::ExecutionCompleted { .. })
                    && let Some((record, permit)) = pending.take().and_then(|(capture, permit)| {
                        capture.finish(&event.event).map(|record| (record, permit))
                    })
                {
                    history_send
                        .try_send((record, permit))
                        .map_err(|_| dbunk_lib::backend::SinkClosed)?;
                }
            }
            drop(pending);
            if matches!(event.event, QueryEvent::ExecutionCompleted { .. }) {
                release_execution(&completed, event.execution_id.as_deref());
            }
            sink.send(Message::Event(event))
        }),
    );
    tokio::select! {
        biased;
        _ = stop.changed() => return Ok(()),
        _ = events.failed() => return Ok(()),
        result = opening => result.map_err(error_message)?,
    };
    if *stop.borrow() {
        return Ok(());
    }
    let focused = *focused.borrow();
    backend
        .set_focus(WINDOW, focused)
        .await
        .map_err(error_message)?;
    opening_state.store(false, Ordering::Release);
    events
        .send(Message::Ready)
        .map_err(|_| "Connection event delivery failed")?;
    let mut query_controls = QueryControls::default();
    loop {
        // Flush a delivered terminal even when the document is closing. The
        // worker owns this SQLite acknowledgement and is joined by the host.
        if let Ok((record, _permit)) = history_receive.try_recv()
            && let Err(error) = backend.append_query_history(record).await
        {
            let _ = events.send(Message::HistoryFailed(error.to_string()));
        }
        if *stop.borrow() || events.is_failed() {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = stop.changed() => continue,
            _ = events.failed() => continue,
            record = history_receive.recv() => {
                if let Some((record, _permit)) = record
                    && let Err(error) = backend.append_query_history(record).await {
                        let _ = events.send(Message::HistoryFailed(error.to_string()));
                }
            }
            changed = discarded_review.changed() => {
                if changed.is_err() { return Ok(()); }
                if let Some(id) = discarded_review.borrow_and_update().as_deref() { query_controls.discard(id); let mut pending = capture.lock().unwrap(); if pending.as_ref().is_some_and(|(capture, _)| capture.record.id == id) { pending.take(); } }
            }
            changed = cancellations.changed() => {
                if changed.is_err() { return Ok(()); }
                let payload = cancellations.borrow_and_update().clone();
                if let Some(payload) = payload {
                    if payload.session_id != session { return Err("Cancellation belongs to another document".into()); }
                    let id = payload.execution_id.clone();
                    if let Err(error) = backend.cancel(WINDOW, payload).await {
                        let _ = events.send(Message::CancelFailed { execution: id, message: error_message(error) });
                    }
                }
            }
            changed = acknowledgements.changed() => {
                if changed.is_err() { return Ok(()); }
                let payload = acknowledgements.borrow_and_update().clone();
                if let Some(payload) = payload {
                    if payload.session_id != session { return Err("Acknowledgement belongs to another document".into()); }
                    let id = payload.execution_id.clone(); let sequence = payload.ack_through_sequence;
                    backend.ack(WINDOW, payload).await.map_err(error_message)?;
                    events.send(Message::Acked { execution: id, sequence }).map_err(|_| "ACK reply delivery failed")?;
                }
            }
            control = transactions.recv() => {
                let Some(control) = control else { return Ok(()); };
                let result = backend.control_transaction(WINDOW, session, control).await;
                events.send(Message::Transaction { session: session.into(), result }).map_err(|_| "Transaction reply delivery failed")?;
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                match command {
                    Command::Run(payload) => {
                        let id = payload.execution_id.clone();
                        let admitted = if payload.session_id != session { Err("Execution belongs to another document") }
                        else if payload.sql.len() > 1024 * 1024 { Err("SQL exceeds the 1 MiB native query/history budget") }
                        else { reserve_execution(&execution, &executions, &id) };
                        if let Err(reason) = admitted {
                            events.send(Message::Rejected { execution: id, message: reason.into() }).map_err(|_| "Query refusal delivery failed")?;
                            continue;
                        }
                        // SQL and the pending terminal share workspace delivery
                        // admission until the SQLite append is acknowledged.
                        capture.lock().unwrap().take();
                        let (name, database) = connection_record.as_ref().map(|value| (value.name.as_str(), value.postgres.as_ref().map(|data| data.database.as_str()).unwrap_or_default())).unwrap_or((connection.as_str(), ""));
                        // Include worst-case JSON expansion for the bounded
                        // terminal error plus fixed record keys/timestamps.
                        let history_bytes = crate::results::encoded_size(&(&payload.sql, &id, &connection, name, database)) + 8192 * 6 + 1024;
                        let Some(history_permit) = events.reserve_history(history_bytes) else {
                            release_execution(&execution, Some(&id));
                            events.send(Message::Rejected { execution: id, message: "Workspace delivery budget is full; retry after results drain".into() }).map_err(|_| "Query refusal delivery failed")?;
                            continue;
                        };
                        *capture.lock().unwrap() = Some((crate::query_library::Capture::new(dbunk_lib::backend::query_library::HistoryRecord::started(id.clone(), payload.sql.clone(), connection.clone(), name.into(), database.into())), history_permit));
                        let accepted = query_controls.submit(backend, payload, &discarded_review, &events).await;
                        if !matches!(accepted, Ok(true)) {
                            release_execution(&execution, Some(&id));
                            if !query_controls.has_confirmation(&id) { capture.lock().unwrap().take(); }
                        }
                        accepted?;
                    }
                    Command::Confirm(id) => {
                        if let Err(reason) = reserve_execution(&execution, &executions, &id) {
                            query_controls.discard(&id);
                            events.send(Message::Rejected { execution: id, message: reason.into() }).map_err(|_| "Query refusal delivery failed")?;
                            continue;
                        }
                        let accepted = query_controls.confirm(backend, id.clone(), &discarded_review, &events).await;
                        if !matches!(accepted, Ok(true)) {
                            release_execution(&execution, Some(&id));
                            if !query_controls.has_confirmation(&id) { capture.lock().unwrap().take(); }
                        }
                        accepted?;
                    }
                    Command::Cancel(_) | Command::Layout(_) | Command::Transaction(_) => unreachable!("separate control channels"),
                }
            }
        }
    }
}

fn reserve_execution(
    state: &ExecutionState,
    permits: &Arc<Semaphore>,
    id: &str,
) -> Result<(), &'static str> {
    let mut active = state.lock().unwrap();
    if active.is_some() {
        return Err("A query is already running in this document");
    }
    let permit = permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| "Two queries are running. Stop one or wait before running another.")?;
    *active = Some(Execution {
        id: id.into(),
        _permit: permit,
    });
    Ok(())
}
fn release_execution(state: &ExecutionState, id: Option<&str>) {
    let mut active = state.lock().unwrap();
    if active
        .as_ref()
        .is_some_and(|execution| Some(execution.id.as_str()) == id)
    {
        active.take();
    }
}

async fn heartbeat(
    backend: Backend,
    owner: String,
    state: Arc<State>,
    registration: Worker,
    mut focus: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
) -> WorkerResult {
    registration
        .join
        .clone()
        .await
        .map_err(|error| error.to_string())?;
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        if *stop.borrow() || state.closing.load(Ordering::Acquire) {
            return Ok(());
        }
        tokio::select! {
            biased;
            _ = stop.changed() => return Ok(()),
            changed = focus.changed() => {
                if changed.is_err() { return Ok(()); }
                let focused = *focus.borrow_and_update();
                if let Err(error) = backend.set_focus(WINDOW, focused).await {
                    if state.closing.load(Ordering::Acquire) { return Ok(()); }
                    return Err(error_message(error));
                }
            }
            _ = interval.tick() => {
                let documents = state.documents.lock().unwrap().active.values().cloned().collect::<Vec<_>>();
                let mut alive = Vec::new();
                for document in documents {
                    match backend.session_alive(WINDOW, &document.session).await {
                        Ok(true) => alive.push(document.session),
                        Ok(false) | Err(QuerySessionError::SessionNotFound) => {
                            if !document.opening.load(Ordering::Acquire) {
                                document.events.fail(Failure::Backend("Connection lost. Reconnect to run again.".into()));
                                document.controls.stop();
                            }
                        },
                        Err(error) => {
                            if state.closing.load(Ordering::Acquire) { return Ok(()); }
                            document.events.fail(Failure::Backend(error_message(error)));
                        }
                    }
                }
                if *focus.borrow() && !alive.is_empty()
                    && let Err(error) = backend.heartbeat(WINDOW, HeartbeatPayload { owner_id: owner.clone(), session_ids: alive }).await {
                        if state.closing.load(Ordering::Acquire) { return Ok(()); }
                        return Err(error_message(error));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execution_admission_is_shared_and_stale_terminal_cannot_release_another_run() {
        let budget = Arc::new(Semaphore::new(EXECUTION_LIMIT));
        let first = Arc::new(Mutex::new(None));
        let second = Arc::new(Mutex::new(None));
        let third = Arc::new(Mutex::new(None));
        reserve_execution(&first, &budget, "one").unwrap();
        reserve_execution(&second, &budget, "two").unwrap();
        assert!(reserve_execution(&third, &budget, "three").is_err());
        assert!(reserve_execution(&first, &budget, "replacement").is_err());
        release_execution(&first, Some("stale"));
        assert_eq!(budget.available_permits(), 0);
        release_execution(&first, Some("one"));
        reserve_execution(&third, &budget, "three").unwrap();
        // Closing one document releases only its own execution permit.
        drop(second);
        assert_eq!(budget.available_permits(), 1);
        assert!(third.lock().unwrap().is_some());
        release_execution(&third, Some("three"));
        assert_eq!(budget.available_permits(), EXECUTION_LIMIT);
    }

    #[tokio::test]
    async fn cancellation_review_discard_and_transaction_lane_bypass_saturated_run_queue() {
        let (commands, _command_rx) = mpsc::channel(1);
        let (cancellation, mut cancel_rx) = watch::channel(None);
        let (transactions, mut transaction_rx) = mpsc::channel(1);
        let (discarded_review, mut review_rx) = watch::channel(None);
        let (layout, _) = watch::channel(None);
        let (acknowledgements, _) = watch::channel(None);
        let (focused, _) = watch::channel(true);
        let (stop, mut stop_rx) = watch::channel(false);
        let controls = Controls {
            commands,
            transactions,
            discarded_review,
            cancellation: Some(cancellation),
            layout,
            acknowledgements,
            focused,
            stop,
            closing: Arc::new(AtomicBool::new(false)),
            submission: Arc::new(Mutex::new(())),
        };
        let run = || {
            Command::Run(ExecutePayload {
                session_id: "session".into(),
                execution_id: "run".into(),
                sql: "select 1".into(),
                parameters: None,
                confirmed: false,
                row_limit: None,
            })
        };
        controls.send(run()).unwrap();
        assert!(controls.send(run()).is_err());
        controls
            .send(Command::Cancel(ExecutionPayload {
                session_id: "session".into(),
                execution_id: "run".into(),
            }))
            .unwrap();
        cancel_rx.changed().await.unwrap();
        assert_eq!(
            cancel_rx.borrow_and_update().as_ref().unwrap().execution_id,
            "run"
        );
        controls
            .send(Command::Transaction(TransactionControl::Recheck))
            .unwrap();
        assert!(
            controls
                .send(Command::Transaction(TransactionControl::Rollback))
                .is_err()
        );
        assert_eq!(
            transaction_rx.recv().await,
            Some(TransactionControl::Recheck)
        );
        controls.discard_confirmation("run".into());
        review_rx.changed().await.unwrap();
        assert_eq!(review_rx.borrow_and_update().as_deref(), Some("run"));
        controls.stop();
        stop_rx.changed().await.unwrap();
        assert!(*stop_rx.borrow());
    }

    struct Profile {
        path: std::path::PathBuf,
        marker: Vec<u8>,
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            if std::fs::read(self.path.join(".dbunk-native-stage03"))
                .ok()
                .as_ref()
                == Some(&self.marker)
            {
                std::fs::remove_dir_all(&self.path).unwrap();
            }
        }
    }
    async fn backend() -> (Profile, Backend) {
        let id = uuid::Uuid::new_v4();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("dbunk-native-workspace-test-{id}"));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let marker = serde_json::to_vec(&serde_json::json!({
            "version":1, "fixture":"dbunk-native-stage03", "host":"127.0.0.1", "port":15432,
            "database":"dbunk_demo", "profile_id":id.to_string(),
        }))
        .unwrap();
        std::fs::write(path.join(".dbunk-native-stage03"), &marker).unwrap();
        let backend = Backend::open_fixture(&path).await.unwrap();
        (Profile { path, marker }, backend)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_one_pending_document_preserves_other_documents() {
        let (_profile, backend) = backend().await;
        let (layout, _) = watch::channel(None);
        let closing = Arc::new(AtomicBool::new(false));
        let mut workspace = WorkspaceRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            "workspace-owner".into(),
            layout,
            closing.clone(),
            Arc::new(Mutex::new(())),
        );
        workspace.registration.join.clone().await.unwrap();
        // Hold only document startup, after the single real owner registration.
        // No open can reach the network in this headless race test.
        let (release, blocked) = tokio::sync::oneshot::channel();
        workspace.registration = Worker::new(tokio::spawn(async {
            blocked.await.unwrap();
            Ok(())
        }));
        let mut receivers = Vec::new();
        for index in 0..SESSION_LIMIT {
            let (events, receiver) = workspace.mailbox();
            workspace
                .connect(
                    format!("tab-{index}"),
                    backend.fixture().id,
                    format!("session-{index}"),
                    events,
                )
                .unwrap();
            receivers.push(receiver);
        }
        let (events, refused) = workspace.mailbox();
        assert!(
            workspace
                .connect(
                    "fifth".into(),
                    backend.fixture().id,
                    "fifth-session".into(),
                    events
                )
                .err()
                .unwrap()
                .contains("Four sessions")
        );
        drop(refused);
        tokio::time::timeout(Duration::from_secs(1), workspace.disconnect("tab-0"))
            .await
            .unwrap()
            .unwrap();
        {
            let documents = workspace.state.documents.lock().unwrap();
            assert_eq!(documents.active.len(), SESSION_LIMIT - 1);
            assert!(
                documents
                    .active
                    .values()
                    .all(|document| !*document.controls.stop.borrow())
            );
        }
        let (events, replacement) = workspace.mailbox();
        workspace
            .connect(
                "tab-0".into(),
                backend.fixture().id,
                "replacement-session".into(),
                events,
            )
            .unwrap();
        receivers.push(replacement);
        workspace.stop();
        closing.store(true, Ordering::Release);
        release.send(()).unwrap();
        let now = Instant::now();
        workspace
            .join(now + Duration::from_secs(1), now + Duration::from_secs(2))
            .await
            .unwrap();
        let owner = backend
            .register_owner(
                WINDOW,
                RegisterOwnerPayload {
                    owner_id: "workspace-owner".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(owner.replaced_session_count, 0);
        backend.shutdown().await.unwrap();
        drop(receivers);
        assert_eq!(workspace.queue_budget.used(), 0);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn query_library_close_fences_queued_edits_and_releases_delivery_budget() {
        use crate::controller::{LibraryCommand, query_library_runtime::LibraryRuntime};
        use dbunk_lib::backend::{WorkspaceTool, query_library::SavedQueryRecord};
        let (_profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(mailbox::WORKSPACE_QUEUE_BYTES);
        let runtime = LibraryRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (wake, awakened) = async_channel::bounded(1);
        let (controls, receiver) = runtime.open("library".into(), wake).unwrap();
        controls
            .send(LibraryCommand::Load(
                WorkspaceTool::SavedQueries,
                Default::default(),
            ))
            .unwrap();
        awakened.recv().await.unwrap();
        assert!(budget.used() >= 8 * 1024 * 1024);
        // The unread page is a read lease: a queued edit cannot overtake it.
        controls
            .send(LibraryCommand::SaveDraft(SavedQueryRecord {
                id: "never-dispatched".into(),
                name: "Draft".into(),
                body: "select 1".into(),
                connection_id: None,
                is_favorite: false,
                owner_id: None,
                created_at: String::new(),
                updated_at: String::new(),
            }))
            .unwrap();
        runtime.close("library").await.unwrap();
        assert!(controls.send(LibraryCommand::Clear).is_err());
        assert!(
            backend
                .load_saved_queries(Default::default())
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        drop(receiver);
        assert_eq!(budget.used(), 0);
        runtime.stop();
        assert!(
            runtime
                .open("new".into(), async_channel::bounded(1).0)
                .is_err()
        );
        backend.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn query_library_save_acknowledgement_survives_document_close_and_reopen() {
        use crate::controller::{
            LibraryCommand, LibraryReply, query_library_runtime::LibraryRuntime,
        };
        use dbunk_lib::backend::{WorkspaceTool, query_library::SavedQueryRecord};
        let (_profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(mailbox::WORKSPACE_QUEUE_BYTES);
        let runtime = LibraryRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (controls, receiver) = runtime
            .open("library".into(), async_channel::bounded(1).0)
            .unwrap();
        controls
            .send(LibraryCommand::SaveDraft(SavedQueryRecord {
                id: "saved".into(),
                name: "Exact draft".into(),
                body: "SELECT '東京';\n\n".into(),
                connection_id: Some("removed".into()),
                is_favorite: false,
                owner_id: None,
                created_at: String::new(),
                updated_at: String::new(),
            }))
            .unwrap();
        let delivery = receiver.recv().await.unwrap();
        assert!(
            matches!(&delivery.result, Ok(LibraryReply::Saved(query)) if query.body == "SELECT '東京';\n\n")
        );
        drop(delivery);
        runtime.close("library").await.unwrap();
        drop(receiver);
        let (controls, receiver) = runtime
            .open("library".into(), async_channel::bounded(1).0)
            .unwrap();
        controls
            .send(LibraryCommand::Load(
                WorkspaceTool::SavedQueries,
                Default::default(),
            ))
            .unwrap();
        let delivery = receiver.recv().await.unwrap();
        assert!(
            matches!(&delivery.result, Ok(LibraryReply::Page(crate::query_library::Rows::Saved(page))) if page.entries[0].body == "SELECT '東京';\n\n")
        );
        drop(delivery);
        runtime.stop();
        let now = Instant::now();
        runtime
            .join(now + Duration::from_secs(1), now + Duration::from_secs(2))
            .await
            .unwrap();
        drop(receiver);
        assert_eq!(budget.used(), 0);
        backend.shutdown().await.unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_audit_delivery_is_bounded_joined_and_reopens_after_disconnect() {
        use crate::controller::{
            LibraryCommand, LibraryReply, query_library_runtime::LibraryRuntime,
        };
        let (_profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(mailbox::WORKSPACE_QUEUE_BYTES);
        let runtime = LibraryRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (wake, awakened) = async_channel::bounded(1);
        let (controls, receiver) = runtime.open("admin".into(), wake).unwrap();
        controls
            .send(LibraryCommand::SafetyAudit(
                7,
                "deleted-connection".into(),
                None,
            ))
            .unwrap();
        let delivery = receiver.recv().await.unwrap();
        awakened.recv().await.unwrap();
        assert!(
            matches!(&delivery.result, Ok(LibraryReply::SafetyAudit(7, Ok(page))) if page.connection_id == "deleted-connection" && page.rows.is_empty() && page.next_cursor.is_none())
        );
        assert!(budget.used() >= 512 * 1024 && budget.used() < 1024 * 1024);
        controls
            .send(LibraryCommand::SafetyAudit(
                8,
                "deleted-connection".into(),
                None,
            ))
            .unwrap();
        runtime.close("admin").await.unwrap();
        // Closing has no result to deliver. The UI still needs a wake to clear
        // its pending request when it observes the now-closed reply channel.
        awakened
            .try_recv()
            .expect("joined worker exit must wake the document");
        assert!(controls.is_closed());
        assert!(
            controls
                .send(LibraryCommand::SafetyAudit(
                    9,
                    "deleted-connection".into(),
                    None
                ))
                .is_err()
        );
        drop(delivery);
        drop(receiver);
        assert_eq!(budget.used(), 0);
        let (controls, receiver) = runtime
            .open("admin".into(), async_channel::bounded(1).0)
            .unwrap();
        controls
            .send(LibraryCommand::SafetyAudit(10, String::new(), None))
            .unwrap();
        let delivery = receiver.recv().await.unwrap();
        // Refused requests preserve their identity and do not fabricate an empty page.
        assert!(matches!(
            &delivery.result,
            Ok(LibraryReply::SafetyAudit(10, Err(_)))
        ));
        drop(delivery);
        runtime.close("admin").await.unwrap();
        drop(receiver);
        assert_eq!(budget.used(), 0);
        backend.shutdown().await.unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pg_tool_observer_shutdown_fences_unconsumed_queue_and_keeps_single_owner() {
        use crate::controller::{ToolCommand, ToolReply, pg_tool_runtime::PgToolRuntime};
        use dbunk_lib::backend::pg_tools::{
            PgToolAttemptId, PgToolFormat, PgToolIntent, PgToolScope,
        };
        let (profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(mailbox::WORKSPACE_QUEUE_BYTES);
        let runtime = PgToolRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (wake, awakened) = async_channel::bounded(1);
        let (controls, receiver) = runtime.open(wake.clone()).unwrap();
        assert!(runtime.open(wake).is_err());
        controls.send(ToolCommand::List(1)).unwrap();
        let delivery = receiver.recv().await.unwrap();
        awakened.recv().await.unwrap();
        assert_eq!(delivery.request, 1);
        assert!(matches!(&delivery.result, Ok(ToolReply::List(list)) if list.jobs.is_empty()));
        assert!(budget.used() >= 512 * 1024);
        let attempt = PgToolAttemptId::new();
        controls
            .send(ToolCommand::Begin(
                2,
                attempt,
                "owned-test".into(),
                PgToolIntent::backup(
                    profile.path.join("must-not-be-created.dump"),
                    PgToolFormat::Custom,
                    PgToolScope::Database,
                    false,
                )
                .unwrap(),
            ))
            .unwrap();
        runtime.stop();
        let now = Instant::now();
        runtime
            .join(now + Duration::from_secs(1), now + Duration::from_secs(2))
            .await
            .unwrap();
        awakened
            .try_recv()
            .expect("exit closes replies before waking");
        assert!(receiver.is_closed());
        assert!(backend.list_pg_tool_jobs(None).unwrap().jobs.is_empty());
        assert!(!profile.path.join("must-not-be-created.dump").exists());
        assert!(controls.send(ToolCommand::List(3)).is_err());
        drop(delivery);
        drop(receiver);
        assert_eq!(budget.used(), 0);
        backend.shutdown().await.unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pg_tool_delivery_refusal_precedes_attempt_registration() {
        use crate::controller::{
            ToolCommand,
            pg_tool_runtime::{PgToolRuntime, ToolFailure},
        };
        use dbunk_lib::backend::pg_tools::{
            PgToolAttemptId, PgToolFormat, PgToolIntent, PgToolScope,
        };
        let (profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(4096);
        let runtime = PgToolRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (controls, receiver) = runtime.open(async_channel::bounded(1).0).unwrap();
        controls
            .send(ToolCommand::Begin(
                17,
                PgToolAttemptId::new(),
                "owned-test".into(),
                PgToolIntent::backup(
                    profile.path.join("not-dispatched.dump"),
                    PgToolFormat::Custom,
                    PgToolScope::Database,
                    false,
                )
                .unwrap(),
            ))
            .unwrap();
        let delivery = receiver.recv().await.unwrap();
        assert_eq!(delivery.request, 17);
        assert!(matches!(delivery.result, Err(ToolFailure::DeliveryBudget)));
        assert!(backend.list_pg_tool_jobs(None).unwrap().jobs.is_empty());
        drop(delivery);
        runtime.stop();
        let now = Instant::now();
        runtime
            .join(now + Duration::from_secs(1), now + Duration::from_secs(2))
            .await
            .unwrap();
        drop(receiver);
        assert_eq!(budget.used(), 0);
        backend.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn csv_observer_shutdown_fences_queued_inspection_and_closes_before_wake() {
        use crate::controller::{CsvCommand, CsvReply, csv_transfer_runtime::CsvRuntime};
        use dbunk_lib::backend::csv_transfers::{
            CsvInspectionId, CsvInspectionIntent, CsvOptions, CsvTarget,
        };
        let (_profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(mailbox::WORKSPACE_QUEUE_BYTES);
        let runtime = CsvRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (wake, awakened) = async_channel::bounded(1);
        let (controls, receiver) = runtime.open(wake.clone()).unwrap();
        assert!(runtime.open(wake).is_err());
        controls.send(CsvCommand::List(1)).unwrap();
        let delivery = receiver.recv().await.unwrap();
        awakened.recv().await.unwrap();
        assert!(
            matches!(&delivery.result,Ok(CsvReply::List(inspections,jobs)) if inspections.inspections.is_empty()&&jobs.jobs.is_empty())
        );
        assert!(budget.used() >= 1024 * 1024);
        controls
            .send(CsvCommand::Inspect(
                2,
                CsvInspectionId::new(),
                "owned-test".into(),
                CsvInspectionIntent::export(
                    CsvTarget {
                        schema: "public".into(),
                        table: "owned".into(),
                    },
                    CsvOptions::default(),
                )
                .unwrap(),
            ))
            .unwrap();
        runtime.stop();
        let now = Instant::now();
        runtime
            .join(now + Duration::from_secs(1), now + Duration::from_secs(2))
            .await
            .unwrap();
        awakened
            .try_recv()
            .expect("CSV exit must close reply channel before wake");
        assert!(receiver.is_closed());
        assert!(
            backend
                .list_csv_inspections(None)
                .unwrap()
                .inspections
                .is_empty()
        );
        assert!(controls.send(CsvCommand::List(3)).is_err());
        drop(delivery);
        drop(receiver);
        assert_eq!(budget.used(), 0);
        backend.shutdown().await.unwrap();
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn csv_reply_budget_refusal_precedes_inspection_registration() {
        use crate::controller::{
            CsvCommand,
            csv_transfer_runtime::{CsvFailure, CsvRuntime},
        };
        use dbunk_lib::backend::csv_transfers::{
            CsvInspectionId, CsvInspectionIntent, CsvOptions, CsvTarget,
        };
        let (_profile, backend) = backend().await;
        let budget = mailbox::ByteBudget::new(4096);
        let runtime = CsvRuntime::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            budget.clone(),
        );
        let (controls, receiver) = runtime.open(async_channel::bounded(1).0).unwrap();
        controls
            .send(CsvCommand::Inspect(
                17,
                CsvInspectionId::new(),
                "owned-test".into(),
                CsvInspectionIntent::export(
                    CsvTarget {
                        schema: "public".into(),
                        table: "owned".into(),
                    },
                    CsvOptions::default(),
                )
                .unwrap(),
            ))
            .unwrap();
        let delivery = receiver.recv().await.unwrap();
        assert_eq!(delivery.request, 17);
        assert!(matches!(delivery.result, Err(CsvFailure::DeliveryBudget)));
        assert!(
            backend
                .list_csv_inspections(None)
                .unwrap()
                .inspections
                .is_empty()
        );
        drop(delivery);
        runtime.stop();
        let now = Instant::now();
        runtime
            .join(now + Duration::from_secs(1), now + Duration::from_secs(2))
            .await
            .unwrap();
        drop(receiver);
        assert_eq!(budget.used(), 0);
        backend.shutdown().await.unwrap();
    }
}
