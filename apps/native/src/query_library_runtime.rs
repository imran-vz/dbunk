//! One bounded, joined SQLite worker per local-data document. Stop fences new
//! requests and joins an in-flight commit instead of detaching Backend::call.
use super::{Worker, WorkerResult, join_worker};
use crate::{
    mailbox::{ByteBudget, BytePermit},
    query_library::Rows,
};
use dbunk_lib::backend::{
    Backend, WorkspaceTool,
    query_library::{LibraryRequest, SavedQueryRecord},
    safety_audit::{SafetyAuditCursor, SafetyAuditPage},
};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};

use dbunk_lib::backend::export_configurations::{
    ExportConfigurationsCapture, ExportConfigurationsRevision, ExportOptions, ExportTarget,
};

pub enum LibraryCommand {
    LoadExportConfigurations(u64),
    SaveExportConfiguration(
        u64,
        ExportTarget,
        ExportOptions,
        ExportConfigurationsRevision,
    ),
    SafetyAudit(u64, String, Option<SafetyAuditCursor>),
    Load(WorkspaceTool, LibraryRequest),
    Save(SavedQueryRecord),
    SaveDraft(SavedQueryRecord),
    Delete(WorkspaceTool, String),
    Clear,
}
pub enum LibraryReply {
    ExportConfigurations(u64, Result<ExportConfigurationsCapture, String>),
    SafetyAudit(u64, Result<SafetyAuditPage, String>),
    Page(Rows),
    Saved(SavedQueryRecord),
    Changed,
}
pub struct LibraryDelivery {
    pub result: Result<LibraryReply, String>,
    _permit: BytePermit,
    _consumed: tokio::sync::oneshot::Sender<()>,
}
#[derive(Clone)]
pub struct LibraryControls {
    commands: mpsc::Sender<(LibraryCommand, BytePermit)>,
    budget: ByteBudget,
    stop: watch::Sender<bool>,
}
impl LibraryControls {
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
    pub fn is_closed(&self) -> bool {
        *self.stop.borrow()
    }
    pub fn send(&self, command: LibraryCommand) -> Result<(), &'static str> {
        if *self.stop.borrow() {
            return Err("Library document is closed");
        }
        let bytes = match &command {
            LibraryCommand::SaveExportConfiguration(_, target, options, _) => {
                target
                    .validate()
                    .map_err(|_| "Export target exceeds bounds")?;
                options
                    .validate()
                    .map_err(|_| "Export options exceed bounds")?;
                target.connection_id.capacity()
                    + target.schema.capacity()
                    + target.table.capacity()
                    + options.null_token.capacity()
                    + 1024
            }
            LibraryCommand::SafetyAudit(_, connection, cursor) => {
                connection.capacity()
                    + match cursor {
                        Some(cursor) => cursor
                            .checked_heap_bytes()
                            .ok_or("Audit cursor exceeds bounds")?,
                        None => 0,
                    }
                    + 512
            }
            LibraryCommand::Save(query) | LibraryCommand::SaveDraft(query) => {
                crate::results::encoded_size(query)
            }
            LibraryCommand::Load(_, request) => {
                request.search.len()
                    + request
                        .cursor
                        .as_ref()
                        .map(crate::results::encoded_size)
                        .unwrap_or(0)
                    + 512
            }
            _ => 512,
        };
        let permit = self
            .budget
            .reserve(bytes)
            .ok_or("Workspace delivery budget is full; retry after results drain")?;
        self.commands
            .try_send((command, permit))
            .map_err(|_| "A library operation is already pending")
    }
}
/// A stopped worker may have no reply to wake its document. Notify on every
/// exit so the host can observe channel closure and clear pending read state.
struct WakeOnExit {
    replies: async_channel::Sender<LibraryDelivery>,
    wake: async_channel::Sender<()>,
}
impl Drop for WakeOnExit {
    fn drop(&mut self) {
        // Publish closure before waking: the receiver must see a closed lane
        // even while the async task still owns another sender clone.
        self.replies.close();
        let _ = self.wake.try_send(());
    }
}

pub struct LibraryRuntime {
    backend: Backend,
    runtime: tokio::runtime::Handle,
    budget: ByteBudget,
    closing: AtomicBool,
    documents: Mutex<HashMap<String, (LibraryControls, Worker)>>,
}
impl LibraryRuntime {
    pub fn new(backend: Backend, runtime: tokio::runtime::Handle, budget: ByteBudget) -> Self {
        Self {
            backend,
            runtime,
            budget,
            closing: AtomicBool::new(false),
            documents: Mutex::new(HashMap::new()),
        }
    }
    pub fn open(
        &self,
        id: String,
        wake: async_channel::Sender<()>,
    ) -> Result<(LibraryControls, async_channel::Receiver<LibraryDelivery>), &'static str> {
        let mut documents = self.documents.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Library is closing");
        }
        if documents.len() >= 16 || documents.contains_key(&id) {
            return Err("Library document limit reached");
        }
        let (commands, mut requests) = mpsc::channel(1);
        let (stop, mut stopping) = watch::channel(false);
        let controls = LibraryControls {
            commands,
            stop,
            budget: self.budget.clone(),
        };
        let (send, receive) = async_channel::bounded(1);
        let backend = self.backend.clone();
        let budget = self.budget.clone();
        let worker = Worker::new(self.runtime.spawn(async move {
            let _wake_on_exit = WakeOnExit { replies: send.clone(), wake: wake.clone() };
            loop {
                let (command, _request_permit) = tokio::select! { biased; _ = stopping.changed() => return Ok(()), command = requests.recv() => match command { Some(command) => command, None => return Ok(()) } };
                if *stopping.borrow() { return Ok(()); }
                // Reserve the maximum response before SQLite creates it. One
                // outstanding reply per document holds this shared delivery lease.
                let response_bytes = if matches!(&command, LibraryCommand::SafetyAudit(..)) { 512 * 1024 } else if matches!(&command, LibraryCommand::LoadExportConfigurations(..) | LibraryCommand::SaveExportConfiguration(..)) { 1024 * 1024 } else { 8 * 1024 * 1024 };
                let (result, permit) = match budget.reserve(response_bytes) {
                    Some(permit) => (execute(&backend, command).await, permit),
                    None => (Err("Workspace delivery budget is full; retry after results drain".into()), budget.reserve(0).unwrap()),
                };
                if *stopping.borrow() { return Ok(()); }
                let (consumed, released) = tokio::sync::oneshot::channel();
                send.try_send(LibraryDelivery { result, _permit: permit, _consumed: consumed }).map_err(|_| "Library reply delivery failed")?;
                let _ = wake.try_send(());
                // Do not process a queued edit until the previous page is
                // consumed, including when a hidden tab is slow to drain.
                tokio::select! { biased; _ = stopping.changed() => return Ok(()), _ = released => {} }
            }
        }));
        documents.insert(id, (controls.clone(), worker));
        Ok((controls, receive))
    }
    pub async fn close(&self, id: &str) -> WorkerResult {
        let entry = self.documents.lock().unwrap().get(id).cloned();
        if let Some((controls, worker)) = entry {
            controls.stop.send_replace(true);
            worker
                .join
                .clone()
                .await
                .map_err(|error| error.to_string())?;
            self.documents.lock().unwrap().remove(id);
        }
        Ok(())
    }
    pub fn stop(&self) {
        let documents = self.documents.lock().unwrap();
        self.closing.store(true, Ordering::Release);
        for (controls, _) in documents.values() {
            controls.stop.send_replace(true);
        }
    }
    pub async fn join(&self, grace: Instant, deadline: Instant) -> WorkerResult {
        let workers = self
            .documents
            .lock()
            .unwrap()
            .values()
            .map(|(_, worker)| worker.clone())
            .collect::<Vec<_>>();
        for worker in workers {
            join_worker(&worker, grace, deadline, false).await?;
        }
        self.documents.lock().unwrap().clear();
        Ok(())
    }
}
async fn execute(backend: &Backend, command: LibraryCommand) -> Result<LibraryReply, String> {
    let result = match command {
        LibraryCommand::LoadExportConfigurations(id) => {
            return Ok(LibraryReply::ExportConfigurations(
                id,
                backend
                    .load_export_configurations()
                    .await
                    .map_err(|e| e.to_string()),
            ));
        }
        LibraryCommand::SaveExportConfiguration(id, target, options, revision) => {
            return Ok(LibraryReply::ExportConfigurations(
                id,
                backend
                    .save_export_configuration(target, options, revision)
                    .await
                    .map_err(|e| e.to_string()),
            ));
        }
        LibraryCommand::SafetyAudit(id, connection, cursor) => {
            return Ok(LibraryReply::SafetyAudit(
                id,
                backend
                    .load_safety_audit(connection, cursor)
                    .await
                    .map_err(|error| error.to_string()),
            ));
        }
        LibraryCommand::Load(
            WorkspaceTool::Objects
            | WorkspaceTool::Administration
            | WorkspaceTool::BackupRestore
            | WorkspaceTool::TableCopy
            | WorkspaceTool::TableSeed
            | WorkspaceTool::CsvTransfer
            | WorkspaceTool::SchemaCompare
            | WorkspaceTool::SchemaMap,
            _,
        )
        | LibraryCommand::Delete(
            WorkspaceTool::Objects
            | WorkspaceTool::Administration
            | WorkspaceTool::BackupRestore
            | WorkspaceTool::TableCopy
            | WorkspaceTool::TableSeed
            | WorkspaceTool::CsvTransfer
            | WorkspaceTool::SchemaCompare
            | WorkspaceTool::SchemaMap,
            _,
        ) => {
            return Err("Database tools require a database document".into());
        }
        LibraryCommand::Load(WorkspaceTool::History, request) => backend
            .load_query_history(request)
            .await
            .map(|page| LibraryReply::Page(Rows::History(page))),
        LibraryCommand::Load(WorkspaceTool::SavedQueries, request) => backend
            .load_saved_queries(request)
            .await
            .map(|page| LibraryReply::Page(Rows::Saved(page))),
        LibraryCommand::SaveDraft(query) => backend
            .save_query_draft(query)
            .await
            .map(LibraryReply::Saved),
        LibraryCommand::Save(query) => backend
            .save_saved_query(query)
            .await
            .map(LibraryReply::Saved),
        LibraryCommand::Delete(WorkspaceTool::History, id) => backend
            .delete_query_history(id)
            .await
            .map(|_| LibraryReply::Changed),
        LibraryCommand::Delete(WorkspaceTool::SavedQueries, id) => backend
            .delete_saved_query(id)
            .await
            .map(|_| LibraryReply::Changed),
        LibraryCommand::Clear => backend
            .clear_query_history()
            .await
            .map(|_| LibraryReply::Changed),
    };
    result.map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_wake_observes_closed_replies_even_while_another_sender_survives() {
        let (send, receive) = async_channel::bounded(1);
        let (wake, awakened) = async_channel::bounded(1);
        drop(WakeOnExit {
            replies: send.clone(),
            wake,
        });
        awakened.try_recv().expect("document must be notified");
        assert!(receive.is_closed());
        assert!(send.is_closed());
        assert!(matches!(
            receive.try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
    }
}
