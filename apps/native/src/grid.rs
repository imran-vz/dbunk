//! Read-only results, virtualized by row and column. Layout switching reuses
//! this entity; per-result selection and scroll handles survive reflow.

use std::ops::Range;

use dbunk_lib::backend::QueryEvent;
use gpui::{
    App, ClipboardItem, Context, FocusHandle, Focusable, ListHorizontalSizingBehavior, MouseButton,
    MouseDownEvent, Pixels, Role, ScrollStrategy, SharedString, UniformListScrollHandle, Window,
    actions, div, prelude::*, px, rgb, uniform_list,
};

use crate::results::{ResultModel, cell_text, display_text};

actions!(
    native,
    [
        CopyCells,
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        SelectUp,
        SelectDown,
        SelectLeft,
        SelectRight
    ]
);

const ROW_HEIGHT: Pixels = px(24.);
const COLUMN_WIDTH: Pixels = px(160.);

struct GridView {
    scroll: UniformListScrollHandle,
    anchor: Option<(usize, usize)>,
    head: Option<(usize, usize)>,
}

impl Default for GridView {
    fn default() -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            anchor: None,
            head: None,
        }
    }
}

pub struct ResultGrid {
    model: ResultModel,
    views: Vec<GridView>,
    empty_scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

impl ResultGrid {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            model: ResultModel::default(),
            views: Vec::new(),
            empty_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn model(&self) -> &ResultModel {
        &self.model
    }

    pub fn pane_focus(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.model = ResultModel::default();
        self.views.clear();
        cx.notify();
    }

    /// Consume before ACK. One paint notification per event, never per cell.
    pub fn consume(&mut self, event: QueryEvent, cx: &mut Context<Self>) -> bool {
        let retain = self.model.consume(event);
        self.views
            .resize_with(self.model.sets.len(), GridView::default);
        cx.notify();
        retain
    }

    pub fn set_active(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.model.sets.len() {
            self.model.active = index;
            cx.notify();
        }
    }

    fn scroll(&self) -> &UniformListScrollHandle {
        self.views
            .get(self.model.active)
            .map_or(&self.empty_scroll, |view| &view.scroll)
    }

    fn total_width(&self) -> Pixels {
        COLUMN_WIDTH * self.model.active_set().map_or(0, |set| set.columns.len())
    }

    fn scroll_left(&self) -> Pixels {
        -self.scroll().0.borrow().base_handle.offset().x
    }

    fn visible_columns(&self) -> Range<usize> {
        let count = self.model.active_set().map_or(0, |set| set.columns.len());
        let left = self.scroll_left().max(px(0.));
        let viewport = self.scroll().0.borrow().base_handle.bounds().size.width;
        let right = left
            + if viewport > px(0.) {
                viewport
            } else {
                px(2400.)
            };
        let first = (left / COLUMN_WIDTH) as usize;
        let last = (right / COLUMN_WIDTH).ceil() as usize;
        first.saturating_sub(1).min(count)..last.saturating_add(1).min(count)
    }

    fn selection(&self) -> Option<(Range<usize>, Range<usize>)> {
        let view = self.views.get(self.model.active)?;
        let (anchor, head) = (view.anchor?, view.head?);
        Some((
            anchor.0.min(head.0)..anchor.0.max(head.0) + 1,
            anchor.1.min(head.1)..anchor.1.max(head.1) + 1,
        ))
    }

    fn select(&mut self, row: usize, column: usize, extend: bool, cx: &mut Context<Self>) {
        let Some(view) = self.views.get_mut(self.model.active) else {
            return;
        };
        if !extend || view.anchor.is_none() {
            view.anchor = Some((row, column));
        }
        view.head = Some((row, column));
        cx.notify();
    }

    fn move_cell(&mut self, dy: isize, dx: isize, extend: bool, cx: &mut Context<Self>) {
        let Some(set) = self.model.active_set() else {
            return;
        };
        if set.rows.is_empty() || set.columns.is_empty() {
            return;
        }
        let current = self.views[self.model.active].head;
        let (row, column) = current.unwrap_or((0, 0));
        let row = if current.is_some() {
            row.saturating_add_signed(dy).min(set.rows.len() - 1)
        } else {
            0
        };
        let column = if current.is_some() {
            column.saturating_add_signed(dx).min(set.columns.len() - 1)
        } else {
            0
        };
        self.select(row, column, extend, cx);
        self.scroll().scroll_to_item(row, ScrollStrategy::Nearest);
        let scroll = self.scroll().0.borrow();
        let mut offset = scroll.base_handle.offset();
        let viewport = scroll.base_handle.bounds().size.width;
        let left = COLUMN_WIDTH * column;
        let right = left + COLUMN_WIDTH;
        if left < -offset.x {
            offset.x = -left;
        } else if right > -offset.x + viewport {
            offset.x = -(right - viewport).max(px(0.));
        }
        scroll.base_handle.set_offset(offset);
    }

    fn copy(&mut self, _: &CopyCells, _window: &mut Window, cx: &mut Context<Self>) {
        let (Some(set), Some((rows, columns))) = (self.model.active_set(), self.selection()) else {
            return;
        };
        let mut text = String::new();
        for (line, row) in rows.enumerate() {
            let Some(cells) = set.rows.get(row) else {
                break;
            };
            if line > 0 {
                text.push('\n');
            }
            for (index, column) in columns.clone().enumerate() {
                if index > 0 {
                    text.push('\t');
                }
                if let Some(value) = cells.get(column) {
                    text.push_str(cell_text(value));
                }
            }
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn render_header(&self, columns: Range<usize>) -> impl IntoElement + use<> {
        let labels = self.model.active_set().map(|set| &set.columns);
        div()
            .h(ROW_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .overflow_hidden()
            .border_b_1()
            .border_color(rgb(0x333333))
            .child(
                div()
                    .flex()
                    .h_full()
                    .w(self.total_width())
                    .ml(-self.scroll_left())
                    .child(div().flex_shrink_0().w(COLUMN_WIDTH * columns.start))
                    .children(columns.map(|column| {
                        let label: SharedString = labels
                            .and_then(|labels| labels.get(column))
                            .and_then(|label| label.as_ref())
                            .map_or_else(
                                || format!("Column {} (name omitted)", column + 1),
                                Clone::clone,
                            )
                            .into();
                        div()
                            .id(("header", column))
                            .role(Role::ColumnHeader)
                            .aria_label(label.clone())
                            .aria_column_index(column + 1)
                            .flex_shrink_0()
                            .w(COLUMN_WIDTH)
                            .h_full()
                            .px_2()
                            .border_r_1()
                            .border_color(rgb(0x333333))
                            .truncate()
                            .child(label)
                    })),
            )
    }

    fn render_rows(&self, rows: Range<usize>, cx: &Context<Self>) -> Vec<impl IntoElement + use<>> {
        let columns = self.visible_columns();
        let selection = self.selection();
        let total_width = self.total_width();
        let left = COLUMN_WIDTH * columns.start;
        let Some(set) = self.model.active_set() else {
            return Vec::new();
        };
        let column_count = set.columns.len().max(1);
        rows.filter_map(|row| {
            let cells = set.rows.get(row)?.clone();
            Some(
                div()
                    .id(("row", row))
                    .role(Role::Row)
                    .aria_row_index(row + 1)
                    .flex()
                    .h(ROW_HEIGHT)
                    .w(total_width)
                    .border_b_1()
                    .border_color(rgb(0x222222))
                    .child(div().flex_shrink_0().w(left))
                    .children(columns.clone().map(|column| {
                        let exact = SharedString::from(cell_text(&cells[column]).to_owned());
                        let selected = selection.as_ref().is_some_and(|(rows, columns)| {
                            rows.contains(&row) && columns.contains(&column)
                        });
                        div()
                            .id(("cell", row * column_count + column))
                            .role(Role::Cell)
                            // AX and copy get the complete retained value, never the
                            // display prefix. A missing value reads as NULL.
                            .aria_label(exact.clone())
                            .aria_value(exact)
                            .aria_column_index(column + 1)
                            .aria_selected(selected)
                            .flex_shrink_0()
                            .w(COLUMN_WIDTH)
                            .h_full()
                            .px_2()
                            .border_r_1()
                            .border_color(rgb(0x222222))
                            .truncate()
                            .when(selected, |cell| cell.bg(rgb(0x203247)))
                            .child(display_text(&cells[column]))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    window.focus(&this.focus, cx);
                                    this.select(row, column, event.modifiers.shift, cx);
                                }),
                            )
                    })),
            )
        })
        .collect()
    }
}

impl Focusable for ResultGrid {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ResultGrid {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let set = self.model.active_set();
        let rows = set.map_or(0, |set| set.rows.len());
        let columns = set.map_or(0, |set| set.columns.len());
        let status = set.map_or_else(
            || "No results".to_string(),
            |set| {
                let count = set.row_count.map_or_else(
                    || "streaming".to_string(),
                    |count| format!("{count} rows returned"),
                );
                format!(
                    "{count} · {rows} retained · {columns} columns{}{}",
                    if set.partial { " · partial" } else { "" },
                    if set.limit.is_some() {
                        " · row limit"
                    } else {
                        ""
                    }
                )
            },
        );
        let header = (columns > 0).then(|| self.render_header(self.visible_columns()));
        let selected_value = self
            .views
            .get(self.model.active)
            .and_then(|view| view.head)
            .and_then(|(row, column)| set?.rows.get(row)?.get(column))
            .map(cell_text);
        div()
            .id("result-grid")
            .role(Role::Table)
            .aria_label("Query results")
            .when_some(selected_value, |grid, value| {
                grid.aria_value(value.to_owned())
            })
            .aria_row_count(rows)
            .aria_column_count(columns)
            .key_context("ResultGrid")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| this.move_cell(-1, 0, false, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| this.move_cell(1, 0, false, cx)))
            .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.move_cell(0, -1, false, cx)))
            .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.move_cell(0, 1, false, cx)))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_cell(-1, 0, true, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_cell(1, 0, true, cx)))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.move_cell(0, -1, true, cx)))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.move_cell(0, 1, true, cx)))
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(rgb(0x000000))
            .text_color(rgb(0xffffff))
            .text_sm()
            .child(
                div()
                    .h(ROW_HEIGHT)
                    .flex_shrink_0()
                    .px_2()
                    .border_b_1()
                    .border_color(rgb(0x333333))
                    .child(status),
            )
            .children(header)
            .child(
                uniform_list(
                    "rows",
                    rows,
                    cx.processor(|this, rows: Range<usize>, _, cx| this.render_rows(rows, cx)),
                )
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .track_scroll(self.scroll())
                .flex_1()
                .min_h_0(),
            )
    }
}
