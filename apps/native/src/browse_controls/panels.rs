//! History, Presets and Inspect popovers. They replace the old cycle-through
//! rows; semantics and limits are unchanged (20 history entries, 8 KiB preset
//! names, 256 KiB inspection).
use super::{Action, BrowseControls, INSPECTION_BYTES, filter_summary, parameters};
use crate::{browse_preferences::HistoryEntry, results::encoded_size, style, ui, ui::popover};
use gpui::{AnyElement, Context, Role, SharedString, div, prelude::*, px};

/// Characters of one history or preset summary shown in a list row.
const SUMMARY_CHARS: usize = 200;

impl BrowseControls {
    pub(super) fn history_panel(&mut self, cx: &mut Context<Self>) -> (AnyElement, usize) {
        let entries = self
            .history
            .iter()
            .map(|entry| (history_summary(entry), entry.applied_at.clone()))
            .collect::<Vec<_>>();
        let count = entries.len();
        let mut list = self.panel_list("browse-history-list", "Filter history");
        for (index, (summary, applied_at)) in entries.into_iter().enumerate() {
            list = list.child(self.panel_item(
                ("browse-history", index),
                summary,
                Some(applied_at.into()),
                Action::ApplyHistory(index),
                cx,
            ));
        }
        if count == 0 {
            list = list.child(empty("No filter history yet"));
        }
        let body = div()
            .flex()
            .flex_col()
            .child(heading_row("Filter history"))
            .child(list)
            .into_any_element();
        (body, count)
    }

    pub(super) fn presets_panel(&mut self, cx: &mut Context<Self>) -> (AnyElement, usize) {
        let presets = self
            .presets
            .iter()
            .map(|preset| {
                (
                    truncate(&preset.name, 80),
                    format!(
                        "{} filters · {} sort",
                        preset.state.typed_filters.len()
                            + usize::from(!preset.state.raw_filter_text.trim().is_empty()),
                        preset.state.sort.len()
                    ),
                )
            })
            .collect::<Vec<_>>();
        let count = presets.len();
        let mut list = self.panel_list("browse-preset-list", "Presets");
        for (index, (name, detail)) in presets.into_iter().enumerate() {
            list = list.child(self.panel_item(
                ("browse-preset", index),
                name,
                Some(detail.into()),
                Action::ApplyPreset(index),
                cx,
            ));
        }
        if count == 0 {
            list = list.child(empty("No presets yet"));
        }
        self.tab_order.push(self.name.editor.focus_handle(cx));
        let save = self.button(
            "browse-save-preset",
            "Save preset",
            Some("icons/bookmark.svg"),
            Action::SavePreset,
            true,
            cx,
        );
        let body = div()
            .flex()
            .flex_col()
            .child(heading_row("Presets"))
            .child(list)
            .child(popover::divider())
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        ui::field()
                            .flex_1()
                            .min_w_0()
                            .child(self.name.accessible.clone()),
                    )
                    .child(save),
            )
            .into_any_element();
        (body, count)
    }

    pub(super) fn inspect_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(8.))
            .pb(px(4.))
            .child(heading_row("Executed query").px(px(0.)));
        let Some(page) = self.page.clone() else {
            return body
                .child(empty("No page is loaded yet").px(px(0.)))
                .into_any_element();
        };
        if encoded_size(&page.inspection) > INSPECTION_BYTES {
            return body
                .child(empty("Query inspection exceeds the 256 KiB display budget").px(px(0.)))
                .into_any_element();
        }
        let params = parameters(&page.inspection.params);
        let copy_sql = self.button(
            "browse-copy-sql",
            "Copy SQL",
            Some("icons/copy.svg"),
            Action::CopySql,
            true,
            cx,
        );
        let copy_params = self.button(
            "browse-copy-params",
            "Copy parameters",
            Some("icons/copy.svg"),
            Action::CopyParams,
            !page.inspection.params.is_empty(),
            cx,
        );
        body = body
            .child(div().flex().gap(px(4.)).child(copy_sql).child(copy_params))
            .child(
                div()
                    .id("browse-inspection")
                    .role(Role::Label)
                    .aria_label(format!(
                        "Executed SQL: {}\nParameters: {}",
                        page.inspection.sql, params
                    ))
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .p(px(6.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(style::line_soft())
                    .bg(style::bg())
                    .font_family(style::MONO)
                    .text_color(style::dim())
                    .child(format!("{}\n{}", page.inspection.sql, params)),
            )
            .when(page.omitted_rows > 0 || page.truncated_cells > 0, |node| {
                node.child(div().text_color(style::faint()).child(format!(
                    "Partial result: {} omitted rows, {} truncated cells",
                    page.omitted_rows, page.truncated_cells
                )))
            });
        body.into_any_element()
    }

    fn panel_list(&self, id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
        let scroll = self
            .panel
            .as_ref()
            .map(|panel| panel.scroll.clone())
            .unwrap_or_default();
        div()
            .id(id)
            .role(Role::ListBox)
            .aria_label(label)
            .flex()
            .flex_col()
            .max_h(px(240.))
            .overflow_y_scroll()
            .track_scroll(&scroll)
    }

    /// One list row: click applies; keyboard reaches it through the panel's
    /// `MenuNav` highlight.
    fn panel_item(
        &mut self,
        id: (&'static str, usize),
        label: String,
        hint: Option<SharedString>,
        action: Action,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (_, index) = id;
        let highlighted = self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.nav.highlighted == index);
        let enabled = self.available(&action, true);
        let weak = cx.weak_entity();
        let click = action.clone();
        popover::item(id, label, None, hint, highlighted, enabled)
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.action(click.clone(), window, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    weak.update(cx, |this, cx| this.action(action.clone(), window, cx))
                        .ok();
                }
            })
            .into_any_element()
    }
}

/// `applied_at · filters AND raw · N sort`, as the old selected-history row.
fn history_summary(entry: &HistoryEntry) -> String {
    let filters = entry
        .typed_filters
        .iter()
        .map(filter_summary)
        .chain(
            (!entry.raw_filter_text.is_empty())
                .then(|| entry.raw_filter_text.chars().take(160).collect::<String>()),
        )
        .collect::<Vec<_>>();
    let filters = if filters.is_empty() {
        "no filters".to_owned()
    } else {
        filters.join(" AND ")
    };
    truncate(
        &format!("{filters} · {} sort", entry.sort.len()),
        SUMMARY_CHARS,
    )
}

fn truncate(text: &str, chars: usize) -> String {
    if text.chars().count() > chars {
        format!("{}…", text.chars().take(chars).collect::<String>())
    } else {
        text.to_owned()
    }
}

fn heading_row(label: &'static str) -> gpui::Div {
    div()
        .flex_none()
        .h(px(26.))
        .px(px(8.))
        .flex()
        .items_center()
        .child(popover::heading(label))
}

fn empty(text: &'static str) -> gpui::Div {
    div()
        .px(px(8.))
        .py(px(4.))
        .text_color(style::faint())
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse_preferences::FilterMode;
    use dbunk_lib::backend::data::*;

    #[test]
    fn history_summary_joins_filters_and_bounds_its_length() {
        let entry = HistoryEntry {
            applied_at: "now".into(),
            typed_filters: vec![BrowseFilter::Comparison {
                column: "a".into(),
                operator: ComparisonOperator::Eq,
                value: "y".repeat(300),
            }],
            raw_filter_text: "x".repeat(400),
            filter_mode: FilterMode::Raw,
            sort: vec![],
        };
        let summary = history_summary(&entry);
        assert!(summary.starts_with("a Eq yyy"));
        assert!(summary.ends_with('…'));
        assert_eq!(summary.chars().count(), SUMMARY_CHARS + 1);
        let blank = HistoryEntry {
            typed_filters: vec![],
            raw_filter_text: String::new(),
            ..entry
        };
        assert_eq!(history_summary(&blank), "no filters · 0 sort");
        assert_eq!(truncate("東京", 1), "東…");
    }
}
