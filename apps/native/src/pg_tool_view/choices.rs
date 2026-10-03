use super::*;
use crate::catalog::{Catalog, Kind};
use dbunk_lib::backend::objects::{MAX_CATALOG_NODES, PgObjectCatalog, PgObjectKind};

const ALLOWANCE: usize = 8 * 1024 * 1024;
struct Lease(Rc<Cell<usize>>);
impl Lease {
    fn new(budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if ALLOWANCE > (128_usize * 1024 * 1024).saturating_sub(budget.get()) {
            return Err(
                "Choice preparation needs 8 MiB of shared allowance; previous choices retained",
            );
        }
        budget.set(budget.get() + ALLOWANCE);
        Ok(Self(budget))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(ALLOWANCE));
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ChoiceKind {
    #[default]
    Schema,
    Table,
}
pub(super) struct Choices {
    pub catalog: Catalog,
    pub visible: Vec<usize>,
    _lease: Lease,
}
impl Choices {
    fn new(mut data: PgObjectCatalog, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        // Before Catalog expands schema identities, admit capacity/index work.
        // Only baseline backup schema.tables (ordinary/partitioned) are choices.
        let lease = Lease::new(budget.clone())?;
        let count = data
            .schemas
            .iter()
            .try_fold(0usize, |count, schema| {
                count.checked_add(schema.tables.len() + 1)
            })
            .ok_or("Choice count exceeds bounds")?;
        if count > MAX_CATALOG_NODES
            || data.schemas.iter().any(|schema| {
                schema.name.len() > 63
                    || schema.name.contains('\0')
                    || schema
                        .tables
                        .iter()
                        .any(|table| table.name.len() > 63 || table.name.contains('\0'))
            })
        {
            return Err("Choice identities exceed supported backup scope bounds");
        }
        for schema in &mut data.schemas {
            schema.views = Vec::new();
            schema.materialized_views = Vec::new();
            schema.foreign_tables = Vec::new();
            schema.sequences = Vec::new();
            schema.functions = Vec::new();
            schema.procedures = Vec::new();
            schema.aggregates = Vec::new();
            schema.types = Vec::new();
            schema.domains = Vec::new();
            schema.extensions = Vec::new();
            for table in &mut schema.tables {
                table.comment = None;
                table.identity_args = None;
                table.type_class = None;
            }
        }
        data.roles = Vec::new();
        data.tablespaces = Vec::new();
        data.event_triggers = Vec::new();
        data.truncated
            .retain(|limit| matches!(limit.kind.as_str(), "schema" | "table"));
        let catalog = Catalog::new(data, budget)?;
        Ok(Self {
            catalog,
            visible: Vec::with_capacity(count),
            _lease: lease,
        })
    }
    pub fn filter(&mut self, kind: ChoiceKind, schema: &str) {
        self.visible.clear();
        self.visible.extend(
            self.catalog
                .rows
                .iter()
                .enumerate()
                .filter_map(|(index, row)| matches_choice(row, kind, schema).then_some(index)),
        );
    }
}
fn matches_choice(row: &crate::catalog::Row, kind: ChoiceKind, schema: &str) -> bool {
    match kind {
        ChoiceKind::Schema => matches!(row.kind, Kind::Object(PgObjectKind::Schema)),
        ChoiceKind::Table => {
            matches!(row.kind, Kind::Object(PgObjectKind::Table))
                && row.schema.as_deref() == Some(schema)
        }
    }
}
#[derive(Default)]
pub(super) struct Metadata {
    pub controls: Option<TableControls>,
    pub receiver: Option<TableReceiver>,
    pub ready: bool,
    pub opening: bool,
    pub pending: Option<u64>,
    pub cancelling: bool,
    pub next: u64,
    pub revision: u64,
    pub filter_revision: u64,
    pub capture: Option<Choices>,
    pub kind: ChoiceKind,
    pub selected: Option<usize>,
}
impl Metadata {
    pub fn busy(&self) -> bool {
        self.opening || self.pending.is_some() || self.cancelling
    }
}
impl PgToolView {
    pub fn has_pending(&self) -> bool {
        self.metadata
            .receiver
            .as_ref()
            .is_some_and(TableReceiver::has_pending)
    }
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        self.stop_choices();
        self.status =
            "Database changed; load fresh choices. Job observer and setup text retained".into();
        cx.notify();
    }
    pub(super) fn stop_choices(&mut self) {
        if let Some(controls) = self.metadata.controls.take() {
            controls.stop();
        }
        self.metadata.receiver = None;
        self.metadata.ready = false;
        self.metadata.opening = false;
        self.metadata.pending = None;
        self.metadata.cancelling = false;
        self.metadata.capture = None;
        self.metadata.selected = None;
    }
    pub(super) fn load_choices(&mut self, cx: &mut Context<Self>) {
        if self.metadata.busy() {
            return;
        }
        if self.metadata.ready {
            self.request_choices(cx);
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        match self
            .host
            .open_table_document(self.id.clone(), connection, self.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.metadata.controls = Some(controls);
                self.metadata.receiver = Some(receiver);
                self.metadata.opening = true;
                self.status = "Connecting read-only backup choices reader".into();
            }
            Err(error) => self.status = error.into(),
        }
    }
    fn request_choices(&mut self, _cx: &mut Context<Self>) {
        let Some(id) = self.metadata.next.checked_add(1) else {
            self.status = "Choice request identity exhausted; reopen setup".into();
            return;
        };
        self.metadata.next = id;
        if let Some(controls) = &self.metadata.controls {
            match controls.send(TableCommand::Catalog(id)) {
                Ok(()) => {
                    self.metadata.pending = Some(id);
                    self.status =
                        "Loading backup choices; exact text entry remains available".into();
                }
                Err(error) => self.status = error.into(),
            }
        }
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(envelope) = self
            .metadata
            .receiver
            .as_ref()
            .and_then(TableReceiver::try_recv)
        else {
            return false;
        };
        match envelope.into_message() {
            TableMessage::Opened => {
                self.metadata.opening = false;
                if !self.metadata.cancelling {
                    self.metadata.ready = true;
                    self.request_choices(cx);
                }
            }
            TableMessage::Catalog(id, result) if self.metadata.pending == Some(id) => {
                self.metadata.pending = None;
                if self.metadata.cancelling {
                    self.status = "Choice read cancelled; late catalog discarded".into();
                } else {
                    match result {
                        Ok(data) => match Choices::new(data, self.budget.clone()) {
                            Ok(mut capture) => {
                                capture.filter(self.metadata.kind, &self.schema_seen);
                                self.metadata.capture = Some(capture);
                                self.metadata.selected = None;
                                self.metadata.revision = id;
                                self.status = "Choices loaded; missing or truncated names can be entered exactly".into();
                            }
                            Err(error) => self.status = error.into(),
                        },
                        Err(error) => {
                            self.status =
                                format!("Choice read failed: {error:?}; previous choices retained")
                        }
                    }
                }
            }
            TableMessage::Catalog(..) => {}
            TableMessage::Error(error) => self.status = format!("Choice reader failed: {error}"),
            TableMessage::Closed(result) => {
                // Retain the last capture, but drop every old delivery authority.
                self.metadata.controls = None;
                self.metadata.receiver = None;
                self.metadata.ready = false;
                self.metadata.opening = false;
                self.metadata.pending = None;
                self.metadata.cancelling = false;
                self.status = match result {
                    Ok(_) => "Choice reader closed; Load choices reconnects explicitly".into(),
                    Err(error) => format!("Choice reader cleanup failed: {error}"),
                };
            }
            _ => self.status = "Unexpected reply in backup choice reader".into(),
        }
        cx.notify();
        true
    }
    pub(super) fn reconcile_schema(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let Some((schema, table)) = &self.fields else {
            return Ok(());
        };
        if schema.update(cx, |field, cx| field.composing(window, cx)) {
            return Err("Finish schema composition before preparing");
        }
        let value = schema.read(cx).value(cx)?;
        if value != self.schema_seen {
            table.update(cx, |field, cx| field.set_value(String::new(), window, cx))?;
            self.schema_seen = value;
            self.metadata.filter_revision = self.metadata.filter_revision.wrapping_add(1);
            self.metadata.selected = None;
            if let Some(capture) = &mut self.metadata.capture {
                capture.filter(self.metadata.kind, &self.schema_seen);
            }
        }
        Ok(())
    }
    pub(super) fn choose(
        &mut self,
        revision: u64,
        filter_revision: u64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.setup_enabled(cx)
            || revision != self.metadata.revision
            || filter_revision != self.metadata.filter_revision
        {
            return;
        }
        let Some(capture) = &self.metadata.capture else {
            return;
        };
        let Some(row) = capture
            .visible
            .get(index)
            .and_then(|row| capture.catalog.rows.get(*row))
        else {
            return;
        };
        let name = row.entry.name.clone();
        let schema = row.schema.clone();
        let kind = self.metadata.kind;
        let Some((schema_field, table_field)) = self.fields.clone() else {
            return;
        };
        let result = match kind {
            ChoiceKind::Schema => {
                // Refuse before changing schema if the old table is composing.
                if table_field.update(cx, |field, cx| field.composing(window, cx)) {
                    Err("Finish table composition before changing schema")
                } else {
                    schema_field
                        .update(cx, |field, cx| field.set_value(name, window, cx))
                        .and_then(|()| self.reconcile_schema(window, cx))
                }
            }
            ChoiceKind::Table => self.reconcile_schema(window, cx).and_then(|()| {
                if schema.as_deref() != Some(self.schema_seen.as_str()) {
                    return Err("Table choice belongs to a previous schema");
                }
                table_field.update(cx, |field, cx| field.set_value(name, window, cx))
            }),
        };
        self.status = match result {
            Ok(()) => "Exact catalog name selected".into(),
            Err(error) => error.into(),
        };
        cx.notify();
    }
}

pub(super) fn unknown_restore(job: &dbunk_lib::backend::pg_tools::PgToolObservation) -> bool {
    job.kind == Operation::Restore
        && job.phase.terminal()
        && job.effect == dbunk_lib::backend::pg_tools::PgToolEffect::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::objects::PgCatalogEntry;
    fn row(kind: PgObjectKind, schema: Option<&str>) -> crate::catalog::Row {
        crate::catalog::Row {
            kind: Kind::Object(kind),
            schema: schema.map(str::to_owned),
            entry: PgCatalogEntry {
                name: "a.b\"c".into(),
                identity_args: None,
                comment: None,
                type_class: None,
            },
        }
    }
    #[test]
    fn baseline_choices_keep_exact_schema_and_only_ordinary_partitioned_tables() {
        assert!(matches_choice(
            &row(PgObjectKind::Schema, None),
            ChoiceKind::Schema,
            ""
        ));
        assert!(matches_choice(
            &row(PgObjectKind::Table, Some("Case.Sensitive")),
            ChoiceKind::Table,
            "Case.Sensitive"
        ));
        assert!(!matches_choice(
            &row(PgObjectKind::Table, Some("Case.Sensitive")),
            ChoiceKind::Table,
            "case.sensitive"
        ));
        for kind in [
            PgObjectKind::View,
            PgObjectKind::MaterializedView,
            PgObjectKind::ForeignTable,
            PgObjectKind::Sequence,
        ] {
            assert!(!matches_choice(
                &row(kind, Some("Case.Sensitive")),
                ChoiceKind::Table,
                "Case.Sensitive"
            ));
        }
    }
    fn data() -> PgObjectCatalog {
        use dbunk_lib::backend::objects::{PgCatalogTruncation, PgSchemaObjects};
        PgObjectCatalog {
            schemas: vec![PgSchemaObjects {
                name: "Case.Sensitive".into(),
                tables: vec![row(PgObjectKind::Table, None).entry],
                views: vec![row(PgObjectKind::View, None).entry],
                materialized_views: vec![],
                foreign_tables: vec![],
                sequences: vec![],
                functions: vec![],
                procedures: vec![],
                aggregates: vec![],
                types: vec![],
                domains: vec![],
                extensions: vec![],
            }],
            event_triggers: vec![],
            roles: vec![],
            tablespaces: vec![],
            truncated: vec![PgCatalogTruncation {
                schema: Some("Case.Sensitive".into()),
                kind: "table".into(),
            }],
        }
    }
    #[test]
    fn choices_preserve_truncation_exact_names_and_old_capture_on_admission_failure() {
        let budget = Rc::new(Cell::new(0));
        let mut first = Choices::new(data(), budget.clone()).unwrap();
        first.filter(ChoiceKind::Table, "Case.Sensitive");
        assert_eq!(first.visible.len(), 1);
        assert_eq!(first.catalog.rows[first.visible[0]].entry.name, "a.b\"c");
        assert_eq!(first.catalog.truncated.len(), 1);
        first.filter(ChoiceKind::Table, "case.sensitive");
        assert!(first.visible.is_empty());
        assert_eq!(first.catalog.truncated.len(), 1);
        let charged = budget.get();
        assert!(charged > ALLOWANCE);
        budget.set(128 * 1024 * 1024);
        assert!(Choices::new(data(), budget.clone()).is_err());
        assert_eq!(budget.get(), 128 * 1024 * 1024);
        assert_eq!(first.catalog.truncated.len(), 1);
        budget.set(charged);
        let second = Choices::new(data(), budget.clone()).unwrap();
        assert!(budget.get() > charged + ALLOWANCE);
        drop(first);
        drop(second);
        assert_eq!(budget.get(), 0);
        let mut invalid = data();
        invalid.schemas[0].name = "x".repeat(64);
        assert!(Choices::new(invalid, budget.clone()).is_err());
        assert_eq!(budget.get(), 0);
    }
}
