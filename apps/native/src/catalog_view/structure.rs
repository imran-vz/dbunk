//! Structure reads share the catalog document's admission, cancellation and
//! delivery lane. A failed refresh leaves the previous capture available.
use super::*;
use dbunk_lib::backend::{
    data::DataError,
    table_structure::{TableStructureRequest, TableStructureSnapshot},
};

impl CatalogView {
    pub fn structure_context(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        let request = TableStructureRequest {
            schema,
            table,
            expected: None,
        };
        if self.ready {
            if self.busy || self.schema_busy || self.maintenance_busy || self.table_ddl_busy {
                self.status =
                    "Finish the current Objects operation, then open Structure again".into();
            } else {
                self.request_structure(request);
            }
        } else if self.editable {
            self.structure_after_connect = Some(request);
            if self.controls.is_none() {
                self.begin_connect(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn inspect_structure(&mut self) {
        let Some(row) = self
            .visible
            .get(self.selected)
            .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
        else {
            return;
        };
        if !row.kind.relation() {
            return;
        }
        let Some(schema) = &row.schema else {
            return;
        };
        let request = TableStructureRequest {
            schema: schema.clone(),
            table: row.entry.name.clone(),
            expected: None,
        };
        self.request_structure(request);
    }

    pub(super) fn request_structure(&mut self, request: TableStructureRequest) {
        if !self.ready
            || !self.editable
            || self.busy
            || self.schema_busy
            || self.maintenance_busy
            || self.table_ddl_busy
        {
            return;
        }
        self.structure_current = false;
        self.incoming_structure = None;
        self.structure_navigation = None;
        self.next = self.next.wrapping_add(1);
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| controls.send(TableCommand::Structure(self.next, request)))
        {
            Ok(()) => {
                self.pending = Some(self.next);
                self.busy = true;
                self.cancellation_requested = false;
                self.status = "Reading table structure; previous capture may be stale".into();
            }
            Err(error) => self.status = error.into(),
        }
    }

    pub(super) fn settle_structure(
        &mut self,
        id: u64,
        result: Result<TableStructureSnapshot, Arc<DataError>>,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        self.busy = false;
        let navigating = self.structure_navigation.take() == Some(id);
        if navigating && let Ok(snapshot) = &result {
            if let Some(connection) = &self.connection {
                cx.emit(CatalogEvent::OpenTable {
                    connection: connection.clone(),
                    schema: snapshot.schema.clone(),
                    table: snapshot.table.clone(),
                });
                self.status = "Related table identity checked; opening a fresh table read".into();
            }
            return;
        }
        match result {
            Ok(snapshot) => {
                match crate::table_structure_model::Capture::new(snapshot, self.budget.clone()) {
                    Ok(capture) => {
                        self.structure_current = true;
                        self.incoming_structure = Some(capture);
                        self.status = "Read-only table structure loaded".into();
                    }
                    Err(error) => self.status = format!("{error}; previous capture retained"),
                }
            }
            Err(error) => {
                let error = match error.as_ref() {
                    DataError::Catalog(error) => error.to_string(),
                    error => format!("{error:?}"),
                };
                self.status = format!("Structure refused: {error}; previous capture retained");
            }
        }
    }

    pub(super) fn render_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::table_structure_view::{StructureEvent, StructureView};
        if let Some(capture) = self.incoming_structure.take() {
            let focused = self.root.contains_focused(window, cx)
                || self
                    .structure
                    .as_ref()
                    .is_some_and(|view| view.read(cx).contains_focus(window, cx));
            let view = cx.new(|cx| StructureView::new(capture, window, cx));
            self.structure_events = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event, window, cx| {
                    match event {
                        StructureEvent::Edit(selection) => {
                            if this.structure_current
                                && this.ready
                                && this.editable
                                && !this.has_pending()
                            {
                                this.open_table_ddl(Some(selection.clone()), window, cx);
                            }
                        }
                        StructureEvent::Back => {
                            this.show_structure = false;
                            this.previous_focus = None;
                            window.focus(&this.list, cx);
                            // A table-context inspection can bypass the catalog's first load.
                            if this.catalog.is_none() {
                                this.load(cx);
                            }
                        }
                        StructureEvent::Refresh(request) => this.request_structure(request.clone()),
                        StructureEvent::Cancel => this.activate(Action::Cancel, window, cx),
                        StructureEvent::OpenRelation {
                            schema,
                            table,
                            identity,
                        } => {
                            if this.structure_current && this.ready && this.editable && !this.busy {
                                this.request_structure(TableStructureRequest {
                                    schema: schema.clone(),
                                    table: table.clone(),
                                    expected: Some(*identity),
                                });
                                if this.busy {
                                    this.structure_navigation = this.pending;
                                }
                            }
                        }
                    }
                    cx.notify();
                },
            ));
            if focused {
                window.focus(&view.read(cx).focus(cx), cx);
            }
            self.structure = Some(view);
            self.show_structure = true;
            self.show_details = false;
        }
        if let Some(view) = &self.structure {
            view.update(cx, |view, cx| {
                view.set_runtime(
                    self.ready
                        && !self.schema_busy
                        && !self.maintenance_busy
                        && !self.table_ddl_busy,
                    self.busy,
                    self.editable,
                    self.structure_current,
                    &self.status,
                    cx,
                )
            });
        }
    }
}
