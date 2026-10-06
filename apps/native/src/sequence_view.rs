//! Sequence Inspect and explicit Advance/Set/Restart for one Objects document.
//!
//! Inspection never advances the sequence. Each write needs a fresh observation,
//! an exact typed review and an explicit Apply (plus Confirm when stored policy
//! requires it). Outcomes are not journaled across restarts: an unknown outcome
//! is disclosed in-session and blocks further writes until an explicit Inspect.
use crate::{
    bounded_field::Field,
    controller::{TableCommand, TableControls, TableMessage},
    sequence_runtime::{SequenceReply, SequenceRequest},
};
use dbunk_lib::backend::objects::{PgObjectKind, PgObjectRef};
use dbunk_lib::backend::sequences::{
    MAX_SEQUENCE_RECEIPT_BYTES, MAX_SEQUENCE_REVIEW_BYTES, ObservedSequence, SEQUENCE_EFFECT_LIMIT,
    SEQUENCE_IDENTITY_LIMIT, SEQUENCE_INSPECT_LIMIT, SequenceConfirmation, SequenceError,
    SequenceIntent, SequenceOutcome, SequenceReceipt, SequenceReview, SequenceSubmission,
};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, Window, prelude::*,
};
use std::{cell::Cell, rc::Rc};

mod render;
#[cfg(test)]
mod tests;

const ALLOWANCE: usize = 256 * 1024;
const VALUE_BYTES: usize = 20;
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
pub enum SequenceEvent {
    Changed,
    Activity(bool),
    Back,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Inspect,
    ReviewAdvance,
    ReviewSet,
    SetCalled,
    ReviewRestart,
    RestartWith,
    Apply,
    Confirm,
    Cancel,
    Discard,
    Copy,
    Back,
}
const ACTIONS: [(Action, &str); 12] = [
    (Action::Inspect, "Inspect sequence"),
    (Action::ReviewAdvance, "Review advance (nextval)"),
    (Action::ReviewSet, "Review set (setval)"),
    (Action::SetCalled, "Set: is_called"),
    (Action::ReviewRestart, "Review restart"),
    (Action::RestartWith, "Restart: use explicit value"),
    (Action::Apply, "Apply reviewed action"),
    (Action::Confirm, "Confirm reviewed action"),
    (Action::Cancel, "Cancel request"),
    (Action::Discard, "Discard review"),
    (Action::Copy, "Copy details"),
    (Action::Back, "Back to objects"),
];

/// Strict signed 64-bit parsing: optional leading '-', ASCII digits only.
pub(crate) fn parse_value(text: &str) -> Result<i64, &'static str> {
    const REFUSED: &str =
        "Enter a whole number in the signed 64-bit range (no spaces, separators or '+')";
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(REFUSED);
    }
    text.parse::<i64>().map_err(|_| REFUSED)
}

/// Build the typed intent from the controls. Range checks against the observed
/// definition happen when the backend observation mints the review.
fn intent_for(
    action: Action,
    set_text: &str,
    set_called: bool,
    restart_with: bool,
    restart_text: &str,
) -> Result<SequenceIntent, String> {
    match action {
        Action::ReviewAdvance => Ok(SequenceIntent::Advance),
        Action::ReviewSet => parse_value(set_text)
            .map(|value| SequenceIntent::Set {
                value,
                is_called: set_called,
            })
            .map_err(|error| format!("Set value: {error}")),
        Action::ReviewRestart if restart_with => parse_value(restart_text)
            .map(|value| SequenceIntent::Restart { with: Some(value) })
            .map_err(|error| format!("Restart value: {error}")),
        Action::ReviewRestart => Ok(SequenceIntent::Restart { with: None }),
        _ => Err("Not a review action".into()),
    }
}

fn outcome_message(outcome: &SequenceOutcome) -> String {
    match outcome {
        SequenceOutcome::Completed { returned: Some(value) } => {
            format!("Completed; PostgreSQL returned {value}. Inspect again before another action.")
        }
        SequenceOutcome::Completed { returned: None } => {
            "Restart committed. Inspect again before another action.".into()
        }
        SequenceOutcome::TargetChanged => {
            "Sequence identity or definition changed since inspection; nothing applied. Inspect again.".into()
        }
        SequenceOutcome::NotDispatched { reason } => format!("Not dispatched: {reason}. Nothing applied."),
        SequenceOutcome::Rejected { reason } => {
            format!("PostgreSQL rejected the call before it took effect: {reason}.")
        }
        SequenceOutcome::RolledBack { reason } => {
            format!("Restart rolled back: {reason}. Nothing committed.")
        }
        SequenceOutcome::OutcomeUnknown { reason } => format!(
            "Outcome unknown ({reason}). The change may or may not have happened; it will not be retried. Inspect the sequence before any other action."
        ),
    }
}

pub struct SequenceView {
    _lease: Lease,
    next: Rc<Cell<u64>>,
    reference: Option<PgObjectRef>,
    observed: Option<ObservedSequence>,
    review: Option<SequenceReview>,
    confirmation: Option<SequenceConfirmation>,
    /// Attempt last sent to the worker, so replies must match it exactly.
    attempt: Option<String>,
    inspecting: Option<u64>,
    cancel_inspect: bool,
    applying: Option<u64>,
    unknown: Option<String>,
    receipt: String,
    set_called: bool,
    restart_with: bool,
    set_value: Entity<Field>,
    restart_value: Entity<Field>,
    controls: Option<TableControls>,
    ready: bool,
    editable: bool,
    message: String,
    /// Marks `message` as an error so it renders as a shaking banner.
    failure: crate::ui::Failure,
    root: FocusHandle,
    details: FocusHandle,
    buttons: Vec<FocusHandle>,
    scroll: ScrollHandle,
}
impl EventEmitter<SequenceEvent> for SequenceView {}
impl SequenceView {
    pub fn new(
        lease: Lease,
        next: Rc<Cell<u64>>,
        reference: PgObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _lease: lease,
            next,
            reference: Some(reference).filter(|r| r.kind == PgObjectKind::Sequence),
            observed: None,
            review: None,
            confirmation: None,
            attempt: None,
            inspecting: None,
            cancel_inspect: false,
            applying: None,
            unknown: None,
            receipt: String::new(),
            set_called: true,
            restart_with: false,
            set_value: cx
                .new(|cx| Field::new("Set value", VALUE_BYTES, false, String::new(), window, cx)),
            restart_value: cx.new(|cx| {
                Field::new(
                    "Restart value",
                    VALUE_BYTES,
                    false,
                    String::new(),
                    window,
                    cx,
                )
            }),
            controls: None,
            ready: false,
            editable: true,
            message:
                "Inspect reads the sequence without advancing it. Every write is reviewed first."
                    .into(),
            failure: crate::ui::Failure::default(),
            root: cx.focus_handle(),
            details: cx.focus_handle(),
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(),
            scroll: ScrollHandle::new(),
        }
    }
    pub fn reference(&self) -> Option<&PgObjectRef> {
        self.reference.as_ref()
    }
    /// True while a reply is owed or an unknown outcome is undisclosed-safe to drop.
    pub fn has_changes(&self) -> bool {
        self.has_pending() || self.unknown.is_some()
    }
    pub fn has_pending(&self) -> bool {
        self.inspecting.is_some() || self.applying.is_some()
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
        let readonly =
            !editable || self.has_pending() || self.review.is_some() || self.confirmation.is_some();
        self.set_value
            .update(cx, |field, cx| field.set_readonly(readonly, cx));
        self.restart_value
            .update(cx, |field, cx| field.set_readonly(readonly, cx));
    }
    fn next_id(&self) -> u64 {
        let id = self.next.get().wrapping_add(1);
        self.next.set(id);
        id
    }
    /// Sets an error status, shown as a banner that shakes on every failure.
    fn fail(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure.record(&self.message);
    }
    fn publish(&self, cx: &mut Context<Self>) {
        cx.emit(SequenceEvent::Changed);
        cx.emit(SequenceEvent::Activity(self.has_pending()));
        cx.notify();
    }
    pub fn disconnected(&mut self, cx: &mut Context<Self>) {
        self.controls = None;
        self.ready = false;
        self.inspecting = None;
        self.cancel_inspect = false;
        self.review = None;
        self.confirmation = None;
        self.observed = None;
        if self.applying.take().is_some() {
            self.attempt = None;
            self.unknown = Some(
                "The Objects connection closed after the sequence action was dispatched. Outcome unknown; it will not be retried. Reconnect and Inspect before any other action.".into(),
            );
            self.message = self.unknown.clone().unwrap();
        } else {
            self.message = "Disconnected. Reconnect and Inspect again; no action was sent.".into();
        }
        self.publish(cx);
    }
    fn idle(&self) -> bool {
        !self.has_pending() && self.review.is_none() && self.confirmation.is_none()
    }
    fn enabled(&self, action: Action) -> bool {
        if !self.editable {
            return matches!(action, Action::Copy | Action::Back | Action::Cancel)
                && self.can(action);
        }
        self.can(action)
    }
    fn can(&self, action: Action) -> bool {
        match action {
            Action::Inspect => self.ready && self.reference.is_some() && self.idle(),
            Action::ReviewAdvance | Action::ReviewSet | Action::ReviewRestart => {
                self.ready && self.observed.is_some() && self.unknown.is_none() && self.idle()
            }
            Action::SetCalled | Action::RestartWith => self.idle(),
            Action::Apply => self.ready && self.review.is_some() && !self.has_pending(),
            Action::Confirm => self.ready && self.confirmation.is_some() && !self.has_pending(),
            Action::Cancel => self.has_pending(),
            Action::Discard => {
                !self.has_pending() && (self.review.is_some() || self.confirmation.is_some())
            }
            Action::Copy => true,
            Action::Back => true,
        }
    }
    fn send(&mut self, request: SequenceRequest) -> Result<u64, &'static str> {
        let id = self.next_id();
        self.controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| controls.send(TableCommand::Sequence(id, request)))
            .map(|()| id)
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::Inspect => {
                let reference = self.reference.clone().unwrap();
                match self.send(SequenceRequest::Inspect(reference)) {
                    Ok(id) => {
                        self.inspecting = Some(id);
                        self.cancel_inspect = false;
                        self.message =
                            "Reading sequence metadata and value; nextval is never called".into();
                    }
                    Err(error) => self.fail(error),
                }
            }
            Action::ReviewAdvance | Action::ReviewSet | Action::ReviewRestart => {
                let set_text = self.set_value.read(cx).value(cx);
                let restart_text = self.restart_value.read(cx).value(cx);
                let intent = set_text.and_then(|set| {
                    restart_text.map(|restart| {
                        intent_for(action, &set, self.set_called, self.restart_with, &restart)
                    })
                });
                let review = match intent {
                    Ok(Ok(intent)) => self
                        .observed
                        .as_ref()
                        .unwrap()
                        .review(intent)
                        .map_err(|error| error.to_string()),
                    Ok(Err(error)) => Err(error),
                    Err(error) => Err(error.into()),
                };
                match review {
                    Ok(review) => {
                        self.review = Some(review);
                        self.message = "Review the exact SQL, parameters and effect below, then Apply. Nothing has been sent.".into();
                        window.focus(&self.details, cx);
                    }
                    Err(error) => self.fail(format!("Review refused: {error}")),
                }
            }
            Action::SetCalled => self.set_called = !self.set_called,
            Action::RestartWith => self.restart_with = !self.restart_with,
            Action::Apply => {
                let review = self.review.take().unwrap();
                let attempt = review.attempt_id().to_owned();
                match self.send(SequenceRequest::Apply(Box::new(review))) {
                    Ok(id) => {
                        self.attempt = Some(attempt);
                        self.applying = Some(id);
                        self.message = "Sequence action dispatched; waiting for its receipt".into();
                    }
                    Err(error) => self.fail(format!(
                        "Not sent: {error}. Review again from the observation."
                    )),
                }
            }
            Action::Confirm => {
                let confirmation = self.confirmation.take().unwrap();
                match self.send(SequenceRequest::Confirm(Box::new(confirmation))) {
                    Ok(id) => {
                        self.applying = Some(id);
                        self.message =
                            "Confirmed sequence action dispatched; waiting for its receipt".into();
                    }
                    Err(error) => {
                        self.attempt = None;
                        self.fail(format!(
                            "Not sent: {error}. Review again from the observation."
                        ))
                    }
                }
            }
            Action::Cancel => {
                if let Some(controls) = &self.controls {
                    controls.cancel();
                }
                if self.inspecting.is_some() {
                    self.cancel_inspect = true;
                    self.message = "Inspection cancellation requested".into();
                } else {
                    self.message = "Cancellation requested. A dispatched nextval/setval may still take effect; waiting for the owned outcome.".into();
                }
            }
            Action::Discard => {
                self.review = None;
                self.confirmation = None;
                self.attempt = None;
                self.message = "Review discarded; nothing was sent".into();
            }
            Action::Copy => cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.text())),
            Action::Back => {
                cx.emit(SequenceEvent::Back);
                return;
            }
        }
        self.publish(cx);
    }
    pub fn receive(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        let TableMessage::Sequence(id, reply) = message else {
            return;
        };
        match reply {
            SequenceReply::Inspected(result) if self.inspecting == Some(id) => {
                self.inspecting = None;
                if std::mem::take(&mut self.cancel_inspect) {
                    self.message = "Inspection cancelled; late reply discarded".into();
                } else {
                    match result {
                        Ok(observed) if self.matches_reference(&observed) => {
                            self.observed = Some(*observed);
                            self.unknown = None;
                            self.message = "Observed. Choose an action to review; nothing is sent until Apply.".into();
                        }
                        Ok(_) => {
                            self.fail("Observation did not match the selected sequence; refused")
                        }
                        Err(error) => self.fail(format!("Inspection failed: {error}")),
                    }
                }
            }
            SequenceReply::Applied(result) if self.applying == Some(id) => {
                self.applying = None;
                self.settle(result);
            }
            _ => return,
        }
        self.publish(cx);
    }
    fn matches_reference(&self, observed: &ObservedSequence) -> bool {
        let target = &observed.observation().target;
        observed.retained_bytes() <= MAX_SEQUENCE_REVIEW_BYTES
            && self.reference.as_ref().is_some_and(|reference| {
                reference.schema.as_deref() == Some(target.schema())
                    && reference.name == target.name()
            })
    }
    fn settle(&mut self, result: Result<SequenceSubmission, std::sync::Arc<SequenceError>>) {
        let attempt = self.attempt.take();
        match result {
            Ok(SequenceSubmission::NeedsConfirmation(confirmation)) => {
                if attempt.as_deref() == Some(confirmation.review().attempt_id())
                    && confirmation.retained_bytes() <= MAX_SEQUENCE_REVIEW_BYTES
                {
                    self.attempt = attempt;
                    self.confirmation = Some(*confirmation);
                    self.message = "Stored policy requires confirmation of this exact action; nothing sent yet.".into();
                } else {
                    self.fail("Confirmation did not match the reviewed attempt; discarded. Nothing was sent.");
                }
            }
            Ok(SequenceSubmission::Finished(receipt)) => {
                // Any finished attempt invalidates the observation.
                self.observed = None;
                if attempt.as_deref() != Some(receipt.attempt_id.as_str())
                    || receipt.retained_bytes() > MAX_SEQUENCE_RECEIPT_BYTES
                {
                    self.unknown = Some("Receipt did not match the dispatched attempt. Treat the outcome as unknown and Inspect before another action.".into());
                    self.message = self.unknown.clone().unwrap();
                    return;
                }
                self.message = outcome_message(&receipt.outcome);
                if receipt.outcome.unknown() {
                    self.unknown = Some(self.message.clone());
                }
                self.receipt = receipt_text(&receipt);
            }
            Err(error) if *error == SequenceError::OutcomeUnavailable => {
                self.observed = None;
                self.unknown = Some(format!(
                    "{error}. The change may or may not have happened; it will not be retried."
                ));
                self.message = self.unknown.clone().unwrap();
            }
            Err(error) => self.fail(format!(
                "Not applied: {error}. Review again from the observation."
            )),
        }
    }
    pub(crate) fn text(&self) -> String {
        let mut text = String::new();
        if let Some(unknown) = &self.unknown {
            text.push_str(&format!("OUTCOME UNKNOWN: {unknown}\n\n"));
        }
        match (&self.observed, &self.reference) {
            (Some(observed), _) => text.push_str(&observed.observation().text()),
            (None, Some(reference)) => text.push_str(&format!(
                "Selected sequence: {:?}.{:?} (not inspected)\n",
                reference.schema.as_deref().unwrap_or(""),
                reference.name
            )),
            (None, None) => text.push_str("No sequence selected\n"),
        }
        let pending = self
            .review
            .as_ref()
            .or(self.confirmation.as_ref().map(SequenceConfirmation::review));
        if let Some(review) = pending {
            text.push_str(&format!(
                "\nReview attempt {}\nAction: {:?}\n{}",
                review.attempt_id(),
                review.intent(),
                review.preview().text()
            ));
        }
        if !self.receipt.is_empty() {
            text.push_str(&format!("\nLast receipt:\n{}", self.receipt));
        }
        text.push_str(&format!(
            "\n{SEQUENCE_INSPECT_LIMIT}\n{SEQUENCE_IDENTITY_LIMIT}\n{SEQUENCE_EFFECT_LIMIT}\nUnknown outcomes are kept for this app session only; closing the tab or app discards the notice.\n"
        ));
        text
    }
}
fn receipt_text(receipt: &SequenceReceipt) -> String {
    format!(
        "Attempt {} · {} ms\nSequence: {} (OID {})\nAction: {:?}\nEquivalent: {}\nOutcome: {:?}\n",
        receipt.attempt_id,
        receipt.runtime_ms,
        receipt.target.qualified(),
        receipt.target.sequence_oid(),
        receipt.intent,
        receipt.preview.summary,
        receipt.outcome
    )
}
impl Focusable for SequenceView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.details.clone()
    }
}
