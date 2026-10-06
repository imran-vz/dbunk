//! The two-pane sort popover (§3.8): unsorted columns with a search on the
//! left (activating one appends it), the ordered keys on the right. Every
//! change applies at once through `BrowseEvent::Apply`.
use super::{Action, BrowseControls};
use crate::{style, ui, ui::popover};
use dbunk_lib::backend::data::*;
use gpui::{AnyElement, Context, Role, div, prelude::*, px};

const PANE_HEIGHT: f32 = 280.;

impl BrowseControls {
    /// The panel body and the length of its keyboard list (the candidates).
    pub(super) fn sort_panel(&mut self, cx: &mut Context<Self>) -> (AnyElement, usize) {
        let candidates = self.sort_candidates(cx);
        let sort = self.state.sort.clone();
        let (highlighted, scroll) = self
            .panel
            .as_ref()
            .map(|panel| (panel.nav.highlighted, panel.scroll.clone()))
            .unwrap_or_default();
        let enabled = self.enabled;

        let clear = self.button(
            "sort-clear",
            "Clear sorting",
            None,
            Action::SortClear,
            !sort.is_empty(),
            cx,
        );
        let header = div()
            .flex_none()
            .h(px(26.))
            .px(px(8.))
            .flex()
            .items_center()
            .child(popover::heading("Sort by"))
            .child(ui::grow())
            .child(clear);

        self.tab_order.push(self.search.editor.focus_handle(cx));
        let mut list = div()
            .id("sort-candidates")
            .role(Role::ListBox)
            .aria_label("Columns to add to the sort")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&scroll);
        let count = candidates.len();
        for (index, column) in candidates.into_iter().enumerate() {
            let name = column.name;
            let click = name.clone();
            let a11y = name.clone();
            let weak = cx.weak_entity();
            list = list.child(
                popover::item(
                    ("sort-candidate", index),
                    name,
                    None,
                    Some(column.cast_type.into()),
                    index == highlighted,
                    enabled,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.action(Action::SortAppend(click.clone()), window, cx)
                }))
                .on_a11y_action(
                    gpui::accesskit::Action::Click,
                    move |_, window, cx| {
                        weak.update(cx, |this, cx| {
                            this.action(Action::SortAppend(a11y.clone()), window, cx)
                        })
                        .ok();
                    },
                ),
            );
        }
        if count == 0 {
            list = list.child(empty(if self.columns.is_empty() {
                "Load a page to sort by column"
            } else {
                "No more columns"
            }));
        }
        let left = div()
            .flex_none()
            .w(px(180.))
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(style::line_soft())
            .child(
                div()
                    .flex_none()
                    .p(px(4.))
                    .child(ui::field().w_full().child(self.search.accessible.clone())),
            )
            .child(list);

        let mut keys = div()
            .id("sort-keys")
            .role(Role::List)
            .aria_label("Sort order")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .p(px(4.))
            .overflow_y_scroll();
        let last = sort.len().saturating_sub(1);
        for (index, key) in sort.iter().enumerate() {
            keys = keys.child(self.sort_key(index, key, index == last, cx));
        }
        if sort.is_empty() {
            keys = keys.child(empty("No sort. Pick a column on the left."));
        }

        let body = div()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .flex()
                    .h(px(PANE_HEIGHT))
                    .border_t_1()
                    .border_color(style::line_soft())
                    .child(left)
                    .child(keys),
            )
            .into_any_element();
        (body, count)
    }

    fn sort_key(
        &mut self,
        index: usize,
        key: &BrowseSortKey,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column = key.column.clone();
        let (direction, direction_label) = match key.direction {
            BrowseSortDirection::Asc => ("ASC", "ascending"),
            BrowseSortDirection::Desc => ("DESC", "descending"),
        };
        let (nulls, nulls_label) = match key.nulls {
            BrowseNulls::Default => ("NULLS –", "default NULL placement"),
            BrowseNulls::First => ("NULLS FIRST", "NULLS FIRST"),
            BrowseNulls::Last => ("NULLS LAST", "NULLS LAST"),
        };
        let toggle = self
            .button(
                format!("sort-direction-{index}"),
                direction,
                None,
                Action::SortDirection(index),
                true,
                cx,
            )
            .aria_label(format!("{column}: {direction_label}; activate to reverse"));
        let cycle = self
            .button(
                format!("sort-nulls-{index}"),
                nulls,
                None,
                Action::SortNulls(index),
                true,
                cx,
            )
            .aria_label(format!("{column}: {nulls_label}; activate to change"));
        let up = self.icon(
            format!("sort-up-{index}"),
            format!("Move {column} up"),
            "icons/arrow_up.svg",
            Action::SortUp(index),
            index > 0,
            cx,
        );
        let down = self.icon(
            format!("sort-down-{index}"),
            format!("Move {column} down"),
            "icons/arrow_down.svg",
            Action::SortDown(index),
            !last,
            cx,
        );
        let remove = self.icon(
            format!("sort-remove-{index}"),
            format!("Remove {column} from the sort"),
            "icons/close.svg",
            Action::SortRemove(index),
            true,
            cx,
        );
        div()
            .id(("sort-key", index))
            .role(Role::ListItem)
            .aria_label(format!(
                "Sort {}: {column} {direction_label}, {nulls_label}",
                index + 1
            ))
            .flex_none()
            .h(px(22.))
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .flex_none()
                    .w(px(14.))
                    .font_family(style::MONO)
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::faint())
                    .child((index + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(style::text())
                    .child(column),
            )
            .child(toggle)
            .child(cycle)
            .child(up)
            .child(down)
            .child(remove)
            .into_any_element()
    }
}

fn empty(text: &'static str) -> gpui::Div {
    div()
        .px(px(8.))
        .py(px(4.))
        .text_color(style::faint())
        .child(text)
}
