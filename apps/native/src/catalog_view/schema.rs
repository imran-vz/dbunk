//! Schema review remains owned by this exact Objects document, even when hidden.
use super::*;
use crate::schema_view::{Lease, SchemaEvent, SchemaView};
use dbunk_lib::backend::WorkspaceSchemaChanges;
impl CatalogView {
    pub fn schema_snapshot(&self, cx: &gpui::App) -> Option<WorkspaceSchemaChanges> {
        self.schema.as_ref().map_or_else(
            || self.schema_recovery.clone(),
            |view| view.read(cx).snapshot(),
        )
    }
    pub fn schema_bytes(&self, cx: &gpui::App) -> usize {
        self.schema.as_ref().map_or_else(
            || {
                self.schema_recovery
                    .as_ref()
                    .map_or(0, crate::results::encoded_size)
            },
            |view| view.read(cx).snapshot_bytes(),
        )
    }
    pub fn has_schema_changes(&self, cx: &gpui::App) -> bool {
        self.schema_recovery.is_some()
            || self
                .schema
                .as_ref()
                .is_some_and(|view| view.read(cx).has_changes())
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if let Some(view) = &self.table_ddl {
            view.update(cx, |view, cx| view.apply_saved(id, result.clone(), cx));
        }
        if let Some(view) = &self.maintenance {
            view.update(cx, |view, cx| view.apply_saved(id, result.clone(), cx));
        }
        if let Some(view) = &self.schema {
            view.update(cx, |view, cx| view.apply_saved(id, result, cx));
        }
    }
    pub(super) fn open_schema(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_maintenance_changes(cx) || self.has_table_ddl_changes(cx) {
            self.status =
                "Finish or reconcile maintenance/table-change recovery before a schema review"
                    .into();
            return;
        }
        self.show_table_ddl = false;
        self.show_maintenance = false;
        if self.schema.is_none() {
            let Some(lease) = Lease::admit(self.budget.clone()) else {
                self.status = "Schema review needs 1 MiB of shared allowance; recovery is retained. Clear another capture and retry.".into();
                return;
            };
            let recovery = self.schema_recovery.take();
            let connection = self.connection.clone().unwrap_or_default();
            let view = cx.new(|cx| {
                SchemaView::new(
                    lease,
                    self.apply_ids.clone(),
                    connection,
                    recovery,
                    window,
                    cx,
                )
            });
            self.schema_events = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event, window, cx| {
                    match event {
                        SchemaEvent::Changed => cx.emit(CatalogEvent::Changed),
                        SchemaEvent::PersistApply(id) => cx.emit(CatalogEvent::PersistApply(*id)),
                        SchemaEvent::Activity(busy) => this.schema_busy = *busy,
                        SchemaEvent::Back => {
                            this.show_schema = false;
                            window.focus(&this.list, cx);
                        }
                    }
                    cx.notify();
                },
            ));
            self.schema = Some(view);
        }
        self.show_schema = true;
        self.show_details = false;
        window.focus(&self.schema.as_ref().unwrap().focus_handle(cx), cx);
    }
}
