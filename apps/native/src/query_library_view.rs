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
                        record_count(self.page.rows.as_ref().unwrap().len())
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
        let text: SharedString = match &action {
            Action::Refresh => "Refresh".into(),
            Action::Next => "Next page".into(),
            Action::Search => "Search".into(),
            Action::Filter => match self.request.status.as_deref() {
                Some("error") => "Errors only".into(),
                Some("success") => "Successes only".into(),
                Some(other) => SharedString::from(other.to_owned()),
                None => "All outcomes".into(),
            },
            Action::Connection => label.clone(),
            Action::Open => "Open query".into(),
            Action::Copy => "Copy SQL".into(),
            Action::Favorite => "Toggle favorite".into(),
            Action::Delete => "Delete".into(),
            Action::Clear => "Clear history".into(),
            Action::ConfirmClear => "Confirm clear all history".into(),
            Action::Select(_) => label.clone(),
        };
        let icon = matches!(action, Action::Connection | Action::Filter)
            .then_some("icons/chevron_up_down.svg");
        let destructive = matches!(action, Action::ConfirmClear);
        let base = if matches!(action, Action::Open) {
            crate::ui::tool_button_accent(id, text, None, enabled)
        } else {
            crate::ui::tool_button(id, text, icon, enabled, false)
        };
        base.aria_label(label.clone())
            .track_focus(&focus)
            .tab_stop(enabled)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .tab_index(0)
            .when(destructive && enabled, |button| {
                button.text_color(crate::style::bad_text())
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
/// `1 record`, `2 records`: the page count in the footer.
fn record_count(count: usize) -> String {
    if count == 1 {
        "1 record".into()
    } else {
        format!("{} records", crate::ui::grouped(count as u64))
    }
}
/// History columns: SQL first, then where, when and how it ran.
fn history_columns() -> Vec<crate::ui::TableColumn> {
    use crate::ui::TableColumn;
    vec![
        TableColumn::flex("Query").mono(),
        TableColumn::fixed("Connection", 140.),
        TableColumn::fixed("Time", 110.),
        TableColumn::fixed("Outcome", 80.),
        TableColumn::fixed("Duration", 80.).end(),
        TableColumn::fixed("Rows", 72.).end(),
    ]
}
fn saved_columns() -> Vec<crate::ui::TableColumn> {
    use crate::ui::TableColumn;
    vec![
        TableColumn::fixed("Name", 220.),
        TableColumn::flex("Query").mono(),
    ]
}
/// `6 ms`, `1.2 s`: a run time inside a data row.
fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{:.1} s", ms as f64 / 1000.)
    }
}
impl LibraryView {
    /// One selectable table row. Keeps the row's id, AX label (the whole
    /// record as text), focus handle and tab stop of the former row button.
    fn row(
        &mut self,
        index: usize,
        ax_label: String,
        cells: Vec<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = format!("library-row-{index}");
        let focus = self
            .focus_handles
            .entry(key.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = !self.busy && self.editable;
        if enabled {
            self.visible_controls.push(focus.clone());
        }
        let selected = self.selected == Some(index);
        let weak = cx.weak_entity();
        crate::ui::data_table_row(SharedString::from(key), cells, selected, true)
            .role(Role::Button)
            .aria_label(ax_label)
            .aria_selected(selected)
            .track_focus(&focus)
            .tab_stop(enabled)
            .tab_index(0)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .when(enabled, |row| row.cursor_pointer())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate(Action::Select(index), window, cx)
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| {
                    this.activate(Action::Select(index), window, cx)
                })
                .ok();
            })
            .into_any_element()
    }
}
impl Render for LibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::overview_view::chrome;
        use crate::style;
        self.visible_controls.clear();
        let history = self.kind == WorkspaceTool::History;
        let columns = if history {
            history_columns()
        } else {
            saved_columns()
        };
        let today = chrome::today();
        // (index, AX label, cell values, failed)
        let mut records: Vec<(usize, String, Vec<Option<SharedString>>, bool)> = Vec::new();
        // The selected record as labelled properties, with its exact SQL.
        let mut details: Vec<(SharedString, Option<SharedString>)> = Vec::new();
        let mut selected_sql: Option<String> = None;
        let mut selected_started: Option<String> = None;
        if let Some(page) = &self.page.rows {
            match page {
                Rows::History(page) => {
                    for (index, row) in page.entries.iter().enumerate() {
                        let rows = row
                            .row_count
                            .map(|count| count.to_string())
                            .unwrap_or("unknown".into());
                        let ax = format!(
                            "{}  {}  {}  {}ms  {} rows  {}",
                            row.started_at,
                            row.connection_name,
                            row.status,
                            row.runtime_ms,
                            rows,
                            row.sql.chars().take(160).collect::<String>()
                        );
                        let when = chrome::compact_timestamp(&row.started_at, today)
                            .unwrap_or_else(|| row.started_at.clone());
                        records.push((
                            index,
                            ax,
                            vec![
                                Some(chrome::single_line(&row.sql, 240).into()),
                                Some(row.connection_name.clone().into()),
                                Some(when.into()),
                                Some(row.status.clone().into()),
                                Some(duration(row.runtime_ms).into()),
                                row.row_count.map(|count| crate::ui::grouped(count).into()),
                            ],
                            row.status == "error",
                        ));
                        if self.selected == Some(index) {
                            details = vec![
                                (
                                    "Connection".into(),
                                    Some(row.connection_name.clone().into()),
                                ),
                                ("Outcome".into(), Some(row.status.clone().into())),
                                ("Duration".into(), Some(duration(row.runtime_ms).into())),
                                (
                                    "Rows".into(),
                                    row.row_count.map(|count| crate::ui::grouped(count).into()),
                                ),
                            ];
                            selected_started = Some(row.started_at.clone());
                            selected_sql = Some(row.sql.clone());
                        }
                    }
                }
                Rows::Saved(page) => {
                    for (index, row) in page.entries.iter().enumerate() {
                        let ax = format!(
                            "{}{}  {}",
                            if row.is_favorite { "★ " } else { "" },
                            row.name,
                            row.body.chars().take(160).collect::<String>()
                        );
                        records.push((
                            index,
                            ax,
                            vec![
                                Some(
                                    format!(
                                        "{}{}",
                                        if row.is_favorite { "★ " } else { "" },
                                        chrome::single_line(&row.name, 120)
                                    )
                                    .into(),
                                ),
                                Some(chrome::single_line(&row.body, 240).into()),
                            ],
                            false,
                        ));
                        if self.selected == Some(index) {
                            details = vec![
                                ("Name".into(), Some(row.name.clone().into())),
                                (
                                    "Favorite".into(),
                                    Some(if row.is_favorite { "Yes" } else { "No" }.into()),
                                ),
                            ];
                            selected_sql = Some(row.body.clone());
                        }
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
        let has_rows = !records.is_empty();
        let loaded = self.page.rows.is_some();
        let searching = !self.request.search.is_empty()
            || self.request.status.is_some()
            || self.request.connection_id.is_some();
        let rows = records
            .into_iter()
            .map(|(index, ax, values, failed)| {
                // Query in `text`, the rest of the row `dim`; failures in red.
                let cells = columns
                    .iter()
                    .zip(values)
                    .enumerate()
                    .map(|(position, (column, value))| {
                        let cell = crate::ui::data_cell(column, value);
                        if history && position == 3 && failed {
                            cell.text_color(style::bad_text())
                        } else if position > 0 && position != 3 {
                            cell.text_color(style::dim())
                        } else {
                            cell
                        }
                    })
                    .collect::<Vec<_>>();
                self.row(index, ax, cells, cx)
            })
            .collect::<Vec<_>>();
        let title = if history {
            "Query history"
        } else {
            "Saved queries"
        };
        let search_focused = self.search.focus_handle(cx).is_focused(window);
        let refresh = self.button("library-refresh", "Refresh", Action::Refresh, cx);
        let clear =
            history.then(|| self.button("library-clear", "Clear history", Action::Clear, cx));
        let confirm = self.confirm_clear.then(|| {
            self.button(
                "library-confirm-clear",
                "Confirm clear all history",
                Action::ConfirmClear,
                cx,
            )
        });
        let search = self.button("library-search", "Search", Action::Search, cx);
        let connection = self.button(
            "library-connection",
            "Filter connection",
            Action::Connection,
            cx,
        );
        let outcome =
            history.then(|| self.button("library-outcome", "Filter outcome", Action::Filter, cx));
        let header_actions = std::iter::once(refresh)
            .chain(clear)
            .chain(confirm)
            .collect::<Vec<_>>();
        let selection = self.selected.is_some() && selected_sql.is_some();
        let selection_actions = if selection {
            let mut actions = vec![
                self.button(
                    "library-open",
                    "Open exact SQL as query draft",
                    Action::Open,
                    cx,
                ),
                self.button("library-copy", "Copy exact SQL", Action::Copy, cx),
            ];
            if !history {
                actions.push(self.button(
                    "library-favorite",
                    "Toggle favorite",
                    Action::Favorite,
                    cx,
                ));
            }
            actions.push(self.button("library-delete", "Delete selected", Action::Delete, cx));
            actions
        } else {
            Vec::new()
        };
        let next = self
            .page
            .rows
            .as_ref()
            .is_some_and(|rows| rows.next().is_some())
            .then(|| self.button("library-next", "Next page", Action::Next, cx));
        let empty = (!has_rows).then(|| {
            let (title, body): (&'static str, &'static str) = if !loaded {
                (
                    if self.busy {
                        "Loading…"
                    } else {
                        "Nothing loaded"
                    },
                    "Refresh to read this profile's records.",
                )
            } else if searching {
                (
                    "No matches",
                    "Change the search or filters, or continue to older records.",
                )
            } else if history {
                ("No queries yet", "Queries you run appear here.")
            } else {
                (
                    "No saved queries",
                    "Save a query from its tab to keep it here.",
                )
            };
            crate::ui::empty_state("icons/magnifying_glass.svg", title, Some(body.into()), None)
        });
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
            .bg(style::bg())
            .text_color(style::text())
            .text_sm()
            .child(crate::ui::tool_header(title, None, header_actions))
            .child(
                div()
                    .flex_none()
                    .px(px(style::INSET_CONTENT))
                    .py(px(style::S3))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(style::S2))
                    .child(
                        crate::ui::input_well(false, search_focused)
                            .flex_1()
                            .min_w(px(200.))
                            .max_w(px(360.))
                            .child(
                                gpui::svg()
                                    .path("icons/magnifying_glass.svg")
                                    .size(px(style::ICON))
                                    .flex_none()
                                    .text_color(style::dim()),
                            )
                            .child(div().flex_1().min_w_0().child(self.accessible.clone())),
                    )
                    .child(search)
                    .child(crate::ui::separator())
                    .child(connection)
                    .children(outcome),
            )
            .child(
                div()
                    .id("library-rows")
                    .flex_1()
                    .min_h_0()
                    .px(px(style::S3))
                    .flex()
                    .flex_col()
                    .when_some(empty, |list, empty| list.child(empty))
                    .when(has_rows, |list| {
                        list.child(crate::ui::data_table_header(&columns)).child(
                            div()
                                .id("library-rows-body")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .children(rows),
                        )
                    }),
            )
            .when(selection, |view| {
                view.child(
                    div()
                        .id("library-selection")
                        .flex_none()
                        .max_h(px(260.))
                        .overflow_y_scroll()
                        .px(px(style::INSET_CONTENT))
                        .pb(px(style::S4))
                        .border_t_1()
                        .border_color(style::line())
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(style::TOOLBAR))
                                .flex()
                                .items_center()
                                .gap(px(style::S2))
                                .child(crate::ui::heading("Details"))
                                .child(crate::ui::grow())
                                .children(selection_actions),
                        )
                        .child(crate::ui::property_list(details))
                        .when_some(selected_started, |list, raw| {
                            list.child(crate::ui::property_row(
                                "Started",
                                chrome::timestamp("library-started", &raw),
                            ))
                        })
                        .when_some(selected_error, |list, message| {
                            list.child(crate::ui::property_row(
                                "Error",
                                div()
                                    .id("library-error")
                                    .role(Role::Label)
                                    .aria_label(message.clone())
                                    .text_color(style::bad_text())
                                    .whitespace_normal()
                                    .child(message),
                            ))
                        })
                        .when_some(selected_sql, |list, sql| {
                            list.child(crate::ui::property_row(
                                "SQL",
                                div()
                                    .font_family(style::MONO)
                                    .text_size(px(style::FONT_DATA))
                                    .whitespace_normal()
                                    .child(sql),
                            ))
                        }),
                )
            })
            .child(
                crate::ui::status_line()
                    .child(
                        div()
                            .id("library-status")
                            .role(Role::Label)
                            .aria_label(self.status.clone())
                            .child(self.status.clone()),
                    )
                    .child(crate::ui::grow())
                    .children(next),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_durations_read_naturally() {
        assert_eq!(record_count(1), "1 record");
        assert_eq!(record_count(0), "0 records");
        assert_eq!(record_count(1200), "1,200 records");
        assert_eq!(duration(6), "6 ms");
        assert_eq!(duration(1300), "1.3 s");
    }

    #[test]
    fn history_leads_with_sql_and_bounds_the_other_columns() {
        let columns = history_columns();
        assert_eq!(columns[0].label.as_ref(), "Query");
        assert_eq!(columns[0].track, crate::ui::Track::Flex);
        assert!(
            columns[1..]
                .iter()
                .all(|column| matches!(column.track, crate::ui::Track::Fixed(_)))
        );
        assert!(columns[4].end && columns[5].end);
    }
}
