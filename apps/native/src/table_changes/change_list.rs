//! The staged-change list popover under the toolbar's "N changes" trigger:
//! include or exclude each change, remove it, discard all or review.
use super::*;
use crate::{
    style,
    ui::{self, popover},
};
use gpui::{AnyElement, MouseDownEvent};

/// Rows rendered in the popover; the draft holds at most 128 changes.
const LIST_ROWS: usize = 128;

pub(super) fn change_summary(operation: &MutationOp) -> String {
    let (verb, table, values) = match operation {
        MutationOp::Insert { table, values } => ("Insert", table, values.as_slice()),
        MutationOp::Update { table, set, .. } => ("Update", table, set.as_slice()),
        MutationOp::Delete { table, .. } => ("Delete", table, &[][..]),
    };
    // Values are shown in the review dialog. Keep this bounded list cheap to
    // paint even when a staged value is several megabytes long.
    let mut text = format!("{verb} {}.{}", table.schema, table.table);
    for value in values {
        if text.len() >= 512 {
            break;
        }
        text.push(' ');
        text.extend(value.column.chars().take(128));
    }
    text.chars().take(512).collect()
}

impl TableChanges {
    pub(super) fn render_change_list(
        &mut self,
        anchor: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selectable = self.can_select_now().is_ok();
        let changes = self
            .draft
            .as_ref()
            .map(|draft| {
                draft
                    .changes()
                    .take(LIST_ROWS)
                    .map(|(id, included, operation)| (id, included, change_summary(operation)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let count = changes.len();
        let mut rows = Vec::with_capacity(count);
        for (index, (id, included, text)) in changes.into_iter().enumerate() {
            let include = self.wire(
                format!("include-{id}").into(),
                ui::icon_button(
                    format!("include-{id}"),
                    format!("Include change {}", index + 1),
                    if included {
                        "icons/check.svg"
                    } else {
                        "icons/circle.svg"
                    },
                    selectable,
                ),
                Action::Include(id, !included),
                selectable,
                Some(included),
                cx,
            );
            let remove = self.wire(
                format!("remove-{id}").into(),
                ui::icon_button(
                    format!("remove-{id}"),
                    format!("Remove change {}", index + 1),
                    "icons/close.svg",
                    selectable,
                ),
                Action::Remove(id),
                selectable,
                None,
                cx,
            );
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(6.))
                    .h(px(style::ROW + 2.))
                    .child(include)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(style::MONO)
                            .text_color(if included {
                                style::text()
                            } else {
                                style::faint()
                            })
                            .child(text),
                    )
                    .child(remove),
            );
        }
        let can_review = self.review_gate().is_ok();
        let discard = self.button("Discard all", Action::Discard, self.applying.is_none(), cx);
        let review = self.button("Review", Action::Review, can_review, cx);
        let panel = popover::panel("change-list", Role::Dialog, "Staged changes")
            .w(px(380.))
            .child(popover::heading(format!(
                "{count} staged change{}",
                if count == 1 { "" } else { "s" }
            )))
            .child(
                div()
                    .id("pending-table-changes")
                    .role(Role::List)
                    .aria_label("Staged changes")
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(popover::divider())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .px(px(6.))
                    .child(ui::grow())
                    .child(discard)
                    .child(review),
            )
            // Outside clicks close it, except on its own trigger, whose click
            // then toggles it closed instead of reopening it.
            .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                if !anchor.contains(&event.position)
                    && matches!(this.dialog, Some(Dialog::ChangeList(_)))
                {
                    this.dialog = None;
                    cx.notify();
                }
            }));
        popover::layer(
            anchor,
            popover::Placement::BelowEnd,
            self.modal_keys(panel, cx),
        )
        .into_any_element()
    }
}
