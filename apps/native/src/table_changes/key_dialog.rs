//! Virtual key and discard dialogs. The key editor keeps its existing rules:
//! a key is a user claim of uniqueness with full-row guards, and it can only
//! change while no staged, excluded or unknown-outcome changes exist.
use super::*;
use crate::{
    style,
    ui::{self, Variant, dialog},
};
use gpui::AnyElement;

impl TableChanges {
    pub(super) fn render_key_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pending = self.pending();
        let available = self.key_available();
        let stored = self.key.stored.as_ref().map(|key| key.columns.join(", "));
        let summary = if !self.key.loaded {
            if self.key.pending.is_some() {
                "Loading virtual key…".to_owned()
            } else {
                "Virtual key: unavailable".to_owned()
            }
        } else {
            format!("Virtual key: {}", stored.as_deref().unwrap_or("none"))
        };
        let mut body = dialog::body("virtual-key-body")
            .child(
                div()
                    .id("virtual-key-current")
                    .role(Role::Label)
                    .aria_label(summary.clone())
                    .text_color(style::dim())
                    .child(summary),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(style::faint())
                    .child("You claim these columns uniquely identify each row. Full-row guards apply; proven keys take precedence."),
            );
        if !self.key_draft_clear() {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(style::warn())
                    .child("Apply or discard staged changes before changing the virtual key."),
            );
        }
        if self.key.editing {
            let sources = self
                .analysis
                .as_ref()
                .map(|analysis| {
                    source_key_columns(
                        analysis,
                        self.source.relation().expect("table key controls"),
                    )
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let selected = sources.get(self.key.cursor).cloned();
            let choices = div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .child(self.button(
                    &format!("Key column: {}", selected.as_deref().unwrap_or("none")),
                    Action::KeyColumn,
                    available && selected.is_some(),
                    cx,
                ))
                .child(self.button(
                    "Add key column",
                    Action::KeyAdd,
                    available && selected.is_some(),
                    cx,
                ));
            body = body.child(choices);
            let mut selected_columns = div()
                .id("selected-key-columns")
                .flex()
                .flex_col()
                .gap(px(2.))
                .max_h(px(120.))
                .overflow_y_scroll();
            for (index, column) in self.key.columns.clone().iter().enumerate() {
                selected_columns = selected_columns.child(self.button(
                    &format!("Remove key column {}: {}", index + 1, column),
                    Action::KeyRemove(index),
                    available,
                    cx,
                ));
            }
            body = body.child(selected_columns).child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(self.button(
                        "Save virtual key",
                        Action::KeySave,
                        available
                            && valid_key_columns(&self.key.columns)
                            && self.analysis.is_some(),
                        cx,
                    ))
                    .child(self.button("Cancel key selection", Action::KeyCancel, !pending, cx)),
            );
        } else {
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(self.button(
                        "Choose virtual key",
                        Action::KeyEdit,
                        available && self.key.loaded && self.analysis.is_some(),
                        cx,
                    ))
                    .child(self.button(
                        "Clear virtual key",
                        Action::KeyClear,
                        available && self.key.loaded && self.key.stored.is_some(),
                        cx,
                    ))
                    .child(self.button("Reload virtual key", Action::KeyReload, available, cx)),
            );
        }
        if !self.key.status.is_empty() {
            body = body.child(
                div()
                    .id("virtual-key-status")
                    .role(Role::Status)
                    .aria_label(self.key.status.clone())
                    .text_sm()
                    .child(self.key.status.clone()),
            );
        }
        let close = self.dialog_button(
            "virtual-key-close",
            "Close",
            Action::CloseDialog,
            Variant::Secondary,
            self.key.pending.is_none(),
            cx,
        );
        let modal = dialog::modal("virtual-key-dialog", "Virtual key", 420.)
            .child(dialog::header("Virtual key", None))
            .child(body)
            .child(dialog::footer().child(close));
        dialog::backdrop("virtual-key-backdrop")
            .child(ui::appear("virtual-key-appear", self.modal_keys(modal, cx)))
            .into_any_element()
    }

    pub(super) fn render_discard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let summary = self.summary();
        let pending = self.pending();
        let text = if self.unrestored.is_some() {
            "Saved changes could not be recovered. Discarding removes them permanently; export the workspace first if you need them.".to_owned()
        } else {
            format!(
                "{} staged change{} will be removed. Nothing is written to the database.",
                summary.staged,
                if summary.staged == 1 { "" } else { "s" }
            )
        };
        let keep = self.dialog_button(
            "discard-keep",
            "Keep changes",
            Action::CancelDiscard,
            Variant::Ghost,
            true,
            cx,
        );
        let discard = self.dialog_button(
            "discard-confirm",
            "Discard changes",
            Action::ConfirmDiscard,
            Variant::Danger,
            !pending,
            cx,
        );
        let modal = dialog::modal("discard-dialog", "Discard staged changes?", 420.)
            .child(dialog::header("Discard staged changes?", None))
            .child(
                dialog::body("discard-body").child(
                    div()
                        .id("discard-text")
                        .role(Role::Label)
                        .aria_label(text.clone())
                        .text_color(style::dim())
                        .child(text),
                ),
            )
            .child(dialog::footer().child(keep).child(discard));
        dialog::backdrop("discard-backdrop")
            .child(ui::appear("discard-appear", self.modal_keys(modal, cx)))
            .into_any_element()
    }
}
