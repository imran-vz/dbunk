//! An observed maintenance review belongs to one Objects document. Durable
//! descriptions survive reconnect; executable tokens never do.
use crate::{
    apply_flow::ApplyFlow,
    controller::{TableCommand, TableControls, TableMessage},
};
use dbunk_lib::backend::maintenance::{
    MAINTENANCE_EFFECT_LIMIT, MAINTENANCE_IDENTITY_LIMIT, MAX_MAINTENANCE_REVIEW_BYTES,
    MaintenanceConfirmation, MaintenanceError, MaintenanceIntent, MaintenanceOutcome,
    MaintenanceReview, MaintenanceSubmission,
};
use dbunk_lib::backend::objects::{PgObjectKind, PgObjectRef};
use dbunk_lib::backend::{WorkspaceMaintenance, WorkspaceMaintenanceState};
use gpui::{
    ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, ScrollHandle,
    Window, div, prelude::*, px,
};
use std::{cell::Cell, rc::Rc};

mod render;
#[cfg(test)]
mod tests;

const ALLOWANCE: usize = 128 * 1024;
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
pub enum MaintenanceEvent {
    Changed,
    PersistApply(u64),
    Activity(bool),
    Back,
}
enum Token {
    Review(MaintenanceReview),
    Confirmation(MaintenanceConfirmation),
}
#[derive(Clone, Copy)]
enum Action {
    Review(MaintenanceIntent),
    Apply,
    Confirm,
    Cancel,
    Clear,
    Copy,
    Back,
}
const ACTIONS: [(Action, &str); 11] = [
    (Action::Review(MaintenanceIntent::Vacuum), "Review VACUUM"),
    (Action::Review(MaintenanceIntent::Analyze), "Review ANALYZE"),
    (
        Action::Review(MaintenanceIntent::ReindexTable),
        "Review REINDEX TABLE",
    ),
    (
        Action::Review(MaintenanceIntent::RefreshMaterializedView {
            concurrently: false,
        }),
        "Review REFRESH",
    ),
    (
        Action::Review(MaintenanceIntent::RefreshMaterializedView { concurrently: true }),
        "Review REFRESH CONCURRENTLY",
    ),
    (Action::Apply, "Apply maintenance"),
    (Action::Confirm, "Confirm maintenance"),
    (Action::Cancel, "Cancel request"),
    (Action::Clear, "Clear recovery"),
    (Action::Copy, "Copy review"),
    (Action::Back, "Back to objects"),
];
pub struct MaintenanceView {
    _lease: Lease,
    next: Rc<Cell<u64>>,
    reference: Option<PgObjectRef>,
    journal: Option<WorkspaceMaintenance>,
    review: Option<MaintenanceReview>,
    reviewing: Option<u64>,
    cancel_review: bool,
    flow: Option<ApplyFlow<Token>>,
    controls: Option<TableControls>,
    ready: bool,
    editable: bool,
    armed: bool,
    message: String,
    receipt: String,
    root: FocusHandle,
    details: FocusHandle,
    buttons: Vec<FocusHandle>,
    scroll: ScrollHandle,
}
impl EventEmitter<MaintenanceEvent> for MaintenanceView {}
impl MaintenanceView {
    pub fn new(
        lease: Lease,
        next: Rc<Cell<u64>>,
        reference: Option<PgObjectRef>,
        journal: Option<WorkspaceMaintenance>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self { _lease: lease, next, reference: if journal.is_some() { None } else { reference },
            journal, review: None, reviewing: None, cancel_review: false, flow: None,
            controls: None, ready: false, editable: true, armed: false,
            message: "Choose an action to observe the target and review SQL. Recovered records cannot be applied.".into(),
            receipt: String::new(), root: cx.focus_handle(), details: cx.focus_handle(),
            buttons: (0..ACTIONS.len()).map(|_| cx.focus_handle()).collect(), scroll: ScrollHandle::new() }
    }
    pub fn snapshot(&self) -> Option<WorkspaceMaintenance> {
        self.journal.clone()
    }
    pub fn snapshot_bytes(&self) -> usize {
        self.journal
            .as_ref()
            .map_or(0, crate::results::encoded_size)
    }
    pub fn has_changes(&self) -> bool {
        self.journal.is_some() || self.has_pending()
    }
    pub fn has_pending(&self) -> bool {
        self.reviewing.is_some() || self.flow.as_ref().is_some_and(|flow| !flow.confirming())
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn set_runtime(&mut self, controls: Option<TableControls>, ready: bool, editable: bool) {
        self.controls = controls;
        self.ready = ready;
        self.editable = editable;
    }
    fn next_id(&self) -> u64 {
        let id = self.next.get().wrapping_add(1);
        self.next.set(id);
        id
    }
    fn uncertain(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|journal| journal.state != WorkspaceMaintenanceState::Staged)
    }
    fn not_sent(&mut self) {
        if let Some(journal) = &mut self.journal {
            journal.state = WorkspaceMaintenanceState::Staged;
        }
    }
    fn cancel_before_dispatch(&mut self) -> bool {
        if self
            .flow
            .as_mut()
            .is_some_and(ApplyFlow::cancel_before_dispatch)
        {
            self.flow = None;
            self.not_sent();
            true
        } else {
            false
        }
    }
    fn publish(&self, cx: &mut Context<Self>) {
        cx.emit(MaintenanceEvent::Changed);
        cx.emit(MaintenanceEvent::Activity(self.has_pending()));
        cx.notify();
    }
    pub fn disconnected(&mut self, cx: &mut Context<Self>) {
        self.cancel_before_dispatch();
        self.flow = None;
        self.review = None;
        self.reviewing = None;
        self.controls = None;
        self.ready = false;
        self.armed = false;
        self.message = "Disconnected. Recovery is read-only; no operation will be replayed.".into();
        self.publish(cx);
    }
    fn enabled(&self, action: Action) -> bool {
        if !self.editable {
            return false;
        }
        match action {
            Action::Review(intent) => {
                self.ready
                    && !self.has_changes()
                    && self.reference.as_ref().is_some_and(|reference| {
                        !matches!(intent, MaintenanceIntent::RefreshMaterializedView { .. })
                            || reference.kind == PgObjectKind::MaterializedView
                    })
            }
            Action::Apply => {
                self.ready && self.review.is_some() && self.flow.is_none() && !self.uncertain()
            }
            Action::Confirm => self.ready && self.flow.as_ref().is_some_and(ApplyFlow::confirming),
            Action::Cancel => self.reviewing.is_some() || self.flow.is_some(),
            Action::Clear => self.journal.is_some() && !self.has_pending(),
            Action::Copy => self.journal.is_some() || !self.receipt.is_empty(),
            Action::Back => true,
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::Review(intent) => {
                let id = self.next_id();
                let reference = self.reference.clone().unwrap();
                match self
                    .controls
                    .as_ref()
                    .ok_or("Connect first")
                    .and_then(|controls| {
                        controls.send(TableCommand::MaintenanceReview(id, reference, intent))
                    }) {
                    Ok(()) => {
                        self.reviewing = Some(id);
                        self.cancel_review = false;
                        self.receipt.clear();
                        self.message = "Observing the selected target; no maintenance sent".into();
                    }
                    Err(error) => self.message = error.into(),
                }
            }
            Action::Apply | Action::Confirm => {
                let id = self.next_id();
                if matches!(action, Action::Apply) {
                    self.flow = Some(ApplyFlow::new(
                        id,
                        Token::Review(self.review.take().unwrap()),
                    ));
                } else if !self.flow.as_mut().is_some_and(|flow| flow.confirm(id)) {
                    return;
                }
                self.journal.as_mut().unwrap().state = WorkspaceMaintenanceState::OutcomeUnknown;
                self.message = "Saving the exact recovery revision before dispatch".into();
                self.publish(cx);
                cx.emit(MaintenanceEvent::PersistApply(id));
                return;
            }
            Action::Cancel => {
                if self.cancel_before_dispatch() {
                    self.message =
                        "Cancelled before dispatch; late save acknowledgements cannot apply it"
                            .into();
                } else {
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.cancel_review = self.reviewing.is_some();
                    self.message = "Cancellation requested; waiting for the owned outcome. Dispatched maintenance can leave effects.".into();
                }
            }
            Action::Clear if self.uncertain() && !self.armed => {
                self.armed = true;
                self.message = "Inspect the database and reconcile this attempt, then discard this local record. Clearing recovery neither retries nor undoes maintenance.".into();
            }
            Action::Clear => {
                self.journal = None;
                self.review = None;
                self.flow = None;
                self.armed = false;
                self.reference = None;
                self.message = "Local recovery cleared. Return to Objects, refresh and select a target before another review.".into();
            }
            Action::Copy => cx.write_to_clipboard(ClipboardItem::new_string(self.text())),
            Action::Back => {
                cx.emit(MaintenanceEvent::Back);
                return;
            }
        }
        if matches!(action, Action::Review(_) | Action::Clear) {
            window.focus(&self.details, cx);
        }
        self.publish(cx);
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if self.flow.as_ref().is_none_or(|flow| !flow.waiting_for(id)) {
            return;
        }
        if let Err(error) = result {
            self.cancel_before_dispatch();
            self.message = format!("Maintenance not sent: {error}");
        } else if let Some(token) = self.flow.as_mut().and_then(|flow| flow.saved(id)) {
            let command = match token {
                Token::Review(review) => TableCommand::MaintenanceApply(id, review),
                Token::Confirmation(confirmation) => {
                    TableCommand::MaintenanceConfirm(id, confirmation)
                }
            };
            match self
                .controls
                .as_ref()
                .ok_or("Objects document disconnected")
                .and_then(|controls| controls.send(command))
            {
                Ok(()) => {
                    self.message =
                        "Maintenance dispatched; exact recovery saved. Waiting for its receipt."
                            .into()
                }
                Err(error) => {
                    self.flow = None;
                    self.not_sent();
                    self.message = format!("Maintenance not sent: {error}");
                }
            }
        }
        self.publish(cx);
    }
    pub fn receive(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        match message {
            TableMessage::MaintenanceReviewed(id, result) if self.reviewing == Some(id) => {
                self.reviewing = None;
                if self.cancel_review {
                    self.cancel_review = false;
                    self.message = "Review cancelled; late observation discarded".into();
                } else {
                    match result {
                        Ok(review) if review.retained_bytes() <= MAX_MAINTENANCE_REVIEW_BYTES => {
                            let journal = WorkspaceMaintenance::from_review(&review);
                            if journal.validate().is_err() {
                                self.message =
                                    "Observed review cannot be saved; maintenance refused".into();
                            } else {
                                self.journal = Some(journal);
                                self.review = Some(review);
                                self.message = "Review the observed target, SQL, deadlines and limitations below before applying.".into();
                            }
                        }
                        Ok(_) => self.message = "Maintenance review exceeds its allowance".into(),
                        Err(error) => self.message = format!("Maintenance review refused: {error}"),
                    }
                }
            }
            TableMessage::MaintenanceApplied(id, result)
                if self
                    .flow
                    .as_ref()
                    .is_some_and(|flow| flow.id() == id && flow.dispatched()) =>
            {
                match result {
                    Ok(MaintenanceSubmission::NeedsConfirmation(confirmation)) => {
                        if confirmation.retained_bytes() <= MAX_MAINTENANCE_REVIEW_BYTES
                            && self.journal.as_ref().is_some_and(|journal| {
                                journal.matches(
                                    confirmation.attempt_id(),
                                    confirmation.intent(),
                                    confirmation.target(),
                                    confirmation.preview(),
                                )
                            })
                        {
                            self.not_sent();
                            self.flow
                                .as_mut()
                                .unwrap()
                                .needs_confirmation(Token::Confirmation(*confirmation));
                            self.message = "Stored policy requires confirmation of this exact target and action; no maintenance sent yet.".into();
                        } else {
                            self.flow = None;
                            self.message = "Confirmation mismatch; recovery remains unknown".into();
                        }
                    }
                    Ok(MaintenanceSubmission::Finished(receipt)) => {
                        self.flow = None;
                        if receipt.retained_bytes()
                            <= dbunk_lib::backend::maintenance::MAX_MAINTENANCE_RECEIPT_BYTES
                            && self.journal.as_ref().is_some_and(|journal| {
                                journal.matches(
                                    &receipt.attempt_id,
                                    receipt.intent,
                                    &receipt.target,
                                    &receipt.preview,
                                )
                            })
                        {
                            self.receipt = format!(
                                "{}\nAttempt {} · {} ms\nOutcome: {:?}\n{}{}",
                                self.review_text(false),
                                receipt.attempt_id,
                                receipt.runtime_ms,
                                receipt.outcome,
                                receipt
                                    .notices
                                    .iter()
                                    .map(|notice| format!(
                                        "{}: {}\n",
                                        notice.severity, notice.message
                                    ))
                                    .collect::<String>(),
                                if receipt.notices_truncated {
                                    "Additional warning text omitted at its limit"
                                } else {
                                    ""
                                }
                            );
                            settle_recovery(&mut self.journal, &receipt.outcome);
                            match receipt.outcome {
                                MaintenanceOutcome::OutcomeUnknown { .. } => {
                                    self.message = "Maintenance outcome unknown. Reconcile explicitly; no automatic retry.".into();
                                }
                                MaintenanceOutcome::InterruptedEffectsPossible { .. } => {
                                    self.message = "Maintenance interrupted; effects may remain. Inspect and reconcile before another attempt.".into();
                                }
                                outcome => {
                                    // A known terminal outcome needs no recovery. The admitted receipt
                                    // retains the exact observed review for this app session only.
                                    self.reference = None;
                                    self.message = match outcome {
                                        MaintenanceOutcome::Completed => "Command completed. Review warnings for skipped work; this does not measure maintenance performed.".into(),
                                        MaintenanceOutcome::TargetChanged => "Observed target changed; maintenance not dispatched. Refresh before another review.".into(),
                                        MaintenanceOutcome::NotDispatched { reason } => format!("Maintenance not dispatched: {reason}"),
                                        MaintenanceOutcome::RolledBack { reason } => format!("Transactional changes rolled back: {reason}. Sequence or external effects are not covered."),
                                        _ => unreachable!(),
                                    };
                                }
                            }
                        } else {
                            self.message =
                                "Maintenance receipt mismatch; recovery remains unknown".into();
                        }
                    }
                    Err(error) => {
                        self.flow = None;
                        if *error != MaintenanceError::OutcomeUnavailable {
                            self.not_sent();
                        }
                        self.message =
                            format!("Maintenance submission: {error}; recovery retained");
                    }
                }
            }
            _ => return,
        }
        self.publish(cx);
    }
    fn review_text(&self, include_state: bool) -> String {
        let target = self.journal.as_ref().map(|j| {
            let state = if include_state { format!("State: {:?}\n", j.state) } else { String::new() };
            format!("Attempt: {}\nAction: {:?}\nDatabase: {:?} (OID {})\nSchema: {:?} (OID {})\nRelation: {:?} (OID {}, {:?})\n{state}SQL: {}\nEffects: {}\nOperation deadline: {} ms\nStatement timeout: {}\n",
                j.attempt_id, j.action, j.database, j.database_oid, j.schema, j.namespace_oid, j.name, j.relation_oid, j.kind, j.sql,
                if j.potentially_partial { "potentially partial after dispatch" } else { "transactional database changes" }, j.operation_timeout_ms,
                j.statement_timeout_ms.map_or_else(|| "inherited/server default".into(), |ms| format!("{ms} ms (configured)")))
        }).or_else(|| self.reference.as_ref().map(|r| format!("Selected target: {:?}.{:?}\n", r.schema.as_deref().unwrap_or(""), r.name))).unwrap_or_default();
        format!("{target}\n{MAINTENANCE_IDENTITY_LIMIT}\n{MAINTENANCE_EFFECT_LIMIT}\n\n")
    }
    fn text(&self) -> String {
        if self.journal.is_none() && !self.receipt.is_empty() {
            return self.receipt.clone();
        }
        format!("{}{}", self.review_text(true), self.receipt)
    }
}

// Acknowledged partial effects remain a distinct durable state from lost
// acknowledgement. Neither state can be automatically retried or cleared.
fn settle_recovery(journal: &mut Option<WorkspaceMaintenance>, outcome: &MaintenanceOutcome) {
    match outcome {
        MaintenanceOutcome::OutcomeUnknown { .. } => {
            if let Some(journal) = journal {
                journal.state = WorkspaceMaintenanceState::OutcomeUnknown;
            }
        }
        MaintenanceOutcome::InterruptedEffectsPossible { .. } => {
            if let Some(journal) = journal {
                journal.state = WorkspaceMaintenanceState::EffectsPossible;
            }
        }
        _ => *journal = None,
    }
}
