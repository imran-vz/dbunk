use super::*;
use std::fmt::Write;
/// Bounded text, including exact SQL. Editors separately own their history.
struct Text(String);
impl std::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if self
            .0
            .len()
            .checked_add(s.len())
            .is_none_or(|n| n > 512 * 1024)
        {
            return Err(std::fmt::Error);
        }
        self.0.push_str(s);
        Ok(())
    }
}
fn reference_text(reference: &PgObjectRef) -> String {
    let schema = reference
        .schema
        .as_deref()
        .map(|s| format!("{s:?}."))
        .unwrap_or_default();
    let arguments = reference
        .identity_args
        .as_deref()
        .map(|a| format!("({a})"))
        .unwrap_or_default();
    format!(
        "{:?} {schema}{:?}{arguments}",
        reference.kind, reference.name
    )
}
fn operation_text(operation: &ObjectDdlOperation) -> String {
    match operation {
        ObjectDdlOperation::DropObject { reference, cascade } => format!(
            "Drop {} {}",
            reference_text(reference),
            if *cascade { "CASCADE" } else { "RESTRICT" }
        ),
        ObjectDdlOperation::CreateView {
            schema,
            name,
            or_replace,
            ..
        } => format!(
            "Create{} view {schema:?}.{name:?}",
            if *or_replace { " or replace" } else { "" }
        ),
        ObjectDdlOperation::CreateMaterializedView {
            schema,
            name,
            with_data,
            ..
        } => format!(
            "Create materialized view {schema:?}.{name:?} {}",
            if *with_data {
                "WITH DATA"
            } else {
                "WITH NO DATA"
            }
        ),
        ObjectDdlOperation::CreateIndex {
            schema,
            table,
            name,
            concurrently,
            ..
        } => format!(
            "Create index {schema:?}.{name:?} on {table:?}{}",
            if *concurrently { " concurrently" } else { "" }
        ),
        ObjectDdlOperation::AddEnumValue {
            schema,
            name,
            value,
            ..
        } => format!("Add label {value:?} to enum {schema:?}.{name:?}"),
    }
}
fn claim_text(claim: &ObjectDdlClaim) -> String {
    match claim {
        ObjectDdlClaim::Existing { reference, address } => format!(
            "Existing {}: class {} OID {} row version {}",
            reference_text(reference),
            address.class_oid,
            address.object_oid,
            address.row_version
        ),
        ObjectDdlClaim::Schema { name, address } => format!(
            "Schema {name:?}: OID {} row version {}",
            address.object_oid, address.row_version
        ),
        ObjectDdlClaim::Absent { schema, name } => {
            format!("Absent: no relation or type named {schema:?}.{name:?}")
        }
    }
}
pub(super) fn receipt_text(receipt: &ObjectDdlReceipt) -> String {
    let total = receipt.operations.len();
    let detail = match &receipt.outcome {
        ObjectDdlOutcome::Applied { runtime_ms } => {
            format!("Applied: all {total} statement(s) committed in {runtime_ms} ms.")
        }
        ObjectDdlOutcome::NotDispatched { reason } => {
            format!("Not dispatched ({reason:?}): no statement reached the server.")
        }
        ObjectDdlOutcome::Stopped {
            committed,
            stopped_at,
            stop,
            reason,
            residue,
        } => format!(
            "Stopped ({reason:?}). Committed: {committed} of {total} leading statement(s). The group starting at statement {} ended {stop:?}; later statements were not sent.{}",
            stopped_at.saturating_add(1),
            match residue {
                Some(ObjectDdlResidue::InvalidIndex { schema, name }) => format!(
                    " Residue: INVALID index {schema:?}.{name:?} remains; drop or rebuild it explicitly."
                ),
                Some(ObjectDdlResidue::Unverified) => {
                    " Residue could not be verified; inspect the target.".into()
                }
                None => String::new(),
            }
        ),
        ObjectDdlOutcome::OutcomeUnknown {
            committed,
            uncertain_end,
            reason,
        } => format!(
            "Outcome unknown ({reason:?}). Committed: {committed} leading statement(s); statements {} to {uncertain_end} may or may not have committed; later statements were not sent. Never retried.",
            committed.saturating_add(1)
        ),
    };
    format!("Attempt {}: {detail}", receipt.attempt_id.as_str())
}
impl ObjectDdlView {
    pub(super) fn review_text(&self) -> String {
        let mut out = Text(String::new());
        let result: std::fmt::Result = (|| {
            writeln!(out, "Connection: {:?}", self.recovery.connection())?;
            if let Some(purpose) = &self.purpose {
                writeln!(out, "Purpose: {}", purpose.label())?;
            }
            if let Some(j) = self.recovery.journal() {
                writeln!(
                    out,
                    "Attempt: {}\nRecovery: {:?}\nDatabase OID: {}\nOperation digest: {}",
                    j.attempt_id.as_str(),
                    j.apply_state,
                    j.target.database_oid,
                    j.preview.operation_digest
                )?;
                for (index, (operation, claims)) in
                    j.operations.iter().zip(&j.target.claims).enumerate()
                {
                    writeln!(
                        out,
                        "\nOperation {}: {}",
                        index + 1,
                        operation_text(operation)
                    )?;
                    for claim in claims {
                        writeln!(out, "  Observed: {}", claim_text(claim))?;
                    }
                }
                if let Some(review) = &self.review {
                    for (operation, impact) in review.operations().iter().zip(review.impacts()) {
                        let Some(impact) = impact else { continue };
                        writeln!(
                            out,
                            "\nDrop impact for {} ({} dependent(s){}), captured with the observed identity:",
                            operation_text(operation),
                            impact.dependents.len(),
                            if impact.truncated {
                                "; walk truncated, more may exist"
                            } else {
                                ""
                            }
                        )?;
                        for dependent in &impact.dependents {
                            writeln!(
                                out,
                                "  depth {}: {} {}",
                                dependent.depth, dependent.object_type, dependent.identity
                            )?;
                        }
                        if impact.dependents.is_empty() && !impact.truncated {
                            writeln!(out, "  No reported dependents.")?;
                        }
                    }
                } else if j
                    .operations
                    .iter()
                    .any(|o| matches!(o, ObjectDdlOperation::DropObject { .. }))
                {
                    writeln!(
                        out,
                        "\nDrop impact is review evidence only; observe again to load it."
                    )?;
                }
                if j.operations
                    .iter()
                    .any(|o| matches!(o, ObjectDdlOperation::DropObject { cascade: false, .. }))
                {
                    writeln!(
                        out,
                        "RESTRICT: PostgreSQL refuses the drop, with no effect, if any dependent exists."
                    )?;
                }
                if j.operations
                    .iter()
                    .any(|o| matches!(o, ObjectDdlOperation::DropObject { cascade: true, .. }))
                {
                    writeln!(out, "{OBJECT_DDL_CASCADE_DISCLOSURE}")?;
                }
                writeln!(
                    out,
                    "\nStored policy at review: {}\nOperation deadline: {} ms\nStatement timeout: {}",
                    if j.preview.confirmation_required {
                        "confirmation required"
                    } else {
                        "no confirmation required"
                    },
                    j.preview.operation_timeout_ms,
                    j.preview.statement_timeout_ms.map_or_else(
                        || "inherited/server default".into(),
                        |ms| format!("{ms} ms")
                    ),
                )?;
                for group in &j.preview.groups {
                    let (label, statements) = match group {
                        ObjectDdlGroup::Atomic { .. } => ("Atomic transaction", group.statements()),
                        ObjectDdlGroup::Standalone { .. } => {
                            ("Standalone (outside a transaction)", group.statements())
                        }
                    };
                    writeln!(out, "\n{label}:")?;
                    for index in statements {
                        let Some(statement) = j.preview.statements.get(index) else {
                            continue;
                        };
                        writeln!(
                            out,
                            "  Statement {}{}: {}\n{}",
                            index + 1,
                            if statement.destructive {
                                " (destructive)"
                            } else {
                                ""
                            },
                            statement.summary,
                            statement.sql
                        )?;
                    }
                }
                writeln!(out, "\n{}", j.preview.effect_scope())?;
            } else {
                writeln!(
                    out,
                    "Draft: {}{}{}",
                    match (&self.purpose, self.draft.materialized) {
                        (Some(Purpose::Drop(_)), _) => "drop",
                        (_, true) => "materialized view",
                        _ => "view",
                    },
                    if matches!(self.purpose, Some(Purpose::Drop(_))) && self.draft.cascade {
                        ", CASCADE"
                    } else if matches!(self.purpose, Some(Purpose::Drop(_))) {
                        ", RESTRICT"
                    } else {
                        ""
                    },
                    if self.creating() && !self.draft.materialized && self.draft.or_replace {
                        ", OR REPLACE"
                    } else if self.creating() && self.draft.materialized && self.draft.with_data {
                        ", WITH DATA"
                    } else if self.creating() && self.draft.materialized {
                        ", WITH NO DATA"
                    } else {
                        ""
                    }
                )?;
                writeln!(
                    out,
                    "Observe and review loads exact identities{} before any SQL is shown.",
                    if self.creating() {
                        " and confirms the name is free"
                    } else {
                        " and the drop impact"
                    }
                )?;
            }
            Ok(())
        })();
        if result.is_err() {
            "Review display exceeds its bound; nothing has been truncated or dispatched".into()
        } else {
            out.0
        }
    }
    fn toggled(&self, action: Action) -> Option<bool> {
        match action {
            Action::Mode if self.creating() => Some(self.draft.materialized),
            Action::Mode => Some(self.draft.cascade),
            Action::Option if self.creating() && self.draft.materialized => {
                Some(self.draft.with_data)
            }
            Action::Option if self.creating() => Some(self.draft.or_replace),
            _ => None,
        }
    }
    fn label(&self, action: Action, fallback: &'static str) -> &'static str {
        match action {
            Action::Mode if self.creating() => "Materialized view",
            Action::Mode => "CASCADE: also drop dependents",
            Action::Option if self.creating() && self.draft.materialized => {
                "WITH DATA: populate now"
            }
            Action::Option if self.creating() => "OR REPLACE: replace an existing view",
            Action::Option => "No options for drop",
            Action::Discard if self.recovery.unknown() && self.armed => {
                "Discard reconciled recovery"
            }
            Action::Discard if self.recovery.unknown() => "Reconcile unknown outcome",
            _ => fallback,
        }
    }
    fn button(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let (action, fallback) = ACTIONS[index];
        let label = self.label(action, fallback);
        let enabled = self.enabled(action);
        let toggled = self.toggled(action);
        let weak = cx.weak_entity();
        // A toggle shows its state as a pressed face plus a check mark.
        let icon = (toggled == Some(true)).then_some("icons/check.svg");
        crate::ui::pressed(
            crate::ui::tool_button(
                ("object-ddl-action", index),
                label.to_owned(),
                icon,
                enabled,
                false,
            ),
            toggled == Some(true),
        )
        .role(if toggled.is_some() {
            Role::CheckBox
        } else {
            Role::Button
        })
        .aria_label(label)
        .track_focus(&self.buttons[index])
        .tab_stop(enabled)
        .tab_index(0)
        .a11y_synthetic_children(move |b| {
            if !enabled {
                b.parent_node().set_disabled();
            }
        })
        .when_some(toggled, |b, on| b.aria_toggled(on.into()))
        // GPUI activates a focused clickable on Enter/Space key-up through
        // on_click; no key-down handler, so activation happens once.
        .on_click(cx.listener(move |this, _, window, cx| this.click(index, window, cx)))
        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
            weak.update(cx, |this, cx| this.click(index, window, cx))
                .ok();
        })
        .into_any_element()
    }
    fn click(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(ACTIONS[index].0) {
            return;
        }
        if self.composing(window, cx) {
            self.message = "Finish composition before changing this review".into();
            cx.notify();
            return;
        }
        window.focus(&self.buttons[index], cx);
        self.activate(ACTIONS[index].0, window, cx);
    }
    fn focus_control(&self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        let mut handles = ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, a)| self.enabled(a.0))
            .map(|(i, _)| self.buttons[i].clone())
            .collect::<Vec<_>>();
        if self.editable_recipe() {
            handles.extend(self.fields().map(|field| field.focus_handle(cx)));
        }
        handles.push(self.details.clone());
        let at = handles.iter().position(|h| h.contains_focused(window, cx));
        let next = if reverse {
            at.map_or(handles.len() - 1, |i| {
                (i + handles.len() - 1) % handles.len()
            })
        } else {
            at.map_or(0, |i| (i + 1) % handles.len())
        };
        window.focus(&handles[next], cx);
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        let m = event.keystroke.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => self.focus_control(m.shift, window, cx),
            "escape" => self.activate(Action::Back, window, cx),
            key if self.details.is_focused(window) => {
                let mut offset = self.scroll.offset();
                offset.y = match key {
                    "up" => offset.y + px(24.),
                    "down" => offset.y - px(24.),
                    "pageup" => offset.y + px(180.),
                    "pagedown" => offset.y - px(180.),
                    "home" => px(0.),
                    "end" => -self.scroll.max_offset().y,
                    _ => return,
                }
                .max(-self.scroll.max_offset().y)
                .min(px(0.));
                self.scroll.set_offset(offset);
                cx.notify();
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }
}
impl Focusable for ObjectDdlView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.details.clone()
    }
}
impl Render for ObjectDdlView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_fields(cx);
        let text = self.review_text();
        div()
            .id("object-ddl-review")
            .key_context("ObjectDdl")
            .role(Role::Group)
            .aria_label("Object change review")
            .track_focus(&self.root)
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_xs()
            .capture_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|this, _: &NextControl, window, cx| {
                if !this.composing(window, cx) {
                    this.focus_control(false, window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| {
                if !this.composing(window, cx) {
                    this.focus_control(true, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children((0..ACTIONS.len()).map(|i| self.button(i, cx))),
            )
            .when_some(self.name.as_ref(), |v, field| {
                v.child(div().px_2().pt_1().child(field.clone()))
            })
            .when_some(self.body.as_ref(), |v, field| {
                v.child(div().px_2().pt_1().child(field.clone()))
            })
            .child(
                div()
                    .id("object-ddl-status")
                    .role(Role::Status)
                    .aria_label(self.message.clone())
                    .p_2()
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .id("object-ddl-details")
                    .role(Role::Group)
                    .aria_label("Exact observed identities, impact, SQL and deadlines")
                    .aria_value(text.clone())
                    .track_focus(&self.details)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_2()
                    .child(text)
                    .when(!self.receipt.is_empty(), |v| {
                        v.child(
                            div()
                                .id("object-ddl-receipt")
                                .role(Role::Label)
                                .aria_label(self.receipt.clone())
                                .child(self.receipt.clone()),
                        )
                    }),
            )
    }
}
