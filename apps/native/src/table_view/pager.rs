//! Plan 032 §1: the toolbar pager (`‹ 1–100 of ~18,204 ›`) and its popover
//! (Limit, Page jump, First, Last). The browse contract is page based, so the
//! popover offers a page size and a 1-based page instead of Limit/Offset.
#[cfg(test)]
use super::menus::LegacyAction;
use super::{Action, TableView, menus::Popover};
use crate::{accessible_editor::AccessibleEditor, browse_preferences::PAGE_SIZES};
use editor::Editor;
use gpui::{AnyElement, Context, Focusable, Role, SharedString, Window, div, prelude::*, px};

/// What the pager popover offers, for the reachability test.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PagerItem {
    Limit,
    Jump,
    First,
    Last,
}

#[cfg(test)]
pub(super) const PAGER_ITEMS: [PagerItem; 4] = [
    PagerItem::Limit,
    PagerItem::Jump,
    PagerItem::First,
    PagerItem::Last,
];

#[cfg(test)]
impl PagerItem {
    pub(super) fn covers(self) -> &'static [LegacyAction] {
        match self {
            Self::Limit => &[LegacyAction::PageSize],
            Self::First => &[LegacyAction::FirstPage],
            Self::Last => &[LegacyAction::LastPage],
            Self::Jump => &[],
        }
    }
}

/// Thousands separators: `18204` → `18,204`.
pub(super) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The clickable range: `1–100`, or `No rows`.
pub(super) fn range_label(page: u32, page_size: u32, rows: usize) -> String {
    if rows == 0 {
        return "No rows".to_owned();
    }
    let start = u64::from(page.max(1) - 1) * u64::from(page_size) + 1;
    format!("{}–{}", grouped(start), grouped(start + rows as u64 - 1))
}

/// The clickable total: `~18,204` when estimated, `18,204` when exact.
pub(super) fn total_label(total: Option<(u64, bool)>) -> Option<String> {
    total
        .map(|(value, estimated)| format!("{}{}", if estimated { "~" } else { "" }, grouped(value)))
}

/// `1–100 of ~18,204`; the toolbar renders the two halves as separate
/// triggers and uses this whole string as the AX summary.
pub(super) fn page_range_label(
    page: u32,
    page_size: u32,
    rows: usize,
    total: Option<(u64, bool)>,
) -> String {
    let range = range_label(page, page_size, rows);
    match total_label(total) {
        Some(total) if rows > 0 => format!("{range} of {total}"),
        _ => range,
    }
}

/// The last page when the total is exact; estimates never bound a jump.
pub(super) fn last_page(total: Option<(u64, bool)>, page_size: u32) -> Option<u32> {
    let (value, estimated) = total?;
    if estimated || page_size == 0 {
        return None;
    }
    let pages = value.div_ceil(u64::from(page_size)).max(1);
    Some(u32::try_from(pages).unwrap_or(u32::MAX))
}

/// Validates the Page field: a whole number ≥ 1, and ≤ the last page when
/// the total is exact.
pub(super) fn parse_jump(text: &str, last: Option<u32>) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a page number".to_owned());
    }
    let page = text
        .parse::<u32>()
        .map_err(|_| "Page must be a whole number".to_owned())?;
    if page == 0 {
        return Err("Pages start at 1".to_owned());
    }
    match last {
        Some(last) if page > last => Err(format!("The last page is {}", grouped(last.into()))),
        _ => Ok(page),
    }
}

/// `(page, page_size, rows, total)`; the total is `(count, exact)`.
type PageNumbers = (u32, u32, usize, Option<(u64, bool)>);

impl TableView {
    /// `(page, page_size, rows, total)` for the current result, if any.
    pub(super) fn page_numbers(&self) -> Option<PageNumbers> {
        let model = self.model.as_ref()?;
        let result = model.result()?;
        let total = model
            .exact_count()
            .map(|count| (count.value, false))
            .or_else(|| match result.count.kind {
                dbunk_lib::backend::data::BrowseCountKind::Unknown => None,
                kind => result.count.value.map(|value| {
                    (
                        value,
                        kind == dbunk_lib::backend::data::BrowseCountKind::Estimated,
                    )
                }),
            });
        Some((
            model.page(),
            model.query().page_size,
            result.rows.len(),
            total,
        ))
    }

    pub(super) fn toggle_pager(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.popover, Some(Popover::Pager { .. })) {
            self.close_popover(cx);
            return;
        }
        let Some(anchor) = self.anchors.pager.get() else {
            return;
        };
        let page = cx.new(|cx| Editor::single_line(window, cx));
        let field = cx.new(|cx| AccessibleEditor::field(page.clone(), "Go to page", false, cx));
        window.focus(&page.focus_handle(cx), cx);
        self.popover = Some(Popover::Pager {
            anchor,
            page,
            field,
            error: None,
        });
        cx.notify();
    }

    /// Validates the Page field and jumps; a refusal stays in the popover.
    pub(super) fn submit_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Popover::Pager { page, .. }) = &self.popover else {
            return;
        };
        let text = page.read(cx).text(cx);
        let last = self
            .page_numbers()
            .and_then(|(_, size, _, total)| last_page(total, size));
        match parse_jump(&text, last) {
            Ok(page) => {
                self.close_popover(cx);
                self.activate(Action::Jump(page), window, cx);
            }
            Err(message) => {
                if let Some(Popover::Pager { error, .. }) = &mut self.popover {
                    *error = Some(message.into());
                }
                cx.notify();
            }
        }
    }

    fn pager_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        self.close_popover(cx);
        self.activate(action, window, cx);
    }

    pub(super) fn render_pager_popover(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let Some(Popover::Pager {
            anchor,
            field,
            error,
            ..
        }) = &self.popover
        else {
            return None;
        };
        let (anchor, field, error) = (*anchor, field.clone(), error.clone());
        let enabled = self.can_browse(cx);
        let numbers = self.page_numbers();
        let current_size = self
            .model
            .as_ref()
            .map_or(self.state.page_size, |model| model.query().page_size);
        let page = numbers.map_or(1, |(page, ..)| page);
        let last = numbers.and_then(|(_, size, _, total)| last_page(total, size));
        let mut sizes = div().flex().flex_wrap().gap(px(2.)).px(px(6.));
        for size in PAGE_SIZES {
            let weak = cx.weak_entity();
            let selected = size == current_size;
            sizes = sizes.child(
                crate::ui::pressed(
                    crate::ui::tool_button(
                        ("pager-size", size as usize),
                        size.to_string(),
                        None,
                        enabled,
                        false,
                    ),
                    selected,
                )
                .aria_label(SharedString::from(format!("Limit {size} rows per page")))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if enabled && !selected {
                        this.pager_action(Action::PageSize(size), window, cx);
                    }
                }))
                .on_a11y_action(
                    gpui::accesskit::Action::Click,
                    move |_, window, cx| {
                        if enabled && !selected {
                            let _ = weak.update(cx, |this, cx| {
                                this.pager_action(Action::PageSize(size), window, cx)
                            });
                        }
                    },
                ),
            );
        }
        let go_weak = cx.weak_entity();
        let jump = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .px(px(6.))
            .child(
                crate::ui::field()
                    .flex_1()
                    .child(div().flex_1().child(field)),
            )
            .child(
                crate::ui::tool_button("pager-go", "Go", None, enabled, true)
                    .aria_label("Go to page")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if enabled {
                            this.submit_jump(window, cx);
                        }
                    }))
                    .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                        if enabled {
                            let _ = go_weak.update(cx, |this, cx| this.submit_jump(window, cx));
                        }
                    }),
            );
        let hint = match last {
            Some(last) => format!("Page {page} of {}", grouped(last.into())),
            None => format!("Page {page}"),
        };
        let mut panel = crate::ui::popover::panel("pager-popover", Role::Dialog, "Pagination")
            .w(px(220.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .on_mouse_down_out(self.dismiss_outside(anchor, cx))
            .child(crate::ui::popover::heading("Limit"))
            .child(sizes)
            .child(crate::ui::popover::heading(hint))
            .child(jump)
            .children(error.map(|message| {
                div()
                    .id("pager-error")
                    .role(Role::Alert)
                    .aria_label(message.clone())
                    .px(px(8.))
                    .text_size(px(crate::style::FONT_SMALL))
                    .text_color(crate::style::bad_text())
                    .child(message)
            }))
            .child(crate::ui::popover::divider());
        let previous = numbers.is_some_and(|(page, ..)| page > 1);
        let at_end = numbers.is_some_and(|(page, ..)| last.is_some_and(|last| page >= last));
        for (index, (label, icon, action, item_enabled)) in [
            (
                "First page",
                "icons/chevron_left.svg",
                Action::First,
                enabled && previous,
            ),
            (
                "Last page",
                "icons/chevron_right.svg",
                Action::Last,
                enabled && !at_end,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let weak = cx.weak_entity();
            panel = panel.child(
                crate::ui::popover::item(
                    ("pager-item", index),
                    label,
                    Some(icon),
                    None,
                    false,
                    item_enabled,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if item_enabled {
                        this.pager_action(action, window, cx);
                    }
                }))
                .on_a11y_action(
                    gpui::accesskit::Action::Click,
                    move |_, window, cx| {
                        if item_enabled {
                            let _ =
                                weak.update(cx, |this, cx| this.pager_action(action, window, cx));
                        }
                    },
                ),
            );
        }
        Some(
            crate::ui::popover::layer(anchor, crate::ui::popover::Placement::BelowEnd, panel)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_range_label_marks_estimates_and_handles_empty_pages() {
        assert_eq!(
            page_range_label(1, 100, 100, Some((18_204, true))),
            "1–100 of ~18,204"
        );
        assert_eq!(
            page_range_label(3, 100, 7, Some((207, false))),
            "201–207 of 207"
        );
        assert_eq!(page_range_label(2, 50, 50, None), "51–100");
        assert_eq!(page_range_label(1, 100, 0, None), "No rows");
        assert_eq!(page_range_label(1, 100, 0, Some((0, false))), "No rows");
        assert_eq!(page_range_label(201, 1000, 1000, None), "200,001–201,000");
    }

    #[test]
    fn grouped_inserts_thousands_separators() {
        assert_eq!(grouped(1_000_000), "1,000,000");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(0), "0");
    }

    #[test]
    fn last_page_needs_an_exact_total() {
        assert_eq!(last_page(Some((207, false)), 100), Some(3));
        assert_eq!(last_page(Some((200, false)), 100), Some(2));
        assert_eq!(last_page(Some((0, false)), 100), Some(1));
        assert_eq!(last_page(Some((207, true)), 100), None);
        assert_eq!(last_page(None, 100), None);
    }

    #[test]
    fn jump_validation() {
        assert_eq!(parse_jump(" 3 ", Some(3)), Ok(3));
        assert_eq!(parse_jump("1", None), Ok(1));
        assert_eq!(parse_jump("9999", None), Ok(9999), "estimates never bound");
        assert!(parse_jump("4", Some(3)).is_err());
        assert!(parse_jump("0", None).is_err());
        assert!(parse_jump("", None).is_err());
        assert!(parse_jump("-1", None).is_err());
        assert!(parse_jump("2.5", None).is_err());
        assert!(parse_jump("abc", None).is_err());
        assert!(parse_jump("99999999999", None).is_err());
    }
}
