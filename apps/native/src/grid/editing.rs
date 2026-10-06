//! Table-mode editing surface: staged-value tints from the draft overlay, the
//! insert band for draft rows, and the host layer for the inline cell editor.
//! The grid only paints and reports intent; TableChanges owns every value.
use super::*;
use crate::data_model::{CellRef, InsertCell, InsertRow, OverlayValue, RowMark};
use crate::style;
use gpui::{AnyView, Bounds, Rgba};

/// Pushed by TableView. `editable` and `reason` describe the current policy;
/// edit gestures are still reported so TableView can explain a refusal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableEditing {
    pub editable: bool,
    pub checkboxes: bool,
    pub reason: Option<SharedString>,
}

/// The inline editor TableChanges owns, hosted at one cell.
#[derive(Clone)]
pub struct InlineEditorSlot {
    pub cell: CellRef,
    pub source: usize,
    pub view: AnyView,
}

impl InlineEditorSlot {
    fn same(&self, other: &Self) -> bool {
        self.cell == other.cell
            && self.source == other.source
            && self.view.entity_id() == other.view.entity_id()
    }
}

/// Draft rows visible at once before the band scrolls.
pub(super) const BAND_ROWS: usize = 5;
/// Staged display text is already capped at 2 KiB; the grid shows less.
const STAGED_DISPLAY_CHARS: usize = 256;

/// What the draft says about one painted cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CellMark {
    pub deleted: bool,
    pub edited: bool,
    pub inserted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CellTone {
    pub fill: Option<Rgba>,
    pub text: Option<Rgba>,
    pub strike: bool,
    pub outline: bool,
}

fn half(color: Rgba) -> Rgba {
    Rgba {
        a: color.a * 0.5,
        ..color
    }
}

/// Pure tint rules. Deleted wins over edited, which wins over inserted;
/// excluded changes keep their hue at half alpha; the change named by the
/// last apply failure is outlined. Selection replaces the fill only.
pub(super) fn cell_tone(selected: bool, mark: CellMark, included: bool, failed: bool) -> CellTone {
    let (fill, text, strike) = if mark.deleted {
        (Some(style::deleted_fill()), Some(style::faint()), true)
    } else if mark.edited || mark.inserted {
        (Some(style::edited_fill()), Some(style::warn()), false)
    } else {
        (None, None, false)
    };
    let fill = if included { fill } else { fill.map(half) };
    CellTone {
        fill: if selected {
            Some(style::select())
        } else {
            fill
        },
        text,
        strike,
        outline: failed,
    }
}

/// Display text for a staged value: NULL and the empty string stay distinct.
pub(super) fn staged_text(value: &OverlayValue) -> (String, StagedKind) {
    match value.text.as_deref() {
        None => ("NULL".into(), StagedKind::Null),
        Some("") => ("''".into(), StagedKind::Empty),
        Some(text) => {
            let mut shown = match text.char_indices().nth(STAGED_DISPLAY_CHARS) {
                Some((end, _)) => text[..end].to_owned(),
                None => text.to_owned(),
            };
            if value.truncated || shown.len() < text.len() {
                shown.push('…');
            }
            (shown, StagedKind::Text)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StagedKind {
    Null,
    Empty,
    Text,
}

impl ResultGrid {
    pub(super) fn table_mode(&self) -> bool {
        self.editing.is_some()
    }

    /// Tint inputs for one page cell from the overlay.
    pub(super) fn page_mark(
        &self,
        row: usize,
        source: usize,
    ) -> (CellMark, bool, bool, Option<&OverlayValue>) {
        if !self.table_mode() {
            return (CellMark::default(), true, false, None);
        }
        match self.overlay.mark(row) {
            Some(RowMark::Updated {
                change, included, ..
            }) => {
                let value = self.overlay.cell(row, source);
                (
                    CellMark {
                        edited: value.is_some(),
                        ..CellMark::default()
                    },
                    *included,
                    self.overlay.failed == Some(*change),
                    value,
                )
            }
            Some(RowMark::Deleted { change, included }) => (
                CellMark {
                    deleted: true,
                    ..CellMark::default()
                },
                *included,
                self.overlay.failed == Some(*change),
                None,
            ),
            None => (CellMark::default(), true, false, None),
        }
    }

    /// Whether the last apply failure names this page row's change.
    pub(super) fn row_failed(&self, row: usize) -> bool {
        self.table_mode()
            && self.overlay.failed.is_some()
            && match self.overlay.mark(row) {
                Some(RowMark::Updated { change, .. } | RowMark::Deleted { change, .. }) => {
                    self.overlay.failed == Some(*change)
                }
                None => false,
            }
    }

    fn band_rows(&self) -> usize {
        if self.table_mode() && self.table.is_some() {
            self.overlay.inserts.len()
        } else {
            0
        }
    }

    /// Height of the insert band above the rows: at most five rows.
    pub(super) fn band_height(&self) -> f32 {
        self.band_rows().min(BAND_ROWS) as f32 * crate::style::ROW
    }

    pub(super) fn display_of(&self, source: usize) -> Option<usize> {
        if self.table.is_some() {
            self.columns.display(source)
        } else {
            self.model.active_set()?.widths.display(source)
        }
    }

    pub(super) fn cell_geometry(&self, display: usize, height: f32) -> frozen::CellGeometry {
        frozen::CellGeometry {
            gutter: self.gutter(),
            panes: self.panes(),
            pinned_left: self.pinned_left(),
            scroll_left: self.scroll_left().max(px(0.)) / px(1.),
            left: self.column_left(display) / px(1.),
            width: self.column_width(display) / px(1.),
            height,
        }
    }

    /// One cell's rectangle relative to the body (insert band + rows), and
    /// the pane range that clips it.
    pub(super) fn body_cell_rect(
        &self,
        cell: CellRef,
        source: usize,
    ) -> Option<(Bounds<Pixels>, (f32, f32))> {
        let display = self.display_of(source)?;
        let y = match cell {
            CellRef::Page(row) => {
                if row >= self.row_count() {
                    return None;
                }
                let offset = self.scroll().0.borrow().base_handle.offset().y / px(1.);
                self.band_height() + row as f32 * crate::style::ROW + offset
            }
            CellRef::Insert(change) => {
                let index = self
                    .overlay
                    .inserts
                    .iter()
                    .position(|insert| insert.change == change)?;
                index as f32 * crate::style::ROW + self.band_scroll.offset().y / px(1.)
            }
        };
        let pinned = display < self.pinned_count();
        let geometry = self.cell_geometry(display, crate::style::ROW);
        Some((
            frozen::cell_rect(geometry, y, pinned),
            frozen::pane_range(geometry, pinned),
        ))
    }

    fn insert_cell(
        &self,
        index: usize,
        insert: &InsertRow,
        display: usize,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let source = self.source_column(display).unwrap_or(display);
        let change = insert.change;
        let tone = cell_tone(
            false,
            CellMark {
                inserted: true,
                ..CellMark::default()
            },
            insert.included,
            self.overlay.failed == Some(change),
        );
        let (text, kind) = match insert.cells.get(source) {
            Some(InsertCell::Value(value)) => staged_text(value),
            _ => ("DEFAULT".to_owned(), StagedKind::Null),
        };
        let column = self.column_name(display).unwrap_or("column").to_owned();
        div()
            .id(("insert-cell", index * self.column_count().max(1) + display))
            .role(Role::Cell)
            .aria_label(SharedString::from(format!("{column}: {text}, new row")))
            .aria_column_index(display + 1)
            .flex_shrink_0()
            .w(self.column_width(display))
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_r_1()
            .border_color(style::line_soft())
            .when_some(tone.fill, |cell, fill| cell.bg(fill))
            .when_some(tone.text, |cell, color| cell.text_color(color))
            .when(kind != StagedKind::Text, |cell| {
                cell.text_color(style::faint())
            })
            .when(kind == StagedKind::Null, |cell| cell.italic())
            .child(div().min_w_0().truncate().child(text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.finish_resize(cx);
                    window.focus(&this.focus, cx);
                    if event.click_count == 2 {
                        cx.emit(GridEvent::EditCell {
                            cell: CellRef::Insert(change),
                            source,
                            seed: crate::data_model::EditSeed::Keep,
                        });
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |_, event: &MouseDownEvent, _, cx| {
                    cx.emit(GridEvent::ContextMenu {
                        cell: CellRef::Insert(change),
                        source,
                        position: event.position,
                    });
                }),
            )
            .into_any_element()
    }

    fn insert_row(&self, index: usize, insert: &InsertRow, cx: &Context<Self>) -> gpui::AnyElement {
        let panes = self.panes();
        let pinned = self.pinned_columns();
        let scrolling = self.scrolling_columns();
        let gutter = self.gutter();
        let change = insert.change;
        let failed = self.overlay.failed == Some(change);
        div()
            .id(("insert-row", index))
            .role(Role::Row)
            .aria_label(SharedString::from(format!("New row {}", index + 1)))
            .relative()
            .flex_none()
            .h(ROW_HEIGHT)
            .w_full()
            .border_b_1()
            .border_color(if failed {
                style::bad_line()
            } else {
                style::line_soft()
            })
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(px(gutter))
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .bg(style::panel())
                    .border_r_1()
                    .border_color(style::line_soft())
                    .child(div().text_color(style::warn()).child("+"))
                    .child(
                        div()
                            .id(("remove-insert", index))
                            .role(Role::Button)
                            .aria_label("Remove new row")
                            .size(px(14.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .cursor_pointer()
                            .text_color(style::faint())
                            .hover(|button| button.bg(style::hover()).text_color(style::text()))
                            .child(
                                gpui::svg()
                                    .path("icons/close.svg")
                                    .size(px(9.))
                                    .text_color(style::faint()),
                            )
                            .on_click(cx.listener(move |_, _: &gpui::ClickEvent, _, cx| {
                                cx.emit(GridEvent::RemoveInsert { change });
                            })),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(gutter))
                    .top_0()
                    .h_full()
                    .w(px(panes.pinned_viewport))
                    .overflow_hidden()
                    .bg(style::bg())
                    .child(
                        div()
                            .flex()
                            .h_full()
                            .w(self.total_width())
                            .ml(-px(self.pinned_left()))
                            .child(div().flex_shrink_0().w(self.column_left(pinned.start)))
                            .children(
                                pinned.map(|display| self.insert_cell(index, insert, display, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(gutter + panes.pinned_viewport))
                    .top_0()
                    .h_full()
                    .w(px(panes.scrolling_viewport))
                    .overflow_hidden()
                    .bg(style::bg())
                    .when(panes.pinned_viewport > 0., |pane| {
                        pane.border_l_1().border_color(style::line())
                    })
                    .child(
                        div()
                            .flex()
                            .h_full()
                            .w(self.total_width())
                            .ml(-px(panes.pinned_width) - self.scroll_left().max(px(0.)))
                            .child(div().flex_shrink_0().w(self.column_left(scrolling.start)))
                            .children(
                                scrolling
                                    .map(|display| self.insert_cell(index, insert, display, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// Draft rows pinned above the page, newest first as staged. Five rows
    /// are visible; more scroll inside the band.
    pub(super) fn render_insert_band(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        if self.band_rows() == 0 || self.column_count() == 0 {
            return None;
        }
        Some(
            div()
                .id("insert-band")
                .role(Role::Group)
                .aria_label(SharedString::from(format!(
                    "{} new rows",
                    self.overlay.inserts.len()
                )))
                .flex_none()
                .h(px(self.band_height()))
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .track_scroll(&self.band_scroll)
                .font_family(style::MONO)
                .border_b_1()
                .border_color(style::line())
                .children(
                    self.overlay
                        .inserts
                        .iter()
                        .enumerate()
                        .map(|(index, insert)| self.insert_row(index, insert, cx)),
                )
                .into_any_element(),
        )
    }

    /// Hosts TableChanges' inline editor outside the virtualized rows, so it
    /// stays in the element tree (and keeps focus) when its row scrolls away.
    pub(super) fn render_editor_layer(&self) -> Option<gpui::AnyElement> {
        let slot = self.inline_editor.as_ref()?;
        let view = slot.view.clone();
        Some(match self.body_cell_rect(slot.cell, slot.source) {
            Some((rect, (pane_left, pane_width))) => div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(pane_left))
                .w(px(pane_width))
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .left(rect.origin.x - px(pane_left))
                        .top(rect.origin.y)
                        .w(rect.size.width.max(px(crate::grid_columns::RESIZE_MIN)))
                        .h(rect.size.height)
                        .child(view),
                )
                .into_any_element(),
            // A hidden column or stale row: keep the editor mounted off-screen
            // so focus and draft text survive until TableChanges closes it.
            None => div()
                .absolute()
                .top_0()
                .left(px(-10_000.))
                .w(px(160.))
                .h(ROW_HEIGHT)
                .child(view)
                .into_any_element(),
        })
    }

    pub fn set_table_editing(&mut self, editing: TableEditing, cx: &mut Context<Self>) {
        if self.editing.as_ref() == Some(&editing) {
            return;
        }
        if !editing.checkboxes {
            self.reset_checked(cx);
        }
        self.editing = Some(editing);
        cx.notify();
    }

    pub fn set_overlay(
        &mut self,
        overlay: Rc<crate::data_model::DraftOverlay>,
        cx: &mut Context<Self>,
    ) {
        if Rc::ptr_eq(&self.overlay, &overlay) {
            return;
        }
        self.overlay = overlay;
        cx.notify();
    }

    pub fn set_inline_editor(&mut self, slot: Option<InlineEditorSlot>, cx: &mut Context<Self>) {
        let unchanged = match (&self.inline_editor, &slot) {
            (Some(old), Some(new)) => old.same(new),
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        self.inline_editor = slot;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(deleted: bool, edited: bool, inserted: bool) -> CellMark {
        CellMark {
            deleted,
            edited,
            inserted,
        }
    }

    #[test]
    fn tone_precedence_is_deleted_then_edited_then_inserted() {
        let deleted = cell_tone(false, mark(true, true, true), true, false);
        assert_eq!(deleted.fill, Some(style::deleted_fill()));
        assert_eq!(deleted.text, Some(style::faint()));
        assert!(deleted.strike);
        let edited = cell_tone(false, mark(false, true, true), true, false);
        assert_eq!(edited.fill, Some(style::edited_fill()));
        assert_eq!(edited.text, Some(style::warn()));
        assert!(!edited.strike);
        let inserted = cell_tone(false, mark(false, false, true), true, false);
        assert_eq!(inserted.fill, Some(style::edited_fill()));
        let plain = cell_tone(false, CellMark::default(), true, false);
        assert_eq!((plain.fill, plain.text, plain.strike), (None, None, false));
    }

    #[test]
    fn excluded_halves_alpha_failed_outlines_and_selection_wins_the_fill() {
        let excluded = cell_tone(false, mark(false, true, false), false, false);
        let fill = excluded.fill.unwrap();
        assert_eq!(fill.a, style::edited_fill().a * 0.5);
        assert_eq!((fill.r, fill.g, fill.b), {
            let full = style::edited_fill();
            (full.r, full.g, full.b)
        });
        let excluded_delete = cell_tone(false, mark(true, false, false), false, false);
        assert_eq!(
            excluded_delete.fill.unwrap().a,
            style::deleted_fill().a * 0.5
        );
        assert!(cell_tone(false, mark(true, false, false), true, true).outline);
        assert!(!cell_tone(false, mark(true, false, false), true, false).outline);
        let selected = cell_tone(true, mark(false, true, false), true, false);
        assert_eq!(selected.fill, Some(style::select()));
        // The staged text colour survives selection.
        assert_eq!(selected.text, Some(style::warn()));
    }

    #[test]
    fn staged_text_keeps_null_empty_and_truncation_distinct() {
        let value = |text: Option<&str>, truncated| OverlayValue {
            text: text.map(str::to_owned),
            truncated,
        };
        assert_eq!(
            staged_text(&value(None, false)),
            ("NULL".into(), StagedKind::Null)
        );
        assert_eq!(
            staged_text(&value(Some(""), false)),
            ("''".into(), StagedKind::Empty)
        );
        assert_eq!(staged_text(&value(Some("NULL"), false)).1, StagedKind::Text);
        assert_eq!(staged_text(&value(Some("ab"), true)).0, "ab…");
        let long = "é".repeat(STAGED_DISPLAY_CHARS + 3);
        let (shown, _) = staged_text(&value(Some(&long), false));
        assert_eq!(shown.chars().count(), STAGED_DISPLAY_CHARS + 1);
        assert!(shown.ends_with('…'));
    }
}
