//! Finite literal buffers and CRDT history. Normal marked input stays editor-owned.
use super::*;
use crate::accessible_editor::fresh_editor;
use editor::{EditorEvent, EditorMode};
use gpui::{EntityInputHandler, Subscription};
use multi_buffer::MultiBufferOffset;

const REFRESH_HISTORY: usize = 4 * 1024 * 1024;
const MAX_HISTORY: usize = 8 * 1024 * 1024;
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Change {
    Keep,
    Refresh,
    Refuse,
}
pub(super) struct History {
    pub(super) committed: String,
    previous: usize,
    cost: usize,
}
impl History {
    pub(super) fn new(text: String) -> Self {
        Self {
            previous: text.len(),
            committed: text,
            cost: 0,
        }
    }
    pub(super) fn change(&mut self, bytes: usize, marked: bool, edited: bool) -> Change {
        if edited {
            self.cost = self
                .cost
                .saturating_add(self.previous)
                .saturating_add(bytes)
                .saturating_add(256);
            self.previous = bytes;
        }
        if bytes > cell_value::MAX_VALUE_BYTES || (marked && self.cost >= MAX_HISTORY) {
            Change::Refuse
        } else if !marked && self.cost >= REFRESH_HISTORY {
            Change::Refresh
        } else {
            Change::Keep
        }
    }
}
pub(super) fn relevant(event: &EditorEvent) -> bool {
    matches!(
        event,
        EditorEvent::BufferEdited
            | EditorEvent::InputHandled { .. }
            | EditorEvent::SelectionsChanged { .. }
            | EditorEvent::Blurred
    )
}
pub(super) fn replace<T: 'static>(
    old: &Entity<Editor>,
    value: String,
    refused: bool,
    label: String,
    multiline: bool,
    window: &mut Window,
    cx: &mut Context<T>,
) -> (Entity<Editor>, Entity<AccessibleEditor>) {
    let focused = old.focus_handle(cx).is_focused(window);
    let read_only = old.read(cx).read_only(cx);
    let selection = old.update(cx, |editor, cx| {
        let snapshot = editor.display_snapshot(cx);
        editor.selections.newest::<MultiBufferOffset>(&snapshot)
    });
    let (anchor, head) = if refused {
        (value.len(), value.len())
    } else if selection.reversed {
        (selection.end.0, selection.start.0)
    } else {
        (selection.start.0, selection.end.0)
    };
    let editor = cx.new(|cx| {
        let mut editor = if multiline {
            let buffer = cx.new(|cx| language::Buffer::local(value, cx));
            Editor::for_buffer(buffer, None, window, cx)
        } else {
            fresh_editor(value, EditorMode::SingleLine, window, cx)
        };
        editor.set_read_only(read_only);
        editor
    });
    editor.update(cx, |editor, cx| {
        editor.change_selections(
            editor::SelectionEffects::no_scroll(),
            window,
            cx,
            |selections| {
                selections.select_ranges([MultiBufferOffset(anchor)..MultiBufferOffset(head)]);
            },
        )
    });
    if focused {
        window.focus(&editor.focus_handle(cx), cx);
    }
    let accessible = cx.new(|cx| {
        if multiline {
            AccessibleEditor::new(editor.clone(), label, cx)
        } else {
            AccessibleEditor::field(editor.clone(), label, false, cx)
        }
    });
    (editor, accessible)
}
pub(super) fn notice(refused: bool) -> &'static str {
    if refused {
        "Edit refused at the 1 MiB text or 8 MiB history limit; restored the last committed value and cleared undo history"
    } else {
        "Undo history cleared at its size limit; current text and selection preserved"
    }
}

impl TableChanges {
    fn literal_subscription(&self, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(
            &self.edit.as_ref().unwrap().editor,
            window,
            |this, editor, event, window, cx| {
                if this.edit.as_ref().is_none_or(|edit| &edit.editor != editor) || !relevant(event)
                {
                    return;
                }
                this.check_literal(matches!(event, EditorEvent::BufferEdited), window, cx);
            },
        )
    }
    pub(super) fn install_literal_guard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edit.is_none() {
            return;
        }
        let subscription = self.literal_subscription(window, cx);
        if let Some(edit) = &mut self.edit {
            edit._literal_events = Some(subscription);
        }
    }
    fn check_literal(&mut self, edited: bool, window: &mut Window, cx: &mut Context<Self>) {
        let edit = self.edit.as_mut().unwrap();
        let editor = edit.editor.clone();
        let length = editor.read(cx).buffer().read(cx).len(cx).0;
        let marked = editor.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        });
        if edited {
            // Typing replaces the DEFAULT placeholder; inline typing also
            // replaces NULL (the popover keeps its explicit NULL toggle).
            edit.default = false;
            edit.error = None;
            if edit.presentation == Presentation::Inline {
                edit.null = false;
            }
        }
        let change = edit.history.change(length, marked, edited);
        if change == Change::Keep {
            if !marked {
                edit.history.committed = editor.read(cx).text(cx);
            }
            return;
        }
        let refused = change == Change::Refuse;
        let value = if refused {
            edit.history.committed.clone()
        } else {
            editor.read(cx).text(cx)
        };
        edit.history = History::new(value.clone());
        let label = edit.column.as_ref().map_or_else(
            || "Insert row JSON, omitted columns use defaults".to_owned(),
            |column| format!("Value for {column}"),
        );
        let (replacement, accessible) = replace(
            &editor,
            value,
            refused,
            label,
            edit.multiline(),
            window,
            cx,
        );
        edit.editor = replacement;
        edit.accessible = accessible;
        self.relabel_batch(cx);
        self.sync_cell_editor(cx);
        cx.defer_in(window, |this, window, cx| {
            this.install_literal_guard(window, cx)
        });
        self.message = notice(refused).into();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_composition_is_retained_but_large_input_and_history_are_refused() {
        let mut history = History::new("before".into());
        assert_eq!(history.change(20, true, true), Change::Keep);
        assert_eq!(history.committed, "before");
        assert_eq!(
            history.change(cell_value::MAX_VALUE_BYTES + 1, true, true),
            Change::Refuse
        );
        assert_eq!(history.committed, "before");
        let mut history = History::new(String::new());
        for _ in 0..10 {
            if history.change(cell_value::MAX_VALUE_BYTES, true, true) == Change::Refuse {
                return;
            }
        }
        panic!("marked editing history must have a finite cap");
    }
    #[test]
    fn unmarked_history_refresh_preserves_current_text_instead_of_refusing_it() {
        let mut history = History::new(String::new());
        assert_eq!(history.change(1024 * 1024, false, true), Change::Keep);
        assert_eq!(history.change(1024 * 1024, false, true), Change::Keep);
        assert_eq!(history.change(1024 * 1024, false, true), Change::Refresh);
    }
}
