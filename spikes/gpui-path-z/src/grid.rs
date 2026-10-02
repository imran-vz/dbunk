//! A result grid virtualized in both directions: `uniform_list` draws only
//! the rows in view, and each row draws only the columns in view.

use std::collections::HashMap;
use std::ops::Range;

use editor::Editor;
use gpui::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, ListHorizontalSizingBehavior,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Role, SharedString,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, uniform_list,
};
use theme::ActiveTheme;

use crate::results::ResultModel;

actions!(spike, [CopyCells, CommitCell, EditCell]);

const ROW_HEIGHT: Pixels = px(24.);
const DEFAULT_COLUMN_WIDTH: Pixels = px(132.);
const MIN_COLUMN_WIDTH: Pixels = px(48.);
const RESIZE_HANDLE_WIDTH: Pixels = px(6.);

struct Resize {
    column: usize,
    start_x: Pixels,
    start_width: Pixels,
}

/// A specialized editor open over one cell (ADR-0014). Its text only reaches
/// the grid as a pending edit; nothing is written to the model.
struct CellEditor {
    row: usize,
    column: usize,
    editor: Entity<Editor>,
    accessible: Entity<crate::accessible_editor::AccessibleEditor>,
}

pub struct ResultGrid {
    model: ResultModel,
    /// Staged values, keyed by row and column.
    pending: HashMap<(usize, usize), SharedString>,
    cell_editor: Option<CellEditor>,
    widths: Vec<Pixels>,
    /// `offsets[c]` is the left edge of column `c`; the last entry is the
    /// total width.
    offsets: Vec<Pixels>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    anchor: Option<(usize, usize)>,
    head: Option<(usize, usize)>,
    resize: Option<Resize>,
}

impl ResultGrid {
    /// Returning to results restores an open cell editor, if there is one.
    pub fn pane_focus(&self, cx: &App) -> FocusHandle {
        self.cell_editor
            .as_ref()
            .map_or_else(|| self.focus.clone(), |open| open.editor.focus_handle(cx))
    }

    fn edit_cell(&mut self, _: &EditCell, window: &mut Window, cx: &mut Context<Self>) {
        let (row, column) = self.head.unwrap_or((0, 0));
        self.open_cell_editor(row, column, window, cx);
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            model: ResultModel::default(),
            pending: HashMap::new(),
            cell_editor: None,
            widths: Vec::new(),
            offsets: vec![px(0.)],
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            anchor: None,
            head: None,
            resize: None,
        }
    }

    pub fn begin(&mut self, columns: Vec<SharedString>, cx: &mut Context<Self>) {
        self.widths = vec![DEFAULT_COLUMN_WIDTH; columns.len()];
        self.model = ResultModel::start(columns);
        self.pending.clear();
        self.cell_editor = None;
        self.anchor = None;
        self.head = None;
        self.relayout_columns();
        cx.notify();
    }

    /// One notification per batch, never per row.
    pub fn push(&mut self, batch: Vec<Vec<String>>, cx: &mut Context<Self>) {
        self.model.push_batch(batch);
        cx.notify();
    }

    pub fn finish(&mut self, cx: &mut Context<Self>) {
        self.model.complete = true;
        cx.notify();
    }

    fn relayout_columns(&mut self) {
        let mut offsets = Vec::with_capacity(self.widths.len() + 1);
        let mut edge = px(0.);
        offsets.push(edge);
        for width in &self.widths {
            edge += *width;
            offsets.push(edge);
        }
        self.offsets = offsets;
    }

    fn total_width(&self) -> Pixels {
        *self.offsets.last().unwrap_or(&px(0.))
    }

    /// Horizontal scroll position, as a non-negative distance.
    fn scroll_left(&self) -> Pixels {
        -self.scroll.0.borrow().base_handle.offset().x
    }

    /// Columns that intersect the viewport, plus one on each side so a
    /// column never appears half a frame late.
    fn visible_columns(&self) -> Range<usize> {
        let count = self.widths.len();
        if count == 0 {
            return 0..0;
        }
        let left = self.scroll_left();
        let viewport = self.scroll.0.borrow().base_handle.bounds().size.width;
        // Before the first layout the viewport is unknown; draw a screenful.
        let right = left
            + if viewport > px(0.) {
                viewport
            } else {
                px(2400.)
            };
        let first = self
            .offsets
            .partition_point(|edge| *edge <= left)
            .saturating_sub(1);
        let last = self
            .offsets
            .partition_point(|edge| *edge < right)
            .min(count);
        first.saturating_sub(1)..(last + 1).min(count)
    }

    fn selection(&self) -> Option<(Range<usize>, Range<usize>)> {
        let (anchor, head) = (self.anchor?, self.head?);
        Some((
            anchor.0.min(head.0)..anchor.0.max(head.0) + 1,
            anchor.1.min(head.1)..anchor.1.max(head.1) + 1,
        ))
    }

    fn select(&mut self, row: usize, column: usize, extend: bool, cx: &mut Context<Self>) {
        if !extend || self.anchor.is_none() {
            self.anchor = Some((row, column));
        }
        self.head = Some((row, column));
        cx.notify();
    }

    fn open_cell_editor(
        &mut self,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cell) = self.model.rows.get(row).and_then(|cells| cells.get(column)) else {
            return;
        };
        let value = self
            .pending
            .get(&(row, column))
            .cloned()
            .unwrap_or_else(|| cell.value.clone());
        let language = crate::sql::json_language(cx).ok();
        let editor = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_text(value.to_string(), window, cx);
            if let Some(buffer) = editor.buffer().read(cx).as_singleton() {
                buffer.update(cx, |buffer, cx| buffer.set_language(language, cx));
            }
            editor
        });
        window.focus(&editor.focus_handle(cx), cx);
        let accessible = cx.new(|cx| {
            crate::accessible_editor::AccessibleEditor::new(editor.clone(), "Cell editor", cx)
        });
        self.cell_editor = Some(CellEditor {
            row,
            column,
            editor,
            accessible,
        });
        cx.notify();
    }

    fn commit_cell(&mut self, _: &CommitCell, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.cell_editor.take() else {
            return;
        };
        let text = open.editor.read(cx).text(cx);
        self.pending.insert((open.row, open.column), text.into());
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn cancel_cell(
        &mut self,
        _: &editor::actions::Cancel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cell_editor.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Copies the selection as tab-separated text, from the full values.
    fn copy(&mut self, _: &CopyCells, _window: &mut Window, cx: &mut Context<Self>) {
        let Some((rows, columns)) = self.selection() else {
            return;
        };
        let mut text = String::new();
        for row in rows {
            let Some(cells) = self.model.rows.get(row) else {
                break;
            };
            let line = columns
                .clone()
                .filter_map(|column| cells.get(column))
                .map(|cell| cell.value.as_ref())
                .collect::<Vec<_>>()
                .join("\t");
            text.push_str(&line);
            text.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn drag(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(resize) = &self.resize else {
            return;
        };
        let width =
            (resize.start_width + (event.position.x - resize.start_x)).max(MIN_COLUMN_WIDTH);
        self.widths[resize.column] = width;
        self.relayout_columns();
        cx.notify();
    }

    fn render_header(&self, columns: Range<usize>, cx: &Context<Self>) -> impl IntoElement + use<> {
        let colors = cx.theme().colors();
        let border = colors.border;
        div()
            .h(ROW_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .overflow_hidden()
            .bg(colors.title_bar_background)
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .h_full()
                    .w(self.total_width())
                    // The header follows the body's horizontal scroll.
                    .ml(-self.scroll_left())
                    .child(div().flex_shrink_0().w(self.offsets[columns.start]))
                    .children(columns.map(|column| {
                        let width = self.widths[column];
                        div()
                            .id(("header", column))
                            .role(Role::ColumnHeader)
                            .aria_label(self.model.columns[column].clone())
                            .aria_column_index(column + 1)
                            .flex()
                            .flex_row()
                            .flex_shrink_0()
                            .w(width)
                            .h_full()
                            .border_r_1()
                            .border_color(border)
                            .child(
                                div()
                                    .flex_1()
                                    .px_2()
                                    .truncate()
                                    .child(self.model.columns[column].clone()),
                            )
                            .child(
                                div()
                                    .w(RESIZE_HANDLE_WIDTH)
                                    .h_full()
                                    .flex_shrink_0()
                                    .cursor_col_resize()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                            this.resize = Some(Resize {
                                                column,
                                                start_x: event.position.x,
                                                start_width: width,
                                            });
                                            cx.stop_propagation();
                                        }),
                                    ),
                            )
                    })),
            )
    }

    fn render_rows(
        &mut self,
        rows: Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let colors = cx.theme().colors();
        let border = colors.border_variant;
        let selected_background = colors.element_selected;
        let pending_color = cx.theme().status().modified;
        let columns = self.visible_columns();
        let selection = self.selection();
        let total_width = self.total_width();
        let left = self.offsets[columns.start];
        let column_count = self.widths.len().max(1);
        rows.filter_map(|row| {
            let cells = self.model.rows.get(row)?.clone();
            Some(
                div()
                    .id(("row", row))
                    .role(Role::Row)
                    .aria_row_index(row + 1)
                    .flex()
                    .flex_row()
                    .h(ROW_HEIGHT)
                    .w(total_width)
                    .border_b_1()
                    .border_color(border)
                    .child(div().flex_shrink_0().w(left))
                    .children(columns.clone().map(|column| {
                        let selected = selection.as_ref().is_some_and(|(rows, columns)| {
                            rows.contains(&row) && columns.contains(&column)
                        });
                        div()
                            // Assistive technology gets the grid as a table:
                            // every drawn cell has a role, a position and
                            // its text as the label.
                            .id(("cell", row * column_count + column))
                            .role(Role::Cell)
                            .aria_label(cells[column].display.clone())
                            .aria_column_index(column + 1)
                            .aria_selected(selected)
                            .flex_shrink_0()
                            .w(self.widths[column])
                            .h_full()
                            .px_2()
                            .border_r_1()
                            .border_color(border)
                            .truncate()
                            .when(selected, |cell| cell.bg(selected_background))
                            .map(|cell| match self.pending.get(&(row, column)) {
                                // A staged value is drawn in place of the stored one.
                                Some(staged) => {
                                    let end = staged
                                        .char_indices()
                                        .nth(crate::results::DISPLAY_CHARS)
                                        .map_or(staged.len(), |(end, _)| end);
                                    cell.text_color(pending_color)
                                        .child(SharedString::from(staged[..end].to_string()))
                                }
                                None => cell.child(cells[column].display.clone()),
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    if event.click_count >= 2 {
                                        this.open_cell_editor(row, column, window, cx);
                                        // The grid's own focus tracking runs
                                        // after this listener and would take
                                        // the focus back from the editor.
                                        cx.stop_propagation();
                                        return;
                                    }
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
        let header = (!self.model.columns.is_empty())
            .then(|| self.render_header(self.visible_columns(), cx));
        let colors = cx.theme().colors();
        let cell_editor = self.cell_editor.as_ref().map(|open| {
            div()
                .key_context("CellEditor")
                .absolute()
                .top(px(64.))
                .left(px(96.))
                .w(px(640.))
                .h(px(280.))
                .flex()
                .flex_col()
                .bg(colors.elevated_surface_background)
                .border_1()
                .border_color(colors.border_focused)
                .rounded_md()
                .shadow_lg()
                .child(
                    div()
                        .h(ROW_HEIGHT)
                        .px_2()
                        .flex_shrink_0()
                        .text_color(colors.text_muted)
                        .child(format!(
                            "{} · row {} · Cmd-S stages the value, Escape discards",
                            self.model.columns[open.column],
                            open.row + 1
                        )),
                )
                .child(div().flex_1().min_h_0().child(open.accessible.clone()))
        });
        let status: SharedString = if self.model.columns.is_empty() {
            "No result. Put the cursor in a statement and press Cmd-Enter.".into()
        } else {
            format!(
                "{} rows retained{}, {} columns, {:.1} MiB{}{}",
                self.model.rows.len(),
                if self.model.omitted_rows > 0 {
                    format!(" ({} omitted by limits)", self.model.omitted_rows)
                } else {
                    String::new()
                },
                self.model.columns.len(),
                self.model.retained_bytes as f64 / 1_048_576.,
                if self.model.complete {
                    ""
                } else {
                    ", streaming"
                },
                if self.pending.is_empty() {
                    String::new()
                } else {
                    format!(", {} staged edits", self.pending.len())
                },
            )
            .into()
        };
        div()
            .id("result-grid")
            .role(Role::Table)
            .aria_label("Query results")
            .aria_row_count(self.model.rows.len())
            .aria_column_count(self.model.columns.len())
            .key_context("ResultGrid")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::edit_cell))
            .on_action(cx.listener(Self::commit_cell))
            .on_action(cx.listener(Self::cancel_cell))
            .relative()
            .on_mouse_move(cx.listener(Self::drag))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.resize = None),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.editor_background)
            .text_color(colors.text)
            .text_sm()
            .child(
                div()
                    .h(ROW_HEIGHT)
                    .flex_shrink_0()
                    .px_2()
                    .text_color(colors.text_muted)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(status),
            )
            .children(header)
            .child(
                uniform_list(
                    "rows",
                    self.model.rows.len(),
                    cx.processor(|this, rows: Range<usize>, _window, cx| {
                        this.render_rows(rows, cx)
                    }),
                )
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .track_scroll(&self.scroll)
                .flex_1(),
            )
            .children(cell_editor)
    }
}
