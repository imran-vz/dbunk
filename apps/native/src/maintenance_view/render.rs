//! Keyboard, AX and rendering for the maintenance review.
use super::*;

impl MaintenanceView {
    fn label(&self, action: Action, fallback: &'static str) -> &'static str {
        if matches!(action, Action::Clear) && self.uncertain() {
            if self.armed {
                "Discard reconciled recovery"
            } else {
                "Reconcile maintenance outcome"
            }
        } else {
            fallback
        }
    }
    fn button(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let (action, fallback) = ACTIONS[index];
        let label = self.label(action, fallback);
        let enabled = self.enabled(action);
        let weak = cx.entity().downgrade();
        let primary = enabled && matches!(action, Action::Apply | Action::Confirm);
        let button = if primary {
            crate::ui::tool_button_accent(("maintenance-action", index), label, None, enabled)
        } else {
            crate::ui::tool_button(("maintenance-action", index), label, None, enabled, false)
        };
        button
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    /// Review actions show while they apply to the reference; actions of a
    /// later step (apply, confirm, cancel, recovery, copy) appear only at
    /// that step, so the bar never leads with disabled controls.
    fn shown(&self, action: Action) -> bool {
        match action {
            Action::Review(intent) => self.reference.as_ref().is_some_and(|reference| {
                !matches!(intent, MaintenanceIntent::RefreshMaterializedView { .. })
                    || reference.kind == PgObjectKind::MaterializedView
            }),
            Action::Back => true,
            _ => self.enabled(action),
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
            return;
        }
        if key.key == "escape" {
            self.activate(Action::Back, window, cx);
            cx.stop_propagation();
            return;
        }
        if key.key == "tab" {
            let mut order = ACTIONS
                .iter()
                .enumerate()
                .filter(|(_, (action, _))| self.enabled(*action))
                .map(|(i, _)| self.buttons[i].clone())
                .collect::<Vec<_>>();
            order.push(self.details.clone());
            let current = order
                .iter()
                .position(|f| f.is_focused(window))
                .unwrap_or(order.len() - 1);
            let next = (current
                + if key.modifiers.shift {
                    order.len() - 1
                } else {
                    1
                })
                % order.len();
            window.focus(&order[next], cx);
            cx.stop_propagation();
            return;
        }
        if !self.details.is_focused(window) {
            return;
        }
        let mut offset = self.scroll.offset();
        offset.y = match key.key.as_str() {
            "up" => (offset.y + px(24.)).min(px(0.)),
            "down" => offset.y - px(24.),
            "pageup" => (offset.y + px(180.)).min(px(0.)),
            "pagedown" => offset.y - px(180.),
            "home" => px(0.),
            "end" => -self.scroll.max_offset().y,
            _ => return,
        };
        offset.y = offset.y.max(-self.scroll.max_offset().y).min(px(0.));
        self.scroll.set_offset(offset);
        cx.stop_propagation();
        cx.notify();
    }
}
impl Focusable for MaintenanceView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.details.clone()
    }
}
impl Render for MaintenanceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.text();
        let failure = self.failure.shown(&self.message);
        div()
            .id("maintenance-review")
            .role(Role::Group)
            .aria_label("PostgreSQL maintenance review")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_size(px(crate::style::FONT))
            .capture_key_down(cx.listener(Self::key))
            .child(
                crate::ui::toolbar().children(
                    (0..ACTIONS.len())
                        .filter(|&i| self.shown(ACTIONS[i].0))
                        .map(|i| self.button(i, cx)),
                ),
            )
            .child(
                div()
                    .id("maintenance-details")
                    .role(Role::Group)
                    .aria_label("Observed target, SQL, deadlines and recovery")
                    .aria_value(text.clone())
                    .track_focus(&self.details)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(crate::style::INSET_CONTENT))
                    .font_family(crate::style::MONO)
                    .text_size(px(crate::style::FONT_DATA))
                    .focus(|s| s.bg(crate::style::row_hover()))
                    .child(text),
            )
            .when_some(failure, |v, seq| {
                v.child(crate::ui::error_strip(
                    "maintenance-error",
                    seq,
                    self.message.clone(),
                ))
            })
            .child(
                crate::ui::status_line()
                    .id("maintenance-status")
                    .role(Role::Status)
                    .aria_label(self.message.clone())
                    .when(failure.is_none(), |v| v.child(self.message.clone())),
            )
    }
}
