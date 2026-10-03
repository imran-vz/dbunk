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
        div()
            .id(("schema-action", index))
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
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .px_2()
            .py_1()
            .border_1()
            .border_color(crate::style::line())
            .focus(|style| style.bg(crate::style::hover()))
            .child(label)
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
        div()
            .id("schema-review")
            .role(Role::Group)
            .aria_label("Create schema and SQL review")
            .track_focus(&self.root)
            .flex().flex_col().h_full().min_h_0()
            .bg(crate::style::bg()).text_color(crate::style::text())
            .border_t_1().border_color(crate::style::line())
            .capture_key_down(cx.listener(Self::key))
            .child(div().child(format!("Connection: {}", self.recovery.connection())))
            .child(div().flex().gap_2().px_2().child("Schema")
                .child(div().flex_1().child(self.name.clone())))
            .when(self.include_comment, |panel| panel.child(
                div().flex().gap_2().px_2().child("Comment")
                    .child(div().flex_1().child(self.comment.clone()))))
            .child(div().flex().flex_wrap().gap_1().children(
                ACTIONS.into_iter().enumerate().map(|(index, (action, label))|
                    self.button(index, action, label, cx))))
            .child(div().id("schema-status").role(Role::Status)
                .aria_label(self.message.clone()).max_h(px(64.)).overflow_y_scroll()
                .child(self.message.clone()))
            .child(div().child("One transaction. 30 s async operation limit; configured statement timeout; 10 s lock timeout."))
            .child(div().id("schema-sql").role(Role::Label)
                .aria_label("Exact SQL or saved recovery intent").aria_value(details.clone())
                .track_focus(&self.details).tab_stop(true).tab_index(0)
                .flex_1().min_h_0().overflow_y_scroll().child(details))
    }
}
