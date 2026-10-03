//! Query mutation ownership is independent of the mutable SQL editor and ACK
//! lifetime. A separate data lane uses the same document ID for joined cleanup.
use super::*;
use crate::{
    controller::{TableControls, TableMessage, TableReceiver},
    query_result::{ExecutedSource, Provenance, QueryRows},
    table_changes::{ChangesEvent, TableChanges},
};
use std::rc::Rc;

#[derive(Default)]
pub(super) struct QueryChanges {
    pub(super) source: Option<Rc<ExecutedSource>>,
    pub(super) unavailable: Option<String>,
    pub(super) view: Option<Entity<TableChanges>>,
    events: Option<Subscription>,
    observer: Option<Subscription>,
    pub(super) controls: Option<TableControls>,
    pub(super) receiver: Option<TableReceiver>,
    pub(super) ready: bool,
    pub(super) turn: bool,
}
impl QueryChanges {
    pub(super) fn pending(&self) -> bool {
        self.receiver
            .as_ref()
            .is_some_and(TableReceiver::has_pending)
    }
}
impl Workbench {
    pub fn query_changes_blocked(&self, cx: &gpui::App) -> bool {
        self.query_changes.view.as_ref().is_some_and(|view| {
            let changes = view.read(cx);
            changes.has_intent() || changes.navigation_blocked()
        })
    }
    pub fn query_changes_snapshot(&self, cx: &gpui::App) -> Option<backend::WorkspaceQueryChanges> {
        self.query_changes
            .view
            .as_ref()
            .and_then(|view| view.read(cx).query_snapshot())
    }
    pub fn query_changes_bytes(&self, cx: &gpui::App) -> usize {
        self.query_changes
            .view
            .as_ref()
            .map_or(0, |view| view.read(cx).query_snapshot_bytes())
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if let Some(view) = &self.query_changes.view {
            view.update(cx, |view, cx| view.apply_saved(id, result, cx));
        }
    }
    pub(super) fn install_changes(&mut self, view: Entity<TableChanges>, cx: &mut Context<Self>) {
        self.query_changes.events = Some(cx.subscribe(&view, |this, _, event, cx| {
            match event {
                ChangesEvent::Changed => cx.emit(WorkbenchEvent::DraftChanged),
                ChangesEvent::PersistApply(id) => cx.emit(WorkbenchEvent::PersistApply(*id)),
                ChangesEvent::Applied => {
                    this.release_query_rows(cx);
                    this.grid.update(cx, |grid, cx| grid.begin(cx));
                    this.account_retained(cx);
                    this.status = "Changes committed separately; results invalidated. Run SQL explicitly to refresh".into();
                }
                ChangesEvent::FocusGrid(_) => this.query_changes_return_focus = true,
                ChangesEvent::KeyChanged => {}
            }
            cx.notify();
        }));
        self.query_changes.observer = Some(cx.observe(&view, |_, _, cx| cx.notify()));
        if self.query_changes.ready
            && let Some(controls) = &self.query_changes.controls
        {
            view.update(cx, |view, cx| view.query_connected(controls.clone(), cx));
        }
        self.query_changes.view = Some(view);
    }
    pub(super) fn restore_query_changes(
        &mut self,
        saved: backend::WorkspaceQueryChanges,
        cx: &mut Context<Self>,
    ) {
        let budget = self.retained_budget.clone().expect("workspace budget");
        let provenance = Provenance::restore(saved.source, budget.clone());
        let view = cx.new(|_| TableChanges::new_query(provenance, None, Some(saved.draft), budget));
        self.install_changes(view, cx);
    }
    pub(super) fn release_query_rows(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.query_changes.view {
            view.update(cx, |view, cx| view.clear_query_rows(cx));
        }
        self.query_changes.source = None;
        self.query_changes.unavailable = None;
    }
    pub(super) fn retire_query_changes(&mut self, cx: &mut Context<Self>) {
        self.reset_completion(self.connected, cx);
        self.release_query_rows(cx);
        if let Some(controls) = self.query_changes.controls.take() {
            controls.stop();
        }
        self.query_changes.receiver = None;
        self.query_changes.ready = false;
        self.query_changes.events = None;
        self.query_changes.observer = None;
        self.query_changes.view = None;
    }
    pub(super) fn disconnect_query_changes(&mut self, cx: &mut Context<Self>) {
        self.reset_completion(false, cx);
        self.release_query_rows(cx);
        if let Some(view) = &self.query_changes.view {
            view.update(cx, |view, cx| view.disconnected(cx));
        }
        if let Some(controls) = self.query_changes.controls.take() {
            controls.stop();
        }
        self.query_changes.receiver = None;
        self.query_changes.ready = false;
    }
    pub(super) fn connect_query_changes(&mut self, cx: &mut Context<Self>) {
        if (self.query_changes.view.is_none()
            && !self
                .completion
                .as_ref()
                .is_some_and(|handle| handle.pending_request().is_some()))
            || self.query_changes.receiver.is_some()
            || !self.connected
        {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let Some(connection) = document.connection_id.clone() else {
            return;
        };
        match self
            .host
            .open_table_document(document.id.clone(), connection, document.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.query_changes.controls = Some(controls);
                self.query_changes.receiver = Some(receiver);
            }
            Err(error) => {
                self.status = format!("Result editing: {error}; retry Edit selected cell")
            }
        }
        cx.notify();
    }
    pub(super) fn drain_query_changes(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(message) = self
            .query_changes
            .receiver
            .as_ref()
            .and_then(TableReceiver::try_recv)
        else {
            return false;
        };
        match message.into_message() {
            TableMessage::Catalog(id, result) => {
                if self.completion_inflight == Some(id) {
                    self.completion_inflight = None;
                    if let Some(handle) = &self.completion {
                        handle.accept_catalog(id, result.map_err(|error| format!("{error:?}")));
                    }
                }
            }
            TableMessage::CompletionColumns(id, result) => {
                if self.completion_inflight == Some(id) {
                    self.completion_inflight = None;
                    if let Some(handle) = &self.completion {
                        handle.accept_columns(id, result.map_err(|error| format!("{error:?}")));
                    }
                }
            }
            TableMessage::Opened => {
                self.query_changes.ready = true;
                if let (Some(view), Some(controls)) =
                    (&self.query_changes.view, &self.query_changes.controls)
                {
                    view.update(cx, |view, cx| view.query_connected(controls.clone(), cx));
                }
            }
            TableMessage::Closed(result) => {
                self.disconnect_query_changes(cx);
                self.status = format!("Result editing disconnected: {result:?}");
            }
            TableMessage::Error(error) => self.status = format!("Result editing failed: {error}"),
            message => {
                if let Some(view) = &self.query_changes.view {
                    view.update(cx, |view, cx| view.consume(message, cx));
                }
            }
        }
        cx.notify();
        true
    }
    pub(super) fn can_edit_query_cell(&self, cx: &gpui::App) -> bool {
        self.document.is_some()
            && self.connected
            && !self.closing
            && self.execution.is_none()
            && self.grid.read(cx).model().active == 0
            && self.grid.read(cx).selected_cell().is_some()
            && (self.query_changes.source.is_some() || self.query_changes.view.is_some())
    }
    pub(super) fn edit_query_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit_query_cell(cx) {
            return;
        }
        if self.query_changes.view.is_none() {
            let Some(source) = self.query_changes.source.clone() else {
                return;
            };
            let connection = self
                .document
                .as_ref()
                .and_then(|d| d.connection_id.as_deref())
                .unwrap_or("");
            if source.connection != connection || source.session != self.session {
                return;
            }
            let budget = self.retained_budget.clone().expect("workspace budget");
            let rows = match QueryRows::capture(
                source.clone(),
                self.grid.read(cx).model(),
                budget.clone(),
            ) {
                Ok(rows) => rows,
                Err(error) => {
                    self.status = error.into();
                    cx.notify();
                    return;
                }
            };
            let view = cx.new(|_| {
                TableChanges::new_query(source.provenance.clone(), Some(rows), None, budget)
            });
            self.install_changes(view, cx);
            self.connect_query_changes(cx);
            self.status =
                "Checking result identity. Choose Edit selected cell again after analysis".into();
        } else if self.query_changes.receiver.is_none() {
            self.connect_query_changes(cx);
        } else if let Some((row, column)) = self.grid.read(cx).selected_cell() {
            self.query_changes
                .view
                .as_ref()
                .unwrap()
                .update(cx, |view, cx| view.edit_cell(row, column, window, cx));
        }
        cx.notify();
    }
}
