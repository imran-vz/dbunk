//! Read-only table Structure inspector. Capture owns the payload allowance;
//! only one selected editor is retained, and navigation carries observed OIDs.
use crate::{
    accessible_editor::AccessibleEditor,
    table_structure_model::{Capture, Section},
};
use dbunk_lib::backend::table_structure::{TableIdentity, TableStructureRequest};
use editor::Editor;
use gpui::{
    ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    UniformListScrollHandle, Window, div, prelude::*, px, rgb, uniform_list,
};

mod render;
#[cfg(test)]
mod tests;

gpui::actions!(table_structure, [NextControl, PreviousControl]);

pub enum StructureEvent {
    Edit(crate::table_ddl_model::Selection),
    Back,
    Refresh(TableStructureRequest),
    Cancel,
    OpenRelation {
        schema: String,
        table: String,
        identity: TableIdentity,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Edit,
    Back,
    Refresh,
    Cancel,
    Copy,
    Open,
    Section(usize),
}
const ACTIONS: [(Action, &str); 6] = [
    (Action::Back, "Catalog"),
    (Action::Refresh, "Refresh"),
    (Action::Cancel, "Cancel read"),
    (Action::Copy, "Copy selected details"),
    (Action::Open, "Open related table"),
    (Action::Edit, "Comment / rename"),
];
struct SelectedEditor {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
pub struct StructureView {
    capture: Capture,
    section: Section,
    selected: Option<usize>,
    editor: Option<SelectedEditor>,
    root: FocusHandle,
    list: FocusHandle,
    buttons: Vec<FocusHandle>,
    sections: Vec<FocusHandle>,
    scroll: UniformListScrollHandle,
    ready: bool,
    busy: bool,
    editable: bool,
    capture_current: bool,
    status: String,
    message: Option<String>,
}
impl EventEmitter<StructureEvent> for StructureView {}
impl StructureView {
    pub fn new(capture: Capture, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let section = Section::ALL[0];
        let mut view = Self {
            capture,
            section,
            selected: None,
            editor: None,
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            sections: Section::ALL.iter().map(|_| cx.focus_handle()).collect(),
            scroll: UniformListScrollHandle::new(),
            ready: false,
            busy: false,
            editable: true,
            capture_current: true,
            status: String::new(),
            message: None,
        };
        view.select(section, 0, window, cx);
        view
    }
    pub fn request(&self) -> TableStructureRequest {
        self.capture.request()
    }
    pub fn identity(&self) -> TableIdentity {
        self.capture.identity()
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn focus(&self, _cx: &gpui::App) -> FocusHandle {
        self.list.clone()
    }
    /// Parent supplies transport readiness separately from capture currency.
    /// Called during parent rendering, so unchanged values must not repaint.
    pub fn set_runtime(
        &mut self,
        ready: bool,
        busy: bool,
        editable: bool,
        capture_current: bool,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        if status.len() <= 4096
            && self.status == status
            && (self.ready, self.busy, self.editable, self.capture_current)
                == (ready, busy, editable, capture_current)
        {
            return;
        }
        let status = bounded_status(status);
        if (self.ready, self.busy, self.editable, self.capture_current)
            == (ready, busy, editable, capture_current)
            && self.status == status
        {
            return;
        }
        self.ready = ready;
        self.busy = busy;
        self.editable = editable;
        self.capture_current = capture_current;
        self.status = status;
        cx.notify();
    }
    fn enabled(&self, action: Action) -> bool {
        if matches!(action, Action::Cancel) {
            return self.busy;
        }
        if !self.editable {
            return false;
        }
        match action {
            Action::Back => true,
            Action::Section(_) => true,
            Action::Refresh => self.ready && !self.busy,
            Action::Cancel => self.busy,
            Action::Copy => self.editor.is_some(),
            Action::Edit => {
                self.ready
                    && !self.busy
                    && self.capture_current
                    && self
                        .selected
                        .and_then(|index| self.capture.ddl_selection(self.section, index))
                        .is_some()
            }
            Action::Open => {
                self.ready
                    && !self.busy
                    && self.capture_current
                    && self
                        .selected
                        .and_then(|index| self.capture.navigation(self.section, index))
                        .is_some()
            }
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::Edit => {
                if let Some(selection) = self
                    .selected
                    .and_then(|index| self.capture.ddl_selection(self.section, index))
                {
                    cx.emit(StructureEvent::Edit(selection));
                }
            }
            Action::Section(index) => self.select_section(index, window, cx),
            Action::Back => cx.emit(StructureEvent::Back),
            Action::Refresh => {
                self.capture_current = false;
                self.message = None;
                cx.emit(StructureEvent::Refresh(self.request()));
            }
            Action::Cancel => cx.emit(StructureEvent::Cancel),
            Action::Copy => {
                if let Some(editor) = &self.editor {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        editor.editor.read(cx).text(cx),
                    ));
                    self.message = Some("Copied exact selected details".into());
                }
            }
            Action::Open => {
                if let Some(target) = self
                    .selected
                    .and_then(|index| self.capture.navigation(self.section, index))
                {
                    cx.emit(StructureEvent::OpenRelation {
                        schema: target.schema.to_owned(),
                        table: target.table.to_owned(),
                        identity: target.identity,
                    });
                }
            }
        }
        if !matches!(action, Action::Back)
            && let Some(index) = ACTIONS.iter().position(|(item, _)| *item == action)
        {
            window.focus(&self.buttons[index], cx);
        }
        cx.notify();
    }
    fn select(
        &mut self,
        section: Section,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable
            || (self.section == section && self.selected == Some(index) && self.editor.is_some())
        {
            return;
        }
        let text = match self.capture.details(section, index) {
            Ok(text) => text,
            Err(error) => {
                self.message = Some(error.into());
                cx.notify();
                return;
            }
        };
        // A fresh buffer has no undo history containing the previous selection.
        // Capture's working allowance includes the temporary replacement overlap.
        let editor = text.map(|text| {
            let editor = cx.new(|cx| {
                let mut editor = Editor::for_buffer(
                    cx.new(|cx| language::Buffer::local(text, cx)),
                    None,
                    window,
                    cx,
                );
                editor.set_read_only(true);
                // Admission counts logical AX runs. Keep long metadata lines
                // horizontally scrollable instead of creating extra wrap runs.
                editor.set_soft_wrap_mode(language::language_settings::SoftWrap::None, cx);
                editor
            });
            let accessible = cx.new(|cx| {
                AccessibleEditor::new(
                    editor.clone(),
                    "Read-only selected table structure details",
                    cx,
                )
            });
            SelectedEditor { editor, accessible }
        });
        self.editor = editor;
        self.section = section;
        self.selected = (index < self.capture.count(section)).then_some(index);
        self.message = None;
        self.scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
        cx.notify();
    }
    fn select_section(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        let Some(section) = Section::ALL.get(index).copied() else {
            return;
        };
        self.select(section, 0, window, cx);
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        window.focus(&self.sections[index], cx);
    }
    fn select_row(
        &mut self,
        section: Section,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A queued AX row action from an earlier section cannot select a row in
        // the newly displayed section with the same numeric index.
        if section != self.section || index >= self.capture.count(section) {
            return;
        }
        self.select(section, index, window, cx);
        window.focus(&self.list, cx);
    }
    fn focus_control(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut handles = ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, (action, _))| self.enabled(*action))
            .map(|(index, _)| self.buttons[index].clone())
            .collect::<Vec<_>>();
        if self.editable {
            handles.extend(self.sections.iter().cloned());
        }
        handles.push(self.list.clone());
        if let Some(editor) = &self.editor {
            handles.push(editor.editor.focus_handle(cx));
        }
        let current = handles.iter().position(|handle| handle.is_focused(window));
        let next = if reverse {
            current.map_or(handles.len() - 1, |index| {
                (index + handles.len() - 1) % handles.len()
            })
        } else {
            current.map_or(0, |index| (index + 1) % handles.len())
        };
        window.focus(&handles[next], cx);
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => self.focus_control(modifiers.shift, window, cx),
            "escape" => self.activate(Action::Back, window, cx),
            key @ ("left" | "right")
                if self.sections.iter().any(|handle| handle.is_focused(window)) =>
            {
                let current = self
                    .sections
                    .iter()
                    .position(|handle| handle.is_focused(window))
                    .unwrap();
                if let Some(index) = move_section(current, self.sections.len(), key) {
                    self.select_section(index, window, cx);
                }
            }
            "enter" if self.list.is_focused(window) => {
                if let Some(editor) = &self.editor {
                    window.focus(&editor.editor.focus_handle(cx), cx);
                }
            }
            key if self.list.is_focused(window) => {
                let Some(index) = move_index(self.selected, self.capture.count(self.section), key)
                else {
                    return;
                };
                self.select(self.section, index, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        window.prevent_default();
    }
}
fn move_index(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let missing = current.is_none();
    let current = current.unwrap_or(0).min(count - 1);
    Some(match key {
        "up" => current.saturating_sub(1),
        "down" if missing => 0,
        "down" => (current + 1).min(count - 1),
        "pageup" => current.saturating_sub(20),
        "pagedown" => current.saturating_add(20).min(count - 1),
        "home" => 0,
        "end" => count - 1,
        _ => return None,
    })
}
fn move_section(current: usize, count: usize, key: &str) -> Option<usize> {
    if current >= count {
        return None;
    }
    match key {
        "left" => Some(current.saturating_sub(1)),
        "right" => Some(current.saturating_add(1).min(count - 1)),
        _ => None,
    }
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
