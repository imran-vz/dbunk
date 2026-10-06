//! Both row and header strips use the same source/display geometry. Frozen and
//! scrolling viewports are clipped independently, including oversized pin sets.
//! A row-number gutter stays fixed at the left of both strips; in table mode it
//! starts with the row checkbox and carries the staged-change marker.
use super::editing::{StagedKind, cell_tone, staged_text};
use super::*;
use crate::data_model::CellRef;
use crate::style;
use dbunk_lib::backend::data::{BrowseSortDirection, BrowseSortKey};

/// Presentation class of one cell; never affects copy, AX or edit values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CellKind {
    Null,
    Number,
    Boolean,
    Text,
}

/// Declared column types decide when known (table pages). Query results carry
/// no types, so only plain decimal literals are shown as numbers.
pub(super) fn cell_kind(value: Option<&str>, cast_type: Option<&str>) -> CellKind {
    let Some(value) = value else {
        return CellKind::Null;
    };
    match cast_type {
        Some(cast_type) => {
            let base = cast_type
                .split('(')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            match base.as_str() {
                "smallint" | "integer" | "bigint" | "int" | "int2" | "int4" | "int8"
                | "numeric" | "decimal" | "real" | "double precision" | "float4" | "float8"
                | "oid" => CellKind::Number,
                "boolean" | "bool" => CellKind::Boolean,
                _ => CellKind::Text,
            }
        }
        None if looks_numeric(value) => CellKind::Number,
        None => CellKind::Text,
    }
}

fn looks_numeric(text: &str) -> bool {
    if text.is_empty() || text.len() > 40 {
        return false;
    }
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    // "007" and zip-code-like text stay text.
    digits(whole) && (whole == "0" || !whole.starts_with('0')) && fraction.is_none_or(digits)
}

/// Fits the largest 1-based row number in the 11 px monospace font.
pub(super) fn gutter_width(rows: usize) -> f32 {
    let digits = rows.max(1).ilog10() as f32 + 1.;
    (digits.max(2.) * 7. + 16.).round()
}

/// Header sort indicator for one column: its direction, plus the 1-based
/// priority only when more than one key sorts the page.
pub(super) fn sort_badge(
    sort: &[BrowseSortKey],
    column: &str,
) -> Option<(BrowseSortDirection, Option<usize>)> {
    let index = sort.iter().position(|key| key.column == column)?;
    Some((sort[index].direction, (sort.len() > 1).then_some(index + 1)))
}

impl ResultGrid {
    fn cast_type(&self, source: usize) -> Option<&str> {
        self.table
            .as_ref()?
            .columns
            .get(source)
            .map(|column| column.cast_type.as_str())
    }
    /// 24 px in table mode, the 20 px row height for query results.
    pub(super) fn header_height(&self) -> Pixels {
        if self.table_mode() {
            px(style::FOOTER)
        } else {
            ROW_HEIGHT
        }
    }
    fn sort_indicator(&self, name: &str) -> Option<gpui::AnyElement> {
        if !self.table_mode() {
            return None;
        }
        Some(match sort_badge(&self.sort_keys, name) {
            Some((direction, priority)) => div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(1.))
                .text_color(style::accent())
                .child(
                    gpui::svg()
                        .path(match direction {
                            BrowseSortDirection::Asc => "icons/arrow_up.svg",
                            BrowseSortDirection::Desc => "icons/arrow_down.svg",
                        })
                        .size(px(9.))
                        .text_color(style::accent()),
                )
                .when_some(priority, |badge, priority| {
                    badge.child(
                        div()
                            .font_family(style::MONO)
                            .text_size(px(style::FONT_SMALL))
                            .child(priority.to_string()),
                    )
                })
                .into_any_element(),
            None => div()
                .flex_none()
                .opacity(0.)
                .group_hover("grid-header", |style| style.opacity(1.))
                .child(
                    gpui::svg()
                        .path("icons/chevron_up_down.svg")
                        .size(px(9.))
                        .text_color(style::faint()),
                )
                .into_any_element(),
        })
    }
    fn header_cell(&self, column: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let source = self.source_column(column).unwrap_or(column);
        let label: SharedString = crate::column_widths::heading(self.column_name(column), source)
            .into_owned()
            .into();
        let cast_type = self.cast_type(source).map(str::to_owned);
        let table_mode = self.table_mode();
        let sort = sort_badge(&self.sort_keys, &label).filter(|_| table_mode);
        let interactive = self.sortable || table_mode;
        let accessible_label: SharedString = match sort {
            Some((BrowseSortDirection::Asc, _)) => format!("{label}, sorted ascending").into(),
            Some((BrowseSortDirection::Desc, _)) => format!("{label}, sorted descending").into(),
            None => label.clone(),
        };
        let weak = cx.entity().downgrade();
        div()
            .id(("header", column))
            .role(Role::ColumnHeader)
            .aria_label(accessible_label)
            .aria_column_index(column + 1)
            .group("grid-header")
            .relative()
            .flex_shrink_0()
            .w(self.column_width(column))
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .gap(px(4.))
            .overflow_hidden()
            .whitespace_nowrap()
            .border_r_1()
            .border_color(style::line_soft())
            .text_color(style::dim())
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(
                div()
                    .flex_shrink(1.)
                    .min_w_0()
                    .truncate()
                    .child(label.clone()),
            )
            .when_some(cast_type, |header, cast_type| {
                header.child(
                    div()
                        .flex_shrink(1.)
                        .min_w_0()
                        .truncate()
                        .font_family(style::MONO)
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_size(px(style::FONT_SMALL))
                        .text_color(style::faint())
                        .child(cast_type),
                )
            })
            .children(self.sort_indicator(&label))
            .when(interactive, |header| {
                header
                    .cursor_pointer()
                    .hover(|s| s.bg(style::hover()).text_color(style::text()))
                    // Click (mouse-up), not mouse-down, so it never fights the
                    // resize drag, which also ends with a mouse-up.
                    .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                        if cx.has_active_drag() || this.resize.is_some() {
                            return;
                        }
                        this.header_clicked(source, &label, event, cx);
                    }))
            })
            .when(table_mode, |header| {
                header.on_a11y_action(gpui::accesskit::Action::Click, move |_, _, cx| {
                    weak.update(cx, |this, cx| this.open_header_menu(source, None, cx))
                        .ok();
                })
            })
            .child(self.resize_handle(column, cx))
            .into_any_element()
    }
    fn header_clicked(
        &mut self,
        source: usize,
        label: &SharedString,
        event: &gpui::ClickEvent,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers().shift && self.sortable {
            cx.emit(GridEvent::Sort {
                column: label.to_string(),
                append: true,
            });
        } else if self.table_mode() {
            self.open_header_menu(source, Some(event.position()), cx);
        } else if self.sortable {
            cx.emit(GridEvent::Sort {
                column: label.to_string(),
                append: false,
            });
        }
    }
    fn open_header_menu(
        &mut self,
        source: usize,
        fallback: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let anchor = self.header_bounds(source).or_else(|| {
            fallback.map(|position| {
                gpui::Bounds::new(position, gpui::size(px(0.), self.header_height()))
            })
        });
        if let Some(anchor) = anchor {
            cx.emit(GridEvent::HeaderMenu { source, anchor });
        }
    }
    /// Row-number gutter of the header: select-all box and `#`.
    fn header_gutter(&self, gutter: f32, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .absolute()
            .left_0()
            .top_0()
            .h_full()
            .w(px(gutter))
            .flex()
            .items_center()
            .border_r_1()
            .border_color(style::line_soft())
            .font_family(style::MONO)
            .text_color(style::faint())
            .when(self.checkboxes(), |gutter| {
                gutter.child(self.header_check(cx))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child("#"),
            )
    }
    pub(super) fn render_header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let panes = self.panes();
        let pinned = self.pinned_columns();
        let scrolling = self.scrolling_columns();
        let gutter = self.gutter();
        let probe = self.header_probe.clone();
        div()
            .h(self.header_height())
            .w_full()
            .relative()
            .flex_shrink_0()
            .overflow_hidden()
            .bg(style::panel())
            .border_b_1()
            .border_color(style::line())
            .child(
                gpui::canvas(move |bounds, _, _| probe.set(Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(self.header_gutter(gutter, cx))
            .child(
                div()
                    .absolute()
                    .left(px(gutter))
                    .top_0()
                    .h_full()
                    .w(px(panes.pinned_viewport))
                    .overflow_hidden()
                    .bg(style::panel())
                    .on_scroll_wheel(cx.listener(Self::scroll_pinned))
                    .child(
                        div()
                            .flex()
                            .h_full()
                            .w(self.total_width())
                            .ml(-px(self.pinned_left()))
                            .child(div().flex_shrink_0().w(self.column_left(pinned.start)))
                            .children(pinned.map(|column| self.header_cell(column, cx))),
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
                    .bg(style::panel())
                    .when(panes.pinned_viewport > 0., |pane| {
                        pane.border_l_1().border_color(style::line())
                    })
                    .child(
                        div()
                            .flex()
                            .h_full()
                            .w(self.total_width())
                            .ml(-px(panes.pinned_width) - self.scroll_left())
                            .child(div().flex_shrink_0().w(self.column_left(scrolling.start)))
                            .children(scrolling.map(|column| self.header_cell(column, cx))),
                    ),
            )
    }
    fn row_cell(
        &self,
        row: usize,
        column: usize,
        cells: &[Option<String>],
        selection: &Option<(Range<usize>, Range<usize>)>,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let source = self.source_column(column).expect("visible source column");
        let selected = selection
            .as_ref()
            .is_some_and(|(rows, columns)| rows.contains(&row) && columns.contains(&column));
        let table_mode = self.table_mode();
        let (mark, included, failed, staged) = self.page_mark(row, source);
        let tone = cell_tone(selected, mark, included, failed);
        let cast_type = self.cast_type(source);
        // Staged values replace the page value on screen; AX says so.
        let (text, kind, exact, label) = match staged {
            Some(value) => {
                let (text, staged_kind) = staged_text(value);
                let exact = SharedString::from(value.text.as_deref().unwrap_or("NULL").to_owned());
                let kind = match staged_kind {
                    StagedKind::Null => CellKind::Null,
                    _ => cell_kind(value.text.as_deref(), cast_type),
                };
                let label = SharedString::from(format!("{exact}, edited"));
                (text, (kind, staged_kind == StagedKind::Empty), exact, label)
            }
            None => {
                let exact = SharedString::from(cell_text(&cells[source]).to_owned());
                let kind = cell_kind(cells[source].as_deref(), cast_type);
                let empty = table_mode && cells[source].as_deref() == Some("");
                let text = if empty {
                    "''".to_owned()
                } else {
                    display_text(&cells[source])
                };
                let label = if mark.deleted {
                    SharedString::from(format!("{exact}, staged for deletion"))
                } else {
                    exact.clone()
                };
                (text, (kind, empty), exact, label)
            }
        };
        let (kind, empty) = kind;
        div()
            .id(("cell", row * self.column_count().max(1) + column))
            .role(Role::Cell)
            // AX/copy keep the complete retained value, never a display prefix.
            .aria_label(label)
            .aria_value(exact)
            .aria_column_index(column + 1)
            .aria_selected(selected)
            .flex_shrink_0()
            .w(self.column_width(column))
            .h_full()
            .px_2()
            .flex()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .border_r_1()
            .border_color(style::line_soft())
            .when(kind == CellKind::Number, |cell| {
                cell.justify_end().text_color(style::number())
            })
            .when(kind == CellKind::Boolean, |cell| {
                cell.text_color(style::boolean())
            })
            .when(kind == CellKind::Null, |cell| {
                cell.italic().text_color(style::faint())
            })
            .when(empty, |cell| cell.text_color(style::faint()))
            .when_some(tone.fill, |cell, fill| cell.bg(fill))
            .when_some(tone.text, |cell, color| cell.text_color(color))
            .when(tone.strike, |cell| cell.line_through())
            .when(selected, |cell| {
                cell.border_1().border_color(style::accent())
            })
            .child(div().min_w_0().truncate().child(text))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.finish_resize(cx);
                    window.focus(&this.focus, cx);
                    this.select(row, column, event.modifiers.shift, cx);
                    if this.table_mode() && event.click_count == 2 && !event.modifiers.shift {
                        cx.emit(GridEvent::EditCell {
                            cell: CellRef::Page(row),
                            source,
                            seed: crate::data_model::EditSeed::Keep,
                        });
                    }
                }),
            )
            .when(table_mode, |cell| {
                cell.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        let inside = this.selection().is_some_and(|(rows, columns)| {
                            rows.contains(&row) && columns.contains(&column)
                        });
                        if !inside {
                            this.select(row, column, false, cx);
                        }
                        cx.emit(GridEvent::ContextMenu {
                            cell: CellRef::Page(row),
                            source,
                            position: event.position,
                        });
                    }),
                )
            })
            .into_any_element()
    }
    /// Row-number gutter of one page row: checkbox, staged marker, number.
    fn row_gutter(
        &self,
        row: usize,
        left: Pixels,
        gutter: f32,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let marker = if self.table_mode() {
            match self.overlay.mark(row) {
                Some(crate::data_model::RowMark::Deleted { .. }) => Some(("−", style::bad())),
                Some(crate::data_model::RowMark::Updated { .. }) => Some(("•", style::warn())),
                None => None,
            }
        } else {
            None
        };
        div()
            .absolute()
            .left(left)
            .top_0()
            .h_full()
            .w(px(gutter))
            .flex()
            .items_center()
            .bg(style::panel())
            .border_r_1()
            .border_color(style::line_soft())
            .text_color(style::faint())
            .when(self.checkboxes(), |gutter| {
                gutter.child(self.row_check(row, cx))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(2.))
                    .when_some(marker, |number, (glyph, color)| {
                        number.child(div().text_color(color).child(glyph))
                    })
                    .child((row + 1).to_string()),
            )
            .into_any_element()
    }
    pub(super) fn render_rows(
        &self,
        rows: Range<usize>,
        cx: &Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let panes = self.panes();
        let pinned = self.pinned_columns();
        let scrolling = self.scrolling_columns();
        let selection = self.selection();
        let scroll = self.scroll_left();
        let gutter = self.gutter();
        rows.filter_map(|row| {
            let cells = self.row(row)?;
            let failed = self.row_failed(row);
            Some(
                div()
                    .id(("row", row))
                    .role(Role::Row)
                    .aria_row_index(row + 1)
                    .group("grid-row")
                    .font_family(style::MONO)
                    .relative()
                    .h(ROW_HEIGHT)
                    .w(px(gutter + panes.content_width))
                    .border_b_1()
                    .border_color(style::line_soft())
                    .when(failed, |row| row.border_1().border_color(style::bad_line()))
                    .child(self.row_gutter(row, scroll, gutter, cx))
                    .child(
                        div()
                            .absolute()
                            .left(scroll + px(gutter))
                            .top_0()
                            .h_full()
                            .w(px(panes.pinned_viewport))
                            .overflow_hidden()
                            .bg(style::bg())
                            .group_hover("grid-row", |s| s.bg(style::row_hover()))
                            .on_scroll_wheel(cx.listener(Self::scroll_pinned))
                            .child(
                                div()
                                    .flex()
                                    .h_full()
                                    .w(self.total_width())
                                    .ml(-px(self.pinned_left()))
                                    .child(div().flex_shrink_0().w(self.column_left(pinned.start)))
                                    .children(pinned.clone().map(|column| {
                                        self.row_cell(row, column, cells, &selection, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(scroll + px(gutter + panes.pinned_viewport))
                            .top_0()
                            .h_full()
                            .w(px(panes.scrolling_viewport))
                            .overflow_hidden()
                            .bg(style::bg())
                            .group_hover("grid-row", |s| s.bg(style::row_hover()))
                            .when(panes.pinned_viewport > 0., |pane| {
                                pane.border_l_1().border_color(style::line())
                            })
                            .child(
                                div()
                                    .flex()
                                    .h_full()
                                    .w(self.total_width())
                                    .ml(-px(panes.pinned_width) - scroll)
                                    .child(
                                        div().flex_shrink_0().w(self.column_left(scrolling.start)),
                                    )
                                    .children(scrolling.clone().map(|column| {
                                        self.row_cell(row, column, cells, &selection, cx)
                                    })),
                            ),
                    ),
            )
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::data::BrowseNulls;

    #[test]
    fn declared_types_win_and_untyped_results_only_promote_plain_decimals() {
        assert_eq!(cell_kind(None, Some("text")), CellKind::Null);
        assert_eq!(
            cell_kind(Some("12.50"), Some("numeric(12,2)")),
            CellKind::Number
        );
        assert_eq!(
            cell_kind(Some("1"), Some("double precision")),
            CellKind::Number
        );
        assert_eq!(cell_kind(Some("t"), Some("boolean")), CellKind::Boolean);
        // A numeric-looking value in a text or array column stays text.
        assert_eq!(cell_kind(Some("42"), Some("text")), CellKind::Text);
        assert_eq!(cell_kind(Some("{1,2}"), Some("integer[]")), CellKind::Text);
        for number in ["0", "-7", "1042", "3.25", "0.5"] {
            assert_eq!(cell_kind(Some(number), None), CellKind::Number, "{number}");
        }
        for text in ["", "007", "1.", ".5", "1e3", "12a", "NULL", "-", "1.2.3"] {
            assert_eq!(cell_kind(Some(text), None), CellKind::Text, "{text}");
        }
        // The literal text "NULL" is not a database NULL.
        assert_eq!(cell_kind(Some("NULL"), Some("text")), CellKind::Text);
    }

    #[test]
    fn gutter_grows_with_the_largest_row_number() {
        assert_eq!(gutter_width(0), gutter_width(99));
        assert!(gutter_width(1_000) > gutter_width(999));
        assert!(gutter_width(10_000_000) > gutter_width(1_000));
    }

    #[test]
    fn sort_badge_shows_priority_only_with_several_keys() {
        let key = |column: &str, direction| BrowseSortKey {
            column: column.into(),
            direction,
            nulls: BrowseNulls::Default,
        };
        let single = [key("id", BrowseSortDirection::Desc)];
        assert_eq!(
            sort_badge(&single, "id"),
            Some((BrowseSortDirection::Desc, None))
        );
        assert_eq!(sort_badge(&single, "name"), None);
        let several = [
            key("name", BrowseSortDirection::Asc),
            key("id", BrowseSortDirection::Desc),
        ];
        assert_eq!(
            sort_badge(&several, "name"),
            Some((BrowseSortDirection::Asc, Some(1)))
        );
        assert_eq!(
            sort_badge(&several, "id"),
            Some((BrowseSortDirection::Desc, Some(2)))
        );
        assert_eq!(sort_badge(&[], "id"), None);
    }
}
