//! Review dialog (plan 032 §3.3): a per-change diff, the exact SQL with bound
//! values, and an environment-aware apply gate. The dialog only collects
//! intent. The backend NeedsConfirmation token stays the enforcement boundary
//! (ADR-0024): the UI confirms automatically at most once per apply, and only
//! when the user's click already acknowledged that confirmation.
use super::*;
use crate::{
    data_model::{
        ConfirmStyle, DiffCell, DiffChange, DiffKind, DiffValue, diff_summary, format_param,
        param_label, preconfirmation, review_diff,
    },
    style,
    ui::{
        self, Variant,
        confirm::{TypedConfirm, TypedConfirmEvent},
        dialog,
    },
};
use dbunk_lib::backend::DevelopmentEnvironment;
use gpui::AnyElement;

/// Changes rendered in the diff list; the plan is capped at 128 already.
const DIFF_CHANGES: usize = 128;
/// Cells rendered per change before a "+N more columns" note.
const DIFF_CELLS: usize = 32;

pub(super) struct ReviewStatement {
    sql: SharedString,
    /// (visible line from `format_param`, AX label with the value).
    params: Vec<(SharedString, SharedString)>,
}

#[derive(Default)]
pub(super) struct ReviewDialog {
    pub(super) diff: Vec<DiffChange>,
    pub(super) summary: String,
    pub(super) operations: usize,
    pub(super) deletes: bool,
    /// Copied from the backend preview, so the SQL stays visible while the
    /// token itself is in flight.
    pub(super) statements: Vec<ReviewStatement>,
    /// Fresh for every review; recreated empty when confirmation escalates.
    pub(super) typed: Option<Entity<TypedConfirm>>,
    pub(super) typed_events: Option<Subscription>,
    /// The backend asked for a confirmation the click did not acknowledge.
    pub(super) escalated: bool,
    pub(super) failure: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReviewPhase {
    /// Waiting for the backend review (SQL and bound values).
    Preparing,
    /// Reviewed; the apply button follows the gate.
    Ready,
    /// Journal being saved before the first dispatch.
    Saving,
    /// Journal being saved before the one automatic confirmation.
    AutoConfirming,
    /// Sent; only the backend can settle it now.
    Applying,
    /// The backend requires a confirmation the user must type.
    NeedsTyped,
    Failed,
}

/// The dialog phase from the controller state. A pending apply wins over
/// everything else; without one, a recorded failure wins over a stale review.
pub(super) fn review_phase<T>(
    failed: bool,
    preparing: bool,
    ready: bool,
    flow: Option<&ApplyFlow<T>>,
    auto_confirms: u32,
) -> ReviewPhase {
    if let Some(flow) = flow {
        return if flow.confirming() {
            ReviewPhase::NeedsTyped
        } else if flow.dispatched() {
            ReviewPhase::Applying
        } else if auto_confirms > 0 {
            ReviewPhase::AutoConfirming
        } else {
            ReviewPhase::Saving
        };
    }
    if failed {
        ReviewPhase::Failed
    } else if ready {
        ReviewPhase::Ready
    } else if preparing {
        ReviewPhase::Preparing
    } else {
        ReviewPhase::Failed
    }
}

/// Apply is enabled only on a fresh review (or an escalated confirmation),
/// never on a read-only connection, and typed policies need the exact word.
pub(super) fn apply_enabled(
    style: Option<ConfirmStyle>,
    typed_matched: bool,
    phase: ReviewPhase,
    read_only: bool,
) -> bool {
    if read_only {
        return false;
    }
    match (phase, style) {
        (_, None) => false,
        (ReviewPhase::Ready, Some(ConfirmStyle::Plain)) => true,
        (ReviewPhase::Ready, Some(ConfirmStyle::Typed)) | (ReviewPhase::NeedsTyped, Some(_)) => {
            typed_matched
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReviewEscape {
    /// Nothing was sent: drop the review and close.
    Close,
    /// The journal or confirmation is not dispatched yet: revoke it.
    CancelBeforeDispatch,
    /// Dispatched: Esc never cancels; the dialog offers Stop instead.
    Ignore,
}
pub(super) fn review_escape(phase: ReviewPhase) -> ReviewEscape {
    match phase {
        ReviewPhase::Preparing | ReviewPhase::Ready | ReviewPhase::Failed => ReviewEscape::Close,
        ReviewPhase::Saving | ReviewPhase::AutoConfirming | ReviewPhase::NeedsTyped => {
            ReviewEscape::CancelBeforeDispatch
        }
        ReviewPhase::Applying => ReviewEscape::Ignore,
    }
}

/// Footer label and variant. Protected says what the click does; Typed with
/// deletes is the one destructive styling.
pub(super) fn apply_label(
    policy: &TablePolicy,
    phase: ReviewPhase,
    operations: usize,
    deletes: bool,
) -> (String, Variant) {
    let style = policy.confirm_style();
    let label = match phase {
        ReviewPhase::Saving | ReviewPhase::AutoConfirming | ReviewPhase::Applying => {
            "Applying…".to_owned()
        }
        ReviewPhase::NeedsTyped => "Confirm and apply".to_owned(),
        _ if style == Some(ConfirmStyle::Plain) && policy.expects_backend_confirmation() => {
            "Confirm and apply".to_owned()
        }
        _ => format!(
            "Apply {operations} change{}",
            if operations == 1 { "" } else { "s" }
        ),
    };
    let variant = if style == Some(ConfirmStyle::Typed) && deletes {
        Variant::Danger
    } else {
        Variant::Primary
    };
    (label, variant)
}

fn statements(preview: &PreviewResult) -> Vec<ReviewStatement> {
    preview
        .statements
        .iter()
        .map(|statement| ReviewStatement {
            sql: statement.sql.clone().into(),
            params: statement
                .params
                .iter()
                .enumerate()
                .map(|(index, param)| {
                    (
                        SharedString::from(format_param(index, param)),
                        SharedString::from(param_label(index, param)),
                    )
                })
                .collect(),
        })
        .collect()
}

impl TableChanges {
    pub(super) fn review_dialog(&self) -> Option<&ReviewDialog> {
        match &self.dialog {
            Some(Dialog::Review(dialog)) => Some(dialog),
            _ => None,
        }
    }
    fn review_dialog_mut(&mut self) -> Option<&mut ReviewDialog> {
        match &mut self.dialog {
            Some(Dialog::Review(dialog)) => Some(dialog),
            _ => None,
        }
    }
    pub(super) fn review_phase(&self) -> ReviewPhase {
        review_phase(
            self.review_dialog()
                .is_some_and(|dialog| dialog.failure.is_some()),
            self.reviewing.is_some(),
            self.review.is_some(),
            self.applying.as_ref().map(|pending| &pending.flow),
            self.applying
                .as_ref()
                .map_or(0, |pending| pending.auto_confirms),
        )
    }
    pub(super) fn typed_matched(&self, cx: &App) -> bool {
        self.review_dialog()
            .and_then(|dialog| dialog.typed.as_ref())
            .is_some_and(|typed| typed.read(cx).matches())
    }
    /// Every save goes through here. An open inline edit is staged first, so
    /// ⌘S includes the value being typed.
    pub(super) fn open_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.dialog, Some(Dialog::Review(_))) {
            return;
        }
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| edit.presentation == Presentation::Inline)
        {
            self.stage(Advance::Stay, window, cx);
        }
        if self.edit.is_some() {
            if self.message.is_empty() {
                self.message = "Save or cancel the open editor before reviewing".into();
            }
            cx.notify();
            return;
        }
        if let Err(reason) = self.review_gate() {
            self.message = reason.to_string();
            cx.notify();
            return;
        }
        if self.modal_open() {
            self.message = "Close the open dialog first".into();
            cx.notify();
            return;
        }
        self.dialog = None;
        self.request_review(cx);
        if self.reviewing.is_some() {
            self.dialog = Some(Dialog::Review(ReviewDialog::default()));
            self.focus_request = true;
        }
        cx.notify();
    }
    /// The backend review arrived: build the diff and SQL from exactly what
    /// will be sent.
    pub(super) fn reviewed(&mut self, plan: &ReviewPlan, preview: &PreviewResult) {
        let diff = review_diff(plan.plan());
        let summary = diff_summary(&diff);
        let deletes = diff.iter().any(|change| change.kind == DiffKind::Delete);
        let operations = plan.plan().operations.len();
        let statements = statements(preview);
        if let Some(dialog) = self.review_dialog_mut() {
            dialog.diff = diff;
            dialog.summary = summary;
            dialog.deletes = deletes;
            dialog.operations = operations;
            dialog.statements = statements;
            dialog.failure = None;
        }
        self.focus_request = true;
    }
    pub(super) fn review_failed(&mut self, message: String) {
        if let Some(dialog) = self.review_dialog_mut() {
            dialog.failure = Some(message.clone());
        }
        self.message = message;
    }
    /// Apply or Confirm. The gate is recomputed from the live policy and the
    /// typed field; the rendered button state never authorizes by itself.
    pub(super) fn apply_clicked(&mut self, cx: &mut Context<Self>) {
        let phase = self.review_phase();
        let typed = self.typed_matched(cx);
        if !apply_enabled(
            self.policy.confirm_style(),
            typed,
            phase,
            self.policy.read_only,
        ) {
            if let Some(reason) = self.policy.read_only_reason() {
                self.message = reason.into();
            } else if matches!(phase, ReviewPhase::Ready | ReviewPhase::NeedsTyped) {
                self.message = "Type confirm to apply".into();
            }
            cx.notify();
            return;
        }
        match phase {
            ReviewPhase::Ready => {
                let preconfirmed = preconfirmation(&self.policy, typed);
                self.prepare_apply(preconfirmed, cx);
            }
            ReviewPhase::NeedsTyped => self.send_confirmation(cx),
            _ => {}
        }
        cx.notify();
    }
    /// Releases the held confirmation through the journal: PersistApply first,
    /// then `apply_saved` sends `TableCommand::Confirm`.
    fn send_confirmation(&mut self, cx: &mut Context<Self>) {
        if !self
            .applying
            .as_ref()
            .is_some_and(|pending| pending.flow.confirming())
        {
            return;
        }
        let id = self.next();
        let pending = self.applying.as_mut().expect("confirming apply");
        if pending.flow.confirm(id) {
            self.message = "Saving change recovery record".into();
            cx.emit(ChangesEvent::PersistApply(id));
        }
    }
    /// Safe Mode refused the apply with a one-use confirmation token.
    pub(super) fn needs_confirmation(&mut self, token: Token, cx: &mut Context<Self>) {
        let step = {
            let pending = self.applying.as_mut().expect("dispatched apply");
            pending.flow.needs_confirmation(token);
            on_needs_confirmation(pending.preconfirmed, pending.auto_confirms)
        };
        match step {
            ConfirmationStep::AutoConfirm => {
                let id = self.next();
                let pending = self.applying.as_mut().expect("confirming apply");
                if pending.flow.confirm(id) {
                    pending.auto_confirms += 1;
                    self.message = "Confirming these exact changes with Safe Mode".into();
                    cx.emit(ChangesEvent::PersistApply(id));
                }
            }
            ConfirmationStep::AskTyped => {
                // The UI's policy was stale (or this is a second request):
                // never confirm automatically; ask for the typed word.
                if self.review_dialog().is_none() {
                    self.dialog = Some(Dialog::Review(ReviewDialog::default()));
                }
                // Show exactly what the held confirmation will run.
                let confirmed =
                    self.applying
                        .as_ref()
                        .and_then(|pending| match pending.flow.token() {
                            Some(Token::Confirmation(confirmation)) => {
                                Some(statements(confirmation.preview()))
                            }
                            _ => None,
                        });
                if let Some(dialog) = self.review_dialog_mut() {
                    dialog.escalated = true;
                    dialog.typed = None;
                    dialog.typed_events = None;
                    if let Some(statements) = confirmed {
                        dialog.statements = statements;
                    }
                }
                self.focus_request = true;
                self.message =
                    "Safe Mode requires confirmation. Type confirm to apply these exact changes"
                        .into();
            }
        }
    }
    /// Cancel or Esc: only before dispatch. A dispatched apply shows Stop.
    pub(super) fn cancel_review(&mut self, cx: &mut Context<Self>) {
        match review_escape(self.review_phase()) {
            ReviewEscape::Close => {
                self.discard_review();
                self.dialog = None;
                self.return_focus(cx);
            }
            ReviewEscape::CancelBeforeDispatch => {
                if let Some(pending) = &mut self.applying
                    && pending.flow.cancel_before_dispatch()
                {
                    self.cancel_pending(
                        "Apply cancelled before dispatch; changes retained".into(),
                        cx,
                    );
                    self.dialog = None;
                }
            }
            ReviewEscape::Ignore => {
                self.message = "Applying. Use Stop to cancel the database operation".into();
            }
        }
        cx.notify();
    }
    /// A typed field exists whenever the live policy is Typed or the backend
    /// escalated. Created lazily because replies arrive without a window.
    fn ensure_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wants = self.policy.confirm_style() == Some(ConfirmStyle::Typed)
            || self.review_phase() == ReviewPhase::NeedsTyped;
        if !wants
            || self
                .review_dialog()
                .is_none_or(|dialog| dialog.typed.is_some())
        {
            return;
        }
        let typed = cx.new(|cx| TypedConfirm::new(window, cx));
        let events = cx.subscribe_in(
            &typed,
            window,
            |this, _, event: &TypedConfirmEvent, window, cx| match event {
                TypedConfirmEvent::Submit => this.activate(Action::Apply, window, cx),
                TypedConfirmEvent::Changed(_) => cx.notify(),
            },
        );
        if let Some(dialog) = self.review_dialog_mut() {
            dialog.typed = Some(typed);
            dialog.typed_events = Some(events);
        }
        self.focus_request = true;
    }
    pub(super) fn render_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_typed(window, cx);
        let phase = self.review_phase();
        let policy = self.policy;
        let style = policy.confirm_style();
        let enabled = apply_enabled(style, self.typed_matched(cx), phase, policy.read_only);
        let Some(dialog) = self.review_dialog() else {
            return div().into_any_element();
        };
        let typed = dialog.typed.clone();
        let failure = dialog.failure.clone();
        let escalated = dialog.escalated;
        let (label, variant) = apply_label(&policy, phase, dialog.operations, dialog.deletes);
        let title = if dialog.summary.is_empty() {
            "Review changes".to_owned()
        } else {
            format!("Review changes · {}", dialog.summary)
        };
        let diff = diff_list(&dialog.diff);
        let sql = sql_list(&dialog.statements);

        let status = match phase {
            ReviewPhase::Preparing => Some("Preparing the exact SQL…"),
            ReviewPhase::Saving => Some("Saving the change journal…"),
            ReviewPhase::AutoConfirming => Some("Confirming with Safe Mode…"),
            ReviewPhase::Applying => Some("Applying. Only the database can settle this now."),
            _ => None,
        };
        let mut body = dialog::body("review-body")
            .child(dialog::env_notice(
                policy.environment,
                policy.describe(),
                style == Some(ConfirmStyle::Typed) || escalated,
            ))
            .when(escalated, |body| {
                body.child(dialog::env_notice(
                    policy.environment,
                    "Safe Mode asked for a confirmation this review did not expect. Type confirm to apply these exact changes.",
                    true,
                ))
            })
            .when_some(failure, |body, message| {
                body.child(ui::shake(
                    ("review-failure", message.len()),
                    ui::error_banner("review-failure", message),
                ))
            })
            .child(diff)
            .child(sql);
        if let Some(typed) = typed {
            body = body.child(typed);
        }
        if policy.read_only {
            body = body.child(ui::error_banner(
                "review-read-only",
                policy.read_only_reason().unwrap_or_default(),
            ));
        }

        let mut footer = dialog::footer().child(
            div()
                .id("review-status")
                .role(Role::Status)
                .aria_label(status.unwrap_or_default())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(style::dim())
                .child(status.unwrap_or_default()),
        );
        footer = match phase {
            ReviewPhase::Applying => footer.child(self.dialog_button(
                "review-stop",
                "Stop",
                Action::CancelPending,
                Variant::Secondary,
                true,
                cx,
            )),
            ReviewPhase::Failed => footer
                .child(self.dialog_button(
                    "review-close",
                    "Close",
                    Action::CloseDialog,
                    Variant::Ghost,
                    true,
                    cx,
                ))
                .child(self.dialog_button(
                    "review-again",
                    "Review again",
                    Action::ReviewAgain,
                    Variant::Secondary,
                    !self.pending(),
                    cx,
                )),
            _ => {
                let action = if phase == ReviewPhase::NeedsTyped {
                    Action::Confirm
                } else {
                    Action::Apply
                };
                footer
                    .child(self.dialog_button(
                        "review-cancel",
                        "Cancel",
                        Action::CancelReview,
                        Variant::Ghost,
                        true,
                        cx,
                    ))
                    .child(self.dialog_button("review-apply", &label, action, variant, enabled, cx))
            }
        };
        let modal = dialog::modal("review-dialog", "Review changes", 640.)
            .child(dialog::header(title, Some(env_chip(policy.environment))))
            .child(body)
            .child(footer);
        dialog::backdrop("review-backdrop")
            .child(ui::appear(
                "review-dialog-appear",
                self.modal_keys(modal, cx),
            ))
            .into_any_element()
    }
}

fn env_chip(environment: Option<DevelopmentEnvironment>) -> AnyElement {
    let color = style::env(environment);
    let label = environment.map_or("Unknown environment", style::env_label);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .h(px(18.))
        .px(px(6.))
        .rounded(px(9.))
        .border_1()
        .border_color(style::with_alpha(color, 0x73))
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
        .child(
            div()
                .size(px(6.))
                .rounded_full()
                .bg(style::with_alpha(color, 0xff)),
        )
        .child(label)
        .into_any_element()
}

/// Display text and whether it is a placeholder (NULL, or `''` for an empty
/// string, matching the grid) rather than the value itself.
fn value_text(value: &DiffValue) -> (String, bool) {
    match &value.text {
        Some(text) if text.is_empty() => ("''".to_owned(), true),
        Some(text) => (
            format!("{text}{}", if value.truncated { "…" } else { "" }),
            false,
        ),
        None => ("NULL".to_owned(), true),
    }
}

fn value_cell(value: &DiffValue, color: gpui::Rgba, struck: bool) -> gpui::Div {
    let (text, null) = value_text(value);
    div()
        .min_w_0()
        .truncate()
        .font_family(style::MONO)
        .text_color(if null { style::faint() } else { color })
        .when(null, |cell| cell.italic())
        .when(struck, |cell| cell.line_through())
        .child(text)
}

fn cell_row(index: usize, kind: DiffKind, cell: &DiffCell) -> AnyElement {
    let old = cell.old.as_ref();
    let new = cell.new.as_ref();
    let label = format!(
        "{}: {}{}",
        cell.column,
        old.map_or(String::new(), |value| format!(
            "{} to ",
            value_text(value).0
        )),
        new.map_or_else(|| "removed".to_owned(), |value| value_text(value).0),
    );
    let mut row = div()
        .id(("review-cell", index))
        .role(Role::Label)
        .aria_label(label)
        .flex()
        .items_center()
        .gap(px(6.))
        .min_h(px(style::ROW))
        .child(
            div()
                .flex_none()
                .w(px(140.))
                .truncate()
                .font_family(style::MONO)
                .text_color(style::dim())
                .child(cell.column.clone()),
        );
    match (kind, old, new) {
        (DiffKind::Delete, Some(old), _) => {
            row = row.child(value_cell(old, style::bad_text(), true));
        }
        (_, Some(old), Some(new)) => {
            row = row
                .child(value_cell(old, style::faint(), true))
                .child(div().flex_none().text_color(style::faint()).child("→"))
                .child(value_cell(new, style::warn(), false));
        }
        (_, None, Some(new)) => row = row.child(value_cell(new, style::warn(), false)),
        (_, Some(old), None) => row = row.child(value_cell(old, style::bad_text(), true)),
        (_, None, None) => {}
    }
    row.into_any_element()
}

fn diff_list(changes: &[DiffChange]) -> AnyElement {
    let mut list = div()
        .id("review-diff")
        .role(Role::List)
        .aria_label("Changes")
        .flex()
        .flex_col()
        .gap(px(6.));
    if changes.is_empty() {
        return list.into_any_element();
    }
    for (index, change) in changes.iter().take(DIFF_CHANGES).enumerate() {
        let (verb, tint) = match change.kind {
            DiffKind::Update => ("Update", style::warn()),
            DiffKind::Insert => ("Insert", style::ok()),
            DiffKind::Delete => ("Delete", style::bad()),
        };
        let target = if change.identity.is_empty() {
            change.target.clone()
        } else {
            format!("{} · {}", change.target, change.identity)
        };
        let more = change.cells.len().saturating_sub(DIFF_CELLS);
        let item = div()
            .id(("review-change", index))
            .role(Role::ListItem)
            .aria_label(format!("Change {}: {verb} {target}", change.op_index + 1))
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(5.))
            .border_1()
            .border_color(style::line_soft())
            .bg(style::bg())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(tint)
                            .child(verb),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(style::MONO)
                            .text_color(style::dim())
                            .child(target),
                    ),
            )
            .children(
                change
                    .cells
                    .iter()
                    .take(DIFF_CELLS)
                    .enumerate()
                    .map(|(cell_index, cell)| cell_row(cell_index, change.kind, cell)),
            )
            .when(more > 0, |item| {
                item.child(
                    div()
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::faint())
                        .child(format!("+{more} more columns, shown in the SQL below")),
                )
            })
            .when(change.omitted_defaults, |item| {
                item.child(
                    div()
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::faint())
                        .child("Other columns use their defaults"),
                )
            });
        list = list.child(item);
    }
    list.into_any_element()
}

fn sql_list(statements: &[ReviewStatement]) -> AnyElement {
    let count = statements.len();
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(ui::section_label(format!(
            "SQL · {count} statement{}",
            if count == 1 { "" } else { "s" }
        )))
        .children(statements.iter().enumerate().map(|(index, statement)| {
            div()
                .id(("review-statement", index))
                .role(Role::Group)
                .aria_label(format!("Statement {}", index + 1))
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(8.))
                .py(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(style::line_soft())
                .bg(style::bg())
                .font_family(style::MONO)
                .child(
                    div()
                        .id("sql")
                        .role(Role::Label)
                        .aria_label(statement.sql.clone())
                        .text_color(style::text())
                        .child(statement.sql.clone()),
                )
                .children(
                    statement
                        .params
                        .iter()
                        .enumerate()
                        .map(|(param, (line, label))| {
                            div()
                                .id(("param", param))
                                .role(Role::Label)
                                .aria_label(label.clone())
                                .truncate()
                                .text_color(style::dim())
                                .child(line.clone())
                        }),
                )
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::DevelopmentSafeMode;

    #[test]
    fn diff_values_keep_null_and_empty_string_distinct() {
        let value = |text: Option<&str>, truncated| DiffValue {
            text: text.map(str::to_owned),
            truncated,
        };
        assert_eq!(value_text(&value(None, false)), ("NULL".into(), true));
        assert_eq!(value_text(&value(Some(""), false)), ("''".into(), true));
        assert_eq!(value_text(&value(Some("a"), false)), ("a".into(), false));
        assert_eq!(value_text(&value(Some("ab"), true)), ("ab…".into(), false));
    }

    const PHASES: [ReviewPhase; 7] = [
        ReviewPhase::Preparing,
        ReviewPhase::Ready,
        ReviewPhase::Saving,
        ReviewPhase::AutoConfirming,
        ReviewPhase::Applying,
        ReviewPhase::NeedsTyped,
        ReviewPhase::Failed,
    ];

    #[test]
    fn apply_gate_truth_table() {
        let styles = [None, Some(ConfirmStyle::Plain), Some(ConfirmStyle::Typed)];
        for phase in PHASES {
            for style in styles {
                for typed in [false, true] {
                    for read_only in [false, true] {
                        let expected = !read_only
                            && match (phase, style) {
                                (_, None) => false,
                                (ReviewPhase::Ready, Some(ConfirmStyle::Plain)) => true,
                                (ReviewPhase::Ready, Some(ConfirmStyle::Typed)) => typed,
                                (ReviewPhase::NeedsTyped, Some(_)) => typed,
                                _ => false,
                            };
                        assert_eq!(
                            apply_enabled(style, typed, phase, read_only),
                            expected,
                            "{phase:?} {style:?} typed={typed} read_only={read_only}"
                        );
                    }
                }
            }
        }
        // Spot checks that encode the safety rules directly.
        assert!(!apply_enabled(
            Some(ConfirmStyle::Typed),
            false,
            ReviewPhase::Ready,
            false
        ));
        assert!(apply_enabled(
            Some(ConfirmStyle::Typed),
            true,
            ReviewPhase::Ready,
            false
        ));
        assert!(!apply_enabled(
            Some(ConfirmStyle::Plain),
            true,
            ReviewPhase::Ready,
            true
        ));
        // An escalated plain policy still needs the typed word.
        assert!(!apply_enabled(
            Some(ConfirmStyle::Plain),
            false,
            ReviewPhase::NeedsTyped,
            false
        ));
        for phase in [
            ReviewPhase::Preparing,
            ReviewPhase::Saving,
            ReviewPhase::AutoConfirming,
            ReviewPhase::Applying,
            ReviewPhase::Failed,
        ] {
            assert!(!apply_enabled(
                Some(ConfirmStyle::Plain),
                true,
                phase,
                false
            ));
        }
    }

    #[test]
    fn saving_apply_cancels_before_dispatch_but_dispatched_apply_ignores_escape() {
        let mut flow = ApplyFlow::new(7, "reviewed");
        let phase = review_phase(false, false, false, Some(&flow), 0);
        assert_eq!(phase, ReviewPhase::Saving);
        assert_eq!(review_escape(phase), ReviewEscape::CancelBeforeDispatch);
        assert!(flow.cancel_before_dispatch());
        assert_eq!(flow.saved(7), None, "a cancelled journal never releases");

        let mut flow = ApplyFlow::new(8, "reviewed");
        assert_eq!(flow.saved(8), Some("reviewed"));
        let phase = review_phase(false, false, false, Some(&flow), 0);
        assert_eq!(phase, ReviewPhase::Applying);
        assert_eq!(review_escape(phase), ReviewEscape::Ignore);
        assert!(!flow.cancel_before_dispatch());

        // A held confirmation was never sent, so it may be revoked.
        flow.needs_confirmation("confirmation");
        let phase = review_phase(false, false, false, Some(&flow), 0);
        assert_eq!(phase, ReviewPhase::NeedsTyped);
        assert_eq!(review_escape(phase), ReviewEscape::CancelBeforeDispatch);
        // The automatic confirmation's journal save is cancellable too.
        assert!(flow.confirm(9));
        let phase = review_phase(false, false, false, Some(&flow), 1);
        assert_eq!(phase, ReviewPhase::AutoConfirming);
        assert_eq!(review_escape(phase), ReviewEscape::CancelBeforeDispatch);
    }

    #[test]
    fn phase_without_an_apply_follows_failure_then_review_state() {
        let none: Option<&ApplyFlow<()>> = None;
        assert_eq!(
            review_phase(false, true, false, none, 0),
            ReviewPhase::Preparing
        );
        assert_eq!(
            review_phase(false, false, true, none, 0),
            ReviewPhase::Ready
        );
        assert_eq!(
            review_phase(true, false, true, none, 0),
            ReviewPhase::Failed
        );
        assert_eq!(review_escape(ReviewPhase::Preparing), ReviewEscape::Close);
        assert_eq!(review_escape(ReviewPhase::Failed), ReviewEscape::Close);
    }

    #[test]
    fn apply_label_names_the_confirmation_and_marks_typed_deletes_as_danger() {
        let protected = TablePolicy::resolve(
            DevelopmentEnvironment::Staging,
            DevelopmentSafeMode::Inherit,
            false,
        );
        let disabled = TablePolicy::resolve(
            DevelopmentEnvironment::Development,
            DevelopmentSafeMode::Inherit,
            false,
        );
        let production = TablePolicy::resolve(
            DevelopmentEnvironment::Production,
            DevelopmentSafeMode::Inherit,
            false,
        );
        assert_eq!(
            apply_label(&protected, ReviewPhase::Ready, 2, true),
            ("Confirm and apply".to_owned(), Variant::Primary)
        );
        assert_eq!(
            apply_label(&disabled, ReviewPhase::Ready, 1, true),
            ("Apply 1 change".to_owned(), Variant::Primary)
        );
        assert_eq!(
            apply_label(&production, ReviewPhase::Ready, 3, true),
            ("Apply 3 changes".to_owned(), Variant::Danger)
        );
        assert_eq!(
            apply_label(&production, ReviewPhase::Ready, 3, false).1,
            Variant::Primary
        );
        assert_eq!(
            apply_label(&disabled, ReviewPhase::NeedsTyped, 1, false).0,
            "Confirm and apply"
        );
    }
}
