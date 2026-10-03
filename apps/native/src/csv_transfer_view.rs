//! CSV setup owns only transient fields and inspections. The workspace store
//! keeps accepted transfers observable after this view is closed.
use crate::{
    bounded_field::Field,
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
    csv_transfer_model::{self as model, Mapping, Setup, SetupToken},
    csv_transfer_store::{CsvStore, InspectionOwner},
};
use dbunk_lib::backend::{WorkspaceDocument, csv_transfers::*};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, PathPromptOptions,
    ScrollHandle, Subscription, Task, UniformListScrollHandle, Window, accesskit::Role, div,
    prelude::*, px, rgb,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
mod choices;
mod render;
mod workbook;

pub struct CsvViewResources {
    pub host: Arc<Host>,
    pub store: Entity<CsvStore>,
    pub wake: async_channel::Sender<()>,
    pub retained: Rc<Cell<usize>>,
}
const FIELD_BYTES: usize = 3 * 1024 * 1024;
struct FieldLease(Rc<Cell<usize>>);
impl Drop for FieldLease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(FIELD_BYTES));
    }
}
#[derive(Clone, Copy)]
enum Action {
    Import,
    Export,
    Xlsx,
    LoadWorkbook,
    SelectSheet,
    Header,
    Pick,
    Inspect,
    LoadInspection,
    Review,
    Begin,
    Confirm,
    Refresh,
    Reacquire,
    Cancel,
    Dismiss,
    Clear,
    LoadChoices,
    Schemas,
    Tables,
    UseChoice,
    CancelChoices,
    Skip,
    PreviousTarget,
    NextTarget,
    ReleaseInspection,
    Delimiter(&'static str),
}
const ACTIONS: [(Action, &str); 30] = [
    (Action::Import, "Import CSV"),
    (Action::Export, "Export CSV"),
    (Action::Header, "Header row"),
    (Action::Pick, "Choose CSV file"),
    (Action::Inspect, "Inspect target and source"),
    (Action::LoadInspection, "Load prepared inspection"),
    (Action::Review, "Review exact transfer"),
    (Action::Begin, "Start reviewed transfer"),
    (Action::Confirm, "Confirm exact import"),
    (Action::Refresh, "Refresh observations"),
    (Action::Reacquire, "Review awaiting confirmation"),
    (Action::Cancel, "Cancel selected transfer"),
    (Action::Dismiss, "Dismiss finished transfer"),
    (Action::Clear, "Clear setup"),
    (Action::LoadChoices, "Load choices"),
    (Action::Schemas, "Schema choices"),
    (Action::Tables, "Table choices"),
    (Action::UseChoice, "Use selected choice"),
    (Action::CancelChoices, "Cancel choice read"),
    (Action::Skip, "Skip source column"),
    (Action::PreviousTarget, "Previous target column"),
    (Action::NextTarget, "Next target column"),
    (Action::ReleaseInspection, "Release inspection"),
    (Action::Delimiter(","), "Comma delimiter"),
    (Action::Delimiter(";"), "Semicolon delimiter"),
    (Action::Delimiter("\t"), "Tab delimiter"),
    (Action::Delimiter("|"), "Pipe delimiter"),
    (Action::Xlsx, "Import XLSX"),
    (Action::LoadWorkbook, "Load workbook sheets"),
    (Action::SelectSheet, "Inspect selected sheet"),
];
pub struct CsvTransferView {
    host: Arc<Host>,
    id: String,
    wake: async_channel::Sender<()>,
    store: Entity<CsvStore>,
    _observe: Subscription,
    budget: Rc<Cell<usize>>,
    setup: Option<Setup>,
    owner: Option<InspectionOwner>,
    connection: Option<String>,
    generation: u64,
    fields: Vec<Entity<Field>>,
    field_events: Vec<Subscription>,
    field_lease: Option<FieldLease>,
    field_seed: Option<(String, String)>,
    schema_seen: String,
    inspection: Option<(CsvInspectionId, SetupToken)>,
    mapping: Option<Mapping>,
    mapping_source: Option<usize>,
    sample_cell: Option<usize>,
    review_pair: Option<usize>,
    selected: Option<CsvTransferAttemptId>,
    unknown_ack: Option<CsvTransferAttemptId>,
    editable: bool,
    picking: bool,
    picker: Option<Task<()>>,
    status: String,
    metadata: choices::Metadata,
    choice_focus: FocusHandle,
    choice_scroll: UniformListScrollHandle,
    sheet: Option<u16>,
    sheet_focus: FocusHandle,
    sheet_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    buttons: Vec<FocusHandle>,
    mapping_focus: FocusHandle,
    review_focus: FocusHandle,
    sample_focus: FocusHandle,
    list: FocusHandle,
    details: FocusHandle,
    previous_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
}
impl CsvTransferView {
    pub fn new(
        resources: CsvViewResources,
        document: &WorkspaceDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let CsvViewResources {
            host,
            store,
            wake,
            retained,
        } = resources;
        let observe = cx.observe(&store, |this, _, cx| {
            this.sync_inspection(cx);
            this.freeze_fields(cx);
            cx.notify();
        });
        let mut this = Self {
            host,
            id: document.id.clone(),
            wake,
            store,
            _observe: observe,
            budget: retained,
            setup: None,
            owner: None,
            connection: document.connection_id.clone(),
            generation: 0,
            fields: vec![],
            field_events: vec![],
            field_lease: None,
            field_seed: Some((String::new(), String::new())),
            schema_seen: String::new(),
            inspection: None,
            mapping: None,
            mapping_source: None,
            sample_cell: None,
            review_pair: None,
            selected: None,
            unknown_ack: None,
            editable: true,
            picking: false,
            picker: None,
            status: "Choose the exact target and CSV options, then explicitly inspect and review"
                .into(),
            metadata: Default::default(),
            choice_focus: cx.focus_handle(),
            choice_scroll: UniformListScrollHandle::new(),
            sheet: None,
            sheet_focus: cx.focus_handle(),
            sheet_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(),
            mapping_focus: cx.focus_handle(),
            review_focus: cx.focus_handle(),
            sample_focus: cx.focus_handle(),
            list: cx.focus_handle(),
            details: cx.focus_handle(),
            previous_focus: None,
            scroll: ScrollHandle::new(),
        };
        this.admit_setup();
        this.rebuild_fields(window, cx);
        this
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.focus_order(cx);
        let focus = self
            .previous_focus
            .as_ref()
            .filter(|handle| order.contains(handle))
            .unwrap_or(&self.list);
        window.focus(focus, cx);
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.contains_focused(window, cx) {
            self.previous_focus = window.focused(cx);
        }
    }
    fn abandon(&mut self, cx: &mut Context<Self>) {
        if let Some(owner) = &self.owner {
            self.store
                .update(cx, |store, cx| store.abandon_inspection(owner, cx));
        }
        self.inspection = None;
        self.sheet = None;
        self.mapping = None;
        self.mapping_source = None;
        self.sample_cell = None;
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.abandon(cx);
        if let Some(setup) = &mut self.setup
            && let Err(error) = setup.invalidate_review()
        {
            self.status = error.into();
        }
        cx.notify();
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        if !editable {
            self.stop_choices();
            self.changed(cx);
            if let Some(generation) = self.generation.checked_add(1) {
                self.generation = generation;
                if let Some(setup) = &mut self.setup {
                    let connection = setup.connection().to_owned();
                    let _ = setup.retarget(connection, generation);
                }
            } else {
                self.setup = None;
            }
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    pub fn bind_connection(&mut self, connection: String, cx: &mut Context<Self>) {
        if connection.is_empty()
            || connection.len() > 128
            || connection.capacity() > 1024
            || connection.chars().any(char::is_control)
        {
            self.status = "Invalid connection identity".into();
            cx.notify();
            return;
        }
        self.abandon(cx);
        self.stop_choices();
        self.setup = None;
        self.owner = None;
        self.connection = Some(connection);
        self.field_seed = Some((String::new(), String::new()));
        self.selected = None;
        self.unknown_ack = None;
        self.admit_setup();
        cx.notify();
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.abandon(cx);
        self.setup = None;
        self.owner = None;
        self.field_seed = Some((String::new(), String::new()));
        self.admit_setup();
        self.status = "Setup cleared; accepted transfers remain in session observations".into();
        cx.notify();
    }
    /// Explicit whole-table export prefill. Parse and admit everything before
    /// replacing setup; no capture, inspection, picker or transfer is started.
    pub fn set_export_context(
        &mut self,
        target: CsvTarget,
        null_token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            self.status = "Finish composition before changing CSV export setup".into();
            cx.notify();
            return;
        }
        let result = (|| {
            let connection = self.connection.clone().ok_or("Choose a connection")?;
            let mut setup = Setup::new(
                connection,
                self.generation,
                CsvDirection::Export,
                Some(target.clone()),
                self.budget.clone(),
            )?;
            setup.set_options(CsvOptions {
                null_token,
                ..CsvOptions::default()
            })?;
            Ok::<_, &'static str>(setup)
        })();
        match result {
            Ok(setup) => {
                self.abandon(cx);
                self.owner = Some(InspectionOwner::new(setup.owner()));
                self.setup = Some(setup);
                self.field_seed = Some((target.schema, target.table));
                self.rebuild_fields(window, cx);
                if self.fields.len() == 6 {
                    self.status =
                        "Whole-table CSV export configured; inspect and review before starting"
                            .into();
                }
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    pub fn set_context(
        &mut self,
        direction: CsvDirection,
        target: CsvTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            self.status = "Finish text composition before changing CSV context".into();
            cx.notify();
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        match Setup::new(
            connection,
            self.generation,
            direction,
            Some(target.clone()),
            self.budget.clone(),
        ) {
            Ok(setup) => {
                self.abandon(cx);
                self.owner = Some(InspectionOwner::new(setup.owner()));
                self.setup = Some(setup);
                self.field_seed = Some((target.schema, target.table));
                self.rebuild_fields(window, cx);
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    fn admit_setup(&mut self) {
        if self.setup.is_none()
            && let Some(connection) = self.connection.clone()
        {
            match Setup::new(
                connection,
                self.generation,
                CsvDirection::Import,
                None,
                self.budget.clone(),
            ) {
                Ok(setup) => {
                    self.owner = Some(InspectionOwner::new(setup.owner()));
                    self.setup = Some(setup);
                }
                Err(error) => self.status = error.into(),
            }
        }
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields
            .iter()
            .any(|field| field.update(cx, |field, cx| field.composing(window, cx)))
    }
    fn rebuild_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if (self.field_seed.is_none() && !self.fields.is_empty()) || self.composing(window, cx) {
            return;
        }
        if self.field_lease.is_none() {
            if FIELD_BYTES > (128usize * 1024 * 1024).saturating_sub(self.budget.get()) {
                self.status="CSV fields need 3 MiB of shared allowance; clear captures and retry Clear setup".into();
                return;
            }
            self.budget.set(self.budget.get() + FIELD_BYTES);
            self.field_lease = Some(FieldLease(self.budget.clone()));
        }
        let (schema, table) = self.field_seed.take().unwrap_or_default();
        self.schema_seen = schema.clone();
        self.field_events.clear();
        self.fields.clear();
        let options = self
            .setup
            .as_ref()
            .map_or_else(CsvOptions::default, |setup| setup.options().clone());
        for (label, limit, value) in [
            ("CSV schema", 63, schema),
            ("CSV table", 63, table),
            ("CSV delimiter (one byte)", 1, options.delimiter),
            ("CSV quote (one byte)", 1, options.quote),
            ("CSV escape (one byte)", 1, options.escape),
            ("CSV NULL token (64 characters)", 256, options.null_token),
        ] {
            let field = cx.new(|cx| Field::new(label, limit, false, value, window, cx));
            self.field_events.push(cx.subscribe_in(
                &field,
                window,
                |this, _, _: &crate::bounded_field::Changed, window, cx| {
                    this.changed(cx);
                    if let Err(error) = this.reconcile_schema(window, cx) {
                        this.status = error.into();
                    }
                    cx.notify();
                },
            ));
            self.fields.push(field);
        }
        self.freeze_fields(cx);
    }
    fn setup_enabled(&self, cx: &gpui::App) -> bool {
        self.editable
            && !self.picking
            && self.field_seed.is_none()
            && self.setup.is_some()
            && !self.store.read(cx).busy()
            && !self.store.read(cx).capture().is_some_and(|capture| {
                self.connection.as_ref().is_some_and(|connection| {
                    capture.active_on(connection) || capture.unknown_import_on(connection)
                })
            })
    }
    fn freeze_fields(&mut self, cx: &mut Context<Self>) {
        let readonly = !self.setup_enabled(cx);
        let xlsx = self.setup.as_ref().is_some_and(Setup::xlsx);
        for (index, field) in self.fields.iter().enumerate() {
            field.update(cx, |field, cx| {
                field.set_readonly(readonly || (xlsx && (2..5).contains(&index)), cx)
            });
        }
    }
    fn field_value(&self, index: usize, cx: &gpui::App) -> Result<String, &'static str> {
        let mut value = self
            .fields
            .get(index)
            .ok_or("CSV field allowance unavailable")?
            .read(cx)
            .value(cx)?;
        value.shrink_to_fit();
        Ok(value)
    }
    fn reconcile_schema(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        if self.fields.len() != 6 {
            return Ok(());
        }
        if self.fields[0].update(cx, |field, cx| field.composing(window, cx)) {
            return Err("Finish schema composition before changing the CSV target");
        }
        let schema = self.field_value(0, cx)?;
        if schema != self.schema_seen {
            self.fields[1].update(cx, |field, cx| field.set_value(String::new(), window, cx))?;
            self.schema_seen = schema;
            self.metadata.filter_revision = self.metadata.filter_revision.wrapping_add(1);
            self.metadata.selected = None;
            self.filter_choices();
        }
        Ok(())
    }
    fn sync_fields(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        self.reconcile_schema(window, cx)?;
        let target = CsvTarget {
            schema: self.field_value(0, cx)?,
            table: self.field_value(1, cx)?,
        };
        let options = if self.setup.as_ref().is_some_and(Setup::xlsx) {
            CsvOptions {
                null_token: self.field_value(5, cx)?,
                ..CsvOptions::default()
            }
        } else {
            CsvOptions {
                delimiter: self.field_value(2, cx)?,
                quote: self.field_value(3, cx)?,
                escape: self.field_value(4, cx)?,
                null_token: self.field_value(5, cx)?,
                header: self
                    .setup
                    .as_ref()
                    .is_some_and(|setup| setup.options().header),
            }
        };
        let setup = self.setup.as_mut().ok_or("Choose a connection")?;
        setup.set_target(Some(target))?;
        setup.set_options(options)
    }
    fn inspection_data<'a>(&self, cx: &'a gpui::App) -> Option<&'a CsvInspectionData> {
        let (id, token) = self.inspection.as_ref()?;
        let data = self.store.read(cx).inspection()?;
        (data.inspection_id == *id
            && self.setup.as_ref()?.matches_inspection(token, data)
            && data
                .workbook
                .as_ref()
                .is_none_or(|source| Some(source.sheet_index) == self.sheet))
        .then_some(data)
    }
    fn sync_inspection(&mut self, cx: &mut Context<Self>) {
        if self.mapping.is_none()
            && let Some(data) = self.inspection_data(cx)
            && data.direction == CsvDirection::Import
        {
            match Mapping::new(data, self.budget.clone()) {
                Ok(mapping) => self.mapping = Some(mapping),
                Err(error) => self.status = error.into(),
            }
        }
    }
    fn selected_row<'a>(&self, cx: &'a gpui::App) -> Option<&'a CsvTransferObservation> {
        let capture = self.store.read(cx).capture()?;
        let row = capture.row(capture.index_for_key(self.selected?)?)?;
        (Some(row.connection_id.as_str()) == self.connection.as_deref()).then_some(row)
    }
    fn current_review<'a>(&self, cx: &'a gpui::App) -> Option<&'a CsvTransferReview> {
        let store = self.store.read(cx);
        let review = store
            .confirmation()
            .map(|value| value.review())
            .or_else(|| store.review())?;
        if Some(review.inspection().data().connection_id.as_str()) != self.connection.as_deref() {
            return None;
        }
        if let Some(attempt) = review.attempt_id() {
            return (self.selected == Some(attempt)).then_some(review);
        }
        let (id, token) = self.inspection.as_ref()?;
        (review.inspection().inspection_id() == *id
            && review
                .inspection()
                .data()
                .workbook
                .as_ref()
                .is_none_or(|source| Some(source.sheet_index) == self.sheet)
            && self
                .setup
                .as_ref()?
                .matches_inspection(token, review.inspection().data()))
        .then_some(review)
    }
    fn enabled(&self, action: Action, cx: &gpui::App) -> bool {
        if !self.editable {
            return false;
        }
        let store = self.store.read(cx);
        match action {
            Action::Header | Action::Delimiter(_)
                if self.setup.as_ref().is_some_and(Setup::xlsx) =>
            {
                false
            }
            Action::LoadWorkbook => self.can_load_workbook(cx),
            Action::SelectSheet => {
                self.setup_enabled(cx)
                    && self.workbook_data(cx).is_some_and(|book| {
                        book.sheets
                            .iter()
                            .any(|sheet| Some(sheet.index) == self.sheet)
                    })
            }
            Action::Refresh => !store.busy(),
            Action::Clear => !self.picking,
            Action::LoadChoices => self.setup_enabled(cx) && !self.metadata.busy(),
            Action::Schemas | Action::Tables => {
                self.setup_enabled(cx) && self.metadata.capture.is_some()
            }
            Action::UseChoice => self.setup_enabled(cx) && self.metadata.selected.is_some(),
            Action::CancelChoices => self.metadata.busy(),
            Action::LoadInspection => {
                !store.busy()
                    && self.inspection.as_ref().is_some_and(|(id, _)| {
                        store
                            .capture()
                            .and_then(|capture| capture.inspection(*id))
                            .is_some_and(|row| row.phase == CsvInspectionPhase::Ready)
                    })
            }
            Action::ReleaseInspection => !store.busy() && self.inspection.is_some(),
            Action::Inspect => {
                self.setup_enabled(cx) && store.observation_current() && self.fields.len() == 6
            }
            Action::Review => {
                self.setup_enabled(cx)
                    && self.inspection_data(cx).is_some()
                    && self
                        .setup
                        .as_ref()
                        .is_some_and(|setup| setup.path().is_some())
            }
            Action::Begin => {
                !store.busy() && store.review().is_some() && self.current_review(cx).is_some()
            }
            Action::Confirm => {
                !store.busy()
                    && store.confirmation().is_some_and(|confirmation| {
                        Some(confirmation.attempt_id()) == self.selected
                    })
                    && self.current_review(cx).is_some()
            }
            Action::Reacquire => {
                !store.busy()
                    && self
                        .selected_row(cx)
                        .is_some_and(|row| row.phase == CsvTransferPhase::AwaitingConfirmation)
            }
            Action::Cancel => {
                !store.busy()
                    && self.selected_row(cx).is_some_and(|row| {
                        !row.phase.terminal() && row.phase != CsvTransferPhase::Cancelling
                    })
            }
            Action::Dismiss => {
                !store.busy() && self.selected_row(cx).is_some_and(model::releasable)
            }
            Action::Skip | Action::PreviousTarget | Action::NextTarget => {
                self.setup_enabled(cx)
                    && self.mapping_source.is_some()
                    && self.mapping.is_some()
                    && self.inspection_data(cx).is_some()
            }
            _ => self.setup_enabled(cx),
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action, cx) || self.composing(window, cx) {
            return;
        }
        let result = match action {
            Action::Import | Action::Export | Action::Xlsx => {
                self.abandon(cx);
                let direction = if matches!(action, Action::Import | Action::Xlsx) {
                    CsvDirection::Import
                } else {
                    CsvDirection::Export
                };
                // Direction changes clear the selected file, not uncommitted
                // dialect/target text still owned by the field editors.
                self.setup
                    .as_mut()
                    .unwrap()
                    .set_xlsx(matches!(action, Action::Xlsx))
                    .and_then(|()| self.setup.as_mut().unwrap().set_direction(direction))
            }
            Action::LoadWorkbook => {
                if let Some((id, _)) = self.inspection.as_ref() {
                    let id = *id;
                    self.store
                        .update(cx, |store, cx| store.load_workbook(id, cx));
                }
                Ok(())
            }
            Action::SelectSheet => self.inspect_sheet(cx),
            Action::Delimiter(value) => {
                if let Some(field) = self.fields.get(2).cloned() {
                    self.changed(cx);
                    field.update(cx, |field, cx| field.set_value(value.into(), window, cx))
                } else {
                    Err("CSV field allowance unavailable")
                }
            }
            Action::Header => {
                self.changed(cx);
                let setup = self.setup.as_mut().unwrap();
                let mut options = setup.options().clone();
                options.header = !options.header;
                setup.set_options(options)
            }
            Action::Pick => {
                match self.sync_fields(window, cx) {
                    Ok(()) => self.pick(window, cx),
                    Err(error) => self.status = error.into(),
                };
                Ok(())
            }
            Action::Inspect => self.inspect(window, cx),
            Action::LoadInspection => {
                if let Some((id, _)) = &self.inspection {
                    let id = *id;
                    self.store
                        .update(cx, |store, cx| store.load_inspection(id, cx));
                }
                Ok(())
            }
            Action::ReleaseInspection => {
                self.abandon(cx);
                Ok(())
            }
            Action::Review => self.review(cx),
            Action::Begin => {
                if let Some(review) = self.current_review(cx) {
                    let inspection = review.inspection().inspection_id();
                    let attempt = review.attempt_id();
                    if let Some(id) = self.store.update(cx, |store, cx| {
                        store.begin_transfer(inspection, attempt, cx)
                    }) {
                        self.selected = Some(id);
                    }
                }
                Ok(())
            }
            Action::Refresh => {
                self.store.update(cx, |store, cx| store.refresh(cx));
                Ok(())
            }
            Action::Clear => {
                self.clear(cx);
                Ok(())
            }
            Action::LoadChoices => {
                self.load_choices(cx);
                Ok(())
            }
            Action::Schemas | Action::Tables => {
                self.metadata.tables = matches!(action, Action::Tables);
                self.metadata.filter_revision = self.metadata.filter_revision.wrapping_add(1);
                self.metadata.selected = None;
                self.filter_choices();
                Ok(())
            }
            Action::UseChoice => {
                self.use_choice(window, cx);
                Ok(())
            }
            Action::CancelChoices => {
                self.stop_choices();
                Ok(())
            }
            Action::Skip | Action::PreviousTarget | Action::NextTarget => {
                self.change_mapping(action, cx)
            }
            Action::Dismiss
                if self.selected_row(cx).is_some_and(|row| {
                    row.direction == CsvDirection::Import && row.effect == CsvEffect::Unknown
                }) && self.unknown_ack != self.selected =>
            {
                self.unknown_ack = self.selected;
                self.status="Inspect the target in a SQL session before acknowledging this unknown import. Dismissal does not establish success or rollback.".into();
                Ok(())
            }
            _ => {
                if let Some(id) = self.selected {
                    self.store.update(cx, |store, cx| match action {
                        Action::Confirm => store.confirm(id, cx),
                        Action::Reacquire => store.review_transfer(id, cx),
                        Action::Cancel => store.cancel(id, cx),
                        Action::Dismiss => store.release(id, cx),
                        _ => {}
                    });
                }
                Ok(())
            }
        };
        if let Err(error) = result {
            self.status = error.into();
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    fn inspect(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), &'static str> {
        self.sync_fields(window, cx)?;
        let setup = self.setup.as_ref().unwrap();
        let intent = setup.inspection_intent()?;
        let connection = setup.connection().to_owned();
        let token = setup.token();
        let owner = self.owner.as_ref().ok_or("CSV setup owner unavailable")?;
        if let Some(id) = self.store.update(cx, |store, cx| {
            store.begin_inspection(owner, connection, intent, cx)
        }) {
            self.inspection = Some((id, token));
            self.mapping = None;
            self.mapping_source = None;
            self.sheet = None;
            self.status = if self.setup.as_ref().is_some_and(Setup::xlsx) {
                "Workbook inspection requested. Load its sheets when ready, then select and inspect a sheet."
            } else {
                "Inspection requested. Load it when Ready, then review explicitly"
            }.into();
        }
        Ok(())
    }
    fn review(&mut self, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let data = self
            .inspection_data(cx)
            .ok_or("Load this setup's current inspection")?;
        let id = data.inspection_id;
        match data.direction {
            CsvDirection::Import => {
                let request = self
                    .mapping
                    .as_ref()
                    .ok_or("Mapping allowance unavailable")?
                    .to_backend(data)?;
                self.store
                    .update(cx, |store, cx| store.review_import(id, request, cx));
            }
            CsvDirection::Export => {
                let path = self
                    .setup
                    .as_ref()
                    .and_then(Setup::path)
                    .ok_or("Select a destination")?
                    .to_path_buf();
                self.store
                    .update(cx, |store, cx| store.review_export(id, path, cx));
            }
        }
        Ok(())
    }
    fn change_mapping(
        &mut self,
        action: Action,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let source = self.mapping_source.ok_or("Select a source column")?;
        let mut mapping = self.mapping.take().ok_or("Mapping unavailable")?;
        let result = (|| {
            let data = self.inspection_data(cx).ok_or("Inspection changed")?;
            let next = if matches!(action, Action::Skip) {
                None
            } else {
                mapping.adjacent_target(data, source, matches!(action, Action::NextTarget))?
            };
            mapping.set(data, source, next)
        })();
        self.mapping = Some(mapping);
        if result.is_ok()
            && let Some((id, _)) = self.inspection.as_ref()
        {
            let id = *id;
            self.store
                .update(cx, |store, cx| store.discard_setup_review(id, cx));
        }
        result
    }
    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let setup = self.setup.as_ref().unwrap();
        let token = setup.token();
        let direction = setup.direction();
        self.picking = true;
        self.freeze_fields(cx);
        let picker = if direction == CsvDirection::Export {
            let receiver = cx.prompt_for_new_path(std::path::Path::new("/tmp"), Some("export.csv"));
            futures_util::future::Either::Left(async move {
                receiver
                    .await
                    .map_err(|_| ())
                    .and_then(|result| result.map_err(|_| ()))
            })
        } else {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            });
            futures_util::future::Either::Right(async move {
                receiver
                    .await
                    .map_err(|_| ())
                    .and_then(|result| result.map_err(|_| ()))
                    .map(|paths| {
                        paths
                            .and_then(|mut paths| if paths.len() == 1 { paths.pop() } else { None })
                    })
            })
        };
        self.picker = Some(cx.spawn_in(window, async move |this, cx| {
            let result = picker.await;
            this.update(cx, |this, cx| {
                this.picking = false;
                match result {
                    Ok(Some(path)) => match this
                        .setup
                        .as_mut()
                        .ok_or("Setup changed")
                        .and_then(|setup| setup.accept_path(&token, path))
                    {
                        Ok(()) => {
                            this.abandon(cx);
                            this.status =
                                "File selected; inspect and review before starting".into();
                        }
                        Err(error) => this.status = error.into(),
                    },
                    Ok(None) => this.status = "File selection cancelled".into(),
                    Err(()) => this.status = "Native file picker unavailable".into(),
                }
                this.freeze_fields(cx);
                cx.notify();
            })
            .ok();
        }));
    }
    fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut order = vec![];
        for (index, (action, _)) in ACTIONS.iter().enumerate() {
            if self.enabled(*action, cx) {
                order.push(self.buttons[index].clone());
            }
        }
        let xlsx = self.setup.as_ref().is_some_and(Setup::xlsx);
        order.extend(
            self.fields
                .iter()
                .enumerate()
                .filter(|(index, _)| !xlsx || !(2..5).contains(index))
                .map(|(_, field)| field.focus_handle(cx)),
        );
        if self.workbook_data(cx).is_some() {
            order.push(self.sheet_focus.clone());
        }
        if self.metadata.capture.is_some() {
            order.push(self.choice_focus.clone());
        }
        if self
            .inspection_data(cx)
            .is_some_and(|data| data.direction == CsvDirection::Import)
        {
            order.push(self.mapping_focus.clone());
            order.push(self.sample_focus.clone());
        }
        if self
            .current_review(cx)
            .is_some_and(|review| !review.mapping().is_empty())
        {
            order.push(self.review_focus.clone());
        }
        order.push(self.list.clone());
        if self.selected_row(cx).is_some() {
            order.push(self.details.clone());
        }
        order
    }
}
impl Drop for CsvTransferView {
    fn drop(&mut self) {
        self.stop_choices();
        self.field_events.clear();
        self.fields.clear();
        self.field_lease = None;
    }
}
