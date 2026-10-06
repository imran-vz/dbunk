use super::*;
use gpui::{KeyDownEvent, Role, div, px};
impl SchemaView {
    fn label(&self, action: Action, default: &'static str) -> &'static str {
        if matches!(action, Action::Discard) && self.recovery.unknown() {
            if self.reconcile_armed {
                "Discard reconciled recovery"
            } else {
                "Reconcile unknown outcome"
            }
        } else {
            default
        }
    }
    fn button(
        &self,
        index: usize,
        action: Action,
        label: &'static str,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let label = self.label(action, label);
        let enabled = self.enabled(action);
        let checked = self.include_comment;
        let weak = cx.entity().downgrade();
        let primary = enabled && matches!(action, Action::Review | Action::Apply | Action::Confirm);
        let toggle = matches!(action, Action::Comment).then_some(checked);
        crate::ui::pressed(
            crate::ui::tool_button(
                ("schema-action", index),
                label,
                (toggle == Some(true)).then_some("icons/check.svg"),
                enabled,
                primary,
            ),
            toggle == Some(true),
        )
        .role(if matches!(action, Action::Comment) {
            Role::CheckBox
        } else {
            Role::Button
        })
        .aria_label(label)
        .a11y_synthetic_children(move |builder| {
            if !enabled {
                builder.parent_node().set_disabled();
            }
            if matches!(action, Action::Comment) {
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
}
impl SchemaView {
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        if event.keystroke.key == "escape" {
            self.activate(Action::Back, window, cx);
            cx.stop_propagation();
            return;
        }
        let modifiers = &event.keystroke.modifiers;
        if event.keystroke.key != "tab" || modifiers.control || modifiers.alt || modifiers.platform
        {
            return;
        }
        let mut handles = vec![self.name.focus_handle(cx)];
        if self.include_comment {
            handles.push(self.comment.focus_handle(cx));
        }
        handles.extend(
            self.buttons
                .iter()
                .zip(ACTIONS)
                .filter(|(_, (action, _))| self.enabled(*action))
                .map(|(handle, _)| handle.clone()),
        );
        handles.push(self.details.clone());
        let current = handles.iter().position(|handle| handle.is_focused(window));
        let next = if modifiers.shift {
            current.map_or(handles.len() - 1, |i| {
                (i + handles.len() - 1) % handles.len()
            })
        } else {
            current.map_or(0, |i| (i + 1) % handles.len())
        };
        window.focus(&handles[next], cx);
        cx.stop_propagation();
    }
}
impl Render for SchemaView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let details = if self.preview.is_empty() {
            self.recovery_text()
        } else {
            self.preview.clone()
        };
        let failure = self.failure.shown(&self.message);
        let field_row = |label: &'static str, field: gpui::AnyView| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .child(
                    div()
                        .flex_none()
                        .w(px(64.))
                        .text_size(px(crate::style::FONT_SMALL))
                        .text_color(crate::style::dim())
                        .child(label),
                )
                .child(div().flex_1().child(field))
        };
        div()
            .id("schema-review")
            .role(Role::Group)
            .aria_label("Create schema and SQL review")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .h_full()
            .min_h_0()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_size(px(crate::style::FONT))
            .border_t_1()
            .border_color(crate::style::line())
            .capture_key_down(cx.listener(Self::key))
            .child(
                crate::ui::toolbar().children(
                    ACTIONS
                        .into_iter()
                        .enumerate()
                        .map(|(index, (action, label))| self.button(index, action, label, cx)),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .py(px(8.))
                    .child(
                        div()
                            .px(px(8.))
                            .text_color(crate::style::dim())
                            .child(format!("Connection: {}", self.recovery.connection())),
                    )
                    .child(field_row("Schema", self.name.clone().into()))
                    .when(self.include_comment, |panel| {
                        panel.child(field_row("Comment", self.comment.clone().into()))
                    }),
            )
            .child(
                div()
                    .px(px(8.))
                    .pb(px(6.))
                    .text_size(px(crate::style::FONT_SMALL))
                    .text_color(crate::style::faint())
                    .child("One transaction. 30 s async operation limit; configured statement timeout; 10 s lock timeout."),
            )
            .child(
                div()
                    .id("schema-sql")
                    .role(Role::Label)
                    .aria_label("Exact SQL or saved recovery intent")
                    .aria_value(details.clone())
                    .track_focus(&self.details)
                    .tab_stop(true)
                    .tab_index(0)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(8.))
                    .border_t_1()
                    .border_color(crate::style::line_soft())
                    .font_family(crate::style::MONO)
                    .focus(|s| s.bg(crate::style::row_hover()))
                    .child(details),
            )
            .when_some(failure, |panel, seq| {
                panel.child(crate::ui::error_strip(
                    "schema-error",
                    seq,
                    self.message.clone(),
                ))
            })
            .child(
                crate::ui::status_line()
                    .id("schema-status")
                    .role(Role::Status)
                    .aria_label(self.message.clone())
                    .max_h(px(64.))
                    .overflow_y_scroll()
                    .when(failure.is_none(), |status| status.child(self.message.clone())),
            )
    }
}
