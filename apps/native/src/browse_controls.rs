//! Browse controls emit candidate state; the table document owns admission,
//! paging and durable acknowledgements. Inspection always belongs to its page.
//!
//! Plan 032: a Drizzle-style filter bar (one row per ANDed condition) under
//! the table toolbar, plus Sort, History, Presets and Inspect popovers that
//! the toolbar anchors. Every applied change leaves as `BrowseEvent::Apply`.
mod model;
mod panels;
mod sort_panel;

pub use model::{HeaderSort, header_sort};

use crate::{
    accessible_editor::AccessibleEditor,
    browse_preferences::{BrowsePreferences, BrowseState, FilterMode, HistoryEntry, Preset},
    results::encoded_size,
    ui::popover::{self, AnchorSlot, MenuKey, MenuNav, Placement},
};
use dbunk_lib::backend::data::*;
use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, Bounds, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    KeyDownEvent, MouseDownEvent, Pixels, Role, ScrollHandle, SharedString, Stateful, Subscription,
    Window, div, prelude::*, px,
};
use model::{FilterDraft, FilterRowDraft, Operator, ROW_LIMIT, VALUE_BYTES, sort_edit};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

pub enum BrowseEvent {
    Apply(BrowseState, bool),
    Mode(FilterMode),
    SavePreset(Preset),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowsePanel {
    Sort,
    History,
    Presets,
    Inspect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectKind {
    Column,
    Operator,
}

#[derive(Clone, Debug)]
enum Action {
    Mode(FilterMode),
    AddRow,
    RemoveRow(usize),
    RemoveKept(usize),
    Select(usize, SelectKind),
    Choose(usize, SelectKind, usize),
    Apply,
    Clear,
    SortAppend(String),
    SortDirection(usize),
    SortNulls(usize),
    SortRemove(usize),
    SortUp(usize),
    SortDown(usize),
    SortClear,
    ApplyHistory(usize),
    ApplyPreset(usize),
    SavePreset,
    CopySql,
    CopyParams,
}

impl Action {
    /// Inspection reads the current page and stays usable while busy.
    fn always_available(&self) -> bool {
        matches!(self, Action::CopySql | Action::CopyParams)
    }
}

/// Height of one filter-bar row (§1).
const BAR_ROW: f32 = 26.;
/// Rows visible before the condition list scrolls.
const BAR_VISIBLE_ROWS: f32 = 6.;
const RAW_BYTES: usize = 65536;
const PRESET_NAME_BYTES: usize = 8192;
const INSPECTION_BYTES: usize = 256 * 1024;
const COLUMN_BUDGET: usize = 64 * 1024;

struct Field {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}

impl Field {
    fn new(label: &'static str, window: &mut Window, cx: &mut Context<BrowseControls>) -> Self {
        let editor = cx.new(|cx| Editor::single_line(window, cx));
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, false, cx));
        Self { editor, accessible }
    }

    fn len(&self, cx: &App) -> usize {
        self.editor.read(cx).buffer().read(cx).len(cx).0
    }

    fn text(&self, cx: &App) -> String {
        self.editor.read(cx).text(cx)
    }

    fn focused(&self, window: &Window, cx: &App) -> bool {
        self.editor.focus_handle(cx).contains_focused(window, cx)
    }

    fn composing(&self, window: &mut Window, cx: &mut Context<BrowseControls>) -> bool {
        self.editor.focus_handle(cx).is_focused(window)
            && self
                .editor
                .update(cx, |editor, cx| {
                    gpui::EntityInputHandler::marked_text_range(editor, window, cx)
                })
                .is_some()
    }
}

struct FilterRow {
    column: String,
    operator: Operator,
    value: Field,
    /// Column and operator trigger bounds, recorded each paint.
    anchors: [AnchorSlot; 2],
    _edited: Subscription,
}

#[derive(Clone)]
struct ColumnMeta {
    name: String,
    cast_type: String,
}

struct OpenSelect {
    row: usize,
    kind: SelectKind,
    nav: MenuNav,
    anchor: Bounds<Pixels>,
    scroll: ScrollHandle,
}

struct OpenPanel {
    kind: BrowsePanel,
    anchor: Bounds<Pixels>,
    nav: MenuNav,
    scroll: ScrollHandle,
    restore: Option<FocusHandle>,
}

pub struct BrowseControls {
    state: BrowseState,
    history: Vec<HistoryEntry>,
    presets: Vec<Preset>,
    columns: Vec<ColumnMeta>,
    bar_open: bool,
    rows: Vec<FilterRow>,
    /// The rows as loaded from `state`; the bar is dirty when they differ.
    base: FilterDraft,
    /// Applied typed filters the bar cannot edit as rows (raw SQL, values a
    /// row would rewrite, conditions beyond `ROW_LIMIT`). Re-sent on Apply.
    kept: Vec<BrowseFilter>,
    base_kept: Vec<BrowseFilter>,
    /// Rows need an editor per condition, so they are rebuilt with a window.
    rows_stale: bool,
    select: Option<OpenSelect>,
    panel: Option<OpenPanel>,
    select_focus: FocusHandle,
    panel_focus: FocusHandle,
    raw: Field,
    name: Field,
    search: Field,
    pending_raw: Option<String>,
    enabled: bool,
    page: Option<Rc<BrowseTableResult>>,
    message: Option<String>,
    focus: HashMap<SharedString, FocusHandle>,
    used: HashSet<SharedString>,
    tab_order: Vec<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<BrowseEvent> for BrowseControls {}

impl BrowseControls {
    pub fn new(state: BrowseState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let raw = Field::new("SQL filter", window, cx);
        let name = Field::new("Preset name", window, cx);
        let search = Field::new("Search columns to sort", window, cx);
        let subscriptions = vec![
            cx.subscribe(&raw.editor, |_, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    cx.notify();
                }
            }),
            cx.subscribe(&search.editor, |this, _, event: &EditorEvent, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    if let Some(panel) = &mut this.panel {
                        panel.nav.highlighted = 0;
                    }
                    cx.notify();
                }
            }),
        ];
        Self {
            pending_raw: Some(state.raw_filter_text.clone()),
            bar_open: !state.typed_filters.is_empty() || !state.raw_filter_text.is_empty(),
            state,
            history: vec![],
            presets: vec![],
            columns: vec![],
            rows: vec![],
            base: FilterDraft::default(),
            kept: vec![],
            base_kept: vec![],
            rows_stale: true,
            select: None,
            panel: None,
            select_focus: cx.focus_handle(),
            panel_focus: cx.focus_handle(),
            raw,
            name,
            search,
            enabled: false,
            page: None,
            message: None,
            focus: HashMap::new(),
            used: HashSet::new(),
            tab_order: vec![],
            _subscriptions: subscriptions,
        }
    }

    pub fn state(&self) -> &BrowseState {
        &self.state
    }

    /// Applied state from the table document. Condition rows are rebuilt only
    /// when the applied typed filters changed, so a sort or page-size change
    /// keeps unapplied filter drafts.
    pub fn set_state(&mut self, state: BrowseState, cx: &mut Context<Self>) {
        if self.state.typed_filters != state.typed_filters {
            self.rows_stale = true;
            self.select = None;
        }
        self.pending_raw = Some(state.raw_filter_text.clone());
        self.state = state;
        self.message = None;
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: FilterMode, cx: &mut Context<Self>) {
        self.state.filter_mode = mode;
        cx.notify();
    }

    pub fn preferences(&mut self, prefs: BrowsePreferences, cx: &mut Context<Self>) {
        self.history = prefs.history;
        self.presets = prefs.presets;
        cx.notify();
    }

    /// Disables every control and select while the table is busy; Inspect and
    /// its copy buttons stay available.
    pub fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if !enabled {
                self.select = None;
            }
            for editor in self.editors() {
                editor.update(cx, |editor, _| editor.set_read_only(!enabled));
            }
            cx.notify();
        }
    }

    pub fn page(&mut self, page: Option<Rc<BrowseTableResult>>, cx: &mut Context<Self>) {
        if let Some(page) = &page {
            let bytes = page
                .columns
                .iter()
                .map(|column| column.name.len() + column.cast_type.len())
                .sum::<usize>();
            if bytes <= COLUMN_BUDGET {
                self.columns = page
                    .columns
                    .iter()
                    .map(|column| ColumnMeta {
                        name: column.name.clone(),
                        cast_type: column.cast_type.clone(),
                    })
                    .collect();
            } else {
                self.columns.clear();
                self.message = Some("Column names exceed the filter control budget".into());
            }
            if self
                .select
                .as_ref()
                .is_some_and(|select| select.kind == SelectKind::Column)
            {
                self.select = None;
            }
        }
        self.page = page;
        cx.notify();
    }

    pub fn composition_active(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        [&self.raw, &self.name, &self.search]
            .into_iter()
            .chain(self.rows.iter().map(|row| &row.value))
            .any(|field| field.composing(window, cx))
    }

    pub fn focus_handles(&self, _: &App) -> Vec<FocusHandle> {
        self.tab_order.clone()
    }

    pub fn set_bar_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_rows(window, cx);
        if self.bar_open == open {
            return;
        }
        self.bar_open = open;
        if !open {
            self.select = None;
        } else if self.rows.is_empty()
            && self.kept.is_empty()
            && self.enabled
            && !self.columns.is_empty()
        {
            // Opening an empty bar starts one condition, as Drizzle does.
            self.add_condition(None, window, cx);
        } else if let Some(row) = self.rows.first() {
            row.value.editor.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    pub fn bar_open(&self) -> bool {
        self.bar_open
    }

    /// Opens the bar and appends a condition on `column` (else the previous
    /// row's column, else the first column), then focuses its value.
    pub fn add_condition(
        &mut self,
        column: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_rows(window, cx);
        self.bar_open = true;
        self.message = None;
        cx.notify();
        if !self.enabled {
            self.message = Some("Filters are unavailable while the table is busy".into());
            return;
        }
        if self.rows.len() >= ROW_LIMIT {
            self.message = Some(format!(
                "The filter bar holds at most {ROW_LIMIT} conditions"
            ));
            return;
        }
        let Some(column) = column
            .map(str::to_owned)
            .or_else(|| self.rows.last().map(|row| row.column.clone()))
            .or_else(|| self.columns.first().map(|column| column.name.clone()))
            .filter(|column| !column.is_empty())
        else {
            self.message = Some("Load a page before adding a filter".into());
            return;
        };
        let row = self.new_row(
            FilterRowDraft {
                column,
                operator: Operator::Eq,
                value: String::new(),
            },
            window,
            cx,
        );
        let focus = row.value.editor.focus_handle(cx);
        self.rows.push(row);
        focus.focus(window, cx);
    }

    /// Opens `panel` under `anchor`. Opening the panel that is already open
    /// at the same anchor closes it, so a toolbar trigger toggles.
    pub fn open_panel(
        &mut self,
        panel: BrowsePanel,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .panel
            .as_ref()
            .is_some_and(|open| open.kind == panel && open.anchor == anchor)
        {
            self.dismiss_panel(window, cx);
            return;
        }
        self.select = None;
        self.message = None;
        // Switching panels keeps the focus the first one will restore.
        let restore = self
            .panel
            .take()
            .and_then(|old| old.restore)
            .or_else(|| window.focused(cx));
        self.panel = Some(OpenPanel {
            kind: panel,
            anchor,
            nav: MenuNav::new(0),
            scroll: ScrollHandle::new(),
            restore,
        });
        if panel == BrowsePanel::Sort {
            self.search
                .editor
                .update(cx, |editor, cx| editor.set_text("", window, cx));
            self.search.editor.focus_handle(cx).focus(window, cx);
        } else {
            self.panel_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        if self.panel.take().is_some() {
            cx.notify();
        }
    }

    /// Applied typed filters plus the raw WHERE text counted as one.
    pub fn active_filter_count(&self) -> usize {
        self.state.typed_filters.len() + usize::from(!self.state.raw_filter_text.trim().is_empty())
    }

    pub fn sort_count(&self) -> usize {
        self.state.sort.len()
    }

    fn editors(&self) -> Vec<Entity<Editor>> {
        [&self.raw, &self.name, &self.search]
            .into_iter()
            .chain(self.rows.iter().map(|row| &row.value))
            .map(|field| field.editor.clone())
            .collect()
    }

    fn new_row(
        &mut self,
        draft: FilterRowDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> FilterRow {
        let value = Field::new("Filter value", window, cx);
        let enabled = self.enabled;
        let text = draft.value;
        value.editor.update(cx, |editor, cx| {
            if !text.is_empty() {
                editor.set_text(text, window, cx);
            }
            editor.set_read_only(!enabled);
        });
        let edited = cx.subscribe(&value.editor, |_, _, event: &EditorEvent, cx| {
            if matches!(event, EditorEvent::BufferEdited) {
                cx.notify();
            }
        });
        FilterRow {
            column: draft.column,
            operator: draft.operator,
            value,
            anchors: [popover::anchor_slot(), popover::anchor_slot()],
            _edited: edited,
        }
    }

    fn sync_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.rows_stale {
            return;
        }
        self.rows_stale = false;
        let (draft, kept) = FilterDraft::from_filters_capped(&self.state.typed_filters, ROW_LIMIT);
        let rows: Vec<FilterRow> = draft
            .rows
            .iter()
            .cloned()
            .map(|row| self.new_row(row, window, cx))
            .collect();
        self.rows = rows;
        self.base = draft;
        self.base_kept = kept.clone();
        self.kept = kept;
        self.select = None;
    }

    /// The rows as a draft; `None` when a value exceeds its byte budget, so
    /// an oversized buffer is never copied out of its editor.
    fn draft(&self, cx: &App) -> Option<FilterDraft> {
        let mut rows = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            if row.value.len(cx) > VALUE_BYTES {
                return None;
            }
            rows.push(FilterRowDraft {
                column: row.column.clone(),
                operator: row.operator,
                value: row.value.text(cx),
            });
        }
        Some(FilterDraft { rows })
    }

    fn dirty(&self, cx: &App) -> bool {
        if self.rows_stale {
            return false;
        }
        let raw = self.state.filter_mode == FilterMode::Raw
            && self.pending_raw.is_none()
            && (self.raw.len(cx) > RAW_BYTES || self.raw.text(cx) != self.state.raw_filter_text);
        raw || self.kept != self.base_kept
            || self
                .draft(cx)
                .is_none_or(|draft| draft.is_dirty(&self.base))
    }

    fn apply_bar(&mut self, cx: &mut Context<Self>) {
        let raw_mode = self.state.filter_mode == FilterMode::Raw;
        let raw_too_long = raw_mode && self.raw.len(cx) > RAW_BYTES;
        let Some(draft) = self.draft(cx).filter(|_| !raw_too_long) else {
            self.message =
                Some("Input exceeds the browse setting limit; shorten it before applying".into());
            return;
        };
        match draft.to_filters() {
            Err((index, error)) => {
                self.message = Some(format!("Condition {}: {error}", index + 1));
            }
            Ok(mut filters) => {
                filters.extend(self.kept.iter().cloned());
                let mut state = self.state.clone();
                state.typed_filters = filters;
                if raw_mode {
                    state.raw_filter_text = self.raw.text(cx);
                }
                cx.emit(BrowseEvent::Apply(state, true));
            }
        }
    }

    fn sort_candidates(&self, cx: &App) -> Vec<ColumnMeta> {
        let query = if self.search.len(cx) > 256 {
            String::new()
        } else {
            self.search.text(cx).trim().to_lowercase()
        };
        self.columns
            .iter()
            .filter(|column| !self.state.sort.iter().any(|key| key.column == column.name))
            .filter(|column| query.is_empty() || column.name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    fn dismiss_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(select) = self.select.take() {
            let id = trigger_id(select.row, select.kind);
            if let Some(focus) = self.focus.get(&id).cloned() {
                focus.focus(window, cx);
            }
            cx.notify();
        }
    }

    fn dismiss_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.panel.take() {
            if let Some(focus) = panel.restore {
                focus.focus(window, cx);
            }
            cx.notify();
        }
    }

    fn action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled && !action.always_available() {
            return;
        }
        self.message = None;
        if matches!(action, Action::SavePreset) && self.name.len(cx) > PRESET_NAME_BYTES {
            self.message =
                Some("Input exceeds the browse setting limit; shorten it before applying".into());
            cx.notify();
            return;
        }
        let mut state = self.state.clone();
        match action {
            Action::Mode(mode) => cx.emit(BrowseEvent::Mode(mode)),
            Action::AddRow => self.add_condition(None, window, cx),
            Action::RemoveRow(index) => {
                if index < self.rows.len() {
                    // Removing from a clean bar applies at once, as the old
                    // per-filter remove did; with other pending edits it
                    // only edits the draft.
                    let clean = !self.dirty(cx);
                    self.rows.remove(index);
                    self.select = None;
                    if clean {
                        self.apply_bar(cx);
                    }
                }
            }
            Action::RemoveKept(index) => {
                if index < self.kept.len() {
                    let clean = !self.dirty(cx);
                    self.kept.remove(index);
                    if clean {
                        self.apply_bar(cx);
                    }
                }
            }
            Action::Select(row, kind) => {
                if self
                    .select
                    .as_ref()
                    .is_some_and(|open| open.row == row && open.kind == kind)
                {
                    self.dismiss_select(window, cx);
                } else if let Some(target) = self.rows.get(row) {
                    let (slot, len, current) = match kind {
                        SelectKind::Column => (
                            &target.anchors[0],
                            self.columns.len(),
                            self.columns
                                .iter()
                                .position(|column| column.name == target.column),
                        ),
                        SelectKind::Operator => (
                            &target.anchors[1],
                            Operator::ALL.len(),
                            Some(target.operator.index()),
                        ),
                    };
                    if let Some(anchor) = slot.get()
                        && len > 0
                    {
                        let mut nav = MenuNav::new(len);
                        nav.highlighted = current.unwrap_or(0).min(len - 1);
                        let scroll = ScrollHandle::new();
                        scroll.scroll_to_item(nav.highlighted);
                        self.panel = None;
                        self.select = Some(OpenSelect {
                            row,
                            kind,
                            nav,
                            anchor,
                            scroll,
                        });
                        self.select_focus.focus(window, cx);
                    }
                }
            }
            Action::Choose(row, kind, item) => {
                if let Some(target) = self.rows.get_mut(row) {
                    match kind {
                        SelectKind::Column => {
                            if let Some(column) = self.columns.get(item) {
                                target.column = column.name.clone();
                            }
                        }
                        SelectKind::Operator => {
                            if let Some(operator) = Operator::ALL.get(item) {
                                target.operator = *operator;
                            }
                        }
                    }
                }
                self.dismiss_select(window, cx);
            }
            Action::Apply => self.apply_bar(cx),
            Action::Clear => {
                let applied = !state.typed_filters.is_empty() || !state.raw_filter_text.is_empty();
                self.rows.clear();
                self.kept.clear();
                self.select = None;
                if applied {
                    state.typed_filters.clear();
                    state.raw_filter_text.clear();
                    cx.emit(BrowseEvent::Apply(state, true));
                } else {
                    self.base = FilterDraft::default();
                    self.base_kept.clear();
                }
            }
            Action::SortAppend(column) => {
                if sort_edit::append(&mut state.sort, &column) {
                    cx.emit(BrowseEvent::Apply(state, true));
                } else if state.sort.len() >= model::SORT_LIMIT {
                    self.message = Some(format!(
                        "At most {} sort keys are supported",
                        model::SORT_LIMIT
                    ));
                }
            }
            Action::SortDirection(index)
            | Action::SortNulls(index)
            | Action::SortRemove(index)
            | Action::SortUp(index)
            | Action::SortDown(index) => {
                let changed = match action {
                    Action::SortDirection(_) => sort_edit::toggle_direction(&mut state.sort, index),
                    Action::SortNulls(_) => sort_edit::cycle_nulls(&mut state.sort, index),
                    Action::SortRemove(_) => sort_edit::remove(&mut state.sort, index),
                    Action::SortUp(_) => sort_edit::move_up(&mut state.sort, index),
                    _ => sort_edit::move_down(&mut state.sort, index),
                };
                if changed {
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::SortClear => {
                if !state.sort.is_empty() {
                    state.sort.clear();
                    cx.emit(BrowseEvent::Apply(state, true));
                }
            }
            Action::ApplyHistory(index) => {
                if let Some(entry) = self.history.get(index) {
                    cx.emit(BrowseEvent::Apply(entry.state(state.page_size), false));
                    self.dismiss_panel(window, cx);
                }
            }
            Action::ApplyPreset(index) => {
                if let Some(preset) = self.presets.get(index) {
                    cx.emit(BrowseEvent::Apply(preset.state.clone(), false));
                    self.dismiss_panel(window, cx);
                }
            }
            Action::SavePreset => {
                let name = self.name.text(cx).trim().to_owned();
                if name.is_empty() || name.len() > PRESET_NAME_BYTES {
                    self.message = Some("Preset name must contain 1 to 8192 bytes".into());
                } else {
                    cx.emit(BrowseEvent::SavePreset(Preset { name, state }));
                }
            }
            Action::CopySql | Action::CopyParams => {
                if let Some(page) = &self.page {
                    if encoded_size(&page.inspection) > INSPECTION_BYTES {
                        self.message =
                            Some("Query inspection exceeds 256 KiB; copy refused".into());
                    } else {
                        let text = if matches!(action, Action::CopySql) {
                            page.inspection.sql.clone()
                        } else {
                            parameters(&page.inspection.params)
                        };
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    }
                }
            }
        }
        cx.notify();
    }

    fn focus_for(&mut self, id: &SharedString, cx: &mut Context<Self>) -> FocusHandle {
        self.used.insert(id.clone());
        self.focus
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    /// Focus, tab order, click and AX click for one control.
    fn control(
        &mut self,
        id: SharedString,
        element: Stateful<Div>,
        action: Action,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focus = self.focus_for(&id, cx);
        if enabled {
            self.tab_order.push(focus.clone());
        }
        let weak = cx.weak_entity();
        let click = action.clone();
        element
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .on_click(cx.listener(move |this, _, window, cx| {
                if enabled {
                    this.action(click.clone(), window, cx);
                }
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                if enabled {
                    weak.update(cx, |this, cx| this.action(action.clone(), window, cx))
                        .ok();
                }
            })
    }

    fn available(&self, action: &Action, available: bool) -> bool {
        available && (self.enabled || action.always_available())
    }

    fn button(
        &mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: Option<&'static str>,
        action: Action,
        available: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let enabled = self.available(&action, available);
        let element = crate::ui::tool_button(id.clone(), label, icon, enabled, false);
        self.control(id, element, action, enabled, cx)
    }

    fn icon(
        &mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: &'static str,
        action: Action,
        available: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = id.into();
        let enabled = self.available(&action, available);
        let element = crate::ui::icon_button(id.clone(), label, icon, enabled);
        self.control(id, element, action, enabled, cx)
    }

    fn select_trigger(
        &mut self,
        row: usize,
        kind: SelectKind,
        value: SharedString,
        hint: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self
            .select
            .as_ref()
            .is_some_and(|select| select.row == row && select.kind == kind);
        let (label, width, slot, available) = match kind {
            SelectKind::Column => (
                format!("Condition {} column", row + 1),
                150.,
                self.rows[row].anchors[0].clone(),
                !self.columns.is_empty(),
            ),
            SelectKind::Operator => (
                format!("Condition {} operator", row + 1),
                130.,
                self.rows[row].anchors[1].clone(),
                true,
            ),
        };
        let id = trigger_id(row, kind);
        let action = Action::Select(row, kind);
        let enabled = self.available(&action, available);
        let element = popover::select_trigger(id.clone(), label, value, open, enabled)
            .w(px(width))
            .relative()
            .child(popover::probe(slot))
            .when(!hint.is_empty(), |trigger| {
                trigger
                    .tooltip(crate::ui::tooltip(hint))
                    .tooltip_show_delay(crate::ui::tooltip_delay())
            });
        self.control(id, element, action, enabled, cx)
            .into_any_element()
    }

    fn render_row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = &self.rows[index];
        let column = row.column.clone();
        let operator = row.operator;
        let accessible = row.value.accessible.clone();
        let value_focus = row.value.editor.focus_handle(cx);
        let cast = self
            .columns
            .iter()
            .find(|meta| meta.name == column)
            .map(|meta| meta.cast_type.clone())
            .unwrap_or_default();
        let remove = self.icon(
            format!("filter-remove-{index}"),
            format!("Remove condition {}", index + 1),
            "icons/close.svg",
            Action::RemoveRow(index),
            true,
            cx,
        );
        let column_trigger =
            self.select_trigger(index, SelectKind::Column, column.into(), cast.into(), cx);
        let operator_trigger = self.select_trigger(
            index,
            SelectKind::Operator,
            operator.label().into(),
            operator.sql_hint().into(),
            cx,
        );
        if operator.takes_value() {
            self.tab_order.push(value_focus);
        }
        div()
            .id(("filter-row", index))
            .role(Role::Group)
            .aria_label(format!("Condition {}", index + 1))
            .flex_none()
            .h(px(BAR_ROW))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(remove)
            .child(conjunction(index))
            .child(column_trigger)
            .child(operator_trigger)
            .when(operator.takes_value(), |row| {
                row.child(
                    crate::ui::field()
                        .w(px(220.))
                        .font_family(crate::style::MONO)
                        .child(accessible),
                )
            })
            .into_any_element()
    }

    fn render_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let dirty = self.dirty(cx);
        let mut rows = div()
            .id("filter-rows")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .max_h(px(BAR_ROW * BAR_VISIBLE_ROWS))
            .overflow_y_scroll();
        for index in 0..self.rows.len() {
            rows = rows.child(self.render_row(index, cx));
        }
        for (index, filter) in self.kept.clone().iter().enumerate() {
            let summary = filter_summary(filter);
            let remove = self.icon(
                format!("filter-kept-remove-{index}"),
                format!("Remove filter {summary}"),
                "icons/close.svg",
                Action::RemoveKept(index),
                true,
                cx,
            );
            rows = rows.child(
                div()
                    .id(("filter-kept", index))
                    .role(Role::Group)
                    .aria_label(format!("Filter not editable here: {summary}"))
                    .flex_none()
                    .h(px(BAR_ROW))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(remove)
                    .child(conjunction(self.rows.len() + index))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(crate::style::MONO)
                            .text_color(crate::style::dim())
                            .child(summary),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(crate::style::faint())
                            .child("not editable here"),
                    ),
            );
        }
        if self.rows.is_empty() && self.kept.is_empty() {
            rows = rows.child(
                div()
                    .h(px(BAR_ROW))
                    .flex()
                    .items_center()
                    .text_color(crate::style::faint())
                    .child(if self.columns.is_empty() {
                        "Load a page to filter by column"
                    } else {
                        "No conditions"
                    }),
            );
        }
        let raw_mode = self.state.filter_mode == FilterMode::Raw;
        let add = self.button(
            "browse-add-filter",
            "Add filter",
            Some("icons/plus.svg"),
            Action::AddRow,
            self.rows.len() < ROW_LIMIT && !self.columns.is_empty(),
            cx,
        );
        let apply = self.button(
            "browse-apply",
            "Apply",
            Some("icons/filter.svg"),
            Action::Apply,
            dirty,
            cx,
        );
        let clear = self.button(
            "browse-clear",
            "Clear",
            None,
            Action::Clear,
            !self.rows.is_empty()
                || !self.kept.is_empty()
                || !self.state.typed_filters.is_empty()
                || !self.state.raw_filter_text.is_empty(),
            cx,
        );
        let next_mode = if raw_mode {
            FilterMode::Typed
        } else {
            FilterMode::Raw
        };
        let sql = crate::ui::pressed(
            self.button(
                "browse-raw",
                "SQL",
                Some("icons/code.svg"),
                Action::Mode(next_mode),
                true,
                cx,
            ),
            raw_mode,
        )
        .aria_label(if raw_mode {
            "SQL WHERE filter: on"
        } else {
            "SQL WHERE filter: off"
        });
        let actions = div()
            .flex_none()
            .h(px(BAR_ROW))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(add)
            .child(apply)
            .child(clear)
            .child(sql);
        let raw_row = if raw_mode {
            self.tab_order.push(self.raw.editor.focus_handle(cx));
            Some(
                div()
                    .flex_none()
                    .h(px(BAR_ROW))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex_none()
                            .w(px(64.))
                            .text_color(crate::style::faint())
                            .child("SQL WHERE"),
                    )
                    .child(
                        crate::ui::field()
                            .flex_1()
                            .min_w(px(250.))
                            .font_family(crate::style::MONO)
                            .child(self.raw.accessible.clone()),
                    ),
            )
        } else if !self.state.raw_filter_text.trim().is_empty() {
            let text = self
                .state
                .raw_filter_text
                .chars()
                .take(160)
                .collect::<String>();
            Some(
                div()
                    .flex_none()
                    .h(px(BAR_ROW))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_color(crate::style::faint())
                    .child("and SQL")
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_family(crate::style::MONO)
                            .child(text),
                    ),
            )
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .px(px(8.))
            .py(px(2.))
            .text_sm()
            .text_color(crate::style::dim())
            .border_b_1()
            .border_color(crate::style::line_soft())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(rows)
                    .child(actions),
            )
            .children(raw_row)
            .children(self.message_line("browse-control-message"))
            .into_any_element()
    }

    fn message_line(&self, id: &'static str) -> Option<Stateful<Div>> {
        self.message.clone().map(|text| {
            div()
                .id(id)
                .role(Role::Status)
                .aria_label(text.clone())
                .py(px(2.))
                .text_color(crate::style::dim())
                .child(text)
        })
    }

    fn render_select(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let select = self.select.as_ref()?;
        let (row, kind, anchor, highlighted, scroll) = (
            select.row,
            select.kind,
            select.anchor,
            select.nav.highlighted,
            select.scroll.clone(),
        );
        let options: Vec<(SharedString, SharedString)> = match kind {
            SelectKind::Column => self
                .columns
                .iter()
                .map(|column| (column.name.clone().into(), column.cast_type.clone().into()))
                .collect(),
            SelectKind::Operator => Operator::ALL
                .iter()
                .map(|operator| (operator.label().into(), operator.sql_hint().into()))
                .collect(),
        };
        if let Some(select) = &mut self.select {
            select.nav.set_len(options.len());
        }
        self.tab_order.push(self.select_focus.clone());
        let label = match kind {
            SelectKind::Column => format!("Condition {} column", row + 1),
            SelectKind::Operator => format!("Condition {} operator", row + 1),
        };
        let mut items = div()
            .id("filter-select-items")
            .flex()
            .flex_col()
            .max_h(px(312.))
            .overflow_y_scroll()
            .track_scroll(&scroll);
        for (index, (label, hint)) in options.into_iter().enumerate() {
            let weak = cx.weak_entity();
            items = items.child(
                popover::item(
                    ("filter-option", index),
                    label,
                    None,
                    Some(hint),
                    index == highlighted,
                    true,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.action(Action::Choose(row, kind, index), window, cx)
                }))
                .on_a11y_action(
                    gpui::accesskit::Action::Click,
                    move |_, window, cx| {
                        weak.update(cx, |this, cx| {
                            this.action(Action::Choose(row, kind, index), window, cx)
                        })
                        .ok();
                    },
                ),
            );
        }
        let panel = popover::panel("filter-select", Role::ListBox, label)
            .track_focus(&self.select_focus)
            .flex()
            .flex_col()
            .w(px(if kind == SelectKind::Column {
                240.
            } else {
                220.
            }))
            .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                // The trigger's own click toggles; closing here would reopen it.
                if !anchor.contains(&event.position) {
                    this.select = None;
                    cx.notify();
                }
            }))
            .child(items);
        Some(popover::layer(anchor, Placement::Below, panel).into_any_element())
    }

    fn render_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.panel.as_ref()?;
        let (kind, anchor) = (open.kind, open.anchor);
        self.tab_order.push(self.panel_focus.clone());
        let (body, len, width, title) = match kind {
            BrowsePanel::Sort => {
                let (body, len) = self.sort_panel(cx);
                (body, len, 420., "Sort")
            }
            BrowsePanel::History => {
                let (body, len) = self.history_panel(cx);
                (body, len, 380., "Filter history")
            }
            BrowsePanel::Presets => {
                let (body, len) = self.presets_panel(cx);
                (body, len, 320., "Presets")
            }
            BrowsePanel::Inspect => (self.inspect_panel(cx), 0, 520., "Inspect query"),
        };
        if let Some(open) = &mut self.panel {
            open.nav.set_len(len);
        }
        let panel = popover::panel("browse-panel", Role::Dialog, title)
            .track_focus(&self.panel_focus)
            .flex()
            .flex_col()
            .w(px(width))
            .max_h(px(360.))
            .text_sm()
            .text_color(crate::style::dim())
            .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                // A click on the trigger reaches `open_panel`, which toggles.
                if !anchor.contains(&event.position) {
                    this.close_panel(cx);
                }
            }))
            .child(body)
            .children(
                self.message_line("browse-panel-message")
                    .map(|line| line.px(px(8.))),
            );
        Some(popover::layer(anchor, Placement::Below, panel).into_any_element())
    }

    /// Enter applies from a condition value or the SQL field; open popovers
    /// take arrow keys, Enter and Escape.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift {
            return;
        }
        if self.composition_active(window, cx) {
            return;
        }
        let key = event.keystroke.key.as_str();
        if self.select.is_some() && self.select_focus.is_focused(window) {
            let Some((row, kind, result, highlighted, scroll)) =
                self.select.as_mut().map(|select| {
                    let result = select.nav.key(key);
                    (
                        select.row,
                        select.kind,
                        result,
                        select.nav.highlighted,
                        select.scroll.clone(),
                    )
                })
            else {
                return;
            };
            match result {
                MenuKey::Moved => {
                    scroll.scroll_to_item(highlighted);
                    cx.notify();
                }
                MenuKey::Activate(index) => {
                    self.action(Action::Choose(row, kind, index), window, cx)
                }
                MenuKey::Dismiss => self.dismiss_select(window, cx),
                MenuKey::Ignored => return,
            }
            cx.stop_propagation();
            return;
        }
        if let Some(kind) = self.panel.as_ref().map(|panel| panel.kind)
            && self.panel_focus.contains_focused(window, cx)
        {
            if key == "escape" {
                self.dismiss_panel(window, cx);
                cx.stop_propagation();
                return;
            }
            let in_field = self.search.focused(window, cx) || self.name.focused(window, cx);
            if kind == BrowsePanel::Presets && key == "enter" && self.name.focused(window, cx) {
                self.action(Action::SavePreset, window, cx);
                cx.stop_propagation();
                return;
            }
            // Only the panel itself or its search field drives the list; a
            // focused button keeps its own Enter and Space activation.
            if kind == BrowsePanel::Inspect
                || !(self.panel_focus.is_focused(window) || in_field)
                || (in_field && matches!(key, "space" | "home" | "end"))
            {
                return;
            }
            let Some((result, highlighted, scroll)) = self.panel.as_mut().map(|panel| {
                let result = panel.nav.key(key);
                (result, panel.nav.highlighted, panel.scroll.clone())
            }) else {
                return;
            };
            match result {
                MenuKey::Moved => {
                    scroll.scroll_to_item(highlighted);
                    cx.notify();
                }
                MenuKey::Activate(index) => {
                    let action = match kind {
                        BrowsePanel::Sort => self
                            .sort_candidates(cx)
                            .into_iter()
                            .nth(index)
                            .map(|column| Action::SortAppend(column.name)),
                        BrowsePanel::History => Some(Action::ApplyHistory(index)),
                        BrowsePanel::Presets => Some(Action::ApplyPreset(index)),
                        BrowsePanel::Inspect => None,
                    };
                    if let Some(action) = action {
                        self.action(action, window, cx);
                    }
                }
                MenuKey::Dismiss => self.dismiss_panel(window, cx),
                MenuKey::Ignored => return,
            }
            cx.stop_propagation();
            return;
        }
        if key == "enter"
            && (self.raw.focused(window, cx)
                || self.rows.iter().any(|row| row.value.focused(window, cx)))
        {
            self.action(Action::Apply, window, cx);
            cx.stop_propagation();
        }
    }
}

impl Render for BrowseControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(raw) = self.pending_raw.take() {
            self.raw
                .editor
                .update(cx, |editor, cx| editor.set_text(raw, window, cx));
        }
        self.sync_rows(window, cx);
        self.tab_order.clear();
        self.used.clear();
        let bar = self.bar_open.then(|| self.render_bar(cx));
        let select = if self.bar_open {
            self.render_select(cx)
        } else {
            None
        };
        let panel = self.render_panel(cx);
        let used = &self.used;
        self.focus.retain(|id, _| used.contains(id));
        div()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.key_down(event, window, cx)
            }))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .children(bar)
            .children(select)
            .children(panel)
    }
}

fn trigger_id(row: usize, kind: SelectKind) -> SharedString {
    match kind {
        SelectKind::Column => format!("filter-column-{row}").into(),
        SelectKind::Operator => format!("filter-operator-{row}").into(),
    }
}

fn conjunction(index: usize) -> Div {
    div()
        .flex_none()
        .w(px(36.))
        .text_color(crate::style::faint())
        .child(if index == 0 { "where" } else { "and" })
}

fn filter_summary(filter: &BrowseFilter) -> String {
    let text = match filter {
        BrowseFilter::Comparison {
            column,
            operator,
            value,
        } => format!("{column} {operator:?} {value}"),
        BrowseFilter::TextMatch {
            column,
            operator,
            value,
        } => format!("{column} {operator:?} {value}"),
        BrowseFilter::IsNull { column } => format!("{column} IS NULL"),
        BrowseFilter::IsNotNull { column } => format!("{column} IS NOT NULL"),
        BrowseFilter::InList { column, values } => format!("{column} IN ({})", values.join(", ")),
        BrowseFilter::RawSql { text } => text.clone(),
    };
    if text.chars().count() > 180 {
        format!("{}…", text.chars().take(180).collect::<String>())
    } else {
        text
    }
}

fn parameters(params: &[InspectionParam]) -> String {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            // JSON retains element boundaries and escaped newlines. A comma-joined
            // array would make ["a,b", "c"] indistinguishable from ["a", "b,c"].
            format!(
                "${}={}",
                index + 1,
                serde_json::to_string(param).expect("typed inspection parameter")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::model::build_filter;
    use super::*;
    #[test]
    fn inspection_parameters_preserve_array_boundaries_and_control_characters() {
        let source = vec![
            InspectionParam::TextArray {
                values: vec!["a,b".into(), "c\n東京".into()],
            },
            InspectionParam::Text {
                value: "\tNULL".into(),
            },
        ];
        let rendered = parameters(&source);
        assert_eq!(rendered.lines().count(), 2);
        for (line, param) in rendered.lines().zip(&source) {
            let (_, json) = line.split_once('=').unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(json).unwrap(),
                serde_json::to_value(param).unwrap()
            );
        }
        assert_ne!(
            parameters(&[InspectionParam::TextArray {
                values: vec!["a".into(), "b,c".into()]
            }]),
            parameters(&[InspectionParam::TextArray {
                values: vec!["a,b".into(), "c".into()]
            }])
        );
    }
    #[test]
    fn typed_editor_matches_baseline_null_empty_and_list_rules() {
        assert!(matches!(
            build_filter("name", 11, ""),
            Ok(BrowseFilter::IsNull { .. })
        ));
        assert!(build_filter("name", 0, "  ").is_err());
        assert!(
            matches!(build_filter("name", 10, " 東京, , É "), Ok(BrowseFilter::InList { values, .. }) if values == ["東京", "É"])
        );
        let mut state = BrowseState {
            raw_filter_text: "enabled".into(),
            ..Default::default()
        };
        state.apply_filter(build_filter("name", 0, "old").unwrap());
        state.apply_filter(build_filter("name", 1, "new").unwrap());
        assert_eq!(state.typed_filters.len(), 1);
        assert_eq!(state.raw_filter_text, "enabled");
    }
}
