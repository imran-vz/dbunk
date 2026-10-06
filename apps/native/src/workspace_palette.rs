//! Open Anything for the workspace. The index is a snapshot of commands, open
//! tabs, saved connections and the Navigator's retained catalog; choosing an
//! item routes through the same guarded operations as buttons and menus.
use super::*;
use crate::{
    catalog::Kind,
    open_anything::{Item, ItemKind, Target},
    palette_view::{PaletteEvent, PaletteView},
};
use dbunk_lib::backend::{DevelopmentEnvironment, objects::PgObjectKind};

const COMMANDS: &[(&str, &str)] = &[
    ("new-query", "New query"),
    ("close-tab", "Close tab"),
    ("next-tab", "Next tab"),
    ("previous-tab", "Previous tab"),
    ("rename-tab", "Rename tab"),
    ("pin-tab", "Pin or unpin tab"),
    ("new-connection", "New connection"),
    ("open-table", "Open table by name"),
    ("history", "Query history"),
    ("saved", "Saved queries"),
    ("objects", "Objects"),
    ("administration", "Administration"),
    ("backup", "Backup and restore"),
    ("csv", "CSV transfer"),
    ("copy-table", "Copy table"),
    ("compare", "Compare schemas"),
    ("schema-map", "Schema map"),
    ("save-query", "Save query"),
    ("credentials", "Credentials"),
    ("density", "Toggle density"),
    ("disconnect", "Disconnect active tab"),
    ("clear-results", "Clear results"),
    ("seed-table", "Seed table"),
    ("toggle-sidebar", "Toggle sidebar"),
    ("toggle-status-bar", "Toggle status bar"),
    ("toggle-console", "Toggle console"),
    ("env-all", "Show all environments"),
    ("env-development", "Show Development connections"),
    ("env-test", "Show Test connections"),
    ("env-staging", "Show Staging connections"),
    ("env-production", "Show Production connections"),
    ("bastions", "Bastion servers"),
    ("managed", "Managed servers"),
];
/// Commands only the general PostgreSQL profile offers.
const GENERAL_ONLY: [&str; 2] = ["bastions", "managed"];

/// Palette entries whose destination `Target` cannot carry.
#[derive(Clone)]
enum Route {
    Project(String),
    EngineTab { connection: String, tab: String },
}

/// Search text for a saved connection: its endpoint (host, port, database
/// or file path, user) for every engine, plus engine and organization.
fn connection_keywords(connection: &DevelopmentConnection) -> String {
    let endpoint = connection
        .endpoint()
        .map(|endpoint| {
            format!(
                "{} {} {} {}",
                endpoint.host,
                endpoint
                    .port
                    .map(|port| port.to_string())
                    .unwrap_or_default(),
                endpoint.database,
                endpoint.user
            )
        })
        .or_else(|| {
            connection
                .postgres
                .as_ref()
                .map(|pg| format!("{} {} {} {}", pg.host, pg.port, pg.database, pg.user))
        })
        .unwrap_or_default();
    format!(
        "{endpoint} {} {} {}",
        connection.engine, connection.organization.project, connection.organization.folder
    )
}
fn command(id: &str) -> Option<Operation> {
    Some(match id {
        "new-query" => Operation::New,
        "close-tab" => Operation::Close,
        "next-tab" => Operation::Next,
        "previous-tab" => Operation::Previous,
        "rename-tab" => Operation::Rename,
        "pin-tab" => Operation::Pin,
        "new-connection" => Operation::NewConnection,
        "open-table" => Operation::OpenTable,
        "history" => Operation::Library(WorkspaceTool::History),
        "saved" => Operation::Library(WorkspaceTool::SavedQueries),
        "objects" => Operation::Library(WorkspaceTool::Objects),
        "administration" => Operation::Library(WorkspaceTool::Administration),
        "backup" => Operation::Library(WorkspaceTool::BackupRestore),
        "csv" => Operation::Library(WorkspaceTool::CsvTransfer),
        "copy-table" => Operation::Library(WorkspaceTool::TableCopy),
        "compare" => Operation::Library(WorkspaceTool::SchemaCompare),
        "schema-map" => Operation::Library(WorkspaceTool::SchemaMap),
        "save-query" => Operation::SaveQuery,
        "credentials" => Operation::Credentials,
        "density" => Operation::Density,
        "disconnect" => Operation::Disconnect,
        "clear-results" => Operation::Clear,
        "seed-table" => Operation::Library(WorkspaceTool::TableSeed),
        "toggle-sidebar" => Operation::ToggleSidebar,
        "toggle-status-bar" => Operation::ToggleStatusBar,
        "toggle-console" => Operation::Console,
        "env-all" => Operation::EnvFilter(None),
        "env-development" => Operation::EnvFilter(Some(DevelopmentEnvironment::Development)),
        "env-test" => Operation::EnvFilter(Some(DevelopmentEnvironment::Test)),
        "env-staging" => Operation::EnvFilter(Some(DevelopmentEnvironment::Staging)),
        "env-production" => Operation::EnvFilter(Some(DevelopmentEnvironment::Production)),
        "bastions" => Operation::Bastions,
        "managed" => Operation::ManagedServers,
        _ => return None,
    })
}

impl Workspace {
    /// Every entry, each with the route `Target` cannot express (projects,
    /// engine tabs); `None` routes through [`Self::open_target`].
    fn palette_items(&self, cx: &Context<Self>) -> (Vec<Item<&'static str>>, Vec<Option<Route>>) {
        let general = self.general_profile();
        let mut items: Vec<_> = COMMANDS
            .iter()
            .filter(|(id, _)| general || !GENERAL_ONLY.contains(id))
            .map(|(id, label)| {
                Item::new(
                    format!("command:{id}"),
                    ItemKind::Command,
                    (*label).into(),
                    String::new(),
                    "",
                    Target::Command(*id),
                )
            })
            .collect();
        let mut routes: Vec<Option<Route>> = vec![None; items.len()];
        for project in super::shell::projects(&self.connections) {
            if self.shell.project.as_ref() == Some(&project) {
                continue;
            }
            items.push(Item::new(
                format!("project:{project}"),
                ItemKind::Command,
                format!("Switch to project {project}"),
                String::new(),
                "project",
                Target::Command("project"),
            ));
            routes.push(Some(Route::Project(project)));
        }
        let name = |id: Option<&String>| {
            id.and_then(|id| self.connections.iter().find(|c| &c.id == id))
                .map(|c| c.name.clone())
                .unwrap_or_default()
        };
        for document in &self.documents {
            let connection = name(document.metadata.connection_id.as_ref());
            items.push(Item::new(
                format!("tab:{}", document.metadata.id),
                ItemKind::Tab,
                document.metadata.name.clone(),
                if connection.is_empty() {
                    "Open tab".into()
                } else {
                    format!("Open tab · {connection}")
                },
                document
                    .metadata
                    .table
                    .as_ref()
                    .map(|table| format!("{}.{}", table.schema, table.table))
                    .unwrap_or_default()
                    .as_str(),
                Target::Tab(document.metadata.id.clone()),
            ));
            routes.push(None);
        }
        for (connection, tab, item) in self.engine_palette_items(cx) {
            items.push(item);
            routes.push(Some(Route::EngineTab { connection, tab }));
        }
        for connection in &self.connections {
            items.push(Item::new(
                format!("connection:{}", connection.id),
                ItemKind::Connection,
                connection.name.clone(),
                match &connection.unsupported_reason {
                    Some(_) => "Connection · unsupported here".into(),
                    None => match connection.endpoint() {
                        Some(endpoint) => {
                            format!("{} · {}", connection.engine, endpoint.label())
                        }
                        None => "Connection · select".into(),
                    },
                },
                &connection_keywords(connection),
                Target::Connection(connection.id.clone()),
            ));
            routes.push(None);
        }
        if let Some((connection, catalog)) = self.navigator.read(cx).catalog() {
            let connection_name = name(Some(&connection.to_owned()));
            for row in &catalog.rows {
                let Kind::Object(kind) = row.kind else {
                    continue;
                };
                if kind == PgObjectKind::Schema {
                    items.push(Item::new(
                        format!("schema:{connection}:{}", row.entry.name),
                        ItemKind::Schema,
                        row.entry.name.clone(),
                        format!("Schema · {connection_name}"),
                        "schema",
                        Target::Schema {
                            connection: connection.to_owned(),
                            schema: row.entry.name.clone(),
                        },
                    ));
                    continue;
                }
                let (Some(schema), Some(reference)) = (&row.schema, row.reference()) else {
                    continue;
                };
                let label = match &row.entry.identity_args {
                    Some(args) => format!("{}({args})", row.entry.name),
                    None => row.entry.name.clone(),
                };
                let description = format!("{} · {schema} · {connection_name}", row.kind.label());
                let keywords = format!("{schema}.{} {}", row.entry.name, row.kind.label());
                items.push(if row.kind.relation() {
                    Item::new(
                        format!("relation:{connection}:{schema}:{}", row.entry.name),
                        ItemKind::Relation,
                        label,
                        description,
                        &keywords,
                        Target::Relation {
                            connection: connection.to_owned(),
                            schema: schema.clone(),
                            name: row.entry.name.clone(),
                        },
                    )
                } else {
                    Item::new(
                        format!(
                            "object:{connection}:{}",
                            serde_json::to_string(&reference).unwrap_or_default()
                        ),
                        ItemKind::Object,
                        label,
                        description,
                        &keywords,
                        Target::Object {
                            connection: connection.to_owned(),
                            reference,
                        },
                    )
                });
            }
        }
        // Catalog entries come last and route through `open_target`.
        routes.resize(items.len(), None);
        (items, routes)
    }

    pub(super) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing
            || self.busy
            || self.loading
            || self.dialog.is_some()
            || self.managed.is_some()
        {
            return;
        }
        if self.palette.is_some() {
            return;
        }
        self.previous_focus = window.focused(cx);
        let (items, routes) = self.palette_items(cx);
        let note = if self.navigator.read(cx).catalog().is_none() {
            "Schemas and objects appear after Load objects in the Navigator. Saved queries and history are in their Tool tabs."
        } else {
            "Objects come from the Navigator's retained capture for the selected connection. Saved queries and history are in their Tool tabs."
        };
        let frecency = &self.frecency;
        let palette =
            cx.new(|cx| PaletteView::new(items.clone(), frecency, Some(note.into()), window, cx));
        self._palette_events = Some(cx.subscribe_in(
            &palette,
            window,
            move |this, _, event: &PaletteEvent, window, cx| {
                this.palette = None;
                this._palette_events = None;
                match event {
                    PaletteEvent::Dismissed => {
                        if let Some(focus) = this.previous_focus.clone() {
                            window.focus(&focus, cx);
                        } else {
                            this.focus_active(window, cx);
                        }
                    }
                    PaletteEvent::Chosen(index) => {
                        // Leave the destroyed palette editor before routing, so
                        // operations and forms record a live focus target.
                        if let Some(focus) = this.previous_focus.clone() {
                            window.focus(&focus, cx);
                        } else {
                            this.focus_active(window, cx);
                        }
                        if let Some(item) = items.get(*index) {
                            this.frecency.record(&item.key);
                            match routes.get(*index).cloned().flatten() {
                                Some(Route::Project(project)) => {
                                    this.activate(Operation::SelectProject(project), window, cx)
                                }
                                Some(Route::EngineTab { connection, tab }) => {
                                    this.open_engine_tab(connection, tab, window, cx)
                                }
                                None => this.open_target(item.target.clone(), window, cx),
                            }
                        }
                    }
                }
                cx.notify();
            },
        ));
        self.palette = Some(palette);
        cx.notify();
    }

    /// The palette backdrop's click: dismiss, and nothing behind it reacts.
    pub(super) fn dismiss_palette(
        &mut self,
        _: &gpui::MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if let Some(palette) = self.palette.clone() {
            palette.update(cx, |palette, cx| palette.dismiss(cx));
        }
    }

    fn open_target(
        &mut self,
        target: Target<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            Target::Command(id) => {
                if let Some(operation) = command(id) {
                    self.activate(operation, window, cx);
                }
            }
            Target::Tab(id) => {
                if self
                    .documents
                    .iter()
                    .any(|document| document.metadata.id == id)
                {
                    self.activate(Operation::SelectDocument(id), window, cx);
                } else {
                    self.message = Some("That tab was closed".into());
                }
            }
            Target::Connection(id) => {
                if self
                    .connections
                    .iter()
                    .any(|connection| connection.id == id)
                {
                    self.activate(Operation::SelectConnection(id), window, cx);
                } else {
                    self.message = Some("That connection no longer exists".into());
                }
            }
            Target::Schema { connection, schema } => {
                if self.selected_connection.as_deref() == Some(connection.as_str()) {
                    self.navigator
                        .update(cx, |view, cx| view.reveal_schema(&schema, window, cx));
                } else {
                    self.message = Some("Select that connection and Load objects again".into());
                }
            }
            Target::Relation {
                connection,
                schema,
                name,
            } => {
                if !self.restored || self.documents.len() >= 16 {
                    self.message = Some("Close a tab before opening another (limit 16)".into());
                } else {
                    self.open_table_on(connection, schema, name, window, cx);
                }
            }
            Target::Object {
                connection,
                reference,
            } => {
                self.selected_connection = Some(connection);
                if let Some(index) = self.open_library(WorkspaceTool::Objects, window, cx) {
                    self.documents[index]
                        .view
                        .update(cx, |view, cx| view.describe_context(reference, cx));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_command_routes_to_an_operation() {
        let mut ids = std::collections::HashSet::new();
        for &(id, _) in COMMANDS {
            assert!(ids.insert(id), "duplicate command {id}");
            assert!(command(id).is_some(), "unrouted command {id}");
        }
        assert!(command("missing").is_none());
        for id in GENERAL_ONLY {
            assert!(ids.contains(id), "general-only {id} is not a command");
        }
    }

    #[test]
    fn connection_search_covers_every_engine_endpoint() {
        for connection in super::super::engines::tests::engine_records() {
            let endpoint = connection
                .endpoint()
                .expect("engine records carry settings");
            let keywords = connection_keywords(&connection);
            assert!(keywords.contains(&endpoint.database), "{keywords}");
            assert!(keywords.contains(&connection.engine), "{keywords}");
            if !endpoint.host.is_empty() {
                assert!(keywords.contains(&endpoint.host), "{keywords}");
            }
        }
        // SQLite is found by its file path.
        let sqlite = super::super::engines::tests::engine_records()
            .into_iter()
            .find(|connection| connection.engine == "SQLite")
            .unwrap();
        assert!(connection_keywords(&sqlite).contains("/tmp/a.db"));
    }
}
