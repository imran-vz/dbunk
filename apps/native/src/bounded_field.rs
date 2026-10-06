//! Short text fields own bounded histories inside the caller's shared lease.
//! Admit editor/history working memory before constructing a field.
use crate::accessible_editor::{AccessibleEditor, fresh_editor};
use editor::{Editor, EditorEvent, EditorMode};
use gpui::{
    Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Subscription,
    Window, div, prelude::*,
};

pub(super) struct Changed;

pub(super) struct Field {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    label: &'static str,
    limit: usize,
    multiline: bool,
    committed: String,
    previous_bytes: usize,
    history_bytes: usize,
    readonly: bool,
    notice: Option<&'static str>,
    subscription: Option<Subscription>,
}
impl EventEmitter<Changed> for Field {}
fn field_mode(multiline: bool) -> EditorMode {
    if multiline {
        EditorMode::AutoHeight {
            min_lines: 1,
            max_lines: Some(3),
        }
    } else {
        EditorMode::SingleLine
    }
}

impl Field {
    pub(super) fn new(
        label: &'static str,
        limit: usize,
        multiline: bool,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| fresh_editor(text.clone(), field_mode(multiline), window, cx));
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, false, cx));
        let mut this = Self {
            editor,
            accessible,
            label,
            limit,
            multiline,
            previous_bytes: text.len(),
            committed: text,
            history_bytes: 0,
            readonly: false,
            notice: None,
            subscription: None,
        };
        this.subscribe(window, cx);
        this
    }
    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscription = Some(cx.subscribe_in(
            &self.editor,
            window,
            |this, editor, event, window, cx| {
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
                let bytes = editor.read(cx).buffer().read(cx).len(cx).0;
                if matches!(event, EditorEvent::BufferEdited) {
                    this.history_bytes = this
                        .history_bytes
                        .saturating_add(this.previous_bytes)
                        .saturating_add(bytes)
                        .saturating_add(256);
                    this.previous_bytes = bytes;
                }
                let marked = this.composing(window, cx);
                let refuse = bytes > this.limit || (marked && this.history_bytes >= 128 * 1024);
                let refresh = !marked && this.history_bytes >= 64 * 1024;
                if !refuse && !refresh {
                    if !marked {
                        let text = editor.read(cx).text(cx);
                        if this.committed != text {
                            this.committed = text;
                            cx.emit(Changed);
                            cx.notify();
                        }
                    }
                    return;
                }
                let text = if refuse {
                    this.committed.clone()
                } else {
                    editor.read(cx).text(cx)
                };
                let selected = editor.update(cx, |editor, cx| {
                    editor
                        .selections
                        .newest::<multi_buffer::MultiBufferOffset>(&editor.display_snapshot(cx))
                });
                let focused = editor.focus_handle(cx).is_focused(window);
                let replacement = cx.new(|cx| {
                    let mut editor =
                        fresh_editor(text.clone(), field_mode(this.multiline), window, cx);
                    editor.set_read_only(this.readonly);
                    editor
                });
                if refresh && !refuse {
                    replacement.update(cx, |editor, cx| {
                        editor.change_selections(
                            editor::SelectionEffects::no_scroll(),
                            window,
                            cx,
                            |selections| {
                                let (anchor, head) = if selected.reversed {
                                    (selected.end, selected.start)
                                } else {
                                    (selected.start, selected.end)
                                };
                                selections.select_ranges([anchor..head]);
                            },
                        )
                    });
                }
                if focused {
                    window.focus(&replacement.focus_handle(cx), cx);
                }
                this.accessible = cx
                    .new(|cx| AccessibleEditor::field(replacement.clone(), this.label, false, cx));
                this.editor = replacement;
                this.previous_bytes = text.len();
                let changed = this.committed != text;
                this.committed = text;
                this.history_bytes = 0;
                this.notice = Some(if refuse {
                    "Edit exceeded the field/history limit; committed text restored"
                } else {
                    "Undo history cleared at its size limit"
                });
                cx.defer_in(window, |this, window, cx| this.subscribe(window, cx));
                if changed {
                    cx.emit(Changed);
                }
                cx.notify();
            },
        ));
    }
    pub(super) fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.editor.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        })
    }
    pub(super) fn value(&self, cx: &gpui::App) -> Result<String, &'static str> {
        if self.editor.read(cx).buffer().read(cx).len(cx).0 > self.limit {
            return Err("Field exceeds its byte limit");
        }
        Ok(self.editor.read(cx).text(cx))
    }
    /// Programmatic selection keeps the existing editor/focus. Never replace or
    /// overwrite an active marked range; the caller can retry after composition.
    pub(super) fn set_value(
        &mut self,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        if value.len() > self.limit {
            return Err("Field exceeds its byte limit");
        }
        if self.composing(window, cx) {
            return Err("Finish text composition before changing this field");
        }
        self.editor
            .update(cx, |editor, cx| editor.set_text(value, window, cx));
        Ok(())
    }
    pub(super) fn set_readonly(&mut self, readonly: bool, cx: &mut Context<Self>) {
        if self.readonly == readonly {
            return;
        }
        self.readonly = readonly;
        self.editor
            .update(cx, |editor, _| editor.set_read_only(readonly));
        cx.notify();
    }
}
impl Focusable for Field {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}
impl Render for Field {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(self.accessible.clone())
            .when_some(self.notice, |row, notice| row.child(notice))
    }
}
