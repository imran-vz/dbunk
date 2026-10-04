//! Plan 031 step 4: SQLite connections in the workspace shell.
//!
//! Each selected SQLite connection gets one `SqliteWorkspace` (session, tree,
//! tabs). While a SQLite connection is selected the shell shows its tabs,
//! tree and active document instead of the PostgreSQL documents, which stay
//! open underneath. Tab operations route here first; everything else keeps
//! its PostgreSQL behaviour.
use super::*;
use crate::sqlite_workspace::{CLOSE_DEADLINE, SqliteEvent, SqliteWorkspace, TabInfo};
use dbunk_lib::backend::sqlite_session::SqliteSession;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct SqliteHost {
    workspaces: HashMap<String, Opened>,
}

struct Opened {
    workspace: Entity<SqliteWorkspace>,
    /// The record the session was opened for; any change closes it.
    record: String,
    _events: Subscription,
}

fn record_of(connection: &DevelopmentConnection) -> String {
    connection
        .settings
        .as_ref()
        .and_then(|settings| serde_json::to_string(settings).ok())
        .unwrap_or_default()
}

impl Workspace {
    pub(super) fn is_sqlite(&self, id: &str) -> bool {
        self.connections
            .iter()
            .any(|connection| connection.id == id && crate::sqlite_workspace::is_sqlite(connection))
    }

    /// The SQLite workspace the shell shows, when a SQLite connection is
    /// selected.
    pub(super) fn sqlite_active(&self) -> Option<&Entity<SqliteWorkspace>> {
        let id = self.selected_connection.as_ref()?;
        self.sqlite
            .workspaces
            .get(id)
            .filter(|_| self.is_sqlite(id))
            .map(|opened| &opened.workspace)
    }

    pub(super) fn sqlite_phase(&self, id: &str, cx: &gpui::App) -> Option<ConnectionPhase> {
        self.sqlite
            .workspaces
            .get(id)
            .map(|opened| opened.workspace.read(cx).phase())
    }

    pub(super) fn sqlite_tabs(&self, cx: &gpui::App) -> Option<Vec<TabInfo>> {
        self.sqlite_active()
            .map(|workspace| workspace.read(cx).tabs(cx))
    }

    fn sqlite_workspace(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SqliteWorkspace> {
        if let Some(opened) = self.sqlite.workspaces.get(id) {
            return opened.workspace.clone();
        }
        let host = self.host.clone();
        let retained = self.retained.clone();
        let workspace =
            cx.new(|cx| SqliteWorkspace::new(id.to_owned(), host, retained, window, cx));
        let connection = id.to_owned();
        let events = cx.subscribe(&workspace, move |this, _, event, cx| match event {
            SqliteEvent::Changed => cx.notify(),
            SqliteEvent::Latency(ms) => {
                this.shell.last_latency.insert(connection.clone(), *ms);
                cx.notify();
            }
        });
        let record = self
            .connections
            .iter()
            .find(|connection| connection.id == id)
            .map(record_of)
            .unwrap_or_default();
        self.sqlite.workspaces.insert(
            id.to_owned(),
            Opened {
                workspace: workspace.clone(),
                record,
                _events: events,
            },
        );
        workspace
    }

    /// Selects a SQLite connection and opens its session if needed.
    fn sqlite_select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_connection = Some(id.clone());
        let workspace = self.sqlite_workspace(&id, window, cx);
        workspace.update(cx, |workspace, cx| workspace.connect(window, cx));
        cx.notify();
    }

    fn sqlite_disconnect(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(opened) = self.sqlite.workspaces.get(id) else {
            return;
        };
        let task = opened
            .workspace
            .update(cx, |workspace, cx| workspace.disconnect(cx));
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = task.await {
                this.update(cx, |this, cx| {
                    this.message = Some(error);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Routes an operation to SQLite when it targets a SQLite connection or
    /// the SQLite workspace on screen. Returns true when handled.
    pub(super) fn sqlite_operation(
        &mut self,
        operation: &Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match operation {
            Operation::SelectConnection(id) if self.is_sqlite(id) => {
                self.sqlite_select(id.clone(), window, cx);
                return true;
            }
            Operation::DisconnectConnection(id) if self.sqlite.workspaces.contains_key(id) => {
                self.sqlite_disconnect(&id.clone(), window, cx);
                return true;
            }
            Operation::SelectDocument(id) | Operation::CloseDocument(id) => {
                let owner = self
                    .sqlite
                    .workspaces
                    .values()
                    .find(|opened| opened.workspace.read(cx).owns_tab(id))
                    .map(|opened| opened.workspace.clone());
                if let Some(workspace) = owner {
                    let close = matches!(operation, Operation::CloseDocument(_));
                    workspace.update(cx, |workspace, cx| {
                        if close {
                            workspace.close_tab(id, window, cx);
                        } else {
                            workspace.select_tab(id, window, cx);
                        }
                    });
                    return true;
                }
                // A workspace document chosen elsewhere (palette, tab
                // shortcuts) replaces the SQLite view with its connection.
                if matches!(operation, Operation::SelectDocument(_))
                    && self.sqlite_active().is_some()
                {
                    self.selected_connection = self
                        .documents
                        .iter()
                        .find(|document| &document.metadata.id == id)
                        .and_then(|document| document.metadata.connection_id.clone());
                }
                return false;
            }
            _ => {}
        }
        let Some(workspace) = self.sqlite_active().cloned() else {
            return false;
        };
        let id = self.selected_connection.clone().unwrap_or_default();
        match operation {
            Operation::New => workspace.update(cx, |workspace, cx| workspace.new_query(window, cx)),
            Operation::Close => {
                workspace.update(cx, |workspace, cx| workspace.close_active(window, cx));
            }
            Operation::Next | Operation::Previous => {
                let backwards = matches!(operation, Operation::Previous);
                workspace.update(cx, |workspace, cx| {
                    workspace.select_next(backwards, window, cx)
                });
            }
            Operation::Connect => {
                workspace.update(cx, |workspace, cx| workspace.connect(window, cx))
            }
            Operation::Disconnect => self.sqlite_disconnect(&id, window, cx),
            Operation::OpenTable => {
                self.message = Some("Open SQLite tables from the sidebar tree".into());
            }
            _ => return false,
        }
        true
    }

    /// After connections reload: a removed connection drops its workspace; a
    /// changed record (path, read-only, policy) closes its session so the
    /// next connect uses the saved settings. Nothing reconnects by itself.
    pub(super) fn sqlite_reconcile(&mut self, cx: &mut Context<Self>) {
        let mut close = Vec::new();
        let mut removed = Vec::new();
        for (id, opened) in &mut self.sqlite.workspaces {
            match self
                .connections
                .iter()
                .find(|connection| &connection.id == id)
            {
                Some(connection) if crate::sqlite_workspace::is_sqlite(connection) => {
                    let record = record_of(connection);
                    if record != opened.record {
                        opened.record = record;
                        close.push(opened.workspace.clone());
                    }
                }
                _ => {
                    removed.push(id.clone());
                    close.push(opened.workspace.clone());
                }
            }
        }
        let sessions: Vec<Arc<SqliteSession>> = close
            .iter()
            .filter_map(|workspace| {
                workspace.update(cx, |workspace, cx| workspace.take_session(cx))
            })
            .collect();
        for id in removed {
            if let Some(opened) = self.sqlite.workspaces.remove(&id) {
                opened
                    .workspace
                    .update(cx, |workspace, cx| workspace.release_all(cx));
            }
        }
        if !sessions.is_empty() {
            self.host.runtime.spawn(async move {
                for session in sessions {
                    let _ = session.close(CLOSE_DEADLINE).await;
                }
            });
        }
    }

    /// Detaches every SQLite session for a joined close at quit.
    pub(super) fn sqlite_take_sessions(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<Arc<SqliteSession>> {
        self.sqlite
            .workspaces
            .values()
            .filter_map(|opened| {
                opened
                    .workspace
                    .update(cx, |workspace, cx| workspace.take_session(cx))
            })
            .collect()
    }
}

/// Closes every session, each within `CLOSE_DEADLINE`, and reports the first
/// failure.
pub(super) async fn close_sqlite_sessions(sessions: Vec<Arc<SqliteSession>>) -> Result<(), String> {
    let mut first = Ok(());
    for session in sessions {
        if let Err(error) = session.close(CLOSE_DEADLINE).await {
            first = first.and(Err(error));
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::{
        DevelopmentEngineConnection, DevelopmentEnvironment, DevelopmentSafeMode,
        DevelopmentSqliteConnection,
    };

    fn sqlite(path: &str, read_only: bool) -> DevelopmentConnection {
        DevelopmentConnection {
            id: "local".into(),
            name: "local".into(),
            engine: "SQLite".into(),
            organization: Default::default(),
            unsupported_reason: None,
            postgres: None,
            settings: Some(DevelopmentEngineConnection::SQLite(
                DevelopmentSqliteConnection {
                    name: "local".into(),
                    path: path.into(),
                    environment: DevelopmentEnvironment::Development,
                    safe_mode: DevelopmentSafeMode::Inherit,
                    read_only,
                },
            )),
            environment: DevelopmentEnvironment::Development,
        }
    }

    #[test]
    fn only_supported_sqlite_records_open_a_sqlite_workspace() {
        assert!(crate::sqlite_workspace::is_sqlite(&sqlite(
            "/tmp/a.db",
            false
        )));
        let mut unsupported = sqlite("/tmp/a.db", false);
        unsupported.unsupported_reason = Some("outside fixture".into());
        assert!(!crate::sqlite_workspace::is_sqlite(&unsupported));
        let mut unreadable = sqlite("/tmp/a.db", false);
        unreadable.settings = None;
        assert!(!crate::sqlite_workspace::is_sqlite(&unreadable));
    }

    #[test]
    fn a_changed_path_or_policy_changes_the_session_record() {
        let base = record_of(&sqlite("/tmp/a.db", false));
        assert_eq!(base, record_of(&sqlite("/tmp/a.db", false)));
        assert_ne!(base, record_of(&sqlite("/tmp/b.db", false)));
        assert_ne!(base, record_of(&sqlite("/tmp/a.db", true)));
        let mut renamed = sqlite("/tmp/a.db", false);
        renamed.organization.folder = "elsewhere".into();
        assert_eq!(
            base,
            record_of(&renamed),
            "organization is not a session input"
        );
    }
}
