//! Sidebar object tree for the selected ClickHouse connection. The catalog is
//! read once per session (and on explicit Refresh) by an owned task that is
//! aborted when the connection, session or view goes away.
use super::{
    sessions::{ClickHouseSessions, SessionsChanged},
    tree_model::{self, ObjectKind, Row, RowKind, TreeState},
};
use crate::{
    accessible_editor::AccessibleEditor, controller::Host, document_view::ConnectionPhase, style,
};
use dbunk_lib::backend::clickhouse::{ClickHouseCatalog, ClickHouseErrorKind, ClickHouseSession};
use editor::{Editor, EditorEvent};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Role, SharedString, Subscription,
    Task, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use std::{collections::HashMap, sync::Arc};

pub enum TreeEvent {
    /// Open an object's data (`structure == false`) or structure.
    Open {
        database: String,
        name: String,
        kind: ObjectKind,
        structure: bool,
    },
    NewQuery,
}

#[derive(Default)]
struct Loaded {
    catalog: Option<Arc<ClickHouseCatalog>>,
    /// The session the catalog was read with; a new session reloads it.
    session: Option<ClickHouseSession>,
    state: TreeState,
    error: Option<String>,
}

struct Load {
    connection: String,
    abort: tokio::task::AbortHandle,
    _task: Task<()>,
}

pub struct ClickHouseTree {
    host: Arc<Host>,
    sessions: Entity<ClickHouseSessions>,
    connection: Option<String>,
    loaded: HashMap<String, Loaded>,
    load: Option<Load>,
    rows: Vec<Row>,
    selected: usize,
    filter: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    list: FocusHandle,
    scroll: UniformListScrollHandle,
    _filter_events: Subscription,
    _sessions: Subscription,
}
impl EventEmitter<TreeEvent> for ClickHouseTree {}

impl Drop for ClickHouseTree {
    fn drop(&mut self) {
        if let Some(load) = self.load.take() {
            load.abort.abort();
        }
    }
}

impl ClickHouseTree {
    pub fn new(
        host: Arc<Host>,
        sessions: Entity<ClickHouseSessions>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Filter objects", window, cx);
            editor
        });
        let accessible = cx.new(|cx| {
            AccessibleEditor::field(filter.clone(), "Filter databases and objects", false, cx)
        });
        let filter_events = cx.subscribe(&filter, |this, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                this.rebuild(cx);
            }
        });
        let session_events = cx.subscribe(&sessions, |this, _, _: &SessionsChanged, cx| {
            this.sync(cx);
        });
        Self {
            host,
            sessions,
            connection: None,
            loaded: HashMap::new(),
            load: None,
            rows: Vec::new(),
            selected: 0,
            filter,
            accessible,
            list: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            _filter_events: filter_events,
            _sessions: session_events,
        }
    }

    /// Shows `connection`'s tree; `None` hides it. Cheap when unchanged.
    pub fn set_connection(&mut self, connection: Option<String>, cx: &mut Context<Self>) {
        if self.connection == connection {
            return;
        }
        self.connection = connection;
        self.selected = 0;
        self.sync(cx);
    }

    fn phase(&self, cx: &gpui::App) -> ConnectionPhase {
        self.connection
            .as_deref()
            .map_or(ConnectionPhase::Idle, |id| self.sessions.read(cx).phase(id))
    }

    /// Reacts to session changes: drop catalogs of ended sessions, load the
    /// current connection's catalog once per session.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let sessions = self.sessions.read(cx);
        // A disconnect forgets the catalog; a failed session keeps its last
        // catalog for orientation until the user reconnects or disconnects.
        self.loaded
            .retain(|id, _| sessions.phase(id) != ConnectionPhase::Idle);
        let current = self
            .connection
            .as_deref()
            .and_then(|id| sessions.session(id).map(|session| (id.to_owned(), session)));
        if let Some(load) = &self.load
            && current
                .as_ref()
                .is_none_or(|(id, _)| id != &load.connection)
        {
            load.abort.abort();
            self.load = None;
        }
        if let Some((id, session)) = current {
            let fresh = self
                .loaded
                .get(&id)
                .and_then(|loaded| loaded.session.as_ref())
                .is_some_and(|used| used.same(&session));
            if !fresh && self.load.is_none() {
                self.start_load(id, session, cx);
            }
        }
        self.rebuild(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.connection.clone() else {
            return;
        };
        let Some(session) = self.sessions.read(cx).session(&id) else {
            return;
        };
        if let Some(load) = self.load.take() {
            load.abort.abort();
        }
        self.start_load(id, session, cx);
        cx.notify();
    }

    fn start_load(&mut self, id: String, session: ClickHouseSession, cx: &mut Context<Self>) {
        let reader = session.clone();
        let read = self
            .host
            .runtime
            .spawn(async move { reader.catalog().await });
        let abort = read.abort_handle();
        let connection = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if this
                    .load
                    .as_ref()
                    .is_none_or(|load| load.connection != connection)
                {
                    return;
                }
                this.load = None;
                let loaded = this.loaded.entry(connection.clone()).or_default();
                loaded.session = Some(session.clone());
                match result {
                    Ok(Ok(catalog)) => {
                        loaded.state.adopt(&catalog);
                        loaded.catalog = Some(Arc::new(catalog));
                        loaded.error = None;
                    }
                    Ok(Err(error)) => {
                        loaded.error = Some(error.message.clone());
                        if error.kind == ClickHouseErrorKind::Lost {
                            this.sessions.update(cx, |sessions, cx| {
                                sessions.lost(&connection, &session, error.message, cx)
                            });
                        }
                    }
                    Err(_) => loaded.error = Some("Catalog read stopped".into()),
                }
                this.rebuild(cx);
            })
            .ok();
        });
        self.load = Some(Load {
            connection: id,
            abort,
            _task: task,
        });
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let key = self.rows.get(self.selected).map(|row| row.key.clone());
        let filter = self.filter.read(cx).text(cx);
        self.rows = self
            .connection
            .as_ref()
            .and_then(|id| self.loaded.get(id))
            .and_then(|loaded| Some(loaded.state.rows(loaded.catalog.as_ref()?, &filter)))
            .unwrap_or_default();
        self.selected = tree_model::reselect(&self.rows, key.as_deref(), self.selected);
        cx.notify();
    }

    fn status(&self, cx: &gpui::App) -> Option<String> {
        let loaded = self.connection.as_ref().and_then(|id| self.loaded.get(id));
        let catalog = loaded.and_then(|loaded| loaded.catalog.as_ref());
        Some(match self.phase(cx) {
            _ if self.load.is_some() => "Reading databases…".into(),
            ConnectionPhase::Idle => "Not connected".into(),
            ConnectionPhase::Connecting => "Connecting…".into(),
            ConnectionPhase::Failed(_) if catalog.is_some() => {
                "Connection failed; showing the last catalog".into()
            }
            ConnectionPhase::Failed(error) => format!("Connection failed: {error}"),
            ConnectionPhase::Connected => match loaded.and_then(|loaded| loaded.error.as_ref()) {
                Some(error) => format!("Catalog failed: {}", error.lines().next().unwrap_or("")),
                None if catalog.is_some() && self.rows.is_empty() => "No matching objects".into(),
                None => return None,
            },
        })
    }

    fn activate(&mut self, index: usize, structure: bool, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        self.selected = index;
        match &row.kind {
            RowKind::Database { .. } | RowKind::Group { .. } => {
                if let Some(loaded) = self
                    .connection
                    .as_ref()
                    .and_then(|id| self.loaded.get_mut(id))
                {
                    loaded.state.toggle(&row);
                }
                self.rebuild(cx);
            }
            RowKind::Object { kind, name } => {
                if self.connection.is_some() {
                    cx.emit(TreeEvent::Open {
                        database: row.database.clone(),
                        name: name.clone(),
                        kind: *kind,
                        structure,
                    });
                }
            }
            RowKind::Note { .. } => {}
        }
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.list.is_focused(window) || self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let row = &self.rows[self.selected.min(last)];
        let expanded = match row.kind {
            RowKind::Database { expanded } | RowKind::Group { expanded, .. } => Some(expanded),
            _ => None,
        };
        match event.keystroke.key.as_str() {
            "down" => self.selected = (self.selected + 1).min(last),
            "up" => self.selected = self.selected.saturating_sub(1),
            "home" => self.selected = 0,
            "end" => self.selected = last,
            "right" if expanded == Some(false) => self.activate(self.selected, false, cx),
            "left" if expanded == Some(true) => self.activate(self.selected, false, cx),
            "left" => {
                if let Some(parent) = tree_model::parent(&self.rows, self.selected) {
                    self.selected = parent;
                }
            }
            "enter" => self.activate(self.selected, event.keystroke.modifiers.shift, cx),
            _ => return,
        }
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        cx.stop_propagation();
        cx.notify();
    }

    fn header_button(
        &self,
        id: &'static str,
        label: &'static str,
        icon: &'static str,
        enabled: bool,
        cx: &Context<Self>,
        action: fn(&mut Self, &mut Context<Self>),
    ) -> impl IntoElement {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .tab_stop(enabled)
            .tab_index(0)
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(enabled, |button| {
                crate::ui::press(button.cursor_pointer().hover(|s| s.bg(style::hover())))
                    .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
            })
            .when(!enabled, |button| {
                button.a11y_synthetic_children(|builder| builder.parent_node().set_disabled())
            })
            .tooltip(crate::ui::tooltip(label))
            .tooltip_show_delay(crate::ui::tooltip_delay())
            .child(
                gpui::svg()
                    .path(icon)
                    .size(px(style::ICON))
                    .text_color(if enabled {
                        style::dim()
                    } else {
                        style::faint()
                    }),
            )
    }
}

fn row_icon(row: &Row) -> (&'static str, gpui::Rgba) {
    use crate::style::TreeKind as T;
    style::kind_icon(match row.kind {
        RowKind::Database { .. } => T::Database,
        RowKind::Group { .. } => T::Group,
        RowKind::Note { warning: true } => T::Warning,
        RowKind::Note { warning: false } => T::Info,
        RowKind::Object { kind, .. } => match kind {
            ObjectKind::Table => T::Table,
            ObjectKind::View => T::View,
            ObjectKind::MaterializedView => T::MaterializedView,
            ObjectKind::Dictionary => T::Dictionary,
        },
    })
}

fn row_label(row: &Row) -> String {
    let base = match &row.kind {
        RowKind::Database { expanded } => format!(
            "Database {}, {}",
            row.label,
            if *expanded { "expanded" } else { "collapsed" }
        ),
        RowKind::Group { expanded, .. } => format!(
            "{}, {}",
            row.label,
            if *expanded { "expanded" } else { "collapsed" }
        ),
        RowKind::Object { kind, .. } => format!("{} {}", kind.label(), row.label),
        RowKind::Note { .. } => row.label.clone(),
    };
    match &row.detail {
        Some(detail) => format!("{base}, {detail}"),
        None => base,
    }
}

impl Render for ClickHouseTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.phase(cx) == ConnectionPhase::Connected;
        let status = self.status(cx);
        let focused = self.list.is_focused(window);
        let selected_label = self
            .rows
            .get(self.selected)
            .map(row_label)
            .unwrap_or_default();
        div()
            .id("clickhouse-objects")
            .role(Role::Group)
            .aria_label("ClickHouse databases and objects")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .on_key_down(cx.listener(Self::key_down))
            .child(
                div()
                    .h(px(26.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .pl(px(10.))
                    .pr(px(6.))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(style::text())
                            .child("Objects"),
                    )
                    .child(self.header_button(
                        "clickhouse-new-query",
                        "New ClickHouse query",
                        "icons/terminal.svg",
                        self.connection.is_some(),
                        cx,
                        |this, cx| {
                            if this.connection.is_some() {
                                cx.emit(TreeEvent::NewQuery);
                            }
                        },
                    ))
                    .child(self.header_button(
                        "clickhouse-refresh",
                        "Refresh objects",
                        "icons/rotate_cw.svg",
                        connected && self.load.is_none(),
                        cx,
                        Self::refresh,
                    )),
            )
            .child(
                div()
                    .mx(px(8.))
                    .mb(px(4.))
                    .h(px(20.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(style::line_soft())
                    .child(
                        gpui::svg()
                            .path("icons/filter.svg")
                            .size(px(style::ICON))
                            .text_color(style::faint()),
                    )
                    .child(div().flex_1().min_w_0().h(px(16.)).child(self.accessible.clone())),
            )
            .when_some(status, |root, status| {
                root.child(
                    div()
                        .id("clickhouse-objects-status")
                        .role(Role::Status)
                        .aria_label(status.clone())
                        .px(px(10.))
                        .pb(px(2.))
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::faint())
                        .child(status),
                )
            })
            .child(
                div()
                    .id("clickhouse-tree")
                    .role(Role::Tree)
                    .aria_label(format!(
                        "{} rows; arrows move, Right expands, Left collapses, Enter opens data, Shift-Enter opens structure",
                        self.rows.len()
                    ))
                    .aria_value(selected_label)
                    .track_focus(&self.list)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "clickhouse-rows",
                            self.rows.len(),
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|position| {
                                        let row = &this.rows[position];
                                        let (path, color) = row_icon(row);
                                        let selected = position == this.selected;
                                        let expanded = match row.kind {
                                            RowKind::Database { expanded }
                                            | RowKind::Group { expanded, .. } => Some(expanded),
                                            _ => None,
                                        };
                                        let object = matches!(row.kind, RowKind::Object { .. });
                                        div()
                                            .id(("clickhouse-row", position))
                                            .group("clickhouse-row")
                                            .role(Role::TreeItem)
                                            .aria_label(row_label(row))
                                            .aria_selected(selected)
                                            .h(px(style::ROW))
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .pl(px(8. + 12. * row.level as f32))
                                            .pr(px(6.))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_color(match row.kind {
                                                RowKind::Note { warning: true } => style::warn(),
                                                RowKind::Note { .. } => style::faint(),
                                                _ => style::text(),
                                            })
                                            .hover(|s| s.bg(style::hover()))
                                            .when(selected, |row| {
                                                row.bg(if focused { style::select() } else { style::raised() })
                                            })
                                            .child(match expanded {
                                                Some(open) => gpui::svg()
                                                    .path(if open {
                                                        "icons/chevron_down.svg"
                                                    } else {
                                                        "icons/chevron_right.svg"
                                                    })
                                                    .size(px(style::ICON))
                                                    .flex_none()
                                                    .text_color(style::faint())
                                                    .into_any_element(),
                                                None => div().w(px(style::ICON)).flex_none().into_any_element(),
                                            })
                                            .child(
                                                gpui::svg()
                                                    .path(path)
                                                    .size(px(style::ICON))
                                                    .flex_none()
                                                    .text_color(color),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .child(SharedString::from(row.label.clone())),
                                            )
                                            .when_some(row.detail.clone(), |row, detail| {
                                                row.child(
                                                    div()
                                                        .flex_none()
                                                        .max_w(px(110.))
                                                        .overflow_hidden()
                                                        .font_family(style::MONO)
                                                        .text_size(px(style::FONT_SMALL))
                                                        .text_color(style::faint())
                                                        .child(detail),
                                                )
                                            })
                                            .when(object, |row| {
                                                row.child(
                                                    div()
                                                        .id(("clickhouse-structure", position))
                                                        .role(Role::Button)
                                                        .aria_label("Open structure")
                                                        .flex_none()
                                                        .size(px(16.))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .rounded(px(3.))
                                                        .invisible()
                                                        .group_hover("clickhouse-row", |s| s.visible())
                                                        .hover(|s| s.bg(style::raised()))
                                                        .tooltip(crate::ui::tooltip("Structure (Shift-Enter)"))
                                                        .tooltip_show_delay(crate::ui::tooltip_delay())
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.activate(position, true, cx);
                                                        }))
                                                        .child(
                                                            gpui::svg()
                                                                .path("icons/list_tree.svg")
                                                                .size(px(style::ICON))
                                                                .text_color(style::dim()),
                                                        ),
                                                )
                                            })
                                            .on_click(cx.listener(
                                                move |this, event: &gpui::ClickEvent, window, cx| {
                                                    let Some(row) = this.rows.get(position) else {
                                                        return;
                                                    };
                                                    let expandable = row.expandable();
                                                    this.selected = position;
                                                    window.focus(&this.list, cx);
                                                    if event.click_count() > 1 || expandable {
                                                        this.activate(position, false, cx);
                                                    }
                                                    cx.notify();
                                                },
                                            ))
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .h_full(),
                    ),
            )
    }
}
