//! Read-only table Structure inspector. Every section is a compact table on
//! one virtualized page, with a section outline on the left and the exact
//! selected details on the right. Capture owns the payload allowance; only one
//! selected editor is retained, and navigation carries observed OIDs.
use crate::{
    accessible_editor::AccessibleEditor,
    table_structure_model::{Capture, Section},
};
use dbunk_lib::backend::table_structure::{TableIdentity, TableStructureRequest};
use editor::Editor;
use gpui::{
    ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
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
    Details,
    Section(usize),
}
/// Toolbar actions first, then the details-pane actions; Tab follows this
/// order around the section outline and the list.
const ACTIONS: [(Action, &str); 7] = [
    (Action::Cancel, "Cancel read"),
    (Action::Refresh, "Refresh"),
    (Action::Details, "Details"),
    (Action::Back, "Catalog"),
    (Action::Copy, "Copy"),
    (Action::Open, "Open table"),
    (Action::Edit, "Comment / rename"),
];
const TOOLBAR_ACTIONS: usize = 4;

/// One 28 px line of the structure page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item {
    /// Section heading, with the empty message inline when it has no rows.
    Title(Section),
    /// Scope note under a heading (relation grants).
    Note(Section),
    /// Column titles of a non-empty section.
    Head(Section),
    Row(Section, usize),
}
/// Page lines in section order; empty sections collapse to their heading.
fn layout(capture: &Capture) -> Vec<Item> {
    let mut items = Vec::new();
    for section in Section::ALL {
        let count = capture.count(section);
        items.push(Item::Title(section));
        if section == Section::RelationGrants {
            items.push(Item::Note(section));
        }
        if count > 0 {
            items.push(Item::Head(section));
            items.extend((0..count).map(|index| Item::Row(section, index)));
        }
    }
    items
}

struct SelectedEditor {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
pub struct StructureView {
    capture: Capture,
    items: Vec<Item>,
    /// Every selectable row in page order, for arrow navigation across sections.
    rows: Vec<(Section, usize)>,
    section: Section,
    selected: Option<usize>,
    editor: Option<SelectedEditor>,
    root: FocusHandle,
    list: FocusHandle,
    buttons: Vec<FocusHandle>,
    sections: Vec<FocusHandle>,
    scroll: UniformListScrollHandle,
    show_details: bool,
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
        let items = layout(&capture);
        let rows = items
            .iter()
            .filter_map(|item| match item {
                Item::Row(section, index) => Some((*section, *index)),
                _ => None,
            })
            .collect();
        let section = Section::ALL[0];
        let mut view = Self {
            capture,
            items,
            rows,
            section,
            selected: None,
            editor: None,
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            sections: Section::ALL.iter().map(|_| cx.focus_handle()).collect(),
            scroll: UniformListScrollHandle::new(),
            show_details: true,
            ready: false,
            busy: false,
            editable: true,
            capture_current: true,
            status: String::new(),
            message: None,
        };
        // Columns are what people open Structure for; the table row is above.
        let first = if view.capture.count(Section::Columns) > 0 {
            Section::Columns
        } else {
            section
        };
        view.select(first, 0, window, cx);
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
        match action {
            Action::Cancel => return self.busy,
            Action::Details => return true,
            _ => {}
        }
        if !self.editable {
            return false;
        }
        match action {
            Action::Back | Action::Section(_) => true,
            Action::Refresh => self.ready && !self.busy,
            Action::Cancel | Action::Details => unreachable!(),
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
            Action::Details => self.show_details = !self.show_details,
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
        if !matches!(action, Action::Back | Action::Section(_))
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
        let item = self
            .selected
            .map_or(Item::Title(section), |index| Item::Row(section, index));
        if let Some(position) = self.items.iter().position(|candidate| *candidate == item) {
            self.scroll
                .scroll_to_item(position, gpui::ScrollStrategy::Nearest);
        }
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
        if let Some(position) = self
            .items
            .iter()
            .position(|item| *item == Item::Title(section))
        {
            self.scroll
                .scroll_to_item(position, gpui::ScrollStrategy::Top);
        }
        window.focus(&self.sections[index], cx);
    }
    fn select_row(
        &mut self,
        section: Section,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A queued AX row action against a replaced capture cannot select a
        // row that no longer exists.
        if index >= self.capture.count(section) {
            return;
        }
        self.select(section, index, window, cx);
        window.focus(&self.list, cx);
    }
    fn focus_control(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = |(index, (action, _)): (usize, &(Action, &str))| {
            self.enabled(*action).then(|| self.buttons[index].clone())
        };
        let mut handles = ACTIONS[..TOOLBAR_ACTIONS]
            .iter()
            .enumerate()
            .filter_map(enabled)
            .collect::<Vec<_>>();
        if self.editable {
            handles.extend(self.sections.iter().cloned());
        }
        handles.push(self.list.clone());
        if self.show_details {
            handles.extend(
                ACTIONS
                    .iter()
                    .enumerate()
                    .skip(TOOLBAR_ACTIONS)
                    .filter_map(enabled),
            );
            if let Some(editor) = &self.editor {
                handles.push(editor.editor.focus_handle(cx));
            }
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
            key @ ("left" | "right" | "up" | "down")
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
                    self.show_details = true;
                    window.focus(&editor.editor.focus_handle(cx), cx);
                }
            }
            key if self.list.is_focused(window) => {
                let current = self.selected.and_then(|index| {
                    self.rows
                        .iter()
                        .position(|row| *row == (self.section, index))
                });
                let Some(next) = move_index(current, self.rows.len(), key) else {
                    return;
                };
                let (section, index) = self.rows[next];
                self.select(section, index, window, cx);
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
/// The outline is vertical, so up/down move like left/right.
fn move_section(current: usize, count: usize, key: &str) -> Option<usize> {
    if current >= count {
        return None;
    }
    match key {
        "left" | "up" => Some(current.saturating_sub(1)),
        "right" | "down" => Some(current.saturating_add(1).min(count - 1)),
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
