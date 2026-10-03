//! Keyboard, AX and rendering for the sequence tool. Buttons activate through
//! GPUI's focused-click path only (no duplicate Enter/Space key handler), and
//! element/focus keys are stable indexes independent of labels or state.
use super::*;
use gpui::{KeyDownEvent, Role, div, px};

impl SequenceView {
    fn button(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let (action, label) = ACTIONS[index];
        let enabled = self.enabled(action);
        let toggle = match action {
            Action::SetCalled => Some(self.set_called),
            Action::RestartWith => Some(self.restart_with),
            _ => None,
        };
        let weak = cx.entity().downgrade();
        let text = match toggle {
            Some(true) => format!("[x] {label}"),
            Some(false) => format!("[ ] {label}"),
            None => label.to_owned(),
        };
        div()
            .id(("sequence-action", index))
            .role(if toggle.is_some() {
                Role::CheckBox
            } else {
                Role::Button
            })
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if let Some(checked) = toggle {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(checked));
                }
            })
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .px_2()
            .py_1()
            .border_1()
            .border_color(crate::style::line())
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .focus(|style| style.bg(crate::style::hover()))
            .child(text)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.set_value
            .update(cx, |field, cx| field.composing(window, cx))
            || self
                .restart_value
                .update(cx, |field, cx| field.composing(window, cx))
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
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
            let mut order = vec![self.set_value.focus_handle(cx)];
            if self.restart_with {
                order.push(self.restart_value.focus_handle(cx));
            }
            order.extend(
                ACTIONS
                    .iter()
                    .enumerate()
                    .filter(|(_, (action, _))| self.enabled(*action))
                    .map(|(i, _)| self.buttons[i].clone()),
            );
            order.push(self.details.clone());
            let current = order.iter().position(|f| f.is_focused(window));
            let next = if key.modifiers.shift {
                current.map_or(order.len() - 1, |i| (i + order.len() - 1) % order.len())
            } else {
                current.map_or(0, |i| (i + 1) % order.len())
            };
            window.focus(&order[next], cx);
            cx.stop_propagation();
            return;
        }
        if !self.details.is_focused(window) {
            return;
        }
        let scroll = &self.scroll;
        let mut offset = scroll.offset();
        offset.y = match key.key.as_str() {
            "up" => offset.y + px(24.),
            "down" => offset.y - px(24.),
            "pageup" => offset.y + px(180.),
            "pagedown" => offset.y - px(180.),
            "home" => px(0.),
            "end" => -scroll.max_offset().y,
            _ => return,
        };
        offset.y = offset.y.max(-scroll.max_offset().y).min(px(0.));
        scroll.set_offset(offset);
        cx.stop_propagation();
        cx.notify();
    }
}
impl Render for SequenceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.text();
        div()
            .id("sequence-tool")
            .role(Role::Group)
            .aria_label("PostgreSQL sequence inspect and change review")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .border_t_1()
            .border_color(crate::style::line())
            .capture_key_down(cx.listener(Self::key))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .px_2()
                    .child("Set value")
                    .child(div().w(px(220.)).child(self.set_value.clone()))
                    .when(self.restart_with, |row| {
                        row.child("Restart value")
                            .child(div().w(px(220.)).child(self.restart_value.clone()))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children((0..ACTIONS.len()).map(|i| self.button(i, cx))),
            )
            .child(
                div()
                    .id("sequence-status")
                    .role(Role::Status)
                    .aria_label(self.message.clone())
                    .px_2()
                    .py_1()
                    .max_h(px(64.))
                    .overflow_y_scroll()
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .id("sequence-details")
                    .role(Role::Group)
                    .aria_label("Observed sequence, reviewed SQL and outcome")
                    .aria_value(text.clone())
                    .track_focus(&self.details)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_2()
                    .child(text),
            )
    }
}
