//! Plan 033 review and apply for the structure draft editor, on the
//! row-change review shell (`table_changes/review.rs`): the changes grouped by
//! Columns, Keys, Foreign keys, Checks, Indexes and Table with field-level
//! old → new; the impact (destructive items, engine notes, inbound foreign
//! keys and dependents); the exact SQL per group in a read-only editor; and
//! the environment-aware gate. Apply journals the attempt before dispatch
//! (`ApplyFlow`: Saving → Dispatched or Confirmation); the backend
//! NeedsConfirmation stays the enforcement boundary (ADR-0004).
use crate::{
    accessible_editor::AccessibleEditor,
    apply_flow::ApplyFlow,
    data_model::{
        ConfirmStyle, ConfirmationStep, Preconfirmation, on_needs_confirmation, preconfirmation,
    },
    schema_edit_model::{
        OutcomeText, PlanToggle, ReviewEscape, ReviewPhase, Settlement, apply_enabled, apply_label,
        note_label, note_warns, outcome_text, plan_toggles, review_escape, review_phase,
        sql_document, toggle, toggle_on,
    },
    schema_edit_view::{SchemaEditEvent, SchemaEditView},
    style,
    ui::{
        self, Variant,
        confirm::{TypedConfirm, TypedConfirmEvent},
        dialog,
    },
};
use dbunk_lib::backend::DevelopmentEnvironment;
use dbunk_lib::backend::schema_edit::{
    ChangeGroup, ChangeKind, Dependent, InboundReference, PlanOptions, ReviewRefusal, SchemaChange,
    SchemaDraft, SchemaEditConfirmation, SchemaEditError, SchemaEditOutcome, SchemaEditPreview,
    SchemaEditReceipt, SchemaEditReview, SchemaEditSubmission, SchemaEditTarget,
};
use editor::Editor;
use gpui::{
    AnyElement, Context, Entity, Role, SharedString, Subscription, Window, div, prelude::*, px,
};

pub mod runtime;

use runtime::SchemaEditCall;

/// The single-use authority an apply holds until the backend consumes it.
pub(crate) enum Token {
    Review(Box<SchemaEditReview>),
    Confirmation(Box<SchemaEditConfirmation>),
}

/// The open review dialog. It stays open from Review until the outcome is
/// read and the dialog closed, or until it is cancelled before dispatch.
pub(crate) struct ReviewState {
    /// Held until Apply moves it into the flow; Cancel gives its target back.
    pub(crate) review: Option<Box<SchemaEditReview>>,
    /// A copy of what was reviewed, shown while the token is in flight.
    pub(crate) preview: SchemaEditPreview,
    /// Distinct objects the preview changes, counted once per review.
    pub(crate) objects: usize,
    pub(crate) inbound: Vec<InboundReference>,
    pub(crate) dependents: Vec<Dependent>,
    pub(crate) estimated_rows: Option<u64>,
    pub(crate) sql: Entity<AccessibleEditor>,
    _editor: Entity<Editor>,
    pub(crate) flow: Option<ApplyFlow<Token>>,
    /// What the user's click acknowledged, recorded before dispatch.
    pub(crate) preconfirmed: Preconfirmation,
    /// Backend confirmations sent automatically for this apply (at most one).
    pub(crate) auto_confirms: u32,
    /// Fresh for every review; recreated empty when confirmation escalates.
    pub(crate) typed: Option<Entity<TypedConfirm>>,
    pub(crate) typed_events: Option<Subscription>,
    /// The backend asked for a confirmation the click did not acknowledge.
    pub(crate) escalated: bool,
    pub(crate) failure: Option<String>,
    pub(crate) outcome: Option<OutcomeText>,
    pub(crate) settlement: Option<Settlement>,
}

impl ReviewState {
    pub(crate) fn phase(&self) -> ReviewPhase {
        review_phase(
            self.flow.as_ref(),
            self.auto_confirms,
            self.review.is_some(),
            self.settlement.is_some(),
        )
    }
}

fn sql_editor(
    text: String,
    window: &mut Window,
    cx: &mut Context<SchemaEditView>,
) -> (Entity<Editor>, Entity<AccessibleEditor>) {
    let buffer = cx.new(|cx| {
        let mut buffer = language::Buffer::local(text, cx);
        if let Ok(language) = crate::sql::language(cx) {
            buffer.set_language(Some(language), cx);
        }
        buffer
    });
    let editor = cx.new(|cx| {
        let mut editor = Editor::for_buffer(buffer, None, window, cx);
        editor.set_read_only(true);
        editor
    });
    let accessible =
        cx.new(|cx| AccessibleEditor::new(editor.clone(), "Exact SQL to apply, read only", cx));
    (editor, accessible)
}

impl SchemaEditView {
    /// Review N / ⌘S: plans the draft against the held target. Planning is
    /// synchronous; a refusal gives the target back for further editing.
    pub(crate) fn open_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.review.is_some() {
            return;
        }
        if let Some(reason) = self.review_block() {
            self.fail(reason);
            cx.notify();
            return;
        }
        // The journal is coalesced while editing; never review ahead of it.
        self.flush_journal(cx);
        let Some((draft, options)) = self
            .model
            .as_ref()
            .map(|model| (model.draft().clone(), model.plan_options()))
        else {
            return;
        };
        let Some(target) = self.target.take() else {
            return;
        };
        self.review_with(target, draft, options, window, cx);
    }

    fn review_with(
        &mut self,
        target: SchemaEditTarget,
        draft: SchemaDraft,
        options: PlanOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.host.backend.review_schema_edit(target, draft, options) {
            Ok(review) => {
                let preview = review.preview().clone();
                let objects = preview.changes.object_count();
                let baseline = review.baseline();
                let inbound = baseline.inbound.clone();
                let dependents = baseline.dependents.clone();
                let estimated_rows = baseline.estimated_rows;
                let (editor, sql) = sql_editor(sql_document(&preview), window, cx);
                self.review = Some(ReviewState {
                    review: Some(Box::new(review)),
                    preview,
                    objects,
                    inbound,
                    dependents,
                    estimated_rows,
                    sql,
                    _editor: editor,
                    flow: None,
                    preconfirmed: Preconfirmation::NotGranted,
                    auto_confirms: 0,
                    typed: None,
                    typed_events: None,
                    escalated: false,
                    failure: None,
                    outcome: None,
                    settlement: None,
                });
                self.message = "Review the exact SQL; nothing is sent until Apply".into();
            }
            Err(refusal) => {
                let ReviewRefusal { target, error } = *refusal;
                self.target = Some(target);
                let text = format!("Review refused: {error}");
                match &mut self.review {
                    Some(state) => {
                        state.review = None;
                        state.failure = Some(text);
                    }
                    None => self.fail(text),
                }
            }
        }
        self.inputs_stale = true;
        self.focus_request = true;
        self.publish(cx);
    }

    /// Flips a planner option and re-plans; only before Apply.
    fn set_option(&mut self, choice: PlanToggle, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = &mut self.review else {
            return;
        };
        if state.phase() != ReviewPhase::Ready {
            return;
        }
        let Some(review) = state.review.take() else {
            return;
        };
        let options = toggle(&review.preview().options, choice, &review.preview().changes);
        let draft = review.draft().clone();
        let review: SchemaEditReview = *review;
        let target = review.into_target();
        self.review_with(target, draft, options, window, cx);
    }

    fn typed_matched(&self, cx: &gpui::App) -> bool {
        self.review
            .as_ref()
            .and_then(|state| state.typed.as_ref())
            .is_some_and(|typed| typed.read(cx).matches())
    }

    /// Apply or Confirm. The gate is recomputed from the live policy and the
    /// typed field; the rendered button state never authorizes by itself.
    pub(crate) fn apply_clicked(&mut self, cx: &mut Context<Self>) {
        let Some(phase) = self.review.as_ref().map(ReviewState::phase) else {
            return;
        };
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

    /// Journals the exact attempt as `OutcomeUnknown`, then waits for the
    /// save (PostgreSQL) before `apply_saved` dispatches it.
    fn prepare_apply(&mut self, preconfirmed: Preconfirmation, cx: &mut Context<Self>) {
        let id = match self.next_id() {
            Ok(id) => id,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let Some(state) = &mut self.review else {
            return;
        };
        let Some(review) = state.review.take() else {
            return;
        };
        if let Err(error) = self.recovery.stage(&review) {
            state.review = Some(review);
            state.failure = Some(error.into());
            self.message = error.into();
            return;
        }
        state.flow = Some(ApplyFlow::new(id, Token::Review(review)));
        state.preconfirmed = preconfirmed;
        state.auto_confirms = 0;
        state.failure = None;
        self.message = "Saving the exact attempt before dispatch".into();
        self.persist_or_proceed(id, cx);
    }

    fn persist_or_proceed(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.persist {
            self.publish(cx);
            cx.emit(SchemaEditEvent::PersistApply(id));
        } else {
            // MySQL and SQLite documents are not journaled; the flow still
            // fences the single dispatch.
            self.apply_saved(id, Ok(()), cx);
        }
    }

    /// The workspace saved (or refused) the revision holding the attempt.
    pub fn apply_saved(&mut self, id: u64, result: Result<(), String>, cx: &mut Context<Self>) {
        let waiting = self
            .review
            .as_ref()
            .and_then(|state| state.flow.as_ref())
            .is_some_and(|flow| flow.waiting_for(id));
        if !waiting {
            return;
        }
        if let Err(error) = result {
            self.revoke_before_dispatch();
            self.set_failure(format!("Structure change not sent: {error}"));
            self.publish(cx);
            return;
        }
        if !self.recovery.unknown() {
            self.revoke_before_dispatch();
            self.set_failure("The uncertain attempt was not preserved; nothing was sent".into());
            self.publish(cx);
            return;
        }
        let Some(token) = self
            .review
            .as_mut()
            .and_then(|state| state.flow.as_mut())
            .and_then(|flow| flow.saved(id))
        else {
            return;
        };
        let exact = match &token {
            Token::Review(review) => self.recovery.matches_review(review),
            Token::Confirmation(confirmation) => {
                self.recovery.matches_review(confirmation.review())
            }
        };
        if !exact {
            if let Some(state) = &mut self.review {
                state.flow = None;
                state.settlement = Some(Settlement::Unknown);
                state.outcome = Some(OutcomeText {
                    text: "The saved attempt no longer matches this review; nothing was sent. The recovery record stays unknown until reconciled.".into(),
                    error: true,
                });
            }
            self.publish(cx);
            return;
        }
        let call = match token {
            Token::Review(review) => SchemaEditCall::Apply(review),
            Token::Confirmation(confirmation) => SchemaEditCall::Confirm(confirmation),
        };
        match self.send(id, call, cx) {
            Ok(()) => {
                self.message = "Saved; the change is dispatched. Waiting for its outcome.".into()
            }
            Err(error) => {
                self.recovery.not_sent();
                self.journal();
                if let Some(state) = &mut self.review {
                    state.flow = None;
                    state.settlement = Some(Settlement::Unchanged);
                    state.outcome = Some(OutcomeText {
                        text: format!("Not sent: {error}"),
                        error: true,
                    });
                }
            }
        }
        self.publish(cx);
    }

    fn set_failure(&mut self, text: String) {
        if let Some(state) = &mut self.review {
            state.failure = Some(text.clone());
        }
        self.fail(text);
    }

    /// Revokes a save fence or a held confirmation that was never sent. The
    /// review token goes with it, so a new review needs a fresh capture.
    pub(crate) fn revoke_before_dispatch(&mut self) -> bool {
        let Some(state) = &mut self.review else {
            return false;
        };
        if !state
            .flow
            .as_mut()
            .is_some_and(ApplyFlow::cancel_before_dispatch)
        {
            return false;
        }
        state.flow = None;
        state.failure = Some("Cancelled before dispatch; nothing was sent".into());
        self.recovery.not_sent();
        self.journal();
        true
    }

    pub(crate) fn applied(
        &mut self,
        id: u64,
        result: Result<SchemaEditSubmission, SchemaEditError>,
        cx: &mut Context<Self>,
    ) {
        let dispatched = self
            .review
            .as_ref()
            .and_then(|state| state.flow.as_ref())
            .is_some_and(|flow| flow.id() == id && flow.dispatched());
        if !dispatched {
            return;
        }
        match result {
            Ok(SchemaEditSubmission::NeedsConfirmation(confirmation)) => {
                self.needs_confirmation(confirmation, cx)
            }
            Ok(SchemaEditSubmission::Finished(receipt)) => self.finished(*receipt, cx),
            Err(error) => {
                self.recovery.submission_error(&error);
                let unknown = error == SchemaEditError::OutcomeUnavailable;
                if let Some(state) = &mut self.review {
                    state.flow = None;
                    state.settlement = Some(if unknown {
                        Settlement::Unknown
                    } else {
                        Settlement::Unchanged
                    });
                    state.outcome = Some(OutcomeText {
                        text: if unknown {
                            format!(
                                "Outcome unknown: {error}. Inspect the table and reload its structure; this change is never replayed."
                            )
                        } else {
                            format!("Not applied: {error}")
                        },
                        error: true,
                    });
                }
                self.target = None;
                self.publish(cx);
                if unknown {
                    cx.emit(SchemaEditEvent::StructureChanged);
                }
            }
        }
    }

    /// Safe Mode refused with a one-use confirmation token. Nothing was sent.
    fn needs_confirmation(
        &mut self,
        confirmation: Box<SchemaEditConfirmation>,
        cx: &mut Context<Self>,
    ) {
        if !self.recovery.matches_review(confirmation.review()) {
            // Safe Mode refused before executing anything, so this is not an
            // unknown outcome. The mismatched token is dropped unused and the
            // draft is kept; closing takes a fresh capture to review again.
            self.recovery.not_sent();
            self.journal();
            self.target = None;
            if let Some(state) = &mut self.review {
                state.flow = None;
                state.review = None;
            }
            self.set_failure(
                "The confirmation does not match the saved attempt; nothing was sent. Close and review again."
                    .into(),
            );
            self.publish(cx);
            return;
        }
        self.recovery.not_sent();
        let mut auto = false;
        if let Some(state) = &mut self.review
            && let Some(flow) = &mut state.flow
        {
            flow.needs_confirmation(Token::Confirmation(confirmation));
            match on_needs_confirmation(state.preconfirmed, state.auto_confirms) {
                ConfirmationStep::AutoConfirm => {
                    state.auto_confirms += 1;
                    auto = true;
                }
                ConfirmationStep::AskTyped => {
                    // The policy view was stale, or this is a second request:
                    // never confirm automatically; ask for the typed word.
                    state.escalated = true;
                    state.typed = None;
                    state.typed_events = None;
                    self.message =
                        "Safe Mode requires confirmation. Type confirm to apply this exact change"
                            .into();
                }
            }
        }
        if auto {
            self.message = "Confirming this exact change with Safe Mode".into();
            self.send_confirmation(cx);
        }
        self.focus_request = true;
        self.publish(cx);
    }

    /// Releases the held confirmation through the journal: stage, save, then
    /// `apply_saved` sends the confirm call.
    fn send_confirmation(&mut self, cx: &mut Context<Self>) {
        let id = match self.next_id() {
            Ok(id) => id,
            Err(error) => {
                self.fail(error);
                return;
            }
        };
        let Some(state) = &mut self.review else {
            return;
        };
        let Some(flow) = &mut state.flow else {
            return;
        };
        if !flow.confirming() {
            return;
        }
        let staged = match flow.token() {
            Some(Token::Confirmation(confirmation)) => self.recovery.stage(confirmation.review()),
            _ => Err("The confirmation is no longer available"),
        };
        if let Err(error) = staged {
            state.failure = Some(error.into());
            return;
        }
        if flow.confirm(id) {
            self.message = "Saving the confirmed attempt before dispatch".into();
            self.persist_or_proceed(id, cx);
        }
    }

    fn finished(&mut self, receipt: SchemaEditReceipt, cx: &mut Context<Self>) {
        let settlement = self.recovery.settle(&receipt, &self.request);
        let changed = receipt.outcome.may_have_changed() || settlement == Settlement::Unknown;
        if let Some(state) = &mut self.review {
            state.flow = None;
            let mut outcome = outcome_text(&receipt.outcome, state.preview.statements.len());
            if settlement == Settlement::Unknown
                && !matches!(receipt.outcome, SchemaEditOutcome::OutcomeUnknown { .. })
            {
                outcome = OutcomeText {
                    text: "The receipt does not match the saved attempt, so the outcome is unknown. Inspect the table and reload before another attempt.".into(),
                    error: true,
                };
            }
            state.outcome = Some(outcome);
            state.settlement = Some(settlement);
        }
        self.target = None;
        self.message = match settlement {
            Settlement::Applied => "Structure change applied".into(),
            Settlement::Unchanged => "Nothing changed; the draft is kept".into(),
            Settlement::Partial => "Partly applied; reload the structure".into(),
            Settlement::Unknown => "Outcome unknown; reconcile before another attempt".into(),
        };
        self.publish(cx);
        if changed {
            cx.emit(SchemaEditEvent::StructureChanged);
        }
    }

    /// Cancel or Escape: closes before dispatch; a dispatched apply cannot be
    /// cancelled from here.
    pub(crate) fn cancel_review(&mut self, cx: &mut Context<Self>) {
        let Some(phase) = self.review.as_ref().map(ReviewState::phase) else {
            return;
        };
        match review_escape(phase) {
            ReviewEscape::Close => self.close_review(cx),
            ReviewEscape::CancelBeforeDispatch => {
                if self.revoke_before_dispatch() {
                    self.message = "Cancelled before dispatch; nothing was sent".into();
                    self.close_review(cx);
                }
            }
            ReviewEscape::Ignore => {
                self.message = "Applying. Only the database can settle this now.".into();
                cx.notify();
            }
        }
    }

    /// Closes the dialog. An unused review gives its target back; a settled
    /// one takes a fresh capture (the draft carries over when the table is
    /// unchanged); an applied one leaves draft mode.
    fn close_review(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.review.take() else {
            return;
        };
        if state.flow.is_some() {
            // Never drop an in-flight token; put the dialog back.
            self.review = Some(state);
            return;
        }
        if let Some(review) = state.review {
            let review: SchemaEditReview = *review;
            self.target = Some(review.into_target());
        }
        self.inputs_stale = true;
        self.focus_request = true;
        match state.settlement {
            Some(Settlement::Applied) => {
                self.model = None;
                self.publish(cx);
                cx.emit(SchemaEditEvent::Close);
                return;
            }
            Some(Settlement::Partial) => {
                // Some statements committed: the draft no longer describes
                // a change from the table as it is. Drop it and capture the
                // table again (the journal already dropped its record).
                self.model = None;
                self.selected = crate::schema_edit_model::Target::Table;
                self.observe(cx);
            }
            Some(_) => self.observe(cx),
            None if self.target.is_none() => self.observe(cx),
            None => {}
        }
        self.publish(cx);
    }

    /// A typed field exists whenever the live policy is Typed or the backend
    /// escalated. Created lazily because replies arrive without a window.
    fn ensure_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed_policy = self.policy.confirm_style() == Some(ConfirmStyle::Typed);
        let Some(state) = &self.review else {
            return;
        };
        let wants = (typed_policy && state.phase() == ReviewPhase::Ready)
            || state.phase() == ReviewPhase::NeedsTyped;
        if !wants || state.typed.is_some() {
            return;
        }
        let typed = cx.new(|cx| TypedConfirm::new(window, cx));
        let events = cx.subscribe_in(
            &typed,
            window,
            |this, _, event: &TypedConfirmEvent, _, cx| match event {
                TypedConfirmEvent::Submit => this.apply_clicked(cx),
                TypedConfirmEvent::Changed(_) => cx.notify(),
            },
        );
        if let Some(state) = &mut self.review {
            state.typed = Some(typed);
            state.typed_events = Some(events);
        }
    }

    fn dialog_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        variant: Variant,
        enabled: bool,
        cx: &Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        ui::button(id, label, variant, enabled)
            .tab_index(0)
            .when(enabled, |button| {
                button.on_click(cx.listener(move |this, _, window, cx| action(this, window, cx)))
            })
            .into_any_element()
    }

    pub(crate) fn render_review(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.ensure_typed(window, cx);
        let policy = self.policy;
        let typed_matched = self.typed_matched(cx);
        let state = self.review.as_ref()?;
        let phase = state.phase();
        let style_kind = policy.confirm_style();
        let enabled = apply_enabled(style_kind, typed_matched, phase, policy.read_only);
        let objects = state.objects;
        let destructive = state.preview.destructive();
        let (label, danger) = apply_label(&policy, phase, objects, destructive);
        let title = format!(
            "Review structure changes · {} · {objects} object{}",
            self.request.table,
            if objects == 1 { "" } else { "s" }
        );
        let status = match phase {
            ReviewPhase::Saving => "Saving the recovery record…",
            ReviewPhase::AutoConfirming => "Confirming with Safe Mode…",
            ReviewPhase::Applying => "Applying. Only the database can settle this now.",
            ReviewPhase::NeedsTyped => "Safe Mode asks for a typed confirmation",
            _ => "",
        };
        let mut body = dialog::body("schema-review-body")
            .child(dialog::env_notice(
                policy.environment,
                policy.describe(),
                style_kind == Some(ConfirmStyle::Typed) || state.escalated,
            ))
            .when(state.escalated, |body| {
                body.child(dialog::env_notice(
                    policy.environment,
                    "Safe Mode asked for a confirmation this review did not expect. Type confirm to apply this exact change.",
                    true,
                ))
            })
            .when_some(state.failure.clone(), |body, message| {
                body.child(ui::shake(
                    ("schema-review-failure", message.len()),
                    ui::error_banner("schema-review-failure", message),
                ))
            })
            .when_some(state.outcome.clone(), |body, outcome| {
                body.child(outcome_banner(outcome))
            })
            .child(changes_list(&state.preview))
            .child(impact_list(state))
            .children(self.options(state, phase, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(ui::section_label(format!(
                        "SQL · {} statement{} in {} group{}",
                        state.preview.statements.len(),
                        if state.preview.statements.len() == 1 { "" } else { "s" },
                        state.preview.groups.len(),
                        if state.preview.groups.len() == 1 { "" } else { "s" },
                    )))
                    .child(
                        div()
                            .h(px(180.))
                            .flex_none()
                            .p(px(6.))
                            .rounded(px(5.))
                            .border_1()
                            .border_color(style::line_soft())
                            .bg(style::bg())
                            .font_family(style::MONO)
                            .child(state.sql.clone()),
                    ),
            );
        if let Some(typed) = state.typed.clone()
            && matches!(phase, ReviewPhase::Ready | ReviewPhase::NeedsTyped)
        {
            body = body.child(typed);
        }
        if let Some(reason) = policy.read_only_reason() {
            body = body.child(ui::error_banner("schema-review-read-only", reason));
        }
        let mut footer = dialog::footer().child(
            div()
                .id("schema-review-status")
                .role(Role::Status)
                .aria_label(status)
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(style::dim())
                .child(status),
        );
        footer = match phase {
            ReviewPhase::Applying => footer.child(self.dialog_button(
                "schema-review-applying",
                "Applying…",
                Variant::Secondary,
                false,
                cx,
                |_, _, _| {},
            )),
            ReviewPhase::Failed | ReviewPhase::Settled => footer.child(self.dialog_button(
                "schema-review-close",
                "Close",
                if phase == ReviewPhase::Settled {
                    Variant::Primary
                } else {
                    Variant::Ghost
                },
                true,
                cx,
                |this, _, cx| this.cancel_review(cx),
            )),
            _ => footer
                .child(self.dialog_button(
                    "schema-review-cancel",
                    "Cancel",
                    Variant::Ghost,
                    true,
                    cx,
                    |this, _, cx| this.cancel_review(cx),
                ))
                .child(self.dialog_button(
                    "schema-review-apply",
                    label,
                    if danger {
                        Variant::Danger
                    } else {
                        Variant::Primary
                    },
                    enabled,
                    cx,
                    |this, _, cx| this.apply_clicked(cx),
                )),
        };
        let modal = dialog::modal("schema-review-dialog", "Review structure changes", 680.)
            .child(dialog::header(title, Some(env_chip(policy.environment))))
            .child(body)
            .child(footer);
        Some(
            dialog::backdrop("schema-review-backdrop")
                .child(ui::appear("schema-review-appear", modal))
                .into_any_element(),
        )
    }

    fn options(
        &self,
        state: &ReviewState,
        phase: ReviewPhase,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let toggles = plan_toggles(state.preview.engine, &state.preview.changes);
        if toggles.is_empty() {
            return None;
        }
        let enabled = phase == ReviewPhase::Ready;
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(ui::section_label("Options"));
        for (index, choice) in toggles.into_iter().enumerate() {
            let on = toggle_on(&state.preview.options, choice);
            list = list.child(
                div()
                    .id(("schema-review-option", index))
                    .role(Role::CheckBox)
                    .aria_label(choice.label())
                    .aria_toggled(on.into())
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(22.))
                    .px(px(4.))
                    .rounded(px(4.))
                    .text_color(if enabled {
                        style::text()
                    } else {
                        style::faint()
                    })
                    .when(enabled, |row| {
                        row.cursor_pointer()
                            .hover(|s| s.bg(style::hover()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_option(choice, window, cx)
                            }))
                    })
                    .child(ui::check_box(on))
                    .child(choice.label()),
            );
        }
        Some(list.into_any_element())
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

fn outcome_banner(outcome: OutcomeText) -> AnyElement {
    if outcome.error {
        return ui::shake(
            ("schema-review-outcome", outcome.text.len()),
            ui::error_banner("schema-review-outcome", outcome.text),
        )
        .into_any_element();
    }
    div()
        .id("schema-review-outcome")
        .role(Role::Status)
        .aria_label(SharedString::from(outcome.text.clone()))
        .flex()
        .items_start()
        .gap(px(6.))
        .px(px(8.))
        .py(px(5.))
        .rounded(px(5.))
        .border_1()
        .border_color(style::line())
        .bg(style::ok_fill())
        .text_sm()
        .text_color(style::text())
        .child(
            gpui::svg()
                .path("icons/check.svg")
                .mt(px(1.))
                .size(px(style::ICON))
                .flex_none()
                .text_color(style::ok()),
        )
        .child(div().flex_1().min_w_0().child(outcome.text))
        .into_any_element()
}

fn change_line(index: usize, change: &SchemaChange) -> AnyElement {
    let object = change.object.label();
    let (verb, color) = match &change.kind {
        ChangeKind::Added => ("Add", style::ok()),
        ChangeKind::Removed => ("Drop", style::bad()),
        ChangeKind::Renamed { .. } => ("Rename", style::warn()),
        ChangeKind::Changed { .. } => ("Change", style::warn()),
    };
    let detail = match &change.kind {
        ChangeKind::Renamed { from, to } => Some(("name", from.clone(), to.clone())),
        ChangeKind::Changed {
            field,
            before,
            after,
        } => Some((field.label(), before.clone(), after.clone())),
        _ => None,
    };
    let label = match &detail {
        Some((field, before, after)) => {
            format!(
                "{verb} {object} {}: {field} {before} to {after}",
                change.name
            )
        }
        None => format!("{verb} {object} {}", change.name),
    };
    div()
        .id(("schema-review-change", index))
        .role(Role::ListItem)
        .aria_label(SharedString::from(label))
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(8.))
        .py(px(4.))
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
                        .text_color(color)
                        .child(verb),
                )
                .child(div().flex_none().text_color(style::dim()).child(object))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(style::MONO)
                        .text_color(style::text())
                        .child(crate::structure_table::clip(&change.name)),
                ),
        )
        .when_some(detail, |item, (field, before, after)| {
            item.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .font_family(style::MONO)
                    .child(div().flex_none().text_color(style::dim()).child(field))
                    .child(
                        div()
                            .flex_shrink_1()
                            .min_w_0()
                            .truncate()
                            .text_color(style::faint())
                            .line_through()
                            .child(crate::structure_table::clip(&before)),
                    )
                    .child(div().flex_none().text_color(style::faint()).child("→"))
                    .child(
                        div()
                            .flex_shrink_1()
                            .min_w_0()
                            .truncate()
                            .text_color(style::warn())
                            .child(crate::structure_table::clip(&after)),
                    ),
            )
        })
        .when_some(change.destructive, |item, reason| {
            item.child(
                div()
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::bad_text())
                    .child(format!("Destructive: {}", reason.label())),
            )
        })
        .into_any_element()
}

fn changes_list(preview: &SchemaEditPreview) -> AnyElement {
    let mut list = div()
        .id("schema-review-changes")
        .role(Role::List)
        .aria_label("Changes")
        .flex()
        .flex_col()
        .gap(px(4.));
    let mut start = 0;
    for group in ChangeGroup::ALL {
        let changes: Vec<&SchemaChange> = preview.changes.in_group(group).collect();
        let count = changes.len();
        if count == 0 {
            continue;
        }
        list = list.child(ui::section_label(format!("{} · {count}", group.label())));
        for (offset, change) in changes.into_iter().enumerate() {
            list = list.child(change_line(start + offset, change));
        }
        start += count;
    }
    list.into_any_element()
}

fn impact_line(index: usize, tag: &'static str, warn: bool, text: String) -> AnyElement {
    div()
        .id(("schema-review-impact", index))
        .role(Role::ListItem)
        .aria_label(SharedString::from(format!("{tag}: {text}")))
        .flex()
        .items_start()
        .gap(px(6.))
        .child(
            div()
                .flex_none()
                .px(px(5.))
                .rounded(px(3.))
                .bg(if warn {
                    style::warn_fill()
                } else {
                    style::raised()
                })
                .text_size(px(style::FONT_SMALL))
                .text_color(if warn { style::warn() } else { style::dim() })
                .child(tag),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(style::text())
                .child(text),
        )
        .into_any_element()
}

fn impact_list(state: &ReviewState) -> AnyElement {
    let preview = &state.preview;
    let mut lines = Vec::new();
    for change in preview.changes.destructive() {
        if let Some(reason) = change.destructive {
            lines.push((
                "Destructive",
                true,
                format!(
                    "{} {}: {}",
                    change.object.label(),
                    change.name,
                    reason.label()
                ),
            ));
        }
    }
    for note in &preview.notes {
        lines.push((
            note_label(note.kind),
            note_warns(note.kind),
            note.text.clone(),
        ));
    }
    for reference in &state.inbound {
        lines.push((
            "Referenced by",
            false,
            format!(
                "{}.{} ({}) via {}",
                reference.schema,
                reference.table,
                reference.columns.join(", "),
                reference.name
            ),
        ));
    }
    for dependent in &state.dependents {
        lines.push((
            "Dependent",
            false,
            format!("{} {}.{}", dependent.kind, dependent.schema, dependent.name),
        ));
    }
    if let Some(rows) = state.estimated_rows.filter(|rows| *rows > 0) {
        lines.push(("Rows", false, format!("About {} rows", ui::grouped(rows))));
    }
    let mut list = div()
        .id("schema-review-impact")
        .role(Role::List)
        .aria_label("Impact")
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(ui::section_label("Impact"));
    if !lines.iter().any(|(tag, _, _)| *tag == "Destructive") {
        list = list.child(
            div()
                .text_color(style::dim())
                .child("No destructive changes"),
        );
    }
    for (index, (tag, warn, text)) in lines.into_iter().enumerate() {
        list = list.child(impact_line(index, tag, warn, text));
    }
    list.into_any_element()
}
