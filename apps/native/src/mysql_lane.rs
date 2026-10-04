//! Plan 031 step 4: one MySQL connection's session, sidebar tree and
//! documents. The lane owns at most one backend session; documents borrow it.
//! A failed or retired session stays closed until the user connects again,
//! and replies from an earlier attempt are ignored by epoch.
mod document;
mod model;
mod tree;

use crate::{
    controller::Host,
    document_view::{ConnectionPhase, TabInfo},
};
use dbunk_lib::backend::mysql_sessions::{MySqlObjectKind, MySqlSession};
use document::{DocEvent, DocKind, MySqlDocument};
use gpui::{Context, Entity, EventEmitter, Subscription, Task, Window, div, prelude::*, px};
use model::{Lifecycle, Tree};
use std::sync::Arc;
use tree::{MySqlTreeView, TreeEvent};

/// Documents per connection, like the workspace's query tab limit.
const MAX_DOCUMENTS: usize = 16;

pub enum MySqlEvent {
    /// A statement completed in this many milliseconds.
    Latency(u64),
}

struct Doc {
    id: String,
    title: String,
    kind: DocKind,
    view: Entity<MySqlDocument>,
    _events: Subscription,
}

pub struct MySqlLane {
    host: Arc<Host>,
    connection_id: String,
    session: Option<MySqlSession>,
    life: Lifecycle,
    tree: Tree,
    tree_view: Entity<MySqlTreeView>,
    _tree_events: Subscription,
    documents: Vec<Doc>,
    active: Option<String>,
    message: Option<String>,
    connect_task: Option<Task<()>>,
    watch_task: Option<Task<()>>,
    tree_tasks: Vec<Task<()>>,
}

impl EventEmitter<MySqlEvent> for MySqlLane {}

impl MySqlLane {
    pub fn new(
        host: Arc<Host>,
        connection_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_view = cx.new(MySqlTreeView::new);
        let tree_events = cx.subscribe_in(
            &tree_view,
            window,
            |this, _, event: &TreeEvent, window, cx| this.tree_event(event.clone(), window, cx),
        );
        Self {
            host,
            connection_id,
            session: None,
            life: Lifecycle::default(),
            tree: Tree::default(),
            tree_view,
            _tree_events: tree_events,
            documents: Vec::new(),
            active: None,
            message: None,
            connect_task: None,
            watch_task: None,
            tree_tasks: Vec::new(),
        }
    }

    pub fn phase(&self) -> ConnectionPhase {
        self.life.phase().clone()
    }

    pub fn tree_view(&self) -> Entity<MySqlTreeView> {
        self.tree_view.clone()
    }

    pub fn tabs(&self) -> Vec<TabInfo> {
        self.documents
            .iter()
            .map(|doc| TabInfo {
                id: doc.id.clone(),
                title: doc.title.clone(),
                icon: doc.kind.icon(),
                status: String::new(),
                active: self.active.as_ref() == Some(&doc.id),
                pinned: false,
                closable: true,
            })
            .collect()
    }

    pub fn has_tab(&self, id: &str) -> bool {
        self.documents.iter().any(|doc| doc.id == id)
    }

    /// One explicit attempt; nothing happens while one is open or opening.
    pub fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(epoch) = self.life.begin() else {
            return;
        };
        self.message = None;
        let host = self.host.clone();
        let id = self.connection_id.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { host.backend.open_mysql_session(id).await });
        self.connect_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let result = result
                    .map_err(|_| "Connect task ended unexpectedly".to_string())
                    .and_then(|result| result.map_err(|error| error.to_string()));
                this.opened(epoch, result, window, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn opened(
        &mut self,
        epoch: u64,
        result: Result<MySqlSession, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (session, outcome) = match result {
            Ok(session) => (Some(session), Ok(())),
            Err(error) => (None, Err(error)),
        };
        let adopted = self.life.opened(epoch, outcome);
        cx.notify();
        let Some(session) = session else {
            return;
        };
        if !adopted {
            // Disconnected while opening: close the late session.
            self.host
                .runtime
                .spawn(async move { session.close().await });
            return;
        }
        self.session = Some(session.clone());
        let closed = self.host.runtime.spawn(session.closed_signal());
        self.watch_task = Some(cx.spawn(async move |this, cx| {
            let reason = closed.await.ok().flatten();
            this.update(cx, |this, cx| this.closed(epoch, reason, cx))
                .ok();
        }));
        for doc in &self.documents {
            doc.view
                .update(cx, |view, cx| view.set_session(Some(session.clone()), cx));
        }
        self.tree.reset();
        self.tree.databases = Some(model::Load::Loading);
        self.load_databases(cx);
        if self.documents.is_empty() {
            self.open(DocKind::Query, None, window, cx);
        }
    }

    /// The backend closed the session (failure, retirement or disconnect).
    fn closed(&mut self, epoch: u64, reason: Option<String>, cx: &mut Context<Self>) {
        if self.life.closed(epoch, reason) {
            self.detach(cx);
            cx.notify();
        }
    }

    /// Detaches the session and hands it back for a joined close.
    /// Documents keep their text and last results.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) -> Option<MySqlSession> {
        self.life.disconnect();
        // Dropping the task leaves the open call to the backend; its late
        // session is never delivered and closes when its handle drops.
        self.connect_task = None;
        let session = self.detach(cx);
        cx.notify();
        session
    }

    fn detach(&mut self, cx: &mut Context<Self>) -> Option<MySqlSession> {
        self.watch_task = None;
        self.tree_tasks.clear();
        self.tree.reset();
        self.sync_tree(cx);
        for doc in &self.documents {
            doc.view.update(cx, |view, cx| view.set_session(None, cx));
        }
        self.session.take()
    }

    fn sync_tree(&mut self, cx: &mut Context<Self>) {
        let rows = self.tree.rows();
        let connected = self.session.is_some();
        self.tree_view
            .update(cx, |view, cx| view.set_rows(rows, connected, cx));
    }

    fn load_databases(&mut self, cx: &mut Context<Self>) {
        self.sync_tree(cx);
        let Some(session) = self.session.clone() else {
            return;
        };
        let epoch = self.life.epoch();
        let default = session.server().database.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { session.databases().await });
        self.tree_tasks.push(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if !this.life.is_current(epoch) {
                    return;
                }
                let result = flatten(result);
                if let Some(database) = this.tree.set_databases(result, default.as_deref()) {
                    this.load_objects(database, cx);
                }
                this.sync_tree(cx);
                let names = this.tree.database_names().to_vec();
                for doc in &this.documents {
                    doc.view.update(cx, |view, cx| {
                        view.set_databases(names.clone(), default.clone(), cx)
                    });
                }
            })
            .ok();
        }));
    }

    fn load_objects(&mut self, database: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let epoch = self.life.epoch();
        let name = database.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { session.objects(name).await });
        self.tree_tasks.push(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if !this.life.is_current(epoch) {
                    return;
                }
                this.tree.set_objects(&database, flatten(result));
                this.sync_tree(cx);
            })
            .ok();
        }));
    }

    fn tree_event(&mut self, event: TreeEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Finished loads are dropped so the list stays bounded.
        self.tree_tasks.retain(|task| !task.is_ready());
        match event {
            TreeEvent::ToggleDatabase(database) => {
                if let Some(database) = self.tree.toggle_database(&database) {
                    self.load_objects(database, cx);
                }
                self.sync_tree(cx);
            }
            TreeEvent::ToggleGroup(database, group) => {
                self.tree.toggle_group(&database, group);
                self.sync_tree(cx);
            }
            TreeEvent::Refresh => {
                if self.session.is_some() {
                    let open = self.tree.refresh();
                    self.load_databases(cx);
                    for database in open {
                        self.load_objects(database, cx);
                    }
                }
            }
            TreeEvent::Open(object) => {
                let kind = if object.has_rows() {
                    DocKind::Data(object)
                } else {
                    DocKind::Definition(object)
                };
                self.open(kind, None, window, cx);
            }
            TreeEvent::Structure(object) => self.open(DocKind::Structure(object), None, window, cx),
            TreeEvent::NewQuery(database) => self.open(DocKind::Query, database, window, cx),
        }
    }

    /// Opens a document, reusing an open one for the same object and view.
    fn open(
        &mut self,
        kind: DocKind,
        database: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if kind != DocKind::Query
            && let Some(doc) = self.documents.iter().find(|doc| doc.kind == kind)
        {
            self.active = Some(doc.id.clone());
            cx.notify();
            return;
        }
        if self.documents.len() >= MAX_DOCUMENTS {
            self.message = Some(format!(
                "Close a tab before opening another (limit {MAX_DOCUMENTS})"
            ));
            cx.notify();
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let title = match &kind {
            DocKind::Query => {
                let number = (1..)
                    .find(|number| {
                        !self
                            .documents
                            .iter()
                            .any(|doc| doc.title == format!("Query {number}"))
                    })
                    .unwrap_or(1);
                format!("Query {number}")
            }
            DocKind::Data(object) => object.name.clone(),
            DocKind::Structure(object) => format!("{} · structure", object.name),
            DocKind::Definition(object) => object.name.clone(),
        };
        let database = database
            .or_else(|| self.session.as_ref()?.server().database.clone())
            .or_else(|| self.tree.database_names().first().cloned());
        let host = self.host.clone();
        let session = self.session.clone();
        let databases = self.tree.database_names().to_vec();
        let doc_kind = kind.clone();
        let view = cx
            .new(|cx| MySqlDocument::new(host, doc_kind, session, database, databases, window, cx));
        let events =
            cx.subscribe_in(
                &view,
                window,
                |this, _, event: &DocEvent, window, cx| match event {
                    DocEvent::Latency(ms) => cx.emit(MySqlEvent::Latency(*ms)),
                    DocEvent::Structure(object) => {
                        this.open(DocKind::Structure(object.clone()), None, window, cx)
                    }
                    DocEvent::Data(object) => {
                        this.open(DocKind::Data(object.clone()), None, window, cx)
                    }
                },
            );
        self.documents.push(Doc {
            id: id.clone(),
            title,
            kind,
            view,
            _events: events,
        });
        self.active = Some(id.clone());
        self.message = None;
        self.select_tab(&id, window, cx);
    }

    pub fn new_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(DocKind::Query, None, window, cx);
    }

    pub fn select_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(doc) = self.documents.iter().find(|doc| doc.id == id) {
            self.active = Some(doc.id.clone());
            doc.view.update(cx, |view, cx| view.focus(window, cx));
            cx.notify();
        }
    }

    /// Steps through tabs, wrapping at the ends.
    pub fn cycle(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.documents.len();
        if count == 0 {
            return;
        }
        let current = self
            .documents
            .iter()
            .position(|doc| Some(&doc.id) == self.active.as_ref())
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        let id = self.documents[next].id.clone();
        self.select_tab(&id, window, cx);
    }

    /// Closes a tab; a running request in it is abandoned to the worker,
    /// which finishes it and keeps the session usable.
    pub fn close_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.documents.iter().position(|doc| doc.id == id) else {
            return;
        };
        self.documents.remove(index);
        if self.active.as_deref() == Some(id) {
            self.active = self
                .documents
                .get(index.min(self.documents.len().saturating_sub(1)))
                .map(|doc| doc.id.clone());
            self.focus_active(window, cx);
        }
        cx.notify();
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active.clone() {
            self.close_tab(&id, window, cx);
        }
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut gpui::App) {
        if let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(&doc.id) == self.active.as_ref())
        {
            doc.view.update(cx, |view, cx| view.focus(window, cx));
        }
    }
}

impl Drop for MySqlLane {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            self.host
                .runtime
                .spawn(async move { session.close().await });
        }
    }
}

fn flatten<T>(
    result: Result<
        Result<T, dbunk_lib::backend::mysql_sessions::MySqlSessionError>,
        tokio::task::JoinError,
    >,
) -> Result<T, String> {
    match result {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("Request task ended unexpectedly".into()),
    }
}

impl Render for MySqlLane {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let active = self
            .documents
            .iter()
            .find(|doc| Some(&doc.id) == self.active.as_ref());
        let body = match (active, self.life.phase()) {
            (Some(doc), _) => crate::ui::appear(
                gpui::SharedString::from(format!("mysql-document-{}", doc.id)),
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(doc.view.clone()),
            )
            .into_any_element(),
            (None, phase) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(crate::style::faint())
                .child(match phase {
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
            .bg(crate::style::bg())
            .child(body)
            .when_some(self.message.clone(), |root, message| {
                root.child(
                    crate::ui::error_banner("mysql-lane-message", message)
                        .flex_none()
                        .m(px(8.)),
                )
            })
    }
}

/// Labels for tab and object kinds.
pub(crate) fn kind_label(kind: MySqlObjectKind) -> &'static str {
    match kind {
        MySqlObjectKind::Table => "table",
        MySqlObjectKind::View => "view",
        MySqlObjectKind::Procedure => "procedure",
        MySqlObjectKind::Function => "function",
        MySqlObjectKind::Event => "event",
        MySqlObjectKind::Trigger => "trigger",
    }
}
