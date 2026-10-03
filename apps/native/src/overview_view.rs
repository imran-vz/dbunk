//! Explicit overview/statistics refresh. Parent owns the admitted data lane and
//! request/cancel fences; this child owns only editable scope and retained pages.
use crate::{
    accessible_editor::AccessibleEditor,
    bounded_field::Field,
    overview_model::{Capture, Scope, Section},
};
use dbunk_lib::backend::overview::{OverviewSnapshot, RelationStatsRequest};
use editor::Editor;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, Subscription,
    UniformListScrollHandle, Window, div, prelude::*, px, rgb, uniform_list,
};
use std::{cell::Cell, rc::Rc};
mod actions;
mod render;
#[cfg(test)]
mod tests;
gpui::actions!(overview, [NextControl, PreviousControl]);
pub enum OverviewEvent {
    Back,
    Connect,
    Refresh(RelationStatsRequest),
    Next(RelationStatsRequest),
    Cancel,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Back,
    Connect,
    Refresh,
    Next,
    Cancel,
    Scope(usize),
    Section(usize),
}
const ACTIONS: [(Action, &str); 5] = [
    (Action::Back, "Administration"),
    (Action::Connect, "Connect"),
    (Action::Refresh, "Refresh scope"),
    (Action::Next, "Next page"),
    (Action::Cancel, "Cancel read"),
];
const FIELD_BYTES: usize = 2 * 1024 * 1024;
struct FieldLease(Rc<Cell<usize>>);
impl FieldLease {
    fn new(budget: Rc<Cell<usize>>) -> Option<Self> {
        if FIELD_BYTES > (128 * 1024 * 1024usize).saturating_sub(budget.get()) {
            return None;
        }
        budget.set(budget.get() + FIELD_BYTES);
        Some(Self(budget))
    }
}
impl Drop for FieldLease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(FIELD_BYTES));
    }
}
struct SelectedEditor {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
pub struct OverviewRuntime<'a> {
    pub ready: bool,
    pub busy: bool,
    pub can_cancel: bool,
    pub editable: bool,
    pub capture_current: bool,
    pub status: &'a str,
}
pub struct OverviewView {
    budget: Rc<Cell<usize>>,
    capture: Option<Capture>,
    capture_revision: u64,
    section: Section,
    scope: Scope,
    selected: Option<usize>,
    editor: Option<SelectedEditor>,
    fields: Vec<Entity<Field>>,
    _field_events: Vec<Subscription>,
    _field_lease: Option<FieldLease>,
    root: FocusHandle,
    list: FocusHandle,
    buttons: Vec<FocusHandle>,
    scopes: Vec<FocusHandle>,
    sections: Vec<FocusHandle>,
    scroll: UniformListScrollHandle,
    ready: bool,
    busy: bool,
    can_cancel: bool,
    editable: bool,
    capture_current: bool,
    status: String,
    message: Option<String>,
}
impl EventEmitter<OverviewEvent> for OverviewView {}
impl OverviewView {
    pub fn new(budget: Rc<Cell<usize>>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            budget,
            capture: None,
            capture_revision: 0,
            section: Section::Database,
            scope: Scope::Database,
            selected: None,
            editor: None,
            fields: vec![],
            _field_events: vec![],
            _field_lease: None,
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            scopes: Scope::ALL.iter().map(|_| cx.focus_handle()).collect(),
            sections: Section::ALL.iter().map(|_| cx.focus_handle()).collect(),
            scroll: UniformListScrollHandle::new(),
            ready: false,
            busy: false,
            can_cancel: false,
            editable: true,
            capture_current: false,
            status: String::new(),
            message: None,
        };
        view.ensure_fields(window, cx);
        view
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn focus(&self, _cx: &gpui::App) -> FocusHandle {
        if self.capture.is_some() {
            self.list.clone()
        } else if self.ready {
            self.buttons[2].clone()
        } else {
            self.buttons[1].clone()
        }
    }
    pub fn set_runtime(&mut self, runtime: OverviewRuntime<'_>, cx: &mut Context<Self>) {
        let OverviewRuntime {
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
    /// Parent calls only after checking request identity, cancellation and bound
    /// document. Admission failure preserves the previous capture and editor.
    pub fn receive(
        &mut self,
        data: OverviewSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let revision = self
            .capture_revision
            .checked_add(1)
            .ok_or("Overview capture identity exhausted; reopen this view")?;
        let next = Capture::new(data, self.budget.clone(), self.capture.as_ref())?;
        let section = if next.count(self.section) > 0 {
            self.section
        } else {
            Section::Totals
        };
        let text = next.details(section, 0)?;
        let editor = text.map(|text| selected_editor(text, window, cx));
        let focused = self
            .editor
            .as_ref()
            .is_some_and(|old| old.editor.focus_handle(cx).contains_focused(window, cx));
        // Old editor dies before its allowance; replacement allowance was
        // reserved with the old capture still alive.
        self.editor = editor;
        self.capture = Some(next);
        self.capture_revision = revision;
        self.section = section;
        self.selected = Some(0);
        self.capture_current = true;
        self.message = None;
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        if focused && let Some(editor) = &self.editor {
            window.focus(&editor.editor.focus_handle(cx), cx);
        }
        cx.notify();
        Ok(())
    }
    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fields.is_empty() {
            return;
        }
        let Some(lease) = FieldLease::new(self.budget.clone()) else {
            self.message = Some(
                "Overview fields need 2 MiB of shared allowance; release another capture or tool"
                    .into(),
            );
            return;
        };
        self._field_lease = Some(lease);
        self.fields = ["Exact schema name", "Exact relation name"]
            .into_iter()
            .map(|label| cx.new(|cx| Field::new(label, 63, false, String::new(), window, cx)))
            .collect();
        self._field_events = self
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
    fn request(&self, cx: &gpui::App) -> Result<RelationStatsRequest, &'static str> {
        if self.fields.len() != 2 {
            return Err("Scope fields are unavailable");
        }
        if self.scope == Scope::Database {
            return self.scope.request("", "");
        }
        let schema = self.fields[0].read(cx).value(cx)?;
        let table = if self.scope == Scope::Relation {
            self.fields[1].read(cx).value(cx)?
        } else {
            String::new()
        };
        self.scope.request(&schema, &table)
    }
    fn scope_matches(&self, cx: &gpui::App) -> bool {
        self.request(cx).is_ok_and(|request| {
            self.capture
                .as_ref()
                .is_some_and(|capture| capture.matches(&request))
        })
    }
    fn select(
        &mut self,
        section: Section,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable
            || self.composing(window, cx)
            || (self.section == section && self.selected == Some(index) && self.editor.is_some())
        {
            return;
        }
        let Some(capture) = self.capture.as_ref() else {
            return;
        };
        let text = match capture.details(section, index) {
            Ok(text) => text,
            Err(error) => {
                self.message = Some(error.into());
                cx.notify();
                return;
            }
        };
        self.editor = text.map(|text| selected_editor(text, window, cx));
        self.section = section;
        self.selected = (index < capture.count(section)).then_some(index);
        self.message = None;
        self.scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }
    fn select_row(
        &mut self,
        revision: u64,
        section: Section,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if revision != self.capture_revision
            || section != self.section
            || self
                .capture
                .as_ref()
                .is_none_or(|capture| index >= capture.count(section))
        {
            return;
        }
        self.select(section, index, window, cx);
        if !self.composing(window, cx) {
            window.focus(&self.list, cx);
        }
    }
}
fn selected_editor(
    text: String,
    window: &mut Window,
    cx: &mut Context<OverviewView>,
) -> SelectedEditor {
    let editor = cx.new(|cx| {
        let buffer = cx.new(|cx| language::Buffer::local(text, cx));
        let mut editor = Editor::for_buffer(buffer, None, window, cx);
        editor.set_read_only(true);
        editor.set_soft_wrap_mode(language::language_settings::SoftWrap::None, cx);
        editor
    });
    let accessible = cx
        .new(|cx| AccessibleEditor::new(editor.clone(), "Read-only exact overview statistics", cx));
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
