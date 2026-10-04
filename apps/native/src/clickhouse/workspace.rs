//! Plan 031 step 4: one ClickHouse connection's session, sidebar tree and
//! tabs (query, table data, structure). The workspace shows them while the
//! connection is selected; tabs are session-scoped and never persisted.
//! Connecting is explicit and never retried; a session from a superseded
//! attempt is closed when it settles (see [`super::sessions`]).
use super::{
    document::{ClickHouseDocument, DocumentEvent, Mode},
    sessions::{ClickHouseSessions, SessionsChanged},
    tree_view::{ClickHouseTree, TreeEvent},
};
use crate::{
    controller::Host,
    document_view::{ConnectionPhase, TabInfo},
    style,
};
use dbunk_lib::backend::clickhouse::ClickHouseSession;
use gpui::{Context, Entity, EventEmitter, Subscription, Window, div, prelude::*, px};
use std::{cell::Cell, rc::Rc, sync::Arc};

/// Tabs per connection, like the workspace's query tab limit.
const MAX_TABS: usize = 16;

pub enum ClickHouseEvent {
    /// A statement completed in this many milliseconds.
    Latency(u64),
}

struct Tab {
    id: String,
    title: String,
    view: Entity<ClickHouseDocument>,
    _events: Subscription,
}

pub struct ClickHouseWorkspace {
    host: Arc<Host>,
    connection: String,
    sessions: Entity<ClickHouseSessions>,
    tree: Entity<ClickHouseTree>,
    retained: Rc<Cell<usize>>,
    tabs: Vec<Tab>,
    active: Option<String>,
    message: Option<String>,
    _tree_events: Subscription,
    _session_events: Subscription,
}
impl EventEmitter<ClickHouseEvent> for ClickHouseWorkspace {}

impl ClickHouseWorkspace {
    pub fn new(
        host: Arc<Host>,
        connection: String,
        retained: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sessions = cx.new(|_| ClickHouseSessions::new(host.clone()));
        let tree = cx.new(|cx| {
            let mut tree = ClickHouseTree::new(host.clone(), sessions.clone(), window, cx);
            tree.set_connection(Some(connection.clone()), cx);
            tree
        });
        let tree_events = cx.subscribe_in(
            &tree,
            window,
            |this, _, event: &TreeEvent, window, cx| match event {
                TreeEvent::Open {
                    database,
                    name,
                    kind,
                    structure,
                } => {
                    let mode = object_mode(database.clone(), name.clone(), *kind, *structure);
                    this.open(mode, window, cx);
                }
                TreeEvent::NewQuery => this.open(Mode::Query, window, cx),
            },
        );
        let session_events = cx.subscribe(&sessions, |_, _, _: &SessionsChanged, cx| cx.notify());
        Self {
            host,
            connection,
            sessions,
            tree,
            retained,
            tabs: Vec::new(),
            active: None,
            message: None,
            _tree_events: tree_events,
            _session_events: session_events,
        }
    }

    pub fn phase(&self, cx: &gpui::App) -> ConnectionPhase {
        self.sessions.read(cx).phase(&self.connection)
    }

    pub fn tree(&self) -> Entity<ClickHouseTree> {
        self.tree.clone()
    }

    /// Opens a session unless one is open or opening. One bounded attempt.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        let connection = self.connection.clone();
        self.sessions
            .update(cx, |sessions, cx| sessions.connect(&connection, cx));
    }

    /// Ends the session and hands it back for a joined close. Tabs keep
    /// their contents; a reconnect gives them the new session.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) -> Option<ClickHouseSession> {
        let connection = self.connection.clone();
        self.sessions
            .update(cx, |sessions, cx| sessions.disconnect(&connection, cx))
    }

    pub fn tabs(&self, cx: &gpui::App) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|tab| TabInfo {
                id: tab.id.clone(),
                title: tab.title.clone(),
                icon: match tab.view.read(cx).mode() {
                    Mode::Query => "icons/terminal.svg",
                    Mode::Data { .. } => "icons/table.svg",
                    Mode::Structure { .. } => "icons/list_tree.svg",
                },
                status: tab.view.read(cx).status().to_owned(),
                active: self.active.as_ref() == Some(&tab.id),
                pinned: false,
                closable: true,
            })
            .collect()
    }

    pub fn owns_tab(&self, id: &str) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    pub fn new_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(Mode::Query, window, cx);
    }

    pub fn select_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.owns_tab(id) {
            self.active = Some(id.to_owned());
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    /// Steps through tabs, wrapping at the ends.
    pub fn cycle(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.tabs.len();
        if count == 0 {
            return;
        }
        let current = self
            .tabs
            .iter()
            .position(|tab| Some(&tab.id) == self.active.as_ref())
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        let id = self.tabs[next].id.clone();
        self.select_tab(&id, window, cx);
    }

    /// Closes a tab; dropping its document aborts its running request.
    pub fn close_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active.as_deref() == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id.clone());
            self.focus_active(window, cx);
        }
        self.message = None;
        cx.notify();
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active.clone() {
            self.close_tab(&id, window, cx);
        }
    }

    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.active_tab() {
            tab.view.update(cx, |view, cx| view.clear_results(cx));
        }
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut gpui::App) {
        if let Some(tab) = self.active_tab() {
            tab.view
                .update(cx, |view, cx| view.focus_document(window, cx));
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs
            .iter()
            .find(|tab| Some(&tab.id) == self.active.as_ref())
    }

    /// Focuses the tab already showing `mode` (queries always open a new
    /// tab), else opens one within [`MAX_TABS`].
    fn open(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        if mode != Mode::Query
            && let Some(id) = self
                .tabs
                .iter()
                .find(|tab| tab.view.read(cx).mode() == &mode)
                .map(|tab| tab.id.clone())
        {
            self.select_tab(&id, window, cx);
            return;
        }
        if self.tabs.len() >= MAX_TABS {
            self.message = Some(format!(
                "Close a tab before opening another (limit {MAX_TABS})"
            ));
            cx.notify();
            return;
        }
        let title = match &mode {
            Mode::Query => {
                let number = (1..)
                    .find(|number| {
                        !self
                            .tabs
                            .iter()
                            .any(|tab| tab.title == format!("Query {number}"))
                    })
                    .unwrap_or(1);
                format!("Query {number}")
            }
            _ => String::new(),
        };
        let view = cx.new(|cx| {
            ClickHouseDocument::new(
                self.host.clone(),
                self.sessions.clone(),
                self.connection.clone(),
                mode,
                self.retained.clone(),
                window,
                cx,
            )
        });
        let title = if title.is_empty() {
            view.read(cx).title()
        } else {
            title
        };
        let events = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &DocumentEvent, window, cx| match event {
                DocumentEvent::Open { structure } => {
                    if let Some((database, name, kind)) = view.read(cx).mode().object() {
                        let mode =
                            object_mode(database.to_owned(), name.to_owned(), kind, *structure);
                        this.open(mode, window, cx);
                    }
                }
                DocumentEvent::Latency(ms) => cx.emit(ClickHouseEvent::Latency(*ms)),
            },
        );
        let id = uuid::Uuid::new_v4().to_string();
        self.tabs.push(Tab {
            id: id.clone(),
            title,
            view,
            _events: events,
        });
        self.message = None;
        self.select_tab(&id, window, cx);
    }
}

fn object_mode(
    database: String,
    name: String,
    kind: super::tree_model::ObjectKind,
    structure: bool,
) -> Mode {
    if structure {
        Mode::Structure {
            database,
            name,
            kind,
        }
    } else {
        Mode::Data {
            database,
            name,
            kind,
        }
    }
}

impl Render for ClickHouseWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.active_tab() {
            Some(tab) => crate::ui::appear(
                gpui::SharedString::from(format!("clickhouse-document-{}", tab.id)),
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(tab.view.clone()),
            )
            .into_any_element(),
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(style::faint())
                .child(match self.phase(cx) {
                    ConnectionPhase::Connecting => "Connecting…".to_owned(),
                    ConnectionPhase::Failed(error) => format!("Connection failed: {error}"),
                    ConnectionPhase::Idle => "Not connected. Connect from the sidebar.".to_owned(),
                    ConnectionPhase::Connected => {
                        "Open a table from the sidebar or press + for a query".to_owned()
                    }
                })
                .into_any_element(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(style::bg())
            .child(body)
            .when_some(self.message.clone(), |root, message| {
                root.child(
                    crate::ui::error_banner("clickhouse-message", message)
                        .flex_none()
                        .m(px(8.)),
                )
            })
    }
}
