//! Connection-bound object Tool tab. Reads share the owned data worker; opening
//! a relation carries this document's connection, never navigator selection.
use crate::{
    accessible_editor::AccessibleEditor,
    catalog::Catalog,
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
};
use dbunk_lib::backend::WorkspaceDocument;
use editor::Editor;
use gpui::{
    ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, Role, SharedString, UniformListScrollHandle, Window, div, prelude::*, px, rgb,
    uniform_list,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

mod ddl_export;
mod maintenance;
mod schema;
mod structure;
mod table_ddl;

pub enum CatalogEvent {
    DatabaseChanged(String),
    SchemasChanged,
    Changed,
    PersistApply(u64),
    OpenTable {
        connection: String,
        schema: String,
        table: String,
    },
}
#[derive(Clone, Copy)]
enum Action {
    TableDdl,
    Schema,
    Maintenance,
    Connect,
    Refresh,
    Cancel,
    Search,
    Open,
    Describe,
    Structure,
    DdlExport,
    DropImpact,
    Copy,
    Clear,
}
pub struct CatalogView {
    table_ddl: Option<Entity<crate::table_ddl_view::TableDdlView>>,
    table_ddl_recovery: Option<dbunk_lib::backend::WorkspaceTableDdl>,
    table_ddl_events: Option<gpui::Subscription>,
    table_ddl_busy: bool,
    show_table_ddl: bool,
    ddl_export: ddl_export::DdlExport,
    structure: Option<Entity<crate::table_structure_view::StructureView>>,
    incoming_structure: Option<crate::table_structure_model::Capture>,
    structure_events: Option<gpui::Subscription>,
    show_structure: bool,
    structure_current: bool,
    structure_navigation: Option<u64>,
    structure_after_connect: Option<dbunk_lib::backend::table_structure::TableStructureRequest>,
    maintenance: Option<Entity<crate::maintenance_view::MaintenanceView>>,
    maintenance_recovery: Option<dbunk_lib::backend::WorkspaceMaintenance>,
    maintenance_events: Option<gpui::Subscription>,
    maintenance_busy: bool,
    show_maintenance: bool,
    schema: Option<Entity<crate::schema_view::SchemaView>>,
    schema_recovery: Option<dbunk_lib::backend::WorkspaceSchemaChanges>,
    schema_events: Option<gpui::Subscription>,
    schema_busy: bool,
    apply_ids: Rc<Cell<u64>>,
    show_schema: bool,
    host: Arc<Host>,
    id: String,
    connection: Option<String>,
    wake: async_channel::Sender<()>,
    controls: Option<TableControls>,
    receiver: Option<TableReceiver>,
    ready: bool,
    budget: Rc<Cell<usize>>,
    catalog: Option<Catalog>,
    schema_choices_current: bool,
    visible: Vec<usize>,
    selected: usize,
    applied_search: String,
    search: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    incoming_details: Option<crate::object_details::Details>,
    details: Option<Entity<crate::object_details::DetailsView>>,
    details_events: Option<gpui::Subscription>,
    show_details: bool,
    root: FocusHandle,
    list: FocusHandle,
    buttons: Vec<FocusHandle>,
    scroll: UniformListScrollHandle,
    previous_focus: Option<FocusHandle>,
    editable: bool,
    busy: bool,
    next: u64,
    pending: Option<u64>,
    cancellation_requested: bool,
    status: String,
    failure: Option<String>,
}
impl EventEmitter<CatalogEvent> for CatalogView {}
impl CatalogView {
    pub fn new(
        host: Arc<Host>,
        document: &mut WorkspaceDocument,
        wake: async_channel::Sender<()>,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| Editor::single_line(window, cx));
        let accessible = cx.new(|cx| {
            AccessibleEditor::field(
                search.clone(),
                "Search object names, schemas or signatures",
                false,
                cx,
            )
        });
        Self {
            table_ddl: None,
            table_ddl_recovery: document.table_ddl.take(),
            table_ddl_events: None,
            table_ddl_busy: false,
            show_table_ddl: false,
            ddl_export: ddl_export::DdlExport::default(),
            structure: None,
            incoming_structure: None,
            structure_events: None,
            show_structure: false,
            structure_current: false,
            structure_navigation: None,
            structure_after_connect: None,
            maintenance: None,
            maintenance_recovery: document.maintenance.take(),
            maintenance_events: None,
            maintenance_busy: false,
            show_maintenance: false,
            schema: None,
            schema_recovery: document.schema_changes.take(),
            schema_events: None,
            schema_busy: false,
            apply_ids: Rc::new(Cell::new(0)),
            show_schema: false,
            host,
            id: document.id.clone(),
            connection: document.connection_id.clone(),
            wake,
            controls: None,
            receiver: None,
            ready: false,
            budget,
            catalog: None,
            schema_choices_current: false,
            visible: vec![],
            selected: 0,
            applied_search: String::new(),
            search,
            accessible,
            incoming_details: None,
            details: None,
            details_events: None,
            show_details: false,
            root: cx.focus_handle(),
            list: cx.focus_handle(),
            buttons: (0..14).map(|_| cx.focus_handle()).collect(),
            scroll: UniformListScrollHandle::new(),
            previous_focus: None,
            editable: true,
            busy: false,
            next: 0,
            pending: None,
            cancellation_requested: false,
            status: "Disconnected".into(),
            failure: None,
        }
    }
    /// Suggestions reuse the admitted catalog only; this never starts a read.
    pub fn visit_cached_schemas(&self, connection: &str, visit: &mut dyn FnMut(&str)) {
        if !self.ready
            || !self.schema_choices_current
            || self.connection.as_deref() != Some(connection)
        {
            return;
        }
        if let Some(catalog) = &self.catalog {
            for row in &catalog.rows {
                if matches!(
                    row.kind,
                    crate::catalog::Kind::Object(dbunk_lib::backend::objects::PgObjectKind::Schema)
                ) {
                    visit(&row.entry.name);
                }
            }
        }
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn has_pending(&self) -> bool {
        self.busy || self.schema_busy || self.maintenance_busy || self.table_ddl_busy
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        self.search
            .update(cx, |editor, _| editor.set_read_only(!editable));
        self.sync_table_ddl(cx);
        cx.notify();
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_table_ddl
            && let Some(view) = &self.table_ddl
        {
            window.focus(&view.focus_handle(cx), cx);
        } else if self.ddl_export.show
            && let Some(view) = &self.ddl_export.view
        {
            window.focus(&view.read(cx).focus(cx), cx);
        } else if self.show_structure
            && let Some(view) = &self.structure
        {
            window.focus(&view.read(cx).focus(cx), cx);
        } else if self.show_maintenance
            && let Some(view) = &self.maintenance
        {
            window.focus(&view.focus_handle(cx), cx);
        } else if self.show_details
            && let Some(details) = &self.details
        {
            window.focus(&details.read(cx).focus(cx), cx);
        } else {
            window.focus(self.previous_focus.as_ref().unwrap_or(&self.list), cx);
        }
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.contains_focused(window, cx)
            || self
                .table_ddl
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
            || self
                .ddl_export
                .view
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
            || self
                .structure
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
            || self
                .details
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
        {
            self.previous_focus = window.focused(cx);
        }
    }
    pub fn bind_connection(&mut self, id: String, cx: &mut Context<Self>) {
        if self.connection.as_ref() == Some(&id) {
            return;
        }
        if self.connection.as_ref() != Some(&id)
            && (self.has_schema_changes(cx)
                || self.has_maintenance_changes(cx)
                || self.has_table_ddl_changes(cx))
        {
            self.status =
                "Finish or reconcile the retained Objects review before changing its connection"
                    .into();
            cx.notify();
            return;
        }
        if self.controls.is_none() {
            self.table_ddl = None;
            self.table_ddl_events = None;
            self.show_table_ddl = false;
            self.connection = Some(id);
            self.clear_results(cx);
        }
    }
    pub fn begin_connect(&mut self, cx: &mut Context<Self>) {
        if !self.editable || self.controls.is_some() {
            return;
        }
        let Some(connection) = &self.connection else {
            self.status = "Select a connection".into();
            cx.notify();
            return;
        };
        match self
            .host
            .open_table_document(self.id.clone(), connection.clone(), self.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.controls = Some(controls);
                self.receiver = Some(receiver);
                self.ready = false;
                self.busy = true;
                self.failure = None;
                self.status = "Connecting catalog".into();
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    /// A restore changed this connection. Stop the metadata/data lane and
    /// preserve staged intent; query sessions are owned by a different lane.
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = &self.controls {
            controls.stop();
            self.ready = false;
            self.structure_current = false;
            self.ddl_export.current = false;
            self.schema_choices_current = false;
            self.status =
                "Database may have changed; waiting for the owned Objects worker to close".into();
            cx.emit(CatalogEvent::SchemasChanged);
            cx.notify();
            return;
        }
        self.mark_disconnected(cx);
        self.status =
            "Database may have changed; reconnect and refresh before using retained data".into();
        cx.notify();
    }
    pub fn mark_disconnected(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.table_ddl {
            view.update(cx, |view, cx| view.disconnected(cx));
        }
        self.table_ddl_busy = false;
        self.ddl_export.incoming = None;
        self.ddl_export.current = false;
        self.incoming_structure = None;
        self.structure_current = false;
        self.structure_navigation = None;
        self.structure_after_connect = None;
        if let Some(view) = &self.maintenance {
            view.update(cx, |view, cx| view.disconnected(cx));
        }
        self.maintenance_busy = false;
        if let Some(schema) = &self.schema {
            schema.update(cx, |view, cx| view.disconnected(cx));
        }
        self.schema_busy = false;
        self.controls = None;
        self.receiver = None;
        self.ready = false;
        self.busy = false;
        self.pending = None;
        self.cancellation_requested = false;
        self.schema_choices_current = false;
        self.status = "Disconnected; retained catalog may be stale".into();
        cx.emit(CatalogEvent::SchemasChanged);
        cx.notify();
    }
    pub fn clear_results(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.schema_busy || self.maintenance_busy || self.table_ddl_busy {
            return;
        }
        self.ddl_export = ddl_export::DdlExport::default();
        self.visible.clear();
        self.catalog = None;
        self.schema_choices_current = false;
        cx.emit(CatalogEvent::SchemasChanged);
        self.structure_current = false;
        self.structure_navigation = None;
        self.incoming_structure = None;
        self.structure = None;
        self.structure_events = None;
        self.show_structure = false;
        self.incoming_details = None;
        self.details = None;
        self.details_events = None;
        self.show_details = false;
        self.previous_focus = None;
        self.selected = 0;
        cx.notify();
    }
    fn load(&mut self, cx: &mut Context<Self>) {
        if !self.editable
            || self.busy
            || self.schema_busy
            || self.maintenance_busy
            || self.table_ddl_busy
            || !self.ready
        {
            return;
        }
        self.next = self.next.wrapping_add(1);
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| controls.send(TableCommand::Catalog(self.next)))
        {
            Ok(()) => {
                self.busy = true;
                self.pending = Some(self.next);
                self.cancellation_requested = false;
                self.status = "Loading catalog; retained results may be stale".into()
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    fn filter(&mut self, cx: &mut Context<Self>) {
        if self.search.read(cx).buffer().read(cx).len(cx).0 > 8192 {
            self.status = "Search exceeds 8 KiB; previous filter preserved".into();
            return;
        }
        let search = self.search.read(cx).text(cx);
        self.applied_search = search.clone();
        self.visible = self
            .catalog
            .as_ref()
            .map(|catalog| catalog.matching(&search))
            .unwrap_or_default();
        self.selected = 0;
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(message) = self
            .receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv())
        else {
            return false;
        };
        let message = message.into_message();
        if matches!(
            &message,
            TableMessage::TableDdlObserved(..)
                | TableMessage::TableDdlReviewed(..)
                | TableMessage::TableDdlApplied(..)
        ) {
            if let Some(view) = &self.table_ddl {
                view.update(cx, |view, cx| view.receive(message, cx));
            }
            return true;
        }
        if matches!(
            &message,
            TableMessage::MaintenanceReviewed(..) | TableMessage::MaintenanceApplied(..)
        ) {
            if let Some(view) = &self.maintenance {
                view.update(cx, |view, cx| view.receive(message, cx));
            }
            return true;
        }
        if matches!(
            &message,
            TableMessage::SchemaReviewed(..) | TableMessage::SchemaApplied(..)
        ) {
            if let Some(schema) = &self.schema {
                schema.update(cx, |view, cx| view.receive(message, cx));
            }
            return true;
        }
        if self.cancellation_requested
            && match &message {
                TableMessage::Catalog(id, _)
                | TableMessage::Structure(id, _)
                | TableMessage::DdlExport(id, _)
                | TableMessage::Description(id, _)
                | TableMessage::DropImpact(id, _, _) => self.pending == Some(*id),
                _ => false,
            }
        {
            self.pending = None;
            self.busy = false;
            self.cancellation_requested = false;
            self.structure_navigation = None;
            self.status = "Catalog read cancelled; late reply discarded".into();
            cx.notify();
            return true;
        }
        match message {
            TableMessage::Opened => {
                self.ready = true;
                self.busy = false;
                if self.cancellation_requested {
                    self.structure_after_connect = None;
                    self.cancellation_requested = false;
                    self.status = "Connected; cancelled metadata read was not dispatched".into();
                } else if let Some(request) = self.structure_after_connect.take() {
                    self.request_structure(request);
                } else {
                    self.load(cx);
                }
            }
            TableMessage::Catalog(id, result) if self.pending == Some(id) => {
                self.pending = None;
                self.busy = false;
                match result {
                    Ok(catalog) => match Catalog::new(catalog, self.budget.clone()) {
                        Ok(catalog) => {
                            let count = catalog.rows.len();
                            let partial = catalog.truncated.len();
                            self.visible = catalog.matching(&self.applied_search);
                            self.selected = 0;
                            self.catalog = Some(catalog);
                            self.schema_choices_current = true;
                            cx.emit(CatalogEvent::SchemasChanged);
                            self.status = format!(
                                "{count} objects{}",
                                if partial > 0 {
                                    format!(
                                        "; {partial} catalog groups truncated (2,000 per group)"
                                    )
                                } else {
                                    String::new()
                                }
                            );
                        }
                        Err(error) => self.status = format!("{error}; previous catalog retained"),
                    },
                    Err(error) => {
                        self.status =
                            format!("Catalog read failed: {error:?}; previous catalog retained")
                    }
                }
            }
            TableMessage::DdlExport(id, result) if self.pending == Some(id) => {
                self.settle_ddl_export(result);
            }
            TableMessage::Structure(id, result) if self.pending == Some(id) => {
                self.settle_structure(id, result, cx)
            }
            TableMessage::Description(id, result) if self.pending == Some(id) => {
                self.pending = None;
                self.busy = false;
                match result {
                    Ok(description) => {
                        match crate::object_details::Details::new(description, self.budget.clone())
                        {
                            Ok(details) => {
                                self.incoming_details = Some(details);
                                self.status = "Object metadata loaded".into();
                            }
                            Err(error) => self.status = error.into(),
                        }
                    }
                    Err(error) => self.status = format!("Object description refused: {error:?}"),
                }
            }
            TableMessage::DropImpact(id, reference, result) if self.pending == Some(id) => {
                self.pending = None;
                self.busy = false;
                match result {
                    Ok(impact) => match crate::object_details::Details::drop_impact(
                        reference,
                        impact,
                        self.budget.clone(),
                    ) {
                        Ok(details) => {
                            self.incoming_details = Some(details);
                            self.status = "Read-only drop impact loaded".into();
                        }
                        Err(error) => self.status = error.into(),
                    },
                    Err(error) => self.status = format!("Drop impact refused: {error:?}"),
                }
            }
            TableMessage::Structure(..)
            | TableMessage::WholeTableExport(..)
            | TableMessage::DdlExport(..)
            | TableMessage::Catalog(..)
            | TableMessage::Description(..)
            | TableMessage::DropImpact(..) => {}
            TableMessage::Error(error) => {
                self.status = format!("Catalog failed: {error}");
                self.failure = Some(self.status.clone());
            }
            TableMessage::Closed(result) => {
                self.mark_disconnected(cx);
                self.status = match result {
                    Ok(dbunk_lib::backend::data::DataCloseOutcome::Closed) => self
                        .failure
                        .take()
                        .unwrap_or_else(|| "Catalog disconnected".into()),
                    Ok(dbunk_lib::backend::data::DataCloseOutcome::ConnectionDataClosed) => {
                        "All data documents on this connection closed during cleanup".into()
                    }
                    Err(error) => format!("Catalog cleanup failed: {error}"),
                };
            }
            _ => self.status = "Unexpected data reply in catalog document".into(),
        }
        cx.notify();
        true
    }
    fn describe(&mut self, impact: bool, _cx: &mut Context<Self>) {
        if !self.editable
            || self.busy
            || self.schema_busy
            || self.maintenance_busy
            || self.table_ddl_busy
            || !self.ready
        {
            return;
        }
        let Some(reference) = self
            .visible
            .get(self.selected)
            .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
            .and_then(|row| row.reference())
        else {
            self.status = "Descriptions are not available for this cluster object kind".into();
            return;
        };
        self.next = self.next.wrapping_add(1);
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| {
                controls.send(if impact {
                    TableCommand::DropImpact(self.next, reference)
                } else {
                    TableCommand::Describe(self.next, reference)
                })
            }) {
            Ok(()) => {
                self.pending = Some(self.next);
                self.cancellation_requested = false;
                self.busy = true;
                self.status = if impact {
                    "Reading downstream drop impact; no DDL is executed"
                } else {
                    "Loading object metadata"
                }
                .into();
            }
            Err(error) => self.status = error.into(),
        }
    }
    fn enabled(&self, action: Action) -> bool {
        if !self.editable {
            return false;
        }
        let selected = self.visible.get(self.selected).is_some();
        if self.table_ddl_busy {
            return matches!(action, Action::TableDdl);
        }
        if self.maintenance_busy {
            // Back may hide an owned operation. Keep its review reachable so
            // its cancel control remains available until the worker settles.
            return matches!(action, Action::Maintenance);
        }
        if self.schema_busy {
            return false;
        }
        match action {
            Action::TableDdl => !self.busy && self.connection.is_some(),
            Action::Schema | Action::Maintenance | Action::DdlExport => {
                !self.busy && self.connection.is_some()
            }
            Action::Connect => self.controls.is_none() && !self.busy,
            Action::Refresh => self.ready && !self.busy,
            Action::Cancel => self.controls.is_some() && self.busy,
            Action::Search => self.catalog.is_some(),
            Action::Open => selected && !self.busy,
            Action::Structure => {
                self.ready
                    && !self.busy
                    && self
                        .visible
                        .get(self.selected)
                        .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
                        .is_some_and(|row| row.kind.relation())
            }
            Action::Describe | Action::DropImpact => selected && self.ready && !self.busy,
            Action::Copy => selected,
            Action::Clear => !self.busy,
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::TableDdl => self.open_table_ddl(None, window, cx),
            Action::Schema => self.open_schema(window, cx),
            Action::DdlExport => self.show_ddl_export(window, cx),
            Action::Maintenance => self.open_maintenance(window, cx),
            Action::Connect => self.begin_connect(cx),
            Action::Refresh => self.load(cx),
            Action::Cancel => {
                self.structure_after_connect = None;
                if self.busy
                    && let Some(controls) = &self.controls
                {
                    controls.cancel();
                    self.cancellation_requested = true;
                    self.status = "Catalog cancellation requested; waiting for outcome".into();
                }
            }
            Action::Search => self.filter(cx),
            Action::Clear => self.clear_results(cx),
            Action::Copy => {
                if let Some(row) = self
                    .visible
                    .get(self.selected)
                    .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(row.qualified()));
                }
            }
            Action::Structure => self.inspect_structure(),
            Action::Describe => self.describe(false, cx),
            Action::DropImpact => self.describe(true, cx),
            Action::Open => {
                if self.busy {
                    return;
                }
                if let Some(row) = self
                    .visible
                    .get(self.selected)
                    .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
                {
                    if row.kind.relation()
                        && let (Some(connection), Some(schema)) = (&self.connection, &row.schema)
                    {
                        cx.emit(CatalogEvent::OpenTable {
                            connection: connection.clone(),
                            schema: schema.clone(),
                            table: row.entry.name.clone(),
                        });
                    } else {
                        self.describe(false, cx);
                    }
                }
            }
        }
        let _ = window;
        cx.notify();
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        action: Action,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let weak = cx.entity().downgrade();
        let enabled = self.enabled(action);
        div()
            .id(("catalog-action", index))
            .role(Role::Button)
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .focus(|style| style.bg(rgb(0x222222)))
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .px_2()
            .py_1()
            .border_1()
            .border_color(rgb(0x444444))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
}
impl Render for CatalogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_table_ddl(cx);
        if let Some(view) = &self.maintenance {
            view.update(cx, |view, _| {
                view.set_runtime(
                    self.controls.clone(),
                    self.ready && !self.busy && !self.schema_busy && !self.table_ddl_busy,
                    self.editable && !self.busy && !self.schema_busy && !self.table_ddl_busy,
                )
            });
        }
        if let Some(schema) = &self.schema {
            schema.update(cx, |view, cx| {
                view.set_runtime(
                    self.controls.clone(),
                    self.ready && !self.busy && !self.table_ddl_busy && !self.maintenance_busy,
                    self.editable && !self.busy && !self.table_ddl_busy && !self.maintenance_busy,
                    cx,
                )
            });
        }
        self.sync_ddl_export(window, cx);
        if self.ddl_export.show
            && let Some(view) = &self.ddl_export.view
        {
            return view.clone().into_any_element();
        }
        self.render_structure(window, cx);
        if self.show_structure
            && let Some(view) = &self.structure
        {
            return view.clone().into_any_element();
        }
        if let Some(data) = self.incoming_details.take() {
            let focused = self.root.contains_focused(window, cx);
            let view = cx.new(|cx| crate::object_details::DetailsView::new(data, window, cx));
            self.details_events = Some(cx.subscribe_in(&view, window, |this, _, _, window, cx| {
                this.show_details = false;
                this.previous_focus = None;
                window.focus(&this.list, cx);
                cx.notify();
            }));
            if focused {
                window.focus(&view.read(cx).focus(cx), cx);
            }
            self.details = Some(view);
            self.show_details = true;
        }
        if self.show_details
            && let Some(view) = &self.details
        {
            return view.clone().into_any_element();
        }

        let selected = self
            .visible
            .get(self.selected)
            .and_then(|i| self.catalog.as_ref()?.rows.get(*i));
        let details = selected
            .map(|row| {
                format!(
                    "{}\n{}\n{}",
                    row.kind.label(),
                    row.qualified(),
                    row.entry.comment.as_deref().unwrap_or("")
                )
            })
            .unwrap_or_default();
        let truncated = self
            .catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .truncated
                    .iter()
                    .map(|group| {
                        format!(
                            "{}: {}",
                            group.schema.as_deref().unwrap_or("cluster"),
                            group.kind
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        div()
            .id("catalog-tool")
            .role(Role::Group)
            .aria_label("PostgreSQL objects")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0))
            .text_color(rgb(0xffffff))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.show_table_ddl
                    && this
                        .table_ddl
                        .as_ref()
                        .is_some_and(|view| view.read(cx).contains_focus(window, cx))
                {
                    return;
                }
                if this.show_maintenance
                    && this
                        .maintenance
                        .as_ref()
                        .is_some_and(|view| view.read(cx).contains_focus(window, cx))
                {
                    return;
                }
                if this.show_schema
                    && this
                        .schema
                        .as_ref()
                        .is_some_and(|schema| schema.read(cx).contains_focus(window, cx))
                {
                    return;
                }
                let modifiers = &event.keystroke.modifiers;
                if this.search.focus_handle(cx).is_focused(window)
                    && this
                        .search
                        .update(cx, |editor, cx| editor.marked_text_range(window, cx))
                        .is_some()
                {
                    return;
                }

                if event.keystroke.key == "tab"
                    && !modifiers.control
                    && !modifiers.alt
                    && !modifiers.platform
                {
                    let handles = std::iter::once(this.search.focus_handle(cx))
                        .chain(
                            this.buttons
                                .iter()
                                .zip([
                                    Action::Connect,
                                    Action::Refresh,
                                    Action::Cancel,
                                    Action::Search,
                                    Action::Open,
                                    Action::Copy,
                                    Action::Clear,
                                    Action::Describe,
                                    Action::DropImpact,
                                    Action::Schema,
                                    Action::Maintenance,
                                    Action::Structure,
                                    Action::DdlExport,
                                    Action::TableDdl,
                                ])
                                .filter(|(_, action)| this.enabled(*action))
                                .map(|(focus, _)| focus.clone()),
                        )
                        .chain(std::iter::once(this.list.clone()))
                        .collect::<Vec<_>>();
                    let current = handles.iter().position(|focus| focus.is_focused(window));
                    let next = if modifiers.shift {
                        current.map_or(handles.len() - 1, |i| {
                            (i + handles.len() - 1) % handles.len()
                        })
                    } else {
                        current.map_or(0, |i| (i + 1) % handles.len())
                    };
                    window.focus(&handles[next], cx);
                    cx.stop_propagation();
                    return;
                }
                if this.search.focus_handle(cx).is_focused(window)
                    && event.keystroke.key == "enter"
                    && this
                        .search
                        .update(cx, |editor, cx| editor.marked_text_range(window, cx))
                        .is_none()
                {
                    this.filter(cx);
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if !this.list.is_focused(window)
                    || modifiers.control
                    || modifiers.alt
                    || modifiers.platform
                {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "down" => {
                        this.selected =
                            (this.selected + 1).min(this.visible.len().saturating_sub(1))
                    }
                    "up" => this.selected = this.selected.saturating_sub(1),
                    "home" => this.selected = 0,
                    "end" => this.selected = this.visible.len().saturating_sub(1),
                    "enter" => this.activate(Action::Open, window, cx),
                    _ => return,
                }
                this.scroll
                    .scroll_to_item(this.selected, gpui::ScrollStrategy::Center);
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .flex_shrink_0()
                    .child(self.button(0, "Connect", Action::Connect, cx))
                    .child(self.button(1, "Refresh", Action::Refresh, cx))
                    .child(self.button(2, "Cancel", Action::Cancel, cx))
                    .child(self.button(3, "Search", Action::Search, cx))
                    .child(self.button(4, "Open", Action::Open, cx))
                    .child(self.button(5, "Copy identity", Action::Copy, cx))
                    .child(self.button(6, "Clear", Action::Clear, cx))
                    .child(self.button(7, "Describe", Action::Describe, cx))
                    .child(self.button(8, "Drop impact", Action::DropImpact, cx))
                    .child(
                        self.button(
                            9,
                            if self.schema_recovery.is_some()
                                || self
                                    .schema
                                    .as_ref()
                                    .is_some_and(|schema| schema.read(cx).has_changes())
                            {
                                "Schema draft"
                            } else {
                                "Create schema"
                            },
                            Action::Schema,
                            cx,
                        ),
                    )
                    .child(self.button(10, "Maintenance", Action::Maintenance, cx))
                    .child(self.button(11, "Structure", Action::Structure, cx))
                    .child(self.button(12, "Export DDL", Action::DdlExport, cx))
                    .child(self.button(13, "Table change draft", Action::TableDdl, cx)),
            )
            .child(div().h(px(28.)).child(self.accessible.clone()))
            .child(
                div()
                    .id("catalog-status")
                    .role(Role::Label)
                    .aria_label(self.status.clone())
                    .child(self.status.clone()),
            )
            .when(!truncated.is_empty(), |root| {
                root.child(
                    div()
                        .id("catalog-truncation")
                        .role(Role::Label)
                        .aria_label(format!("Truncated groups: {truncated}"))
                        .max_h(px(80.))
                        .overflow_y_scroll()
                        .child(format!("Truncated: {truncated}")),
                )
            })
            .child(
                div()
                    .id("catalog-list")
                    .role(Role::ListBox)
                    .aria_label(format!(
                        "{} matching objects; arrows select, Enter opens",
                        self.visible.len()
                    ))
                    .aria_value(details.clone())
                    .track_focus(&self.list)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .child(
                        uniform_list(
                            "catalog-rows",
                            self.visible.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|position| {
                                        let index = this.visible[position];
                                        let label =
                                            this.catalog.as_ref().unwrap().rows[index].label();
                                        div()
                                            .id(("catalog-object", index))
                                            .role(Role::ListBoxOption)
                                            .aria_label(label.clone())
                                            .aria_selected(position == this.selected)
                                            .h(px(26.))
                                            .px_2()
                                            .overflow_hidden()
                                            .when(position == this.selected, |row| {
                                                row.bg(rgb(0x252525))
                                            })
                                            .child(SharedString::from(label))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.selected = position;
                                                window.focus(&this.list, cx);
                                                cx.notify();
                                            }))
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .h_full(),
                    ),
            )
            .child(
                div()
                    .id("catalog-selection")
                    .role(Role::Label)
                    .aria_label(details.clone())
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .child(details),
            )
            .when_some(
                self.table_ddl.clone().filter(|_| self.show_table_ddl),
                |root, view| root.child(div().h(px(330.)).flex_shrink_0().child(view)),
            )
            .when_some(
                self.maintenance.clone().filter(|_| self.show_maintenance),
                |root, view| root.child(div().h(px(330.)).flex_shrink_0().child(view)),
            )
            .when_some(
                self.schema.clone().filter(|_| self.show_schema),
                |root, view| root.child(div().h(px(330.)).flex_shrink_0().child(view)),
            )
            .into_any_element()
    }
}
