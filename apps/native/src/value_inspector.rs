//! Read-only retained-value inspection. Derived views never replace the raw value
//! copied to the clipboard. The owned copy shares the workspace payload budget.
use crate::{cell_value, results::encoded_size};
use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, Window,
    div, prelude::*, px,
};
use std::{cell::Cell, rc::Rc};

const VALUE_BYTES: usize = 1024 * 1024;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Copy)]
enum View {
    Text,
    Json,
    Hex,
}
#[derive(Clone, Copy)]
enum Action {
    View(View),
    Copy,
    Close,
}
pub struct Close;

pub struct Inspection {
    column: String,
    value: Option<String>,
    derived: Option<String>,
    disclosure: String,
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Inspection {
    pub fn new(
        column: String,
        value: &Option<String>,
        partial: bool,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if value
            .as_ref()
            .is_some_and(|value| value.len() > VALUE_BYTES)
            || column.len() > 8192
        {
            return Err("Value inspection exceeds 1 MiB");
        }
        let bytes = encoded_size(&(&column, value, Option::<String>::None));
        if bytes > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err("Workspace memory budget is full; clear results or close another tool");
        }
        budget.set(budget.get() + bytes);
        Ok(Self {
            column,
            value: value.clone(),
            derived: None,
            disclosure: if partial {
                "Retained value only; source is partial".into()
            } else {
                String::new()
            },
            budget,
            bytes,
        })
    }
    fn display(&self) -> &str {
        match (&self.value, &self.derived) {
            (None, _) => "NULL",
            (_, Some(text)) => text,
            (Some(text), _) if text.is_empty() => "(empty string)",
            (Some(text), _) => text,
        }
    }
    fn change_view(&mut self, view: View) -> Result<String, String> {
        let (derived, status) = match (view, self.value.as_deref()) {
            (_, None) => (None, format!("SQL NULL. {}", self.disclosure)),
            (View::Text, _) => (None, self.disclosure.clone()),
            (View::Json, Some(value)) => (
                Some(cell_value::pretty_json(value).map_err(|error| error.to_string())?),
                self.disclosure.clone(),
            ),
            (View::Hex, Some(value)) => {
                let dump = cell_value::hex_dump(value).map_err(|error| error.to_string())?;
                let status = if dump.truncated {
                    format!(
                        "Hex preview: {} of {} UTF-8 bytes; Copy value retains the full retained text. {}",
                        dump.shown_bytes, dump.total_bytes, self.disclosure
                    )
                } else {
                    self.disclosure.clone()
                };
                (Some(dump.text), status)
            }
        };
        let bytes = encoded_size(&(&self.column, &self.value, &derived));
        let remaining = self.budget.get().saturating_sub(self.bytes);
        if bytes > WORKSPACE_BYTES.saturating_sub(remaining) {
            return Err("Workspace memory budget is full; the current view was preserved".into());
        }
        self.budget.set(remaining + bytes);
        self.bytes = bytes;
        self.derived = derived;
        Ok(status)
    }
}
impl Drop for Inspection {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

pub struct ValueInspector {
    data: Inspection,
    focus: FocusHandle,
    buttons: [FocusHandle; 5],
    status: String,
}
impl EventEmitter<Close> for ValueInspector {}
impl ValueInspector {
    pub fn new(data: Inspection, cx: &mut Context<Self>) -> Self {
        let status = if data.value.is_none() {
            format!("SQL NULL. {}", data.disclosure)
        } else {
            data.disclosure.clone()
        };
        Self {
            data,
            focus: cx.focus_handle(),
            buttons: std::array::from_fn(|_| cx.focus_handle()),
            status,
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.buttons[0], cx);
    }
    fn activate(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Close => cx.emit(Close),
            Action::View(view) => {
                self.status = self.data.change_view(view).unwrap_or_else(|error| error)
            }
            Action::Copy => {
                if let Some(value) = &self.data.value {
                    cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
                    self.status = if self.data.disclosure.is_empty() {
                        "Copied exact retained value".into()
                    } else {
                        format!("Copied exact retained value. {}", self.data.disclosure)
                    };
                }
            }
        }
        cx.notify();
    }
    fn button(
        &self,
        index: usize,
        label: &'static str,
        action: Action,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let enabled = !matches!(action, Action::Copy) || self.data.value.is_some();
        let weak = cx.weak_entity();
        div()
            .id(("inspect-action", index))
            .role(Role::Button)
            .aria_label(label)
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .px_2()
            .py_1()
            .focus(|style| style.bg(crate::style::hover()))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    this.activate(action, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                if enabled {
                    weak.update(cx, |this, cx| this.activate(action, cx)).ok();
                }
            })
    }
}
impl Focusable for ValueInspector {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ValueInspector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("value-inspector")
            .role(Role::Group)
            .aria_label(format!("Value inspector: {}", self.data.column))
            .track_focus(&self.focus)
            .key_context("ValueInspector")
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(Close);
                    cx.stop_propagation();
                }
                if event.keystroke.key == "tab" {
                    let handles = this
                        .buttons
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != 3 || this.data.value.is_some())
                        .map(|(_, handle)| handle)
                        .collect::<Vec<_>>();
                    let current = handles.iter().position(|focus| focus.is_focused(window));
                    let next = if event.keystroke.modifiers.shift {
                        current.map_or(handles.len() - 1, |i| {
                            (i + handles.len() - 1) % handles.len()
                        })
                    } else {
                        current.map_or(0, |i| (i + 1) % handles.len())
                    };
                    window.focus(handles[next], cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::grid::CopyCells, _, cx| {
                this.activate(Action::Copy, cx)
            }))
            .flex()
            .flex_col()
            .min_h_0()
            .h(px(230.))
            .flex_shrink_0()
            .border_t_1()
            .border_color(crate::style::line())
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(self.data.column.clone())
                    .child(self.button(0, "Text", Action::View(View::Text), cx))
                    .child(self.button(1, "JSON", Action::View(View::Json), cx))
                    .child(self.button(2, "Hex", Action::View(View::Hex), cx))
                    .child(self.button(3, "Copy value", Action::Copy, cx))
                    .child(self.button(4, "Close inspector", Action::Close, cx)),
            )
            .child(
                div()
                    .id("inspected-value")
                    .role(Role::Label)
                    .aria_label(self.data.display().to_owned())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_2()
                    .child(self.data.display().to_owned()),
            )
            .child(
                div()
                    .id("inspection-status")
                    .role(Role::Label)
                    .aria_label(self.status.clone())
                    .child(self.status.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_is_distinct_from_text_and_empty_values_and_budget_is_released() {
        let budget = Rc::new(Cell::new(0));
        let null = Inspection::new("value".into(), &None, false, budget.clone()).unwrap();
        let text =
            Inspection::new("value".into(), &Some("NULL".into()), false, budget.clone()).unwrap();
        let empty =
            Inspection::new("value".into(), &Some(String::new()), false, budget.clone()).unwrap();
        assert!(null.value.is_none());
        assert_eq!(text.value.as_deref(), Some("NULL"));
        assert_eq!(empty.display(), "(empty string)");
        assert!(budget.get() > 0);
        drop((null, text, empty));
        assert_eq!(budget.get(), 0);
    }
    #[test]
    fn derived_view_refusal_preserves_raw_data_and_existing_budget() {
        let budget = Rc::new(Cell::new(0));
        let mut value = Inspection::new(
            "value".into(),
            &Some("{\"n\":9007199254740993}".into()),
            true,
            budget.clone(),
        )
        .unwrap();
        let before = value.display().to_owned();
        let bytes = budget.get();
        budget.set(WORKSPACE_BYTES);
        assert!(value.change_view(View::Json).is_err());
        assert_eq!(value.display(), before);
        assert_eq!(budget.get(), WORKSPACE_BYTES);
        budget.set(bytes);
        value.change_view(View::Json).unwrap();
        assert!(value.display().contains("9007199254740993"));
        assert_eq!(value.value.as_deref(), Some(before.as_str()));
        drop(value);
        assert_eq!(budget.get(), 0);
    }
}
