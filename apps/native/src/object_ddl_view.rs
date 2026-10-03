//! Generic typed object-DDL review over the Objects document worker. Each
//! consumer (drop, create view) supplies a purpose; the lifecycle is shared:
//! observe exact identities, review regenerated SQL, durably save the exact
//! attempt, then Apply/Confirm once; Unknown requires explicit reconciliation.
use crate::{
    apply_flow::ApplyFlow,
    bounded_field::{Changed, Field},
    controller::{DdlApplied, DdlObserved, DdlReviewed, TableCommand, TableControls, TableMessage},
    object_ddl_model::{Draft, Lease, Purpose, Recovery, Settlement, TOKEN_BYTES},
};
use dbunk_lib::backend::{WorkspaceObjectDdl, object_ddl::*};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, ScrollHandle,
    Subscription, Window, div, prelude::*, px,
};
use std::{cell::Cell, rc::Rc};
mod actions;
mod render;
mod replies;
gpui::actions!(object_ddl, [NextControl, PreviousControl]);
pub enum ObjectDdlEvent {
    Changed,
    PersistApply(u64),
    Activity(bool),
    Back,
    DatabaseChanged(String),
}
enum Token {
    Review(Box<ObjectDdlReview>),
    Confirmation(Box<ObjectDdlConfirmation>),
}
enum Pending {
    Observe {
        id: u64,
        operations: Vec<ObjectDdlOperation>,
        cancelled: bool,
    },
    Review {
        id: u64,
        operations: Vec<ObjectDdlOperation>,
        target: ObjectDdlDescription,
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
    Mode,
    Option,
    Review,
    Edit,
    Apply,
    Confirm,
    Cancel,
    Discard,
}
const ACTIONS: [(Action, &str); 9] = [
    (Action::Back, "Objects"),
    (Action::Mode, "Mode"),
    (Action::Option, "Option"),
    (Action::Review, "Observe and review SQL"),
    (Action::Edit, "Edit draft"),
    (Action::Apply, "Apply"),
    (Action::Confirm, "Confirm apply"),
    (Action::Cancel, "Cancel request"),
    (Action::Discard, "Discard draft"),
];
pub struct ObjectDdlView {
    recovery: Recovery,
    purpose: Option<Purpose>,
    draft: Draft,
    next: Rc<Cell<u64>>,
    controls: Option<TableControls>,
    ready: bool,
    editable: bool,
    name: Option<Entity<Field>>,
    body: Option<Entity<Field>>,
    subscriptions: Vec<Subscription>,
    review: Option<Box<ObjectDdlReview>>,
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
    purpose: Option<Purpose>,
}
impl EventEmitter<ObjectDdlEvent> for ObjectDdlView {}
impl ObjectDdlView {
    /// Admit Lease first. Borrowed validation preserves the caller's recovery
    /// on refusal. A restored journal's purpose wins over a new selection.
    pub fn prepare(
        connection: &str,
        purpose: Option<&Purpose>,
        restored: Option<&WorkspaceObjectDdl>,
    ) -> Result<Prepared, &'static str> {
        let recovery = Recovery::new(connection.to_owned(), restored.cloned())?;
        // A journal from a consumer this build cannot edit still opens with no
        // purpose, so it can be inspected, reconciled and discarded.
        let purpose = match recovery.journal() {
            Some(journal) => Purpose::from_operations(&journal.operations),
            None => purpose.cloned(),
        };
        Ok(Prepared { recovery, purpose })
    }
    pub fn new(
        lease: Lease,
        next: Rc<Cell<u64>>,
        prepared: Prepared,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Prepared { recovery, purpose } = prepared;
        let (draft, name, body) = recovery
            .journal()
            .map(|j| Draft::from_operations(&j.operations))
            .unwrap_or_default();
        let creating = matches!(purpose, Some(Purpose::CreateView { .. }));
        let (name, body) = if creating {
            (
                Some(cx.new(|cx| Field::new("New view name", 63, false, name, window, cx))),
                Some(cx.new(|cx| {
                    Field::new(
                        "View SQL body (one SELECT or VALUES query)",
                        MAX_OBJECT_DDL_SQL_BODY_BYTES,
                        true,
                        body,
                        window,
                        cx,
                    )
                })),
            )
        } else {
            (None, None)
        };
        let subscriptions = name
            .iter()
            .chain(body.iter())
            .map(|field| cx.subscribe(field, |_, _, _: &Changed, cx| cx.notify()))
            .collect();
        let message = if recovery.unknown() {
            "Outcome unknown. Inspect the database and explicitly reconcile; this operation cannot be retried."
        } else if purpose.is_none() && recovery.journal().is_some() {
            "This restored change cannot be edited here; inspect it, then Discard to remove the local record."
        } else if purpose.is_none() {
            "Select an Objects row first."
        } else {
            "Observe exact identities and review the regenerated SQL. Review sends no change."
        }
        .into();
        Self {
            _lease: lease,
            recovery,
            purpose,
            draft,
            next,
            controls: None,
            ready: false,
            editable: true,
            name,
            body,
            subscriptions,
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
    pub fn snapshot(&self) -> Option<WorkspaceObjectDdl> {
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
        self.sync_fields(cx);
        if changed {
            cx.notify();
        }
    }
    fn creating(&self) -> bool {
        matches!(self.purpose, Some(Purpose::CreateView { .. }))
    }
    fn editable_recipe(&self) -> bool {
        self.editable
            && !self.recovery.unknown()
            && self.pending.is_none()
            && self.review.is_none()
            && self.flow.is_none()
    }
    fn fields(&self) -> impl Iterator<Item = &Entity<Field>> {
        self.name.iter().chain(self.body.iter())
    }
    fn sync_fields(&mut self, cx: &mut Context<Self>) {
        let readonly = !self.editable_recipe();
        for field in self.name.iter().chain(self.body.iter()) {
            field.update(cx, |field, cx| field.set_readonly(readonly, cx));
        }
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.fields()
            .any(|f| f.update(cx, |field, cx| field.composing(window, cx)))
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
        self.sync_fields(cx);
        cx.emit(ObjectDdlEvent::Changed);
        cx.emit(ObjectDdlEvent::Activity(self.has_pending()));
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
            cx.emit(ObjectDdlEvent::DatabaseChanged(
                self.recovery.connection().to_owned(),
            ));
        }
    }
}
impl Drop for ObjectDdlView {
    fn drop(&mut self) {
        if self.has_pending()
            && let Some(controls) = &self.controls
        {
            controls.cancel();
        }
        self.subscriptions.clear();
        self.name = None;
        self.body = None;
        self.review = None;
        self.pending = None;
        self.flow = None;
    }
}
