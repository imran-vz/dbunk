//! Row checkboxes for table mode. Checks are page-relative and separate from
//! the cell selection; every new page or `begin` clears them.
use super::*;
use crate::ui::CheckState;
use std::collections::BTreeSet;

/// Width of the checkbox column at the left of the row-number gutter.
pub(super) const CHECK_WIDTH: f32 = 20.;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct CheckedRows {
    set: BTreeSet<usize>,
    anchor: Option<usize>,
}

impl CheckedRows {
    pub fn contains(&self, row: usize) -> bool {
        self.set.contains(&row)
    }
    pub fn rows(&self) -> Vec<usize> {
        self.set.iter().copied().collect()
    }
    /// Plain click flips one row and moves the anchor. Shift-click checks the
    /// whole range from the anchor, like Drizzle and most file browsers.
    pub fn toggle(&mut self, row: usize, shift: bool) {
        match (shift, self.anchor) {
            (true, Some(anchor)) => {
                self.set.extend(anchor.min(row)..=anchor.max(row));
            }
            _ => {
                if !self.set.remove(&row) {
                    self.set.insert(row);
                }
            }
        }
        self.anchor = Some(row);
    }
    /// Header box: everything when not all rows are checked, else nothing.
    pub fn toggle_all(&mut self, rows: usize) {
        if self.state(rows) == CheckState::On {
            self.clear();
        } else {
            self.set = (0..rows).collect();
            self.anchor = None;
        }
    }
    pub fn state(&self, rows: usize) -> CheckState {
        let checked = self.set.range(..rows).count();
        if checked == 0 {
            CheckState::Off
        } else if checked == rows {
            CheckState::On
        } else {
            CheckState::Mixed
        }
    }
    /// Returns whether anything was cleared.
    pub fn clear(&mut self) -> bool {
        self.anchor = None;
        let had = !self.set.is_empty();
        self.set.clear();
        had
    }
}

impl ResultGrid {
    pub(super) fn checkboxes(&self) -> bool {
        self.editing
            .as_ref()
            .is_some_and(|editing| editing.checkboxes)
    }

    /// The 20 px checkbox cell for one page row.
    pub(super) fn row_check(&self, row: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let state = if self.checked.contains(row) {
            CheckState::On
        } else {
            CheckState::Off
        };
        div()
            .id(("row-check", row))
            .role(Role::CheckBox)
            .aria_label(SharedString::from(format!("Select row {}", row + 1)))
            .aria_toggled(state.toggled())
            .flex_none()
            .w(px(CHECK_WIDTH))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(crate::ui::tri_check_box(state))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                this.checked.toggle(row, event.modifiers().shift);
                cx.emit(GridEvent::CheckedRowsChanged);
                cx.notify();
            }))
            .into_any_element()
    }

    /// The header's select-all box, with a mixed state for partial checks.
    pub(super) fn header_check(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let state = self.checked.state(self.row_count());
        div()
            .id("row-check-all")
            .role(Role::CheckBox)
            .aria_label("Select all rows on this page")
            .aria_toggled(state.toggled())
            .flex_none()
            .w(px(CHECK_WIDTH))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(crate::ui::tri_check_box(state))
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                let rows = this.row_count();
                this.checked.toggle_all(rows);
                cx.emit(GridEvent::CheckedRowsChanged);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Clears checks and reports the change once.
    pub(super) fn reset_checked(&mut self, cx: &mut Context<Self>) {
        if self.checked.clear() {
            cx.emit(GridEvent::CheckedRowsChanged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_shift_range_select_all_and_clear() {
        let mut checked = CheckedRows::default();
        assert_eq!(checked.state(5), CheckState::Off);
        checked.toggle(1, false);
        assert_eq!(checked.rows(), [1]);
        assert_eq!(checked.state(5), CheckState::Mixed);
        // Shift extends from the anchor in either direction.
        checked.toggle(3, true);
        assert_eq!(checked.rows(), [1, 2, 3]);
        checked.toggle(0, true);
        assert_eq!(checked.rows(), [0, 1, 2, 3]);
        // A plain click flips one row and becomes the next anchor.
        checked.toggle(2, false);
        assert_eq!(checked.rows(), [0, 1, 3]);
        checked.toggle(4, true);
        assert_eq!(checked.rows(), [0, 1, 2, 3, 4]);
        assert_eq!(checked.state(5), CheckState::On);
        // Rows beyond the current page do not count toward the state.
        assert_eq!(checked.state(3), CheckState::On);
        checked.toggle_all(5);
        assert_eq!(checked.state(5), CheckState::Off);
        checked.toggle(2, false);
        checked.toggle_all(5);
        assert_eq!(checked.rows().len(), 5);
        assert_eq!(checked.state(5), CheckState::On);
        assert!(checked.clear());
        assert!(!checked.clear());
        assert_eq!(checked.state(5), CheckState::Off);
        // Shift without an anchor behaves like a plain click.
        checked.toggle(3, true);
        assert_eq!(checked.rows(), [3]);
    }

    #[test]
    fn select_all_on_an_empty_page_stays_off() {
        let mut checked = CheckedRows::default();
        checked.toggle_all(0);
        assert_eq!(checked.state(0), CheckState::Off);
        assert!(checked.rows().is_empty());
    }
}
