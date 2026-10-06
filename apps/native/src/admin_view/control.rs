//! A signal token is never reconstructed from recovery. Only the exact saved
//! revision releases it, and cancellation leaves an admitted outcome to settle.
use super::*;
use crate::apply_flow::ApplyFlow;
use dbunk_lib::backend::admin::{
    ADMIN_CONTROL_LIMIT, AdminControlAction, AdminControlConfirmation, AdminControlError,
    AdminControlOutcome, AdminControlReview, AdminControlSubmission, MAX_ADMIN_CONTROL_BYTES,
};
use dbunk_lib::backend::{WorkspaceAdminControl, WorkspaceApplyState};

const ALLOWANCE: usize = 32 * 1024;
#[derive(Clone, Copy)]
pub(super) enum ControlAction {
    ReviewCancel,
    ReviewTerminate,
    Apply,
    Confirm,
    Cancel,
    Clear,
}
enum Token {
    Review(AdminControlReview),
    Confirmation(AdminControlConfirmation),
}
pub(super) struct Control {
    pub journal: Option<WorkspaceAdminControl>,
    review: Option<AdminControlReview>,
    flow: Option<ApplyFlow<Token>>,
    next: u64,
    armed: bool,
    budget: Rc<Cell<usize>>,
    admitted: bool,
}
impl Control {
    pub fn new(journal: Option<WorkspaceAdminControl>, budget: Rc<Cell<usize>>) -> Self {
        let admitted = ALLOWANCE <= (128usize * 1024 * 1024).saturating_sub(budget.get());
        if admitted {
            budget.set(budget.get() + ALLOWANCE);
        }
        Self {
            journal,
            review: None,
            flow: None,
            next: 0,
            armed: false,
            budget,
            admitted,
        }
    }
    // Raw recovery remains in the bounded workspace journal when display
    // admission is unavailable. No token or rendered review is admitted then.
    pub fn try_admit(&mut self) {
        if !self.admitted && ALLOWANCE <= (128usize * 1024 * 1024).saturating_sub(self.budget.get())
        {
            self.budget.set(self.budget.get() + ALLOWANCE);
            self.admitted = true;
        }
    }
    pub fn can_display(&self) -> bool {
        self.admitted && self.journal.is_some()
    }
    pub fn pending(&self) -> bool {
        self.flow.as_ref().is_some_and(|flow| !flow.confirming())
    }
    fn unknown(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|journal| journal.apply_state == WorkspaceApplyState::OutcomeUnknown)
    }
    fn not_sent(&mut self) {
        if let Some(journal) = &mut self.journal {
            journal.apply_state = WorkspaceApplyState::Staged;
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
    pub fn disconnected(&mut self) {
        self.cancel_before_dispatch();
        self.flow = None;
        self.review = None;
        self.armed = false;
    }
    pub fn clear_label(&self) -> &'static str {
        if self.unknown() {
            if self.armed {
                "Discard reconciled signal recovery"
            } else {
                "Reconcile unknown signal outcome"
            }
        } else {
            "Clear signal recovery"
        }
    }
    fn text(&self) -> String {
        self.journal.as_ref().map(|journal| format!("Attempt: {}\nAction: {:?}\nPID: {}\nBackend start: {}\nQuery start: {}\nDatabase: {}\nState: {:?}\n{}", journal.attempt_id, journal.action, journal.pid, journal.backend_start, journal.query_start.as_deref().unwrap_or("NULL"), journal.database.as_deref().unwrap_or("NULL"), journal.apply_state, ADMIN_CONTROL_LIMIT)).unwrap_or_default()
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        if self.admitted {
            self.budget.set(self.budget.get().saturating_sub(ALLOWANCE));
        }
    }
}
impl AdminView {
    pub fn control_snapshot(&self) -> Option<WorkspaceAdminControl> {
        self.control.journal.clone()
    }
    pub fn control_bytes(&self) -> usize {
        self.control
            .journal
            .as_ref()
            .map_or(0, crate::results::encoded_size)
    }
    pub fn has_control_recovery(&self) -> bool {
        self.control.journal.is_some() || self.control.flow.is_some()
    }
    fn publish_control(&self, cx: &mut Context<Self>) {
        cx.emit(AdminEvent::Changed);
        cx.notify();
    }
    pub(super) fn control_enabled(&self, action: ControlAction) -> bool {
        match action {
            ControlAction::ReviewCancel | ControlAction::ReviewTerminate => {
                self.ready
                    && !self.stale
                    && !self.read.busy()
                    && self.audit.pending.is_none()
                    && self.control.admitted
                    && !self.has_control_recovery()
                    && matches!(self.section, Section::Activity(_))
                    && self.selected_index().is_some()
            }
            ControlAction::Apply => {
                self.ready
                    && self.control.review.is_some()
                    && self.control.flow.is_none()
                    && !self.control.unknown()
            }
            ControlAction::Confirm => {
                self.ready
                    && self
                        .control
                        .flow
                        .as_ref()
                        .is_some_and(ApplyFlow::confirming)
            }
            ControlAction::Cancel => self.control.flow.is_some(),
            ControlAction::Clear => self.control.can_display() && !self.control.pending(),
        }
    }
    pub(super) fn control_action(
        &mut self,
        action: ControlAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ControlAction::ReviewCancel | ControlAction::ReviewTerminate => {
                let Section::Activity(section) = self.section else {
                    return;
                };
                let Some(index) = self.selected_index() else {
                    return;
                };
                let action = if matches!(action, ControlAction::ReviewCancel) {
                    AdminControlAction::CancelQuery
                } else {
                    AdminControlAction::TerminateSession
                };
                let result = self
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .review(section, index, action);
                match result {
                    Ok(review) if review.retained_bytes() <= MAX_ADMIN_CONTROL_BYTES => {
                        let journal = WorkspaceAdminControl::from_review(&review);
                        if journal.validate().is_err() {
                            self.status = "Observed target cannot be saved; signal refused".into();
                        } else {
                            self.control.journal = Some(journal);
                            self.control.review = Some(review);
                            self.control.armed = false;
                            self.status = "Review exact target below. Sending first saves recovery. Termination disconnects the target and can roll back its transaction.".into();
                            window.focus(&self.control_details, cx);
                        }
                    }
                    Ok(_) => self.status = "Signal review exceeds its allowance".into(),
                    Err(error) => self.status = format!("Cannot review selected target: {error}"),
                }
            }
            ControlAction::Apply | ControlAction::Confirm => {
                self.control.next = self.control.next.wrapping_add(1);
                let id = self.control.next;
                if matches!(action, ControlAction::Apply) {
                    let Some(review) = self.control.review.take() else {
                        return;
                    };
                    self.control.flow = Some(ApplyFlow::new(id, Token::Review(review)));
                } else if !self
                    .control
                    .flow
                    .as_mut()
                    .is_some_and(|flow| flow.confirm(id))
                {
                    return;
                }
                self.control.journal.as_mut().unwrap().apply_state =
                    WorkspaceApplyState::OutcomeUnknown;
                self.status = "Saving exact signal recovery revision before dispatch".into();
                self.publish_control(cx);
                cx.emit(AdminEvent::PersistApply(id));
                return;
            }
            ControlAction::Cancel => {
                if self.control.cancel_before_dispatch() {
                    self.status = "Signal request cancelled before dispatch; late save acknowledgements cannot send it".into();
                } else {
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.status = "Cancellation requested; a signal already admitted may still be sent. Waiting for its receipt.".into();
                }
            }
            ControlAction::Clear if self.control.unknown() && !self.control.armed => {
                self.control.armed = true;
                self.status = "Inspect the target and reconcile this attempt. Discard reconciled recovery only removes this local record; it neither retries nor undoes a signal.".into();
            }
            ControlAction::Clear => {
                self.control.journal = None;
                self.control.review = None;
                self.control.flow = None;
                self.control.armed = false;
                self.status = "Signal recovery cleared locally; this action sends no signal. Refresh before reviewing another target.".into();
                self.stale = self.snapshot.is_some();
            }
        }
        self.publish_control(cx);
    }
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        if self
            .control
            .flow
            .as_ref()
            .is_none_or(|flow| !flow.waiting_for(id))
        {
            return;
        }
        if let Err(error) = result {
            self.control.cancel_before_dispatch();
            self.status = format!("Signal was not sent: {error}");
        } else if let Some(token) = self.control.flow.as_mut().and_then(|flow| flow.saved(id)) {
            let command = match token {
                Token::Review(review) => TableCommand::AdminApply(id, review),
                Token::Confirmation(confirmation) => TableCommand::AdminConfirm(id, confirmation),
            };
            match self
                .controls
                .as_ref()
                .ok_or("Administration is disconnected")
                .and_then(|controls| controls.send(command))
            {
                Ok(()) => self.status = "Sending reviewed signal; exact recovery saved".into(),
                Err(error) => {
                    self.control.flow = None;
                    self.control.not_sent();
                    self.status = format!("Signal was not sent: {error}");
                }
            }
        }
        self.publish_control(cx);
    }
    pub(super) fn control_received(
        &mut self,
        id: u64,
        result: Result<AdminControlSubmission, Arc<AdminControlError>>,
        cx: &mut Context<Self>,
    ) {
        if self
            .control
            .flow
            .as_ref()
            .is_none_or(|flow| flow.id() != id || !flow.dispatched())
        {
            return;
        }
        match result {
            Ok(AdminControlSubmission::NeedsConfirmation(confirmation)) => {
                if confirmation.retained_bytes() <= MAX_ADMIN_CONTROL_BYTES
                    && self.control.journal.as_ref().is_some_and(|journal| {
                        journal.matches(
                            confirmation.attempt_id(),
                            confirmation.action(),
                            confirmation.target(),
                        )
                    })
                {
                    self.control.not_sent();
                    self.control
                        .flow
                        .as_mut()
                        .unwrap()
                        .needs_confirmation(Token::Confirmation(*confirmation));
                    self.status = "Stored policy requires confirmation for this exact target; no signal sent yet".into();
                } else {
                    self.control.flow = None;
                    self.status = "Confirmation mismatch; recovery remains unknown".into();
                }
            }
            Ok(AdminControlSubmission::Finished(receipt)) => {
                self.control.flow = None;
                if self.control.journal.as_ref().is_some_and(|journal| {
                    journal.matches(&receipt.attempt_id, receipt.action, &receipt.target)
                }) {
                    self.status = match receipt.outcome {
                        AdminControlOutcome::OutcomeUnknown { reason } => format!(
                            "Signal outcome unknown: {reason}. Reconcile explicitly; never retry automatically."
                        ),
                        outcome => {
                            self.control.journal = None;
                            match outcome {
                                AdminControlOutcome::SignalSent => "Signal sent. This does not prove the query or session stopped; Refresh to inspect.".into(),
                                AdminControlOutcome::SignalNotSent => "PostgreSQL reported that the signal was not sent".into(),
                                AdminControlOutcome::TargetChanged => "Observed target changed; no signal sent. Refresh and review again.".into(),
                                AdminControlOutcome::NotDispatched { reason } => format!("Signal not dispatched: {reason}"),
                                AdminControlOutcome::OutcomeUnknown { .. } => unreachable!(),
                            }
                        }
                    };
                } else {
                    self.status = "Signal receipt mismatch; recovery remains unknown".into();
                }
                self.stale = self.snapshot.is_some();
            }
            Err(error) => {
                self.control.flow = None;
                if !matches!(*error, AdminControlError::OutcomeUnavailable) {
                    self.control.not_sent();
                }
                self.status = format!("Signal submission: {error}; recovery retained");
            }
        }
        self.publish_control(cx);
    }
    pub(super) fn control_panel(&self, cx: &Context<Self>) -> gpui::AnyElement {
        div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(crate::style::line())
            .child(
                crate::ui::toolbar()
                    .children((15..ACTIONS.len()).map(|index| self.button(index, cx))),
            )
            .when(self.control.can_display(), |panel| {
                panel.child(
                    div()
                        .id("admin-signal-review")
                        .role(Role::Group)
                        .aria_label("Exact signal target and recovery")
                        .aria_value(self.control.text())
                        .track_focus(&self.control_details)
                        .tab_stop(true)
                        .tab_index(0)
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .track_scroll(&self.control_scroll)
                        .px_2()
                        .py_1()
                        .whitespace_normal()
                        .child(self.control.text()),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn journal() -> WorkspaceAdminControl {
        WorkspaceAdminControl {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            action: dbunk_lib::backend::WorkspaceAdminAction::CancelQuery,
            pid: 12345,
            backend_start: "2026-10-03T01:02:03.123456Z".into(),
            query_start: None,
            database: Some("owned".into()),
            apply_state: WorkspaceApplyState::OutcomeUnknown,
        }
    }
    #[test]
    fn restored_unknown_disconnects_without_losing_identity_or_creating_a_replay_token() {
        let journal = journal();
        let budget = Rc::new(Cell::new(0));
        let mut control = Control::new(Some(journal.clone()), budget.clone());
        control.disconnected();
        assert!(control.unknown());
        assert_eq!(control.journal, Some(journal));
        assert!(control.review.is_none());
        assert!(control.flow.is_none());
        assert!(!control.pending());
        assert_eq!(control.clear_label(), "Reconcile unknown signal outcome");
        assert_eq!(budget.get(), ALLOWANCE);
        drop(control);
        assert_eq!(budget.get(), 0);
    }
    #[test]
    fn signal_allowance_refusal_never_evicts_other_retained_payloads() {
        let budget = Rc::new(Cell::new(128 * 1024 * 1024 - ALLOWANCE + 1));
        let before = budget.get();
        let mut control = Control::new(None, budget.clone());
        assert!(!control.admitted);
        assert_eq!(budget.get(), before);
        budget.set(before - ALLOWANCE);
        control.try_admit();
        assert!(control.admitted);
        assert_eq!(budget.get(), before);
        drop(control);
        assert_eq!(budget.get(), before - ALLOWANCE);
    }
}
