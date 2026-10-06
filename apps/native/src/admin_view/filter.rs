//! Small captured-settings filter. It never sends a database request while typing.
use crate::accessible_editor::{AccessibleEditor, fresh_editor};
use editor::{Editor, EditorEvent, EditorMode};
use gpui::{
    Context, Entity, EntityInputHandler, FocusHandle, Focusable, Subscription, Window, div,
    prelude::*,
};

use std::{cell::Cell, rc::Rc};
const MAX_TEXT: usize = crate::server_details_model::MAX_SEARCH_BYTES;
const FILTER_ALLOWANCE: usize = 256 * 1024;
const WORKSPACE_ALLOWANCE: usize = 128 * 1024 * 1024;
struct Lease(Rc<Cell<usize>>);
impl Lease {
    fn admit(budget: Rc<Cell<usize>>) -> Option<Self> {
        if FILTER_ALLOWANCE > WORKSPACE_ALLOWANCE.saturating_sub(budget.get()) {
            return None;
        }
        budget.set(budget.get() + FILTER_ALLOWANCE);
        Some(Self(budget))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(FILTER_ALLOWANCE));
    }
}
const REFRESH_HISTORY: usize = 64 * 1024;
const MAX_HISTORY: usize = 128 * 1024;

#[derive(Default)]
struct History {
    committed: String,
    previous: usize,
    cost: usize,
}
#[derive(PartialEq, Eq, Debug)]
enum Change {
    Keep,
    Refresh,
    Refuse,
}
impl History {
    fn change(&mut self, bytes: usize, edited: bool, marked: bool) -> Change {
        if edited {
            self.cost = self
                .cost
                .saturating_add(self.previous)
                .saturating_add(bytes)
                .saturating_add(256);
            self.previous = bytes;
        }
        if bytes > MAX_TEXT || (marked && self.cost >= MAX_HISTORY) {
            Change::Refuse
        } else if !marked && self.cost >= REFRESH_HISTORY {
            Change::Refresh
        } else {
            Change::Keep
        }
    }
}

pub(super) struct FilterInput {
    budget: Rc<Cell<usize>>,
    lease: Option<Lease>,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    history: History,
    notice: Option<&'static str>,
    subscription: Option<Subscription>,
}
impl FilterInput {
    pub(super) fn new(
        budget: Rc<Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_read_only(true);
            editor
        });
        let accessible = cx.new(|cx| {
            AccessibleEditor::field(editor.clone(), "Search captured settings", false, cx)
        });
        let mut field = Self {
            budget,
            lease: None,
            editor,
            accessible,
            history: History::default(),
            notice: None,
            subscription: None,
        };
        field.try_admit(cx);
        field.subscribe(window, cx);
        field
    }
    pub(super) fn try_admit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.lease.is_none() {
            self.lease = Lease::admit(self.budget.clone());
        }
        let admitted = self.lease.is_some();
        self.editor
            .update(cx, |editor, _| editor.set_read_only(!admitted));
        if !admitted {
            self.notice = Some(
                "Search needs shared memory. Clear a capture, then press Search to enable input",
            );
        } else if self
            .notice
            .is_some_and(|notice| notice.starts_with("Search needs"))
        {
            self.notice = None;
        }
        cx.notify();
        admitted
    }
    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscription =
            Some(
                cx.subscribe_in(&self.editor, window, |this, editor, event, window, cx| {
                    if editor != &this.editor
                        || !matches!(
                            event,
                            EditorEvent::BufferEdited
                                | EditorEvent::InputHandled { .. }
                                | EditorEvent::SelectionsChanged { .. }
                                | EditorEvent::Blurred
                        )
                    {
                        return;
                    }
                    this.check_field(matches!(event, EditorEvent::BufferEdited), window, cx);
                }),
            );
    }
    fn check_field(&mut self, edited: bool, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let bytes = editor.read(cx).buffer().read(cx).len(cx).0;
        let marked = self.composing(window, cx);
        let change = self.history.change(bytes, edited, marked);
        if change == Change::Keep {
            if !marked {
                self.history.committed = editor.read(cx).text(cx);
            }
            return;
        }
        let value = if change == Change::Refuse {
            self.history.committed.clone()
        } else {
            editor.read(cx).text(cx)
        };
        let focused = editor.focus_handle(cx).is_focused(window);
        let old_selection = editor.update(cx, |editor, cx| {
            editor
                .selections
                .newest::<multi_buffer::MultiBufferOffset>(&editor.display_snapshot(cx))
        });
        self.history = History {
            previous: value.len(),
            committed: value.clone(),
            cost: 0,
        };
        let replacement = cx.new(|cx| {
            let mut editor = fresh_editor(value, EditorMode::SingleLine, window, cx);
            editor.set_read_only(self.lease.is_none());
            editor
        });
        if change == Change::Refresh {
            replacement.update(cx, |editor, cx| {
                editor.change_selections(
                    editor::SelectionEffects::no_scroll(),
                    window,
                    cx,
                    |selections| {
                        let (anchor, head) = if old_selection.reversed {
                            (old_selection.end, old_selection.start)
                        } else {
                            (old_selection.start, old_selection.end)
                        };
                        selections.select_ranges([anchor..head]);
                    },
                )
            });
        }
        if focused {
            window.focus(&replacement.focus_handle(cx), cx);
        }
        self.accessible = cx.new(|cx| {
            AccessibleEditor::field(replacement.clone(), "Search captured settings", false, cx)
        });
        self.editor = replacement;
        self.notice = Some(if change == Change::Refuse {
            "Filter edit refused at its text/history limit; previous committed text restored"
        } else {
            "Filter undo history cleared at its size limit"
        });
        cx.defer_in(window, |this, window, cx| this.subscribe(window, cx));
        cx.notify();
    }
    pub(super) fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.editor.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        })
    }
    pub(super) fn value(&self, cx: &gpui::App) -> Result<String, &'static str> {
        if self.editor.read(cx).buffer().read(cx).len(cx).0 > MAX_TEXT {
            return Err("Settings search exceeds 1 KiB");
        }
        if self.lease.is_none() {
            return Err("Search input has no shared memory allowance");
        }
        let value = self.editor.read(cx).text(cx);
        crate::server_details_model::validate_filter_query(&value)?;
        Ok(value)
    }
}
impl Focusable for FilterInput {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}
impl Render for FilterInput {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w_full()
            .child(div().h(gpui::px(28.)).child(self.accessible.clone()))
            .when_some(self.notice, |view, notice| {
                view.child(
                    div()
                        .id("server-filter-notice")
                        .role(gpui::Role::Status)
                        .aria_label(notice)
                        .child(notice),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_filter_reservation_refuses_without_changing_budget_and_releases() {
        let budget = Rc::new(Cell::new(WORKSPACE_ALLOWANCE - FILTER_ALLOWANCE));
        let lease = Lease::admit(budget.clone()).unwrap();
        assert_eq!(budget.get(), WORKSPACE_ALLOWANCE);
        assert!(Lease::admit(budget.clone()).is_none());
        assert_eq!(budget.get(), WORKSPACE_ALLOWANCE);
        drop(lease);
        assert_eq!(budget.get(), WORKSPACE_ALLOWANCE - FILTER_ALLOWANCE);
    }
    #[test]
    fn filter_keeps_normal_composition_but_bounds_text_and_retained_history() {
        let mut history = History::default();
        assert_eq!(history.change(MAX_TEXT, true, true), Change::Keep);
        assert_eq!(history.change(MAX_TEXT + 1, true, false), Change::Refuse);
        history.cost = REFRESH_HISTORY;
        assert_eq!(history.change(20, false, true), Change::Keep);
        assert_eq!(history.change(20, false, false), Change::Refresh);
        history.cost = MAX_HISTORY;
        assert_eq!(history.change(20, false, true), Change::Refuse);
    }
}
