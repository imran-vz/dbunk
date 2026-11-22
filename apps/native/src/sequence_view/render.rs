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
        let primary = enabled && matches!(action, Action::Apply | Action::Confirm);
        let element = match toggle {
            Some(on) => {
                crate::object_ddl_view::check_row(("sequence-action", index), label, on, enabled)
            }
            None if primary => {
                crate::ui::tool_button_accent(("sequence-action", index), label, None, enabled)
            }
            None => crate::ui::tool_button(("sequence-action", index), label, None, enabled, false),
        };
        element
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
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    /// Actions of a later step appear at that step, so the bar never leads
    /// with disabled controls; options render as checkbox rows below it.
    fn shown_in_toolbar(&self, action: Action) -> bool {
        match action {
            Action::SetCalled | Action::RestartWith => false,
            Action::Apply | Action::Confirm | Action::Cancel | Action::Discard | Action::Copy => {
                self.enabled(action)
            }
            _ => true,
        }
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
        let failure = self.failure.shown(&self.message);
        let option = |wanted: fn(&Action) -> bool| {
            ACTIONS
                .iter()
                .position(|(action, _)| wanted(action))
                .map(|index| self.button(index, cx))
        };
        let set_called = option(|action| matches!(action, Action::SetCalled));
        let restart_with = option(|action| matches!(action, Action::RestartWith));
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
            .text_size(px(crate::style::FONT))
            .capture_key_down(cx.listener(Self::key))
            .child(
                crate::ui::toolbar().children(
                    (0..ACTIONS.len())
                        .filter(|&i| self.shown_in_toolbar(ACTIONS[i].0))
                        .map(|i| self.button(i, cx)),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .max_w(px(
                        crate::style::TOOL_COLUMN + 2. * crate::style::INSET_CONTENT
                    ))
                    .px(px(crate::style::INSET_CONTENT))
                    .py(px(crate::style::S5))
                    .flex()
                    .flex_col()
                    .gap(px(crate::style::S4))
                    .child(
                        div()
                            .flex()
                            .gap(px(crate::style::S5))
                            .child(div().flex_1().min_w_0().child(crate::ui::labelled(
                                "Set value",
                                crate::object_ddl_view::well(self.set_value.clone()),
                                None,
                            )))
                            .when(self.restart_with, |row| {
                                row.child(div().flex_1().min_w_0().child(crate::ui::labelled(
                                    "Restart value",
                                    crate::object_ddl_view::well(self.restart_value.clone()),
                                    None,
                                )))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(crate::style::S4))
                            .children(set_called)
                            .children(restart_with),
                    ),
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
                    .border_t_1()
                    .border_color(crate::style::line_soft())
                    .px(px(crate::style::INSET_CONTENT))
                    .py(px(crate::style::S4))
                    .font_family(crate::style::MONO)
                    .text_size(px(crate::style::FONT_DATA))
                    .focus(|s| s.bg(crate::style::row_hover()))
                    .child(text),
            )
            .when_some(failure, |v, seq| {
                v.child(crate::ui::error_strip(
                    "sequence-error",
                    seq,
                    self.message.clone(),
                ))
            })
            .child(
                crate::ui::status_line()
                    .id("sequence-status")
                    .role(Role::Status)
                    .aria_label(self.message.clone())
                    .max_h(px(64.))
                    .overflow_y_scroll()
                    .when(failure.is_none(), |v| v.child(self.message.clone())),
            )
    }
}
