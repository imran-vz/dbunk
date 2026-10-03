//! Maintenance and schema tools share the document's save-attempt sequence.
//! Late acknowledgements cannot cross from one tool's cancelled flow to another.
use super::*;
use crate::maintenance_view::{Lease, MaintenanceEvent, MaintenanceView};
use dbunk_lib::backend::WorkspaceMaintenance;
impl CatalogView {
    pub fn maintenance_snapshot(&self, cx: &gpui::App) -> Option<WorkspaceMaintenance> {
        self.maintenance.as_ref().map_or_else(
            || self.maintenance_recovery.clone(),
            |view| view.read(cx).snapshot(),
        )
    }
    pub fn maintenance_bytes(&self, cx: &gpui::App) -> usize {
        self.maintenance.as_ref().map_or_else(
            || {
                self.maintenance_recovery
                    .as_ref()
                    .map_or(0, crate::results::encoded_size)
            },
            |view| view.read(cx).snapshot_bytes(),
        )
    }
    pub fn has_maintenance_changes(&self, cx: &gpui::App) -> bool {
        self.maintenance_recovery.is_some()
            || self
                .maintenance
                .as_ref()
                .is_some_and(|view| view.read(cx).has_changes())
    }
    pub(super) fn open_maintenance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_schema_changes(cx) || self.has_table_ddl_changes(cx) {
            self.status =
                "Finish or reconcile schema/table-change recovery before maintenance".into();
            return;
        }
        if !self.has_maintenance_changes(cx) {
            self.maintenance = None;
            self.maintenance_events = None;
        }
        if self.maintenance.is_none() {
            let reference = self
                .visible
                .get(self.selected)
                .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
                .and_then(|row| row.reference());
            if self.maintenance_recovery.is_none()
                && !reference.as_ref().is_some_and(|r| {
                    matches!(
                        r.kind,
                        dbunk_lib::backend::objects::PgObjectKind::Table
                            | dbunk_lib::backend::objects::PgObjectKind::MaterializedView
                    )
                })
            {
                self.status = "Select a table or materialized view".into();
                return;
            }
            let Some(lease) = Lease::admit(self.budget.clone()) else {
                self.status = "Maintenance review needs 128 KiB of shared allowance; recovery is retained. Clear another capture and retry.".into();
                return;
            };
            let recovery = self.maintenance_recovery.take();
            let view = cx.new(|cx| {
                MaintenanceView::new(lease, self.apply_ids.clone(), reference, recovery, cx)
            });
            self.maintenance_events =
                Some(
                    cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                        match event {
                            MaintenanceEvent::Changed => cx.emit(CatalogEvent::Changed),
                            MaintenanceEvent::PersistApply(id) => {
                                cx.emit(CatalogEvent::PersistApply(*id))
                            }
                            MaintenanceEvent::Activity(busy) => this.maintenance_busy = *busy,
                            MaintenanceEvent::Back => {
                                this.show_maintenance = false;
                                window.focus(&this.list, cx);
                            }
                        }
                        cx.notify();
                    }),
                );
            self.maintenance = Some(view);
        }
        self.show_table_ddl = false;
        self.show_maintenance = true;
        self.show_schema = false;
        self.show_details = false;
        window.focus(&self.maintenance.as_ref().unwrap().focus_handle(cx), cx);
    }
}
