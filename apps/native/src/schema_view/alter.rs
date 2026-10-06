//! Exact observed existing-schema comment/rename over the Objects document worker.
//! Same lifecycle as the table-change review: observe (pins the schema OID),
//! render exact SQL, durably save the uncertain attempt, then dispatch once.
use crate::{
    apply_flow::ApplyFlow,
    bounded_field::{Changed, Field},
    controller::{DdlApplied, DdlObserved, DdlReviewed, TableCommand, TableControls, TableMessage},
};
use dbunk_lib::backend::{WorkspaceSchemaAlter, schema_alter::*};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, ScrollHandle,
    Subscription, Window, div, prelude::*, px,
};
use model::{Lease, Recovery, Selection, Settlement, TOKEN_BYTES};
use std::{cell::Cell, rc::Rc};
pub mod model;
mod render;
#[cfg(test)]
mod tests;
pub enum SchemaAlterEvent {
    Changed,
    PersistApply(u64),
    Activity(bool),
    Back,
    DatabaseChanged(String),
}
enum Token {
    Review(Box<SchemaAlterReview>),
    Confirmation(Box<SchemaAlterConfirmation>),
}
enum Pending {
    Observe {
        id: u64,
        intent: SchemaAlterIntent,
        cancelled: bool,
    },
    Review {
        id: u64,
        intent: SchemaAlterIntent,
        target: SchemaAlterDescription,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
pub struct SchemaAlterView {
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
    review: Option<Box<SchemaAlterReview>>,
    pending: Option<Pending>,
    flow: Option<ApplyFlow<Token>>,
    armed: bool,
    message: String,
    /// Marks `message` as an error so it renders as a shaking banner.
    failure: crate::ui::Failure,
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
impl EventEmitter<SchemaAlterEvent> for SchemaAlterView {}
impl SchemaAlterView {
    /// Admit Lease first. Borrowed validation preserves the caller's recovery on
    /// refusal. A restored journal's identity wins over any new selection.
    pub fn prepare(
        connection: &str,
        selection: Option<&Selection>,
        restored: Option<&WorkspaceSchemaAlter>,
    ) -> Result<Prepared, &'static str> {
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
            Some(SchemaAlterIntent::Rename { new_name }) => (true, false, new_name.clone()),
            Some(SchemaAlterIntent::SetComment { comment }) => (
                false,
                comment.is_none(),
                comment.clone().unwrap_or_default(),
            ),
            None => (false, false, String::new()),
        };
        let value = cx.new(|cx| {
            Field::new(
                "Schema change value: comment or new name",
                4096,
                true,
                value,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe(&value, |_, _, _: &Changed, cx| cx.notify());
        let message = if recovery.unknown() {
            "Outcome unknown. Inspect the database and explicitly reconcile; this operation cannot be retried."
        } else {
            "Observe the selected schema and review exact SQL. No change is sent by Review."
        }
        .into();
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
            failure: crate::ui::Failure::default(),
            receipt: String::new(),
            root: cx.focus_handle(),
            details: cx.focus_handle(),
            buttons: ACTIONS.iter().map(|_| cx.focus_handle()).collect(),
            scroll: ScrollHandle::new(),
        }
    }
    pub fn snapshot(&self) -> Option<WorkspaceSchemaAlter> {
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
    /// Sets an error status, shown as a banner that shakes on every failure.
    fn fail(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure.record(&self.message);
    }
    fn publish(&mut self, cx: &mut Context<Self>) {
        self.sync_field(cx);
        cx.emit(SchemaAlterEvent::Changed);
        cx.emit(SchemaAlterEvent::Activity(self.has_pending()));
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
            cx.emit(SchemaAlterEvent::DatabaseChanged(
                self.recovery.connection().to_owned(),
            ));
        }
    }
    fn enabled(&self, action: Action) -> bool {
        match action {
            Action::Back => !self.has_pending(),
            Action::Cancel => self.pending.is_some() || self.flow.is_some(),
            _ if !self.editable => false,
            Action::Operation | Action::Remove => {
                self.editable_recipe() && (!matches!(action, Action::Remove) || !self.rename)
            }
            Action::Review => self.ready && self.editable_recipe() && self.selection.is_some(),
            Action::Edit => {
                !self.has_pending()
                    && !self.recovery.unknown()
                    && self.flow.is_none()
                    && self.review.is_some()
            }
            Action::Apply => {
                self.ready
                    && self.review.is_some()
                    && self.flow.is_none()
                    && !self.recovery.unknown()
            }
            Action::Confirm => self.ready && self.flow.as_ref().is_some_and(ApplyFlow::confirming),
            Action::Discard => {
                !self.has_pending() && self.flow.is_none() && self.recovery.journal().is_some()
            }
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        if self.composing(window, cx) {
            self.fail("Finish text composition before changing this review");
            cx.notify();
            return;
        }
        if let Err(error) = self.act(action, cx) {
            self.fail(error);
        }
        self.publish(cx);
    }
    fn act(&mut self, action: Action, cx: &mut Context<Self>) -> Result<(), &'static str> {
        match action {
            Action::Back => cx.emit(SchemaAlterEvent::Back),
            Action::Operation => {
                self.rename = !self.rename;
                self.armed = false;
            }
            Action::Remove => self.remove_comment = !self.remove_comment,
            Action::Edit => {
                self.review = None;
                self.message = "Edit locally, then observe and review again. The previous staged journal remains until a new review is accepted.".into();
            }
            Action::Review => {
                let value = self
                    .value
                    .as_ref()
                    .ok_or("Value editor unavailable")?
                    .read(cx)
                    .value(cx)?;
                let intent = model::intent(self.rename, self.remove_comment, value)?;
                let request = self
                    .selection
                    .as_ref()
                    .ok_or("Choose a schema row in Objects")?
                    .request();
                let id = self.next_id()?;
                self.controls
                    .as_ref()
                    .ok_or("Connect first")?
                    .send(TableCommand::SchemaAlterObserve(id, request))?;
                self.pending = Some(Pending::Observe {
                    id,
                    intent,
                    cancelled: false,
                });
                self.armed = false;
                self.receipt.clear();
                self.message = "Observing the exact selected schema; no change sent".into();
            }
            Action::Apply | Action::Confirm => {
                let id = self.next_id()?;
                if action == Action::Apply {
                    if !self
                        .review
                        .as_ref()
                        .is_some_and(|r| self.recovery.matches_review(r))
                    {
                        return Err("Review no longer matches the durable intent");
                    }
                    self.flow = Some(ApplyFlow::new(
                        id,
                        Token::Review(self.review.take().unwrap()),
                    ));
                } else if !self.flow.as_mut().is_some_and(|f| f.confirm(id)) {
                    return Err("Confirmation no longer available");
                }
                if !self.recovery.mark_unknown() {
                    self.flow = None;
                    return Err("Saveable intent missing; nothing dispatched");
                }
                self.message =
                    "Saving the exact uncertain recovery revision before dispatch".into();
                self.publish(cx);
                cx.emit(SchemaAlterEvent::PersistApply(id));
            }
            Action::Cancel => {
                if self.not_sent() {
                    self.message = "Cancelled before dispatch; late save acknowledgements cannot release this change".into();
                } else {
                    if let Some(pending) = &mut self.pending {
                        pending.cancel();
                    }
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.message =
                        "Cancellation requested. Waiting for owned cleanup and the actual outcome."
                            .into();
                }
            }
            Action::Discard => {
                if self.recovery.unknown() && !self.armed {
                    self.armed = true;
                    self.message = "Inspect and reconcile the database first. Activate Discard reconciled recovery to remove this local record; it neither retries nor undoes SQL.".into();
                } else if self.recovery.discard(self.armed) {
                    self.review = None;
                    self.selection = None;
                    self.armed = false;
                    self.receipt.clear();
                    self.message =
                        "Recovery discarded. Return to Objects and select a schema again.".into();
                }
            }
        }
        Ok(())
    }
    /// Dispatches only for the exact flow waiting on this persisted revision.
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if self.flow.as_ref().is_none_or(|f| !f.waiting_for(id)) {
            return;
        }
        if let Err(error) = result {
            self.not_sent();
            self.fail(format!("Schema change not sent: {error}"));
        } else if !self.recovery.unknown() {
            self.not_sent();
            self.fail("Exact uncertain recovery was not preserved; nothing dispatched");
        } else if let Some(token) = self.flow.as_mut().and_then(|f| f.saved(id)) {
            let exact = match &token {
                Token::Review(review) => self.recovery.matches_review(review),
                Token::Confirmation(confirmation) => {
                    self.recovery.matches_review(confirmation.review())
                }
            };
            if !exact {
                self.flow = None;
                self.fail("Saved intent no longer matches authority; no dispatch, recovery remains unknown");
                self.publish(cx);
                return;
            }
            let command = match token {
                Token::Review(review) => TableCommand::SchemaAlterApply(id, review),
                Token::Confirmation(token) => TableCommand::SchemaAlterConfirm(id, token),
            };
            match self
                .controls
                .as_ref()
                .ok_or("Objects document disconnected")
                .and_then(|c| c.send(command))
            {
                Ok(()) => {
                    self.message =
                        "Exact recovery saved; schema change dispatched. Waiting for its receipt."
                            .into()
                }
                Err(error) => {
                    self.flow = None;
                    self.recovery.not_sent();
                    self.fail(format!("Schema change not sent: {error}"));
                }
            }
        }
        self.publish(cx);
    }
    pub fn receive(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        let mut invalidate = false;
        match message {
            TableMessage::TableDdlObserved(id, DdlObserved::Schema(result))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p, Pending::Observe { .. }) && p.id() == id) =>
            {
                let Some(Pending::Observe {
                    intent, cancelled, ..
                }) = self.pending.take()
                else {
                    unreachable!()
                };
                if cancelled {
                    self.message = "Observation cancelled; late target discarded".into();
                } else {
                    match result {
                        Ok(target)
                            if target.retained_bytes() <= TOKEN_BYTES
                                && self
                                    .selection
                                    .as_mut()
                                    .is_some_and(|s| s.pin(target.description())) =>
                        {
                            let description = target.description().clone();
                            match self.next_id().and_then(|next| {
                                self.controls
                                    .as_ref()
                                    .ok_or("Objects document disconnected")?
                                    .send(TableCommand::SchemaAlterReview(
                                        next,
                                        target,
                                        intent.clone(),
                                    ))?;
                                self.pending = Some(Pending::Review {
                                    id: next,
                                    intent,
                                    target: description,
                                    cancelled: false,
                                });
                                Ok(())
                            }) {
                                Ok(()) => {
                                    self.message = "Rendering exact SQL for the observed schema; no change sent"
                                        .into()
                                }
                                Err(error) => self.fail(error),
                            }
                        }
                        Ok(_) => self.fail(
                            "Observed schema does not match the selected identity or exceeds bounds",
                        ),
                        Err(error) => self.fail(format!("Observation refused: {error}")),
                    }
                }
            }
            TableMessage::TableDdlReviewed(id, DdlReviewed::Schema(result))
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|p| matches!(p, Pending::Review { .. }) && p.id() == id) =>
            {
                let Some(Pending::Review {
                    intent,
                    target,
                    cancelled,
                    ..
                }) = self.pending.take()
                else {
                    unreachable!()
                };
                if cancelled {
                    self.message = "Review cancelled; late authority discarded".into();
                } else {
                    match result {
                        Ok(review)
                            if review.retained_bytes() <= TOKEN_BYTES
                                && review.target() == &target
                                && review.intent() == &intent =>
                        {
                            match self.recovery.stage(&review) {
                                Ok(()) => {
                                    self.review = Some(review);
                                    self.message = "Review the exact schema, SQL and deadline below. Apply requires a durable save first.".into();
                                }
                                Err(error) => self.fail(error),
                            }
                        }
                        Ok(_) => self.fail(
                            "Review identity or intent mismatch; no executable token retained",
                        ),
                        Err(error) => self.fail(format!("Review refused: {error}")),
                    }
                }
            }
            TableMessage::TableDdlApplied(id, DdlApplied::Schema(result))
                if self
                    .flow
                    .as_ref()
                    .is_some_and(|f| f.id() == id && f.dispatched()) =>
            {
                invalidate = self.settle(result);
            }
            _ => return,
        }
        self.publish(cx);
        // The worker has returned and flow settled before parent invalidation.
        if invalidate {
            cx.emit(SchemaAlterEvent::DatabaseChanged(
                self.recovery.connection().to_owned(),
            ));
        }
    }
    /// Returns whether a potentially changing outcome must invalidate captures.
    fn settle(
        &mut self,
        result: Result<Box<SchemaAlterSubmission>, std::sync::Arc<SchemaAlterError>>,
    ) -> bool {
        match result {
            Ok(submission) => match *submission {
                SchemaAlterSubmission::NeedsConfirmation(confirmation) => {
                    if confirmation.retained_bytes() <= TOKEN_BYTES
                        && self.recovery.matches_review(confirmation.review())
                    {
                        self.recovery.not_sent();
                        self.flow
                            .as_mut()
                            .unwrap()
                            .needs_confirmation(Token::Confirmation(confirmation));
                        self.message = "Stored policy requires confirmation of this exact review. No change sent; confirmation requires its own save acknowledgement.".into();
                        false
                    } else {
                        self.flow = None;
                        self.fail("Confirmation mismatch; recovery remains unknown. Reconcile explicitly.");
                        true
                    }
                }
                SchemaAlterSubmission::Finished(receipt) => {
                    self.flow = None;
                    match self.recovery.receipt(&receipt) {
                        Settlement::Applied => {
                            self.selection = None;
                            self.receipt = format!(
                                "Applied transactional schema change. Attempt {}. {:?}",
                                receipt.attempt_id.as_str(),
                                receipt.outcome
                            );
                            self.message =
                                "Change applied. Refresh Objects before another operation.".into();
                            true
                        }
                        Settlement::Staged => {
                            self.receipt = format!(
                                "Attempt {}: {:?}",
                                receipt.attempt_id.as_str(),
                                receipt.outcome
                            );
                            self.message = "No transactional schema change remains. Intent retained; another attempt requires fresh observation and review.".into();
                            // Known rollback can still leave server-hook external effects.
                            matches!(receipt.outcome, SchemaAlterOutcome::RolledBack { .. })
                        }
                        Settlement::Unknown => {
                            self.fail("Outcome or receipt identity unknown. Recovery retained; inspect and reconcile explicitly.");
                            true
                        }
                    }
                }
            },
            Err(error) => {
                self.flow = None;
                self.recovery.submission_error(&error);
                self.fail(format!("Schema change: {error}. Recovery retained."));
                *error == SchemaAlterError::OutcomeUnavailable
            }
        }
    }
}
impl Drop for SchemaAlterView {
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
