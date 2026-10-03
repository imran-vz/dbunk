use super::*;
use std::fmt::Write;
/// Bounded text, including exact SQL. The value editor separately owns history.
struct Text(String);
impl std::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if self
            .0
            .len()
            .checked_add(s.len())
            .is_none_or(|n| n > 64 * 1024)
        {
            return Err(std::fmt::Error);
        }
        self.0.push_str(s);
        Ok(())
    }
}
impl SchemaAlterView {
    pub(super) fn review_text(&self) -> String {
        let mut out = Text(String::new());
        let result: std::fmt::Result = (|| {
            writeln!(out, "Connection: {:?}", self.recovery.connection())?;
            if let Some(j) = self.recovery.journal() {
                writeln!(
                    out,
                    "Attempt: {}\nRecovery: {:?}\nDatabase OID: {}\nSchema: {:?} (OID {})\nNamespace row version: xmin {} ctid {}",
                    j.attempt_id.as_str(),
                    j.apply_state,
                    j.target.identity.database_oid,
                    j.target.schema,
                    j.target.identity.schema_oid,
                    j.target.namespace_xmin,
                    j.target.namespace_ctid,
                )?;
                match &j.target.comment {
                    Some(comment) => writeln!(out, "Observed comment: {comment:?}")?,
                    None => writeln!(out, "Observed comment: SQL NULL")?,
                }
                match &j.intent {
                    SchemaAlterIntent::SetComment {
                        comment: Some(comment),
                    } => writeln!(out, "New comment: {comment:?}")?,
                    SchemaAlterIntent::SetComment { comment: None } => {
                        writeln!(out, "Remove comment: SQL NULL")?
                    }
                    SchemaAlterIntent::Rename { new_name } => {
                        writeln!(out, "New name: {new_name:?}")?
                    }
                }
                writeln!(
                    out,
                    "{}\nOperation deadline: {} ms\nStatement timeout: {}\n\nExact SQL:\n{}",
                    j.preview.summary,
                    j.preview.operation_timeout_ms,
                    j.preview.statement_timeout_ms.map_or_else(
                        || "inherited/server default".into(),
                        |ms| format!("{ms} ms")
                    ),
                    j.preview.sql
                )?;
            } else if let Some(selection) = &self.selection {
                let r = selection.request();
                writeln!(
                    out,
                    "Selected schema: {:?}\nIdentity: {}",
                    r.schema,
                    r.expected.map_or_else(
                        || "pinned by the first observation".into(),
                        |i| format!(
                            "database OID {}, schema OID {}",
                            i.database_oid, i.schema_oid
                        )
                    ),
                )?;
            }
            writeln!(out, "\n{SCHEMA_ALTER_EFFECT_SCOPE}")?;
            Ok(())
        })();
        if result.is_err() {
            "Review display exceeds its bound; nothing has been truncated or dispatched".into()
        } else {
            out.0
        }
    }
    fn label(&self, action: Action, fallback: &'static str) -> &'static str {
        match action {
            Action::Operation if self.rename => "Operation: Rename",
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
        let weak = cx.weak_entity();
        div()
            .id(("schema-alter-action", index))
            .role(if matches!(action, Action::Remove) {
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
            .when(matches!(action, Action::Remove), |b| {
                b.aria_toggled(self.remove_comment.into())
            })
            .px_2()
            .py_1()
            .border_1()
            .border_color(crate::style::line())
            .text_color(gpui::rgb(if enabled { 0xffffff } else { 0x888888 }))
            .focus(|s| s.bg(crate::style::line()))
            .on_click(cx.listener(move |this, _, window, cx| this.click(index, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.click(index, window, cx))
                    .ok();
            })
            .child(label)
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
        if self.editable_recipe()
            && (!self.remove_comment || self.rename)
            && let Some(value) = &self.value
        {
            handles.push(value.focus_handle(cx));
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
impl Focusable for SchemaAlterView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.details.clone()
    }
}
impl Render for SchemaAlterView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_field(cx);
        let text = self.review_text();
        div()
            .id("schema-alter-review")
            // Shares the table-change Tab bindings ("TableDdl > Editor") so the
            // value editor cannot swallow Tab; no new global keymap entry.
            .key_context("TableDdl")
            .role(Role::Group)
            .aria_label("Observed existing-schema change")
            .track_focus(&self.root)
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_xs()
            .capture_key_down(cx.listener(Self::key))
            .on_action(
                cx.listener(|this, _: &crate::table_ddl_view::NextControl, window, cx| {
                    if !this.composing(window, cx) {
                        this.focus_control(false, window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_action(cx.listener(
                |this, _: &crate::table_ddl_view::PreviousControl, window, cx| {
                    if !this.composing(window, cx) {
                        this.focus_control(true, window, cx);
                        cx.stop_propagation();
                    }
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children((0..ACTIONS.len()).map(|i| self.button(i, cx))),
            )
            .when_some(self.value.as_ref(), |v, field| {
                v.child(div().p_2().child(field.clone()))
            })
            .child(
                div()
                    .id("schema-alter-status")
                    .role(Role::Status)
                    .aria_label(self.message.clone())
                    .p_2()
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .id("schema-alter-details")
                    .role(Role::Group)
                    .aria_label("Exact observed target, intent, SQL and deadlines")
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
                                .id("schema-alter-receipt")
                                .role(Role::Label)
                                .aria_label(self.receipt.clone())
                                .child(self.receipt.clone()),
                        )
                    }),
            )
    }
}
