//! Go-to-row owns only a bounded field. It never fetches rows or changes a query.
use crate::bounded_field::Field;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, Window, div,
    prelude::*, px,
};
use std::{cell::Cell, rc::Rc};

const FIELD_BYTES: usize = 24;
// Covers the field, committed copies and bounded editor history, including
// transient replacement at the history limit. Payload accounting, not RSS.
const RESERVATION: usize = 1024 * 1024;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
struct Lease(Rc<Cell<usize>>);
impl Lease {
    fn admit(budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if budget.get() > WORKSPACE_BYTES - RESERVATION {
            return Err("Go to row needs 1 MiB of workspace allowance; clear another result");
        }
        budget.set(budget.get() + RESERVATION);
        Ok(Self(budget))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(RESERVATION));
    }
}

pub(super) struct Finished(pub Option<usize>);
pub(super) struct GoToRowView {
    field: Entity<Field>,
    buttons: [FocusHandle; 2],
    rows: usize,
    message: Option<&'static str>,
    _lease: Lease,
}
impl EventEmitter<Finished> for GoToRowView {}
impl GoToRowView {
    pub(super) fn create<T: 'static>(
        rows: usize,
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Result<Entity<Self>, &'static str> {
        if rows == 0 {
            return Err("No retained rows");
        }
        let lease = Lease::admit(budget)?;
        Ok(cx.new(|cx| Self {
            field: cx.new(|cx| {
                Field::new(
                    "Retained row number",
                    FIELD_BYTES,
                    false,
                    String::new(),
                    window,
                    cx,
                )
            }),
            buttons: [cx.focus_handle(), cx.focus_handle()],
            rows,
            message: None,
            _lease: lease,
        }))
    }
    pub(super) fn contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.field.focus_handle(cx).contains_focused(window, cx)
            || self.buttons.iter().any(|focus| focus.is_focused(window))
    }
    fn activate(&mut self, submit: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .field
            .update(cx, |field, cx| field.composing(window, cx))
        {
            return;
        }
        if !submit {
            cx.emit(Finished(None));
            return;
        }
        match self
            .field
            .read(cx)
            .value(cx)
            .and_then(|text| row_index(&text, self.rows))
        {
            Ok(row) => cx.emit(Finished(Some(row))),
            Err(message) => {
                self.message = Some(message);
                cx.notify();
            }
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .field
            .update(cx, |field, cx| field.composing(window, cx))
        {
            return;
        }
        let key = &event.keystroke;
        if key.modifiers.platform || key.modifiers.control || key.modifiers.alt {
            return;
        }
        match key.key.as_str() {
            "escape" => self.activate(false, window, cx),
            "enter" if self.field.focus_handle(cx).is_focused(window) => {
                self.activate(true, window, cx)
            }
            "tab" => {
                let handles = [
                    self.field.focus_handle(cx),
                    self.buttons[0].clone(),
                    self.buttons[1].clone(),
                ];
                let current = handles
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(0);
                let next = (current + if key.modifiers.shift { 2 } else { 1 }) % 3;
                window.focus(&handles[next], cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
    fn button(&self, index: usize, cx: &Context<Self>) -> impl IntoElement + use<> {
        let label = if index == 0 { "Go" } else { "Cancel" };
        let weak = cx.entity().downgrade();
        crate::ui::tool_button(("go-to-row-action", index), label, None, true, index == 0)
            .track_focus(&self.buttons[index])
            .tab_stop(true)
            .tab_index(0)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(index == 0, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(index == 0, window, cx))
                    .ok();
            })
    }
}
impl Focusable for GoToRowView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}
impl Render for GoToRowView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let range = format!("1–{} in this retained result or table page", self.rows);
        div()
            .id("go-to-row")
            .role(Role::Dialog)
            .aria_label("Go to retained row")
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .size_full()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_sm()
            .capture_key_down(cx.listener(Self::key))
            .child(crate::ui::section_label("Go to row"))
            .child(
                div()
                    .id("go-to-row-range")
                    .role(Role::Label)
                    .aria_label(range.clone())
                    .text_color(crate::style::dim())
                    .child(range),
            )
            .child(crate::ui::field().w(px(220.)).child(self.field.clone()))
            .when_some(self.message, |view, message| {
                view.child(
                    div()
                        .id("go-to-row-error")
                        .role(Role::Status)
                        .aria_label(message)
                        .a11y_synthetic_children(|builder| {
                            builder
                                .parent_node()
                                .set_live(gpui::accesskit::Live::Polite)
                        })
                        .child(message),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(self.button(0, cx))
                    .child(self.button(1, cx)),
            )
    }
}

/// One-based positive decimal input, clamped to the captured retained range.
/// Saturation accepts long valid numbers without wrapping or allocating.
fn row_index(text: &str, rows: usize) -> Result<usize, &'static str> {
    let text = text.trim();
    if rows == 0
        || text.is_empty()
        || text.len() > FIELD_BYTES
        || !text.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("Enter a positive whole row number");
    }
    let row = text.bytes().fold(0usize, |row, c| {
        row.saturating_mul(10).saturating_add(usize::from(c - b'0'))
    });
    if row == 0 {
        return Err("Row numbers start at 1");
    }
    Ok(row.min(rows) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_bounds_do_not_wrap_or_accept_partial_numbers() {
        for (text, expected) in [
            ("1", 0),
            (" 003 ", 2),
            ("60", 59),
            ("61", 59),
            ("999999999999999999999999", 59),
        ] {
            assert_eq!(row_index(text, 60), Ok(expected));
        }
        for text in [
            "",
            "0",
            "-1",
            "+1",
            "1.5",
            "3tail",
            "1e2",
            "三",
            "9999999999999999999999999",
        ] {
            assert!(row_index(text, 60).is_err(), "{text}");
        }
        assert!(row_index("1", 0).is_err());
    }
    #[test]
    fn field_admission_is_atomic_and_released() {
        let budget = Rc::new(Cell::new(WORKSPACE_BYTES - RESERVATION + 1));
        assert!(Lease::admit(budget.clone()).is_err());
        assert_eq!(budget.get(), WORKSPACE_BYTES - RESERVATION + 1);
        budget.set(WORKSPACE_BYTES - RESERVATION);
        let lease = Lease::admit(budget.clone()).unwrap();
        assert_eq!(budget.get(), WORKSPACE_BYTES);
        assert!(Lease::admit(budget.clone()).is_err());
        drop(lease);
        assert_eq!(budget.get(), WORKSPACE_BYTES - RESERVATION);
    }
}
