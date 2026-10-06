//! The changes overlay layer (plan 032): the popover editor, the review,
//! discard and virtual-key dialogs, and the staged-change list. Nothing
//! renders, and nothing takes pointer input, while none is open. The inline
//! cell editor is rendered by the grid through `inline_editor()`.
use super::*;
use crate::{
    style,
    ui::{self, Variant, dialog, popover},
};
use gpui::{AnyElement, Stateful};

enum Layer {
    Review,
    Discard,
    VirtualKey,
    ChangeList(Bounds<Pixels>),
}

impl TableChanges {
    /// Gives a styled control its focus handle, AX state and activation.
    pub(super) fn wire(
        &mut self,
        key: SharedString,
        control: Stateful<gpui::Div>,
        action: Action,
        enabled: bool,
        checked: Option<bool>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let focus = self
            .buttons
            .entry(key.to_string())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let enabled = enabled && self.enabled;
        let weak = cx.weak_entity();
        self.rendered_buttons.push(focus.clone());
        if enabled {
            self.visible_buttons.push(focus.clone());
        }
        control
            .role(if checked.is_some() {
                Role::CheckBox
            } else {
                Role::Button
            })
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if let Some(checked) = checked {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(checked));
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.activate(action, window, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
                }
            })
            .into_any_element()
    }
    /// A 20 px tool button (editor footer toggles, virtual key controls).
    pub(super) fn button(
        &mut self,
        label: &str,
        action: Action,
        enabled: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let label = SharedString::from(label.to_string());
        let key = SharedString::from(match action {
            Action::Include(id, _) => format!("include-{id}"),
            Action::Remove(id) => format!("remove-{id}"),
            Action::KeyRemove(index) => format!("key-remove-{index}"),
            _ => format!("change-{action:?}"),
        });
        let checked = match action {
            Action::RawValue => Some(self.edit.as_ref().is_some_and(|edit| edit.raw)),
            Action::Null => Some(self.edit.as_ref().is_some_and(|edit| edit.null)),
            Action::Include(_, next) => Some(!next),
            _ => None,
        };
        let primary = matches!(action, Action::Stage);
        let control = crate::ui::pressed(
            crate::ui::tool_button(key.clone(), label, None, enabled, primary),
            checked.unwrap_or(false),
        );
        self.wire(key, control, action, enabled, checked, cx)
    }
    /// A 24 px dialog button with an explicit variant.
    pub(super) fn dialog_button(
        &mut self,
        key: &'static str,
        label: &str,
        action: Action,
        variant: Variant,
        enabled: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let control = ui::button(key, label.to_owned(), variant, enabled);
        self.wire(key.into(), control, action, enabled, None, cx)
    }
    pub fn focus_handles(&self, cx: &App) -> Vec<FocusHandle> {
        self.edit_handles(cx, false)
            .into_iter()
            .chain(
                self.review_dialog()
                    .and_then(|dialog| dialog.typed.as_ref())
                    .map(|typed| typed.focus_handle(cx)),
            )
            .chain(self.visible_buttons.iter().cloned())
            .collect()
    }
    fn cycle_focus(&self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handles = self.focus_handles(cx);
        if handles.is_empty() {
            return;
        }
        let len = handles.len();
        let next = match handles.iter().position(|handle| handle.is_focused(window)) {
            Some(index) if backwards => (index + len - 1) % len,
            Some(index) => (index + 1) % len,
            None if backwards => len - 1,
            None => 0,
        };
        window.focus(&handles[next], cx);
    }
    /// Esc and a Tab cycle that stays inside the open dialog.
    pub(super) fn modal_keys(
        &self,
        modal: Stateful<gpui::Div>,
        cx: &Context<Self>,
    ) -> Stateful<gpui::Div> {
        modal.capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            let keystroke = &event.keystroke;
            let modifiers = keystroke.modifiers;
            match keystroke.key.as_str() {
                "escape" if !modifiers.modified() => {
                    cx.stop_propagation();
                    let review = matches!(this.dialog, Some(Dialog::Review(_)));
                    let discard = matches!(this.dialog, Some(Dialog::Discard));
                    if review {
                        this.cancel_review(cx);
                    } else if discard {
                        this.activate(Action::CancelDiscard, window, cx);
                    } else {
                        this.close_dialog(cx);
                    }
                    cx.notify();
                }
                "tab" if !modifiers.control && !modifiers.alt && !modifiers.platform => {
                    cx.stop_propagation();
                    this.cycle_focus(modifiers.shift, window, cx);
                }
                _ => {}
            }
        }))
    }

    fn render_edit_popover(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pending = self.pending();
        let edit = self.edit.as_ref().unwrap();
        let title = edit
            .column
            .clone()
            .unwrap_or_else(|| "Insert JSON object".into());
        let context = edit.context.label();
        let kind = edit.kind;
        let null = edit.null;
        let raw = edit.raw;
        let default = edit.default;
        let error = edit.error.clone();
        let array = edit.array.clone();
        let accessible = edit.accessible.clone();
        let cell = edit.column.is_some();
        let bulk = matches!(edit.context, batch_edit::EditContext::Bulk(_));
        let geometry = (kind == Some(Kind::Geometry) && !null && !raw)
            .then(|| crate::geometry_preview::parse(&edit.editor.read(cx).text(cx)));

        let mut body = div().flex().flex_col().gap(px(6.)).px(px(8.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(style::MONO)
                        .text_color(style::dim())
                        .child(title.clone()),
                )
                .when(null, |row| row.child(ui::badge("NULL")))
                .when(default, |row| row.child(ui::badge("DEFAULT"))),
        );
        if let Some(label) = context {
            body = body.child(
                div()
                    .id("table-edit-context")
                    .role(Role::Label)
                    .aria_label(label.clone())
                    .text_sm()
                    .text_color(style::dim())
                    .child(label),
            );
        }
        if let Some(array) = array {
            let enabled = self.enabled && !pending && !null;
            array.update(cx, |array, cx| array.set_enabled(enabled, cx));
            body = body.child(div().max_h(px(220.)).child(array));
        } else {
            body = body.child(
                div()
                    .h(px(150.))
                    .px_1()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if error.is_some() {
                        style::bad_line()
                    } else {
                        style::line()
                    })
                    .bg(style::bg())
                    .when(null, |frame| frame.opacity(0.55))
                    .child(accessible),
            );
        }
        match geometry {
            Some(Ok(preview)) => {
                let summary = format!(
                    "{:?} · {} points · bounds {}",
                    preview.shape,
                    preview.points.len(),
                    preview.bounds
                );
                body = body
                    .child(
                        div()
                            .id("geometry-preview")
                            .role(Role::Image)
                            .aria_label(format!("Geometry preview: {summary}"))
                            .border_1()
                            .border_color(style::line())
                            .child(crate::geometry_preview::render(&preview)),
                    )
                    .child(div().text_xs().child(summary));
            }
            Some(Err(message)) => {
                body = body.child(
                    div()
                        .id("geometry-preview-refusal")
                        .role(Role::Label)
                        .aria_label(message)
                        .text_xs()
                        .text_color(style::warn())
                        .child(message),
                );
            }
            None => {}
        }
        if kind == Some(Kind::Geometry) {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(style::faint())
                    .child("WKT prefix check only; the database still validates the value"),
            );
        }
        if let Some(error) = error {
            body = body.child(ui::error_banner("cell-editor-error", error));
        }

        let mut tools = div().flex().flex_wrap().items_center().gap(px(4.));
        if cell {
            tools = tools.child(self.button("Set NULL", Action::Null, !pending, cx));
        }
        if matches!(kind, Some(Kind::Json | Kind::Array)) {
            tools = tools.child(self.button(
                "Format",
                Action::FormatValue,
                !pending && !null && !raw,
                cx,
            ));
        }
        if kind.is_some() {
            tools = tools.child(self.button("Raw literal", Action::RawValue, !pending, cx));
        }
        if kind == Some(Kind::Geometry) {
            tools = tools.child(self.button("Copy EWKT", Action::CopyLiteral, true, cx));
        }
        if bulk {
            tools = tools.child(self.button("Next column", Action::BulkColumn, !pending, cx));
        }
        let cancel = self.dialog_button(
            "cell-editor-cancel",
            "Cancel",
            Action::CancelEdit,
            Variant::Ghost,
            !pending,
            cx,
        );
        let save = self.dialog_button(
            "cell-editor-save",
            "Save ⌘↵",
            Action::Stage,
            Variant::Primary,
            !pending,
            cx,
        );
        let footer = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .pt(px(4.))
            .border_t_1()
            .border_color(style::line_soft())
            .child(tools)
            .child(ui::grow())
            .child(cancel)
            .child(save);

        let panel = popover::panel("cell-popover-editor", Role::Dialog, format!("Edit {title}"))
            .w(px(480.))
            .gap(px(6.))
            // Modal-like: outside clicks never discard or stage the draft.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let keystroke = &event.keystroke;
                let action = if keystroke.key == "escape" && !keystroke.modifiers.modified() {
                    Some(Action::CancelEdit)
                } else if keystroke.key == "enter"
                    && keystroke.modifiers.platform
                    && !keystroke.modifiers.shift
                    && !keystroke.modifiers.alt
                {
                    Some(Action::Stage)
                } else {
                    None
                };
                if let Some(action) = action
                    && !this.composition_active(window, cx)
                {
                    this.activate(action, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(body)
            .child(footer);
        match self.popover_anchor {
            Some(anchor) => {
                popover::layer(anchor, popover::Placement::Below, panel).into_any_element()
            }
            // No cell on screen (legacy JSON insert, or before the host sends
            // the anchor): centre it over the table instead.
            None => dialog::backdrop("cell-editor-backdrop")
                .child(panel)
                .into_any_element(),
        }
    }
}

impl Render for TableChanges {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_buttons.clear();
        self.rendered_buttons.clear();
        let mut root = div().size_full();
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| edit.presentation == Presentation::Popover)
        {
            root = root.child(self.render_edit_popover(cx));
        }
        let layer = match &self.dialog {
            Some(Dialog::Review(_)) => Some(Layer::Review),
            Some(Dialog::Discard) => Some(Layer::Discard),
            Some(Dialog::VirtualKey) => Some(Layer::VirtualKey),
            Some(Dialog::ChangeList(anchor)) => Some(Layer::ChangeList(*anchor)),
            None => None,
        };
        let content = match layer {
            Some(Layer::Review) => Some(self.render_review(window, cx)),
            Some(Layer::Discard) => Some(self.render_discard(cx)),
            Some(Layer::VirtualKey) => Some(self.render_key_dialog(cx)),
            Some(Layer::ChangeList(anchor)) => Some(self.render_change_list(anchor, cx)),
            None => None,
        };
        root = root.children(content);
        // Removed rows must not leave focus handles accumulating for this tab.
        self.buttons
            .retain(|_, handle| self.rendered_buttons.contains(handle));
        if self.focus_request {
            self.focus_request = false;
            let target = self
                .review_dialog()
                .and_then(|dialog| dialog.typed.as_ref())
                .map(|typed| typed.focus_handle(cx))
                .or_else(|| self.visible_buttons.first().cloned());
            if let Some(target) = target {
                window.defer(cx, move |window, cx| window.focus(&target, cx));
            }
        }
        root
    }
}
