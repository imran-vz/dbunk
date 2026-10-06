//! Plan 032 §1: the single 28 px table toolbar and the notice strip.
//! `toolbar_mode` decides what shows; rendering only lays it out with the
//! existing kit and routes clicks to the same handlers as the menus.
#[cfg(test)]
use super::menus::LegacyAction;
use super::{
    Action, TableView,
    columns_popover::columns_badge,
    menus::Popover,
    pager::{range_label, total_label},
};
use crate::{
    browse_controls::BrowsePanel,
    data_model::{PageAction, TablePolicy},
    style,
    table_changes::{ChangesCommand, ChangesNotice, ChangesSummary},
    ui,
};
use gpui::{
    AnyElement, Context, Div, Focusable, Role, SharedString, Stateful, Window, div, prelude::*, px,
    svg,
};
use std::rc::Rc;

/// The toolbar's controls in render order, for the reachability test.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ToolbarItem {
    Data,
    Structure,
    Filter,
    Sort,
    Columns,
    AddRow,
    DeleteChecked,
    ChangeList,
    Discard,
    Review,
    Previous,
    Range,
    Total,
    Next,
    Refresh,
    Cancel,
    Connect,
    Overflow,
}

#[cfg(test)]
pub(super) const TOOLBAR_ITEMS: [ToolbarItem; 18] = [
    ToolbarItem::Data,
    ToolbarItem::Structure,
    ToolbarItem::Filter,
    ToolbarItem::Sort,
    ToolbarItem::Columns,
    ToolbarItem::AddRow,
    ToolbarItem::DeleteChecked,
    ToolbarItem::ChangeList,
    ToolbarItem::Discard,
    ToolbarItem::Review,
    ToolbarItem::Previous,
    ToolbarItem::Range,
    ToolbarItem::Total,
    ToolbarItem::Next,
    ToolbarItem::Refresh,
    ToolbarItem::Cancel,
    ToolbarItem::Connect,
    ToolbarItem::Overflow,
];

#[cfg(test)]
impl ToolbarItem {
    pub(super) fn covers(self) -> &'static [LegacyAction] {
        match self {
            Self::Structure => &[LegacyAction::Structure],
            Self::AddRow => &[LegacyAction::InsertRow],
            Self::DeleteChecked => &[LegacyAction::DeleteRow],
            Self::Previous => &[LegacyAction::PreviousPage],
            Self::Next => &[LegacyAction::NextPage],
            Self::Total => &[LegacyAction::Count],
            Self::Refresh => &[LegacyAction::Refresh],
            Self::Cancel => &[LegacyAction::Cancel],
            Self::Connect => &[LegacyAction::Connect],
            Self::Data
            | Self::Filter
            | Self::Sort
            | Self::Columns
            | Self::ChangeList
            | Self::Discard
            | Self::Review
            | Self::Range
            | Self::Overflow => &[],
        }
    }
}

/// The staged-changes group: `N changes ▾`, Discard and `Review N ⌘S`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct StagedGroup {
    pub changes: usize,
    pub review: usize,
    pub can_review: Result<(), SharedString>,
    pub can_discard: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ToolbarMode {
    /// Disconnected: Connect replaces the pager and Refresh.
    pub connect: bool,
    /// Busy or an apply is in flight: Cancel replaces Refresh.
    pub cancel: bool,
    /// None hides `+ Row` (read-only); Err disables it with the reason.
    pub add_row: Option<Result<(), SharedString>>,
    /// Shown only while rows are checked.
    pub delete: Option<(usize, Result<(), SharedString>)>,
    pub staged: Option<StagedGroup>,
    /// The Read-only badge and its tooltip.
    pub read_only: Option<SharedString>,
}

const LOADING: &str = "Wait for the table to finish loading";
const CONNECT: &str = "Connect this table first";

pub(super) fn toolbar_mode(
    summary: &ChangesSummary,
    checked: usize,
    policy: TablePolicy,
    connected: bool,
    busy: bool,
    can_edit: Result<(), SharedString>,
) -> ToolbarMode {
    let read_only = policy.read_only_reason().map(SharedString::from);
    let stage = if !connected {
        Err(SharedString::from(CONNECT))
    } else if busy {
        Err(SharedString::from(LOADING))
    } else {
        can_edit
    };
    ToolbarMode {
        connect: !connected,
        cancel: busy || summary.pending,
        add_row: read_only.is_none().then(|| stage.clone()),
        delete: (checked > 0).then(|| {
            (
                checked,
                match &read_only {
                    Some(reason) => Err(reason.clone()),
                    None => stage.clone(),
                },
            )
        }),
        staged: (summary.staged > 0).then(|| StagedGroup {
            changes: summary.staged,
            review: summary.included,
            can_review: if busy {
                Err(SharedString::from(LOADING))
            } else {
                summary.can_review.clone()
            },
            can_discard: !summary.pending,
        }),
        read_only,
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

impl TableView {
    /// Gives a toolbar control a stable focus handle, a tab stop and both
    /// pointer and accessibility activation.
    pub(super) fn control(
        &mut self,
        key: &'static str,
        element: Stateful<Div>,
        enabled: bool,
        cx: &Context<Self>,
        run: impl Fn(&mut TableView, &mut Window, &mut Context<TableView>) + 'static,
    ) -> Stateful<Div> {
        let focus = self
            .buttons
            .entry(key.to_owned())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        if enabled {
            self.tab_order.push(focus.clone());
        }
        let run = Rc::new(run);
        let click = run.clone();
        let weak = cx.weak_entity();
        element
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    (*click)(this, window, cx);
                    this.sync_grid(cx);
                    cx.notify();
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    let _ = weak.update(cx, |this, cx| {
                        (*run)(this, window, cx);
                        this.sync_grid(cx);
                        cx.notify();
                    });
                }
            })
    }

    pub(super) fn current_mode(&self, summary: &ChangesSummary, cx: &gpui::App) -> ToolbarMode {
        let changes = self.changes.read(cx);
        toolbar_mode(
            summary,
            self.grid.read(cx).checked_rows().len(),
            changes.policy(),
            self.editable && self.controls.is_some(),
            self.busy,
            changes.can_edit_now(),
        )
    }

    pub(super) fn add_row(&mut self, cx: &mut Context<Self>) {
        match self.changes.update(cx, |changes, cx| changes.add_row(cx)) {
            Ok(_) => self.status = "New row staged; double-click a cell to set its value".into(),
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn delete_checked(&mut self, cx: &mut Context<Self>) {
        let rows = self.grid.read(cx).checked_rows();
        if rows.is_empty() {
            return;
        }
        match self
            .changes
            .update(cx, |changes, cx| changes.stage_deletes(&rows, cx))
        {
            Ok(count) => {
                self.grid.update(cx, |grid, cx| grid.clear_checked(cx));
                self.status = format!("{} staged for review", plural(count, "delete", "deletes"));
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    pub(super) fn changes_command(
        &mut self,
        command: ChangesCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_popover(cx);
        self.changes
            .update(cx, |changes, cx| changes.command(command, window, cx));
    }

    pub(super) fn render_toolbar(
        &mut self,
        summary: &ChangesSummary,
        cx: &mut Context<Self>,
    ) -> Div {
        let mode = self.current_mode(summary, cx);
        let navigation_blocked = self.changes.read(cx).navigation_blocked();
        let browse = self.can_browse(cx);
        let tools = self.editable && self.connection.is_some() && !self.busy && !navigation_blocked;
        let (bar_open, filters, sorts) = {
            let controls = self.browse_controls.read(cx);
            (
                controls.bar_open(),
                controls.active_filter_count(),
                controls.sort_count(),
            )
        };
        let entries = self.grid.read(cx).column_entries();
        let columns_open = matches!(self.popover, Some(Popover::Columns { .. }));
        let overflow_open = matches!(self.popover, Some(Popover::Overflow { .. }));
        let pager_open = matches!(self.popover, Some(Popover::Pager { .. }));

        let mut bar = ui::toolbar_strip();

        // Data | Structure
        let structure = self.control(
            "table-structure",
            ui::segment("table-structure", "Structure", false, tools),
            tools,
            cx,
            |this, window, cx| this.activate(Action::Structure, window, cx),
        );
        bar = bar
            .child(
                ui::segment_group()
                    .child(ui::segment("table-data", "Data", true, true).aria_selected(true))
                    .child(structure),
            )
            .child(ui::separator());

        // Filter · Sort · Columns
        let filter_enabled = browse || bar_open;
        let filter = self.control(
            "table-filter",
            ui::pressed(
                ui::tool_button(
                    "table-filter",
                    "Filter",
                    Some("icons/filter.svg"),
                    filter_enabled,
                    false,
                ),
                bar_open,
            )
            .aria_expanded(bar_open)
            .when(filters > 0, |button| {
                button.child(ui::count_badge(filters.to_string()))
            }),
            filter_enabled,
            cx,
            move |this, window, cx| {
                this.browse_controls.update(cx, |controls, cx| {
                    controls.set_bar_open(!bar_open, window, cx)
                })
            },
        );
        let sort_anchor = self.anchors.sort.clone();
        let sort = self.control(
            "table-sort",
            ui::tool_button(
                "table-sort",
                "Sort",
                Some("icons/chevron_up_down.svg"),
                browse,
                false,
            )
            .relative()
            .when(sorts > 0, |button| {
                button.child(ui::count_badge(sorts.to_string()))
            })
            .child(ui::popover::probe(sort_anchor.clone())),
            browse,
            cx,
            move |this, window, cx| {
                if let Some(anchor) = sort_anchor.get() {
                    this.close_popover(cx);
                    this.browse_controls.update(cx, |controls, cx| {
                        controls.open_panel(BrowsePanel::Sort, anchor, window, cx)
                    });
                }
            },
        );
        let columns_enabled = self.editable && self.controls.is_some();
        let columns = self.control(
            "table-columns",
            ui::pressed(
                ui::tool_button(
                    "table-columns",
                    "Columns",
                    Some("icons/eye.svg"),
                    columns_enabled,
                    false,
                ),
                columns_open,
            )
            .relative()
            .aria_expanded(columns_open)
            .when_some(columns_badge(&entries), |button, badge| {
                button.child(ui::count_badge(badge))
            })
            .child(ui::popover::probe(self.anchors.columns.clone())),
            columns_enabled,
            cx,
            |this, window, cx| this.toggle_columns(window, cx),
        );
        bar = bar
            .child(filter)
            .child(sort)
            .child(columns)
            .child(ui::separator());

        // + Row · Delete N rows
        if let Some(add) = mode.add_row.clone() {
            let enabled = add.is_ok();
            let button = ui::tool_button(
                "table-add-row",
                "Row",
                Some("icons/plus.svg"),
                enabled,
                false,
            )
            .aria_label("Add row");
            let button = match add {
                Err(reason) => button
                    .tooltip(ui::tooltip(reason))
                    .tooltip_show_delay(ui::tooltip_delay()),
                Ok(()) => button,
            };
            bar = bar.child(
                self.control("table-add-row", button, enabled, cx, |this, _, cx| {
                    this.add_row(cx)
                }),
            );
        }
        if let Some((count, delete)) = mode.delete.clone() {
            let enabled = delete.is_ok();
            let label = format!("Delete {}", plural(count, "row", "rows"));
            let button = ui::tool_button(
                "table-delete-checked",
                label,
                Some("icons/trash.svg"),
                enabled,
                false,
            )
            .when(enabled, |button| button.text_color(style::bad_text()));
            let button = match delete {
                Err(reason) => button
                    .tooltip(ui::tooltip(reason))
                    .tooltip_show_delay(ui::tooltip_delay()),
                Ok(()) => button,
            };
            bar = bar.child(self.control(
                "table-delete-checked",
                button,
                enabled,
                cx,
                |this, window, cx| {
                    this.delete_checked(cx);
                    // The button disappears with the checked rows; keep focus
                    // in the table so ⌘S and the grid keys still work.
                    window.focus(&this.grid.focus_handle(cx), cx);
                },
            ));
        }
        bar = bar.child(ui::grow());

        // N changes ▾ · Discard · Review N ⌘S
        if let Some(staged) = mode.staged.clone() {
            let list_anchor = self.anchors.changes.clone();
            let list = self.control(
                "table-change-list",
                ui::tool_button(
                    "table-change-list",
                    plural(staged.changes, "change", "changes"),
                    None,
                    true,
                    false,
                )
                .relative()
                .text_color(style::warn())
                .child(
                    svg()
                        .path("icons/chevron_down.svg")
                        .size(px(style::ICON))
                        .flex_none()
                        .text_color(style::warn()),
                )
                .child(ui::popover::probe(list_anchor.clone())),
                true,
                cx,
                move |this, window, cx| {
                    if let Some(anchor) = list_anchor.get() {
                        this.changes_command(ChangesCommand::OpenChangeList(anchor), window, cx);
                    }
                },
            );
            let discard = self.control(
                "table-discard",
                ui::tool_button("table-discard", "Discard", None, staged.can_discard, false)
                    .aria_label("Discard staged changes"),
                staged.can_discard,
                cx,
                |this, window, cx| this.changes_command(ChangesCommand::Discard, window, cx),
            );
            let review_enabled = staged.can_review.is_ok();
            let review = self.control(
                "table-review",
                ui::tool_button_accent(
                    "table-review",
                    format!("Review {}", staged.review),
                    Some("⌘S"),
                    review_enabled,
                )
                .aria_keyshortcuts("Meta+S"),
                review_enabled,
                cx,
                |this, window, cx| this.changes_command(ChangesCommand::Review, window, cx),
            );
            // The accent button may carry its own tooltip; the refusal reason
            // goes on a wrapper so a second tooltip is never installed.
            let review = match staged.can_review {
                Err(reason) => div()
                    .id("table-review-reason")
                    .flex_none()
                    .tooltip(ui::tooltip(reason))
                    .tooltip_show_delay(ui::tooltip_delay())
                    .child(review)
                    .into_any_element(),
                Ok(()) => review.into_any_element(),
            };
            bar = bar.child(list).child(discard).child(review);
        }

        if let Some(reason) = mode.read_only.clone() {
            bar = bar.child(
                ui::badge("Read-only")
                    .id("table-read-only")
                    .role(Role::Label)
                    .aria_label(SharedString::from(format!("Read-only. {reason}")))
                    .tooltip(ui::tooltip(reason))
                    .tooltip_show_delay(ui::tooltip_delay()),
            );
        }

        let numbers = self.page_numbers();
        if let Some(model) = &self.model
            && let Some(result) = model.result()
        {
            bar = bar.child(
                div()
                    .flex_none()
                    .px(px(4.))
                    .font_family(style::MONO)
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::faint())
                    .child(format!("{} ms", result.runtime_ms)),
            );
        }

        // Pager, or Connect while disconnected.
        if mode.connect {
            let enabled = self.editable && self.controls.is_none();
            bar = bar.child(self.control(
                "table-connect",
                ui::tool_button(
                    "table-connect",
                    "Connect",
                    Some("icons/power.svg"),
                    enabled,
                    false,
                ),
                enabled,
                cx,
                |this, window, cx| this.activate(Action::Connect, window, cx),
            ));
        } else {
            let has_more = self
                .model
                .as_ref()
                .and_then(|model| model.result())
                .is_some_and(|result| result.page_info.has_more);
            let previous_enabled = browse && numbers.is_some_and(|(page, ..)| page > 1);
            let next_enabled = browse && has_more;
            let range = numbers.map_or_else(
                || "–".to_owned(),
                |(page, size, rows, _)| range_label(page, size, rows),
            );
            let summary_label = numbers.map_or_else(
                || "No page loaded".to_owned(),
                |(page, size, rows, total)| super::pager::page_range_label(page, size, rows, total),
            );
            let total = numbers.and_then(|(.., total)| total_label(total));
            let estimated = numbers.is_some_and(|(.., total)| total.is_some_and(|(_, e)| e));
            let previous = self.control(
                "table-previous",
                ui::icon_button(
                    "table-previous",
                    "Previous page",
                    "icons/chevron_left.svg",
                    previous_enabled,
                ),
                previous_enabled,
                cx,
                |this, window, cx| this.activate(Action::Previous, window, cx),
            );
            let range_enabled = browse;
            let range = self.control(
                "table-range",
                ui::pressed(
                    ui::tool_button("table-range", range, None, range_enabled, false),
                    pager_open,
                )
                .relative()
                .font_family(style::MONO)
                .aria_label(SharedString::from(format!(
                    "{summary_label}. Change page or limit"
                )))
                .aria_expanded(pager_open)
                .child(ui::popover::probe(self.anchors.pager.clone())),
                range_enabled,
                cx,
                |this, window, cx| this.toggle_pager(window, cx),
            );
            let count_enabled =
                self.editable && !self.busy && !navigation_blocked && self.controls.is_some();
            let total = total.map(|total| {
                let tip = if estimated {
                    "Estimated; click to count exactly"
                } else {
                    "Exact count; click to count again"
                };
                self.control(
                    "table-total",
                    ui::tool_button("table-total", total, None, count_enabled, false)
                        .font_family(style::MONO)
                        .aria_label(SharedString::from(tip))
                        .tooltip(ui::tooltip(tip))
                        .tooltip_show_delay(ui::tooltip_delay()),
                    count_enabled,
                    cx,
                    |this, window, cx| this.activate(Action::Count, window, cx),
                )
            });
            let next = self.control(
                "table-next",
                ui::icon_button(
                    "table-next",
                    "Next page",
                    "icons/chevron_right.svg",
                    next_enabled,
                ),
                next_enabled,
                cx,
                |this, window, cx| this.activate(Action::Next, window, cx),
            );
            bar = bar.child(previous).child(range);
            if let Some(total) = total {
                bar = bar
                    .child(div().flex_none().text_color(style::faint()).child("of"))
                    .child(total);
            }
            bar = bar.child(next);
        }

        // Refresh / Cancel
        if mode.cancel {
            bar = bar.child(self.control(
                "table-cancel",
                ui::tool_button(
                    "table-cancel",
                    "Cancel",
                    Some("icons/stop.svg"),
                    self.editable,
                    false,
                ),
                self.editable,
                cx,
                |this, window, cx| this.activate(Action::Cancel, window, cx),
            ));
        } else if !mode.connect {
            let enabled = self.editable && !navigation_blocked && self.controls.is_some();
            bar = bar.child(self.control(
                "table-refresh",
                ui::icon_button("table-refresh", "Refresh", "icons/rotate_cw.svg", enabled),
                enabled,
                cx,
                |this, window, cx| this.activate(Action::Refresh, window, cx),
            ));
        }

        // ⋯
        let overflow = self.control(
            "table-overflow",
            ui::pressed(
                ui::icon_button(
                    "table-overflow",
                    "More table actions",
                    "icons/ellipsis.svg",
                    true,
                ),
                overflow_open,
            )
            .relative()
            .aria_expanded(overflow_open)
            .child(ui::popover::probe(self.anchors.overflow.clone())),
            true,
            cx,
            |this, _, cx| this.toggle_overflow(cx),
        );
        bar.child(overflow)
    }

    /// The 24 px notice strip under the toolbar: staged-change notices first,
    /// then column-preference status.
    pub(super) fn render_notices(
        &mut self,
        summary: &ChangesSummary,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut notices = Vec::new();
        if let Some(notice) = &summary.notice {
            let refresh_enabled = self.editable && !self.busy && self.controls.is_some();
            let (message, warn): (SharedString, bool) = match notice {
                ChangesNotice::OutcomeUnknown => {
                    ("Outcome unknown: refresh, then Mark resolved".into(), true)
                }
                ChangesNotice::Unrestored => (
                    "Staged changes could not be restored; retry or discard them".into(),
                    true,
                ),
                ChangesNotice::ReadOnlyWithStaged => (
                    "Read-only connection: staged changes cannot be applied".into(),
                    true,
                ),
                ChangesNotice::Unavailable(message) => (message.clone(), false),
            };
            let mut strip = div()
                .id("table-notice")
                .role(Role::Status)
                .aria_label(message.clone())
                .flex_none()
                .h(px(style::FOOTER))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .overflow_hidden()
                .border_b_1()
                .border_color(style::line_soft())
                .when(warn, |strip| strip.bg(style::warn_fill()))
                .text_sm()
                .text_color(if warn { style::warn() } else { style::dim() })
                .when(warn, |strip| {
                    strip.child(
                        svg()
                            .path("icons/warning.svg")
                            .size(px(style::ICON))
                            .flex_none()
                            .text_color(style::warn()),
                    )
                })
                .child(div().flex_1().min_w_0().truncate().child(message));
            match notice {
                ChangesNotice::OutcomeUnknown => {
                    strip = strip
                        .child(self.control(
                            "notice-refresh",
                            ui::tool_button(
                                "notice-refresh",
                                "Refresh",
                                None,
                                refresh_enabled,
                                true,
                            ),
                            refresh_enabled,
                            cx,
                            |this, _, cx| this.browse(PageAction::First, true, cx),
                        ))
                        .child(self.control(
                            "notice-resolve",
                            ui::tool_button("notice-resolve", "Mark resolved", None, true, false),
                            true,
                            cx,
                            |this, window, cx| {
                                this.changes_command(ChangesCommand::Reconcile, window, cx)
                            },
                        ));
                }
                ChangesNotice::Unrestored => {
                    strip = strip
                        .child(self.control(
                            "notice-retry",
                            ui::tool_button("notice-retry", "Retry", None, true, true),
                            true,
                            cx,
                            |this, window, cx| {
                                this.changes_command(ChangesCommand::RetryRecovery, window, cx)
                            },
                        ))
                        .child(self.control(
                            "notice-discard",
                            ui::tool_button("notice-discard", "Discard", None, true, false),
                            true,
                            cx,
                            |this, window, cx| {
                                this.changes_command(ChangesCommand::Discard, window, cx)
                            },
                        ));
                }
                ChangesNotice::ReadOnlyWithStaged => {
                    strip = strip.child(self.control(
                        "notice-discard",
                        ui::tool_button("notice-discard", "Discard", None, true, false),
                        true,
                        cx,
                        |this, window, cx| {
                            this.changes_command(ChangesCommand::Discard, window, cx)
                        },
                    ));
                }
                ChangesNotice::Unavailable(_) => {}
            }
            notices.push(strip.into_any_element());
        }
        if let Some(message) = &self.preferences_status {
            notices.push(
                div()
                    .id("column-preferences-status")
                    .role(Role::Status)
                    .aria_label(message.clone())
                    .flex_none()
                    .min_h(px(style::FOOTER))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .text_sm()
                    .text_color(style::dim())
                    .border_b_1()
                    .border_color(style::line_soft())
                    .child(message.clone())
                    .into_any_element(),
            );
        }
        notices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::EffectiveSafeMode;

    fn summary(staged: usize) -> ChangesSummary {
        ChangesSummary {
            staged,
            included: staged,
            updates: staged,
            inserts: 0,
            deletes: 0,
            pending: false,
            editing: false,
            dialog_open: false,
            can_review: if staged > 0 {
                Ok(())
            } else {
                Err("Stage a change first".into())
            },
            notice: None,
            message: SharedString::default(),
        }
    }

    fn writable() -> TablePolicy {
        TablePolicy {
            environment: None,
            safe_mode: EffectiveSafeMode::Disabled,
            read_only: false,
        }
    }

    fn read_only() -> TablePolicy {
        TablePolicy {
            read_only: true,
            ..writable()
        }
    }

    #[test]
    fn staged_changes_show_review_with_the_count() {
        let mut changes = summary(3);
        changes.included = 2;
        let mode = toolbar_mode(&changes, 0, writable(), true, false, Ok(()));
        let staged = mode.staged.expect("staged group");
        assert_eq!(staged.changes, 3);
        assert_eq!(staged.review, 2);
        assert_eq!(staged.can_review, Ok(()));
        assert!(staged.can_discard);
        assert!(
            toolbar_mode(&summary(0), 0, writable(), true, false, Ok(()))
                .staged
                .is_none()
        );
    }

    #[test]
    fn review_and_discard_wait_for_a_pending_apply() {
        let mut changes = summary(1);
        changes.pending = true;
        let mode = toolbar_mode(&changes, 0, writable(), true, false, Ok(()));
        assert!(!mode.staged.unwrap().can_discard);
        assert!(mode.cancel, "an in-flight apply shows Cancel");
        let busy = toolbar_mode(&summary(1), 0, writable(), true, true, Ok(()));
        assert!(busy.staged.unwrap().can_review.is_err());
    }

    #[test]
    fn checked_rows_show_delete_with_the_count() {
        let mode = toolbar_mode(&summary(0), 2, writable(), true, false, Ok(()));
        assert_eq!(mode.delete, Some((2, Ok(()))));
        assert_eq!(
            toolbar_mode(&summary(0), 0, writable(), true, false, Ok(())).delete,
            None
        );
    }

    #[test]
    fn read_only_shows_the_badge_hides_add_row_and_disables_delete() {
        let mode = toolbar_mode(
            &summary(0),
            1,
            read_only(),
            true,
            false,
            Err("Read-only connection".into()),
        );
        let reason = read_only().read_only_reason().map(SharedString::from);
        assert!(reason.is_some());
        assert_eq!(mode.read_only, reason);
        assert_eq!(mode.add_row, None);
        let (count, delete) = mode.delete.expect("delete stays visible");
        assert_eq!(count, 1);
        assert_eq!(delete, Err(reason.unwrap()));
    }

    #[test]
    fn add_row_is_disabled_with_the_reason_until_analysis_is_ready() {
        let mode = toolbar_mode(
            &summary(0),
            0,
            writable(),
            true,
            false,
            Err("Checking editable columns…".into()),
        );
        assert_eq!(mode.add_row, Some(Err("Checking editable columns…".into())));
        let ready = toolbar_mode(&summary(0), 0, writable(), true, false, Ok(()));
        assert_eq!(ready.add_row, Some(Ok(())));
        assert_eq!(ready.read_only, None);
    }

    #[test]
    fn disconnected_shows_connect() {
        let mode = toolbar_mode(&summary(0), 0, writable(), false, false, Ok(()));
        assert!(mode.connect);
        assert_eq!(mode.add_row, Some(Err(CONNECT.into())));
        assert!(!toolbar_mode(&summary(0), 0, writable(), true, false, Ok(())).connect);
    }

    #[test]
    fn busy_shows_cancel() {
        let mode = toolbar_mode(&summary(0), 0, writable(), true, true, Ok(()));
        assert!(mode.cancel);
        assert_eq!(mode.add_row, Some(Err(LOADING.into())));
        assert!(!toolbar_mode(&summary(0), 0, writable(), true, false, Ok(())).cancel);
    }
}
