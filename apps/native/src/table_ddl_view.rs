//! Exact observed table/column changes over the Objects document worker.
use crate::{
    apply_flow::ApplyFlow,
    bounded_field::{Changed, Field},
    controller::{DdlApplied, DdlObserved, DdlReviewed, TableCommand, TableControls, TableMessage},
    table_ddl_model::{self as model, Lease, Recovery, Selection, Settlement, TOKEN_BYTES},
};
use dbunk_lib::backend::{WorkspaceTableDdl, table_ddl::*};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, ScrollHandle,
    Subscription, Window, div, prelude::*, px,
};
use std::{cell::Cell, rc::Rc};
mod actions;
mod render;
mod replies;
gpui::actions!(table_ddl, [NextControl, PreviousControl]);
pub enum TableDdlEvent {
    Changed,
    PersistApply(u64),
    Activity(bool),
    Back,
    DatabaseChanged(String),
}
enum Token {
    Review(Box<TableDdlReview>),
    Confirmation(Box<TableDdlConfirmation>),
}
enum Pending {
    Observe {
        id: u64,
        intent: TableDdlIntent,
        cancelled: bool,
    },
    Review {
        id: u64,
        intent: TableDdlIntent,
        target: TableDdlDescription,
        cancelled: bool,
    },
}
impl Pending {
    fn id(&self) -> u64 {
        match self {
            Self::Observe { id, .. } | Self::Review { id, .. } => *id,
        }
    }
    fn cancel(&mut self) {
        match self {
            Self::Observe { cancelled, .. } | Self::Review { cancelled, .. } => *cancelled = true,
        }
    }
}
#[derive(Clone, Copy)]
enum Action {
    Back,
    Operation,
    Remove,
    Review,
    Edit,
    Apply,
    Confirm,
    Cancel,
    Discard,
}
const ACTIONS: [(Action, &str); 9] = [
    (Action::Back, "Objects"),
    (Action::Operation, "Operation: Comment"),
    (Action::Remove, "Remove comment"),
    (Action::Review, "Observe and review SQL"),
    (Action::Edit, "Edit draft"),
    (Action::Apply, "Apply"),
    (Action::Confirm, "Confirm apply"),
    (Action::Cancel, "Cancel request"),
    (Action::Discard, "Discard draft"),
];
pub struct TableDdlView {
    recovery: Recovery,
    selection: Option<Selection>,
    next: Rc<Cell<u64>>,
    controls: Option<TableControls>,
    ready: bool,
    editable: bool,
    value: Option<Entity<Field>>,
    subscription: Option<Subscription>,
    rename: bool,
    remove_comment: bool,
    review: Option<Box<TableDdlReview>>,
    pending: Option<Pending>,
    flow: Option<ApplyFlow<Token>>,
    armed: bool,
    message: String,
    receipt: String,
    root: FocusHandle,
    details: FocusHandle,
    buttons: Vec<FocusHandle>,
    scroll: ScrollHandle,
    // Dropped after all typed payload and editor handles.
    _lease: Lease,
}
pub struct Prepared {
    recovery: Recovery,
    selection: Option<Selection>,
}
impl EventEmitter<TableDdlEvent> for TableDdlView {}
impl TableDdlView {
    /// Admit Lease first. Borrowed validation preserves the caller's recovery on
    /// refusal; the reservation covers the bounded old/new journal overlap.
    pub fn prepare(
        connection: &str,
        selection: Option<&Selection>,
        restored: Option<&WorkspaceTableDdl>,
    ) -> Result<Prepared, &'static str> {
        if connection.is_empty()
            || connection.len() > 256
            || restored.is_some_and(|j| j.validate().is_err() || j.retained_bytes() > 48 * 1024)
        {
            return Err("Table-change recovery exceeds its validated bounds");
        }
        let recovery = Recovery::new(connection.to_owned(), restored.cloned())?;
        let selection = recovery.selection()?.or_else(|| selection.cloned());
        Ok(Prepared {
            recovery,
            selection,
        })
    }
    pub fn new(
        lease: Lease,
        next: Rc<Cell<u64>>,
        prepared: Prepared,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Prepared {
            recovery,
            selection,
        } = prepared;
        let (rename, remove_comment, value) = match recovery.journal().map(|j| &j.intent) {
            Some(TableDdlIntent::Rename { new_name }) => (true, false, new_name.clone()),
            Some(TableDdlIntent::SetComment { comment }) => (
                false,
                comment.is_none(),
                comment.clone().unwrap_or_default(),
            ),
            None => (false, false, String::new()),
        };
        let value = cx.new(|cx| {
            Field::new(
                "Table change value: comment or new name",
                4096,
                true,
                value,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(&value, |_, _, _: &Changed, cx| cx.notify());
        let message = if recovery.unknown() { "Outcome unknown. Inspect the database and explicitly reconcile; this operation cannot be retried." }
            else { "Observe the selected identity and review exact SQL. No change is sent by Review." }.into();
        Self {
            _lease: lease,
            recovery,
            selection,
            next,
            controls: None,
            ready: false,
            editable: true,
            value: Some(value),
            subscription: Some(subscription),
            rename,
            remove_comment,
            review: None,
            pending: None,
            flow: None,
            armed: false,
            message,
            receipt: String::new(),
            root: cx.focus_handle(),
            details: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            scroll: ScrollHandle::new(),
        }
    }
    pub fn snapshot(&self) -> Option<WorkspaceTableDdl> {
        self.recovery.journal().cloned()
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.recovery
            .journal()
            .map_or(0, crate::results::encoded_size)
    }
    pub fn has_changes(&self) -> bool {
        self.recovery.journal().is_some() || self.has_pending()
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some() || self.flow.as_ref().is_some_and(|f| !f.confirming())
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn set_runtime(
        &mut self,
        controls: Option<TableControls>,
        ready: bool,
        editable: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = (self.ready, self.editable) != (ready, editable);
        self.controls = controls;
        self.ready = ready;
        self.editable = editable;
        self.sync_field(cx);
        if changed {
            cx.notify();
        }
    }
    fn editable_recipe(&self) -> bool {
        self.editable
            && !self.recovery.unknown()
            && self.pending.is_none()
            && self.review.is_none()
            && self.flow.is_none()
    }
    fn sync_field(&mut self, cx: &mut Context<Self>) {
        let readonly = !self.editable_recipe() || self.remove_comment && !self.rename;
        if let Some(value) = &self.value {
            value.update(cx, |field, cx| field.set_readonly(readonly, cx));
        }
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.value
            .as_ref()
            .is_some_and(|v| v.update(cx, |field, cx| field.composing(window, cx)))
    }
    fn next_id(&self) -> Result<u64, &'static str> {
        let id = self
            .next
            .get()
            .checked_add(1)
            .ok_or("Request identity exhausted; reopen the workspace")?;
        self.next.set(id);
        Ok(id)
    }
    fn publish(&mut self, cx: &mut Context<Self>) {
        self.sync_field(cx);
        cx.emit(TableDdlEvent::Changed);
        cx.emit(TableDdlEvent::Activity(self.has_pending()));
        cx.notify();
    }
    fn not_sent(&mut self) -> bool {
        if self
            .flow
            .as_mut()
            .is_some_and(ApplyFlow::cancel_before_dispatch)
        {
            self.flow = None;
            self.recovery.not_sent();
            true
        } else {
            false
        }
    }
    /// Parent joins the document worker; this method revokes all UI authority.
    pub fn disconnected(&mut self, cx: &mut Context<Self>) {
        let dispatched = self.flow.as_ref().is_some_and(ApplyFlow::dispatched);
        self.not_sent();
        self.flow = None;
        self.review = None;
        self.pending = None;
        self.controls = None;
        self.ready = false;
        self.armed = false;
        self.message = "Disconnected. Recovery retained; no operation will replay.".into();
        self.publish(cx);
        if dispatched {
            cx.emit(TableDdlEvent::DatabaseChanged(
                self.recovery.connection().to_owned(),
            ));
        }
    }
}
impl Drop for TableDdlView {
    fn drop(&mut self) {
        if self.has_pending()
            && let Some(controls) = &self.controls
        {
            controls.cancel();
        }
        self.subscription = None;
        self.value = None;
        self.review = None;
        self.pending = None;
        self.flow = None;
    }
}
