//! Read-only DDL capture, bounded preview and explicit new-file publication.
use crate::{
    accessible_editor::AccessibleEditor,
    bounded_field::Field,
    controller::Host,
    ddl_export_model::{Capture, Lease, Scope, Section},
};
use dbunk_lib::backend::{
    ddl_export::{DdlExportArtifact, DdlExportRequest},
    result_files as files,
};
use editor::Editor;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, Subscription,
    Window, div, prelude::*,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
mod actions;
mod render;
mod save;
#[cfg(test)]
mod tests;
gpui::actions!(ddl_export, [NextControl, PreviousControl]);
pub enum DdlExportEvent {
    Back,
    Connect,
    Refresh(DdlExportRequest),
    Cancel,
}
pub struct DdlExportRuntime<'a> {
    pub ready: bool,
    pub busy: bool,
    pub can_cancel: bool,
    pub editable: bool,
    pub capture_current: bool,
    pub status: &'a str,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Back,
    Connect,
    Refresh,
    Cancel,
    Clear,
    Save,
    CancelFile,
    Previous,
    Next,
    Scope(usize),
    Section(usize),
}
const ACTIONS: [(Action, &str); 9] = [
    (Action::Back, "Objects"),
    (Action::Connect, "Connect"),
    (Action::Refresh, "Capture DDL"),
    (Action::Cancel, "Cancel read"),
    (Action::Clear, "Clear capture / reset identity"),
    (Action::Save, "Save full SQL to new file"),
    (Action::CancelFile, "Cancel file save"),
    (Action::Previous, "Previous preview page"),
    (Action::Next, "Next preview page"),
];
struct SelectedEditor {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
pub struct DdlExportView {
    host: Arc<Host>,
    connection: String,
    budget: Rc<Cell<usize>>,
    capture: Option<Capture>,
    editor: Option<SelectedEditor>,
    revision: u64,
    scope: Scope,
    section: Section,
    page: usize,
    fields: Vec<Entity<Field>>,
    field_events: Vec<Subscription>,
    field_lease: Option<Rc<Lease>>,
    root: FocusHandle,
    buttons: Vec<FocusHandle>,
    scopes: Vec<FocusHandle>,
    sections: Vec<FocusHandle>,
    ready: bool,
    busy: bool,
    can_cancel: bool,
    editable: bool,
    capture_current: bool,
    status: String,
    message: Option<String>,
    file_busy: bool,
    cancellation: Option<files::Cancellation>,
}
impl EventEmitter<DdlExportEvent> for DdlExportView {}
impl DdlExportView {
    pub fn new(
        host: Arc<Host>,
        connection: String,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            host,
            connection,
            budget,
            capture: None,
            editor: None,
            revision: 0,
            scope: Scope::Database,
            section: Section::Metadata,
            page: 0,
            fields: vec![],
            field_events: vec![],
            field_lease: None,
            root: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            scopes: Scope::ALL.iter().map(|_| cx.focus_handle()).collect(),
            sections: Section::ALL.iter().map(|_| cx.focus_handle()).collect(),
            ready: false,
            busy: false,
            can_cancel: false,
            editable: true,
            capture_current: false,
            status: String::new(),
            message: None,
            file_busy: false,
            cancellation: None,
        };
        view.ensure_fields(window, cx);
        view
    }
    pub fn focus(&self, cx: &gpui::App) -> FocusHandle {
        if let Some(editor) = &self.editor {
            editor.editor.focus_handle(cx)
        } else if self.ready {
            self.buttons[2].clone()
        } else {
            self.buttons[1].clone()
        }
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn sync(&mut self, runtime: DdlExportRuntime<'_>, cx: &mut Context<Self>) {
        let DdlExportRuntime {
            ready,
            busy,
            can_cancel,
            editable,
            capture_current,
            status,
        } = runtime;
        let status = bounded_status(status);
        if (
            self.ready,
            self.busy,
            self.can_cancel,
            self.editable,
            self.capture_current,
        ) == (ready, busy, can_cancel, editable, capture_current)
            && self.status == status
        {
            return;
        }
        self.ready = ready;
        self.busy = busy;
        self.can_cancel = can_cancel;
        self.editable = editable;
        self.capture_current = capture_current;
        self.status = status;
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_readonly(!editable, cx));
        }
        cx.notify();
    }
    /// Called only after the parent's document/request/cancellation fence. Both
    /// old capture and editor stay charged until replacement admission succeeds.
    pub fn receive(
        &mut self,
        artifact: DdlExportArtifact,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or("DDL capture identity exhausted; reopen this view")?;
        let capture = Capture::new(artifact, &self.connection, self.budget.clone())?;
        let text = capture
            .page(Section::Metadata, 0)
            .ok_or("DDL metadata preview unavailable")?;
        let editor = selected_editor(text, window, cx);
        let focused = self
            .editor
            .as_ref()
            .is_some_and(|old| old.editor.focus_handle(cx).contains_focused(window, cx));
        self.editor = Some(editor);
        self.capture = Some(capture);
        self.revision = revision;
        self.section = Section::Metadata;
        self.page = 0;
        self.capture_current = true;
        self.message = None;
        if focused {
            window.focus(&self.editor.as_ref().unwrap().editor.focus_handle(cx), cx);
        }
        cx.notify();
        Ok(())
    }
    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fields.is_empty() {
            return;
        }
        let lease = match Lease::new(self.budget.clone(), 2 * 1024 * 1024) {
            Ok(lease) => lease,
            Err(error) => {
                self.message = Some(error.into());
                return;
            }
        };
        self.field_lease = Some(lease);
        self.fields = ["Exact DDL schema name", "Exact DDL relation name"]
            .into_iter()
            .map(|label| cx.new(|cx| Field::new(label, 63, false, String::new(), window, cx)))
            .collect();
        self.field_events = self
            .fields
            .iter()
            .map(|field| {
                cx.subscribe(field, |_, _, _: &crate::bounded_field::Changed, cx| {
                    cx.notify()
                })
            })
            .collect();
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields
            .iter()
            .any(|field| field.update(cx, |field, cx| field.composing(window, cx)))
    }
    fn request(&self, cx: &gpui::App) -> Result<DdlExportRequest, &'static str> {
        if self.fields.len() != 2 {
            return Err("DDL scope fields are unavailable");
        }
        if self.scope == Scope::Database {
            return self.scope.request("", "");
        }
        let schema = self.fields[0].read(cx).value(cx)?;
        let relation = if self.scope == Scope::Relation {
            self.fields[1].read(cx).value(cx)?
        } else {
            String::new()
        };
        self.scope.request(&schema, &relation)
    }
    fn show_page(
        &mut self,
        section: Section,
        page: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self
            .capture
            .as_ref()
            .and_then(|capture| capture.page(section, page))
        else {
            return;
        };
        let next = selected_editor(text, window, cx);
        let focused = self
            .editor
            .as_ref()
            .is_some_and(|old| old.editor.focus_handle(cx).contains_focused(window, cx));
        self.editor = Some(next);
        self.section = section;
        self.page = page;
        if focused {
            window.focus(&self.editor.as_ref().unwrap().editor.focus_handle(cx), cx);
        }
    }
}
impl Drop for DdlExportView {
    fn drop(&mut self) {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
        self.editor = None;
        self.capture = None;
    }
}
fn selected_editor(
    text: &str,
    window: &mut Window,
    cx: &mut Context<DdlExportView>,
) -> SelectedEditor {
    let editor = cx.new(|cx| {
        let buffer = cx.new(|cx| language::Buffer::local(text, cx));
        let mut editor = Editor::for_buffer(buffer, None, window, cx);
        editor.set_read_only(true);
        editor.set_soft_wrap_mode(language::language_settings::SoftWrap::None, cx);
        editor
    });
    let accessible = cx.new(|cx| {
        AccessibleEditor::new(
            editor.clone(),
            "Read-only DDL preview page; copying selects only this page",
            cx,
        )
    });
    SelectedEditor { editor, accessible }
}
fn bounded_status(status: &str) -> String {
    if status.len() <= 4096 {
        return status.to_owned();
    }
    let mut end = 4096;
    while !status.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [truncated]", &status[..end])
}
