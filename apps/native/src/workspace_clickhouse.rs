//! Workspace side of native ClickHouse (Plan 031 step 4): selecting a
//! ClickHouse connection opens its session, the sidebar shows its object tree,
//! and its documents live in the ordinary tab list without being persisted.
use super::*;
use crate::clickhouse::{
    document::{ClickHouseDocument, DocumentEvent as ClickHouseEvent, Mode},
    is_clickhouse,
    session_model::survives_change,
    sessions::{ClickHouseSessions, SessionsChanged},
    tree_view::{ClickHouseTree, TreeEvent},
};

pub(super) struct ClickHouseState {
    pub sessions: Entity<ClickHouseSessions>,
    pub tree: Entity<ClickHouseTree>,
    _tree_events: Subscription,
    _session_events: Subscription,
}

impl ClickHouseState {
    pub fn new(host: Arc<Host>, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let sessions = cx.new(|_| ClickHouseSessions::new(host.clone()));
        let tree = cx.new(|cx| ClickHouseTree::new(host, sessions.clone(), window, cx));
        let tree_events = cx.subscribe_in(
            &tree,
            window,
            |this: &mut Workspace, _, event: &TreeEvent, window, cx| match event {
                TreeEvent::Open {
                    connection,
                    database,
                    name,
                    kind,
                    structure,
                } => {
                    let (database, name, kind) = (database.clone(), name.clone(), *kind);
                    let mode = if *structure {
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
                    };
                    this.open_clickhouse(connection.clone(), mode, window, cx);
                }
                TreeEvent::NewQuery { connection } => {
                    this.open_clickhouse(connection.clone(), Mode::Query, window, cx)
                }
            },
        );
        let session_events = cx.subscribe(
            &sessions,
            |_: &mut Workspace, _, _: &SessionsChanged, cx| cx.notify(),
        );
        Self {
            sessions,
            tree,
            _tree_events: tree_events,
            _session_events: session_events,
        }
    }
}

impl Workspace {
    pub(super) fn is_clickhouse(&self, id: &str) -> bool {
        self.connections
            .iter()
            .any(|connection| connection.id == id && is_clickhouse(connection))
    }

    pub(super) fn clickhouse_selected(&self) -> Option<String> {
        self.selected_connection
            .clone()
            .filter(|id| self.is_clickhouse(id))
    }

    /// Selecting a ClickHouse connection opens its session (once; an open or
    /// opening session is left alone, a failed one is retried explicitly).
    pub(super) fn select_clickhouse(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_connection = Some(id.clone());
        self.clickhouse
            .sessions
            .update(cx, |sessions, cx| sessions.connect(&id, cx));
        cx.notify();
    }

    pub(super) fn disconnect_clickhouse(&mut self, id: String, cx: &mut Context<Self>) {
        self.clickhouse
            .sessions
            .update(cx, |sessions, cx| sessions.disconnect(&id, cx));
    }

    pub(super) fn clickhouse_phase(&self, id: &str, cx: &gpui::App) -> Option<ConnectionPhase> {
        self.is_clickhouse(id)
            .then(|| self.clickhouse.sessions.read(cx).phase(id))
    }

    /// After connections reload: sessions of deleted or no-longer-ClickHouse
    /// connections end.
    pub(super) fn sync_clickhouse(&mut self, cx: &mut Context<Self>) {
        let keep = self
            .connections
            .iter()
            .filter(|connection| is_clickhouse(connection))
            .map(|connection| connection.id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.clickhouse.sessions.update(cx, |sessions, cx| {
            sessions.retain(|id| keep.contains(id), cx)
        });
    }

    /// An edited or deleted connection (`scope`) or a credential change
    /// (`all`) invalidates the endpoint and secret a session resolved. A new
    /// connection (no scope) leaves every session alone.
    pub(super) fn invalidate_clickhouse(
        &mut self,
        scope: Option<&str>,
        all: bool,
        cx: &mut Context<Self>,
    ) {
        self.clickhouse.sessions.update(cx, |sessions, cx| {
            sessions.retain(|id| survives_change(scope, all, id), cx)
        });
    }

    pub(super) fn close_clickhouse(&mut self, cx: &mut Context<Self>) {
        self.clickhouse
            .sessions
            .update(cx, |sessions, cx| sessions.close_all(cx));
    }

    /// ClickHouse tabs are not persisted. Returns their ids and the active
    /// id to save: a ClickHouse active tab saves as the first persisted tab.
    pub(super) fn transient_documents(
        &self,
        cx: &gpui::App,
    ) -> (std::collections::HashSet<String>, Option<String>) {
        let transient = self
            .documents
            .iter()
            .filter(|document| document.view.read(cx).is_transient())
            .map(|document| document.metadata.id.clone())
            .collect::<std::collections::HashSet<_>>();
        let active = self
            .active
            .clone()
            .filter(|id| !transient.contains(id))
            .or_else(|| {
                self.documents
                    .iter()
                    .map(|document| &document.metadata.id)
                    .find(|id| !transient.contains(*id))
                    .cloned()
            });
        (transient, active)
    }

    /// The sidebar's object tree: ClickHouse for a ClickHouse selection,
    /// otherwise the PostgreSQL navigator.
    pub(super) fn object_tree(&self) -> gpui::AnyElement {
        if self.clickhouse_selected().is_some() {
            self.clickhouse.tree.clone().into_any_element()
        } else {
            self.navigator.clone().into_any_element()
        }
    }

    pub(super) fn sync_clickhouse_tree(&mut self, cx: &mut Context<Self>) {
        let selected = self.clickhouse_selected();
        self.clickhouse
            .tree
            .update(cx, |tree, cx| tree.set_connection(selected, cx));
    }

    /// `New` with a ClickHouse connection selected opens a ClickHouse query.
    pub(super) fn new_clickhouse_query(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(connection) = self.clickhouse_selected() else {
            return false;
        };
        self.open_clickhouse(connection, Mode::Query, window, cx);
        true
    }

    pub(super) fn clickhouse_tab_icon(
        &self,
        document: &Document,
        cx: &gpui::App,
    ) -> Option<&'static str> {
        let view = document.view.read(cx).clickhouse()?;
        Some(match view.read(cx).mode() {
            Mode::Query => "icons/terminal.svg",
            Mode::Data { .. } => "icons/table.svg",
            Mode::Structure { .. } => "icons/list_tree.svg",
        })
    }

    /// Focuses the tab already showing `mode` on `connection` (queries always
    /// open a new tab), else opens one. Bounded by the workspace tab limit.
    fn open_clickhouse(
        &mut self,
        connection: String,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode != Mode::Query
            && let Some(existing) = self.documents.iter().find(|document| {
                document.view.read(cx).clickhouse().is_some_and(|view| {
                    let view = view.read(cx);
                    view.connection() == connection && view.mode() == &mode
                })
            })
        {
            let id = existing.metadata.id.clone();
            self.remember_focus(window, cx);
            self.active = Some(id);
            self.changed(cx);
            self.focus_active(window, cx);
            return;
        }
        if !self.restored || self.documents.len() >= 16 {
            self.message = Some("Close a tab before opening another (limit 16)".into());
            cx.notify();
            return;
        }
        self.selected_connection = Some(connection.clone());
        let view = cx.new(|cx| {
            ClickHouseDocument::new(
                self.host.clone(),
                self.clickhouse.sessions.clone(),
                connection.clone(),
                mode,
                self.retained.clone(),
                window,
                cx,
            )
        });
        let id = uuid::Uuid::new_v4().to_string();
        let name = match view.read(cx).mode() {
            Mode::Query => {
                let number = (1..)
                    .find(|number| {
                        !self
                            .documents
                            .iter()
                            .any(|document| document.metadata.name == format!("CH {number}"))
                    })
                    .unwrap();
                format!("CH {number}")
            }
            _ => view.read(cx).title(),
        };
        let events = cx.subscribe_in(&view, window, {
            let connection = connection.clone();
            move |this: &mut Workspace, view, event: &ClickHouseEvent, window, cx| match event {
                ClickHouseEvent::Open { structure } => {
                    if let Some((database, name, kind)) = view.read(cx).mode().object() {
                        let (database, name) = (database.to_owned(), name.to_owned());
                        let mode = if *structure {
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
                        };
                        this.open_clickhouse(connection.clone(), mode, window, cx);
                    }
                }
                ClickHouseEvent::Latency(ms) => {
                    this.shell.last_latency.insert(connection.clone(), *ms);
                    cx.notify();
                }
            }
        });
        let document_view = cx.new(|cx| DocumentView::from_clickhouse(view, cx));
        let status = cx.observe(&document_view, |_, _, cx| cx.notify());
        self.documents.push(Document {
            metadata: WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                tool: None,
                saved_query_id: None,
                table: None,
                id: id.clone(),
                name,
                connection_id: Some(connection),
                sql: String::new(),
                pinned: false,
                selection: WorkspaceSelection::default(),
            },
            view: document_view,
            _events: events,
            _status: status,
        });
        self.remember_focus(window, cx);
        self.active = Some(id);
        self.changed(cx);
        self.focus_active(window, cx);
    }
}
