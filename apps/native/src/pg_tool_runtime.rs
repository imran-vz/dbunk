//! One app-owned command lane for PostgreSQL file jobs. It outlives setup tabs;
//! Backend owns preparation/processes and shutdown joins them separately.
use super::{Worker, WorkerResult, join_worker};
use crate::mailbox::{ByteBudget, BytePermit};
use dbunk_lib::backend::{
    Backend,
    pg_tools::{
        PgToolAttemptId, PgToolConfirmation, PgToolError, PgToolIntent, PgToolJobList,
        PgToolObservation, PgToolReview, PgToolSubmission,
    },
};
use std::sync::Mutex;
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};

pub enum ToolCommand {
    List(u64),
    Begin(u64, PgToolAttemptId, String, PgToolIntent),
    Review(u64, PgToolAttemptId),
    Start(u64, PgToolReview),
    Confirm(u64, PgToolConfirmation),
    Cancel(u64, PgToolAttemptId),
    Release(u64, PgToolAttemptId),
}
impl ToolCommand {
    fn request(&self) -> u64 {
        match self {
            Self::List(id)
            | Self::Begin(id, ..)
            | Self::Review(id, ..)
            | Self::Start(id, ..)
            | Self::Confirm(id, ..)
            | Self::Cancel(id, ..)
            | Self::Release(id, ..) => *id,
        }
    }
    fn bytes(&self) -> Option<usize> {
        let payload = match self {
            Self::Begin(_, _, connection, intent) => connection
                .capacity()
                .checked_add(intent.checked_heap_bytes()?)?,
            Self::Start(_, review) => review.retained_bytes(),
            Self::Confirm(_, confirmation) => confirmation.retained_bytes(),
            _ => 0,
        };
        payload.checked_add(512)
    }
}
pub enum ToolReply {
    List(PgToolJobList),
    Observation(PgToolObservation),
    Review(PgToolReview),
    Submission(PgToolSubmission),
    Released(PgToolAttemptId),
}
pub enum ToolFailure {
    Backend(PgToolError),
    DeliveryBudget,
}
impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(error) => error.fmt(f),
            Self::DeliveryBudget => {
                f.write_str("Workspace delivery budget is full; request was not dispatched")
            }
        }
    }
}
pub struct ToolDelivery {
    pub request: u64,
    pub result: Result<ToolReply, ToolFailure>,
    _permit: BytePermit,
    _consumed: tokio::sync::oneshot::Sender<()>,
}
#[derive(Clone)]
pub struct ToolControls {
    commands: mpsc::Sender<(ToolCommand, BytePermit)>,
    stop: watch::Sender<bool>,
    budget: ByteBudget,
}
impl ToolControls {
    pub fn send(&self, command: ToolCommand) -> Result<(), &'static str> {
        if *self.stop.borrow() {
            return Err("File job observer is closing");
        }
        let bytes = command
            .bytes()
            .ok_or("File job command exceeds its bounds")?;
        let permit = self
            .budget
            .reserve(bytes)
            .ok_or("Workspace delivery budget is full; request was not queued")?;
        self.commands
            .try_send((command, permit))
            .map_err(|_| "A file job command is already queued; request was not dispatched")
    }
}
struct ExitWake {
    replies: async_channel::Sender<ToolDelivery>,
    wake: async_channel::Sender<()>,
}
impl Drop for ExitWake {
    fn drop(&mut self) {
        self.replies.close();
        let _ = self.wake.try_send(());
    }
}
struct State {
    closing: bool,
    owner: Option<(ToolControls, Worker)>,
}
pub struct PgToolRuntime {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    budget: ByteBudget,
    state: Mutex<State>,
}
impl PgToolRuntime {
    pub fn new(backend: Backend, runtime: tokio::runtime::Handle, budget: ByteBudget) -> Self {
        Self {
            backend,
            runtime,
            budget,
            state: Mutex::new(State {
                closing: false,
                owner: None,
            }),
        }
    }
    /// The workspace is the only consumer. Setup views share its retained state.
    pub fn open(
        &self,
        wake: async_channel::Sender<()>,
    ) -> Result<(ToolControls, async_channel::Receiver<ToolDelivery>), &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.closing || state.owner.is_some() {
            return Err("File job observer is already owned or closing");
        }
        let (commands, mut requests) = mpsc::channel(1);
        let (stop, mut stopping) = watch::channel(false);
        let controls = ToolControls {
            commands,
            stop,
            budget: self.budget.clone(),
        };
        let (send, receive) = async_channel::bounded(1);
        let backend = self.backend.clone();
        let budget = self.budget.clone();
        let worker = Worker::new(self.runtime.spawn(async move {
            let _exit = ExitWake { replies: send.clone(), wake: wake.clone() };
            loop {
                let (command, _command_permit) = tokio::select! {
                    biased;
                    _ = stopping.changed() => return Ok(()),
                    command = requests.recv() => match command { Some(command) => command, None => return Ok(()) },
                };
                if *stopping.borrow() { return Ok(()); }
                let request = command.request();
                // Reserve before the synchronous facade snapshots or registers
                // any work. These calls schedule owned preparation; they do not
                // await a process or file copy on this observer lane.
                let (result, permit) = match budget.reserve(512 * 1024) {
                    Some(permit) => (execute(&backend, command).map_err(ToolFailure::Backend), permit),
                    None => (Err(ToolFailure::DeliveryBudget), budget.reserve(0).expect("zero-byte permit")),
                };
                if *stopping.borrow() { return Ok(()); }
                let (consumed, released) = tokio::sync::oneshot::channel();
                send.try_send(ToolDelivery { request, result, _permit: permit, _consumed: consumed }).map_err(|_| "File job observer reply unavailable".to_owned())?;
                let _ = wake.try_send(());
                tokio::select! { biased; _ = stopping.changed() => return Ok(()), _ = released => {} }
            }
        }));
        state.owner = Some((controls.clone(), worker));
        Ok((controls, receive))
    }
    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.closing = true;
        if let Some((controls, _)) = &state.owner {
            controls.stop.send_replace(true);
        }
    }
    pub async fn join(&self, grace: Instant, deadline: Instant) -> WorkerResult {
        let worker = self
            .state
            .lock()
            .unwrap()
            .owner
            .as_ref()
            .map(|(_, worker)| worker.clone());
        if let Some(worker) = worker {
            join_worker(&worker, grace, deadline, false).await?;
        }
        self.state.lock().unwrap().owner = None;
        Ok(())
    }
}
fn execute(backend: &Backend, command: ToolCommand) -> Result<ToolReply, PgToolError> {
    match command {
        ToolCommand::List(_) => backend.list_pg_tool_jobs(None).map(ToolReply::List),
        ToolCommand::Begin(_, attempt, connection, intent) => backend
            .begin_pg_tool_job(attempt, connection, intent)
            .map(ToolReply::Observation),
        ToolCommand::Review(_, attempt) => {
            backend.review_pg_tool_job(attempt).map(ToolReply::Review)
        }
        ToolCommand::Start(_, review) => {
            backend.start_pg_tool_job(review).map(ToolReply::Submission)
        }
        ToolCommand::Confirm(_, confirmation) => backend
            .confirm_pg_tool_job(confirmation)
            .map(ToolReply::Submission),
        ToolCommand::Cancel(_, attempt) => backend
            .cancel_pg_tool_job(attempt)
            .map(ToolReply::Observation),
        ToolCommand::Release(_, attempt) => backend
            .release_pg_tool_job(attempt)
            .map(|()| ToolReply::Released(attempt)),
    }
}
