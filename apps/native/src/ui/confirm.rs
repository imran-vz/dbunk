//! Plan 032 typed confirmation for writes that need a deliberate unlock
//! (production or strict safe mode). The field starts empty for every review
//! and never carries text across dialogs; callers create a fresh one each time.
use crate::accessible_editor::AccessibleEditor;
use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Render, Subscription,
    Window, div, prelude::*,
};

/// The exact word a user types to unlock the apply button.
pub const CONFIRM_WORD: &str = "confirm";

/// Case-sensitive match that ignores surrounding whitespace only.
pub fn typed_confirmation_matches(input: &str) -> bool {
    input.trim() == CONFIRM_WORD
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedConfirmEvent {
    /// The match state flipped; carries the new state.
    Changed(bool),
    /// Enter was pressed while the text matched.
    Submit,
}

pub struct TypedConfirm {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    matched: bool,
    _subscription: Subscription,
}

impl EventEmitter<TypedConfirmEvent> for TypedConfirm {}

impl TypedConfirm {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let accessible = cx.new(|cx| {
            AccessibleEditor::field(editor.clone(), "Type confirm to apply", false, cx)
        });
        let subscription = cx.subscribe(&editor, |this: &mut Self, editor, event, cx| {
            if !matches!(event, EditorEvent::BufferEdited) {
                return;
            }
            let matched = typed_confirmation_matches(&editor.read(cx).text(cx));
            if matched != this.matched {
                this.matched = matched;
                cx.emit(TypedConfirmEvent::Changed(matched));
                cx.notify();
            }
        });
        Self {
            editor,
            accessible,
            matched: false,
            _subscription: subscription,
        }
    }

    pub fn matches(&self) -> bool {
        self.matched
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.editor.focus_handle(cx), cx);
    }
}

impl Focusable for TypedConfirm {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for TypedConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("typed-confirm")
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                if keystroke.key != "enter"
                    || keystroke.modifiers.modified()
                    || !this.matched
                    || !this.editor.focus_handle(cx).contains_focused(window, cx)
                {
                    return;
                }
                let composing = this.editor.update(cx, |editor, cx| {
                    gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
                });
                if !composing {
                    cx.emit(TypedConfirmEvent::Submit);
                    cx.stop_propagation();
                }
            }))
            .child(super::labelled(
                "Type confirm to apply",
                super::input_frame(false)
                    .border_color(if self.matched {
                        crate::style::primary_line()
                    } else {
                        crate::style::line()
                    })
                    .child(div().flex_1().min_w_0().child(self.accessible.clone())),
                Some((
                    format!("Type \"{CONFIRM_WORD}\" exactly, in lower case.").into(),
                    false,
                )),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_lower_case_word_matches() {
        for input in ["confirm", " confirm\n", "\tconfirm  "] {
            assert!(typed_confirmation_matches(input), "{input:?}");
        }
        for input in [
            "Confirm",
            "CONFIRM",
            "confirm.",
            "con firm",
            "",
            "   ",
            "confirmed",
            "conf",
        ] {
            assert!(!typed_confirmation_matches(input), "{input:?}");
        }
    }
}
