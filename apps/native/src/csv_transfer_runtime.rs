//! App-owned CSV observation lane. Synchronous facade registration precedes
//! owned preparation; the lane reserves response space before any dispatch.
use super::{Worker, WorkerResult, join_worker};
use crate::mailbox::{ByteBudget, BytePermit};
use dbunk_lib::backend::{Backend, csv_transfers::*};
use std::{path::PathBuf, sync::Mutex};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};

pub enum CsvCommand {
    List(u64),
    Inspect(u64, CsvInspectionId, String, CsvInspectionIntent),
    LoadInspection(u64, CsvInspectionId),
    LoadWorkbook(u64, CsvInspectionId),
    SelectSheet(u64, CsvWorkbook, u16),
    CancelInspection(u64, CsvInspectionId),
    ReleaseInspection(u64, CsvInspectionId),
    ReviewImport(u64, CsvInspection, Vec<CsvMapping>),
    ReviewExport(u64, CsvInspection, PathBuf),
    Begin(u64, CsvTransferAttemptId, CsvTransferReview),
    ReviewTransfer(u64, CsvTransferAttemptId),
    Confirm(u64, CsvTransferConfirmation),
    Cancel(u64, CsvTransferAttemptId),
    Release(u64, CsvTransferAttemptId),
}
impl CsvCommand {
    fn request(&self) -> u64 {
        match self {
            Self::List(id)
            | Self::Inspect(id, ..)
            | Self::LoadInspection(id, ..)
            | Self::LoadWorkbook(id, ..)
            | Self::SelectSheet(id, ..)
            | Self::CancelInspection(id, ..)
            | Self::ReleaseInspection(id, ..)
            | Self::ReviewImport(id, ..)
            | Self::ReviewExport(id, ..)
            | Self::Begin(id, ..)
            | Self::ReviewTransfer(id, ..)
            | Self::Confirm(id, ..)
            | Self::Cancel(id, ..)
            | Self::Release(id, ..) => *id,
        }
    }
    fn reply_bytes(&self) -> usize {
        match self {
            Self::LoadInspection(..)
            | Self::LoadWorkbook(..)
            | Self::ReviewImport(..)
            | Self::ReviewExport(..)
            | Self::ReviewTransfer(..)
            | Self::Begin(..)
            | Self::Confirm(..) => 8 * 1024 * 1024,
            _ => 1024 * 1024,
        }
    }
    fn bytes(&self) -> Option<usize> {
        let payload = match self {
            Self::Inspect(_, _, connection, intent) => connection
                .capacity()
                .checked_add(intent.checked_heap_bytes()?)?,
            Self::ReviewImport(_, inspection, mapping) => {
                let mut bytes = inspection.retained_bytes().checked_add(
                    mapping
                        .capacity()
                        .checked_mul(std::mem::size_of::<CsvMapping>())?,
                )?;
                for column in mapping {
                    bytes = bytes.checked_add(column.target_column.capacity())?;
                }
                bytes
            }
            Self::ReviewExport(_, inspection, path) => {
                inspection.retained_bytes().checked_add(path.capacity())?
            }
            Self::Begin(_, _, review) => review.retained_bytes(),
            Self::Confirm(_, confirmation) => confirmation.retained_bytes(),
            Self::SelectSheet(_, workbook, _) => workbook.retained_bytes(),
            _ => 0,
        };
        payload.checked_add(512)
    }
}
pub enum CsvReply {
    List(CsvInspectionList, CsvTransferList),
    InspectionObservation(CsvInspectionObservation),
    Inspection(CsvInspection),
    Workbook(CsvWorkbook),
    InspectionReleased(CsvInspectionId),
    Review(CsvTransferReview),
    Submission(CsvTransferSubmission),
    Observation(CsvTransferObservation),
    Released(CsvTransferAttemptId),
}
pub enum CsvFailure {
    Backend(CsvError),
    DeliveryBudget,
}
impl std::fmt::Display for CsvFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(error) => error.fmt(f),
            Self::DeliveryBudget => {
                f.write_str("Workspace delivery budget is full; CSV request was not dispatched")
            }
        }
    }
}
pub struct CsvDelivery {
    pub request: u64,
    pub result: Result<CsvReply, CsvFailure>,
    _permit: BytePermit,
    _consumed: tokio::sync::oneshot::Sender<()>,
}
#[derive(Clone)]
pub struct CsvControls {
    commands: mpsc::Sender<(CsvCommand, BytePermit)>,
    stop: watch::Sender<bool>,
    budget: ByteBudget,
}
impl CsvControls {
    pub fn send(&self, command: CsvCommand) -> Result<(), &'static str> {
        if *self.stop.borrow() {
            return Err("CSV observer is closing");
        }
        let bytes = command.bytes().ok_or("CSV command exceeds its bounds")?;
        let permit = self
            .budget
            .reserve(bytes)
            .ok_or("Workspace delivery budget is full; CSV request was not queued")?;
        self.commands
            .try_send((command, permit))
            .map_err(|_| "A CSV request is already queued; request was not dispatched")
    }
}
struct ExitWake {
    replies: async_channel::Sender<CsvDelivery>,
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
    owner: Option<(CsvControls, Worker)>,
}
pub struct CsvRuntime {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    budget: ByteBudget,
    state: Mutex<State>,
}
impl CsvRuntime {
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
    ) -> Result<(CsvControls, async_channel::Receiver<CsvDelivery>), &'static str> {
        let mut state = self.state.lock().unwrap();
        if state.closing || state.owner.is_some() {
            return Err("CSV transfer observer is already owned or closing");
        }
        let (commands, mut requests) = mpsc::channel(1);
        let (stop, mut stopping) = watch::channel(false);
        let controls = CsvControls {
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
                let (result, permit) = match budget.reserve(command.reply_bytes()) {
                    Some(permit) => (execute(&backend, command).map_err(CsvFailure::Backend), permit),
                    None => (Err(CsvFailure::DeliveryBudget), budget.reserve(0).expect("zero-byte permit")),
                };
                if *stopping.borrow() { return Ok(()); }
                let (consumed, released) = tokio::sync::oneshot::channel();
                send.try_send(CsvDelivery { request, result, _permit: permit, _consumed: consumed }).map_err(|_| "CSV transfer observer reply unavailable".to_owned())?;
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
fn execute(backend: &Backend, command: CsvCommand) -> Result<CsvReply, CsvError> {
    match command {
        CsvCommand::List(_) => Ok(CsvReply::List(
            backend.list_csv_inspections(None)?,
            backend.list_csv_transfers(None)?,
        )),
        CsvCommand::Inspect(_, id, connection, intent) => backend
            .begin_csv_inspection(id, connection, intent)
            .map(CsvReply::InspectionObservation),
        CsvCommand::LoadInspection(_, id) => backend.csv_inspection(id).map(CsvReply::Inspection),
        CsvCommand::LoadWorkbook(_, id) => backend.csv_workbook(id).map(CsvReply::Workbook),
        CsvCommand::SelectSheet(_, workbook, index) => backend
            .select_csv_workbook_sheet(workbook, index)
            .map(CsvReply::InspectionObservation),
        CsvCommand::CancelInspection(_, id) => backend
            .cancel_csv_inspection(id)
            .map(CsvReply::InspectionObservation),
        CsvCommand::ReleaseInspection(_, id) => backend
            .release_csv_inspection(id)
            .map(|()| CsvReply::InspectionReleased(id)),
        CsvCommand::ReviewImport(_, inspection, mapping) => backend
            .review_csv_import(inspection, mapping)
            .map(CsvReply::Review),
        CsvCommand::ReviewExport(_, inspection, path) => backend
            .review_csv_export(inspection, path)
            .map(CsvReply::Review),
        CsvCommand::Begin(_, id, review) => backend
            .begin_csv_transfer(id, review)
            .map(CsvReply::Submission),
        CsvCommand::ReviewTransfer(_, id) => backend.review_csv_transfer(id).map(CsvReply::Review),
        CsvCommand::Confirm(_, confirmation) => backend
            .confirm_csv_transfer(confirmation)
            .map(CsvReply::Submission),
        CsvCommand::Cancel(_, id) => backend.cancel_csv_transfer(id).map(CsvReply::Observation),
        CsvCommand::Release(_, id) => backend
            .release_csv_transfer(id)
            .map(|()| CsvReply::Released(id)),
    }
}
