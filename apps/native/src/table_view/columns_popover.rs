//! Plan 032 §1: the Columns popover. A search field, one visibility toggle and
//! one pin toggle per column, then Show all, Auto-fit all and Reload.
//! Visibility and pin changes go through the stored grid preferences, so the
//! popover never owns column state of its own.
#[cfg(test)]
use super::menus::LegacyAction;
use super::{Action, TableView, menus::Popover};
use crate::{accessible_editor::AccessibleEditor, grid::ColumnEntry, grid_columns::ColumnAction};
use editor::{Editor, EditorEvent};
use gpui::{AnyElement, Context, Role, SharedString, Window, div, prelude::*, px};

/// Footer actions of the popover; per-column toggles are generated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ColumnsItem {
    ShowAll,
    AutoFitAll,
    Reload,
}

pub(super) const COLUMNS_ITEMS: [ColumnsItem; 3] = [
    ColumnsItem::ShowAll,
    ColumnsItem::AutoFitAll,
    ColumnsItem::Reload,
];

impl ColumnsItem {
    #[cfg(test)]
    pub(super) fn covers(self) -> &'static [LegacyAction] {
        match self {
            Self::ShowAll => &[LegacyAction::ShowAllColumns],
            Self::AutoFitAll => &[LegacyAction::AutoFitVisibleColumns],
            Self::Reload => &[LegacyAction::ReloadPreferences],
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::ShowAll => "Show all columns",
            Self::AutoFitAll => "Auto-fit all columns",
            Self::Reload => "Reload column preferences",
        }
    }
    fn action(self) -> Action {
        match self {
            Self::ShowAll => Action::Column(ColumnAction::ShowAll),
            Self::AutoFitAll => Action::AutoFit(true),
            Self::Reload => Action::Preferences,
        }
    }
}

/// Toolbar badge text: `visible/total`, only when something is hidden.
pub(super) fn columns_badge(entries: &[ColumnEntry]) -> Option<String> {
    let visible = entries.iter().filter(|entry| entry.visible).count();
    (visible < entries.len()).then(|| format!("{visible}/{}", entries.len()))
}

/// Case-insensitive substring match on the column name or its type.
pub(super) fn column_matches(entry: &ColumnEntry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || entry.name.to_lowercase().contains(&query)
        || entry.cast_type.to_lowercase().contains(&query)
}

impl TableView {
    pub(super) fn toggle_columns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.popover, Some(Popover::Columns { .. })) {
            self.close_popover(cx);
            return;
        }
        let Some(anchor) = self.anchors.columns.get() else {
            return;
        };
        let search = cx.new(|cx| Editor::single_line(window, cx));
        let field =
            cx.new(|cx| AccessibleEditor::field(search.clone(), "Search columns", false, cx));
        let edits = cx.subscribe(&search, |_, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                cx.notify();
            }
        });
        window.focus(&search.focus_handle(cx), cx);
        self.popover = Some(Popover::Columns {
            anchor,
            search,
            field,
            _edits: edits,
        });
        cx.notify();
    }

    fn set_column_visible(&mut self, source: usize, visible: bool, cx: &mut Context<Self>) {
        if !self.can_browse(cx) {
            self.status = "Wait for the table to finish loading".into();
            return;
        }
        match self.grid.read(cx).visibility_patch(source, visible) {
            Ok(patch) => self.save_preferences(patch, cx),
            Err(error) => self.preferences_status = Some(error.into()),
        }
    }

    fn toggle_column_pin(&mut self, source: usize, cx: &mut Context<Self>) {
        if !self.can_browse(cx) {
            self.status = "Wait for the table to finish loading".into();
            return;
        }
        match self
            .grid
            .read(cx)
            .column_patch(source, ColumnAction::TogglePin)
        {
            Ok(patch) => self.save_preferences(patch, cx),
            Err(error) => self.preferences_status = Some(error.into()),
        }
    }

    pub(super) fn render_columns_popover(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(Popover::Columns {
            anchor,
            search,
            field,
            ..
        }) = &self.popover
        else {
            return None;
        };
        let (anchor, field) = (*anchor, field.clone());
        let query = search.read(cx).text(cx);
        let entries = self.grid.read(cx).column_entries();
        let enabled = self.can_browse(cx);
        let visible = entries.iter().filter(|entry| entry.visible).count();
        let mut list = div()
            .id("columns-popover-list")
            .flex()
            .flex_col()
            .max_h(px(260.))
            .overflow_y_scroll();
        let mut shown = 0;
        for entry in entries.iter().filter(|entry| column_matches(entry, &query)) {
            shown += 1;
            let source = entry.source;
            let show = !entry.visible;
            // Hiding the last visible column is refused by the grid model; the
            // toggle is disabled so the refusal is visible up front.
            let toggle_enabled = enabled && (show || visible > 1);
            let label = SharedString::from(format!("{} {}", entry.name, entry.cast_type));
            let weak = cx.weak_entity();
            let pin_weak = cx.weak_entity();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        crate::ui::popover::check_item(
                            ("column-visible", source),
                            label,
                            entry.visible,
                            false,
                            toggle_enabled,
                        )
                        .flex_1()
                        .min_w_0()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if toggle_enabled {
                                this.set_column_visible(source, show, cx);
                            }
                        }))
                        .on_a11y_action(
                            gpui::accesskit::Action::Click,
                            move |_, _, cx| {
                                if toggle_enabled {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.set_column_visible(source, show, cx)
                                    });
                                }
                            },
                        ),
                    )
                    .child(
                        crate::ui::icon_button(
                            ("column-pin", source),
                            if entry.pinned {
                                format!("Unpin {}", entry.name)
                            } else {
                                format!("Pin {}", entry.name)
                            },
                            if entry.pinned {
                                "icons/unpin.svg"
                            } else {
                                "icons/pin.svg"
                            },
                            enabled,
                        )
                        .when(entry.pinned, |button| {
                            button.text_color(crate::style::accent())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if enabled {
                                this.toggle_column_pin(source, cx);
                            }
                        }))
                        .on_a11y_action(
                            gpui::accesskit::Action::Click,
                            move |_, _, cx| {
                                if enabled {
                                    let _ = pin_weak
                                        .update(cx, |this, cx| this.toggle_column_pin(source, cx));
                                }
                            },
                        ),
                    ),
            );
        }
        if shown == 0 {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(4.))
                    .text_sm()
                    .text_color(crate::style::faint())
                    .child("No matching columns"),
            );
        }
        let mut panel = crate::ui::popover::panel("columns-popover", Role::Dialog, "Columns")
            .w(px(260.))
            .max_h(px(360.))
            .flex()
            .flex_col()
            .on_mouse_down_out(self.dismiss_outside(anchor, cx))
            .child(
                div()
                    .px(px(6.))
                    .pb(px(4.))
                    .child(crate::ui::field().child(div().flex_1().child(field))),
            )
            .child(crate::ui::popover::heading(format!(
                "{visible} of {} shown",
                entries.len()
            )))
            .child(list)
            .child(crate::ui::popover::divider());
        for (index, item) in COLUMNS_ITEMS.into_iter().enumerate() {
            let weak = cx.weak_entity();
            let action = item.action();
            let item_enabled = match item {
                ColumnsItem::Reload => self.editable && !self.busy && self.controls.is_some(),
                _ => enabled,
            };
            panel = panel.child(
                crate::ui::popover::item(
                    ("columns-action", index),
                    item.label(),
                    None,
                    None,
                    false,
                    item_enabled,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if item_enabled {
                        this.activate(action, window, cx);
                    }
                }))
                .on_a11y_action(
                    gpui::accesskit::Action::Click,
                    move |_, window, cx| {
                        if item_enabled {
                            let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
                        }
                    },
                ),
            );
        }
        Some(
            crate::ui::popover::layer(anchor, crate::ui::popover::Placement::Below, panel)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: usize, name: &str, cast_type: &str, visible: bool) -> ColumnEntry {
        ColumnEntry {
            source,
            name: name.into(),
            cast_type: cast_type.into(),
            visible,
            pinned: false,
        }
    }

    #[test]
    fn badge_shows_only_when_columns_are_hidden() {
        let all = [entry(0, "id", "int4", true), entry(1, "name", "text", true)];
        assert_eq!(columns_badge(&all), None);
        let hidden = [
            entry(0, "id", "int4", true),
            entry(1, "name", "text", false),
        ];
        assert_eq!(columns_badge(&hidden).as_deref(), Some("1/2"));
    }

    #[test]
    fn search_matches_name_or_type_case_insensitively() {
        let column = entry(0, "CreatedAt", "timestamptz", true);
        assert!(column_matches(&column, ""));
        assert!(column_matches(&column, "  created "));
        assert!(column_matches(&column, "TIMESTAMP"));
        assert!(!column_matches(&column, "email"));
    }
}
