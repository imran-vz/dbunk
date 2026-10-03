//! Completion metadata shares the query-edit data worker. Typing never cancels
//! that worker, because it may own an independently reviewed mutation.
use super::*;
use crate::{
    controller::TableCommand,
    sql_completion::{self, CompletionHandle, MetadataRequest},
};

impl Workbench {
    pub(super) fn install_completion(&mut self, cx: &mut Context<Self>) {
        let (Some(document), Some(budget)) = (&self.document, &self.retained_budget) else {
            return;
        };
        let handle = CompletionHandle::new(budget.clone(), document.wake.clone());
        handle.bind(document.connection_id.clone());
        self.editor.update(cx, |editor, _| {
            editor.set_completion_provider(Some(handle.provider()))
        });
        self.completion = Some(handle);
    }
    pub(super) fn completion_pending(&self, cx: &gpui::App) -> bool {
        self.connected
            && !self.closing
            && self.completion_inflight.is_none()
            && self
                .completion
                .as_ref()
                .is_some_and(|completion| completion.pending_request().is_some())
            && !self
                .query_changes
                .view
                .as_ref()
                .is_some_and(|view| view.read(cx).navigation_blocked())
            && (self.query_changes.controls.is_none() || self.query_changes.ready)
    }
    pub(super) fn drain_completion(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.completion_pending(cx) {
            return false;
        }
        if self.query_changes.controls.is_none() {
            self.connect_query_changes(cx);
            if self.query_changes.controls.is_none()
                && let Some(handle) = &self.completion
                && let Some(request) = handle.pending_request()
            {
                handle.fail(
                    request.id(),
                    "Metadata reader could not open; reconnect and retry",
                );
            }
            return true;
        }
        let Some(handle) = &self.completion else {
            return false;
        };
        let Some(request) = handle.pending_request() else {
            return false;
        };
        let id = request.id();
        let command = match request {
            MetadataRequest::Catalog { id, .. } => TableCommand::Catalog(id),
            MetadataRequest::Columns {
                id,
                schema,
                relation,
                ..
            } => TableCommand::CompletionColumns(id, schema, relation),
        };
        match self.query_changes.controls.as_ref().unwrap().send(command) {
            Ok(()) => {
                handle.mark_dispatched(id);
                self.completion_inflight = Some(id);
            }
            Err(error) => handle.fail(id, error),
        }
        cx.notify();
        true
    }
    /// A retired metadata receiver cannot revive its old cache or menu. The SQL
    /// connection itself may stay usable when only result captures were cleared.
    pub(super) fn reset_completion(&mut self, connected: bool, cx: &mut Context<Self>) {
        self.completion_inflight = None;
        if let Some(handle) = &self.completion {
            handle.bind(
                self.document
                    .as_ref()
                    .and_then(|document| document.connection_id.clone()),
            );
            handle.set_connected(connected && !self.closing);
        }
        self.completion_reset = true;
        // Zed may still be filtering an already-produced response. Until the
        // window can drop that task, its mouse/AX confirm path must not edit.
        self.editor.update(cx, |editor, cx| {
            editor.set_read_only(true);
            sql_completion::clear_menu(editor, cx);
        });
    }
    pub(super) fn sync_completion_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        });
        if let Some(handle) = &self.completion {
            handle.set_composing(composing);
        }
        let composition_started = composing && !self.completion_composing;
        self.completion_composing = composing;
        if self.completion_reset || composition_started {
            let reset = self.completion_reset;
            self.completion_reset = false;
            self.editor.update(cx, |editor, cx| {
                sql_completion::dismiss(editor, window, cx);
                if reset {
                    editor.set_read_only(self.closing);
                }
            });
        }
    }
    pub(super) fn render_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_completion_input(window, cx);
        if self
            .completion
            .as_ref()
            .is_some_and(CompletionHandle::take_refresh)
        {
            self.editor
                .update(cx, |editor, cx| sql_completion::show(editor, window, cx));
        }
        self.drain_completion(cx);
    }
    pub(super) fn refresh_completion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.render_completion(window, cx);
        if !self.closing {
            if let Some(handle) = &self.completion {
                handle.refresh();
            }
            self.drain_completion(cx);
        }
        cx.notify();
    }
}
