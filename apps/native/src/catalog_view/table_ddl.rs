//! Objects-owned table change review. Tokens never cross the data-document lane.
use super::*;
use crate::{
    table_ddl_model::{Lease, Selection},
    table_ddl_view::{TableDdlEvent, TableDdlView},
};
use dbunk_lib::backend::WorkspaceTableDdl;
impl CatalogView {
    pub fn table_ddl_snapshot(&self, cx: &gpui::App) -> Option<WorkspaceTableDdl> {
        self.table_ddl.as_ref().map_or_else(
            || self.table_ddl_recovery.clone(),
            |view| view.read(cx).snapshot(),
        )
    }
    pub fn table_ddl_bytes(&self, cx: &gpui::App) -> usize {
        self.table_ddl.as_ref().map_or_else(
            || {
                self.table_ddl_recovery
                    .as_ref()
                    .map_or(0, crate::results::encoded_size)
            },
            |view| view.read(cx).snapshot_bytes(),
        )
    }
    pub fn has_table_ddl_changes(&self, cx: &gpui::App) -> bool {
        self.table_ddl_recovery.is_some()
            || self
                .table_ddl
                .as_ref()
                .is_some_and(|view| view.read(cx).has_changes())
    }
    pub(super) fn sync_table_ddl(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.table_ddl {
            let idle = self.others_idle(super::object_ddl::Lane::TableDdl);
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
    pub(super) fn open_table_ddl(
        &mut self,
        selection: Option<Selection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.blocked_by_other_lane(super::object_ddl::Lane::TableDdl, cx) {
            self.status =
                "Finish or reconcile the current Objects operation before a table change".into();
            return;
        }
        if selection.is_some() && self.has_table_ddl_changes(cx) {
            self.status = "The previous table change is retained; finish or reconcile it before selecting another target".into();
        } else if selection.is_some() {
            self.table_ddl = None;
            self.table_ddl_events = None;
        }
        if self.table_ddl.is_none() {
            if selection.is_none() && self.table_ddl_recovery.is_none() {
                self.status =
                    "Open Structure and select Overview or a column, then Comment / rename".into();
                return;
            }
            let Some(connection) = self.connection.as_deref() else {
                self.status =
                    "Select the recovery's connection before opening its table change".into();
                return;
            };
            let Some(lease) = Lease::admit(self.budget.clone()) else {
                self.status = "Table change needs 1 MiB of shared allowance; recovery retained. Clear another capture and retry.".into();
                return;
            };
            let prepared = match TableDdlView::prepare(
                connection,
                selection.as_ref(),
                self.table_ddl_recovery.as_ref(),
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            };
            let view =
                cx.new(|cx| TableDdlView::new(lease, self.apply_ids.clone(), prepared, window, cx));
            self.table_ddl_events = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event, window, cx| {
                    match event {
                        TableDdlEvent::Changed => cx.emit(CatalogEvent::Changed),
                        TableDdlEvent::PersistApply(id) => cx.emit(CatalogEvent::PersistApply(*id)),
                        TableDdlEvent::Activity(busy) => this.table_ddl_busy = *busy,
                        TableDdlEvent::Back => {
                            this.show_table_ddl = false;
                            this.previous_focus = None;
                            window.focus(&this.list, cx);
                        }
                        TableDdlEvent::DatabaseChanged(connection) => {
                            cx.emit(CatalogEvent::DatabaseChanged(connection.clone()));
                        }
                    }
                    cx.notify();
                },
            ));
            self.table_ddl = Some(view);
            self.table_ddl_recovery = None;
        }
        self.show_lane(super::object_ddl::Lane::TableDdl);
        self.sync_table_ddl(cx);
        window.focus(&self.table_ddl.as_ref().unwrap().focus_handle(cx), cx);
        cx.notify();
    }
}
