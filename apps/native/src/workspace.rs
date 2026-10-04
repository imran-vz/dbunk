//! Selected stage04 persistent Navigator. Documents own editors and results;
//! the workspace owns focus, fair draining, restoration and acknowledged saves.
#[path = "workspace_clickhouse.rs"]
mod clickhouse_integration;
#[path = "workspace_engines.rs"]
mod engines;
#[path = "workspace_health.rs"]
mod health;
#[path = "managed_view.rs"]
mod managed_view;
#[path = "workspace_palette.rs"]
mod palette;
#[path = "workspace_seed.rs"]
mod seed_integration;
#[path = "workspace_shell.rs"]
mod shell;
#[path = "workspace_sqlite.rs"]
mod sqlite_integration;

use crate::{
    controller::Host,
    document_view::{ConnectionPhase, DocumentEvent, DocumentResources, DocumentView},
    forms::{Form, FormEvent},
    persistence::{DraftWriter, SaveStatus},
};
use dbunk_lib::backend::{
    DevelopmentConnection, DevelopmentCredentialState, Layout, WorkspaceDensity, WorkspaceDocument,
    WorkspaceError, WorkspaceSelection, WorkspaceSnapshot, WorkspaceTool,
};
use gpui::{
    Context, Entity, FocusHandle, Focusable, Role, SharedString, Subscription, Task, Window,
    actions, div, prelude::*, px,
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

actions!(
    native_workspace,
    [
        NewTab,
        CloseTab,
        NextTab,
        PreviousTab,
        RenameTab,
        PinTab,
        MoveTabLeft,
        MoveTabRight,
        NewConnection,
        OpenTable,
        CredentialSettings,
        Disconnect,
        ClearResults,
        RetrySave,
        ExportDraft,
        DiscardDrafts,
        ToggleDensity,
        WidenNavigator,
        NarrowNavigator,
        FocusNavigator,
        OpenAnything,
        ToggleConsole,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        ToggleSidebar,
        ToggleStatusBar,
        ShowAllEnvironments,
        ShowDevelopment,
        ShowTest,
        ShowStaging,
        ShowProduction
    ]
);

struct Document {
    metadata: WorkspaceDocument,
    view: Entity<DocumentView>,
    _events: Subscription,
    _status: Subscription,
}

#[derive(Clone)]
enum Operation {
    New,
    Close,
    /// Close one tab from its own close button.
    CloseDocument(String),
    Next,
    Previous,
    Rename,
    Pin,
    Move(bool),
    NewConnection,
    OpenTable,
    Library(WorkspaceTool),
    SaveQuery,
    Credentials,
    Bastions,
    SelectDocument(String),
    /// Select a saved connection and make sure it has a session.
    SelectConnection(String),
    DisconnectConnection(String),
    EditConnection(String),
    DeleteConnection(String),
    DuplicateConnection(String),
    CopyConnectionUri(String),
    Favorite(String),
    Connect,
    Disconnect,
    Clear,
    Retry,
    Export,
    Discard,
    Density,
    Width(f32),
    Console,
    ToggleSidebar,
    ToggleStatusBar,
    EnvFilter(Option<dbunk_lib::backend::DevelopmentEnvironment>),
    ShellMenu(shell::ShellMenu),
    SelectProject(String),
    ManagedServers,
}

pub struct Workspace {
    shell: shell::ShellState,
    engines: engines::EngineSurfaces,
    host: Arc<Host>,
    pg_tools: Entity<crate::pg_tool_store::ToolStore>,
    restore_changes: crate::pg_tool_jobs::RestoreChanges,
    _pg_tool_events: Subscription,
    csv_transfers: Entity<crate::csv_transfer_store::CsvStore>,
    comparisons: Entity<crate::schema_compare_store::CompareStore>,
    copies: Entity<crate::table_copy_store::CopyStore>,
    _copy_events: Subscription,
    seeds: Entity<crate::table_seed_store::SeedStore>,
    _seed_events: Subscription,
    import_changes: crate::csv_transfer_model::ImportChanges,
    _csv_events: Subscription,
    documents: Vec<Document>,
    clickhouse: clickhouse_integration::ClickHouseState,
    navigator: Entity<crate::navigator_view::NavigatorView>,
    _navigator_events: Subscription,
    dock: Entity<crate::dock_view::DockView>,
    _dock_events: Subscription,
    palette: Option<Entity<crate::palette_view::PaletteView<&'static str>>>,
    _palette_events: Option<Subscription>,
    frecency: crate::open_anything::Frecency,
    health: health::HealthState,
    health_task: Option<Task<()>>,
    health_probe: Option<Task<()>>,
    active: Option<String>,
    selected_connection: Option<String>,
    /// Plan 031 step 4: native SQLite workspaces, one per connected SQLite
    /// connection.
    sqlite: sqlite_integration::SqliteHost,
    connections: Vec<DevelopmentConnection>,
    credential_state: Option<DevelopmentCredentialState>,
    layout: Layout,
    density: WorkspaceDensity,
    navigator_width: f32,
    wake: async_channel::Sender<()>,
    drain_cursor: usize,
    retained: Rc<Cell<usize>>,
    writer: Option<Arc<DraftWriter>>,
    save_status: SaveStatus,
    latest_revision: u64,
    busy: bool,
    resetting: bool,
    form_scope: Option<String>,
    form_credentials: bool,
    loading: bool,
    restored: bool,
    closing: bool,
    cleanup_failed: bool,
    load_error: Option<WorkspaceError>,
    message: Option<String>,
    /// Bumped by each user action; keys the error shake so a repeated
    /// identical error still moves.
    message_seq: u64,
    dialog: Option<Entity<Form>>,
    managed: Option<Entity<managed_view::ManagedServersView>>,
    _managed_events: Option<Subscription>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    _dialog_events: Option<Subscription>,
    _drain: Task<()>,
    loading_task: Option<Task<()>>,
    writer_task: Option<Task<()>>,
    action_task: Option<Task<()>>,
}

impl Workspace {
    pub fn new(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (wake, awakened) = async_channel::bounded(1);
        let drain = cx.spawn(async move |this, cx| {
            while awakened.recv().await.is_ok() {
                loop {
                    let pending = this.update(cx, |this, cx| this.drain(cx)).unwrap_or(false);
                    if !pending {
                        break;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(1))
                        .await;
                }
            }
        });
        let retained = Rc::new(Cell::new(crate::persistence::WORKSPACE_SAVE_ALLOWANCE));
        let pg_tools =
            cx.new(|cx| crate::pg_tool_store::ToolStore::new(host.clone(), retained.clone(), cx));
        let pg_tool_events = cx.subscribe(
            &pg_tools,
            |this, store, _: &crate::pg_tool_store::CaptureChanged, cx| {
                let Some(capture) = store.read(cx).capture() else {
                    return;
                };
                let Some(plan) = capture.restore_invalidation(&this.restore_changes) else {
                    return;
                };
                // At most 36 bounded connection IDs; release the store borrow before
                // invoking view updates. Missing revision coverage invalidates all.
                let connections = plan
                    .connections()
                    .map(|connections| connections.map(str::to_owned).collect::<Vec<_>>());
                for document in &this.documents {
                    if connections.as_ref().is_none_or(|connections| {
                        document
                            .metadata
                            .connection_id
                            .as_ref()
                            .is_some_and(|id| connections.contains(id))
                    }) {
                        document
                            .view
                            .update(cx, |view, cx| view.invalidate_after_restore(cx));
                    }
                }
                let capture = store
                    .read(cx)
                    .capture()
                    .expect("capture retained during invalidation");
                if let Some(plan) = capture.restore_invalidation(&this.restore_changes)
                    && let Err(error) = this.restore_changes.acknowledge(plan)
                {
                    this.message = Some(error.into());
                    cx.notify();
                }
            },
        );
        let comparisons = cx.new(|cx| {
            crate::schema_compare_store::CompareStore::new(host.clone(), retained.clone(), cx)
        });
        let csv_transfers = cx
            .new(|cx| crate::csv_transfer_store::CsvStore::new(host.clone(), retained.clone(), cx));
        let csv_events = cx.subscribe(
            &csv_transfers,
            |this, store, _: &crate::csv_transfer_store::CaptureChanged, cx| {
                let Some(capture) = store.read(cx).capture() else {
                    return;
                };
                let Some(plan) = capture.import_invalidation(&this.import_changes) else {
                    return;
                };
                // At most 36 bounded connection IDs; release the store borrow before
                // invoking view updates. Missing revision coverage invalidates all.
                let connections = plan
                    .connections()
                    .map(|connections| connections.map(str::to_owned).collect::<Vec<_>>());
                for document in &this.documents {
                    if connections.as_ref().is_none_or(|connections| {
                        document
                            .metadata
                            .connection_id
                            .as_ref()
                            .is_some_and(|id| connections.contains(id))
                    }) {
                        document
                            .view
                            .update(cx, |view, cx| view.invalidate_after_restore(cx));
                    }
                }
                let capture = store
                    .read(cx)
                    .capture()
                    .expect("capture retained during invalidation");
                if let Some(plan) = capture.import_invalidation(&this.import_changes)
                    && let Err(error) = this.import_changes.acknowledge(plan)
                {
                    this.message = Some(error.into());
                    cx.notify();
                }
            },
        );
        let copies = cx
            .new(|cx| crate::table_copy_store::CopyStore::new(host.clone(), retained.clone(), cx));
        let copy_events = cx.subscribe(
            &copies,
            |this, _, event: &crate::table_copy_store::CopyEvent, cx| match event {
                crate::table_copy_store::CopyEvent::Changed => this.changed(cx),
                crate::table_copy_store::CopyEvent::Persist(request) => {
                    this.persist_copy(*request, cx)
                }
                crate::table_copy_store::CopyEvent::DestinationChanged(connection) => {
                    for document in &this.documents {
                        if connection.as_ref().is_none_or(|connection| {
                            document.metadata.connection_id.as_ref() == Some(connection)
                        }) {
                            document
                                .view
                                .update(cx, |view, cx| view.invalidate_after_restore(cx));
                        }
                    }
                }
            },
        );
        let seeds = cx
            .new(|cx| crate::table_seed_store::SeedStore::new(host.clone(), retained.clone(), cx));
        let seed_events = cx.subscribe(
            &seeds,
            |this, _, event: &crate::table_seed_store::SeedEvent, cx| match event {
                crate::table_seed_store::SeedEvent::Changed => this.changed(cx),
                crate::table_seed_store::SeedEvent::Persist(request) => {
                    this.persist_seed(*request, cx)
                }
                crate::table_seed_store::SeedEvent::DestinationChanged(connection) => {
                    for document in &this.documents {
                        if connection.as_ref().is_none_or(|connection| {
                            document.metadata.connection_id.as_ref() == Some(connection)
                        }) {
                            document
                                .view
                                .update(cx, |view, cx| view.invalidate_after_restore(cx));
                        }
                    }
                }
            },
        );
        let navigator = cx.new(|cx| {
            crate::navigator_view::NavigatorView::new(
                host.clone(),
                wake.clone(),
                retained.clone(),
                window,
                cx,
            )
        });
        let navigator_events = cx.subscribe_in(
            &navigator,
            window,
            |this, _, event: &crate::navigator_view::NavigatorEvent, window, cx| {
                if this.closing
                    || this.busy
                    || this.loading
                    || this.dialog.is_some()
                    || this.palette.is_some()
                {
                    return;
                }
                this.message = None;
                match event {
                    crate::navigator_view::NavigatorEvent::OpenTable {
                        connection,
                        schema,
                        table,
                    } => {
                        if !this.restored || this.documents.len() >= 16 {
                            this.message =
                                Some("Close a tab before opening another (limit 16)".into());
                        } else {
                            this.open_table_on(
                                connection.clone(),
                                schema.clone(),
                                table.clone(),
                                window,
                                cx,
                            );
                        }
                    }
                    crate::navigator_view::NavigatorEvent::Describe {
                        connection,
                        reference,
                    } => {
                        this.selected_connection = Some(connection.clone());
                        if let Some(index) = this.open_library(WorkspaceTool::Objects, window, cx) {
                            this.documents[index].view.update(cx, |view, cx| {
                                view.describe_context(reference.clone(), cx)
                            });
                        }
                    }
                }
                cx.notify();
            },
        );
        let dock = cx.new(crate::dock_view::DockView::new);
        let dock_events = cx.subscribe_in(
            &dock,
            window,
            |this, _, _: &crate::dock_view::DockClosed, window, cx| {
                this.focus_active(window, cx);
                cx.notify();
            },
        );
        let shell = shell::ShellState::new(window, cx);
        let clickhouse = clickhouse_integration::ClickHouseState::new(host.clone(), window, cx);
        let mut workspace = Self {
            shell,
            clickhouse,
            dock,
            _dock_events: dock_events,
            navigator,
            _navigator_events: navigator_events,
            palette: None,
            _palette_events: None,
            frecency: Default::default(),
            health: Default::default(),
            health_task: None,
            health_probe: None,
            host,
            pg_tools,
            restore_changes: Default::default(),
            _pg_tool_events: pg_tool_events,
            csv_transfers,
            comparisons,
            copies,
            _copy_events: copy_events,
            seeds,
            _seed_events: seed_events,
            import_changes: Default::default(),
            _csv_events: csv_events,
            documents: Vec::new(),
            active: None,
            selected_connection: None,
            sqlite: Default::default(),
            engines: Default::default(),
            connections: Vec::new(),
            credential_state: None,
            layout: Layout::Stacked,
            density: WorkspaceDensity::default(),
            navigator_width: 240.,
            wake,
            drain_cursor: 0,
            retained,
            writer: None,
            save_status: SaveStatus::Saved,
            latest_revision: 0,
            busy: false,
            resetting: false,
            form_scope: None,
            form_credentials: false,
            loading: false,
            restored: false,
            closing: false,
            cleanup_failed: false,
            load_error: None,
            message: None,
            message_seq: 0,
            dialog: None,
            managed: None,
            _managed_events: None,
            focus: cx.focus_handle(),
            previous_focus: None,
            _dialog_events: None,
            _drain: drain,
            loading_task: None,
            writer_task: None,
            action_task: None,
        };
        // Recovery and empty startup still need a keyboard target for Quit and
        // workspace shortcuts before any document editor is available.
        window.focus(&workspace.focus, cx);
        workspace.reload(false, window, cx);
        workspace.start_health_ticks(window, cx);
        workspace
    }

    fn reload(&mut self, show_settings: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.closing {
            return;
        }
        self.loading = true;
        let restore = !self.restored;
        let backend = self.host.backend.clone();
        let load = self.host.runtime.spawn(async move {
            let settings = backend.development_settings().await?;
            let connections = backend.development_connections().await?;
            let workspace = if restore {
                Some(backend.load_development_workspace().await)
            } else {
                None
            };
            Ok::<_, String>((settings, connections, workspace))
        });
        self.loading_task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = load.await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                if this.closing || this.cleanup_failed {
                    return;
                }
                match loaded {
                    Ok(Ok((settings, connections, saved))) => {
                        this.credential_state = Some(settings.state);
                        this.connections = connections;
                        this.sqlite_reconcile(cx);
                        this.sync_engines(cx);
                        this.sync_clickhouse(cx);
                        for document in &this.documents {
                            document.view.update(cx, |view, cx| {
                                view.set_comparison_connections(
                                    &this.connections,
                                    &this.catalog_sources(),
                                    cx,
                                );
                                view.set_connection_metadata(&this.connections, cx);
                                view.set_library_connections(&this.connections, cx)
                            });
                        }
                        if let Some(saved) = saved {
                            match saved {
                                Ok(saved) => {
                                    let snapshot = saved.snapshot.unwrap_or_default();
                                    this.layout = snapshot.layout;
                                    this.density = snapshot.density;
                                    this.navigator_width = snapshot.navigator_width;
                                    this.active = snapshot.active_document_id;
                                    this.copies.update(cx, |store, cx| {
                                        store.restore(snapshot.copy_jobs, cx)
                                    });
                                    this.seeds.update(cx, |store, cx| {
                                        store.restore(snapshot.seed_jobs, cx)
                                    });
                                    for document in snapshot.documents {
                                        this.insert_document(document, window, cx);
                                    }
                                    let writer = DraftWriter::new(
                                        this.host.backend.clone(),
                                        this.host.runtime.clone(),
                                        saved.revision,
                                    );
                                    let mut status = writer.status();
                                    this.writer_task = Some(cx.spawn(async move |this, cx| {
                                        while status.changed().await.is_ok() {
                                            let state = status.borrow_and_update().clone();
                                            if this
                                                .update(cx, |this, cx| {
                                                    if state.revision >= this.latest_revision {
                                                        this.save_status = state.state;
                                                        cx.notify();
                                                    }
                                                })
                                                .is_err()
                                            {
                                                break;
                                            }
                                        }
                                    }));
                                    this.writer = Some(writer);
                                    this.restored = true;
                                    this.focus_active(window, cx);
                                }
                                Err(error) => {
                                    this.message = Some(error.to_string());
                                    this.load_error = Some(error);
                                }
                            }
                        }
                        if show_settings || settings.state != DevelopmentCredentialState::Ready {
                            this.form_credentials = true;
                            let form = cx.new(|cx| {
                                Form::credentials(this.host.clone(), settings, window, cx)
                            });
                            this.show_form(form, None, window, cx);
                        }
                    }
                    Ok(Err(error)) => this.message = Some(error),
                    Err(_) => this.message = Some("Workspace startup could not complete".into()),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn catalog_sources(&self) -> Vec<Entity<DocumentView>> {
        self.documents
            .iter()
            .filter(|document| document.metadata.tool == Some(WorkspaceTool::Objects))
            .map(|document| document.view.clone())
            .collect()
    }
    fn refresh_comparison_connections(&self, cx: &mut Context<Self>) {
        let catalogs = self.catalog_sources();
        for document in &self.documents {
            if document.metadata.tool == Some(WorkspaceTool::SchemaCompare) {
                document.view.update(cx, |view, cx| {
                    view.set_comparison_connections(&self.connections, &catalogs, cx)
                });
            }
        }
    }
    fn insert_document(
        &mut self,
        mut document: WorkspaceDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| {
            DocumentView::new(
                DocumentResources {
                    host: self.host.clone(),
                    wake: self.wake.clone(),
                    retained: self.retained.clone(),
                    pg_tools: self.pg_tools.clone(),
                    csv_transfers: self.csv_transfers.clone(),
                    comparisons: self.comparisons.clone(),
                    copies: self.copies.clone(),
                    seeds: self.seeds.clone(),
                },
                (&mut document, !self.restored),
                self.layout,
                window,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            view.set_comparison_connections(&self.connections, &self.catalog_sources(), cx);
            view.set_connection_metadata(&self.connections, cx);
            view.set_library_connections(&self.connections, cx)
        });
        if let Some(table) = &mut document.table {
            table.draft = None;
        }
        // The editor owns current SQL. Keeping the original in metadata would
        // duplicate it on every snapshot, even after the user cleared the editor.
        document.sql = String::new();
        let document_id = document.id.clone();
        let originating_view = view.clone();
        let events = cx.subscribe_in(
            &view,
            window,
            move |this, _, event, window, cx| match event {
                DocumentEvent::DatabaseChanged(connection) => {
                    if !this.documents.iter().any(|document| {
                        document.metadata.id == document_id
                            && document.metadata.connection_id.as_ref() == Some(connection)
                    }) {
                        return;
                    }
                    for document in &this.documents {
                        if document.metadata.connection_id.as_ref() == Some(connection) {
                            document
                                .view
                                .update(cx, |view, cx| view.invalidate_after_restore(cx));
                        }
                    }
                    this.navigator
                        .update(cx, |view, cx| view.mark_stale(Some(connection), cx));
                    this.refresh_comparison_connections(cx);
                    this.changed(cx);
                }
                DocumentEvent::OpenTableCopy {
                    connection,
                    schema,
                    table,
                } => {
                    this.selected_connection = Some(connection.clone());
                    if let Some(index) = this.open_library(WorkspaceTool::TableCopy, window, cx) {
                        this.documents[index].view.update(cx, |view, cx| {
                            view.copy_context(schema.clone(), table.clone(), cx)
                        });
                    }
                }
                DocumentEvent::OpenStructure {
                    connection,
                    schema,
                    table,
                } => {
                    this.selected_connection = Some(connection.clone());
                    if let Some(index) = this.open_library(WorkspaceTool::Objects, window, cx) {
                        this.documents[index].view.update(cx, |view, cx| {
                            view.structure_context(schema.clone(), table.clone(), cx)
                        });
                    }
                }
                DocumentEvent::OpenTableSeed {
                    connection,
                    schema,
                    table,
                } => {
                    this.selected_connection = Some(connection.clone());
                    if let Some(index) = this.open_library(WorkspaceTool::TableSeed, window, cx) {
                        this.documents[index].view.update(cx, |view, cx| {
                            view.seed_context(schema.clone(), table.clone(), cx)
                        });
                    }
                }
                DocumentEvent::OpenCsvTransfer {
                    connection,
                    direction,
                    target,
                } => {
                    this.selected_connection = Some(connection.clone());
                    if let Some(index) = this.open_library(WorkspaceTool::CsvTransfer, window, cx) {
                        this.documents[index].view.update(cx, |view, cx| {
                            view.csv_context(*direction, target.clone(), window, cx)
                        });
                    }
                }
                DocumentEvent::OpenPgTools {
                    connection,
                    operation,
                    context,
                } => {
                    this.selected_connection = Some(connection.clone());
                    if let Some(index) = this.open_library(WorkspaceTool::BackupRestore, window, cx)
                    {
                        this.documents[index].view.update(cx, |view, cx| {
                            view.pg_tool_context(*operation, context.clone(), cx)
                        });
                    }
                }
                DocumentEvent::OpenWholeTableCsv {
                    connection,
                    target,
                    null_token,
                } => {
                    // The source document carries the target. Navigator selection
                    // must never substitute a different connection for this route.
                    this.selected_connection = Some(connection.clone());
                    if let Some(index) = this.open_library(WorkspaceTool::CsvTransfer, window, cx) {
                        this.documents[index].view.update(cx, |view, cx| {
                            view.whole_table_csv_context(
                                target.clone(),
                                null_token.clone(),
                                window,
                                cx,
                            )
                        });
                    }
                }
                DocumentEvent::OpenTable {
                    connection,
                    schema,
                    table,
                    filters,
                } => this.open_table_filtered(
                    connection.clone(),
                    schema.clone(),
                    table.clone(),
                    filters.clone(),
                    window,
                    cx,
                ),
                DocumentEvent::EditConnection(id) => {
                    if this.documents.iter().any(|document| {
                        document.metadata.id == document_id
                            && document.metadata.connection_id.as_ref() == Some(id)
                    }) {
                        this.activate(Operation::EditConnection(id.clone()), window, cx);
                    }
                }
                DocumentEvent::OpenLibraryQuery(query) => {
                    this.open_library_query(query, window, cx)
                }
                DocumentEvent::CachedSchemasChanged => this.refresh_comparison_connections(cx),
                DocumentEvent::DraftChanged => this.changed(cx),
                DocumentEvent::LayoutChanged(layout) => {
                    this.layout = *layout;
                    for document in &this.documents {
                        document
                            .view
                            .update(cx, |view, cx| view.set_document_layout(*layout, cx));
                    }
                    this.changed(cx);
                }
                DocumentEvent::Quit => this.close(window, cx),
                DocumentEvent::Console(entry) => {
                    let mut entry = entry.clone();
                    if let (Some(latency), Some(connection)) = (entry.latency_ms, &entry.connection)
                    {
                        this.shell.last_latency.insert(connection.clone(), latency);
                    }
                    if let Some(name) = entry.connection.as_ref().and_then(|id| {
                        this.connections
                            .iter()
                            .find(|connection| &connection.id == id)
                            .map(|connection| connection.name.clone())
                    }) {
                        entry.message = format!("{name} · {}", entry.message);
                    }
                    this.dock.update(cx, |dock, cx| dock.append(entry, cx));
                }
                DocumentEvent::PersistApply(request) => {
                    this.persist_apply(&document_id, originating_view.clone(), *request, cx)
                }
            },
        );
        let status = cx.observe(&view, |_, _, cx| cx.notify());
        self.documents.push(Document {
            metadata: document,
            view,
            _events: events,
            _status: status,
        });
    }

    pub fn snapshot(&self, cx: &mut gpui::App) -> WorkspaceSnapshot {
        let (transient, active_document_id) = self.transient_documents(cx);
        WorkspaceSnapshot {
            documents: self
                .documents
                .iter()
                .filter(|document| !transient.contains(&document.metadata.id))
                .map(|document| {
                    let mut metadata = document.metadata.clone();
                    if metadata.table.is_none() {
                        let (sql, selection) = document.view.update(cx, |view, cx| view.draft(cx));
                        metadata.sql = sql;
                        metadata.selection = selection;
                    }
                    metadata.table = document.view.read(cx).table_state(cx);
                    metadata.query_changes = document.view.read(cx).query_changes(cx);
                    metadata.schema_changes = document.view.read(cx).schema_changes(cx);
                    metadata.table_ddl = document.view.read(cx).table_ddl(cx);
                    metadata.schema_alter = document.view.read(cx).schema_alter(cx);
                    metadata.object_ddl = document.view.read(cx).object_ddl(cx);
                    metadata.admin_control = document.view.read(cx).admin_control(cx);
                    metadata.maintenance = document.view.read(cx).maintenance(cx);
                    metadata
                })
                .collect(),
            active_document_id,
            layout: self.layout,
            density: self.density,
            navigator_width: self.navigator_width,
            copy_jobs: self.copies.read(cx).snapshot(),
            seed_jobs: self.seeds.read(cx).snapshot(),
        }
    }
    fn snapshot_payload_bytes(&self, cx: &gpui::App) -> usize {
        self.documents.iter().fold(
            self.copies
                .read(cx)
                .snapshot_bytes()
                .saturating_add(self.seeds.read(cx).snapshot_bytes()),
            |bytes, document| {
                bytes.saturating_add(document.view.read(cx).snapshot_payload_bytes(cx))
            },
        )
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        if self.closing || !self.restored {
            return;
        }
        if let Some(writer) = &self.writer {
            let payload_bytes = self.snapshot_payload_bytes(cx);
            // This is a lower bound on encoded size, checked before SQL/draft
            // clones. The writer still checks the exact complete envelope.
            let snapshot = if payload_bytes > dbunk_lib::backend::NATIVE_WORKSPACE_MAX_BYTES {
                Err(dbunk_lib::backend::WorkspaceError::TooLarge)
            } else {
                Ok(self.snapshot(cx))
            };
            self.save_status = match writer.submit_prepared(snapshot) {
                Ok(revision) => {
                    self.latest_revision = revision;
                    SaveStatus::Pending
                }
                Err(error) => {
                    self.latest_revision = self.latest_revision.saturating_add(1);
                    SaveStatus::Failed(error)
                }
            };
        }
        cx.notify();
    }
    /// Persist the originating document's pending-apply marker before allowing
    /// its runtime to dispatch. Active-tab changes never choose the recipient.
    fn persist_apply(
        &mut self,
        document_id: &str,
        view: Entity<DocumentView>,
        request: u64,
        cx: &mut Context<Self>,
    ) {
        let refused = if !self.documents.iter().any(|document| {
            document.metadata.id == document_id && document.view.entity_id() == view.entity_id()
        }) {
            Some(
                "The originating document is no longer available; changes were not applied"
                    .to_string(),
            )
        } else if self.closing || self.cleanup_failed {
            Some("Workspace is closing; changes were not applied".to_string())
        } else if self.busy || self.loading || self.dialog.is_some() || !self.restored {
            Some("Finish the current workspace operation before applying changes".to_string())
        } else if self.writer.is_none() {
            Some("Draft persistence is unavailable; changes were not applied".to_string())
        } else {
            None
        };
        if let Some(error) = refused {
            view.update(cx, |view, cx| view.apply_saved(request, Err(error), cx));
            return;
        }
        self.changed(cx);
        if let SaveStatus::Failed(error) = &self.save_status {
            let error = error.to_string();
            view.update(cx, |view, cx| view.apply_saved(request, Err(error), cx));
            return;
        }
        let writer = self.writer.as_ref().unwrap().clone();
        let target = self.latest_revision;
        let document_id = document_id.to_owned();
        self.busy = true;
        let task = self
            .host
            .runtime
            .spawn(async move { writer.flush_revision(target).await });
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = task
                .await
                .map_err(|_| "Draft persistence task failed; changes were not applied".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            this.update(cx, |this, cx| {
                this.busy = false;
                let still_owned = this.documents.iter().any(|document| {
                    document.metadata.id == document_id
                        && document.view.entity_id() == view.entity_id()
                });
                let result = if this.closing || this.cleanup_failed || !still_owned {
                    Err(
                        "The originating document is no longer available; changes were not applied"
                            .to_string(),
                    )
                } else {
                    result
                };
                if let Err(error) = &result {
                    this.message = Some(error.clone());
                }
                view.update(cx, |view, cx| view.apply_saved(request, result, cx));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
    fn persist_copy(&mut self, request: u64, cx: &mut Context<Self>) {
        let refuse = if self.closing
            || self.cleanup_failed
            || self.busy
            || self.loading
            || self.dialog.is_some()
            || !self.restored
            || self.writer.is_none()
        {
            Some(
                "Workspace persistence is unavailable or busy; copy was not dispatched".to_string(),
            )
        } else {
            None
        };
        if let Some(error) = refuse {
            self.copies
                .update(cx, |store, cx| store.saved(request, Err(error), cx));
            return;
        }
        self.changed(cx);
        if let SaveStatus::Failed(error) = &self.save_status {
            let error = error.to_string();
            self.copies
                .update(cx, |store, cx| store.saved(request, Err(error), cx));
            return;
        }
        let writer = self.writer.as_ref().unwrap().clone();
        let revision = self.latest_revision;
        self.busy = true;
        let task = self
            .host
            .runtime
            .spawn(async move { writer.flush_revision(revision).await });
        self.action_task = Some(cx.spawn(async move |this, cx| {
            let result = task
                .await
                .map_err(|_| "Copy recovery save did not complete".to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            this.update(cx, |this, cx| {
                this.busy = false;
                let result = if this.closing || this.cleanup_failed {
                    Err("Workspace is closing; copy was not dispatched".into())
                } else {
                    result
                };
                this.copies
                    .update(cx, |store, cx| store.saved(request, result, cx));
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
    fn active_index(&self) -> Option<usize> {
        self.documents
            .iter()
            .position(|document| Some(&document.metadata.id) == self.active.as_ref())
    }
    fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_engine().is_some() {
            self.focus_engine(window, cx);
        } else if let Some(index) = self.active_index() {
            self.documents[index]
                .view
                .update(cx, |view, cx| view.focus_document(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
    }
    fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_engine().is_some() {
            return;
        }
        if let Some(index) = self.active_index() {
            self.documents[index]
                .view
                .update(cx, |view, cx| view.remember_focus(window, cx));
        }
    }
    fn new_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.restored || self.documents.len() >= 16 {
            self.message = Some("Close a query tab before opening another (limit 16)".into());
            cx.notify();
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let number = (1..)
            .find(|number| {
                !self
                    .documents
                    .iter()
                    .any(|document| document.metadata.name == format!("Query {number}"))
            })
            .unwrap();
        self.insert_document(
            WorkspaceDocument {
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
                name: format!("Query {number}"),
                connection_id: self.selected_connection.clone(),
                sql: String::new(),
                pinned: false,
                selection: WorkspaceSelection::default(),
            },
            window,
            cx,
        );
        self.active = Some(id);
        self.changed(cx);
        self.focus_active(window, cx);
    }
    fn open_library(
        &mut self,
        kind: WorkspaceTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if let Some(index) = self.documents.iter().position(|document| {
            document.metadata.tool == Some(kind)
                && (!matches!(
                    kind,
                    WorkspaceTool::Objects
                        | WorkspaceTool::Administration
                        | WorkspaceTool::BackupRestore
                        | WorkspaceTool::CsvTransfer
                        | WorkspaceTool::SchemaCompare
                        | WorkspaceTool::SchemaMap
                        | WorkspaceTool::TableCopy
                        | WorkspaceTool::TableSeed
                ) || document.metadata.connection_id == self.selected_connection)
        }) {
            self.active = Some(self.documents[index].metadata.id.clone());
            self.focus_active(window, cx);
            self.changed(cx);
            return Some(index);
        }
        if !self.restored || self.documents.len() >= 16 {
            self.message = Some("Close a tab before opening another (limit 16)".into());
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.insert_document(
            WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                id: id.clone(),
                name: match kind {
                    WorkspaceTool::Objects => "Objects",
                    WorkspaceTool::Administration => "Administration",
                    WorkspaceTool::History => "History",
                    WorkspaceTool::SavedQueries => "Saved queries",
                    WorkspaceTool::BackupRestore => "Backup / Restore",
                    WorkspaceTool::CsvTransfer => "CSV transfer",
                    WorkspaceTool::SchemaCompare => "Schema comparison",
                    WorkspaceTool::SchemaMap => "Schema map",
                    WorkspaceTool::TableCopy => "Table copy",
                    WorkspaceTool::TableSeed => "Seed table",
                }
                .into(),
                connection_id: self.selected_connection.clone(),
                sql: String::new(),
                pinned: false,
                selection: WorkspaceSelection::default(),
                table: None,
                tool: Some(kind),
                saved_query_id: None,
            },
            window,
            cx,
        );
        self.active = Some(id);
        self.changed(cx);
        self.focus_active(window, cx);
        if kind == WorkspaceTool::Objects {
            let index = self.documents.len() - 1;
            self.documents[index]
                .view
                .update(cx, |view, cx| view.begin_connect(cx));
        }
        Some(self.documents.len() - 1)
    }
    fn open_library_query(
        &mut self,
        query: &crate::query_library_view::OpenQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.busy || !self.restored || self.documents.len() >= 16 {
            self.message = Some("Close a tab before opening another query".into());
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.insert_document(
            WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                id: id.clone(),
                name: query.name.clone(),
                connection_id: query.connection.clone(),
                sql: query.sql.clone(),
                pinned: false,
                selection: WorkspaceSelection::default(),
                table: None,
                tool: None,
                saved_query_id: query.saved_id.clone(),
            },
            window,
            cx,
        );
        self.active = Some(id);
        self.changed(cx);
        self.focus_active(window, cx);
    }
    fn save_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_index() else {
            return;
        };
        if self.documents[index].metadata.tool.is_some()
            || self.documents[index].metadata.table.is_some()
        {
            self.message = Some("Open a query editor to save SQL".into());
            return;
        }
        let (sql, _) = self.documents[index]
            .view
            .update(cx, |view, cx| view.draft(cx));
        let metadata = &mut self.documents[index].metadata;
        let id = metadata
            .saved_query_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let query = dbunk_lib::backend::query_library::SavedQueryRecord {
            id,
            name: metadata.name.clone(),
            body: sql,
            connection_id: metadata.connection_id.clone(),
            is_favorite: false,
            owner_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        self.changed(cx);
        if let Some(index) = self.open_library(WorkspaceTool::SavedQueries, window, cx) {
            self.documents[index]
                .view
                .update(cx, |view, cx| view.save_query(query, cx));
        }
    }
    fn open_table(
        &mut self,
        schema: String,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.restored || self.documents.len() >= 16 {
            self.message = Some("Close a tab before opening another (limit 16)".into());
            return;
        }
        let Some(connection) = self.selected_connection.clone() else {
            self.message = Some("Select a connection first".into());
            return;
        };
        self.open_table_on(connection, schema, table, window, cx);
    }
    fn open_table_on(
        &mut self,
        connection: String,
        schema: String,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_table_filtered(connection, schema, table, Vec::new(), window, cx);
    }
    fn open_table_filtered(
        &mut self,
        connection: String,
        schema: String,
        table: String,
        filters: Vec<dbunk_lib::backend::data::BrowseFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.busy || !self.restored || self.documents.len() >= 16 {
            self.message = Some("Close a tab before opening another (limit 16)".into());
            cx.notify();
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.insert_document(
            WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                tool: None,
                saved_query_id: None,
                id: id.clone(),
                name: format!("{schema}.{table}"),
                connection_id: Some(connection),
                sql: String::new(),
                pinned: false,
                selection: WorkspaceSelection::default(),
                table: Some(dbunk_lib::backend::WorkspaceTableState {
                    schema,
                    table,
                    filters,
                    sort: Vec::new(),
                    page_size: 100,
                    draft: None,
                }),
            },
            window,
            cx,
        );
        self.active = Some(id);
        self.previous_focus = None;
        self.changed(cx);
        if let Some(index) = self.active_index() {
            self.documents[index]
                .view
                .update(cx, |view, cx| view.begin_connect(cx));
        }
    }
    fn select_next(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.documents.is_empty() {
            return;
        }
        self.remember_focus(window, cx);
        let index = self.active_index().unwrap_or(0);
        let next = if backwards {
            (index + self.documents.len() - 1) % self.documents.len()
        } else {
            (index + 1) % self.documents.len()
        };
        self.active = Some(self.documents[next].metadata.id.clone());
        self.changed(cx);
        self.focus_active(window, cx);
    }
    fn close_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.active_index() else {
            return;
        };
        if self.documents[index]
            .view
            .read(cx)
            .has_recoverable_changes(cx)
        {
            self.message =
                Some("Review or explicitly discard pending changes before closing this tab".into());
            cx.notify();
            return;
        }
        if self.documents[index].metadata.pinned {
            self.message = Some("Unpin this query before closing it".into());
            cx.notify();
            return;
        }
        if self.documents[index]
            .view
            .read(cx)
            .table_state(cx)
            .is_some_and(|table| table.draft.is_some_and(|draft| !draft.changes.is_empty()))
        {
            self.message = Some("Pending table changes must be reviewed or explicitly discarded before closing the tab. Export drafts to keep a copy.".into());
            cx.notify();
            return;
        }
        if matches!(self.save_status, SaveStatus::Failed(_)) {
            self.message = Some("Export or retry unsaved drafts before closing a tab".into());
            cx.notify();
            return;
        }
        self.busy = true;
        self.documents[index]
            .view
            .update(cx, |view, cx| view.set_editable(false, cx));
        let id = self.documents[index].metadata.id.clone();
        let host = self.host.clone();
        let close = self
            .host
            .runtime
            .spawn(async move { host.close_document(&id).await });
        let id = self.documents[index].metadata.id.clone();
        self.action_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = close.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(())) => {
                        let Some(index) = this
                            .documents
                            .iter()
                            .position(|document| document.metadata.id == id)
                        else {
                            return;
                        };
                        this.documents.remove(index);
                        this.refresh_comparison_connections(cx);
                        this.active = this
                            .documents
                            .get(index.min(this.documents.len().saturating_sub(1)))
                            .map(|document| document.metadata.id.clone());
                        this.changed(cx);
                        this.focus_active(window, cx);
                    }
                    _ => {
                        this.message = Some("Query cleanup failed; the tab remains open".into());
                        if let Some(document) = this
                            .documents
                            .iter()
                            .find(|document| document.metadata.id == id)
                        {
                            document
                                .view
                                .update(cx, |view, cx| view.set_editable(true, cx));
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }
    fn drain(&mut self, cx: &mut Context<Self>) -> bool {
        // The Navigator lane is not a workspace document; it shares this wake.
        let navigator = self.navigator.update(cx, |view, cx| view.drain_one(cx));
        if self.documents.is_empty() {
            return navigator;
        }
        let started = Instant::now();
        let mut consumed = 0;
        let mut empty = 0;
        while consumed < 8
            && empty < self.documents.len()
            && started.elapsed() < Duration::from_millis(2)
        {
            let index = self.drain_cursor % self.documents.len();
            self.drain_cursor = (index + 1) % self.documents.len();
            if self.documents[index]
                .view
                .update(cx, |view, cx| view.drain_one(cx))
            {
                consumed += 1;
                empty = 0;
            } else {
                empty += 1;
            }
        }
        navigator
            || self
                .documents
                .iter()
                .any(|document| document.view.read(cx).has_pending(cx))
    }

    fn show_form(
        &mut self,
        form: Entity<Form>,
        rename: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._dialog_events =
            Some(
                cx.subscribe_in(&form, window, move |this, _, event, window, cx| {
                    match event {
                        FormEvent::OpenTable { schema, table } => {
                            this.open_table(schema.clone(), table.clone(), window, cx)
                        }
                        FormEvent::Cancelled => {}
                        FormEvent::Saved => this.settle_form_change(window, cx),
                        FormEvent::Renamed(name) => {
                            if let Some(document) = this
                                .documents
                                .iter_mut()
                                .find(|document| Some(&document.metadata.id) == rename.as_ref())
                            {
                                document.metadata.name = name.clone();
                                this.changed(cx);
                            }
                        }
                        FormEvent::ConfirmedDiscard => {
                            if this.resetting {
                                this.reset_saved_workspace(window, cx);
                            } else {
                                this.finish_close(true, window, cx);
                            }
                        }
                    }
                    this.dialog = None;
                    this._dialog_events = None;
                    if let Some(focus) = &this.previous_focus {
                        window.focus(focus, cx);
                    } else {
                        this.focus_active(window, cx);
                    }
                    cx.notify();
                }),
            );
        form.update(cx, |form, cx| form.focus(window, cx));
        self.dialog = Some(form);
        cx.notify();
    }

    fn activate(&mut self, operation: Operation, window: &mut Window, cx: &mut Context<Self>) {
        // View-only shell state never waits on persistence or dialogs.
        let opened_menu = matches!(operation, Operation::ShellMenu(_));
        if !opened_menu && self.shell.menu.take().is_some() {
            cx.notify();
        }
        match &operation {
            Operation::ToggleSidebar => {
                self.shell.sidebar_collapsed = !self.shell.sidebar_collapsed;
                self.shell.sidebar_toggled = true;
                self.shell.sidebar_settling = true;
                self.shell.sidebar_epoch += 1;
                let epoch = self.shell.sidebar_epoch;
                // Both ends stay rendered until the spring has settled.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(450))
                        .await;
                    this.update(cx, |this, cx| {
                        if this.shell.sidebar_epoch == epoch {
                            this.shell.sidebar_settling = false;
                            cx.notify();
                        }
                    })
                    .ok();
                })
                .detach();
                cx.notify();
                return;
            }
            Operation::ToggleStatusBar => {
                self.shell.status_collapsed = !self.shell.status_collapsed;
                cx.notify();
                return;
            }
            Operation::EnvFilter(environment) => {
                self.shell.env_filter = *environment;
                cx.notify();
                return;
            }
            Operation::ShellMenu(menu) => {
                self.shell.menu = (self.shell.menu.as_ref() != Some(menu)).then(|| menu.clone());
                cx.notify();
                return;
            }
            Operation::SelectProject(project) => {
                self.shell.project = Some(project.clone());
                cx.notify();
                return;
            }
            _ => {}
        }
        if self.closing
            || self.busy
            || self.loading
            || self.dialog.is_some()
            || self.palette.is_some()
            || self.managed.is_some()
        {
            return;
        }
        if self.cleanup_failed && !matches!(operation, Operation::Retry | Operation::Export) {
            return;
        }
        self.previous_focus = window.focused(cx);
        self.message = None;
        self.message_seq = self.message_seq.wrapping_add(1);
        self.form_scope = None;
        self.form_credentials = false;
        self.resetting = false;
        if self.sqlite_operation(&operation, window, cx) {
            cx.notify();
            return;
        }
        if self.engine_intercepts(&operation, window, cx) {
            return;
        }
        match operation {
            Operation::Library(kind) => {
                self.open_library(kind, window, cx);
            }
            Operation::SaveQuery => self.save_query(window, cx),
            Operation::OpenTable if self.clickhouse_selected().is_some() => {
                self.message = Some("Open ClickHouse tables from the object tree".into());
            }
            Operation::OpenTable => {
                if self.selected_connection.is_none() {
                    self.message = Some("Select a connection first".into());
                } else {
                    let form = cx.new(|cx| Form::open_table(self.host.clone(), window, cx));
                    self.show_form(form, None, window, cx);
                }
            }
            Operation::New => {
                if !self.new_clickhouse_query(window, cx) {
                    self.new_document(window, cx)
                }
            }
            Operation::Close => self.close_document(window, cx),
            Operation::CloseDocument(id) => {
                if self
                    .documents
                    .iter()
                    .any(|document| document.metadata.id == id)
                {
                    self.active = Some(id);
                    self.close_document(window, cx);
                }
            }
            Operation::Next => self.select_next(false, window, cx),
            Operation::Previous => self.select_next(true, window, cx),
            Operation::SelectDocument(id) => {
                self.remember_focus(window, cx);
                self.active = Some(id);
                self.changed(cx);
                self.focus_active(window, cx);
            }
            Operation::SelectConnection(id) if self.is_clickhouse(&id) => {
                self.select_clickhouse(id, cx)
            }
            Operation::SelectConnection(id) => {
                self.selected_connection = Some(id.clone());
                if let Some(index) = self.active_index()
                    && self.documents[index].metadata.connection_id.is_none()
                    && (self.documents[index].metadata.tool.is_none()
                        || matches!(
                            self.documents[index].metadata.tool,
                            Some(
                                WorkspaceTool::Objects
                                    | WorkspaceTool::Administration
                                    | WorkspaceTool::BackupRestore
                                    | WorkspaceTool::CsvTransfer
                                    | WorkspaceTool::SchemaCompare
                                    | WorkspaceTool::SchemaMap
                                    | WorkspaceTool::TableCopy
                                    | WorkspaceTool::TableSeed
                            )
                        ))
                    && !self.documents[index]
                        .view
                        .read(cx)
                        .has_recoverable_changes(cx)
                {
                    self.documents[index].metadata.connection_id = Some(id.clone());
                    self.documents[index].view.update(cx, |view, cx| {
                        view.bind_connection(id.clone(), cx);
                        view.set_connection_metadata(&self.connections, cx);
                    });
                    self.changed(cx);
                }
                self.connect_selection(id, window, cx);
            }
            Operation::DisconnectConnection(id) if self.is_clickhouse(&id) => {
                self.disconnect_clickhouse(id, cx)
            }
            Operation::DisconnectConnection(id) => self.disconnect_connection(id, window, cx),
            Operation::Rename => {
                if let Some(index) = self.active_index() {
                    let document = &self.documents[index].metadata;
                    let id = document.id.clone();
                    let form = cx.new(|cx| {
                        Form::rename(self.host.clone(), document.name.clone(), window, cx)
                    });
                    self.show_form(form, Some(id), window, cx);
                }
            }
            Operation::Pin => {
                if let Some(index) = self.active_index() {
                    self.documents[index].metadata.pinned = !self.documents[index].metadata.pinned;
                    self.changed(cx);
                }
            }
            Operation::Move(left) => {
                if let Some(index) = self.active_index() {
                    let target = if left {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(self.documents.len() - 1)
                    };
                    self.documents.swap(index, target);
                    self.changed(cx);
                }
            }
            Operation::NewConnection => {
                let form = cx.new(|cx| {
                    Form::connection(self.host.clone(), None, self.retained.clone(), window, cx)
                });
                self.show_form(form, None, window, cx);
            }
            Operation::Credentials => self.reload(true, window, cx),
            Operation::Bastions => {
                let form = cx.new(|cx| Form::bastions(self.host.clone(), window, cx));
                self.show_form(form, None, window, cx);
            }
            Operation::ManagedServers => self.show_managed(window, cx),
            Operation::EditConnection(id) => {
                self.form_scope = Some(id.clone());
                if let Some(connection) =
                    crate::connection_settings_model::editable_connection(&self.connections, &id)
                        .cloned()
                {
                    let form = cx.new(|cx| {
                        Form::connection(
                            self.host.clone(),
                            Some(connection),
                            self.retained.clone(),
                            window,
                            cx,
                        )
                    });
                    self.show_form(form, None, window, cx);
                } else {
                    self.message = Some("Connection is missing or unsupported; reload saved connections before editing".into());
                }
            }
            Operation::DeleteConnection(id) => {
                self.form_scope = Some(id.clone());
                if let Some(connection) = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == id)
                    .cloned()
                {
                    let form = cx.new(|cx| {
                        Form::delete_connection(self.host.clone(), connection, window, cx)
                    });
                    self.show_form(form, None, window, cx);
                }
            }
            Operation::CopyConnectionUri(id) => {
                let result = self
                    .connections
                    .iter()
                    .find(|connection| {
                        connection.id == id && connection.unsupported_reason.is_none()
                    })
                    .and_then(|connection| connection.postgres.as_ref())
                    .ok_or_else(|| "URI copy is unavailable for this connection".to_owned())
                    .and_then(|input| crate::connection_uri::copy(input, cx));
                self.message = Some(match result {
                    Ok(message) | Err(message) => message,
                });
            }
            Operation::DuplicateConnection(id) => {
                let backend = self.host.backend.clone();
                self.connection_change(
                    async move {
                        backend
                            .duplicate_development_connection(id)
                            .await
                            .map(|_| ())
                    },
                    window,
                    cx,
                );
            }
            Operation::Favorite(id) => {
                if let Some(connection) = self
                    .connections
                    .iter()
                    .find(|connection| connection.id == id)
                {
                    let mut organization = connection.organization.clone();
                    organization.is_favorite = !organization.is_favorite;
                    let backend = self.host.backend.clone();
                    self.connection_change(
                        async move {
                            backend
                                .organize_development_connection(id, organization)
                                .await
                                .map(|_| ())
                        },
                        window,
                        cx,
                    );
                }
            }
            Operation::Connect => {
                if let Some(index) = self.active_index() {
                    self.documents[index]
                        .view
                        .update(cx, |view, cx| view.begin_connect(cx));
                }
            }
            Operation::Disconnect => {
                if let Some(index) = self.active_index() {
                    if self.documents[index]
                        .view
                        .read(cx)
                        .has_recoverable_changes(cx)
                    {
                        self.message = Some("Resolve pending changes before disconnecting".into());
                        cx.notify();
                        return;
                    }
                    self.busy = true;
                    self.documents[index]
                        .view
                        .update(cx, |view, cx| view.set_editable(false, cx));
                    let id = self.documents[index].metadata.id.clone();
                    let host = self.host.clone();
                    let task = self
                        .host
                        .runtime
                        .spawn(async move { host.disconnect_document(&id).await });
                    let view = self.documents[index].view.clone();
                    self.action_task = Some(cx.spawn(async move |this, cx| {
                        let result = task.await;
                        this.update(cx, |this, cx| {
                            this.busy = false;
                            view.update(cx, |view, cx| view.set_editable(true, cx));
                            match result {
                                Ok(Ok(())) => {
                                    view.update(cx, |view, cx| view.mark_disconnected(cx))
                                }
                                _ => this.message = Some("Disconnect cleanup failed".into()),
                            }
                            cx.notify();
                        })
                        .ok();
                    }));
                }
            }
            Operation::Console => self.toggle_console(window, cx),
            // Handled before the busy/dialog guard above.
            Operation::ToggleSidebar
            | Operation::ToggleStatusBar
            | Operation::EnvFilter(_)
            | Operation::ShellMenu(_)
            | Operation::SelectProject(_) => {}
            Operation::Clear => {
                if let Some(index) = self.active_index() {
                    self.documents[index]
                        .view
                        .update(cx, |view, cx| view.clear_results(cx));
                }
            }
            Operation::Retry => {
                if self.cleanup_failed {
                    self.finish_close(false, window, cx);
                } else if let Some(writer) = &self.writer {
                    writer.retry();
                } else {
                    self.reload(false, window, cx);
                }
            }
            Operation::Export => self.export_draft(window, cx),
            Operation::Discard => {
                self.resetting = self.load_error.is_some();
                let form = cx.new(|cx| {
                    if self.resetting {
                        Form::reset_workspace(self.host.clone(), window, cx)
                    } else {
                        Form::discard_drafts(self.host.clone(), window, cx)
                    }
                });
                self.show_form(form, None, window, cx);
            }
            Operation::Density => {
                self.density = match self.density {
                    WorkspaceDensity::Compact => WorkspaceDensity::Comfortable,
                    WorkspaceDensity::Comfortable => WorkspaceDensity::Compact,
                };
                self.changed(cx);
            }
            Operation::Width(delta) => {
                self.navigator_width = (self.navigator_width + delta).clamp(160., 480.);
                self.changed(cx);
            }
        }
        cx.notify();
    }

    /// Connects the document that represents `id`: the active document when
    /// it is bound there, else the first bound document, else a new query.
    /// Never restarts a session that is already open or opening.
    fn connect_selection(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.connections.iter().any(|connection| {
            connection.id == id
                && connection.postgres.is_some()
                && connection.unsupported_reason.is_none()
        }) {
            return;
        }
        let bound = |document: &Document| document.metadata.connection_id.as_deref() == Some(&id);
        let index = match self
            .active_index()
            .filter(|&index| bound(&self.documents[index]))
            .or_else(|| self.documents.iter().position(&bound))
        {
            Some(index) => index,
            None => {
                self.new_document(window, cx);
                match self
                    .active_index()
                    .filter(|&index| bound(&self.documents[index]))
                {
                    Some(index) => index,
                    None => return,
                }
            }
        };
        if self.active_index() != Some(index) {
            self.remember_focus(window, cx);
            self.active = Some(self.documents[index].metadata.id.clone());
            self.changed(cx);
            self.focus_active(window, cx);
        }
        let view = self.documents[index].view.clone();
        let idle = {
            let view = view.read(cx);
            // Table-lane documents ignore a connect while their lane is open.
            !view.is_query()
                || matches!(
                    view.connection_phase(cx),
                    ConnectionPhase::Idle | ConnectionPhase::Failed(_)
                )
        };
        if idle {
            view.update(cx, |view, cx| view.begin_connect(cx));
        }
    }

    /// Closes every session on `id`, refusing while a bound document holds
    /// changes that a disconnect would discard.
    fn disconnect_connection(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let bound: Vec<Entity<DocumentView>> = self
            .documents
            .iter()
            .filter(|document| document.metadata.connection_id.as_deref() == Some(&id))
            .map(|document| document.view.clone())
            .collect();
        if bound
            .iter()
            .any(|view| view.read(cx).has_recoverable_changes(cx))
        {
            self.message = Some("Resolve pending changes before disconnecting".into());
            return;
        }
        self.busy = true;
        for view in &bound {
            view.update(cx, |view, cx| view.set_editable(false, cx));
        }
        let host = self.host.clone();
        let target = id.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { host.disconnect_connection_documents(&target).await });
        self.action_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                for view in &bound {
                    view.update(cx, |view, cx| {
                        view.set_editable(true, cx);
                        view.mark_disconnected(cx);
                    });
                }
                if !matches!(result, Ok(Ok(()))) {
                    this.message = Some("Disconnect cleanup failed".into());
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn connection_change(
        &mut self,
        future: impl std::future::Future<Output = Result<(), String>> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        let task = self.host.runtime.spawn(future);
        self.action_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(())) => this.reload(false, window, cx),
                    Ok(Err(error)) => this.message = Some(error),
                    Err(_) => this.message = Some("Connection operation failed".into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn export_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.load_error.is_some();
        let lease = match crate::persistence::ExportSnapshotLease::admit(
            if raw {
                0
            } else {
                self.snapshot_payload_bytes(cx)
            },
            self.retained.clone(),
        ) {
            Ok(lease) => lease,
            Err(error) => {
                self.message = Some(error.into());
                cx.notify();
                return;
            }
        };
        self.busy = true;
        let snapshot = if raw {
            WorkspaceSnapshot::default()
        } else {
            self.snapshot(cx)
        };
        let mixed = crate::persistence::needs_workspace_export(&snapshot);
        let picker = cx.prompt_for_new_path(
            std::path::Path::new("/tmp"),
            Some(if raw || mixed {
                "workspace.json"
            } else {
                "queries.sql"
            }),
        );
        let runtime = self.host.runtime.clone();
        let backend = self.host.backend.clone();
        let host = self.host.clone();
        // Keep the snapshot reservation alive even if the workspace view closes;
        // Host owns and joins the blocking file job independently of this waiter.
        cx.spawn_in(window, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(path))) => {
                    if raw {
                        let loaded = runtime
                            .spawn(async move { backend.export_development_workspace().await })
                            .await;
                        match loaded {
                            Ok(Ok(Some(raw))) => match host.files.start(
                                &runtime,
                                Default::default(),
                                move |cancel| {
                                    if cancel.is_cancelled() {
                                        return Err("Export cancelled before file creation".into());
                                    }
                                    use std::io::Write;
                                    let mut options = std::fs::OpenOptions::new();
                                    options.write(true).create_new(true);
                                    #[cfg(unix)]
                                    {
                                        use std::os::unix::fs::OpenOptionsExt;
                                        options.mode(0o600);
                                    }
                                    let mut file = options
                                        .open(path)
                                        .map_err(|_| "Choose a new file for export".to_string())?;
                                    file.write_all(raw.as_bytes())
                                        .and_then(|_| file.sync_all())
                                        .map_err(|_| "Export failed".to_string())
                                },
                            ) {
                                Ok(job) => job.finish().await.map(|_| ()),
                                Err(error) => Err(error.into()),
                            },
                            _ => Err("Saved workspace could not be exported".into()),
                        }
                    } else {
                        match host
                            .files
                            .start(&runtime, Default::default(), move |cancel| {
                                if cancel.is_cancelled() {
                                    return Err("Export cancelled before file creation".into());
                                }
                                if mixed {
                                    crate::persistence::export_workspace(&snapshot, &path)
                                } else {
                                    crate::persistence::export_sql(&snapshot, &path)
                                }
                            }) {
                            Ok(job) => job.finish().await.map(|_| ()),
                            Err(error) => Err(error.into()),
                        }
                    }
                }
                Ok(Ok(None)) => Ok(()),
                _ => Err("Export picker failed".into()),
            };
            drop(lease);
            this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.message = result.err();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show_managed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view =
            cx.new(|cx| managed_view::ManagedServersView::new(self.host.clone(), window, cx));
        self._managed_events = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &managed_view::ManagedEvent, window, cx| {
                let managed_view::ManagedEvent::Closed { changed, settle } = event;
                this.managed = None;
                this._managed_events = None;
                if let Some(focus) = &this.previous_focus {
                    window.focus(focus, cx);
                } else {
                    this.focus_active(window, cx);
                }
                // Stopped or destroyed servers disconnect their documents.
                match settle.as_slice() {
                    [] if *changed => this.reload(false, window, cx),
                    [] => {}
                    [id] => {
                        this.form_scope = Some(id.clone());
                        this.settle_form_change(window, cx);
                    }
                    _ => {
                        this.form_credentials = true;
                        this.settle_form_change(window, cx);
                    }
                }
                cx.notify();
            },
        ));
        view.update(cx, |view, cx| view.focus(window, cx));
        self.managed = Some(view);
        cx.notify();
    }

    fn settle_form_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scope = self.form_scope.clone();
        let all = self.form_credentials;
        if all || scope.is_some() {
            self.retire_engines(if all { None } else { scope.as_deref() }, cx);
        }
        self.invalidate_clickhouse(scope.as_deref(), all, cx);
        // An edited endpoint keeps its ID; the retained tree may now describe
        // a different database.
        self.navigator.update(cx, |view, cx| {
            view.mark_stale(if all { None } else { scope.as_deref() }, cx)
        });
        if !all && scope.is_none() {
            self.reload(false, window, cx);
            return;
        }
        self.busy = true;
        for document in &self.documents {
            if all || document.metadata.connection_id == scope {
                document
                    .view
                    .update(cx, |view, cx| view.set_editable(false, cx));
            }
        }
        let host = self.host.clone();
        let target = scope.clone();
        let task = self.host.runtime.spawn(async move {
            if let Some(id) = target {
                host.disconnect_connection_documents(&id).await
            } else {
                host.disconnect_all_documents().await
            }
        });
        self.action_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                for document in &this.documents {
                    if all || document.metadata.connection_id == scope {
                        document.view.update(cx, |view, cx| {
                            view.set_editable(true, cx);
                            view.mark_disconnected(cx);
                        });
                    }
                }
                match result {
                    Ok(Ok(())) => this.reload(false, window, cx),
                    _ => {
                        this.message = Some(
                            "Connection cleanup failed; restart this isolated workspace".into(),
                        )
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn reset_saved_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        let backend = self.host.backend.clone();
        let task = self
            .host
            .runtime
            .spawn(async move { backend.reset_development_workspace(true).await });
        self.action_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Ok(_)) => {
                        this.load_error = None;
                        this.message = None;
                        this.reload(false, window, cx);
                    }
                    _ => this.message = Some("Saved workspace reset failed; retry recovery".into()),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn toggle_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() || self.palette.is_some() || self.managed.is_some() {
            return;
        }
        let open = !self.dock.read(cx).is_open();
        self.dock
            .update(cx, |dock, cx| dock.set_open(open, window, cx));
        if !open {
            self.focus_active(window, cx);
        }
        cx.notify();
    }
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_close(false, window, cx);
    }
    fn finish_close(&mut self, discard: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || self.busy {
            return;
        }
        // Failed restore creates no writer or editable documents. Normal quit
        // therefore preserves the original record for a later recovery attempt.
        if !discard && !self.cleanup_failed {
            self.changed(cx);
        }
        self.closing = true;
        self.retire_engines(None, cx);
        self.close_clickhouse(cx);
        self.navigator
            .update(cx, |view, cx| view.set_editable(false, cx));
        for document in &self.documents {
            document
                .view
                .update(cx, |view, cx| view.set_editable(false, cx));
        }
        let writer = if self.cleanup_failed {
            None
        } else {
            self.writer.clone()
        };
        let host = self.host.clone();
        let sqlite_sessions = self.sqlite_take_sessions(cx);
        // Fullscreen keeps the previous record. GPUI reports the frame origin
        // display-locally but sizes new windows by content area.
        let geometry = match window.window_bounds() {
            gpui::WindowBounds::Windowed(bounds) => {
                let content = window.viewport_size();
                Some(dbunk_lib::backend::WindowGeometry {
                    x: f32::from(bounds.origin.x),
                    y: f32::from(bounds.origin.y),
                    width: f32::from(content.width),
                    height: f32::from(content.height),
                    maximized: false,
                    display: window
                        .display(cx)
                        .and_then(|display| display.uuid().ok())
                        .map(|uuid| uuid.to_string()),
                })
            }
            gpui::WindowBounds::Maximized(_) | gpui::WindowBounds::Fullscreen(_) => None,
        };
        let task = self.host.runtime.spawn(async move {
            if let Some(writer) = writer {
                if discard {
                    writer
                        .discard_and_shutdown()
                        .await
                        .map_err(|error| (error.to_string(), false))?;
                } else {
                    writer
                        .flush()
                        .await
                        .map_err(|error| (error.to_string(), true))?;
                    writer
                        .shutdown()
                        .await
                        .map_err(|error| (error.to_string(), false))?;
                }
            }
            // Advisory and bounded: drafts are flushed first, and a slow or
            // failed geometry write never blocks the joined shutdown.
            if let Some(geometry) = geometry {
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    host.backend.set_window_geometry(geometry),
                )
                .await;
            }
            // A session that misses its deadline is aborted; quit continues.
            if let Err(error) = sqlite_integration::close_sqlite_sessions(sqlite_sessions).await {
                log::warn!("{error}");
            }
            host.shutdown().await.map_err(|error| (error, false))
        });
        self.action_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, _, cx| match result {
                Ok(Ok(())) => cx.quit(),
                result => {
                    this.closing = false;
                    let (error, resumable) = match result {
                        Ok(Err(error)) => error,
                        _ => ("Workspace cleanup failed".into(), false),
                    };
                    this.message = Some(error);
                    this.cleanup_failed = !resumable;
                    this.navigator
                        .update(cx, |view, cx| view.set_editable(resumable, cx));
                    for document in &this.documents {
                        document
                            .view
                            .update(cx, |view, cx| view.set_editable(resumable, cx));
                    }
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Only a saved, supported PostgreSQL connection can bind the tree.
        let navigator_connection = self.selected_connection.clone().filter(|id| {
            self.connections.iter().any(|connection| {
                &connection.id == id
                    && connection.postgres.is_some()
                    && connection.unsupported_reason.is_none()
            })
        });
        self.navigator
            .update(cx, |view, cx| view.set_connection(navigator_connection, cx));
        self.sync_clickhouse_tree(cx);
        let projects = shell::projects(&self.connections);
        if self
            .shell
            .project
            .as_ref()
            .is_none_or(|project| !projects.contains(project))
        {
            self.shell.project = projects.first().cloned();
        }
        self.render_shell(window, cx)
            .key_context("NativeWorkspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenTable, window, cx| {
                this.activate(Operation::OpenTable, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NewTab, window, cx| {
                    this.activate(Operation::New, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.activate(Operation::Close, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                this.activate(Operation::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| {
                this.activate(Operation::Previous, window, cx)
            }))
            .on_action(cx.listener(|this, _: &RenameTab, window, cx| {
                this.activate(Operation::Rename, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PinTab, window, cx| {
                    this.activate(Operation::Pin, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &MoveTabLeft, window, cx| {
                this.activate(Operation::Move(true), window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabRight, window, cx| {
                this.activate(Operation::Move(false), window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &OpenAnything, window, cx| this.open_palette(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleConsole, window, cx| this.toggle_console(window, cx)),
            )
            .on_action(cx.listener(|this, _: &WidenNavigator, window, cx| {
                this.activate(Operation::Width(24.), window, cx)
            }))
            .on_action(cx.listener(|this, _: &NarrowNavigator, window, cx| {
                this.activate(Operation::Width(-24.), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.activate(Operation::ToggleSidebar, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleStatusBar, window, cx| {
                this.activate(Operation::ToggleStatusBar, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowAllEnvironments, window, cx| {
                this.activate(Operation::EnvFilter(None), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowDevelopment, window, cx| {
                this.activate(
                    Operation::EnvFilter(Some(
                        dbunk_lib::backend::DevelopmentEnvironment::Development,
                    )),
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ShowTest, window, cx| {
                this.activate(
                    Operation::EnvFilter(Some(dbunk_lib::backend::DevelopmentEnvironment::Test)),
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ShowStaging, window, cx| {
                this.activate(
                    Operation::EnvFilter(Some(dbunk_lib::backend::DevelopmentEnvironment::Staging)),
                    window,
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ShowProduction, window, cx| {
                this.activate(
                    Operation::EnvFilter(Some(
                        dbunk_lib::backend::DevelopmentEnvironment::Production,
                    )),
                    window,
                    cx,
                )
            }))
            .on_action(|_: &MinimizeWindow, window, _| window.minimize_window())
            .on_action(|_: &ZoomWindow, window, _| window.zoom_window())
            .on_action(|_: &ToggleFullScreen, window, _| window.toggle_fullscreen())
            .on_action(cx.listener(|this, _: &FocusNavigator, window, cx| {
                if this.dialog.is_none() {
                    this.navigator
                        .update(cx, |view, cx| view.focus_filter(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &NewConnection, window, cx| {
                this.activate(Operation::NewConnection, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CredentialSettings, window, cx| {
                this.activate(Operation::Credentials, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Disconnect, window, cx| {
                this.activate(Operation::Disconnect, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearResults, window, cx| {
                this.activate(Operation::Clear, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::workbench::Quit, window, cx| this.close(window, cx)),
            )
            .when_some(self.dialog.clone(), |root, dialog| {
                // Occlude: clicks must not reach the workspace behind a page.
                root.child(
                    div()
                        .id("dialog-layer")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(crate::style::bg())
                        .child(dialog),
                )
            })
            .when_some(self.palette.clone(), |root, palette| {
                root.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .justify_center()
                        .pt(px(64.))
                        .child(crate::ui::appear("palette", div().child(palette))),
                )
            })
            .when_some(self.managed.clone(), |root, managed| {
                root.child(
                    div()
                        .id("managed-layer")
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(crate::style::bg())
                        .child(crate::ui::appear(
                            "managed-page",
                            div().size_full().child(managed),
                        )),
                )
            })
    }
}
