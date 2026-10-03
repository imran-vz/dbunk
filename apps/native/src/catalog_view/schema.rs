//! Schema review remains owned by this exact Objects document, even when hidden.
use super::*;
use crate::schema_view::{
    Lease, SchemaEvent, SchemaView,
    alter::{
        SchemaAlterEvent, SchemaAlterView,
        model::{Lease as AlterLease, Selection as AlterSelection},
    },
};
use dbunk_lib::backend::{
    WorkspaceSchemaAlter, WorkspaceSchemaChanges, schema_alter::SchemaAlterRequest,
};
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
        if let Some(view) = &self.object_ddl {
            view.update(cx, |view, cx| view.apply_saved(id, result.clone(), cx));
        }
        if let Some(view) = &self.table_ddl {
            view.update(cx, |view, cx| view.apply_saved(id, result.clone(), cx));
        }
        if let Some(view) = &self.maintenance {
            view.update(cx, |view, cx| view.apply_saved(id, result.clone(), cx));
        }
        if let Some(view) = &self.schema_alter {
            view.update(cx, |view, cx| view.apply_saved(id, result.clone(), cx));
        }
        if let Some(view) = &self.schema {
            view.update(cx, |view, cx| view.apply_saved(id, result, cx));
        }
    }
    pub(super) fn open_schema(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blocked_by_other_lane(super::object_ddl::Lane::Schema, cx) {
            self.status =
                "Finish or reconcile the current Objects operation before a schema review".into();
            return;
        }
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
        self.show_lane(super::object_ddl::Lane::Schema);
        window.focus(&self.schema.as_ref().unwrap().focus_handle(cx), cx);
    }
}

/// Existing-schema comment/rename review. Owned by this exact Objects document;
/// its journal is descriptive recovery and never restores executable authority.
impl CatalogView {
    pub fn schema_alter_snapshot(&self, cx: &gpui::App) -> Option<WorkspaceSchemaAlter> {
        self.schema_alter.as_ref().map_or_else(
            || self.schema_alter_recovery.clone(),
            |view| view.read(cx).snapshot(),
        )
    }
    pub fn schema_alter_bytes(&self, cx: &gpui::App) -> usize {
        self.schema_alter.as_ref().map_or_else(
            || {
                self.schema_alter_recovery
                    .as_ref()
                    .map_or(0, crate::results::encoded_size)
            },
            |view| view.read(cx).snapshot_bytes(),
        )
    }
    pub fn has_schema_alter_changes(&self, cx: &gpui::App) -> bool {
        self.schema_alter_recovery.is_some()
            || self
                .schema_alter
                .as_ref()
                .is_some_and(|view| view.read(cx).has_changes())
    }
    pub(super) fn sync_schema_alter(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.schema_alter {
            let idle = self.others_idle(super::object_ddl::Lane::SchemaAlter);
            view.update(cx, |view, cx| {
                view.set_runtime(
                    self.controls.clone(),
                    self.ready && idle,
                    self.editable && idle,
                    cx,
                )
            });
        }
    }
    /// The selected catalog row's exact schema name, if it is a schema row.
    pub(super) fn selected_schema(&self) -> Option<String> {
        self.visible
            .get(self.selected)
            .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
            .filter(|row| {
                matches!(
                    row.kind,
                    crate::catalog::Kind::Object(dbunk_lib::backend::objects::PgObjectKind::Schema)
                )
            })
            .map(|row| row.entry.name.clone())
    }
    pub(super) fn open_schema_alter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.blocked_by_other_lane(super::object_ddl::Lane::SchemaAlter, cx) {
            self.status =
                "Finish or reconcile the current Objects operation before a schema change".into();
            return;
        }
        let selected = self.selected_schema();
        if selected.is_some() && self.has_schema_alter_changes(cx) {
            self.status = "The previous schema change is retained; finish or reconcile it before selecting another schema".into();
        } else if selected.is_some() {
            self.schema_alter = None;
            self.schema_alter_events = None;
        }
        if self.schema_alter.is_none() {
            if selected.is_none() && self.schema_alter_recovery.is_none() {
                self.status = "Select a schema row, then Alter schema".into();
                return;
            }
            let Some(connection) = self.connection.as_deref() else {
                self.status =
                    "Select the recovery's connection before opening its schema change".into();
                return;
            };
            let selection = match selected
                .map(|schema| {
                    AlterSelection::new(SchemaAlterRequest {
                        schema,
                        expected: None,
                    })
                })
                .transpose()
            {
                Ok(selection) => selection,
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            };
            let Some(lease) = AlterLease::admit(self.budget.clone()) else {
                self.status = "Schema change needs 1 MiB of shared allowance; recovery retained. Clear another capture and retry.".into();
                return;
            };
            let prepared = match SchemaAlterView::prepare(
                connection,
                selection.as_ref(),
                self.schema_alter_recovery.as_ref(),
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            };
            let view = cx.new(|cx| {
                SchemaAlterView::new(lease, self.apply_ids.clone(), prepared, window, cx)
            });
            self.schema_alter_events =
                Some(
                    cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                        match event {
                            SchemaAlterEvent::Changed => cx.emit(CatalogEvent::Changed),
                            SchemaAlterEvent::PersistApply(id) => {
                                cx.emit(CatalogEvent::PersistApply(*id))
                            }
                            SchemaAlterEvent::Activity(busy) => this.schema_alter_busy = *busy,
                            SchemaAlterEvent::Back => {
                                this.show_schema_alter = false;
                                this.previous_focus = None;
                                window.focus(&this.list, cx);
                            }
                            SchemaAlterEvent::DatabaseChanged(connection) => {
                                cx.emit(CatalogEvent::DatabaseChanged(connection.clone()));
                            }
                        }
                        cx.notify();
                    }),
                );
            self.schema_alter = Some(view);
            self.schema_alter_recovery = None;
        }
        self.show_lane(super::object_ddl::Lane::SchemaAlter);
        self.sync_schema_alter(cx);
        window.focus(&self.schema_alter.as_ref().unwrap().focus_handle(cx), cx);
        cx.notify();
    }
}
