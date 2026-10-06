//! Plan 031 step 4: the native SQLite workspace for one connection.
//!
//! `SqliteWorkspace` owns the connection's single backend session, its object
//! tree and its tabs (query, table data, structure). The shell asks it for
//! tabs, the tree and the active document while that connection is selected.
//! Connecting never retries on its own; a failure stays until the user
//! retries or disconnects. A reconnect bumps a generation so late results
//! from an older session are dropped (and a late session is closed).
use crate::document_view::{ConnectionPhase, TabInfo};
use crate::sqlite_documents::{
    SqliteDataView, SqliteDocEvent, SqliteQueryView, SqliteStructureView,
};
use crate::sqlite_model::{self, Key, Move, ObjectKind, RowKind, TabKind, TreeRow};
use crate::{controller::Host, style};
use dbunk_lib::backend::sqlite_session::{SqliteObjects, SqliteSession};
use editor::Editor;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, KeyDownEvent, Role, SharedString,
    Subscription, Task, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
    sync::Arc,
};

/// The connection's current session, shared by its documents. `None` while
/// disconnected; documents read it per request so a reconnect is picked up.
pub type SessionSlot = Rc<RefCell<Option<Arc<SqliteSession>>>>;

/// Everything a SQLite document needs from its workspace.
#[derive(Clone)]
pub struct SqliteContext {
    pub host: Arc<Host>,
    pub session: SessionSlot,
    /// The workspace's shared result-retention allowance.
    pub retained: Rc<Cell<usize>>,
}

impl SqliteContext {
    pub fn session(&self) -> Option<Arc<SqliteSession>> {
        self.session.borrow().clone()
    }
}

pub enum SqliteEvent {
    /// Tabs, phase or the active document changed.
    Changed,
    Latency(u64),
}

enum TabView {
    Query(Entity<SqliteQueryView>),
    Data(Entity<SqliteDataView>),
    Structure(Entity<SqliteStructureView>),
}

struct Tab {
    id: String,
    kind: TabKind,
    view: TabView,
    _events: Subscription,
}

pub struct SqliteWorkspace {
    connection_id: String,
    context: SqliteContext,
    phase: ConnectionPhase,
    generation: u64,
    tree: Entity<SqliteTree>,
    tabs: Vec<Tab>,
    active: Option<String>,
    next_tab: u64,
    queries: u32,
    connect_task: Option<Task<()>>,
    _tree_events: Subscription,
}

impl EventEmitter<SqliteEvent> for SqliteWorkspace {}

impl SqliteWorkspace {
    pub fn new(
        connection_id: String,
        host: Arc<Host>,
        retained: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let context = SqliteContext {
            host,
            session: Rc::new(RefCell::new(None)),
            retained,
        };
        let tree = cx.new(|cx| SqliteTree::new(context.clone(), window, cx));
        let tree_events =
            cx.subscribe_in(&tree, window, |this, _, event, window, cx| match event {
                TreeEvent::Open(target) => this.open_target(target.clone(), window, cx),
            });
        Self {
            connection_id,
            context,
            phase: ConnectionPhase::Idle,
            generation: 0,
            tree,
            tabs: Vec::new(),
            active: None,
            next_tab: 0,
            queries: 0,
            connect_task: None,
            _tree_events: tree_events,
        }
    }

    /// A session whose worker stopped on its own reads as failed, so the
    /// row offers a retry instead of claiming a live connection.
    pub fn phase(&self) -> ConnectionPhase {
        if self.phase == ConnectionPhase::Connected
            && self
                .context
                .session()
                .is_some_and(|session| session.is_closed())
        {
            return ConnectionPhase::Failed("The SQLite session stopped".into());
        }
        self.phase.clone()
    }

    pub fn tree(&self) -> Entity<SqliteTree> {
        self.tree.clone()
    }

    /// Opens a session unless one is open or opening. Never retries. The
    /// first successful connect opens a query tab when none is open.
    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.phase(),
            ConnectionPhase::Connecting | ConnectionPhase::Connected
        ) {
            return;
        }
        // A stopped session has nothing left to close.
        self.context.session.borrow_mut().take();
        self.generation += 1;
        let generation = self.generation;
        self.phase = ConnectionPhase::Connecting;
        let host = self.context.host.clone();
        let id = self.connection_id.clone();
        let task = self
            .context
            .host
            .runtime
            .spawn(async move { host.backend.open_sqlite_session(id).await });
        self.connect_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|_| Err("SQLite connect task failed".into()));
            this.update_in(cx, |this, window, cx| {
                // A session from an abandoned attempt is dropped here,
                // which stops its worker.
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(session) => {
                        *this.context.session.borrow_mut() = Some(Arc::new(session));
                        this.phase = ConnectionPhase::Connected;
                        this.tree.update(cx, |tree, cx| tree.load(cx));
                        for tab in &this.tabs {
                            match &tab.view {
                                TabView::Data(view) => view.update(cx, |view, cx| view.reload(cx)),
                                TabView::Structure(view) => {
                                    view.update(cx, |view, cx| view.reload(cx))
                                }
                                TabView::Query(view) => view.update(cx, |_, cx| cx.notify()),
                            }
                        }
                        if this.tabs.is_empty() {
                            this.new_query(window, cx);
                        }
                    }
                    Err(error) => this.phase = ConnectionPhase::Failed(error),
                }
                cx.emit(SqliteEvent::Changed);
                cx.notify();
            })
            .ok();
        }));
        cx.emit(SqliteEvent::Changed);
        cx.notify();
    }

    /// Detaches the session for the caller to close (a joined, bounded
    /// close; see `workspace_engines.rs`) and returns to Idle. Documents keep their contents but can no longer
    /// run. An attempt in flight is abandoned; its session is closed when it
    /// arrives.
    pub fn take_session(&mut self, cx: &mut Context<Self>) -> Option<Arc<SqliteSession>> {
        self.generation += 1;
        // Dropping the waiter abandons the attempt: the open future still
        // finishes on Tokio and its session is dropped with the result.
        self.connect_task = None;
        let session = self.context.session.borrow_mut().take();
        self.phase = ConnectionPhase::Idle;
        self.tree.update(cx, |tree, cx| tree.clear(cx));
        for tab in &self.tabs {
            match &tab.view {
                TabView::Query(view) => view.update(cx, |view, cx| view.detached(cx)),
                TabView::Data(view) => view.update(cx, |view, cx| view.detached(cx)),
                TabView::Structure(view) => view.update(cx, |view, cx| view.detached(cx)),
            }
        }
        cx.emit(SqliteEvent::Changed);
        cx.notify();
        session
    }

    pub fn tabs(&self, cx: &App) -> Vec<TabInfo> {
        self.tabs
            .iter()
            .map(|tab| TabInfo {
                id: tab.id.clone(),
                title: tab.kind.title(),
                icon: tab.kind.icon(),
                status: match &tab.view {
                    TabView::Query(view) => view.read(cx).status(),
                    TabView::Data(view) => view.read(cx).status(),
                    TabView::Structure(view) => view.read(cx).status(),
                },
                active: self.active.as_ref() == Some(&tab.id),
                pinned: false,
                closable: true,
            })
            .collect()
    }

    pub fn owns_tab(&self, id: &str) -> bool {
        self.tabs.iter().any(|tab| tab.id == id)
    }

    pub fn select_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.owns_tab(id) {
            self.active = Some(id.to_owned());
            self.focus_active(window, cx);
            cx.emit(SqliteEvent::Changed);
            cx.notify();
        }
    }

    /// Moves to the next (or previous) tab, wrapping.
    pub fn select_next(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_index() else {
            return;
        };
        let len = self.tabs.len();
        let next = if backwards {
            (index + len - 1) % len
        } else {
            (index + 1) % len
        };
        let id = self.tabs[next].id.clone();
        self.select_tab(&id, window, cx);
    }

    fn active_index(&self) -> Option<usize> {
        let active = self.active.as_ref()?;
        self.tabs.iter().position(|tab| &tab.id == active)
    }

    /// Closes a tab, stopping its running request and releasing its retained
    /// results. Returns false when the tab is not this workspace's.
    pub fn close_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        let tab = self.tabs.remove(index);
        match &tab.view {
            TabView::Query(view) => view.update(cx, |view, cx| view.release(cx)),
            TabView::Data(view) => view.update(cx, |view, cx| view.release(cx)),
            TabView::Structure(view) => view.update(cx, |view, cx| view.release(cx)),
        }
        if self.active.as_deref() == Some(id) {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map(|tab| tab.id.clone());
            self.focus_active(window, cx);
        }
        cx.emit(SqliteEvent::Changed);
        cx.notify();
        true
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.active.clone() {
            Some(id) => self.close_tab(&id, window, cx),
            None => false,
        }
    }

    /// Releases every tab's retained results (workspace teardown).
    pub fn release_all(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            match &tab.view {
                TabView::Query(view) => view.update(cx, |view, cx| view.release(cx)),
                TabView::Data(view) => view.update(cx, |view, cx| view.release(cx)),
                TabView::Structure(view) => view.update(cx, |view, cx| view.release(cx)),
            }
        }
    }

    pub fn new_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.queries += 1;
        let number = self.queries;
        let context = self.context.clone();
        let view = cx.new(|cx| SqliteQueryView::new(context, window, cx));
        let events = self.subscribe_document(&view, window, cx);
        self.push_tab(
            TabKind::Query(number),
            TabView::Query(view),
            events,
            window,
            cx,
        );
    }

    fn open_target(&mut self, target: OpenTarget, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            OpenTarget::Data { schema, name } => self.open_data(schema, name, window, cx),
            OpenTarget::Structure { schema, name } => self.open_structure(schema, name, window, cx),
        }
    }

    pub fn open_data(
        &mut self,
        schema: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = TabKind::Data {
            schema: schema.clone(),
            name: name.clone(),
        };
        if self.focus_existing(&kind, window, cx) {
            return;
        }
        let context = self.context.clone();
        let view = cx.new(|cx| SqliteDataView::new(context, schema, name, cx));
        let events = self.subscribe_document(&view, window, cx);
        self.push_tab(kind, TabView::Data(view), events, window, cx);
    }

    pub fn open_structure(
        &mut self,
        schema: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = TabKind::Structure {
            schema: schema.clone(),
            name: name.clone(),
        };
        if self.focus_existing(&kind, window, cx) {
            return;
        }
        let context = self.context.clone();
        let view = cx.new(|cx| SqliteStructureView::new(context, schema, name, cx));
        let events = self.subscribe_document(&view, window, cx);
        self.push_tab(kind, TabView::Structure(view), events, window, cx);
    }

    fn focus_existing(
        &mut self,
        kind: &TabKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.tabs.iter().find(|tab| &tab.kind == kind) {
            Some(tab) => {
                let id = tab.id.clone();
                self.select_tab(&id, window, cx);
                true
            }
            None => false,
        }
    }

    fn subscribe_document<V: EventEmitter<SqliteDocEvent>>(
        &mut self,
        view: &Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(view, window, |this, _, event, window, cx| match event {
            SqliteDocEvent::Latency(ms) => cx.emit(SqliteEvent::Latency(*ms)),
            SqliteDocEvent::ObjectsChanged => this.tree.update(cx, |tree, cx| tree.load(cx)),
            SqliteDocEvent::OpenData { schema, name } => {
                this.open_data(schema.clone(), name.clone(), window, cx)
            }
            SqliteDocEvent::OpenStructure { schema, name } => {
                this.open_structure(schema.clone(), name.clone(), window, cx)
            }
            SqliteDocEvent::Changed => {
                cx.emit(SqliteEvent::Changed);
                cx.notify();
            }
        })
    }

    fn push_tab(
        &mut self,
        kind: TabKind,
        view: TabView,
        events: Subscription,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_tab += 1;
        let id = format!("sqlite-{}-{}", self.connection_id, self.next_tab);
        self.tabs.push(Tab {
            id: id.clone(),
            kind,
            view,
            _events: events,
        });
        self.select_tab(&id, window, cx);
    }

    pub fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_index() else {
            return;
        };
        let handle: FocusHandle = match &self.tabs[index].view {
            TabView::Query(view) => view.read(cx).focus(cx),
            TabView::Data(view) => view.read(cx).focus(cx),
            TabView::Structure(view) => view.read(cx).focus(),
        };
        window.focus(&handle, cx);
    }

    fn empty_state(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (message, retry) = match &self.phase {
            ConnectionPhase::Connecting => ("Opening the SQLite database…".to_string(), false),
            ConnectionPhase::Failed(error) => {
                (format!("Could not open the database: {error}"), true)
            }
            ConnectionPhase::Idle => (
                "Disconnected. Connect to browse this database.".into(),
                true,
            ),
            ConnectionPhase::Connected => (
                "Open a table from the sidebar, or press ⌘T for a query".into(),
                false,
            ),
        };
        div()
            .id("sqlite-empty")
            .role(Role::Status)
            .aria_label(message.clone())
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .text_color(style::faint())
            .child(message)
            .when(retry, |root| {
                root.child(
                    crate::ui::button(
                        "sqlite-connect",
                        if matches!(self.phase, ConnectionPhase::Failed(_)) {
                            "Retry"
                        } else {
                            "Connect"
                        },
                        crate::ui::Variant::Secondary,
                        true,
                    )
                    .tab_index(0)
                    .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
                )
            })
            .into_any_element()
    }
}

impl Render for SqliteWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.active_index().map(|index| &self.tabs[index]) {
            Some(tab) => {
                let element = match &tab.view {
                    TabView::Query(view) => view.clone().into_any_element(),
                    TabView::Data(view) => view.clone().into_any_element(),
                    TabView::Structure(view) => view.clone().into_any_element(),
                };
                crate::ui::appear(
                    SharedString::from(format!("document-{}", tab.id)),
                    div().size_full().child(element),
                )
                .into_any_element()
            }
            None => self.empty_state(cx),
        };
        div()
            .id("sqlite-workspace")
            .size_full()
            .bg(style::bg())
            .child(content)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Data { schema: String, name: String },
    Structure { schema: String, name: String },
}

pub enum TreeEvent {
    Open(OpenTarget),
}

/// The sidebar object tree: `main` plus attached databases, each with
/// tables, views, indexes and triggers from one bounded catalog read.
pub struct SqliteTree {
    context: SqliteContext,
    objects: Option<SqliteObjects>,
    error: Option<String>,
    loading: Option<(u64, Task<()>)>,
    expanded: BTreeSet<String>,
    rows: Vec<TreeRow>,
    selected: usize,
    filter: Entity<Editor>,
    filter_field: Entity<crate::accessible_editor::AccessibleEditor>,
    list: FocusHandle,
    scroll: UniformListScrollHandle,
    _filter_events: Subscription,
}

impl EventEmitter<TreeEvent> for SqliteTree {}

impl SqliteTree {
    fn new(context: SqliteContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Filter objects", window, cx);
            editor
        });
        let filter_field = cx.new(|cx| {
            crate::accessible_editor::AccessibleEditor::field(
                filter.clone(),
                "Filter objects",
                false,
                cx,
            )
        });
        let filter_events = cx.subscribe(&filter, |this, _, event: &editor::EditorEvent, cx| {
            if matches!(event, editor::EditorEvent::BufferEdited) {
                this.rebuild(cx);
            }
        });
        Self {
            context,
            objects: None,
            error: None,
            loading: None,
            expanded: sqlite_model::default_expanded(),
            rows: Vec::new(),
            selected: 0,
            filter,
            filter_field,
            list: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            _filter_events: filter_events,
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let filter = self.filter.read(cx).text(cx);
        self.rows = self
            .objects
            .as_ref()
            .map(|objects| sqlite_model::tree_rows(objects, &self.expanded, &filter))
            .unwrap_or_default();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        cx.notify();
    }

    /// Reads the catalog again. A newer load replaces an older one; there is
    /// no automatic retry.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.context.session() else {
            return;
        };
        if let Some((ticket, _)) = self.loading.take() {
            session.cancel(ticket);
        }
        let ticket = session.ticket();
        let task = self
            .context
            .host
            .runtime
            .spawn(async move { session.objects(ticket).await });
        self.error = None;
        self.loading = Some((
            ticket,
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| {
                    if this
                        .loading
                        .as_ref()
                        .is_none_or(|(current, _)| *current != ticket)
                    {
                        return;
                    }
                    this.loading = None;
                    match result {
                        Ok(Ok(objects)) => {
                            this.objects = Some(objects);
                            this.error = None;
                        }
                        Ok(Err(error)) => this.error = Some(error.to_string()),
                        Err(_) => this.error = Some("Object read failed".into()),
                    }
                    this.rebuild(cx);
                })
                .ok();
            }),
        ));
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.loading = None;
        self.objects = None;
        self.error = None;
        self.rebuild(cx);
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        if row.expanded.is_none() {
            return;
        }
        if !self.expanded.remove(&row.key) {
            self.expanded.insert(row.key.clone());
        }
        self.rebuild(cx);
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        match &row.kind {
            RowKind::Database { .. } | RowKind::Group { .. } => self.toggle(index, cx),
            RowKind::Object {
                schema,
                kind,
                name,
                table,
            } => {
                let target = if kind.browsable() {
                    OpenTarget::Data {
                        schema: schema.clone(),
                        name: name.clone(),
                    }
                } else {
                    OpenTarget::Structure {
                        schema: schema.clone(),
                        name: table.clone(),
                    }
                };
                cx.emit(TreeEvent::Open(target));
            }
            RowKind::Truncated { .. } => {}
        }
    }

    fn open_structure(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(TreeRow {
            kind: RowKind::Object { schema, table, .. },
            ..
        }) = self.rows.get(index)
        {
            cx.emit(TreeEvent::Open(OpenTarget::Structure {
                schema: schema.clone(),
                name: table.clone(),
            }));
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = &event.keystroke.modifiers;
        if !self.list.is_focused(window) || modifiers.control || modifiers.alt || modifiers.platform
        {
            return;
        }
        let key = match event.keystroke.key.as_str() {
            "up" => Some(Key::Up),
            "down" => Some(Key::Down),
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            _ => None,
        };
        if let Some(key) = key {
            match sqlite_model::navigate(&self.rows, self.selected, key) {
                Some(Move::Select(index)) => self.selected = index,
                Some(Move::Toggle(index)) => self.toggle(index, cx),
                None => {}
            }
        } else if event.keystroke.key == "enter" && modifiers.shift {
            self.open_structure(self.selected, cx);
        } else if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            self.activate(self.selected, cx);
        } else {
            return;
        }
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        cx.stop_propagation();
        cx.notify();
    }

    fn row_label(row: &TreeRow) -> String {
        match &row.kind {
            RowKind::Database { file, .. } if !file.is_empty() => {
                format!("Database {}, {file}", row.label)
            }
            RowKind::Database { .. } => format!("Database {}", row.label),
            RowKind::Group { .. } => format!("{}, {}", row.label, row.count.unwrap_or(0)),
            RowKind::Object {
                kind, table, name, ..
            } if !kind.browsable() => {
                format!("{} {name} on {table}", singular(*kind))
            }
            RowKind::Object { kind, name, .. } => format!("{} {name}", singular(*kind)),
            RowKind::Truncated { .. } => row.label.clone(),
        }
    }

    fn status(&self) -> Option<String> {
        if self.context.session().is_none() {
            return Some("Not connected".into());
        }
        if self.loading.is_some() {
            return Some("Loading objects…".into());
        }
        if let Some(error) = &self.error {
            return Some(format!("Objects unavailable: {error}"));
        }
        if self.objects.is_some() && self.rows.is_empty() {
            return Some("No objects match".into());
        }
        None
    }
}

fn singular(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Table => "Table",
        ObjectKind::View => "View",
        ObjectKind::Index => "Index",
        ObjectKind::Trigger => "Trigger",
    }
}

fn row_icon(row: &TreeRow) -> (&'static str, gpui::Rgba) {
    match &row.kind {
        RowKind::Database { .. } => style::kind_icon(style::TreeKind::Database),
        RowKind::Group { kind, .. } => (kind.icon(), style::faint()),
        RowKind::Object { kind, .. } => (kind.icon(), style::dim()),
        RowKind::Truncated { .. } => ("icons/warning.svg", style::warn()),
    }
}

impl Render for SqliteTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status();
        let selected_label = self
            .rows
            .get(self.selected)
            .map(Self::row_label)
            .unwrap_or_default();
        let connected = self.context.session().is_some();
        div()
            .id("sqlite-tree")
            .role(Role::Group)
            .aria_label("Databases and objects")
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
                    .child(
                        div()
                            .id("sqlite-refresh-objects")
                            .role(Role::Button)
                            .aria_label("Refresh objects")
                            .tab_index(0)
                            .size(px(style::TOOL))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .focus(|s| s.bg(style::hover()))
                            .tooltip(crate::ui::tooltip("Refresh objects"))
                            .tooltip_show_delay(crate::ui::tooltip_delay())
                            .child(
                                gpui::svg()
                                    .path("icons/rotate_cw.svg")
                                    .size(px(style::ICON))
                                    .text_color(if connected { style::dim() } else { style::faint() }),
                            )
                            .when(connected, |button| {
                                crate::ui::press(
                                    button
                                        .cursor_pointer()
                                        .hover(|s| s.bg(style::hover()))
                                        .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                                )
                            })
                            .when(!connected, |button| {
                                button.a11y_synthetic_children(|builder| {
                                    builder.parent_node().set_disabled()
                                })
                            }),
                    ),
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
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(16.))
                            .child(self.filter_field.clone()),
                    ),
            )
            .when_some(status, |root, status| {
                root.child(
                    div()
                        .id("sqlite-tree-status")
                        .role(Role::Status)
                        .aria_label(status.clone())
                        .px(px(10.))
                        .pb(px(2.))
                        .text_size(px(style::FONT_SMALL))
                        .text_color(if self.error.is_some() {
                            style::bad_text()
                        } else {
                            style::faint()
                        })
                        .child(status),
                )
            })
            .child(
                div()
                    .id("sqlite-tree-rows")
                    .role(Role::Tree)
                    .aria_label(format!(
                        "{} object rows; arrows move, Right expands, Left collapses, Enter opens, Shift-Enter opens structure",
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
                            "sqlite-tree-list",
                            self.rows.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                                let focused = this.list.is_focused(window);
                                range
                                    .map(|position| {
                                        let row = &this.rows[position];
                                        let (path, color) = row_icon(row);
                                        let selected = position == this.selected;
                                        div()
                                            .id(("sqlite-tree-row", position))
                                            .role(Role::TreeItem)
                                            .aria_label(Self::row_label(row))
                                            .aria_selected(selected)
                                            .when_some(row.expanded, |item, open| item.aria_expanded(open))
                                            .h(px(style::ROW))
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .pl(px(8. + 12. * row.depth as f32))
                                            .pr(px(8.))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_color(match row.kind {
                                                RowKind::Truncated { .. } => style::warn(),
                                                _ => style::text(),
                                            })
                                            .hover(|s| s.bg(style::hover()))
                                            .when(selected, |item| {
                                                item.bg(if focused { style::select() } else { style::raised() })
                                            })
                                            .child(match row.expanded {
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
                                                    .when(matches!(row.kind, RowKind::Object { .. }), |label| {
                                                        label.font_family(style::MONO)
                                                    })
                                                    .child(SharedString::from(row.label.clone())),
                                            )
                                            .when_some(row.count, |item, count| {
                                                item.child(
                                                    div()
                                                        .text_size(px(style::FONT_SMALL))
                                                        .text_color(style::faint())
                                                        .child(count.to_string()),
                                                )
                                            })
                                            .on_click(cx.listener(
                                                move |this, event: &gpui::ClickEvent, window, cx| {
                                                    // Rows may have shrunk since this frame.
                                                    let Some(row) = this.rows.get(position) else {
                                                        return;
                                                    };
                                                    let expandable = row.expanded.is_some();
                                                    this.selected = position;
                                                    window.focus(&this.list, cx);
                                                    if event.click_count() > 1 || expandable {
                                                        this.activate(position, cx);
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
