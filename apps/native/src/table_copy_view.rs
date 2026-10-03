//! Editable setup is separate from immutable accepted attempts. The workspace
//! store owns execution and recovery after this tab closes.
use crate::{bounded_field::Field, table_copy_store::CopyStore};
use dbunk_lib::backend::{
    WorkspaceDocument, WorkspaceTableCopy, WorkspaceTableCopyState,
    table_copy::{
        TableCopyAttemptId, TableCopyCleanup, TableCopyColumnAction, TableCopyEndpoint,
        TableCopyIntent, TableCopyObservation, TableCopyPhase,
    },
};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollHandle,
    ScrollStrategy, Subscription, UniformListScrollHandle, Window, accesskit::Role, div,
    prelude::*, px, rgb, uniform_list,
};
use std::{cell::Cell, rc::Rc};
mod actions;
mod render;
#[cfg(test)]
mod tests;

const MIB: usize = 1024 * 1024;
const FIELD_LABELS: [&str; 4] = [
    "Source schema",
    "Source table",
    "Destination schema",
    "Destination table",
];
struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    fn new(budget: Rc<Cell<usize>>, bytes: usize) -> Option<Self> {
        if bytes > (128 * MIB).saturating_sub(budget.get()) {
            return None;
        }
        budget.set(budget.get() + bytes);
        Some(Self { budget, bytes })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}
struct Connections {
    rows: Vec<(String, String)>,
    _lease: Lease,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Source,
    Destination,
}
#[derive(Clone, Copy)]
enum Action {
    Source,
    Destination,
    Prepare,
    Refresh,
    Review,
    Apply,
    Cancel,
    Reconcile,
    Dismiss,
}
const ACTIONS: [(Action, &str); 9] = [
    (Action::Source, "Choose source connection"),
    (Action::Destination, "Choose destination connection"),
    (Action::Prepare, "Prepare table copy"),
    (Action::Refresh, "Refresh copy jobs"),
    (Action::Review, "Review selected copy"),
    (Action::Apply, "Apply reviewed copy"),
    (Action::Cancel, "Cancel selected copy"),
    (Action::Reconcile, "Reconcile unknown outcome"),
    (Action::Dismiss, "Dismiss finished copy"),
];

pub struct TableCopyView {
    store: Entity<CopyStore>,
    _observer: Subscription,
    budget: Rc<Cell<usize>>,
    fields: Vec<Entity<Field>>,
    field_events: Vec<Subscription>,
    field_lease: Option<Lease>,
    seeds: [String; 4],
    pending_seed: bool,
    connections: Option<Connections>,
    connections_current: bool,
    connection_revision: u64,
    source: Option<String>,
    destination: Option<String>,
    choosing: Option<Side>,
    choice: Option<usize>,
    selected: Option<TableCopyAttemptId>,
    column: Option<usize>,
    armed: Option<TableCopyAttemptId>,
    editable: bool,
    status: String,
    root: FocusHandle,
    buttons: Vec<FocusHandle>,
    job_focus: FocusHandle,
    choice_focus: FocusHandle,
    column_focus: FocusHandle,
    previous: Option<FocusHandle>,
    jobs_scroll: UniformListScrollHandle,
    choices_scroll: UniformListScrollHandle,
    columns_scroll: UniformListScrollHandle,
    details_scroll: ScrollHandle,
}
impl TableCopyView {
    pub fn new(
        store: Entity<CopyStore>,
        retained: Rc<Cell<usize>>,
        document: &WorkspaceDocument,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observer = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _observer: observer,
            budget: retained,
            fields: Vec::new(),
            field_events: Vec::new(),
            field_lease: None,
            seeds: Default::default(),
            pending_seed: false,
            connections: None,
            connections_current: false,
            connection_revision: 0,
            source: document.connection_id.clone(),
            destination: None,
            choosing: None,
            choice: None,
            selected: None,
            column: None,
            armed: None,
            editable: true,
            status: "Prepare reads both tables; destination rows change only after exact review and saved admission.".into(),
            root: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            job_focus: cx.focus_handle(),
            choice_focus: cx.focus_handle(),
            column_focus: cx.focus_handle(),
            previous: None,
            jobs_scroll: UniformListScrollHandle::new(),
            choices_scroll: UniformListScrollHandle::new(),
            columns_scroll: UniformListScrollHandle::new(),
            details_scroll: ScrollHandle::new(),
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
                .unwrap_or(&self.buttons[0]),
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
        if self.source.as_ref() != Some(&connection) {
            self.source = Some(connection);
            self.choosing = None;
            self.seeds[0].clear();
            self.seeds[1].clear();
            self.pending_seed = true;
            self.status = "Source connection changed; enter its schema and table.".into();
            cx.notify();
        }
    }
    pub fn set_source(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        if !valid_name(&schema) || !valid_name(&table) {
            self.status =
                "Source schema and table must each contain 1–63 UTF-8 bytes without NUL.".into();
        } else {
            self.seeds[0] = schema;
            self.seeds[1] = table;
            self.pending_seed = true;
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
                self.choosing = None;
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
            let Some(lease) = Lease::new(self.budget.clone(), MIB) else {
                self.status = "Table copy fields need 1 MiB of shared memory; clear a retained result or tool.".into();
                return;
            };
            self.field_lease = Some(lease);
            self.fields = FIELD_LABELS
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    cx.new(|cx| Field::new(label, 63, false, self.seeds[index].clone(), window, cx))
                })
                .collect();
            self.field_events = self
                .fields
                .iter()
                .map(|field| {
                    cx.subscribe(field, |_, _, _: &crate::bounded_field::Changed, cx| {
                        cx.notify()
                    })
                })
                .collect();
            self.pending_seed = false;
        }
        if self.pending_seed && !self.composing(window, cx) {
            for index in 0..2 {
                let value = self.seeds[index].clone();
                self.fields[index]
                    .update(cx, |field, cx| field.set_value(value, window, cx))
                    .ok();
            }
            self.pending_seed = false;
        }
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_readonly(!self.editable, cx));
        }
    }
    fn ids(&self, cx: &gpui::App) -> Vec<TableCopyAttemptId> {
        let store = self.store.read(cx);
        let mut ids: Vec<_> = store.journal().iter().map(|job| job.attempt_id).collect();
        for job in store.jobs() {
            if !ids.contains(&job.attempt_id) {
                ids.push(job.attempt_id);
            }
        }
        ids
    }
    fn observation<'a>(&self, cx: &'a gpui::App) -> Option<&'a TableCopyObservation> {
        let id = self.selected?;
        self.store
            .read(cx)
            .jobs()
            .iter()
            .find(|job| job.attempt_id == id)
    }
    fn journal<'a>(&self, cx: &'a gpui::App) -> Option<&'a WorkspaceTableCopy> {
        let id = self.selected?;
        self.store
            .read(cx)
            .journal()
            .iter()
            .find(|job| job.attempt_id == id)
    }
    fn connection_label(&self, id: Option<&str>) -> String {
        match id {
            None => "Choose a connection".into(),
            Some(id) => self
                .connections
                .as_ref()
                .and_then(|choices| choices.rows.iter().find(|(key, _)| key == id))
                .map_or_else(
                    || format!("Unavailable connection: {id}"),
                    |(_, name)| name.clone(),
                ),
        }
    }
    fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut handles: Vec<_> = self
            .buttons
            .iter()
            .zip(ACTIONS)
            .filter(|(_, (action, _))| self.enabled(*action, cx))
            .map(|(handle, _)| handle.clone())
            .collect();
        handles.extend(self.fields.iter().map(|field| field.focus_handle(cx)));
        if self.choosing.is_some() {
            handles.push(self.choice_focus.clone());
        }
        if !self.ids(cx).is_empty() {
            handles.push(self.job_focus.clone());
        }
        if self.store.read(cx).review_payload().is_some_and(|review| {
            Some(review.attempt_id()) == self.selected && !review.columns().is_empty()
        }) {
            handles.push(self.column_focus.clone());
        }
        handles
    }
}
impl Connections {
    /// Check borrowed metadata and reserve replacement overlap before cloning.
    fn capture<'a>(
        rows: impl Iterator<Item = (&'a str, &'a str)> + Clone,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let count = rows.clone().take(1025).count();
        if count > 1024
            || rows.clone().any(|(id, name)| {
                id.is_empty()
                    || id.len() > 128
                    || name.len() > 512
                    || id.contains('\0')
                    || name.contains('\0')
            })
        {
            return Err("Connection choices exceed their bounds; previous choices retained");
        }
        let lease = Lease::new(budget, MIB).ok_or(
            "Connection choices need 1 MiB of shared allowance; previous choices retained",
        )?;
        let mut captured = Vec::with_capacity(count);
        captured.extend(rows.map(|(id, name)| (id.to_owned(), name.to_owned())));
        if !valid_connections(&captured) {
            return Err("Connection choices are invalid; previous choices retained");
        }
        Ok(Self {
            rows: captured,
            _lease: lease,
        })
    }
}
fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}
fn valid_connections(rows: &Vec<(String, String)>) -> bool {
    if rows.len() > 1024 || rows.capacity() > 1024 {
        return false;
    }
    let mut bytes = rows
        .capacity()
        .saturating_mul(std::mem::size_of::<(String, String)>());
    for (index, (id, name)) in rows.iter().enumerate() {
        if id.is_empty()
            || id.len() > 128
            || id.capacity() > 256
            || name.len() > 512
            || name.capacity() > 1024
            || id.contains('\0')
            || name.contains('\0')
            || rows[..index].iter().any(|(other, _)| other == id)
        {
            return false;
        }
        bytes = bytes
            .saturating_add(id.capacity())
            .saturating_add(name.capacity());
    }
    bytes <= MIB
}
