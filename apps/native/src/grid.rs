//! Read-only results, virtualized by row and column. Layout switching reuses
//! this entity; per-result selection and scroll handles survive reflow.

mod frozen;
mod keyboard;
mod navigation;
mod render;

use dbunk_lib::backend::data::BrowseTableResult;
use std::{cell::Cell, ops::Range, rc::Rc};

use dbunk_lib::backend::QueryEvent;
use gpui::{
    App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    ListHorizontalSizingBehavior, MouseButton, MouseDownEvent, Pixels, Role, ScrollStrategy,
    SharedString, Subscription, UniformListScrollHandle, Window, actions, div, prelude::*, px,
    uniform_list,
};

use crate::grid_columns::{ColumnAction, GridColumns};
use crate::result_export::{self, Completeness, ExportTable, Format, Options, Scope, SqlTarget};
use crate::results::{ResultModel, TerminalStatus, cell_text, display_text};
use crate::value_inspector::{Inspection, ValueInspector};

actions!(
    native,
    [
        CopyCells,
        ExportCells,
        AutoFitColumn,
        AutoFitAllColumns,
        ToggleColumnPin,
        CopyCsv,
        CopyJson,
        CopySql,
        CopyMarkdown,
        CopyHtml,
        CopyTxt,
        SelectAllCells,
        SelectCurrentRow,
        GoToRow,
        InspectCell,
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

struct GridView {
    scroll: UniformListScrollHandle,
    anchor: Option<(usize, usize)>,
    head: Option<(usize, usize)>,
    pinned_left: Cell<f32>,
}

impl Default for GridView {
    fn default() -> Self {
        Self {
            scroll: UniformListScrollHandle::new(),
            anchor: None,
            head: None,
            pinned_left: Cell::new(0.),
        }
    }
}

pub struct ResultGrid {
    navigation: Option<Entity<navigation::GoToRowView>>,
    navigation_events: Option<Subscription>,
    export_host: Option<std::sync::Arc<crate::controller::Host>>,
    exporter: Option<Entity<crate::export_view::ExportView>>,
    export_events: Option<Subscription>,
    export_focus: FocusHandle,
    model: ResultModel,
    table: Option<Rc<BrowseTableResult>>,
    views: Vec<GridView>,
    empty_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    sortable: bool,
    columns: GridColumns,
    sql_target: Option<(String, String)>,
    copy_status: Option<String>,
    inspection_budget: Rc<Cell<usize>>,
    inspector: Option<Entity<ValueInspector>>,
    inspector_events: Option<Subscription>,
}

pub enum GridEvent {
    Sort { column: String, append: bool },
    Preferences(crate::browse_preferences::PreferencePatch),
}
impl EventEmitter<GridEvent> for ResultGrid {}

impl ResultGrid {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            navigation: None,
            navigation_events: None,
            export_host: None,
            exporter: None,
            export_events: None,
            export_focus: cx.focus_handle(),
            model: ResultModel::default(),
            table: None,
            views: Vec::new(),
            empty_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            sortable: false,
            columns: GridColumns::default(),
            sql_target: None,
            copy_status: None,
            inspection_budget: Rc::new(Cell::new(0)),
            inspector: None,
            inspector_events: None,
        }
    }

    pub fn new_table(schema: String, table: String, cx: &mut Context<Self>) -> Self {
        let mut grid = Self::new(cx);
        grid.sortable = true;
        grid.sql_target = Some((schema, table));
        grid
    }

    pub fn set_inspection_budget(&mut self, budget: Rc<Cell<usize>>) {
        assert!(
            self.inspector.is_none() && self.navigation.is_none(),
            "Install the workspace budget before inspecting"
        );
        self.inspection_budget = budget;
    }

    pub fn set_export_host(&mut self, host: std::sync::Arc<crate::controller::Host>) {
        self.export_host = Some(host);
    }

    pub fn model(&self) -> &ResultModel {
        &self.model
    }
    pub fn selected_rows(&self) -> Option<Range<usize>> {
        self.selection().map(|(rows, _)| rows)
    }
    pub fn selected_cell(&self) -> Option<(usize, usize)> {
        let (row, column) = self.views.get(self.model.active)?.head?;
        Some((row, self.source_column(column)?))
    }
    pub fn load_columns(
        &mut self,
        prefs: Option<dbunk_lib::backend::data::TableGridPrefs>,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let mut columns = self.columns.clone();
        columns.load(prefs)?;
        self.set_columns(columns, cx);
        Ok(())
    }
    pub fn change_columns(
        &self,
        action: ColumnAction,
    ) -> Result<crate::browse_preferences::PreferencePatch, &'static str> {
        self.columns.patch(
            self.views
                .get(self.model.active)
                .and_then(|view| view.head)
                .map(|(_, column)| column),
            action,
        )
    }
    pub fn auto_fit(&mut self, all: bool, cx: &mut Context<Self>) {
        let selected = self
            .views
            .get(self.model.active)
            .and_then(|view| view.head)
            .map(|(_, col)| col);
        let range = if all {
            0..self.column_count()
        } else if let Some(column) = selected {
            column..column + 1
        } else {
            self.copy_status = Some("Select a column to auto-fit".into());
            cx.notify();
            return;
        };
        if let Some(page) = &self.table {
            // Preflight names before allocating the patch. Its eventual merge
            // also checks the complete latest preference record atomically.
            let bytes = range.clone().try_fold(0usize, |size, display| {
                let source = self.columns.source(display)?;
                size.checked_add(
                    page.columns
                        .get(source)?
                        .name
                        .len()
                        .saturating_mul(6)
                        .saturating_add(64),
                )
            });
            if bytes.is_none_or(|bytes| bytes > crate::browse_preferences::PREFS_BYTES) {
                self.copy_status = Some("Auto-fit column names exceed the preference limit".into());
                cx.notify();
                return;
            }
            let widths = range
                .filter_map(|display| {
                    let source = self.columns.source(display)?;
                    let name = &page.columns.get(source)?.name;
                    let width = crate::column_widths::fit(
                        name,
                        page.rows
                            .iter()
                            .map(|row| row.get(source).and_then(Option::as_deref)),
                        crate::column_widths::AUTO_FIT_MAX,
                    );
                    Some((name.clone(), width))
                })
                .collect::<Vec<_>>();
            if !widths.is_empty() {
                cx.emit(GridEvent::Preferences(
                    crate::browse_preferences::PreferencePatch::AutoFit(widths),
                ));
            }
        } else if let Some(set) = self.model.sets.get_mut(self.model.active) {
            for display in range {
                let Some(column) = set.widths.source(display) else {
                    continue;
                };
                let name = crate::column_widths::heading(set.columns[column].as_deref(), column);
                let width = crate::column_widths::fit(
                    &name,
                    set.rows
                        .iter()
                        .map(|row| row.get(column).and_then(Option::as_deref)),
                    crate::column_widths::AUTO_FIT_MAX,
                );
                set.widths.set_explicit(column, width);
            }
            set.widths.rebuild_offsets();
            self.copy_status = Some("Columns fit to retained rows".into());
            self.clamp_horizontal_scroll();
            self.reveal_selected_column();
        }
        cx.notify();
    }
    fn toggle_column_pin(&mut self, _: &ToggleColumnPin, _: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(self.model.active) else {
            return;
        };
        let Some((_, selected)) = view.head else {
            self.copy_status = Some("Select a column to pin or unpin".into());
            cx.notify();
            return;
        };
        if self.table.is_some() {
            match self.change_columns(ColumnAction::TogglePin) {
                Ok(patch) => cx.emit(GridEvent::Preferences(patch)),
                Err(error) => self.copy_status = Some(error.into()),
            }
        } else if let Some(set) = self.model.sets.get_mut(self.model.active) {
            let anchor = view
                .anchor
                .and_then(|(row, display)| set.widths.source(display).map(|source| (row, source)));
            let head = view
                .head
                .and_then(|(row, display)| set.widths.source(display).map(|source| (row, source)));
            if set.widths.toggle_pin(selected) {
                let view = &mut self.views[self.model.active];
                view.anchor = anchor.and_then(|(row, source)| {
                    set.widths.display(source).map(|display| (row, display))
                });
                view.head = head.and_then(|(row, source)| {
                    set.widths.display(source).map(|display| (row, display))
                });
                self.copy_status = None;
                self.clamp_horizontal_scroll();
                self.reveal_selected_column();
            }
        }
        cx.notify();
    }
    pub fn set_columns(&mut self, columns: GridColumns, cx: &mut Context<Self>) {
        for view in &mut self.views {
            let remap = |(row, display)| {
                self.columns
                    .source(display)
                    .and_then(|source| columns.display(source))
                    .map(|display| (row, display))
            };
            view.anchor = view.anchor.and_then(remap);
            view.head = view.head.and_then(remap);
            if view.anchor.is_none() || view.head.is_none() {
                view.anchor = None;
                view.head = None;
            }
        }
        self.columns = columns;
        self.clamp_horizontal_scroll();
        self.reveal_selected_column();
        cx.notify();
    }
    /// Restores one source cell of the current table page; hidden columns refuse.
    pub fn select_source_cell(
        &mut self,
        row: usize,
        source: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.table.is_none()
            || row >= self.row_count()
            || self.views.get(self.model.active).is_none()
        {
            return false;
        }
        let Some(display) = self.columns.display(source) else {
            return false;
        };
        self.select(row, display, false, cx);
        self.scroll().scroll_to_item(row, ScrollStrategy::Nearest);
        self.reveal_selected_column();
        true
    }
    pub fn selected_column(&self) -> Option<String> {
        let column = self
            .views
            .get(self.model.active)?
            .head
            .map_or(0, |(_, column)| column);
        self.column_name(column).map(str::to_owned)
    }

    pub fn inspector_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.navigation
            .as_ref()
            .is_some_and(|view| view.read(cx).contains_focus(window, cx))
            || self
                .exporter
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
            || self
                .inspector
                .as_ref()
                .is_some_and(|inspector| inspector.focus_handle(cx).contains_focused(window, cx))
    }

    pub fn pane_focus(&self, cx: &App) -> FocusHandle {
        if let Some(view) = &self.navigation {
            return view.focus_handle(cx);
        }
        self.exporter
            .as_ref()
            .map_or_else(|| self.focus.clone(), |view| view.read(cx).focus())
    }
    pub fn content_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
            || self
                .navigation
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
            || self
                .exporter
                .as_ref()
                .is_some_and(|view| view.read(cx).contains_focus(window, cx))
    }
    pub fn export_focus(&self) -> Option<FocusHandle> {
        self.export_host.as_ref().map(|_| self.export_focus.clone())
    }

    pub fn begin(&mut self, cx: &mut Context<Self>) {
        self.navigation = None;
        self.navigation_events = None;
        self.inspector = None;
        self.inspector_events = None;
        self.model = ResultModel::default();
        self.table = None;
        self.views.clear();
        self.copy_status = None;
        cx.notify();
    }

    pub fn table_page(&mut self, page: Rc<BrowseTableResult>, cx: &mut Context<Self>) {
        self.navigation = None;
        self.navigation_events = None;
        self.inspector = None;
        self.inspector_events = None;
        self.model = ResultModel::default();
        self.columns.table_columns(&page);
        self.table = Some(page);
        self.copy_status = None;
        self.views = vec![GridView::default()];
        self.sortable = true;
        cx.notify();
    }

    fn row_count(&self) -> usize {
        self.table.as_ref().map_or_else(
            || self.model.active_set().map_or(0, |set| set.rows.len()),
            |page| page.rows.len(),
        )
    }
    fn column_count(&self) -> usize {
        self.table.as_ref().map_or_else(
            || self.model.active_set().map_or(0, |set| set.columns.len()),
            |_| self.columns.len(),
        )
    }
    fn source_column(&self, display: usize) -> Option<usize> {
        if self.table.is_some() {
            self.columns.source(display)
        } else {
            self.model.active_set()?.widths.source(display)
        }
    }
    fn column_width(&self, display: usize) -> Pixels {
        if self.table.is_some() {
            px(self.columns.width(display))
        } else {
            self.model
                .active_set()
                .map_or(px(0.), |set| px(set.widths.width(display)))
        }
    }
    fn column_left(&self, display: usize) -> Pixels {
        if self.table.is_some() {
            px(self.columns.offset(display))
        } else {
            self.model
                .active_set()
                .map_or(px(0.), |set| px(set.widths.offset(display)))
        }
    }
    fn column_name(&self, index: usize) -> Option<&str> {
        match &self.table {
            Some(page) => page
                .columns
                .get(self.columns.source(index)?)
                .map(|column| column.name.as_str()),
            None => self
                .model
                .active_set()?
                .columns
                .get(self.source_column(index)?)?
                .as_deref(),
        }
    }
    fn row(&self, index: usize) -> Option<&[Option<String>]> {
        match &self.table {
            Some(page) => page.rows.get(index).map(Vec::as_slice),
            None => self
                .model
                .active_set()?
                .rows
                .get(index)
                .map(|row| row.as_ref()),
        }
    }

    /// Consume before ACK. One paint notification per event, never per cell.
    pub fn consume(&mut self, event: QueryEvent, cx: &mut Context<Self>) -> bool {
        let retain = self.model.consume(event);
        self.views
            .resize_with(self.model.sets.len(), GridView::default);
        cx.notify();
        retain
    }

    pub fn consume_with_limit(
        &mut self,
        event: QueryEvent,
        limit: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        self.model.set_byte_limit(limit);
        self.consume(event, cx)
    }

    pub fn set_active(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.model.sets.len() {
            self.navigation = None;
            self.navigation_events = None;
            self.inspector = None;
            self.inspector_events = None;
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
        if self.table.is_some() {
            px(self.columns.total_width())
        } else {
            self.model
                .active_set()
                .map_or(px(0.), |set| px(set.widths.total_width()))
        }
    }

    fn scroll_left(&self) -> Pixels {
        -self.scroll().0.borrow().base_handle.offset().x
    }

    fn clamp_horizontal_scroll(&self) {
        let scroll = self.scroll().0.borrow();
        let viewport = scroll.base_handle.bounds().size.width;
        if viewport > px(0.) {
            let mut offset = scroll.base_handle.offset();
            offset.x = offset.x.max(-px(self.panes().scrolling_max)).min(px(0.));
            scroll.base_handle.set_offset(offset);
        }
    }

    fn pinned_count(&self) -> usize {
        if self.table.is_some() {
            self.columns.pinned_count()
        } else {
            self.model
                .active_set()
                .map_or(0, |set| set.widths.pinned_count())
        }
    }
    fn viewport_width(&self) -> f32 {
        let width = self.scroll().0.borrow().base_handle.bounds().size.width / px(1.);
        if width > 0. { width } else { 2400. }
    }
    fn panes(&self) -> frozen::Panes {
        let pinned = self.pinned_count();
        frozen::Panes::new(
            self.total_width() / px(1.),
            self.column_left(pinned) / px(1.),
            self.viewport_width(),
            pinned == self.column_count(),
        )
    }
    fn pinned_left(&self) -> f32 {
        self.views
            .get(self.model.active)
            .map_or(0., |view| view.pinned_left.get())
            .clamp(0., self.panes().pinned_max)
    }
    fn columns_in(&self, left: f32, width: f32) -> Range<usize> {
        if self.table.is_some() {
            self.columns.visible_range(left, width)
        } else {
            self.model
                .active_set()
                .map_or(0..0, |set| set.widths.visible_range(left, width))
        }
    }
    fn pinned_columns(&self) -> Range<usize> {
        let panes = self.panes();
        if panes.pinned_viewport <= 0. {
            return 0..0;
        }
        let range = self.columns_in(self.pinned_left(), panes.pinned_viewport);
        range.start.min(self.pinned_count())..range.end.min(self.pinned_count())
    }
    fn scrolling_columns(&self) -> Range<usize> {
        let panes = self.panes();
        if panes.scrolling_viewport <= 0. {
            return self.column_count()..self.column_count();
        }
        let range = self.columns_in(
            panes.pinned_width + self.scroll_left().max(px(0.)) / px(1.),
            panes.scrolling_viewport,
        );
        range.start.max(self.pinned_count())
            ..range.end.max(self.pinned_count()).min(self.column_count())
    }
    fn scroll_pinned(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panes = self.panes();
        let delta = event.delta.pixel_delta(ROW_HEIGHT);
        let horizontal = if event.modifiers.shift && delta.x == px(0.) {
            delta.y
        } else {
            delta.x
        } / px(1.);
        if panes.pinned_max <= 0. || horizontal == 0. {
            return;
        }
        if let Some(view) = self.views.get(self.model.active) {
            view.pinned_left
                .set((self.pinned_left() - horizontal).clamp(0., panes.pinned_max));
            cx.stop_propagation();
            cx.notify();
        }
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
        let (rows, columns) = (self.row_count(), self.column_count());
        if rows == 0 || columns == 0 {
            return;
        }
        let current = self.views[self.model.active].head;
        let (row, column) = current.unwrap_or((0, 0));
        let row = if current.is_some() {
            row.saturating_add_signed(dy).min(rows - 1)
        } else {
            0
        };
        let column = if current.is_some() {
            column.saturating_add_signed(dx).min(columns - 1)
        } else {
            0
        };
        self.select(row, column, extend, cx);
        self.scroll().scroll_to_item(row, ScrollStrategy::Nearest);
        self.reveal_selected_column();
    }

    fn reveal_selected_column(&self) {
        let Some(view) = self.views.get(self.model.active) else {
            return;
        };
        let Some((_, column)) = view.head else {
            return;
        };
        let panes = self.panes();
        let left = self.column_left(column) / px(1.);
        let width = self.column_width(column) / px(1.);
        if column < self.pinned_count() {
            view.pinned_left.set(frozen::reveal(
                left,
                width,
                self.pinned_left(),
                panes.pinned_viewport,
                panes.pinned_max,
            ));
        } else {
            let scroll = self.scroll().0.borrow();
            let mut offset = scroll.base_handle.offset();
            offset.x = -px(frozen::reveal(
                left - panes.pinned_width,
                width,
                -offset.x / px(1.),
                panes.scrolling_viewport,
                panes.scrolling_max,
            ));
            scroll.base_handle.set_offset(offset);
        }
    }

    fn inspect(&mut self, _: &InspectCell, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, source)) = self.selected_cell() else {
            return;
        };
        let Some(value) = self.row(row).and_then(|row| row.get(source)) else {
            return;
        };
        let display = self.views[self.model.active].head.unwrap().1;
        let column = format!(
            "{} · row {}",
            self.column_name(display).unwrap_or("Column name omitted"),
            row + 1
        );
        match Inspection::new(
            column,
            value,
            self.source_is_partial(),
            self.inspection_budget.clone(),
        ) {
            Ok(data) => {
                let inspector = cx.new(|cx| ValueInspector::new(data, cx));
                self.inspector_events = Some(cx.subscribe_in(
                    &inspector,
                    window,
                    |this, _, _: &crate::value_inspector::Close, window, cx| {
                        this.inspector = None;
                        this.inspector_events = None;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    },
                ));
                inspector.update(cx, |inspector, cx| inspector.focus(window, cx));
                self.inspector = Some(inspector);
            }
            Err(error) => self.copy_status = Some(format!("Inspection refused: {error}")),
        }
        cx.notify();
    }

    fn source_is_partial(&self) -> bool {
        self.table.as_ref().map_or_else(
            || {
                self.model
                    .active_set()
                    .is_none_or(|set| set.partial || set.omitted_rows > 0)
                    || self.model.retention_limited
                    || self.model.completion.as_ref().is_none_or(|done| {
                        done.status != TerminalStatus::Completed
                            || done.omitted_rows > 0
                            || done.omitted_metadata_bytes > 0
                            || !done.truncation_reasons.is_empty()
                    })
            },
            |page| page.omitted_rows > 0 || page.truncated_cells > 0,
        )
    }

    fn select_all(&mut self, _: &SelectAllCells, _: &mut Window, cx: &mut Context<Self>) {
        let (rows, columns) = (self.row_count(), self.column_count());
        if rows > 0 && columns > 0 {
            self.select(0, 0, false, cx);
            self.select(rows - 1, columns - 1, true, cx);
        }
    }

    fn select_current_row(&mut self, _: &SelectCurrentRow, _: &mut Window, cx: &mut Context<Self>) {
        let Some((row, _)) = self.selected_cell() else {
            return;
        };
        let columns = self.column_count();
        if columns > 0 {
            self.select(row, 0, false, cx);
            self.select(row, columns - 1, true, cx);
        }
    }

    fn go_to_row(&mut self, _: &GoToRow, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_count() == 0 || self.column_count() == 0 || self.navigation.is_some() {
            return;
        }
        let view = match navigation::GoToRowView::create(
            self.row_count(),
            self.inspection_budget.clone(),
            window,
            cx,
        ) {
            Ok(view) => view,
            Err(error) => {
                self.copy_status = Some(error.into());
                cx.notify();
                return;
            }
        };
        self.navigation_events = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &navigation::Finished, window, cx| {
                if let Some(row) = event.0 {
                    let column = this
                        .views
                        .get(this.model.active)
                        .and_then(|view| view.head)
                        .map_or(0, |(_, column)| column);
                    if row < this.row_count() && column < this.column_count() {
                        this.select(row, column, false, cx);
                        this.scroll().scroll_to_item(row, ScrollStrategy::Nearest);
                        this.reveal_selected_column();
                    }
                }
                this.navigation = None;
                this.navigation_events = None;
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        window.focus(&view.focus_handle(cx), cx);
        self.navigation = Some(view);
        cx.notify();
    }

    fn copy(&mut self, _: &CopyCells, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_as(Format::Tsv, cx);
    }

    /// Copy only the selected retained cells, in visible column order. Never
    /// clone the source strings or retrieve missing rows to fill a selection.
    fn with_export<T>(
        &self,
        allow_all: bool,
        operation: impl FnOnce(&ExportTable<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let (rows, columns) = self
            .selection()
            .or_else(|| allow_all.then(|| (0..self.row_count(), 0..self.column_count())))
            .ok_or("Select cells to copy")?;
        if rows.len() > 100_000
            || columns.len() > 1_024
            || rows.len().saturating_mul(columns.len()) > 1_000_000
        {
            return Err("Selection exceeds the bounded copy limits".into());
        }
        let headings = columns
            .clone()
            .map(|column| {
                self.column_name(column)
                    .ok_or_else(|| "Column name was omitted; refresh before copying".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sources = columns
            .map(|column| {
                self.source_column(column)
                    .ok_or_else(|| "Column selection is stale".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rows = rows
            .map(|row| {
                self.row(row)
                    .ok_or_else(|| "Row selection is stale".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let partial = self.source_is_partial();
        let table = ExportTable {
            columns: &headings,
            rows: &rows,
            source_columns: Some(&sources),
            completeness: if partial {
                Completeness::Partial
            } else {
                Completeness::Complete
            },
        };
        operation(&table)
    }
    fn selected_export(&self, format: Format) -> Result<result_export::PreparedExport, String> {
        self.with_export(false, |table| {
            let mut options = Options::new(format);
            options.scope = Scope::RetainedRows;
            options.null_as = "NULL";
            options.sql_target = Some(match &self.sql_target {
                Some((schema, table)) => SqlTarget {
                    schema: Some(schema),
                    table,
                },
                None => SqlTarget {
                    schema: None,
                    table: "table_name",
                },
            });
            result_export::export(table, &options).map_err(|error| error.to_string())
        })
    }
    fn export(&mut self, _: &ExportCells, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.export_host.clone() else {
            return;
        };
        match self.with_export(true, |table| {
            crate::export_view::Capture::new(
                table,
                self.sql_target.clone(),
                self.inspection_budget.clone(),
            )
        }) {
            Ok(capture) => {
                let view =
                    cx.new(|cx| crate::export_view::ExportView::new(capture, host, window, cx));
                self.export_events =
                    Some(cx.subscribe_in(&view, window, |this, _, _, window, cx| {
                        this.exporter = None;
                        this.export_events = None;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }));
                window.focus(&view.read(cx).focus(), cx);
                self.exporter = Some(view);
            }
            Err(error) => self.copy_status = Some(format!("Export refused: {error}")),
        }
        cx.notify();
    }

    fn copy_as(&mut self, format: Format, cx: &mut Context<Self>) {
        self.copy_status = Some(match self.selected_export(format) {
            Ok(export) => {
                let partial = export.completeness == Completeness::Partial;
                let rows = export.row_count;
                match String::from_utf8(export.bytes) {
                    Ok(text) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        format!(
                            "Copied {rows} selected rows{}",
                            if partial {
                                " from partial results; retained values only"
                            } else {
                                ""
                            }
                        )
                    }
                    Err(_) => "Copy failed: invalid text encoding".into(),
                }
            }
            Err(error) => format!("Copy refused: {error}"),
        });
        cx.notify();
    }
}

impl Focusable for ResultGrid {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.pane_focus(cx)
    }
}

impl Render for ResultGrid {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(view) = &self.navigation {
            return div().size_full().child(view.clone()).into_any_element();
        }
        if let Some(view) = &self.exporter {
            return div().size_full().child(view.clone()).into_any_element();
        }

        let export_weak = cx.entity().downgrade();
        let set = self.model.active_set();
        let rows = self.row_count();
        let columns = self.column_count();
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
        let status = self.table.as_ref().map_or(status, |page| {
            format!(
                "{rows} rows · {columns} columns{}",
                if page.omitted_rows > 0 || page.truncated_cells > 0 {
                    " · incomplete values"
                } else {
                    ""
                }
            )
        });
        let status = if self.pinned_count() > 0 {
            format!(
                "{status} · {} pinned{}",
                self.pinned_count(),
                if self.panes().pinned_max > 0. {
                    " · scroll over pins or use arrows to reveal"
                } else {
                    ""
                }
            )
        } else {
            status
        };
        let status = self
            .copy_status
            .as_ref()
            .map_or(status.clone(), |copy| format!("{status} · {copy}"));
        let header = (columns > 0).then(|| self.render_header(cx));
        let selected_value = self
            .views
            .get(self.model.active)
            .and_then(|view| view.head)
            .and_then(|(row, column)| self.row(row)?.get(self.source_column(column)?))
            .map(cell_text);
        div()
            .id("result-grid")
            .role(Role::Table)
            .aria_label(if self.sortable {
                "Table rows"
            } else {
                "Query results"
            })
            .when_some(selected_value, |grid, value| {
                grid.aria_value(value.to_owned())
            })
            .aria_row_count(rows)
            .aria_column_count(columns)
            .key_context("ResultGrid")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::jump_key))
            .on_action(cx.listener(|this, _: &AutoFitColumn, _, cx| this.auto_fit(false, cx)))
            .on_action(cx.listener(|this, _: &AutoFitAllColumns, _, cx| this.auto_fit(true, cx)))
            .on_action(cx.listener(Self::toggle_column_pin))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::export))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::select_current_row))
            .on_action(cx.listener(Self::go_to_row))
            .on_action(cx.listener(Self::inspect))
            .on_action(cx.listener(|this, _: &CopyCsv, _, cx| this.copy_as(Format::Csv, cx)))
            .on_action(cx.listener(|this, _: &CopyJson, _, cx| this.copy_as(Format::Json, cx)))
            .on_action(cx.listener(|this, _: &CopySql, _, cx| this.copy_as(Format::Sql, cx)))
            .on_action(
                cx.listener(|this, _: &CopyMarkdown, _, cx| this.copy_as(Format::Markdown, cx)),
            )
            .on_action(cx.listener(|this, _: &CopyHtml, _, cx| this.copy_as(Format::Html, cx)))
            .on_action(cx.listener(|this, _: &CopyTxt, _, cx| this.copy_as(Format::Txt, cx)))
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
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_sm()
            .child(
                div()
                    .h(ROW_HEIGHT)
                    .flex_shrink_0()
                    .px_2()
                    .border_b_1()
                    .border_color(crate::style::line())
                    .id("grid-status")
                    .role(Role::Label)
                    .aria_label(status.clone())
                    .child(status),
            )
            .when(self.export_host.is_some(), |root| {
                root.child(
                    div()
                        .id("open-result-export")
                        .role(Role::Button)
                        .aria_label("Export selected cells, or all retained rows when no selection")
                        .track_focus(&self.export_focus)
                        .tab_stop(true)
                        .tab_index(0)
                        .focus(|s| s.bg(crate::style::hover()))
                        .px_2()
                        .py_1()
                        .child("Export retained rows")
                        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                            export_weak
                                .update(cx, |this, cx| this.export(&ExportCells, window, cx))
                                .ok();
                        })
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.export(&ExportCells, window, cx)
                            }),
                        ),
                )
            })
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
            .children(self.inspector.clone())
            .into_any_element()
    }
}
