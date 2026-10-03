//! Ephemeral, per-document bindings. Text goes through the PostgreSQL parameter
//! planner; neither this view nor the workspace persistence interpolates values.
use crate::accessible_editor::AccessibleEditor;
use dbunk_lib::backend::{Backend, ParameterValue};
use editor::{Editor, EditorEvent};
use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, Role, SharedString, Subscription,
    Window, accesskit::Action, div, prelude::*, px,
};

pub enum ParametersEvent {
    Changed,
    Close,
}

struct Field {
    name: String,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    null: bool,
    null_focus: FocusHandle,
    _subscription: Subscription,
}

pub struct QueryParameters {
    enabled: bool,
    editable: bool,
    fields: Vec<Field>,
    limit: Entity<Editor>,
    accessible_limit: Entity<AccessibleEditor>,
    toggle_focus: FocusHandle,
    close_focus: FocusHandle,
    _limit_subscription: Subscription,
}
impl gpui::EventEmitter<ParametersEvent> for QueryParameters {}

#[derive(Clone, Copy)]
enum ActionKind {
    Enable,
    Null(usize),
    Close,
}

impl QueryParameters {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let limit = cx.new(|cx| Editor::single_line(window, cx));
        let accessible_limit = cx.new(|cx| {
            AccessibleEditor::field(limit.clone(), "Row limit, blank for no limit", false, cx)
        });
        let subscription = cx.subscribe(&limit, |_, _, event, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                cx.emit(ParametersEvent::Changed);
            }
        });
        Self {
            enabled: false,
            editable: true,
            fields: Vec::new(),
            limit,
            accessible_limit,
            toggle_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            _limit_subscription: subscription,
        }
    }

    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        if self.editable == editable {
            return;
        }
        self.editable = editable;
        self.limit
            .update(cx, |editor, _| editor.set_read_only(!editable));
        for field in &self.fields {
            field.editor.update(cx, |editor, _| {
                editor.set_read_only(!editable || field.null)
            });
        }
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.toggle_focus, cx);
    }

    /// A changed name list prepares fields but refuses this click, so a newly
    /// discovered binding can never accidentally execute with an empty value.
    pub fn prepare(
        &mut self,
        sql: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(Option<Vec<ParameterValue>>, Option<i64>), String> {
        let row_limit = parse_limit(&self.limit.read(cx).text(cx))?;
        if !self.enabled {
            return Ok((None, row_limit));
        }
        let names = Backend::describe_query_parameters(sql)
            .map_err(|error| format!("Cannot bind parameters: {error:?}"))?
            .names;
        validate_names(&names)?;
        if !self.fields.iter().map(|field| &field.name).eq(names.iter()) {
            let mut old = std::mem::take(&mut self.fields);
            self.fields = names
                .into_iter()
                .map(|name| {
                    if let Some(index) = old.iter().position(|field| field.name == name) {
                        return old.remove(index);
                    }
                    let editor = cx.new(|cx| Editor::single_line(window, cx));
                    let accessible = cx.new(|cx| {
                        AccessibleEditor::field(
                            editor.clone(),
                            format!("Parameter {name}"),
                            false,
                            cx,
                        )
                    });
                    let subscription = cx.subscribe(&editor, |_, _, event, cx| {
                        if matches!(event, EditorEvent::BufferEdited) {
                            cx.emit(ParametersEvent::Changed);
                        }
                    });
                    Field {
                        name,
                        editor,
                        accessible,
                        null: false,
                        null_focus: cx.focus_handle(),
                        _subscription: subscription,
                    }
                })
                .collect();
            cx.emit(ParametersEvent::Changed);
            cx.notify();
            return Err(
                "Review parameter values, then Run again. Empty text and NULL are distinct.".into(),
            );
        }
        let values = self
            .fields
            .iter()
            .map(|field| ParameterValue {
                name: field.name.clone(),
                value: (!field.null).then(|| field.editor.read(cx).text(cx)),
            })
            .collect::<Vec<_>>();
        validate_values(&values)?;
        Ok((Some(values), row_limit))
    }

    fn activate(&mut self, action: ActionKind, cx: &mut Context<Self>) {
        if matches!(action, ActionKind::Close) {
            cx.emit(ParametersEvent::Close);
            return;
        }
        if !self.editable {
            return;
        }
        match action {
            ActionKind::Enable => self.enabled = !self.enabled,
            ActionKind::Null(index) => {
                let field = &mut self.fields[index];
                field.null = !field.null;
                field
                    .editor
                    .update(cx, |editor, _| editor.set_read_only(field.null));
            }
            ActionKind::Close => unreachable!(),
        }
        cx.emit(ParametersEvent::Changed);
        cx.notify();
    }

    fn button(
        &self,
        label: String,
        action: ActionKind,
        focus: FocusHandle,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let weak = cx.weak_entity();
        let enabled = self.editable || matches!(action, ActionKind::Close);
        let toggle = !matches!(action, ActionKind::Close);
        crate::ui::pressed(
            crate::ui::tool_button(
                SharedString::from(label.clone()),
                label,
                (!toggle).then_some("icons/close.svg"),
                enabled,
                false,
            ),
            toggle && selected,
        )
        .role(if toggle { Role::CheckBox } else { Role::Button })
        .when(toggle, |element| element.aria_toggled(selected.into()))
        .track_focus(&focus)
        .tab_stop(enabled)
        .tab_index(0)
        .a11y_synthetic_children(move |builder| {
            if !enabled {
                builder.parent_node().set_disabled();
            }
        })
        .on_click(cx.listener(move |this, _, _, cx| this.activate(action, cx)))
        .on_a11y_action(Action::Click, move |_, _, cx| {
            let _ = weak.update(cx, |this, cx| this.activate(action, cx));
        })
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            cx.emit(ParametersEvent::Close);
            cx.stop_propagation();
        }
        if event.keystroke.key != "tab" {
            return;
        }
        let mut focus = Vec::new();
        if self.editable {
            focus.extend([self.toggle_focus.clone(), self.limit.focus_handle(cx)]);
            if self.enabled {
                for field in &self.fields {
                    if !field.null {
                        focus.push(field.editor.focus_handle(cx));
                    }
                    focus.push(field.null_focus.clone());
                }
            }
        }
        focus.push(self.close_focus.clone());
        let current = focus
            .iter()
            .position(|handle| handle.is_focused(window))
            .unwrap_or(0);
        let next = if event.keystroke.modifiers.shift {
            (current + focus.len() - 1) % focus.len()
        } else {
            (current + 1) % focus.len()
        };
        window.focus(&focus[next], cx);
        cx.stop_propagation();
    }
}

fn parse_limit(text: &str) -> Result<Option<i64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<i64>()
        .ok()
        .filter(|value| (1..=10_000).contains(value))
        .map(Some)
        .ok_or_else(|| "Row limit must be 1–10000, or blank for no limit".into())
}
fn validate_values(values: &[ParameterValue]) -> Result<(), String> {
    let mut total = 0;
    for value in values
        .iter()
        .filter_map(|parameter| parameter.value.as_ref())
    {
        total += value.len();
        if value.len() > 1024 * 1024 || total > 4 * 1024 * 1024 {
            return Err("Parameter values exceed the 1 MiB per value or 4 MiB total limit".into());
        }
    }
    Ok(())
}

fn validate_names(names: &[String]) -> Result<(), String> {
    if names.len() > 256 || names.iter().any(|name| name.len() > 63) {
        return Err("Use at most 256 named parameters, with names up to 63 bytes".into());
    }
    Ok(())
}

impl Render for QueryParameters {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("query-bindings")
            .role(Role::Group)
            .aria_label("Query parameters and row limit")
            .capture_key_down(cx.listener(Self::key))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .max_h(px(200.))
            .overflow_y_scroll()
            .border_b_1()
            .border_color(crate::style::line_soft())
            .bg(crate::style::panel())
            .px_2()
            .py_1()
            .gap_1()
            .text_sm()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .flex_wrap()
                    .child(self.button(
                        "Named parameters".into(),
                        ActionKind::Enable,
                        self.toggle_focus.clone(),
                        self.enabled,
                        cx,
                    ))
                    .child(crate::ui::separator())
                    .child(div().text_color(crate::style::dim()).child("Row limit"))
                    .child(
                        crate::ui::field()
                            .w(px(120.))
                            .child(self.accessible_limit.clone()),
                    )
                    .child(crate::ui::grow())
                    .child(self.button(
                        "Close bindings".into(),
                        ActionKind::Close,
                        self.close_focus.clone(),
                        false,
                        cx,
                    )),
            )
            .child(div().text_color(crate::style::faint()).child(if self.enabled {
                "Text bindings. Run discovers :names in the selected SQL. Values are not saved."
            } else {
                "Named parameters off. Blank row limit keeps the normal result cap."
            }))
            .when(self.enabled, |element| {
                element.children(self.fields.iter().enumerate().map(|(index, field)| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .min_w(px(80.))
                                .font_family(crate::style::MONO)
                                .text_color(crate::style::dim())
                                .child(format!(":{}", field.name)),
                        )
                        .child(
                            crate::ui::field()
                                .flex_1()
                                .min_w_0()
                                .font_family(crate::style::MONO)
                                .child(field.accessible.clone()),
                        )
                        .child(self.button(
                            format!("NULL :{}", field.name),
                            ActionKind::Null(index),
                            field.null_focus.clone(),
                            field.null,
                            cx,
                        ))
                }))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_reject_invalid_input_without_clamping_or_treating_it_as_unlimited() {
        assert_eq!(parse_limit("  ").unwrap(), None);
        assert_eq!(parse_limit("10000").unwrap(), Some(10000));
        for value in ["0", "-1", "10001", "1.5", "NaN", "9223372036854775808"] {
            assert!(parse_limit(value).is_err());
        }
    }
    #[test]
    fn values_are_bounded_by_utf8_bytes_and_null_is_not_empty_text() {
        let null = ParameterValue {
            name: "a".into(),
            value: None,
        };
        let empty = ParameterValue {
            name: "b".into(),
            value: Some(String::new()),
        };
        assert_ne!(null.value, empty.value);
        assert!(validate_values(&[null, empty]).is_ok());
        let large = ParameterValue {
            name: "a".into(),
            value: Some("é".repeat(524_289)),
        };
        assert!(validate_values(&[large]).is_err());
        let max = ParameterValue {
            name: "a".into(),
            value: Some("a".repeat(1024 * 1024)),
        };
        assert!(validate_values(&vec![max.clone(); 4]).is_ok());
        assert!(validate_values(&vec![max; 5]).is_err());
    }

    #[test]
    fn scanner_output_is_bounded_before_creating_native_editors() {
        assert!(validate_names(&vec!["name".into(); 256]).is_ok());
        assert!(validate_names(&vec!["name".into(); 257]).is_err());
        assert!(validate_names(&["a".repeat(63)]).is_ok());
        assert!(validate_names(&["é".repeat(32)]).is_err());
    }
}
