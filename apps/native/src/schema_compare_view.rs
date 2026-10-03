//! Object inspector A in a Tool tab. Draft endpoints and immutable accepted
//! endpoints are separate; closing this view never cancels app-owned jobs.
use crate::{
    bounded_field::Field,
    schema_compare_model::{self as model, ConnectionChoice, Connections, Intent, Turn},
    schema_compare_reader::CompareReader,
    schema_compare_store::CompareStore,
};
use dbunk_lib::backend::{WorkspaceDocument, schema_comparisons::*};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollHandle, Subscription,
    UniformListScrollHandle, Window, accesskit::Role, div, prelude::*, px, rgb,
};
use std::{cell::Cell, rc::Rc};
mod render;
pub struct CompareViewResources {
    pub store: Entity<CompareStore>,
    pub reader: Entity<CompareReader>,
    pub retained: Rc<Cell<usize>>,
}
const FIELD_BYTES: usize = 1024 * 1024;
struct FieldLease(Rc<Cell<usize>>);
impl Drop for FieldLease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(FIELD_BYTES));
    }
}
#[derive(Clone, Copy)]
enum Action {
    Compare,
    Refresh,
    Cancel,
    Dismiss,
    Rerun,
    Retry,
    Clear,
    Coverage,
    PreviousObjects,
    NextObjects,
    PreviousFields,
    NextFields,
    PreviousSource,
    NextSource,
    PreviousTarget,
    NextTarget,
    Copy(Side),
}
const ACTIONS: [(Action, &str); 18] = [
    (Action::Compare, "Compare"),
    (Action::Refresh, "Refresh comparisons"),
    (Action::Cancel, "Cancel selected comparison"),
    (Action::Dismiss, "Dismiss selected comparison"),
    (Action::Rerun, "Run comparison again"),
    (Action::Retry, "Retry result read"),
    (Action::Clear, "Clear result view"),
    (Action::Coverage, "Coverage and capture"),
    (Action::PreviousObjects, "Previous objects"),
    (Action::NextObjects, "Next objects"),
    (Action::PreviousFields, "Previous fields"),
    (Action::NextFields, "Next fields"),
    (Action::PreviousSource, "Previous source chunk"),
    (Action::NextSource, "Next source chunk"),
    (Action::PreviousTarget, "Previous target chunk"),
    (Action::NextTarget, "Next target chunk"),
    (Action::Copy(Side::Source), "Copy displayed source chunk"),
    (Action::Copy(Side::Target), "Copy displayed target chunk"),
];
#[derive(Clone, PartialEq, Eq)]
struct CopyKey {
    request: ResultRequest,
    value: ValueRef,
    offset: u32,
}
pub struct SchemaCompareView {
    store: Entity<CompareStore>,
    reader: Entity<CompareReader>,
    _store_observer: Subscription,
    _reader_observer: Subscription,
    budget: Rc<Cell<usize>>,
    connections: Option<Connections>,
    source: Option<String>,
    target: Option<String>,
    generation: u64,
    fields: Vec<Entity<Field>>,
    field_lease: Option<FieldLease>,
    seed: [Option<String>; 2],
    selected: Option<String>,
    selected_request: Option<String>,
    editable: bool,
    coverage: bool,
    status: String,
    focus: FocusHandle,
    connection_focus: [FocusHandle; 2],
    schema_focus: [FocusHandle; 2],
    connection_scroll: [UniformListScrollHandle; 2],
    schema_scroll: [UniformListScrollHandle; 2],
    buttons: Vec<FocusHandle>,
    objects_focus: FocusHandle,
    fields_focus: FocusHandle,
    value_focus: [FocusHandle; 2],
    jobs_focus: FocusHandle,
    coverage_focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
    object_scroll: UniformListScrollHandle,
    field_scroll: UniformListScrollHandle,
}
impl SchemaCompareView {
    pub fn new(
        resources: CompareViewResources,
        document: &WorkspaceDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let CompareViewResources {
            store,
            reader,
            retained,
        } = resources;
        let store_observer = cx.observe(&store, |this, _, cx| {
            this.sync_selection(cx);
            this.freeze_fields(cx);
            cx.notify();
        });
        let reader_observer = cx.observe(&reader, |_, _, cx| cx.notify());
        let mut this = Self {
            store,
            reader,
            _store_observer: store_observer,
            _reader_observer: reader_observer,
            budget: retained,
            connections: None,
            source: document.connection_id.clone(),
            target: None,
            generation: 0,
            fields: vec![],
            field_lease: None,
            seed: [None, None],
            selected: None,
            selected_request: None,
            editable: true,
            coverage: false,
            status: "Choose explicit source and target schemas. Read-only PostgreSQL 16 ordinary-table comparison.".into(),
            focus: cx.focus_handle(),
            connection_focus: [cx.focus_handle(), cx.focus_handle()],
            schema_focus: [cx.focus_handle(), cx.focus_handle()],
            connection_scroll: [UniformListScrollHandle::new(), UniformListScrollHandle::new()],
            schema_scroll: [UniformListScrollHandle::new(), UniformListScrollHandle::new()],
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(),
            objects_focus: cx.focus_handle(),
            fields_focus: cx.focus_handle(),
            value_focus: [cx.focus_handle(), cx.focus_handle()],
            jobs_focus: cx.focus_handle(),
            coverage_focus: cx.focus_handle(),
            previous_focus: None,
            scroll: ScrollHandle::new(),
            object_scroll: UniformListScrollHandle::new(),
            field_scroll: UniformListScrollHandle::new(),
        };
        this.prepare_fields(window, cx);
        this
    }
    pub fn retained_budget(&self) -> Rc<Cell<usize>> {
        self.budget.clone()
    }
    pub fn status(&self) -> &str {
        &self.status
    }
    pub fn set_connections(&mut self, connections: Vec<ConnectionChoice>, cx: &mut Context<Self>) {
        match Connections::new(connections, self.budget.clone()) {
            Ok(connections) => {
                self.connections = Some(connections);
                self.generation = self.generation.wrapping_add(1);
                self.sync_selection(cx);
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    pub fn reject_connections(&mut self, reason: &str, cx: &mut Context<Self>) {
        self.status = format!(
            "{}. Previous cached choices retained; no new choices were loaded.",
            reason.chars().take(1024).collect::<String>()
        );
        cx.notify();
    }
    pub fn bind_connection(&mut self, connection: String, cx: &mut Context<Self>) {
        if connection.is_empty()
            || connection.len() > 128
            || connection.capacity() > 256
            || connection.contains('\0')
        {
            self.status = "Comparison source connection identity is invalid".into();
            cx.notify();
            return;
        }
        if self.source.as_deref() != Some(connection.as_str()) {
            self.source = Some(connection);
            self.seed[0] = Some(String::new());
            self.generation = self.generation.wrapping_add(1);
        }
        cx.notify();
    }
    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        self.editable = editable;
        if !editable {
            self.generation = self.generation.wrapping_add(1);
            self.reader.update(cx, |reader, cx| reader.close(cx));
        } else {
            self.sync_selection(cx);
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.selected_request = None;
        self.reader.update(cx, |reader, cx| reader.close(cx));
        self.status =
            "Result view cleared. Accepted comparison jobs remain in session history.".into();
        cx.notify();
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        self.reader.update(cx, |reader, cx| reader.drain_one(cx))
    }
    pub fn has_pending(&self, cx: &gpui::App) -> bool {
        self.reader.read(cx).has_pending()
    }
    pub fn focus_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.focus_order(cx);
        let focus = self
            .previous_focus
            .as_ref()
            .filter(|focus| order.contains(focus))
            .unwrap_or(&self.jobs_focus);
        window.focus(focus, cx);
    }
    pub fn remember_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.contains_focused(window, cx) {
            self.previous_focus = window.focused(cx);
        }
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields
            .iter()
            .any(|field| field.update(cx, |field, cx| field.composing(window, cx)))
    }
    fn prepare_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        if self.field_lease.is_none() {
            if FIELD_BYTES > (128usize * 1024 * 1024).saturating_sub(self.budget.get()) {
                self.status = "Comparison schema fields need 1 MiB of shared allowance".into();
                return;
            }
            self.budget.set(self.budget.get() + FIELD_BYTES);
            self.field_lease = Some(FieldLease(self.budget.clone()));
        }
        if self.fields.is_empty() {
            self.fields.push(
                cx.new(|cx| {
                    Field::new("Exact source schema", 63, false, String::new(), window, cx)
                }),
            );
            self.fields.push(
                cx.new(|cx| {
                    Field::new("Exact target schema", 63, false, String::new(), window, cx)
                }),
            );
        }
        for (index, field) in self.fields.iter().enumerate() {
            if let Some(value) = self.seed[index].take()
                && let Err(error) = field.update(cx, |field, cx| field.set_value(value, window, cx))
            {
                self.status = error.into();
            }
        }
        self.freeze_fields(cx);
    }
    fn setup_enabled(&self, cx: &gpui::App) -> bool {
        self.editable
            && !self.store.read(cx).busy()
            && !self.store.read(cx).uncertain()
            && self.seed.iter().all(Option::is_none)
    }
    fn freeze_fields(&self, cx: &mut Context<Self>) {
        let readonly = !self.setup_enabled(cx);
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_readonly(readonly, cx));
        }
    }
    fn select_connection(
        &mut self,
        side: Side,
        id: String,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation
            || !self.setup_enabled(cx)
            || self.composing(window, cx)
            || self
                .connections
                .as_ref()
                .and_then(|connections| connections.get(&id))
                .is_none()
        {
            return;
        }
        let index = side_index(side);
        let current = if index == 0 {
            &self.source
        } else {
            &self.target
        };
        if current.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(field) = self.fields.get(index) else {
            return;
        };
        if let Err(error) = field.update(cx, |field, cx| field.set_value(String::new(), window, cx))
        {
            self.status = error.into();
            cx.notify();
            return;
        }
        if index == 0 {
            self.source = Some(id);
        } else {
            self.target = Some(id);
        }
        self.generation = self.generation.wrapping_add(1);
        cx.notify();
    }
    fn endpoint(&self, side: Side, cx: &gpui::App) -> Result<Endpoint, &'static str> {
        let index = side_index(side);
        let id = if index == 0 {
            &self.source
        } else {
            &self.target
        };
        let connection_id = id
            .as_ref()
            .ok_or("Choose both stored PostgreSQL connections")?;
        if self
            .connections
            .as_ref()
            .and_then(|connections| connections.get(connection_id))
            .is_none()
        {
            return Err("An endpoint connection was removed; choose it explicitly");
        }
        let mut schema = self
            .fields
            .get(index)
            .ok_or("Schema field allowance unavailable")?
            .read(cx)
            .value(cx)?;
        schema.shrink_to_fit();
        if schema.is_empty() || schema.len() > 63 || schema.contains('\0') {
            return Err(
                "Schema names must be 1–63 UTF-8 bytes without NUL; names are never trimmed",
            );
        }
        Ok(Endpoint {
            connection_id: connection_id.clone(),
            schema,
        })
    }
    fn selected_job<'a>(&self, cx: &'a gpui::App) -> Option<&'a Status> {
        self.store
            .read(cx)
            .capture()?
            .row(self.selected.as_deref()?)
    }
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        if let Some(request) = &self.selected_request {
            if let Some(job) = store
                .capture()
                .and_then(|capture| capture.rows().iter().find(|job| &job.request_id == request))
            {
                self.selected = Some(job.job_id.clone());
                self.status = model::phase_label(&job.state).into();
                self.selected_request = None;
            } else if store.observation_current() && !store.busy() && !store.uncertain() {
                self.selected_request = None;
                self.status="No job for the requested admission was observed. Nothing was retried; Compare is an explicit new request.".into();
            }
        }
        if self.selected_request.is_none()
            && let Some(job) = self.selected_job(cx)
        {
            self.status = match &job.state {
                StatusState::Failed { failure } => model::job_failure_text(failure),
                _ => model::phase_label(&job.state).into(),
            };
        }
        if !self.editable {
            return;
        }
        let job = self.selected_job(cx);
        let request = job.and_then(|job| {
            if self.connections.as_ref().is_none_or(|connections| {
                connections.get(&job.source.connection_id).is_none()
                    || connections.get(&job.target.connection_id).is_none()
            }) {
                return None;
            }
            let StatusState::Completed { result_id } = &job.state else {
                return None;
            };
            Some(ResultRequest {
                identity: ResultIdentity {
                    job_id: job.job_id.clone(),
                    result_id: result_id.clone(),
                },
                source: job.source.clone(),
                target: job.target.clone(),
            })
        });
        let current = self
            .reader
            .read(cx)
            .state()
            .and_then(model::ReaderState::request);
        if request.as_ref() == current {
            return;
        }
        if let Some(request) = request {
            self.reader
                .update(cx, |reader, cx| reader.enqueue(Intent::Open(request), cx));
        } else if current.is_some() {
            self.reader.update(cx, |reader, cx| reader.close(cx));
        }
    }
    fn enabled(&self, action: Action, cx: &gpui::App) -> bool {
        if !self.editable {
            return false;
        }
        let store = self.store.read(cx);
        let reader = self.reader.read(cx);
        let state = reader.state();
        match action {
            Action::Compare => {
                self.setup_enabled(cx)
                    && self.fields.len() == 2
                    && self.endpoint(Side::Source, cx).is_ok()
                    && self.endpoint(Side::Target, cx).is_ok()
            }
            Action::Copy(side) => self.copy_key(side, cx).is_some(),
            Action::Refresh => !store.busy(),
            Action::Cancel => {
                store.observation_current()
                    && !store.busy()
                    && self.selected_job(cx).is_some_and(|job| {
                        model::active(&job.state) && !matches!(job.state, StatusState::Cancelling)
                    })
            }
            Action::Dismiss => {
                store.observation_current()
                    && !store.busy()
                    && self
                        .selected_job(cx)
                        .is_some_and(|job| !model::active(&job.state))
            }
            Action::Rerun => self.setup_enabled(cx) && self.selected_job(cx).is_some(),
            Action::Retry => !reader.busy() && state.is_some_and(|state| state.request().is_some()),
            Action::Clear => true,
            Action::Coverage => state.is_some_and(|state| state.metadata().is_some()),
            Action::PreviousObjects => {
                state.is_some_and(|state| state.can_turn_objects(Turn::Previous))
            }
            Action::NextObjects => state.is_some_and(|state| state.can_turn_objects(Turn::Next)),
            Action::PreviousFields => {
                state.is_some_and(|state| state.can_turn_fields(Turn::Previous))
            }
            Action::NextFields => state.is_some_and(|state| state.can_turn_fields(Turn::Next)),
            Action::PreviousSource => {
                state.is_some_and(|state| state.can_turn_value(Side::Source, Turn::Previous))
            }
            Action::NextSource => {
                state.is_some_and(|state| state.can_turn_value(Side::Source, Turn::Next))
            }
            Action::PreviousTarget => {
                state.is_some_and(|state| state.can_turn_value(Side::Target, Turn::Previous))
            }
            Action::NextTarget => {
                state.is_some_and(|state| state.can_turn_value(Side::Target, Turn::Next))
            }
        }
    }
    fn activate_captured(
        &mut self,
        action: Action,
        key: Option<&CopyKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Action::Copy(side) = action
            && self.copy_key(side, cx).as_ref() != key
        {
            return;
        }
        self.activate(action, window, cx);
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action, cx) || self.composing(window, cx) {
            return;
        }
        match action {
            Action::Compare => {
                match (
                    self.endpoint(Side::Source, cx),
                    self.endpoint(Side::Target, cx),
                ) {
                    (Ok(source), Ok(target)) => {
                        if let Some(id) = self
                            .store
                            .update(cx, |store, cx| store.start(source, target, cx))
                        {
                            self.selected_request = Some(id);
                            self.status="Waiting for comparison admission; observation resolves the exact request".into();
                        }
                    }
                    (Err(error), _) | (_, Err(error)) => self.status = error.into(),
                }
            }
            Action::Refresh => self.store.update(cx, |store, cx| store.refresh(cx)),
            Action::Cancel | Action::Dismiss => {
                if let Some(id) = self.selected.clone() {
                    self.store.update(cx, |store, cx| {
                        if matches!(action, Action::Cancel) {
                            store.cancel(&id, cx)
                        } else {
                            store.release(&id, cx)
                        }
                    });
                }
            }
            Action::Rerun => {
                if let Some(job) = self.selected_job(cx) {
                    let source = job.source.clone();
                    let target = job.target.clone();
                    if self.connections.as_ref().is_some_and(|connections| {
                        connections.get(&source.connection_id).is_some()
                            && connections.get(&target.connection_id).is_some()
                    }) {
                        let result = self.fields[0]
                            .update(cx, |field, cx| {
                                field.set_value(source.schema.clone(), window, cx)
                            })
                            .and_then(|()| {
                                self.fields[1].update(cx, |field, cx| {
                                    field.set_value(target.schema.clone(), window, cx)
                                })
                            });
                        if let Err(error) = result {
                            self.status = error.into();
                        } else {
                            self.source = Some(source.connection_id.clone());
                            self.target = Some(target.connection_id.clone());
                            self.generation = self.generation.wrapping_add(1);
                            if let Some(id) = self
                                .store
                                .update(cx, |store, cx| store.start(source, target, cx))
                            {
                                self.selected_request = Some(id);
                            }
                        }
                    }
                }
            }
            Action::Copy(side) => self.copy_value(side, cx),
            Action::Clear => self.clear(cx),
            Action::Coverage => self.coverage = !self.coverage,
            _ => {
                let intent = match action {
                    Action::Retry => Intent::Retry,
                    Action::PreviousObjects => Intent::ObjectPage(Turn::Previous),
                    Action::NextObjects => Intent::ObjectPage(Turn::Next),
                    Action::PreviousFields => Intent::FieldPage(Turn::Previous),
                    Action::NextFields => Intent::FieldPage(Turn::Next),
                    Action::PreviousSource => Intent::ValuePage(Side::Source, Turn::Previous),
                    Action::NextSource => Intent::ValuePage(Side::Source, Turn::Next),
                    Action::PreviousTarget => Intent::ValuePage(Side::Target, Turn::Previous),
                    Action::NextTarget => Intent::ValuePage(Side::Target, Turn::Next),
                    _ => return,
                };
                self.reader
                    .update(cx, |reader, cx| reader.enqueue(intent, cx));
            }
        }
        self.freeze_fields(cx);
        cx.notify();
    }
    fn select_job(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        self.selected = Some(id);
        self.status = self
            .selected_job(cx)
            .map_or("Selected comparison is unavailable", |job| {
                model::phase_label(&job.state)
            })
            .into();
        self.selected_request = None;
        self.sync_selection(cx);
        window.focus(&self.jobs_focus, cx);
        cx.notify();
    }
    fn cached_schemas(&self, side: Side) -> &[String] {
        let id = match side {
            Side::Source => self.source.as_deref(),
            Side::Target => self.target.as_deref(),
        };
        self.connections
            .as_ref()
            .and_then(|connections| id.and_then(|id| connections.get(id)))
            .map_or(&[], |connection| connection.schemas.as_slice())
    }
    fn select_schema(
        &mut self,
        side: Side,
        connection: &str,
        schema: String,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = match side {
            Side::Source => self.source.as_deref(),
            Side::Target => self.target.as_deref(),
        };
        if generation != self.generation
            || current != Some(connection)
            || !self.setup_enabled(cx)
            || self.composing(window, cx)
            || !self.cached_schemas(side).contains(&schema)
        {
            return;
        }
        if let Some(field) = self.fields.get(side_index(side)) {
            match field.update(cx, |field, cx| field.set_value(schema, window, cx)) {
                Ok(()) => {
                    self.generation = self.generation.wrapping_add(1);
                }
                Err(error) => self.status = error.into(),
            }
        }
        cx.notify();
    }
    fn copy_key(&self, side: Side, cx: &gpui::App) -> Option<CopyKey> {
        let state = self.reader.read(cx).state()?;
        let value = state.selected_value(side)?;
        if value.value_kind == ValueKind::Null {
            return None;
        }
        let offset = if value.raw_bytes == 0 {
            0
        } else {
            match state.value(side)? {
                CompareReply::Value { offset, .. } => *offset,
                _ => return None,
            }
        };
        Some(CopyKey {
            request: state.request()?.clone(),
            value,
            offset,
        })
    }
    fn copy_value(&mut self, side: Side, cx: &mut Context<Self>) {
        let Some(key) = self.copy_key(side, cx) else {
            return;
        };
        let state = self.reader.read(cx).state().unwrap();
        let (text, next) = if key.value.raw_bytes == 0 {
            (String::new(), 0)
        } else {
            let Some(CompareReply::Value {
                text, next_offset, ..
            }) = state.value(side)
            else {
                return;
            };
            (text.clone(), *next_offset)
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.status = format!(
            "Copied displayed {} chunk, bytes {}–{} of {}{}",
            side_label(side).to_lowercase(),
            key.offset,
            next,
            key.value.raw_bytes,
            if key.offset != 0 || next < key.value.raw_bytes {
                ". Partial value; other chunks were not copied."
            } else {
                "."
            }
        );
        cx.notify();
    }
    fn copy_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.composing(window, cx) {
            return false;
        }
        for side in [Side::Source, Side::Target] {
            if self.value_focus[side_index(side)].is_focused(window)
                && self.enabled(Action::Copy(side), cx)
            {
                self.copy_value(side, cx);
                return true;
            }
        }
        false
    }
    fn endpoint_label(&self, endpoint: &Endpoint) -> String {
        let connection = self
            .connections
            .as_ref()
            .and_then(|connections| connections.get(&endpoint.connection_id));
        connection.map_or_else(
            || {
                format!(
                    "Removed connection ({}) · {}",
                    endpoint.connection_id, endpoint.schema
                )
            },
            |connection| {
                format!(
                    "{} · {} · {} · {}",
                    connection.name, connection.environment, connection.database, endpoint.schema
                )
            },
        )
    }
    fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut order = vec![];
        if self.connections.is_some() {
            order.push(self.connection_focus[0].clone());
        }
        if let Some(field) = self.fields.first() {
            order.push(field.focus_handle(cx));
        }
        if !self.cached_schemas(Side::Source).is_empty() {
            order.push(self.schema_focus[0].clone());
        }
        if self.connections.is_some() {
            order.push(self.connection_focus[1].clone());
        }
        if let Some(field) = self.fields.get(1) {
            order.push(field.focus_handle(cx));
        }
        if !self.cached_schemas(Side::Target).is_empty() {
            order.push(self.schema_focus[1].clone());
        }
        order.extend(
            ACTIONS
                .iter()
                .enumerate()
                .filter(|(_, (action, _))| self.enabled(*action, cx))
                .map(|(index, _)| self.buttons[index].clone()),
        );
        if let Some(state) = self.reader.read(cx).state() {
            if self.coverage && state.metadata().is_some() {
                order.push(self.coverage_focus.clone());
            }
            if state.objects().is_some() {
                order.push(self.objects_focus.clone());
            }
            if state.fields().is_some() {
                order.push(self.fields_focus.clone());
            }
            if state.selected_object().is_some() {
                order.extend(self.value_focus.iter().cloned());
            }
        }
        order.push(self.jobs_focus.clone());
        order
    }
}
fn side_index(side: Side) -> usize {
    match side {
        Side::Source => 0,
        Side::Target => 1,
    }
}
fn side_label(side: Side) -> &'static str {
    match side {
        Side::Source => "Source",
        Side::Target => "Target",
    }
}
impl Drop for SchemaCompareView {
    fn drop(&mut self) {
        self.fields.clear();
        self.field_lease = None;
    }
}
