//! Create-schema bottom review over the Objects document's existing worker.
//! Tokens never survive reconnect; exact intent and uncertain outcomes do.
use crate::{
    apply_flow::ApplyFlow,
    controller::{TableCommand, TableControls, TableMessage},
    schema_changes::Recovery,
};
use dbunk_lib::backend::schema_ddl::{
    CreateSchemaConfirmation, CreateSchemaError, CreateSchemaIntent, CreateSchemaOutcome,
    CreateSchemaReview, CreateSchemaSubmission,
};
use dbunk_lib::backend::{WorkspaceApplyState, WorkspaceSchemaChanges};
use gpui::{Context, Entity, EventEmitter, FocusHandle, Focusable, Window, prelude::*};
use std::{cell::Cell, rc::Rc};
mod actions;
mod render;
use crate::bounded_field::Field;

const ALLOWANCE: usize = 1024 * 1024;
pub struct Lease(Rc<Cell<usize>>);
impl Lease {
    pub fn admit(budget: Rc<Cell<usize>>) -> Option<Self> {
        if ALLOWANCE > (128usize * 1024 * 1024).saturating_sub(budget.get()) {
            return None;
        }
        budget.set(budget.get() + ALLOWANCE);
        Some(Self(budget))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(ALLOWANCE));
    }
}
pub enum SchemaEvent {
    Changed,
    PersistApply(u64),
    Activity(bool),
    Back,
}
enum Token {
    Review(CreateSchemaReview),
    Confirmation(CreateSchemaConfirmation),
}
#[derive(Clone, Copy)]
enum Action {
    Review,
    Apply,
    Confirm,
    Cancel,
    Copy,
    Comment,
    Edit,
    Discard,
    Back,
}
const ACTIONS: [(Action, &str); 9] = [
    (Action::Review, "Review SQL"),
    (Action::Apply, "Apply"),
    (Action::Confirm, "Confirm apply"),
    (Action::Cancel, "Cancel"),
    (Action::Copy, "Copy review"),
    (Action::Comment, "Include comment"),
    (Action::Edit, "Edit draft"),
    (Action::Discard, "Discard draft"),
    (Action::Back, "Back to objects"),
];
pub struct SchemaView {
    _lease: Lease,
    recovery: Recovery,
    controls: Option<TableControls>,
    ready: bool,
    editable: bool,
    name: Entity<Field>,
    comment: Entity<Field>,
    include_comment: bool,
    review: Option<CreateSchemaReview>,
    reviewing: Option<(u64, CreateSchemaIntent, bool)>,
    applying: Option<ApplyFlow<Token>>,
    next: Rc<Cell<u64>>,
    preview: String,
    reconcile_armed: bool,
    message: String,
    root: FocusHandle,
    details: FocusHandle,
    buttons: Vec<FocusHandle>,
}
impl EventEmitter<SchemaEvent> for SchemaView {}
impl SchemaView {
    pub fn new(
        lease: Lease,
        next: Rc<Cell<u64>>,
        connection: String,
        restored: Option<WorkspaceSchemaChanges>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = restored.as_ref().map_or("", |c| c.intent.name()).to_owned();
        let comment = restored
            .as_ref()
            .and_then(|c| c.intent.comment())
            .unwrap_or("")
            .to_owned();
        let include_comment = restored
            .as_ref()
            .is_some_and(|c| c.intent.comment().is_some());
        let recovery = Recovery::new(connection, restored);
        let message = if recovery.unknown() { "Outcome unknown. Inspect the database, then explicitly reconcile this attempt. It will not be retried." } else { "Review saves this intent. Field edits remain local until Review." }.into();
        Self {
            _lease: lease,
            recovery,
            controls: None,
            ready: false,
            editable: true,
            name: cx.new(|cx| Field::new("Schema name", 63, false, name, window, cx)),
            comment: cx.new(|cx| Field::new("Schema comment", 4096, true, comment, window, cx)),
            include_comment,
            review: None,
            reviewing: None,
            applying: None,
            next,
            preview: String::new(),
            reconcile_armed: false,
            message,
            root: cx.focus_handle(),
            details: cx.focus_handle(),
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(),
        }
    }
    pub fn snapshot(&self) -> Option<WorkspaceSchemaChanges> {
        self.recovery.changes().cloned()
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.recovery
            .changes()
            .map_or(0, crate::results::encoded_size)
    }
    pub fn has_changes(&self) -> bool {
        self.recovery.changes().is_some() || self.has_pending()
    }
    pub fn has_pending(&self) -> bool {
        self.reviewing.is_some()
            || self
                .applying
                .as_ref()
                .is_some_and(|flow| !flow.confirming())
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
        self.controls = controls;
        self.ready = ready;
        self.editable = editable;
        let readonly = !editable
            || self.has_pending()
            || self.recovery.unknown()
            || self.review.is_some()
            || self.applying.is_some();
        self.name
            .update(cx, |field, cx| field.set_readonly(readonly, cx));
        self.comment
            .update(cx, |field, cx| field.set_readonly(readonly, cx));
    }
    pub fn disconnected(&mut self, cx: &mut Context<Self>) {
        self.controls = None;
        self.ready = false;
        self.reviewing = None;
        self.review = None;
        if let Some(mut flow) = self.applying.take()
            && flow.cancel_before_dispatch()
        {
            self.recovery.not_sent();
        }
        self.message = if self.recovery.unknown() {
            "Disconnected with unknown outcome; reconcile before another attempt"
        } else {
            "Disconnected; saved intent retained. Connect and review again."
        }
        .into();
        self.publish(cx);
    }
    fn publish(&self, cx: &mut Context<Self>) {
        cx.emit(SchemaEvent::Changed);
        cx.emit(SchemaEvent::Activity(self.has_pending()));
        cx.notify();
    }
    fn next_id(&mut self) -> u64 {
        let next = self.next.get().wrapping_add(1);
        self.next.set(next);
        next
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.name
            .update(cx, |field, cx| field.composing(window, cx))
            || self
                .comment
                .update(cx, |field, cx| field.composing(window, cx))
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if self
            .applying
            .as_ref()
            .is_none_or(|flow| !flow.waiting_for(id))
        {
            return;
        }
        if let Err(error) = result {
            self.cancel_before_dispatch();
            self.message = format!("Apply was not sent: {error}");
            self.publish(cx);
            return;
        }
        let Some(token) = self.applying.as_mut().and_then(|flow| flow.saved(id)) else {
            return;
        };
        let command = match token {
            Token::Review(review) => TableCommand::SchemaApply(id, review),
            Token::Confirmation(confirmation) => TableCommand::SchemaConfirm(id, confirmation),
        };
        match self
            .controls
            .as_ref()
            .ok_or("Objects document is disconnected")
            .and_then(|controls| controls.send(command))
        {
            Ok(()) => self.message = "Applying reviewed schema intent; recovery saved".into(),
            Err(error) => {
                self.applying = None;
                self.recovery.not_sent();
                self.message = format!("Apply was not sent: {error}");
            }
        }
        self.publish(cx);
    }
    fn cancel_before_dispatch(&mut self) {
        if self
            .applying
            .as_mut()
            .is_some_and(ApplyFlow::cancel_before_dispatch)
        {
            self.applying = None;
            self.recovery.not_sent();
        }
    }
    pub fn receive(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        match message {
            TableMessage::SchemaReviewed(id, result)
                if self
                    .reviewing
                    .as_ref()
                    .is_some_and(|pending| pending.0 == id) =>
            {
                let (_, intent, cancelled) = self.reviewing.take().unwrap();
                if cancelled {
                    self.message = "Review cancelled; late reply discarded".into();
                    self.publish(cx);
                    return;
                }
                match result {
                    Ok(review)
                        if review.intent() == &intent
                            && review.retained_bytes() <= 64 * 1024
                            && review.preview().checked_heap_bytes().is_some() =>
                    {
                        let changes = WorkspaceSchemaChanges {
                            attempt_id: review.attempt_id().clone(),
                            intent,
                            apply_state: WorkspaceApplyState::Staged,
                        };
                        if self.recovery.stage(changes) {
                            self.preview = review
                                .preview()
                                .statements
                                .iter()
                                .map(|s| s.sql.as_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.review = Some(review);
                            self.message = "Review exact SQL. Apply saves recovery before sending this transaction.".into();
                        }
                    }
                    Ok(_) => {
                        self.message =
                            "Review identity or size was inconsistent; no write permitted".into()
                    }
                    Err(error) => self.message = format!("Review refused: {error}"),
                }
            }
            TableMessage::SchemaApplied(id, result)
                if self
                    .applying
                    .as_ref()
                    .is_some_and(|flow| flow.id() == id && flow.dispatched()) =>
            {
                match result {
                    Ok(CreateSchemaSubmission::NeedsConfirmation(confirmation)) => {
                        let matches = self.recovery.changes().is_some_and(|changes| {
                            &changes.attempt_id == confirmation.attempt_id()
                                && &changes.intent == confirmation.intent()
                        });
                        if matches && confirmation.retained_bytes() <= 64 * 1024 {
                            self.recovery.not_sent();
                            self.applying
                                .as_mut()
                                .unwrap()
                                .needs_confirmation(Token::Confirmation(*confirmation));
                            self.message = "Stored policy requires confirmation for this exact SQL; no write sent yet".into();
                        } else {
                            self.applying = None;
                            self.message = "Confirmation mismatch; recovery remains unknown".into();
                        }
                    }
                    Ok(CreateSchemaSubmission::Finished(receipt)) => {
                        self.applying = None;
                        if self.recovery.settle(&receipt) {
                            self.message = match receipt.outcome {
                                CreateSchemaOutcome::Applied {
                                    statements,
                                    runtime_ms,
                                } => {
                                    self.preview.clear();
                                    format!(
                                        "Applied {statements} statements in {runtime_ms} ms. Catalog may be stale; Refresh separately."
                                    )
                                }
                                CreateSchemaOutcome::NotApplied { reason } => format!(
                                    "Not applied: {reason}. Saved intent retained; review again."
                                ),
                                CreateSchemaOutcome::OutcomeUnknown { reason } => format!(
                                    "Outcome unknown: {reason}. Reconcile explicitly; never retry automatically."
                                ),
                            };
                        } else {
                            self.message =
                                "Receipt did not match this attempt; recovery remains unknown"
                                    .into();
                        }
                    }
                    Err(error) => {
                        self.applying = None;
                        if !matches!(error.as_ref(), CreateSchemaError::OutcomeUnavailable) {
                            self.recovery.not_sent();
                        }
                        self.message = format!("Schema submission: {error}; saved intent retained");
                    }
                }
            }
            _ => return,
        }
        self.publish(cx);
    }
}
impl Focusable for SchemaView {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        if self.recovery.unknown() {
            self.details.clone()
        } else {
            self.name.focus_handle(cx)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_review_reservation_cannot_evict_retained_captures_and_releases_on_drop() {
        let budget = Rc::new(Cell::new(128 * 1024 * 1024 - ALLOWANCE + 1));
        let before = budget.get();
        assert!(Lease::admit(budget.clone()).is_none());
        assert_eq!(budget.get(), before);
        budget.set(128 * 1024 * 1024 - ALLOWANCE);
        let lease = Lease::admit(budget.clone()).unwrap();
        assert_eq!(budget.get(), 128 * 1024 * 1024);
        assert!(Lease::admit(budget.clone()).is_none());
        drop(lease);
        assert_eq!(budget.get(), 128 * 1024 * 1024 - ALLOWANCE);
    }
}
