//! Column recipes are editable setup, never executable authority. The shared
//! store owns exact reviewed attempts and recovery after this tab closes.
use crate::{
    bounded_field::Field,
    table_seed_model::{ColumnDraft, ColumnMode, GENERATORS, parse_row_count, parse_seed},
    table_seed_store::SeedStore,
};
use dbunk_lib::backend::{
    WorkspaceDocument, WorkspaceTableSeed, WorkspaceTableSeedState, table_seed::*,
};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollHandle,
    ScrollStrategy, Subscription, UniformListScrollHandle, Window, accesskit::Role, div,
    prelude::*, px, uniform_list,
};
use std::{cell::Cell, rc::Rc};
mod actions;
mod choices;
mod keyboard;
mod recipe;
mod render;
#[cfg(test)]
mod tests;
use choices::{Connections, Lease, valid_name};
use recipe::Recipe;
gpui::actions!(table_seed, [NextControl, PreviousControl]);
const MIB: usize = 1024 * 1024;
// Editors, compact column metadata, recipe replacement and bounded spec build.
// No generated rows or opaque backend preparation ownership is retained here.
const SETUP_BYTES: usize = 3 * MIB;
const FIELDS: [(&str, usize, bool); 7] = [
    ("Destination schema", 63, false),
    ("Destination table", 63, false),
    ("Rows (1–1000000)", 7, false),
    ("Optional unsigned seed", 20, false),
    ("Constant value", 8192, true),
    ("Comma-separated values", 128 * 1024, true),
    ("NULL percentage (0–100, blank for default)", 32, false),
];
#[derive(Clone, Copy)]
enum Action {
    Connection,
    Load,
    Inspect,
    Prepare,
    Mode,
    SaveColumn,
    Refresh,
    Review,
    Apply,
    Cancel,
    Reconcile,
    Dismiss,
}
const ACTIONS: [(Action, &str); 12] = [
    (Action::Connection, "Choose destination connection"),
    (Action::Load, "Load columns with Auto recipe"),
    (Action::Inspect, "Edit selected recipe"),
    (Action::Prepare, "Prepare edited recipe"),
    (Action::Mode, "Choose selected column source"),
    (Action::SaveColumn, "Save selected column"),
    (Action::Refresh, "Refresh seed jobs"),
    (Action::Review, "Review selected seed"),
    (Action::Apply, "Apply reviewed seed"),
    (Action::Cancel, "Cancel selected seed"),
    (Action::Reconcile, "Reconcile uncertain seed"),
    (Action::Dismiss, "Dismiss finished seed"),
];
pub struct TableSeedView {
    store: Entity<SeedStore>,
    _observer: Subscription,
    budget: Rc<Cell<usize>>,
    fields: Vec<Entity<Field>>,
    field_events: Vec<Subscription>,
    _setup_lease: Option<Lease>,
    preset: [String; 2],
    pending_preset: bool,
    connections: Option<Connections>,
    connections_current: bool,
    connection_revision: u64,
    connection: Option<String>,
    choosing: bool,
    choice: Option<usize>,
    selected: Option<TableSeedAttemptId>,
    recipe: Option<Recipe>,
    setup_anchor: Option<recipe::SetupAnchor>,
    column: Option<usize>,
    review_column: Option<usize>,
    review_focus: FocusHandle,
    review_scroll: UniformListScrollHandle,
    mode: ColumnMode,
    choosing_mode: bool,
    mode_choice: Option<usize>,
    column_dirty: bool,
    armed: Option<TableSeedAttemptId>,
    editable: bool,
    status: String,
    root: FocusHandle,
    buttons: Vec<FocusHandle>,
    job_focus: FocusHandle,
    choice_focus: FocusHandle,
    column_focus: FocusHandle,
    mode_focus: FocusHandle,
    details_focus: FocusHandle,
    previous: Option<FocusHandle>,
    jobs_scroll: UniformListScrollHandle,
    choices_scroll: UniformListScrollHandle,
    columns_scroll: UniformListScrollHandle,
    modes_scroll: UniformListScrollHandle,
    scroll: ScrollHandle,
}
impl TableSeedView {
    pub fn new(
        store: Entity<SeedStore>,
        retained: Rc<Cell<usize>>,
        document: &WorkspaceDocument,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observer = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store, _observer: observer, budget: retained, fields: vec![], field_events: vec![],
            _setup_lease: None, preset: Default::default(), pending_preset: false,
            connections: None, connections_current: false, connection_revision: 0,
            connection: document.connection_id.clone(), choosing: false, choice: None,
            selected: None, recipe: None, setup_anchor: None, column: None, review_column: None, review_focus: cx.focus_handle(), review_scroll: UniformListScrollHandle::new(), mode: ColumnMode::Auto,
            choosing_mode: false, mode_choice: None, column_dirty: false, armed: None,
            editable: true, status: "Load columns, edit their sources, then prepare and review the exact recipe. Rows change only after saved admission.".into(),
            root: cx.focus_handle(), buttons: ACTIONS.iter().map(|_|cx.focus_handle()).collect(),
            job_focus: cx.focus_handle(), choice_focus: cx.focus_handle(), column_focus: cx.focus_handle(),
            mode_focus: cx.focus_handle(), details_focus: cx.focus_handle(), previous: None,
            jobs_scroll: UniformListScrollHandle::new(), choices_scroll: UniformListScrollHandle::new(),
            columns_scroll: UniformListScrollHandle::new(), modes_scroll: UniformListScrollHandle::new(),
            scroll: ScrollHandle::new(),
        }
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn focus_document(&self, window: &mut Window, cx: &mut Context<Self>) {
        let handles = self.focus_order(cx);
        window.focus(
            self.previous
                .as_ref()
                .filter(|old| handles.contains(old))
                .unwrap_or(&self.root),
            cx,
        );
    }
    pub fn remember_focus(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.previous = self
            .focus_order(cx)
            .into_iter()
            .find(|handle| handle.is_focused(window));
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_readonly(!editable, cx));
        }
        cx.notify();
    }
    pub fn bind_connection(&mut self, connection: String, cx: &mut Context<Self>) {
        if self.connection.as_ref() != Some(&connection) {
            self.connection = Some(connection);
            self.preset = Default::default();
            self.pending_preset = true;
            self.choosing = false;
            self.status="Destination changed. Previous recipe remains visible but cannot prepare against a different table.".into();
            cx.notify();
        }
    }
    pub fn set_target(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        if valid_name(&schema) && valid_name(&table) {
            self.preset = [schema, table];
            self.pending_preset = true;
        } else {
            self.status = "Schema and table must each contain 1–63 UTF-8 bytes without NUL.".into();
        }
        cx.notify();
    }
    pub fn set_connections(
        &mut self,
        rows: &[dbunk_lib::backend::DevelopmentConnection],
        cx: &mut Context<Self>,
    ) {
        let Some(revision) = self.connection_revision.checked_add(1) else {
            self.connections_current = false;
            self.status = "Connection choice revision exhausted; reopen this tool.".into();
            cx.notify();
            return;
        };
        self.connection_revision = revision;
        match Connections::capture(
            rows.iter()
                .filter(|row| row.postgres.is_some())
                .map(|row| (row.id.as_str(), row.name.as_str())),
            self.budget.clone(),
        ) {
            Ok(choices) => {
                self.connections = Some(choices);
                self.connections_current = true;
                self.choosing = false;
                self.choice = None;
            }
            Err(error) => {
                self.connections_current = false;
                self.status = error.into();
            }
        }
        cx.notify();
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields
            .iter()
            .any(|field| field.update(cx, |field, cx| field.composing(window, cx)))
    }
    fn ensure_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fields.is_empty() {
            let Some(lease) = Lease::new(self.budget.clone(), SETUP_BYTES) else {
                self.status="Seed setup needs 3 MiB of shared allowance; release another retained result or tool.".into();
                return;
            };
            self._setup_lease = Some(lease);
            self.fields = FIELDS
                .iter()
                .enumerate()
                .map(|(index, (label, limit, multiline))| {
                    let value = match index {
                        0 | 1 => self.preset[index].clone(),
                        2 => "100".into(),
                        _ => String::new(),
                    };
                    cx.new(|cx| Field::new(label, *limit, *multiline, value, window, cx))
                })
                .collect();
            self.field_events = self
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    cx.subscribe(
                        field,
                        move |this, _, _: &crate::bounded_field::Changed, cx| {
                            if index >= 4 {
                                this.refresh_column_dirty(cx);
                            }
                            cx.notify();
                        },
                    )
                })
                .collect();
            self.pending_preset = false;
        }
        if self.pending_preset && !self.composing(window, cx) {
            for index in 0..2 {
                let value = self.preset[index].clone();
                self.fields[index]
                    .update(cx, |field, cx| field.set_value(value, window, cx))
                    .ok();
            }
            self.pending_preset = false;
        }
        for (index, field) in self.fields.iter().enumerate() {
            let readonly = !self.editable || (index >= 4 && self.column.is_none());
            field.update(cx, |field, cx| field.set_readonly(readonly, cx));
        }
    }
    fn refresh_column_dirty(&mut self, cx: &gpui::App) {
        self.column_dirty = self
            .recipe
            .as_ref()
            .and_then(|recipe| self.column.and_then(|index| recipe.drafts.get(index)))
            .is_some_and(|draft| {
                self.mode != draft.mode
                    || [
                        (&draft.constant, 4),
                        (&draft.values_text, 5),
                        (&draft.null_percent, 6),
                    ]
                    .iter()
                    .any(|(saved, index)| {
                        self.fields.get(*index).is_none_or(|field| {
                            field
                                .read(cx)
                                .value(cx)
                                .map_or(true, |value| &value != *saved)
                        })
                    })
            });
    }
    fn ids(&self, cx: &gpui::App) -> Vec<TableSeedAttemptId> {
        let store = self.store.read(cx);
        let mut ids: Vec<_> = store.journal().iter().map(|job| job.attempt_id).collect();
        for job in store.jobs() {
            if !ids.contains(&job.attempt_id) {
                ids.push(job.attempt_id);
            }
        }
        ids
    }
    fn observation<'a>(&self, cx: &'a gpui::App) -> Option<&'a TableSeedObservation> {
        let id = self.selected?;
        self.store
            .read(cx)
            .jobs()
            .iter()
            .find(|job| job.attempt_id == id)
    }
    fn journal<'a>(&self, cx: &'a gpui::App) -> Option<&'a WorkspaceTableSeed> {
        let id = self.selected?;
        self.store
            .read(cx)
            .journal()
            .iter()
            .find(|job| job.attempt_id == id)
    }
    fn connection_label(&self) -> String {
        self.connection.as_ref().map_or_else(
            || "Choose a connection".into(),
            |id| {
                self.connections
                    .as_ref()
                    .and_then(|choices| choices.rows.iter().find(|(key, _)| key == id))
                    .map_or_else(|| format!("Unavailable: {id}"), |(_, name)| name.clone())
            },
        )
    }
    fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut handles: Vec<_> = self
            .buttons
            .iter()
            .zip(ACTIONS)
            .filter(|(_, (action, _))| self.enabled(*action, cx))
            .map(|(handle, _)| handle.clone())
            .collect();
        handles.extend(
            self.fields
                .iter()
                .take(4)
                .map(|field| field.focus_handle(cx)),
        );
        if self.column.is_some() {
            handles.extend(
                self.fields
                    .iter()
                    .skip(4)
                    .filter(|field| self.field_visible(field, cx))
                    .map(|field| field.focus_handle(cx)),
            );
        }
        if self.choosing {
            handles.push(self.choice_focus.clone());
        }
        if !self.ids(cx).is_empty() {
            handles.push(self.job_focus.clone());
        }
        if self.recipe.is_some() {
            handles.push(self.column_focus.clone());
        }
        if self.choosing_mode {
            handles.push(self.mode_focus.clone());
        }
        if self
            .store
            .read(cx)
            .review_payload()
            .is_some_and(|review| Some(review.attempt_id()) == self.selected)
        {
            handles.push(self.review_focus.clone());
        }
        handles.push(self.details_focus.clone());
        handles
    }
    fn field_visible(&self, field: &Entity<Field>, _cx: &gpui::App) -> bool {
        let index = self
            .fields
            .iter()
            .position(|item| item == field)
            .unwrap_or(0);
        match index {
            4 => self.mode == ColumnMode::Constant,
            5 => self.mode == ColumnMode::Values,
            6 => {
                self.recipe
                    .as_ref()
                    .and_then(|recipe| self.column.and_then(|i| recipe.columns.get(i)))
                    .is_some_and(|column| column.nullable)
                    && self.mode != ColumnMode::Default
            }
            _ => true,
        }
    }
}
