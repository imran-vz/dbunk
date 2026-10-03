//! Setup views borrow one app-owned job observer. Dropping a view never stops a
//! prepared or running job, and file selection never authorizes execution.
use crate::{
    bounded_field::Field,
    controller::{Host, TableCommand, TableControls, TableMessage, TableReceiver},
    pg_tool_jobs::{self, Format, Operation, Scope, Setup},
    pg_tool_store::ToolStore,
};
use dbunk_lib::backend::{
    WorkspaceDocument,
    pg_tools::{PgToolAttemptId, PgToolPhase},
};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, PathPromptOptions,
    ScrollHandle, Subscription, Task, UniformListScrollHandle, Window, accesskit::Role, div,
    prelude::*, px,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

mod choices;
mod choices_view;
mod render;
pub struct ToolViewResources {
    pub host: Arc<Host>,
    pub store: Entity<ToolStore>,
    pub wake: async_channel::Sender<()>,
    pub retained: Rc<Cell<usize>>,
}
const FIELD_BYTES: usize = 1024 * 1024;
struct FieldLease(Rc<Cell<usize>>);
impl FieldLease {
    fn new(budget: Rc<Cell<usize>>) -> Option<Self> {
        if FIELD_BYTES > (128_usize * 1024 * 1024).saturating_sub(budget.get()) {
            return None;
        }
        budget.set(budget.get() + FIELD_BYTES);
        Some(Self(budget))
    }
}
impl Drop for FieldLease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(FIELD_BYTES));
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeChoice {
    Database,
    Schema,
    Table,
}
#[derive(Clone, Copy)]
enum Action {
    Backup,
    Restore,
    Plain,
    Custom,
    Database,
    Schema,
    Table,
    Clean,
    Browse,
    Prepare,
    Refresh,
    Review,
    Start,
    Confirm,
    Cancel,
    Dismiss,
    Clear,
    LoadChoices,
    SchemaChoices,
    TableChoices,
    UseChoice,
    ClearChoices,
    CancelChoices,
}
const ACTIONS: [(Action, &str); 23] = [
    (Action::Backup, "Backup"),
    (Action::Restore, "Restore"),
    (Action::Plain, "Plain SQL"),
    (Action::Custom, "Custom archive"),
    (Action::Database, "Database"),
    (Action::Schema, "Schema"),
    (Action::Table, "Table"),
    (Action::Clean, "Cleanup option"),
    (Action::Browse, "Choose file"),
    (Action::Prepare, "Prepare job"),
    (Action::Refresh, "Refresh jobs"),
    (Action::Review, "Review selected job"),
    (Action::Start, "Start reviewed job"),
    (Action::Confirm, "Confirm exact restore"),
    (Action::Cancel, "Cancel selected job"),
    (Action::Dismiss, "Dismiss finished job"),
    (Action::Clear, "Clear setup"),
    (Action::LoadChoices, "Load choices"),
    (Action::SchemaChoices, "Schema choices"),
    (Action::TableChoices, "Table choices"),
    (Action::UseChoice, "Use selected choice"),
    (Action::ClearChoices, "Clear choices"),
    (Action::CancelChoices, "Cancel choice read"),
];

pub struct PgToolView {
    host: Arc<Host>,
    id: String,
    wake: async_channel::Sender<()>,
    metadata: choices::Metadata,
    choice_focus: FocusHandle,
    choice_scroll: UniformListScrollHandle,
    schema_seen: String,
    schema_events: Option<Subscription>,
    unknown_ack: Option<PgToolAttemptId>,
    store: Entity<ToolStore>,
    _observer: Subscription,
    budget: Rc<Cell<usize>>,
    setup: Option<Setup>,
    connection: Option<String>,
    fields: Option<(Entity<Field>, Entity<Field>)>,
    lease: Option<FieldLease>,
    field_seed: Option<(String, String)>,
    scope: ScopeChoice,
    selected: Option<PgToolAttemptId>,
    editable: bool,
    picker: Option<Task<()>>,
    picking: bool,
    status: String,
    focus: FocusHandle,
    buttons: Vec<FocusHandle>,
    list: FocusHandle,
    details: FocusHandle,
    previous_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
}
impl PgToolView {
    pub fn new(
        resources: ToolViewResources,
        document: &WorkspaceDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ToolViewResources {
            host,
            store,
            wake,
            retained,
        } = resources;
        let observer = cx.observe(&store, |this, _, cx| {
            this.freeze_fields(cx);
            cx.notify();
        });
        let mut this = Self {
            host,
            id: document.id.clone(),
            wake,
            metadata: choices::Metadata::default(),
            choice_focus: cx.focus_handle(),
            choice_scroll: UniformListScrollHandle::new(),
            schema_seen: String::new(),
            schema_events: None,
            unknown_ack: None,
            store,
            _observer: observer,
            budget: retained,
            setup: None,
            connection: document.connection_id.clone(),
            fields: None,
            lease: None,
            field_seed: Some((String::new(), String::new())),
            scope: ScopeChoice::Database,
            selected: None,
            editable: true,
            picker: None,
            picking: false,
            status: "Choose a file, then prepare and review the exact job before starting".into(),
            focus: cx.focus_handle(),
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(),
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
            .filter(|focus| order.contains(focus))
            .unwrap_or(&self.list);
        window.focus(focus, cx);
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.contains_focused(window, cx) {
            self.previous_focus = window.focused(cx);
        }
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        if !editable {
            self.stop_choices();
        }
        // A connection/settings fence must also invalidate an outstanding picker.
        if !editable && let Some(setup) = &mut self.setup {
            if let Some(generation) = setup.generation().checked_add(1) {
                let connection = setup.connection().to_owned();
                if let Err(error) = setup.retarget(connection, generation) {
                    self.status = error.into();
                }
            } else {
                self.setup = None;
            }
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    pub fn bind_connection(&mut self, connection: String, cx: &mut Context<Self>) {
        if connection.len() > 256
            || connection.capacity() > 1024
            || connection.is_empty()
            || connection.chars().any(char::is_control)
        {
            self.status = "Invalid connection identity".into();
            cx.notify();
            return;
        }
        self.stop_choices();
        self.unknown_ack = None;
        self.setup = None;
        self.connection = Some(connection);
        self.scope = ScopeChoice::Database;
        self.selected = None;
        self.field_seed = Some((String::new(), String::new()));
        self.admit_setup();
        self.freeze_fields(cx);
        cx.notify();
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.picking {
            return;
        }
        self.setup = None;
        self.unknown_ack = None;
        self.scope = ScopeChoice::Database;
        self.field_seed = Some((String::new(), String::new()));
        self.admit_setup();
        self.status = "Setup cleared. Prepared and running jobs remain in session history".into();
        cx.notify();
    }
    pub fn set_context(
        &mut self,
        operation: Operation,
        context: Option<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        if context.as_ref().is_some_and(|(schema, table)| {
            schema.len() > 63 || table.len() > 63 || schema.capacity() > 63 || table.capacity() > 63
        }) {
            self.status = "Contextual table identity exceeds supported bounds".into();
            cx.notify();
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        match Setup::new(
            connection,
            0,
            operation,
            context.clone(),
            self.budget.clone(),
        ) {
            Ok(setup) => {
                self.setup = Some(setup);
                self.scope = if context.is_some() {
                    ScopeChoice::Table
                } else {
                    ScopeChoice::Database
                };
                self.field_seed = Some(context.unwrap_or_default());
                self.status = if operation == Operation::Restore {
                    "Restore targets the entire database, including when opened from a table"
                } else if self.scope == ScopeChoice::Table {
                    "Backup scope captured from this table"
                } else {
                    "Backup targets the selected database"
                }
                .into();
            }
            Err(error) => self.status = error.into(),
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    fn admit_setup(&mut self) {
        if self.setup.is_none()
            && let Some(connection) = self.connection.clone()
        {
            match Setup::new(connection, 0, Operation::Backup, None, self.budget.clone()) {
                Ok(setup) => self.setup = Some(setup),
                Err(error) => self.status = error.into(),
            }
        }
    }
    fn rebuild_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.field_seed.is_none() && self.fields.is_some() {
            return;
        }
        if self.composing(window, cx) {
            return;
        }
        if self.lease.is_none() {
            self.lease = FieldLease::new(self.budget.clone());
        }
        if self.lease.is_none() {
            self.status = "Schema/table fields need 1 MiB of shared allowance; clear captures, then Clear setup to retry".into();
            return;
        }
        let (schema, table) = self.field_seed.take().unwrap_or_default();
        self.schema_seen = schema.clone();
        self.metadata.filter_revision = self.metadata.filter_revision.wrapping_add(1);
        self.metadata.selected = None;
        if let Some(capture) = &mut self.metadata.capture {
            capture.filter(self.metadata.kind, &schema);
        }
        self.schema_events = None;
        self.fields = None;
        self.fields = Some((
            cx.new(|cx| Field::new("Backup schema", 63, false, schema, window, cx)),
            cx.new(|cx| Field::new("Backup table", 63, false, table, window, cx)),
        ));
        if let Some((schema, _)) = &self.fields {
            self.schema_events = Some(cx.subscribe_in(
                schema,
                window,
                |this, _, _: &crate::bounded_field::Changed, window, cx| {
                    if let Err(error) = this.reconcile_schema(window, cx) {
                        this.status = error.into();
                    }
                    cx.notify();
                },
            ));
        }
        self.freeze_fields(cx);
    }
    fn setup_enabled(&self, cx: &gpui::App) -> bool {
        self.editable
            && !self.picking
            && self.field_seed.is_none()
            && !self.store.read(cx).busy()
            && self.setup.is_some()
            && !self.store.read(cx).capture().is_some_and(|capture| {
                self.connection
                    .as_ref()
                    .is_some_and(|connection| capture.active_on(connection))
            })
    }
    fn freeze_fields(&mut self, cx: &mut Context<Self>) {
        let readonly = !self.setup_enabled(cx);
        if let Some((schema, table)) = &self.fields {
            schema.update(cx, |field, cx| field.set_readonly(readonly, cx));
            table.update(cx, |field, cx| field.set_readonly(readonly, cx));
        }
    }
    fn visible_jobs(&self, cx: &gpui::App) -> Vec<usize> {
        self.store
            .read(cx)
            .capture()
            .map_or_else(Vec::new, |capture| {
                capture
                    .rows()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, job)| {
                        (Some(job.connection_id.as_str()) == self.connection.as_deref())
                            .then_some(index)
                    })
                    .collect()
            })
    }
    fn selected_row<'a>(
        &self,
        cx: &'a gpui::App,
    ) -> Option<&'a dbunk_lib::backend::pg_tools::PgToolObservation> {
        let capture = self.store.read(cx).capture()?;
        let row = capture.row(capture.index_for_key(self.selected?)?)?;
        (Some(row.connection_id.as_str()) == self.connection.as_deref()).then_some(row)
    }
    fn enabled(&self, action: Action, cx: &gpui::App) -> bool {
        if !self.editable {
            return false;
        }
        let store = self.store.read(cx);
        let backup = self
            .setup
            .as_ref()
            .is_some_and(|setup| setup.operation() == Operation::Backup);
        match action {
            Action::Refresh => !store.busy(),
            Action::Clear => !self.picking,
            Action::Review => {
                !store.busy()
                    && self.selected_row(cx).is_some_and(|job| {
                        matches!(
                            job.phase,
                            PgToolPhase::ReadyReview | PgToolPhase::AwaitingConfirmation
                        )
                    })
            }
            Action::Start => {
                !store.busy()
                    && store
                        .review()
                        .is_some_and(|review| Some(review.attempt_id()) == self.selected)
                    && self.selected_row(cx).is_some_and(|job| {
                        matches!(
                            job.phase,
                            PgToolPhase::ReadyReview | PgToolPhase::AwaitingConfirmation
                        )
                    })
            }
            Action::Confirm => {
                !store.busy()
                    && store.confirmation().is_some_and(|confirmation| {
                        Some(confirmation.review().attempt_id()) == self.selected
                    })
                    && self
                        .selected_row(cx)
                        .is_some_and(|job| job.phase == PgToolPhase::AwaitingConfirmation)
            }
            Action::Cancel => {
                !store.busy() && self.selected_row(cx).is_some_and(pg_tool_jobs::cancellable)
            }
            Action::Dismiss => {
                !store.busy() && self.selected_row(cx).is_some_and(pg_tool_jobs::releasable)
            }
            Action::Clean => {
                self.setup_enabled(cx) && self.setup.as_ref().is_some_and(Setup::clean_enabled)
            }
            Action::Prepare => {
                self.setup_enabled(cx)
                    && store.observation_current()
                    && !store.capture().is_some_and(|capture| {
                        capture.rows().iter().any(|job| {
                            Some(job.connection_id.as_str()) == self.connection.as_deref()
                                && choices::unknown_restore(job)
                        })
                    })
                    && self.fields.is_some()
                    && self
                        .setup
                        .as_ref()
                        .is_some_and(|setup| setup.path().is_some())
            }
            Action::Database | Action::Schema | Action::Table => {
                self.setup_enabled(cx)
                    && self
                        .setup
                        .as_ref()
                        .is_some_and(|setup| setup.operation() == Operation::Backup)
            }
            Action::LoadChoices => backup && self.setup_enabled(cx) && !self.metadata.busy(),
            Action::SchemaChoices => {
                backup
                    && self.scope != ScopeChoice::Database
                    && self.setup_enabled(cx)
                    && self.metadata.capture.is_some()
            }
            Action::TableChoices => {
                backup
                    && self.scope == ScopeChoice::Table
                    && self.setup_enabled(cx)
                    && self.metadata.capture.is_some()
            }
            Action::UseChoice => {
                backup
                    && self.scope != ScopeChoice::Database
                    && (self.metadata.kind != choices::ChoiceKind::Table
                        || self.scope == ScopeChoice::Table)
                    && self.setup_enabled(cx)
                    && self.metadata.selected.is_some()
            }
            Action::ClearChoices => backup && self.metadata.capture.is_some(),
            Action::CancelChoices => backup && self.metadata.busy() && !self.metadata.cancelling,
            _ => self.setup_enabled(cx),
        }
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields.as_ref().is_some_and(|(schema, table)| {
            [schema, table]
                .into_iter()
                .any(|field| field.update(cx, |field, cx| field.composing(window, cx)))
        })
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action, cx) || self.composing(window, cx) {
            return;
        }
        let mut result = Ok(());
        match action {
            Action::Backup | Action::Restore => {
                result = self.setup.as_mut().unwrap().set_operation(
                    if matches!(action, Action::Backup) {
                        Operation::Backup
                    } else {
                        Operation::Restore
                    },
                );
                if matches!(action, Action::Restore) && result.is_ok() {
                    self.stop_choices();
                }
            }
            Action::Plain | Action::Custom => {
                result =
                    self.setup
                        .as_mut()
                        .unwrap()
                        .set_format(if matches!(action, Action::Plain) {
                            Format::Plain
                        } else {
                            Format::Custom
                        })
            }
            Action::Database | Action::Schema | Action::Table => {
                let next = match action {
                    Action::Schema => ScopeChoice::Schema,
                    Action::Table => ScopeChoice::Table,
                    _ => ScopeChoice::Database,
                };
                if self.scope != next {
                    self.scope = next;
                    self.field_seed = Some((String::new(), String::new()));
                }
            }
            Action::Clean => {
                let setup = self.setup.as_mut().unwrap();
                result = setup.set_clean(!setup.clean());
            }
            Action::Browse => self.pick(window, cx),
            Action::Prepare => {
                result = self
                    .reconcile_schema(window, cx)
                    .and_then(|()| self.prepare(cx))
            }
            Action::Refresh => self.store.update(cx, |store, cx| store.refresh(cx)),
            Action::Clear => self.clear(cx),
            Action::LoadChoices => self.load_choices(cx),
            Action::ClearChoices => {
                self.metadata.capture = None;
                self.metadata.selected = None;
            }
            Action::SchemaChoices | Action::TableChoices => {
                result = self.reconcile_schema(window, cx);
                if result.is_ok() {
                    self.metadata.kind = if matches!(action, Action::SchemaChoices) {
                        choices::ChoiceKind::Schema
                    } else {
                        choices::ChoiceKind::Table
                    };
                    self.metadata.selected = None;
                    self.metadata.filter_revision = self.metadata.filter_revision.wrapping_add(1);
                    if let Some(capture) = &mut self.metadata.capture {
                        capture.filter(self.metadata.kind, &self.schema_seen);
                    }
                }
            }
            Action::UseChoice => {
                if let Some(index) = self.metadata.selected {
                    self.choose(
                        self.metadata.revision,
                        self.metadata.filter_revision,
                        index,
                        window,
                        cx,
                    );
                }
            }
            Action::CancelChoices => {
                self.metadata.cancelling = true;
                if let Some(controls) = &self.metadata.controls {
                    controls.stop();
                }
                self.status = "Cancelling the owned choice reader; jobs are unaffected".into();
            }
            Action::Dismiss
                if self.selected_row(cx).is_some_and(choices::unknown_restore)
                    && self.unknown_ack != self.selected =>
            {
                self.unknown_ack = self.selected;
                self.status="Inspect the database in a SQL session before acknowledging this unknown restore outcome. Dismissal removes only its session record; it does not establish success or rollback.".into();
            }
            _ => {
                if let Some(attempt) = self.selected {
                    self.store.update(cx, |store, cx| match action {
                        Action::Review => store.review_job(attempt, cx),
                        Action::Start => store.start(attempt, cx),
                        Action::Confirm => store.confirm(attempt, cx),
                        Action::Cancel => store.cancel(attempt, cx),
                        Action::Dismiss => store.release(attempt, cx),
                        _ => {}
                    });
                }
            }
        }
        if let Err(error) = result {
            self.status = error.into();
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    fn prepare(&mut self, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let setup = self.setup.as_mut().ok_or("Select a connection")?;
        if setup.operation() == Operation::Backup {
            let (schema, table) = self.fields.as_ref().ok_or("Field allowance unavailable")?;
            let value = |field: &Entity<Field>| -> Result<String, &'static str> {
                let mut value = field.read(cx).value(cx)?;
                value.shrink_to_fit();
                Ok(value)
            };
            let scope = match self.scope {
                ScopeChoice::Database => Scope::Database,
                ScopeChoice::Schema => Scope::Schema {
                    schema: value(schema)?,
                },
                ScopeChoice::Table => Scope::Table {
                    schema: value(schema)?,
                    table: value(table)?,
                },
            };
            setup.set_scope(scope)?;
        }
        let intent = setup.intent()?;
        let connection = setup.connection().to_owned();
        if let Some(attempt) = self
            .store
            .update(cx, |store, cx| store.begin(connection, intent, cx))
        {
            self.selected = Some(attempt);
            self.status =
                "Preparation submitted. Review the observed exact target before starting".into();
        }
        Ok(())
    }
    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let setup = self.setup.as_ref().unwrap();
        let token = setup.token();
        let operation = setup.operation();
        let format = setup.format();
        self.picking = true;
        self.freeze_fields(cx);
        let picker = if operation == Operation::Backup {
            let name = if format == Format::Plain {
                "backup.sql"
            } else {
                "backup.dump"
            };
            let receiver = cx.prompt_for_new_path(std::path::Path::new("/tmp"), Some(name));
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
                            this.status =
                                "File selected. Selection does not authorize execution".into()
                        }
                        Err(error) => this.status = error.into(),
                    },
                    Ok(None) => this.status = "File selection cancelled".into(),
                    Err(()) => this.status = "Unable to open the native file picker".into(),
                }
                this.freeze_fields(cx);
                cx.notify();
            })
            .ok();
        }));
    }
    fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut order = ACTIONS
            .iter()
            .enumerate()
            .filter(|(index, (action, _))| *index < 8 && self.enabled(*action, cx))
            .map(|(index, _)| self.buttons[index].clone())
            .collect::<Vec<_>>();
        if self
            .setup
            .as_ref()
            .is_some_and(|setup| setup.operation() == Operation::Backup)
            && self.scope != ScopeChoice::Database
            && let Some((schema, table)) = &self.fields
        {
            order.push(schema.focus_handle(cx));
            if self.scope == ScopeChoice::Table {
                order.push(table.focus_handle(cx));
            }
        }
        order.extend(
            (17..ACTIONS.len())
                .filter(|index| self.enabled(ACTIONS[*index].0, cx))
                .map(|index| self.buttons[index].clone()),
        );
        if self
            .setup
            .as_ref()
            .is_some_and(|setup| setup.operation() == Operation::Backup)
            && self.metadata.capture.is_some()
        {
            order.push(self.choice_focus.clone());
        }
        order.extend(
            [8, 9, 16, 10, 11, 12, 13, 14, 15]
                .into_iter()
                .filter(|index| self.enabled(ACTIONS[*index].0, cx))
                .map(|index| self.buttons[index].clone()),
        );
        order.push(self.list.clone());
        if self.selected_row(cx).is_some() {
            order.push(self.details.clone());
        }
        order
    }
}
impl Drop for PgToolView {
    fn drop(&mut self) {
        self.stop_choices();
        self.schema_events = None;
        self.fields = None;
        self.lease = None;
    }
}
