//! Both row and header strips use the same source/display geometry. Frozen and
//! scrolling viewports are clipped independently, including oversized pin sets.
use super::*;

impl ResultGrid {
    fn header_cell(&self, column: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let label: SharedString = crate::column_widths::heading(
            self.column_name(column),
            self.source_column(column).unwrap_or(column),
        )
        .into_owned()
        .into();
        div()
            .id(("header", column))
            .role(Role::ColumnHeader)
            .aria_label(label.clone())
            .aria_column_index(column + 1)
            .flex_shrink_0()
            .w(self.column_width(column))
            .h_full()
            .px_2()
            .border_r_1()
            .border_color(crate::style::line())
            .truncate()
            .child(label.clone())
            .when(self.sortable, |header| {
                header.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |_, event: &MouseDownEvent, _, cx| {
                        cx.emit(GridEvent::Sort {
                            column: label.to_string(),
                            append: event.modifiers.shift,
                        });
                    }),
                )
            })
            .into_any_element()
    }
    pub(super) fn render_header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let panes = self.panes();
        let pinned = self.pinned_columns();
        let scrolling = self.scrolling_columns();
        div()
            .h(ROW_HEIGHT)
            .w_full()
            .relative()
            .flex_shrink_0()
            .overflow_hidden()
            .border_b_1()
            .border_color(crate::style::line())
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(px(panes.pinned_viewport))
                    .overflow_hidden()
                    .bg(crate::style::bg())
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
                    .left(px(panes.pinned_viewport))
                    .top_0()
                    .h_full()
                    .w(px(panes.scrolling_viewport))
                    .overflow_hidden()
                    .bg(crate::style::bg())
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
        let exact = SharedString::from(cell_text(&cells[source]).to_owned());
        let selected = selection
            .as_ref()
            .is_some_and(|(rows, columns)| rows.contains(&row) && columns.contains(&column));
        div()
            .id(("cell", row * self.column_count().max(1) + column))
            .role(Role::Cell)
            // AX/copy keep the complete retained value, never a display prefix.
            .aria_label(exact.clone())
            .aria_value(exact)
            .aria_column_index(column + 1)
            .aria_selected(selected)
            .flex_shrink_0()
            .w(self.column_width(column))
            .h_full()
            .px_2()
            .border_r_1()
            .border_color(crate::style::hover())
            .truncate()
            .when(selected, |cell| cell.bg(gpui::rgb(0x203247)))
            .child(display_text(&cells[source]))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.select(row, column, event.modifiers.shift, cx);
                }),
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
        rows.filter_map(|row| {
            let cells = self.row(row)?;
            Some(
                div()
                    .id(("row", row))
                    .role(Role::Row)
                    .aria_row_index(row + 1)
                    .relative()
                    .h(ROW_HEIGHT)
                    .w(px(panes.content_width))
                    .border_b_1()
                    .border_color(crate::style::hover())
                    .child(
                        div()
                            .absolute()
                            .left(scroll)
                            .top_0()
                            .h_full()
                            .w(px(panes.pinned_viewport))
                            .overflow_hidden()
                            .bg(crate::style::bg())
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
                            .left(scroll + px(panes.pinned_viewport))
                            .top_0()
                            .h_full()
                            .w(px(panes.scrolling_viewport))
                            .overflow_hidden()
                            .bg(crate::style::bg())
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
