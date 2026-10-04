//! Selected Tool tabs A: bounded library pages and exact SQL opened as a query
//! draft. Saved SQL is edited in that durable editor, then explicitly saved.
use crate::{
    accessible_editor::AccessibleEditor,
    controller::{Host, LibraryCommand, LibraryControls, LibraryDelivery, LibraryReply},
    query_library::{Page, Rows},
};
use dbunk_lib::backend::{
    WorkspaceTool,
    query_library::{LibraryRequest, SavedQueryRecord},
};
use editor::Editor;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, SharedString,
    Window, div, prelude::*, px,
};
use std::{cell::Cell, collections::HashMap, rc::Rc, sync::Arc};

pub struct OpenQuery {
    pub sql: String,
    pub name: String,
    pub connection: Option<String>,
    pub saved_id: Option<String>,
}
pub enum LibraryEvent {
    Open(OpenQuery),
}
#[derive(Clone)]
enum Action {
    Refresh,
    Next,
    Search,
    Filter,
    Connection,
    Select(usize),
    Open,
    Copy,
    Favorite,
    Delete,
    Clear,
    ConfirmClear,
}
pub struct LibraryView {
    kind: WorkspaceTool,
    controls: Option<LibraryControls>,
    receiver: Option<async_channel::Receiver<LibraryDelivery>>,
    page: Page,
    search: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    focus: FocusHandle,
    request: LibraryRequest,
    connection: Option<String>,
    connections: Vec<(String, String)>,
    selected: Option<usize>,
    initial_load_pending: bool,
    busy: bool,
    editable: bool,
    confirm_clear: bool,
    status: String,
    queued_save: bool,
    focus_handles: HashMap<String, FocusHandle>,
    visible_controls: Vec<FocusHandle>,
}
impl EventEmitter<LibraryEvent> for LibraryView {}
impl LibraryView {
    pub fn new(
        host: Arc<Host>,
        document: &dbunk_lib::backend::WorkspaceDocument,
        wake: async_channel::Sender<()>,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| Editor::single_line(window, cx));
        let accessible =
            cx.new(|cx| AccessibleEditor::field(search.clone(), "Search query library", false, cx));
        let opened = host.open_library(document.id.clone(), wake);
        let (controls, receiver, status) = match opened {
            Ok((controls, receiver)) => (
                Some(controls),
                Some(receiver),
                "Open tab to load records".into(),
            ),
            Err(error) => (None, None, error.into()),
        };
        Self {
            kind: document.tool.expect("library document"),
            controls,
            receiver,
            page: Page::new(budget),
            search,
            accessible,
            focus: cx.focus_handle(),
            request: LibraryRequest::default(),
            connection: document.connection_id.clone(),
            connections: Vec::new(),
            selected: None,
            initial_load_pending: true,
            busy: false,
            editable: true,
            confirm_clear: false,
            status,
            queued_save: false,
            focus_handles: HashMap::new(),
            visible_controls: Vec::new(),
        }
    }
    fn send(&mut self, command: LibraryCommand) {
        if self.busy || !self.editable {
            return;
        }
        match self
            .controls
            .as_ref()
            .ok_or("Library unavailable")
            .and_then(|controls| controls.send(command))
        {
            Ok(()) => {
                self.busy = true;
                self.status = "Loading…".into();
            }
            Err(error) => self.status = error.into(),
        }
    }
    fn load(&mut self, next: bool) {
        self.initial_load_pending = false;
        if next {
            let Some(cursor) = self.page.rows.as_ref().and_then(Rows::next) else {
                return;
            };
            self.request.cursor = Some(cursor);
        } else {
            self.request.cursor = None;
        }
        self.send(LibraryCommand::Load(self.kind, self.request.clone()));
    }
    pub fn set_connections(&mut self, mut connections: Vec<(String, String)>) {
        if let Some(id) = &self.connection
            && !connections.iter().any(|(known, _)| known == id)
        {
            connections.push((id.clone(), format!("Removed connection: {id}")));
        }
        self.connections = connections;
    }
    pub fn save(&mut self, query: SavedQueryRecord, cx: &mut Context<Self>) {
        if self.busy {
            match self
                .controls
                .as_ref()
                .ok_or("Library unavailable")
                .and_then(|controls| controls.send(LibraryCommand::SaveDraft(query)))
            {
                Ok(()) => self.queued_save = true,
                Err(error) => self.status = error.into(),
            }
        } else {
            self.send(LibraryCommand::SaveDraft(query));
        }
        cx.notify();
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(delivery) = self
            .receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        else {
            return false;
        };
        self.busy = false;
        match delivery.result {
            Ok(LibraryReply::Page(rows)) => match self.page.replace(rows) {
                Ok(()) => {
                    self.selected = None;
                    self.status = if self
                        .page
                        .rows
                        .as_ref()
                        .is_some_and(|rows| rows.len() == 0 && rows.next().is_some())
                    {
                        "No matches in this scan. Continue to search older records.".into()
                    } else {
                        format!(
                            "{} records on this page",
                            self.page.rows.as_ref().unwrap().len()
                        )
                    };
                }
                Err(error) => self.status = error.into(),
            },
            Ok(LibraryReply::Saved(query)) => {
                self.status = format!("Saved {}", query.name);
                self.load(false);
            }
            Ok(LibraryReply::Changed) => self.load(false),
            Ok(LibraryReply::SafetyAudit(..) | LibraryReply::ExportConfigurations(..)) => {
                self.status = "Unexpected tool reply in query library".into()
            }
            Err(error) => self.status = error,
        }
        if self.queued_save {
            self.queued_save = false;
            self.busy = true;
            self.status = "Saving query…".into();
        }
        cx.notify();
        true
    }
    pub fn has_pending(&self) -> bool {
        self.receiver
            .as_ref()
            .is_some_and(|receiver| !receiver.is_empty())
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Restoring hidden tabs must not reserve every library page at once.
        // Only the first activation loads implicitly; failures need explicit retry.
        if self.initial_load_pending && self.editable {
            self.load(false);
            cx.notify();
        }
        window.focus(&self.search.focus_handle(cx), cx);
    }
    pub fn set_editable(&mut self, value: bool) {
        self.editable = value;
    }
    fn selected_query(&self) -> Option<OpenQuery> {
        let index = self.selected?;
        match self.page.rows.as_ref()? {
            Rows::History(page) => page.entries.get(index).map(|row| OpenQuery {
                sql: row.sql.clone(),
                name: "History query".into(),
                connection: Some(row.connection_id.clone()),
                saved_id: None,
            }),
            Rows::Saved(page) => page.entries.get(index).map(|row| OpenQuery {
                sql: row.body.clone(),
                name: row.name.clone(),
                connection: row.connection_id.clone(),
                saved_id: Some(row.id.clone()),
            }),
        }
    }
    fn activate(&mut self, action: Action, _: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !self.editable {
            return;
        }
        if !matches!(action, Action::Clear | Action::ConfirmClear) {
            self.confirm_clear = false;
        }
        match action {
            Action::Refresh => self.load(false),
            Action::Next => self.load(true),
            Action::Search => {
                let search = self.search.read(cx).text(cx);
                if search.len() > 8192 {
                    self.status = "Search exceeds 8 KiB".into();
                } else {
                    self.request.search = search;
                    self.load(false);
                }
            }
            Action::Filter => {
                self.request.status = match self.request.status.as_deref() {
                    None => Some("error".into()),
                    Some("error") => Some("success".into()),
                    _ => None,
                };
                self.load(false);
            }
            Action::Connection => {
                self.request.connection_id = match &self.request.connection_id {
                    None => self.connections.first().map(|(id, _)| id.clone()),
                    Some(current) => self
                        .connections
                        .iter()
                        .position(|(id, _)| id == current)
                        .and_then(|index| self.connections.get(index + 1))
                        .map(|(id, _)| id.clone()),
                };
                self.load(false);
            }
            Action::Select(index) => self.selected = Some(index),
            Action::Open => {
                if let Some(query) = self.selected_query() {
                    cx.emit(LibraryEvent::Open(query));
                }
            }
            Action::Copy => {
                if let Some(query) = self.selected_query() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(query.sql));
                    self.status = "Exact SQL copied".into();
                }
            }
            Action::Favorite => {
                if let (Some(index), Some(Rows::Saved(page))) = (self.selected, &self.page.rows)
                    && let Some(query) = page.entries.get(index)
                {
                    let mut query = query.clone();
                    query.is_favorite = !query.is_favorite;
                    self.send(LibraryCommand::Save(query));
                }
            }
            Action::Delete => {
                let id = match (self.selected, &self.page.rows) {
                    (Some(index), Some(Rows::History(page))) => {
                        page.entries.get(index).map(|row| row.id.clone())
                    }
                    (Some(index), Some(Rows::Saved(page))) => {
                        page.entries.get(index).map(|row| row.id.clone())
                    }
                    _ => None,
                };
                if let Some(id) = id {
                    self.send(LibraryCommand::Delete(self.kind, id));
                }
            }
            Action::Clear => self.confirm_clear = true,
            Action::ConfirmClear => {
                self.confirm_clear = false;
                self.send(LibraryCommand::Clear);
            }
        }
        cx.notify();
    }
    fn button(
        &mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = id.into();
        let label = match action {
            Action::Connection => SharedString::from(
                self.request
                    .connection_id
                    .as_ref()
                    .map(|id| {
                        self.connections
                            .iter()
                            .find(|(known, _)| known == id)
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| format!("Removed connection: {id}"))
                    })
                    .unwrap_or_else(|| "All connections".into()),
            ),
            Action::Filter => SharedString::from(
                self.request
                    .status
                    .clone()
                    .unwrap_or_else(|| "All outcomes".into()),
            ),
            _ => label.into(),
        };
        let focus = self
            .focus_handles
            .entry(id.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = !self.busy
            && self.editable
            && (!matches!(
                action,
                Action::Open | Action::Copy | Action::Favorite | Action::Delete
            ) || self.selected.is_some())
            && (!matches!(action, Action::Connection) || !self.connections.is_empty());
        if enabled {
            self.visible_controls.push(focus.clone());
        }
        let click = action.clone();
        let ax = action.clone();
        let weak = cx.weak_entity();
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label.clone())
            .track_focus(&focus)
            .tab_stop(enabled)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .tab_index(0)
            .h(px(crate::style::TOOL))
            .px(px(6.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .text_sm()
            .text_color(if enabled {
                crate::style::dim()
            } else {
                crate::style::faint()
            })
            .when(enabled, |button| {
                crate::ui::press(
                    button
                        .cursor_pointer()
                        .hover(|s| s.bg(crate::style::hover()).text_color(crate::style::text())),
                )
            })
            .focus(|s| s.bg(crate::style::hover()).text_color(crate::style::text()))
            .when(
                matches!(action, Action::Select(index) if self.selected == Some(index)),
                |button| {
                    button
                        .bg(crate::style::select())
                        .text_color(crate::style::text())
                },
            )
            .child(match &action {
                Action::Refresh => "Refresh".into(),
                Action::Next => "Next page".into(),
                Action::Search => "Search".into(),
                Action::Filter => self.request.status.clone().unwrap_or("All outcomes".into()),
                Action::Connection => label.to_string(),
                Action::Open => "Open query".into(),
                Action::Copy => "Copy SQL".into(),
                Action::Favorite => "Toggle favorite".into(),
                Action::Delete => "Delete selected".into(),
                Action::Clear => "Clear history".into(),
                Action::ConfirmClear => "Confirm clear all history".into(),
                Action::Select(_) => label.to_string(),
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.activate(click.clone(), window, cx)),
            )
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(ax.clone(), window, cx))
                    .ok();
            })
            .into_any_element()
    }
}
impl Render for LibraryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_controls.clear();
        let mut rows = Vec::new();
        if let Some(page) = &self.page.rows {
            match page {
                Rows::History(page) => {
                    for (index, row) in page.entries.iter().enumerate() {
                        rows.push((
                            index,
                            format!(
                                "{}  {}  {}  {}ms  {} rows  {}",
                                row.started_at,
                                row.connection_name,
                                row.status,
                                row.runtime_ms,
                                row.row_count
                                    .map(|count| count.to_string())
                                    .unwrap_or("unknown".into()),
                                row.sql.chars().take(160).collect::<String>()
                            ),
                        ));
                    }
                }
                Rows::Saved(page) => {
                    for (index, row) in page.entries.iter().enumerate() {
                        rows.push((
                            index,
                            format!(
                                "{}{}  {}",
                                if row.is_favorite { "★ " } else { "" },
                                row.name,
                                row.body.chars().take(160).collect::<String>()
                            ),
                        ));
                    }
                }
            }
        }
        let selected_error = match (&self.page.rows, self.selected) {
            (Some(Rows::History(page)), Some(index)) => page
                .entries
                .get(index)
                .and_then(|record| record.error_message.as_ref())
                .cloned(),
            _ => None,
        };
        div()
            .id("query-library")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" {
                    let handles = std::iter::once(this.search.focus_handle(cx))
                        .chain(this.visible_controls.iter().cloned())
                        .collect::<Vec<_>>();
                    let current = handles.iter().position(|handle| handle.is_focused(window));
                    let next = if event.keystroke.modifiers.shift {
                        current.map_or(handles.len() - 1, |index| {
                            (index + handles.len() - 1) % handles.len()
                        })
                    } else {
                        current.map_or(0, |index| (index + 1) % handles.len())
                    };
                    window.focus(&handles[next], cx);
                    cx.stop_propagation();
                }
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_sm()
            .gap_2()
            .p_2()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().w(px(280.)).child(self.accessible.clone()))
                    .child(self.button("library-search", "Search", Action::Search, cx))
                    .child(self.button("library-refresh", "Refresh", Action::Refresh, cx))
                    .child(self.button(
                        "library-connection",
                        "Filter connection",
                        Action::Connection,
                        cx,
                    ))
                    .when(self.kind == WorkspaceTool::History, |row| {
                        row.child(self.button(
                            "library-outcome",
                            "Filter outcome",
                            Action::Filter,
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "library-open",
                        "Open exact SQL as query draft",
                        Action::Open,
                        cx,
                    ))
                    .child(self.button("library-copy", "Copy exact SQL", Action::Copy, cx))
                    .child(self.button("library-delete", "Delete selected", Action::Delete, cx))
                    .when(self.kind == WorkspaceTool::SavedQueries, |row| {
                        row.child(self.button(
                            "library-favorite",
                            "Toggle favorite",
                            Action::Favorite,
                            cx,
                        ))
                    })
                    .when(self.kind == WorkspaceTool::History, |row| {
                        row.child(self.button("library-clear", "Clear history", Action::Clear, cx))
                    })
                    .when(self.confirm_clear, |row| {
                        row.child(self.button(
                            "library-confirm-clear",
                            "Confirm clear all history",
                            Action::ConfirmClear,
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .id("library-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows.into_iter().map(|(index, text)| {
                        self.button(
                            format!("library-row-{index}"),
                            text,
                            Action::Select(index),
                            cx,
                        )
                    })),
            )
            .when_some(selected_error, |view, message| {
                view.child(
                    div()
                        .id("library-error")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .role(Role::Label)
                        .aria_label(message.clone())
                        .child(message),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("library-status")
                            .role(Role::Label)
                            .aria_label(self.status.clone())
                            .child(self.status.clone()),
                    )
                    .when(
                        self.page
                            .rows
                            .as_ref()
                            .is_some_and(|rows| rows.next().is_some()),
                        |row| row.child(self.button("library-next", "Next page", Action::Next, cx)),
                    ),
            )
    }
}
