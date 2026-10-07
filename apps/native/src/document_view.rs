//! Workspace document dispatch. SQL and table views keep their own models while
//! exposing the same focus, draining and persistence boundary to the shell.
use crate::{
    admin_view::{AdminEvent, AdminView},
    catalog_view::{CatalogEvent, CatalogView},
    controller::Host,
    csv_transfer_view::{CsvTransferView, CsvViewResources},
    pg_tool_view::{PgToolView, ToolViewResources},
    query_library_view::{LibraryEvent, LibraryView, OpenQuery},
    schema_compare_view::{CompareViewResources, SchemaCompareView},
    schema_map_view::{SchemaMapEvent, SchemaMapView},
    table_view::{TableEvent, TableView},
    workbench::{Workbench, WorkbenchEvent},
};
use dbunk_lib::backend::{Layout, WorkspaceDocument, WorkspaceSelection, WorkspaceTableState};
use gpui::{App, Context, Entity, EventEmitter, Subscription, Window, prelude::*};
use std::{cell::Cell, rc::Rc, sync::Arc};

pub enum DocumentEvent {
    DatabaseChanged(String),
    OpenStructure {
        connection: String,
        schema: String,
        table: String,
    },
    OpenTableSeed {
        connection: String,
        schema: String,
        table: String,
    },
    OpenTableCopy {
        connection: String,
        schema: String,
        table: String,
    },
    CachedSchemasChanged,
    OpenCsvTransfer {
        connection: String,
        direction: dbunk_lib::backend::csv_transfers::CsvDirection,
        target: dbunk_lib::backend::csv_transfers::CsvTarget,
    },
    OpenWholeTableCsv {
        connection: String,
        target: dbunk_lib::backend::csv_transfers::CsvTarget,
        null_token: String,
    },
    OpenPgTools {
        connection: String,
        operation: crate::pg_tool_jobs::Operation,
        context: Option<(String, String)>,
    },
    DraftChanged,
    LayoutChanged(Layout),
    Quit,
    PersistApply(u64),
    OpenLibraryQuery(OpenQuery),
    Console(crate::console_model::Entry),
    EditConnection(String),
    OpenTable {
        connection: String,
        schema: String,
        table: String,
        filters: Vec<dbunk_lib::backend::data::BrowseFilter>,
    },
}

enum Content {
    Query(Entity<Workbench>),
    Table(Entity<TableView>),
    Library(Entity<LibraryView>),
    Catalog(Entity<CatalogView>),
    Admin(Entity<AdminView>),
    SchemaMap(Entity<SchemaMapView>),
    PgTools(Entity<PgToolView>),
    CsvTransfer(Entity<CsvTransferView>),
    SchemaCompare(Entity<SchemaCompareView>),
    TableCopy(Entity<crate::table_copy_view::TableCopyView>),
    TableSeed(Entity<crate::table_seed_view::TableSeedView>),
}
pub struct DocumentResources {
    pub host: Arc<Host>,
    pub wake: async_channel::Sender<()>,
    pub retained: Rc<Cell<usize>>,
    pub pg_tools: Entity<crate::pg_tool_store::ToolStore>,
    pub csv_transfers: Entity<crate::csv_transfer_store::CsvStore>,
    pub comparisons: Entity<crate::schema_compare_store::CompareStore>,
    pub copies: Entity<crate::table_copy_store::CopyStore>,
    pub seeds: Entity<crate::table_seed_store::SeedStore>,
}
/// A document's own session state, as shown on its connection's sidebar row.
#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionPhase {
    Idle,
    Connecting,
    Connected,
    Failed(String),
}
/// One tab in the shell's tab bar: a workspace document or an engine tab.
pub struct TabInfo {
    pub id: String,
    pub title: String,
    pub icon: &'static str,
    pub status: String,
    pub active: bool,
    pub pinned: bool,
    pub closable: bool,
}
pub struct DocumentView {
    content: Content,
    _events: Option<Subscription>,
    _status: Subscription,
}
impl EventEmitter<DocumentEvent> for DocumentView {}
impl DocumentView {
    pub fn new(
        resources: DocumentResources,
        document: (&mut WorkspaceDocument, bool),
        layout: Layout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let DocumentResources {
            host,
            wake,
            retained,
            pg_tools,
            csv_transfers,
            comparisons,
            copies,
            seeds,
        } = resources;
        let (document, restored) = document;
        let (content, events, status) = if document.tool
            == Some(dbunk_lib::backend::WorkspaceTool::BackupRestore)
        {
            let view = cx.new(|cx| {
                PgToolView::new(
                    ToolViewResources {
                        host,
                        store: pg_tools,
                        wake,
                        retained,
                    },
                    document,
                    window,
                    cx,
                )
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::PgTools(view), None, status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::TableCopy) {
            let view = cx.new(|cx| {
                crate::table_copy_view::TableCopyView::new(copies, retained, document, window, cx)
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::TableCopy(view), None, status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::TableSeed) {
            let view = cx.new(|cx| {
                crate::table_seed_view::TableSeedView::new(seeds, retained, document, window, cx)
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::TableSeed(view), None, status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::CsvTransfer) {
            let view = cx.new(|cx| {
                CsvTransferView::new(
                    CsvViewResources {
                        host,
                        store: csv_transfers,
                        wake,
                        retained,
                    },
                    document,
                    window,
                    cx,
                )
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::CsvTransfer(view), None, status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::SchemaCompare) {
            let reader = cx.new(|_| {
                crate::schema_compare_reader::CompareReader::new(
                    host,
                    document.id.clone(),
                    wake,
                    retained.clone(),
                )
            });
            let view = cx.new(|cx| {
                SchemaCompareView::new(
                    CompareViewResources {
                        store: comparisons,
                        reader,
                        retained,
                    },
                    document,
                    window,
                    cx,
                )
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::SchemaCompare(view), None, status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::SchemaMap) {
            let view = cx.new(|cx| SchemaMapView::new(host, document, wake, retained, window, cx));
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            let events = cx.subscribe(&view, |_, _, event, cx| match event {
                SchemaMapEvent::OpenTable {
                    connection,
                    schema,
                    table,
                    ..
                } => cx.emit(DocumentEvent::OpenTable {
                    connection: connection.clone(),
                    schema: schema.clone(),
                    table: table.clone(),
                    filters: Vec::new(),
                }),
            });
            (Content::SchemaMap(view), Some(events), status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::Administration) {
            let view = cx.new(|cx| AdminView::new(host, document, wake, retained, window, cx));
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            let events = cx.subscribe(&view, |_, _, event, cx| match event {
                AdminEvent::EditConnection(id) => {
                    cx.emit(DocumentEvent::EditConnection(id.clone()))
                }
                AdminEvent::Changed => cx.emit(DocumentEvent::DraftChanged),
                AdminEvent::PersistApply(id) => cx.emit(DocumentEvent::PersistApply(*id)),
                AdminEvent::OpenQuery(query) => {
                    cx.emit(DocumentEvent::OpenLibraryQuery(OpenQuery {
                        sql: query.sql.clone(),
                        name: query.name.clone(),
                        connection: query.connection.clone(),
                        saved_id: query.saved_id.clone(),
                    }))
                }
            });
            (Content::Admin(view), Some(events), status)
        } else if document.tool == Some(dbunk_lib::backend::WorkspaceTool::Objects) {
            let view = cx.new(|cx| CatalogView::new(host, document, wake, retained, window, cx));
            let events = cx.subscribe(&view, |_, _, event, cx| match event {
                CatalogEvent::DatabaseChanged(connection) => {
                    cx.emit(DocumentEvent::DatabaseChanged(connection.clone()))
                }
                CatalogEvent::SchemasChanged => cx.emit(DocumentEvent::CachedSchemasChanged),
                CatalogEvent::Changed => cx.emit(DocumentEvent::DraftChanged),
                CatalogEvent::PersistApply(id) => cx.emit(DocumentEvent::PersistApply(*id)),
                CatalogEvent::OpenTable {
                    connection,
                    schema,
                    table,
                } => cx.emit(DocumentEvent::OpenTable {
                    connection: connection.clone(),
                    schema: schema.clone(),
                    table: table.clone(),
                    filters: Vec::new(),
                }),
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::Catalog(view), Some(events), status)
        } else if document.tool.is_some() {
            let view = cx.new(|cx| LibraryView::new(host, document, wake, retained, window, cx));
            let events = cx.subscribe(&view, |_, _, event, cx| match event {
                LibraryEvent::Open(query) => cx.emit(DocumentEvent::OpenLibraryQuery(OpenQuery {
                    sql: query.sql.clone(),
                    name: query.name.clone(),
                    connection: query.connection.clone(),
                    saved_id: query.saved_id.clone(),
                })),
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::Library(view), Some(events), status)
        } else if document.table.is_some() {
            let view =
                cx.new(|cx| TableView::new(host, document, restored, wake, retained, window, cx));
            let events = cx.subscribe(&view, |_, _, event, cx| {
                cx.emit(match event {
                    TableEvent::OpenReference {
                        connection,
                        schema,
                        table,
                        filters,
                    } => DocumentEvent::OpenTable {
                        connection: connection.clone(),
                        schema: schema.clone(),
                        table: table.clone(),
                        filters: filters.clone(),
                    },
                    TableEvent::OpenTableCopy {
                        connection,
                        schema,
                        table,
                    } => DocumentEvent::OpenTableCopy {
                        connection: connection.clone(),
                        schema: schema.clone(),
                        table: table.clone(),
                    },
                    TableEvent::OpenStructure {
                        connection,
                        schema,
                        table,
                    } => DocumentEvent::OpenStructure {
                        connection: connection.clone(),
                        schema: schema.clone(),
                        table: table.clone(),
                    },
                    TableEvent::OpenTableSeed {
                        connection,
                        schema,
                        table,
                    } => DocumentEvent::OpenTableSeed {
                        connection: connection.clone(),
                        schema: schema.clone(),
                        table: table.clone(),
                    },
                    TableEvent::OpenCsvTransfer {
                        connection,
                        direction,
                        target,
                    } => DocumentEvent::OpenCsvTransfer {
                        connection: connection.clone(),
                        direction: *direction,
                        target: target.clone(),
                    },
                    TableEvent::OpenWholeTableCsv {
                        connection,
                        target,
                        null_token,
                    } => DocumentEvent::OpenWholeTableCsv {
                        connection: connection.clone(),
                        target: target.clone(),
                        null_token: null_token.clone(),
                    },
                    TableEvent::OpenPgTools {
                        connection,
                        operation,
                        context,
                    } => DocumentEvent::OpenPgTools {
                        connection: connection.clone(),
                        operation: *operation,
                        context: context.clone(),
                    },
                    TableEvent::Changed => DocumentEvent::DraftChanged,
                    TableEvent::PersistApply(request) => DocumentEvent::PersistApply(*request),
                });
            });
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::Table(view), Some(events), status)
        } else {
            let view = cx.new(|cx| {
                Workbench::new_document(host, document, layout, wake, retained, window, cx)
            });
            let events = cx.subscribe(&view, |_, _, event, cx| relay(event, cx));
            let status = cx.observe(&view, |_, _, cx| cx.notify());
            (Content::Query(view), Some(events), status)
        };
        Self {
            content,
            _events: events,
            _status: status,
        }
    }
    pub fn csv_context(
        &mut self,
        direction: dbunk_lib::backend::csv_transfers::CsvDirection,
        target: dbunk_lib::backend::csv_transfers::CsvTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Content::CsvTransfer(view) = &self.content {
            view.update(cx, |view, cx| {
                view.set_context(direction, target, window, cx)
            });
        }
    }
    pub fn whole_table_csv_context(
        &mut self,
        target: dbunk_lib::backend::csv_transfers::CsvTarget,
        null_token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Content::CsvTransfer(view) = &self.content {
            view.update(cx, |view, cx| {
                view.set_export_context(target, null_token, window, cx)
            });
        }
    }
    pub fn pg_tool_context(
        &mut self,
        operation: crate::pg_tool_jobs::Operation,
        context: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        if let Content::PgTools(view) = &self.content {
            view.update(cx, |view, cx| view.set_context(operation, context, cx));
        }
    }
    pub fn draft(&self, cx: &mut App) -> (String, WorkspaceSelection) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.draft(cx)),
            Content::Table(_)
            | Content::Library(_)
            | Content::Catalog(_)
            | Content::Admin(_)
            | Content::SchemaMap(_)
            | Content::PgTools(_)
            | Content::SchemaCompare(_)
            | Content::TableCopy(_)
            | Content::TableSeed(_)
            | Content::CsvTransfer(_) => (String::new(), WorkspaceSelection::default()),
        }
    }
    pub fn snapshot_payload_bytes(&self, cx: &App) -> usize {
        match &self.content {
            Content::Query(view) => view
                .read(cx)
                .draft_bytes(cx)
                .saturating_add(view.read(cx).query_changes_bytes(cx)),
            Content::Table(view) => view.read(cx).snapshot_bytes(cx),
            Content::Catalog(view) => view
                .read(cx)
                .schema_bytes(cx)
                .saturating_add(view.read(cx).maintenance_bytes(cx))
                .saturating_add(view.read(cx).table_ddl_bytes(cx))
                .saturating_add(view.read(cx).schema_alter_bytes(cx))
                .saturating_add(view.read(cx).object_ddl_bytes(cx)),
            Content::Admin(view) => view.read(cx).control_bytes(),
            Content::SchemaMap(_) => 0,
            Content::Library(_)
            | Content::PgTools(_)
            | Content::SchemaCompare(_)
            | Content::TableCopy(_)
            | Content::TableSeed(_)
            | Content::CsvTransfer(_) => 0,
        }
    }
    pub fn query_changes(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceQueryChanges> {
        match &self.content {
            Content::Query(view) => view.read(cx).query_changes_snapshot(cx),
            _ => None,
        }
    }
    pub fn schema_changes(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceSchemaChanges> {
        match &self.content {
            Content::Catalog(view) => view.read(cx).schema_snapshot(cx),
            _ => None,
        }
    }
    pub fn table_ddl(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceTableDdl> {
        match &self.content {
            Content::Catalog(view) => view.read(cx).table_ddl_snapshot(cx),
            _ => None,
        }
    }
    pub fn schema_alter(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceSchemaAlter> {
        match &self.content {
            Content::Catalog(view) => view.read(cx).schema_alter_snapshot(cx),
            _ => None,
        }
    }
    pub fn object_ddl(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceObjectDdl> {
        match &self.content {
            Content::Catalog(view) => view.read(cx).object_ddl_snapshot(cx),
            _ => None,
        }
    }
    pub fn maintenance(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceMaintenance> {
        match &self.content {
            Content::Catalog(view) => view.read(cx).maintenance_snapshot(cx),
            _ => None,
        }
    }
    pub fn admin_control(&self, cx: &App) -> Option<dbunk_lib::backend::WorkspaceAdminControl> {
        match &self.content {
            Content::Admin(view) => view.read(cx).control_snapshot(),
            _ => None,
        }
    }
    pub fn has_recoverable_changes(&self, cx: &App) -> bool {
        match &self.content {
            Content::Query(view) => view.read(cx).query_changes_blocked(cx),
            Content::Catalog(view) => {
                view.read(cx).has_schema_changes(cx)
                    || view.read(cx).has_maintenance_changes(cx)
                    || view.read(cx).has_table_ddl_changes(cx)
                    || view.read(cx).has_schema_alter_changes(cx)
                    || view.read(cx).has_object_ddl_changes(cx)
            }
            Content::Admin(view) => view.read(cx).has_control_recovery(),
            _ => false,
        }
    }
    pub fn table_state(&self, cx: &App) -> Option<WorkspaceTableState> {
        match &self.content {
            Content::Query(_)
            | Content::Library(_)
            | Content::Catalog(_)
            | Content::Admin(_)
            | Content::SchemaMap(_)
            | Content::PgTools(_)
            | Content::SchemaCompare(_)
            | Content::TableCopy(_)
            | Content::TableSeed(_)
            | Content::CsvTransfer(_) => None,
            Content::Table(view) => Some(view.read(cx).snapshot(cx)),
        }
    }
    /// Only query sessions report a phase; table-lane documents surface
    /// their state in the document and through the host's connected set.
    pub fn connection_phase(&self, cx: &App) -> ConnectionPhase {
        match &self.content {
            Content::Query(view) => view.read(cx).connection_phase(),
            _ => ConnectionPhase::Idle,
        }
    }
    pub fn is_query(&self) -> bool {
        matches!(self.content, Content::Query(_))
    }
    pub fn document_status<'a>(&'a self, cx: &'a App) -> &'a str {
        match &self.content {
            Content::Query(view) => view.read(cx).document_status(),
            Content::Table(view) => view.read(cx).document_status(),
            Content::Library(view) => view.read(cx).status(),
            Content::Catalog(view) => view.read(cx).status(),
            Content::Admin(view) => view.read(cx).status(),
            Content::SchemaMap(view) => view.read(cx).status(),
            Content::CsvTransfer(view) => view.read(cx).status(),
            Content::SchemaCompare(view) => view.read(cx).status(),
            Content::PgTools(view) => view.read(cx).status(),
            Content::TableCopy(view) => view.read(cx).status(),
            Content::TableSeed(view) => view.read(cx).status(),
        }
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::Library(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::CsvTransfer(view) => {
                view.update(cx, |view, cx| view.focus_document(window, cx))
            }
            Content::PgTools(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::TableCopy(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::TableSeed(view) => view.update(cx, |view, cx| view.focus_document(window, cx)),
            Content::SchemaCompare(view) => {
                view.update(cx, |view, cx| view.focus_document(window, cx))
            }
        }
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::Library(_) => {}
            Content::Catalog(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::CsvTransfer(view) => {
                view.update(cx, |view, cx| view.remember_focus(window, cx))
            }
            Content::PgTools(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::TableCopy(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::TableSeed(view) => view.update(cx, |view, cx| view.remember_focus(window, cx)),
            Content::SchemaCompare(view) => {
                view.update(cx, |view, cx| view.remember_focus(window, cx))
            }
        }
    }
    pub fn set_document_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if let Content::Query(view) = &self.content {
            view.update(cx, |view, cx| view.set_document_layout(layout, cx));
        }
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::Library(view) => view.update(cx, |view, _| view.set_editable(editable)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::CsvTransfer(view) => {
                view.update(cx, |view, cx| view.set_editable(editable, cx))
            }
            Content::PgTools(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::TableCopy(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::TableSeed(view) => view.update(cx, |view, cx| view.set_editable(editable, cx)),
            Content::SchemaCompare(view) => {
                view.update(cx, |view, cx| view.set_editable(editable, cx))
            }
        }
    }
    pub fn bind_connection(&mut self, id: String, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::CsvTransfer(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::PgTools(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::TableCopy(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::TableSeed(view) => view.update(cx, |view, cx| view.bind_connection(id, cx)),
            Content::SchemaCompare(view) => {
                view.update(cx, |view, cx| view.bind_connection(id, cx))
            }
            Content::Library(_) => {}
        }
    }
    pub fn begin_connect(&mut self, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.begin_connect(cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.begin_connect(cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.begin_connect(cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.begin_connect(cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.begin_connect(cx)),
            Content::Library(_)
            | Content::PgTools(_)
            | Content::SchemaCompare(_)
            | Content::TableCopy(_)
            | Content::TableSeed(_)
            | Content::CsvTransfer(_) => {}
        }
    }
    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.mark_disconnected(cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.mark_disconnected(cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.mark_disconnected(cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.mark_disconnected(cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.mark_disconnected(cx)),
            Content::Library(_) | Content::TableCopy(_) | Content::TableSeed(_) => {}
            Content::SchemaCompare(view) => view.update(cx, |view, cx| view.clear(cx)),
            Content::CsvTransfer(view) => {
                view.update(cx, |view, cx| view.invalidate_after_restore(cx))
            }
            Content::PgTools(view) => view.update(cx, |view, cx| view.invalidate_after_restore(cx)),
        }
    }
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.invalidate_after_restore(cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.invalidate_after_restore(cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.invalidate_after_restore(cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.invalidate_after_restore(cx)),
            Content::SchemaMap(view) => {
                view.update(cx, |view, cx| view.invalidate_after_restore(cx))
            }
            Content::Library(_)
            | Content::SchemaCompare(_)
            | Content::TableCopy(_)
            | Content::TableSeed(_) => {}
            Content::CsvTransfer(view) => {
                view.update(cx, |view, cx| view.invalidate_after_restore(cx))
            }
            Content::PgTools(view) => view.update(cx, |view, cx| view.invalidate_after_restore(cx)),
        }
    }
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.clear_results(cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.clear_results(cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.clear_results(cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.clear_results(cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.clear_results(cx)),
            Content::PgTools(_)
            | Content::SchemaCompare(_)
            | Content::TableCopy(_)
            | Content::TableSeed(_)
            | Content::CsvTransfer(_) => {}
            Content::Library(_) => {}
        }
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        match &self.content {
            Content::Query(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::Table(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::Library(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::Catalog(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::Admin(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::SchemaMap(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::CsvTransfer(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::PgTools(view) => view.update(cx, |view, cx| view.drain_one(cx)),
            Content::TableCopy(_) => false,
            Content::TableSeed(_) => false,
            Content::SchemaCompare(view) => view.update(cx, |view, cx| view.drain_one(cx)),
        }
    }
    pub fn visit_cached_schemas(&self, connection: &str, cx: &App, visit: &mut dyn FnMut(&str)) {
        if let Content::Catalog(view) = &self.content {
            view.read(cx).visit_cached_schemas(connection, visit);
        }
    }
    pub fn set_comparison_connections(
        &mut self,
        connections: &[dbunk_lib::backend::DevelopmentConnection],
        catalogs: &[Entity<DocumentView>],
        cx: &mut Context<Self>,
    ) {
        let Content::SchemaCompare(view) = &self.content else {
            return;
        };
        // Preflight borrowed strings before cloning any potentially large
        // catalog snapshot. Duplicate cached names count conservatively here.
        let snapshot = (|| {
            if connections.len() > 1024 {
                return Err("Comparison choices exceed 1,024 connections");
            }
            let mut bytes = connections
                .len()
                .checked_mul(std::mem::size_of::<
                    crate::schema_compare_model::ConnectionChoice,
                >())
                .ok_or("Comparison choice size overflow")?;
            for connection in connections {
                let Some(postgres) = &connection.postgres else {
                    continue;
                };
                for text in [&connection.id, &connection.name, &postgres.database] {
                    bytes = bytes
                        .checked_add(text.len())
                        .ok_or("Comparison choice size overflow")?;
                }
                bytes = bytes
                    .checked_add(64)
                    .ok_or("Comparison choice size overflow")?;
                let mut count = 0usize;
                let mut invalid = false;
                for catalog in catalogs {
                    catalog
                        .read(cx)
                        .visit_cached_schemas(&connection.id, cx, &mut |name| {
                            count = count.saturating_add(1);
                            invalid |= name.is_empty() || name.len() > 63 || name.contains('\0');
                            bytes = bytes
                                .saturating_add(std::mem::size_of::<String>())
                                .saturating_add(name.len());
                        });
                }
                if invalid || count > 512 || bytes > 1024 * 1024 {
                    return Err(
                        "Cached schema choices exceed their bounded allowance; previous choices retained",
                    );
                }
            }
            // Account the temporary snapshot until the model has admitted its
            // retained copy. The model separately checks actual capacities.
            let lease = crate::schema_compare_model::Lease::new(
                view.read(cx).retained_budget(),
                1024 * 1024,
            )?;
            let mut choices = Vec::with_capacity(connections.len());
            for connection in connections {
                let Some(postgres) = &connection.postgres else {
                    continue;
                };
                let mut count = 0;
                for catalog in catalogs {
                    catalog
                        .read(cx)
                        .visit_cached_schemas(&connection.id, cx, &mut |_| count += 1);
                }
                let mut schemas = Vec::with_capacity(count);
                for catalog in catalogs {
                    catalog
                        .read(cx)
                        .visit_cached_schemas(&connection.id, cx, &mut |name| {
                            schemas.push(name.to_owned())
                        });
                }
                schemas.sort_unstable();
                schemas.dedup();
                choices.push(crate::schema_compare_model::ConnectionChoice {
                    id: connection.id.clone(),
                    name: connection.name.clone(),
                    database: postgres.database.clone(),
                    environment: format!("{:?}", postgres.environment),
                    schemas,
                });
            }
            Ok((choices, lease))
        })();
        match snapshot {
            Ok((choices, _lease)) => view.update(cx, |view, cx| view.set_connections(choices, cx)),
            Err(error) => view.update(cx, |view, cx| view.reject_connections(error, cx)),
        }
    }
    pub fn set_connection_health(&mut self, health: Option<String>, cx: &mut Context<Self>) {
        if let Content::Admin(view) = &self.content {
            view.update(cx, |view, cx| view.set_health(health, cx));
        }
    }
    pub fn set_connection_metadata(
        &mut self,
        connections: &[dbunk_lib::backend::DevelopmentConnection],
        cx: &mut Context<Self>,
    ) {
        match &self.content {
            Content::Admin(view) => {
                view.update(cx, |view, cx| view.set_connection_metadata(connections, cx))
            }
            // Plan 032 §3.2: the table tab takes environment, safe mode and
            // read-only from its own connection, not the sidebar selection.
            Content::Table(view) => {
                view.update(cx, |view, cx| view.set_connection_metadata(connections, cx))
            }
            // Query-result edits follow the same policy as table tabs.
            Content::Query(view) => {
                view.update(cx, |view, cx| view.set_connection_metadata(connections, cx))
            }
            _ => {}
        }
    }
    pub fn set_library_connections(
        &mut self,
        connections: &[dbunk_lib::backend::DevelopmentConnection],
        cx: &mut Context<Self>,
    ) {
        match &self.content {
            Content::Library(view) => view.update(cx, |view, _| {
                view.set_connections(
                    connections
                        .iter()
                        .map(|connection| (connection.id.clone(), connection.name.clone()))
                        .collect(),
                )
            }),
            Content::TableCopy(view) => {
                view.update(cx, |view, cx| view.set_connections(connections, cx))
            }
            Content::TableSeed(view) => {
                view.update(cx, |view, cx| view.set_connections(connections, cx))
            }
            _ => {}
        }
    }
    pub fn copy_context(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        if let Content::TableCopy(view) = &self.content {
            view.update(cx, |view, cx| view.set_source(schema, table, cx));
        }
    }
    pub fn structure_context(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        if let Content::Catalog(view) = &self.content {
            view.update(cx, |view, cx| view.structure_context(schema, table, cx));
        }
    }
    pub fn describe_context(
        &mut self,
        reference: dbunk_lib::backend::objects::PgObjectRef,
        cx: &mut Context<Self>,
    ) {
        if let Content::Catalog(view) = &self.content {
            view.update(cx, |view, cx| view.describe_context(reference, cx));
        }
    }
    pub fn seed_context(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        if let Content::TableSeed(view) = &self.content {
            view.update(cx, |view, cx| view.set_target(schema, table, cx));
        }
    }
    pub fn save_query(
        &mut self,
        query: dbunk_lib::backend::query_library::SavedQueryRecord,
        cx: &mut Context<Self>,
    ) {
        if let Content::Library(view) = &self.content {
            view.update(cx, |view, cx| view.save(query, cx));
        }
    }
    pub fn apply_saved(
        &mut self,
        request: u64,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        match &self.content {
            Content::Table(view) => {
                view.update(cx, |view, cx| view.apply_saved(request, result, cx))
            }
            Content::Query(view) => {
                view.update(cx, |view, cx| view.apply_saved(request, result, cx))
            }
            Content::Catalog(view) => {
                view.update(cx, |view, cx| view.apply_saved(request, result, cx))
            }
            Content::Admin(view) => {
                view.update(cx, |view, cx| view.apply_saved(request, result, cx))
            }
            _ => {}
        }
    }
    pub fn has_pending(&self, cx: &App) -> bool {
        match &self.content {
            Content::Query(view) => view.read(cx).has_pending(cx),
            Content::Table(view) => view.read(cx).has_pending(),
            Content::Library(view) => view.read(cx).has_pending(),
            Content::Catalog(view) => view.read(cx).has_pending(),
            Content::Admin(view) => view.read(cx).has_pending(),
            Content::SchemaMap(view) => view.read(cx).has_pending(),
            Content::CsvTransfer(view) => view.read(cx).has_pending(),
            Content::SchemaCompare(view) => view.read(cx).has_pending(cx),
            Content::PgTools(view) => view.read(cx).has_pending(),
            Content::TableCopy(_) => false,
            Content::TableSeed(_) => false,
        }
    }
}
fn relay(event: &WorkbenchEvent, cx: &mut Context<DocumentView>) {
    cx.emit(match event {
        WorkbenchEvent::OpenQuery(query) => DocumentEvent::OpenLibraryQuery(OpenQuery {
            sql: query.sql.clone(),
            name: query.name.clone(),
            connection: query.connection.clone(),
            saved_id: query.saved_id.clone(),
        }),
        WorkbenchEvent::PersistApply(request) => DocumentEvent::PersistApply(*request),
        WorkbenchEvent::DraftChanged => DocumentEvent::DraftChanged,
        WorkbenchEvent::LayoutChanged(layout) => DocumentEvent::LayoutChanged(*layout),
        WorkbenchEvent::Quit => DocumentEvent::Quit,
        WorkbenchEvent::Console(entry) => DocumentEvent::Console(entry.clone()),
    });
}
impl Render for DocumentView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.content {
            Content::Query(view) => view.clone().into_any_element(),
            Content::Table(view) => view.clone().into_any_element(),
            Content::Library(view) => view.clone().into_any_element(),
            Content::Catalog(view) => view.clone().into_any_element(),
            Content::Admin(view) => view.clone().into_any_element(),
            Content::SchemaMap(view) => view.clone().into_any_element(),
            Content::CsvTransfer(view) => view.clone().into_any_element(),
            Content::PgTools(view) => view.clone().into_any_element(),
            Content::TableCopy(view) => view.clone().into_any_element(),
            Content::TableSeed(view) => view.clone().into_any_element(),
            Content::SchemaCompare(view) => view.clone().into_any_element(),
        }
    }
}
