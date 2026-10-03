//! Bounded comparison lanes retain response ownership until actual consumption.
use super::{Worker, WorkerResult, join_worker};
use crate::{
    mailbox::{ByteBudget, BytePermit},
    schema_compare_model::{Dispatch, ReadToken},
};
use dbunk_lib::backend::{Backend, schema_comparisons::*};
use std::{collections::HashMap, sync::Mutex};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};

pub enum CompareCommand {
    List(u64),
    Start(u64, SchemaComparisonStart),
    Cancel(u64, String),
    Release(u64, String),
    Read(Dispatch),
}
impl CompareCommand {
    fn identity(&self) -> (u64, Option<ReadToken>) {
        match self {
            Self::List(id) | Self::Start(id, _) | Self::Cancel(id, _) | Self::Release(id, _) => {
                (*id, None)
            }
            Self::Read(dispatch) => (0, Some(dispatch.token)),
        }
    }
    fn bytes(&self) -> Option<usize> {
        let payload = match self {
            Self::Start(_, start) => start.checked_heap_bytes()?,
            Self::Cancel(_, id) | Self::Release(_, id) => {
                if id.is_empty() || id.len() > 128 || id.capacity() > 512 {
                    return None;
                }
                id.capacity()
            }
            Self::Read(dispatch) => {
                checked_schema_comparison_read_bytes(&dispatch.request, &dispatch.read)?
            }
            Self::List(_) => 0,
        };
        (payload <= 16 * 1024).then(|| payload + 512)
    }
}
pub enum CompareReply {
    List(SchemaComparisonList),
    Started(Status),
    Cancelled,
    Released,
    Page(Box<SchemaComparisonResponse>),
}
pub enum CompareFailure {
    Backend(CompareError),
    DeliveryBudget,
}
impl std::fmt::Display for CompareFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(error) => f.write_str(&crate::schema_compare_model::failure_text(error)),
            Self::DeliveryBudget => f.write_str(
                "Workspace delivery budget is full; comparison request was not dispatched",
            ),
        }
    }
}
pub struct CompareDelivery {
    pub request: u64,
    pub token: Option<ReadToken>,
    pub result: Result<CompareReply, CompareFailure>,
    _permit: BytePermit,
    _consumed: tokio::sync::oneshot::Sender<()>,
}
#[derive(Clone)]
pub struct CompareControls {
    commands: mpsc::Sender<(CompareCommand, BytePermit)>,
    stop: watch::Sender<bool>,
    budget: ByteBudget,
    reader: bool,
}
impl CompareControls {
    pub fn send(&self, command: CompareCommand) -> Result<(), &'static str> {
        if *self.stop.borrow() {
            return Err("Comparison lane is closing");
        }
        if matches!(command, CompareCommand::Read(_)) != self.reader {
            return Err("Wrong comparison lane");
        }
        let permit = self
            .budget
            .reserve(command.bytes().ok_or("Comparison command exceeds bounds")?)
            .ok_or("Workspace delivery budget is full; comparison request was not queued")?;
        self.commands
            .try_send((command, permit))
            .map_err(|_| "A comparison request is already queued; request was not dispatched")
    }
}
/// The UI has one receiver owner. Its destruction signals the worker, which
/// destroys queued responses before joining the backend reader's permits.
pub struct CompareReceiver {
    receive: async_channel::Receiver<CompareDelivery>,
    stop: watch::Sender<bool>,
}
impl CompareReceiver {
    pub fn try_recv(&self) -> Result<CompareDelivery, async_channel::TryRecvError> {
        self.receive.try_recv()
    }
    pub fn is_closed(&self) -> bool {
        self.receive.is_closed()
    }
    pub fn is_empty(&self) -> bool {
        self.receive.is_empty()
    }
}
impl Drop for CompareReceiver {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}
struct ExitWake {
    receive: async_channel::Receiver<CompareDelivery>,
    wake: async_channel::Sender<()>,
}
impl ExitWake {
    fn drain(&self) {
        self.receive.close();
        while self.receive.try_recv().is_ok() {}
    }
}
impl Drop for ExitWake {
    fn drop(&mut self) {
        self.drain();
        let _ = self.wake.try_send(());
    }
}
struct State {
    closing: bool,
    lanes: HashMap<String, (CompareControls, Worker)>,
}
pub struct CompareRuntime {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    budget: ByteBudget,
    owner: String,
    state: Mutex<State>,
}
impl CompareRuntime {
    pub fn new(
        backend: Backend,
        runtime: tokio::runtime::Handle,
        budget: ByteBudget,
        owner: String,
    ) -> Self {
        Self {
            backend,
            runtime,
            budget,
            owner,
            state: Mutex::new(State {
                closing: false,
                lanes: HashMap::new(),
            }),
        }
    }
    pub fn open(
        &self,
        tab: Option<String>,
        wake: async_channel::Sender<()>,
    ) -> Result<(CompareControls, CompareReceiver), &'static str> {
        let reader = tab.is_some();
        let key = tab.clone().unwrap_or_default();
        if reader && uuid::Uuid::parse_str(&key).is_err() {
            return Err("Comparison tab identity is invalid");
        }
        let mut state = self.state.lock().unwrap();
        if state.closing
            || state.lanes.contains_key(&key)
            || (reader && state.lanes.keys().filter(|key| !key.is_empty()).count() >= 16)
        {
            return Err("Comparison lane is already owned, full or closing");
        }
        let (commands, mut requests) = mpsc::channel(1);
        let (stop, mut stopping) = watch::channel(false);
        let controls = CompareControls {
            commands,
            stop: stop.clone(),
            budget: self.budget.clone(),
            reader,
        };
        let (send, receive) = async_channel::bounded(1);
        let exit = ExitWake {
            receive: receive.clone(),
            wake: wake.clone(),
        };
        let (backend, budget, owner) = (
            self.backend.clone(),
            self.budget.clone(),
            self.owner.clone(),
        );
        let worker = Worker::new(self.runtime.spawn(async move {
            let mut held: Option<SchemaComparisonReader> = None;
            let result: WorkerResult = async {
                loop {
                    let (command, _command_permit) = tokio::select! {
                        biased;
                        _ = stopping.changed() => break,
                        command = requests.recv() => match command { Some(command) => command, None => break },
                    };
                    if *stopping.borrow() { break; }
                    let (request, token) = command.identity();
                    let bytes = if reader { COMPARISON_RESPONSE_RESERVATION } else { 128 * 1024 };
                    let (result, permit) = match budget.reserve(bytes) {
                        Some(permit) => (execute(&backend, &owner, tab.as_deref(), &mut held, command).await.map_err(CompareFailure::Backend), permit),
                        None => (Err(CompareFailure::DeliveryBudget), budget.reserve(0).expect("zero-byte permit")),
                    };
                    if *stopping.borrow() { drop(result); break; }
                    let (consumed, released) = tokio::sync::oneshot::channel();
                    send.try_send(CompareDelivery { request, token, result, _permit: permit, _consumed: consumed })
                        .map_err(|_| "Comparison delivery unavailable".to_owned())?;
                    let _ = wake.try_send(());
                    tokio::select! { biased; _ = stopping.changed() => break, _ = released => {} }
                }
                Ok(())
            }.await;
            // Closing a channel alone does not drop its queued opaque response.
            exit.drain();
            let closed = match held { Some(reader) => backend.close_schema_comparison_reader(&reader).await.map_err(|error| format!("Comparison reader cleanup failed: {error:?}")), None => Ok(()) };
            result.and(closed)
        }));
        state.lanes.insert(key, (controls.clone(), worker));
        Ok((controls, CompareReceiver { receive, stop }))
    }
    pub async fn close(&self, tab: &str) -> WorkerResult {
        let worker = self
            .state
            .lock()
            .unwrap()
            .lanes
            .get(tab)
            .map(|(controls, worker)| {
                controls.stop.send_replace(true);
                worker.clone()
            });
        if let Some(worker) = worker {
            let now = Instant::now();
            join_worker(&worker, now + super::GRACE, now + super::TOTAL, false).await?;
            self.state.lock().unwrap().lanes.remove(tab);
        }
        Ok(())
    }
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.closing = true;
        for (controls, _) in state.lanes.values() {
            controls.stop.send_replace(true);
        }
    }
    pub async fn join(&self, grace: Instant, deadline: Instant) -> WorkerResult {
        let workers: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .lanes
            .values()
            .map(|(_, worker)| worker.clone())
            .collect();
        let results = futures_util::future::join_all(
            workers
                .iter()
                .map(|worker| join_worker(worker, grace, deadline, false)),
        )
        .await;
        results.into_iter().collect::<Result<Vec<_>, _>>()?;
        self.state.lock().unwrap().lanes.clear();
        Ok(())
    }
}
async fn execute(
    backend: &Backend,
    owner: &str,
    tab: Option<&str>,
    held: &mut Option<SchemaComparisonReader>,
    command: CompareCommand,
) -> Result<CompareReply, CompareError> {
    match command {
        CompareCommand::List(_) => backend.list_schema_comparisons().map(CompareReply::List),
        CompareCommand::Start(_, start) => backend
            .begin_schema_comparison(start)
            .map(CompareReply::Started),
        CompareCommand::Cancel(_, id) => backend
            .cancel_schema_comparison(&id)
            .map(|_| CompareReply::Cancelled),
        CompareCommand::Release(_, id) => backend
            .release_schema_comparison(&id)
            .map(|()| CompareReply::Released),
        CompareCommand::Read(dispatch) => {
            if held
                .as_ref()
                .is_some_and(|reader| reader.request() != &dispatch.request)
                && let Some(reader) = held.take()
            {
                backend.close_schema_comparison_reader(&reader).await?;
            }
            if held.is_none() {
                *held = Some(
                    backend
                        .open_schema_comparison_reader(
                            owner,
                            tab.ok_or(CompareError::InvalidRequest)?,
                            dispatch.request,
                        )
                        .await?,
                );
            }
            backend
                .read_schema_comparison(held.as_ref().expect("reader opened"), dispatch.read)
                .await
                .map(|response| CompareReply::Page(Box::new(response)))
        }
    }
}

#[cfg(test)]
#[path = "schema_compare_runtime/tests.rs"]
mod tests;
