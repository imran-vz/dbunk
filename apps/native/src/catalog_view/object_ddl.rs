//! Objects-owned typed object-DDL review (drop, create view, create index,
//! add enum value). Tokens never cross the data-document lane; only the
//! descriptive journal is persisted.
use super::*;
use crate::{
    catalog::Kind,
    object_ddl_model::{Lease, Purpose},
    object_ddl_view::{ObjectDdlEvent, ObjectDdlView},
};
use dbunk_lib::backend::{
    WorkspaceObjectDdl,
    objects::{PgObjectKind, PgTypeClass},
};
/// Which object change a catalog action starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ObjectDdlStart {
    Drop,
    CreateView,
    CreateIndex,
    AddEnumValue,
}
impl CatalogView {
    pub fn object_ddl_snapshot(&self, cx: &gpui::App) -> Option<WorkspaceObjectDdl> {
        self.object_ddl.as_ref().map_or_else(
            || self.object_ddl_recovery.clone(),
            |view| view.read(cx).snapshot(),
        )
    }
    pub fn object_ddl_bytes(&self, cx: &gpui::App) -> usize {
        self.object_ddl.as_ref().map_or_else(
            || {
                self.object_ddl_recovery
                    .as_ref()
                    .map_or(0, crate::results::encoded_size)
            },
            |view| view.read(cx).snapshot_bytes(),
        )
    }
    pub fn has_object_ddl_changes(&self, cx: &gpui::App) -> bool {
        self.object_ddl_recovery.is_some()
            || self
                .object_ddl
                .as_ref()
                .is_some_and(|view| view.read(cx).has_changes())
    }
    /// Other Objects reviews stay usable while this one is idle; this one is
    /// usable only when no other Objects operation is in flight.
    pub(super) fn sync_object_ddl(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.object_ddl {
            let idle = self.others_idle(Lane::ObjectDdl);
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
    /// The selected row's purpose. Drop needs an exact supported object row;
    /// Create view uses a schema row, or the schema of any selected object;
    /// Create index needs a table row; Add enum value needs an enum type row.
    pub(super) fn object_ddl_purpose(
        &self,
        start: ObjectDdlStart,
    ) -> Result<Purpose, &'static str> {
        let row = self
            .visible
            .get(self.selected)
            .and_then(|i| self.catalog.as_ref()?.rows.get(*i))
            .ok_or("Select an Objects row first")?;
        match start {
            ObjectDdlStart::Drop => Purpose::drop(
                row.reference()
                    .ok_or("Cluster objects cannot be dropped from Objects")?,
            ),
            ObjectDdlStart::CreateView => match (row.reference(), &row.schema) {
                (Some(reference), _) if reference.kind == PgObjectKind::Schema => {
                    Purpose::create_view(reference.name)
                }
                (_, Some(schema)) => Purpose::create_view(schema.clone()),
                _ => Err("Select a schema row, or an object inside the target schema"),
            },
            ObjectDdlStart::CreateIndex => match (&row.kind, &row.schema) {
                (Kind::Object(PgObjectKind::Table), Some(schema)) => {
                    Purpose::create_index(schema.clone(), row.entry.name.clone())
                }
                _ => Err("Select a table row to create an index on it"),
            },
            ObjectDdlStart::AddEnumValue => match (&row.kind, &row.schema) {
                (Kind::Object(PgObjectKind::Type), Some(schema))
                    if row.entry.type_class == Some(PgTypeClass::Enum) =>
                {
                    Purpose::add_enum_value(schema.clone(), row.entry.name.clone())
                }
                _ => Err("Select an enum type row to add a value to it"),
            },
        }
    }
    /// Legacy entry: `Some(true)` drops, `Some(false)` creates a view.
    pub(super) fn open_object_ddl(
        &mut self,
        drop: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = drop.map(|drop| {
            if drop {
                ObjectDdlStart::Drop
            } else {
                ObjectDdlStart::CreateView
            }
        });
        self.open_object_ddl_for(start, window, cx)
    }
    /// Opens the object-change review for `start`, or reopens the retained
    /// change unchanged when `start` is `None` or a change is retained.
    pub(super) fn open_object_ddl_for(
        &mut self,
        start: Option<ObjectDdlStart>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.blocked_by_other_lane(Lane::ObjectDdl, cx) {
            self.status =
                "Finish or reconcile the current Objects operation before an object change".into();
            return;
        }
        // A retained change (live or restored) always reopens unchanged; the
        // selection is consulted only when there is nothing to finish.
        let retained = self.has_object_ddl_changes(cx);
        let purpose = match start.filter(|_| !retained) {
            Some(start) => match self.object_ddl_purpose(start) {
                Ok(purpose) => Some(purpose),
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            },
            None => None,
        };
        if retained && start.is_some() {
            self.status = "The previous object change is retained; finish or reconcile it before selecting another target".into();
        } else if purpose.is_some() {
            self.object_ddl = None;
            self.object_ddl_events = None;
        }
        if self.object_ddl.is_none() {
            if purpose.is_none() && self.object_ddl_recovery.is_none() {
                self.status =
                    "Select a row, then Drop object, Create view, Create index or Add enum value"
                        .into();
                return;
            }
            let Some(connection) = self.connection.as_deref() else {
                self.status =
                    "Select the recovery's connection before opening its object change".into();
                return;
            };
            let Some(lease) = Lease::admit(self.budget.clone()) else {
                self.status = "Object change needs 2 MiB of shared allowance; recovery retained. Clear another capture and retry.".into();
                return;
            };
            let prepared = match ObjectDdlView::prepare(
                connection,
                purpose.as_ref(),
                self.object_ddl_recovery.as_ref(),
            ) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            };
            let view = cx
                .new(|cx| ObjectDdlView::new(lease, self.apply_ids.clone(), prepared, window, cx));
            self.object_ddl_events =
                Some(
                    cx.subscribe_in(&view, window, |this, _, event, window, cx| {
                        match event {
                            ObjectDdlEvent::Changed => cx.emit(CatalogEvent::Changed),
                            ObjectDdlEvent::PersistApply(id) => {
                                cx.emit(CatalogEvent::PersistApply(*id))
                            }
                            ObjectDdlEvent::Activity(busy) => this.object_ddl_busy = *busy,
                            ObjectDdlEvent::Back => {
                                this.show_object_ddl = false;
                                this.previous_focus = None;
                                window.focus(&this.list, cx);
                            }
                            ObjectDdlEvent::DatabaseChanged(connection) => {
                                this.schema_choices_current = false;
                                this.structure_current = false;
                                this.ddl_export.current = false;
                                cx.emit(CatalogEvent::DatabaseChanged(connection.clone()));
                            }
                        }
                        cx.notify();
                    }),
                );
            self.object_ddl = Some(view);
            self.object_ddl_recovery = None;
        }
        self.show_lane(Lane::ObjectDdl);
        self.sync_object_ddl(cx);
        window.focus(&self.object_ddl.as_ref().unwrap().focus_handle(cx), cx);
        cx.notify();
    }
}

/// Objects write lanes. Each review mutually excludes the others: one opens
/// only when no other lane is in flight or retains a change, and showing one
/// bottom review hides the rest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    Schema,
    SchemaAlter,
    Maintenance,
    TableDdl,
    Sequence,
    ObjectDdl,
}
const LANES: [Lane; 6] = [
    Lane::Schema,
    Lane::SchemaAlter,
    Lane::Maintenance,
    Lane::TableDdl,
    Lane::Sequence,
    Lane::ObjectDdl,
];
impl CatalogView {
    fn lane_busy(&self, lane: Lane) -> bool {
        match lane {
            Lane::Schema => self.schema_busy,
            Lane::SchemaAlter => self.schema_alter_busy,
            Lane::Maintenance => self.maintenance_busy,
            Lane::TableDdl => self.table_ddl_busy,
            Lane::Sequence => self.sequence_busy,
            Lane::ObjectDdl => self.object_ddl_busy,
        }
    }
    fn lane_retained(&self, lane: Lane, cx: &gpui::App) -> bool {
        match lane {
            Lane::Schema => self.has_schema_changes(cx),
            Lane::SchemaAlter => self.has_schema_alter_changes(cx),
            Lane::Maintenance => self.has_maintenance_changes(cx),
            Lane::TableDdl => self.has_table_ddl_changes(cx),
            Lane::Sequence => self.has_sequence_changes(cx),
            Lane::ObjectDdl => self.has_object_ddl_changes(cx),
        }
    }
    /// The catalog reader and every other write lane are idle.
    pub(super) fn others_idle(&self, lane: Lane) -> bool {
        !self.busy && LANES.iter().all(|l| *l == lane || !self.lane_busy(*l))
    }
    /// Another lane is in flight or retains a change, so `lane` must not open.
    pub(super) fn blocked_by_other_lane(&self, lane: Lane, cx: &gpui::App) -> bool {
        LANES
            .iter()
            .any(|l| *l != lane && (self.lane_busy(*l) || self.lane_retained(*l, cx)))
    }
    /// Shows exactly one bottom review; hidden reviews keep their state.
    pub(super) fn show_lane(&mut self, lane: Lane) {
        self.show_schema = lane == Lane::Schema;
        self.show_schema_alter = lane == Lane::SchemaAlter;
        self.show_maintenance = lane == Lane::Maintenance;
        self.show_table_ddl = lane == Lane::TableDdl;
        self.show_sequence = lane == Lane::Sequence;
        self.show_object_ddl = lane == Lane::ObjectDdl;
        self.show_structure = false;
        self.show_details = false;
        self.ddl_export.show = false;
    }
}
