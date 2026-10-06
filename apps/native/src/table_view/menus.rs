//! Plan 032 §1 and §3.8: the header menu, the cell context menu and the `⋯`
//! overflow menu. Each menu is a static item table; rendering, keyboard
//! activation and the reachability test all read the same tables, so an action
//! cannot silently disappear from the table tab.
use super::{Action, TableView};
use crate::{
    browse_controls::{BrowsePanel, HeaderSort, header_sort},
    data_model::{CellRef, EditSeed, InsertCell},
    grid_columns::ColumnAction,
    table_changes::ChangesCommand,
    ui::popover::{self, MenuKey, MenuNav, Placement},
};
use dbunk_lib::backend::{csv_transfers::CsvDirection, data::BrowseSortDirection};
use gpui::{
    AnyElement, Bounds, Context, Entity, Focusable, MouseDownEvent, Pixels, Role, SharedString,
    Subscription, Window, prelude::*, px,
};

/// Every action the pre-032 table tab exposed: the 19 toolbar buttons, the
/// 10 column-row buttons and the footer pager. Only the reachability test
/// reads it; the item tables below are what the tab renders.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum LegacyAction {
    Connect,
    Backup,
    Restore,
    ImportCsv,
    ExportCsv,
    ExportTable,
    CopyTable,
    SeedTable,
    Structure,
    Refresh,
    EditCell,
    InsertRow,
    DuplicateRow,
    BulkEdit,
    DeleteRow,
    FollowForeignKey,
    Count,
    SortSelectedColumn,
    Cancel,
    NarrowColumn,
    WidenColumn,
    AutoFitColumn,
    AutoFitVisibleColumns,
    MoveColumnLeft,
    MoveColumnRight,
    PinColumn,
    HideColumn,
    ShowAllColumns,
    ReloadPreferences,
    FirstPage,
    PreviousPage,
    NextPage,
    LastPage,
    PageSize,
}

#[cfg(test)]
pub(super) const LEGACY_ACTIONS: [LegacyAction; 34] = [
    LegacyAction::Connect,
    LegacyAction::Backup,
    LegacyAction::Restore,
    LegacyAction::ImportCsv,
    LegacyAction::ExportCsv,
    LegacyAction::ExportTable,
    LegacyAction::CopyTable,
    LegacyAction::SeedTable,
    LegacyAction::Structure,
    LegacyAction::Refresh,
    LegacyAction::EditCell,
    LegacyAction::InsertRow,
    LegacyAction::DuplicateRow,
    LegacyAction::BulkEdit,
    LegacyAction::DeleteRow,
    LegacyAction::FollowForeignKey,
    LegacyAction::Count,
    LegacyAction::SortSelectedColumn,
    LegacyAction::Cancel,
    LegacyAction::NarrowColumn,
    LegacyAction::WidenColumn,
    LegacyAction::AutoFitColumn,
    LegacyAction::AutoFitVisibleColumns,
    LegacyAction::MoveColumnLeft,
    LegacyAction::MoveColumnRight,
    LegacyAction::PinColumn,
    LegacyAction::HideColumn,
    LegacyAction::ShowAllColumns,
    LegacyAction::ReloadPreferences,
    LegacyAction::FirstPage,
    LegacyAction::PreviousPage,
    LegacyAction::NextPage,
    LegacyAction::LastPage,
    LegacyAction::PageSize,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeaderItem {
    SortAsc,
    SortDesc,
    ClearSort,
    MultiSort,
    Filter,
    Hide,
    TogglePin,
    MoveLeft,
    MoveRight,
    Narrow,
    Widen,
    AutoFit,
}

pub(super) const HEADER_ITEMS: [HeaderItem; 12] = [
    HeaderItem::SortAsc,
    HeaderItem::SortDesc,
    HeaderItem::ClearSort,
    HeaderItem::MultiSort,
    HeaderItem::Filter,
    HeaderItem::Hide,
    HeaderItem::TogglePin,
    HeaderItem::MoveLeft,
    HeaderItem::MoveRight,
    HeaderItem::Narrow,
    HeaderItem::Widen,
    HeaderItem::AutoFit,
];

impl HeaderItem {
    #[cfg(test)]
    pub(super) fn covers(self) -> &'static [LegacyAction] {
        match self {
            Self::SortAsc | Self::SortDesc => &[LegacyAction::SortSelectedColumn],
            Self::ClearSort | Self::MultiSort | Self::Filter => &[],
            Self::Hide => &[LegacyAction::HideColumn],
            Self::TogglePin => &[LegacyAction::PinColumn],
            Self::MoveLeft => &[LegacyAction::MoveColumnLeft],
            Self::MoveRight => &[LegacyAction::MoveColumnRight],
            Self::Narrow => &[LegacyAction::NarrowColumn],
            Self::Widen => &[LegacyAction::WidenColumn],
            Self::AutoFit => &[LegacyAction::AutoFitColumn],
        }
    }
    fn divider_before(self) -> bool {
        matches!(
            self,
            Self::MultiSort | Self::Filter | Self::Hide | Self::Narrow
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CellItem {
    Edit,
    SetNull,
    RevertCell,
    RevertRow,
    Copy,
    Inspect,
    FollowForeignKey,
    Duplicate,
    Delete,
    BulkEdit,
}

pub(super) const CELL_ITEMS: [CellItem; 10] = [
    CellItem::Edit,
    CellItem::SetNull,
    CellItem::RevertCell,
    CellItem::RevertRow,
    CellItem::Copy,
    CellItem::Inspect,
    CellItem::FollowForeignKey,
    CellItem::Duplicate,
    CellItem::Delete,
    CellItem::BulkEdit,
];

impl CellItem {
    #[cfg(test)]
    pub(super) fn covers(self) -> &'static [LegacyAction] {
        match self {
            Self::Edit => &[LegacyAction::EditCell],
            Self::FollowForeignKey => &[LegacyAction::FollowForeignKey],
            Self::Duplicate => &[LegacyAction::DuplicateRow],
            Self::Delete => &[LegacyAction::DeleteRow],
            Self::BulkEdit => &[LegacyAction::BulkEdit],
            Self::SetNull | Self::RevertCell | Self::RevertRow | Self::Copy | Self::Inspect => &[],
        }
    }
    fn divider_before(self) -> bool {
        matches!(self, Self::RevertCell | Self::Copy | Self::Duplicate)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OverflowItem {
    Backup,
    Restore,
    ImportCsv,
    ExportCsv,
    ExportTable,
    CopyTable,
    SeedTable,
    Count,
    VirtualKey,
    FilterHistory,
    Presets,
    InspectQuery,
    ExportRows,
    ReloadPreferences,
    ShowAllColumns,
    AutoFitAll,
}

pub(super) const OVERFLOW_ITEMS: [OverflowItem; 16] = [
    OverflowItem::Count,
    OverflowItem::VirtualKey,
    OverflowItem::FilterHistory,
    OverflowItem::Presets,
    OverflowItem::InspectQuery,
    OverflowItem::ExportRows,
    OverflowItem::ExportTable,
    OverflowItem::ExportCsv,
    OverflowItem::ImportCsv,
    OverflowItem::CopyTable,
    OverflowItem::SeedTable,
    OverflowItem::Backup,
    OverflowItem::Restore,
    OverflowItem::ShowAllColumns,
    OverflowItem::AutoFitAll,
    OverflowItem::ReloadPreferences,
];

impl OverflowItem {
    #[cfg(test)]
    pub(super) fn covers(self) -> &'static [LegacyAction] {
        match self {
            Self::Backup => &[LegacyAction::Backup],
            Self::Restore => &[LegacyAction::Restore],
            Self::ImportCsv => &[LegacyAction::ImportCsv],
            Self::ExportCsv => &[LegacyAction::ExportCsv],
            Self::ExportTable => &[LegacyAction::ExportTable],
            Self::CopyTable => &[LegacyAction::CopyTable],
            Self::SeedTable => &[LegacyAction::SeedTable],
            Self::Count => &[LegacyAction::Count],
            Self::ReloadPreferences => &[LegacyAction::ReloadPreferences],
            Self::ShowAllColumns => &[LegacyAction::ShowAllColumns],
            Self::AutoFitAll => &[LegacyAction::AutoFitVisibleColumns],
            Self::VirtualKey
            | Self::FilterHistory
            | Self::Presets
            | Self::InspectQuery
            | Self::ExportRows => &[],
        }
    }
    fn divider_before(self) -> bool {
        matches!(
            self,
            Self::VirtualKey | Self::ExportRows | Self::Backup | Self::ShowAllColumns
        )
    }
    fn label(self) -> &'static str {
        match self {
            Self::Backup => "Backup…",
            Self::Restore => "Restore…",
            Self::ImportCsv => "Import CSV…",
            Self::ExportCsv => "Export CSV…",
            Self::ExportTable => "Export table…",
            Self::CopyTable => "Copy table…",
            Self::SeedTable => "Seed table…",
            Self::Count => "Count rows exactly",
            Self::VirtualKey => "Virtual key…",
            Self::FilterHistory => "Filter history…",
            Self::Presets => "Presets…",
            Self::InspectQuery => "Inspect query…",
            Self::ExportRows => "Export rows…",
            Self::ReloadPreferences => "Reload column preferences",
            Self::ShowAllColumns => "Show all columns",
            Self::AutoFitAll => "Auto-fit all columns",
        }
    }
    fn icon(self) -> Option<&'static str> {
        match self {
            Self::Count => Some("icons/hash.svg"),
            Self::ExportRows | Self::ExportTable | Self::ExportCsv => Some("icons/download.svg"),
            Self::CopyTable => Some("icons/copy.svg"),
            Self::InspectQuery => Some("icons/code.svg"),
            Self::ShowAllColumns => Some("icons/eye.svg"),
            Self::ReloadPreferences => Some("icons/rotate_ccw.svg"),
            _ => None,
        }
    }
}

/// One rendered row of a menu.
pub(super) struct Entry<I> {
    pub item: I,
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub hint: Option<SharedString>,
    pub enabled: bool,
    pub divider: bool,
}

/// The single open popover of the table tab. Kit popovers close on an outside
/// mouse-down unless the press lands on their own trigger (`anchor`), so the
/// trigger toggles instead of closing and immediately reopening.
pub(super) enum Popover {
    Header {
        source: usize,
        anchor: Bounds<Pixels>,
        nav: MenuNav,
    },
    Cell {
        cell: CellRef,
        source: usize,
        anchor: Bounds<Pixels>,
        nav: MenuNav,
    },
    Overflow {
        anchor: Bounds<Pixels>,
        nav: MenuNav,
    },
    Columns {
        anchor: Bounds<Pixels>,
        search: Entity<editor::Editor>,
        field: Entity<crate::accessible_editor::AccessibleEditor>,
        _edits: Subscription,
    },
    Pager {
        anchor: Bounds<Pixels>,
        page: Entity<editor::Editor>,
        field: Entity<crate::accessible_editor::AccessibleEditor>,
        error: Option<SharedString>,
    },
}

impl Popover {
    pub(super) fn anchor(&self) -> Bounds<Pixels> {
        match self {
            Self::Header { anchor, .. }
            | Self::Cell { anchor, .. }
            | Self::Overflow { anchor, .. }
            | Self::Columns { anchor, .. }
            | Self::Pager { anchor, .. } => *anchor,
        }
    }
}

/// A zero-size anchor at the pointer for the cell context menu.
pub(super) fn point_anchor(position: gpui::Point<Pixels>) -> Bounds<Pixels> {
    Bounds::new(position, gpui::size(px(0.), px(0.)))
}

impl TableView {
    pub(super) fn close_popover(&mut self, cx: &mut Context<Self>) {
        if self.popover.take().is_some() {
            cx.notify();
        }
    }

    /// Navigation, sort and column changes share the old toolbar gate.
    pub(super) fn can_browse(&self, cx: &gpui::App) -> bool {
        self.editable
            && !self.busy
            && self.controls.is_some()
            && self.preferences_ready
            && !self.changes.read(cx).navigation_blocked()
    }

    fn can_stage(&self, cx: &gpui::App) -> Result<(), SharedString> {
        if !self.editable || self.controls.is_none() {
            return Err("Connect this table first".into());
        }
        if self.busy {
            return Err("Wait for the table to finish loading".into());
        }
        self.changes.read(cx).can_edit_now()
    }

    fn column_name(&self, source: usize, cx: &gpui::App) -> Option<String> {
        self.grid
            .read(cx)
            .column_entries()
            .into_iter()
            .find(|entry| entry.source == source)
            .map(|entry| entry.name)
    }

    // ---- Header menu -----------------------------------------------------

    pub(super) fn open_header_menu(
        &mut self,
        source: usize,
        anchor: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if matches!(&self.popover, Some(Popover::Header { source: open, .. }) if *open == source) {
            self.close_popover(cx);
            return;
        }
        let len = self.header_entries(source, cx).len();
        self.popover = Some(Popover::Header {
            source,
            anchor,
            nav: MenuNav::new(len),
        });
        cx.notify();
    }

    pub(super) fn header_entries(&self, source: usize, cx: &gpui::App) -> Vec<Entry<HeaderItem>> {
        let entry = self
            .grid
            .read(cx)
            .column_entries()
            .into_iter()
            .find(|entry| entry.source == source);
        let name = entry.as_ref().map(|entry| entry.name.as_str());
        let sorted = name.and_then(|name| {
            self.state
                .sort
                .iter()
                .find(|key| key.column == name)
                .map(|key| key.direction)
        });
        let pinned = entry.as_ref().is_some_and(|entry| entry.pinned);
        let browse = self.can_browse(cx) && entry.is_some();
        HEADER_ITEMS
            .into_iter()
            .map(|item| {
                let (label, icon, enabled): (&str, Option<&'static str>, bool) = match item {
                    HeaderItem::SortAsc => (
                        "Sort ascending",
                        Some("icons/arrow_up.svg"),
                        browse && sorted != Some(BrowseSortDirection::Asc),
                    ),
                    HeaderItem::SortDesc => (
                        "Sort descending",
                        Some("icons/arrow_down.svg"),
                        browse && sorted != Some(BrowseSortDirection::Desc),
                    ),
                    HeaderItem::ClearSort => ("Clear sort", None, browse && sorted.is_some()),
                    HeaderItem::MultiSort => (
                        "Multi-column sort…",
                        Some("icons/chevron_up_down.svg"),
                        browse,
                    ),
                    HeaderItem::Filter => {
                        ("Filter on this column…", Some("icons/filter.svg"), browse)
                    }
                    HeaderItem::Hide => ("Hide column", Some("icons/eye_off.svg"), browse),
                    HeaderItem::TogglePin if pinned => {
                        ("Unpin column", Some("icons/unpin.svg"), browse)
                    }
                    HeaderItem::TogglePin => ("Pin column", Some("icons/pin.svg"), browse),
                    HeaderItem::MoveLeft => {
                        ("Move column left", Some("icons/arrow_left.svg"), browse)
                    }
                    HeaderItem::MoveRight => {
                        ("Move column right", Some("icons/arrow_right.svg"), browse)
                    }
                    HeaderItem::Narrow => ("Narrow column", None, browse),
                    HeaderItem::Widen => ("Widen column", None, browse),
                    HeaderItem::AutoFit => ("Auto-fit column", None, browse),
                };
                Entry {
                    item,
                    label: label.into(),
                    icon,
                    hint: None,
                    enabled,
                    divider: item.divider_before(),
                }
            })
            .collect()
    }

    pub(super) fn run_header(
        &mut self,
        source: usize,
        item: HeaderItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_browse(cx) {
            self.status = "Wait for the table to finish loading".into();
            return;
        }
        let Some(name) = self.column_name(source, cx) else {
            self.status = "That column is no longer in this page".into();
            return;
        };
        let choice = match item {
            HeaderItem::SortAsc => Some(HeaderSort::Asc),
            HeaderItem::SortDesc => Some(HeaderSort::Desc),
            HeaderItem::ClearSort => Some(HeaderSort::Clear),
            _ => None,
        };
        if let Some(choice) = choice {
            // Asc/Desc replace the whole sort, Clear drops only this column
            // (§3.8); apply_browse re-queries and invalidates the exact count.
            let mut state = self.browse_controls.read(cx).state().clone();
            state.sort = header_sort(&state.sort, &name, choice);
            self.apply_browse(state, true, cx);
            return;
        }
        let column = |this: &mut Self, action: ColumnAction, cx: &mut Context<Self>| match this
            .grid
            .read(cx)
            .column_patch(source, action)
        {
            Ok(patch) => this.save_preferences(patch, cx),
            Err(error) => this.preferences_status = Some(error.into()),
        };
        match item {
            HeaderItem::SortAsc | HeaderItem::SortDesc | HeaderItem::ClearSort => {}
            HeaderItem::MultiSort => {
                let anchor = self
                    .grid
                    .read(cx)
                    .header_bounds(source)
                    .unwrap_or_else(|| point_anchor(window.mouse_position()));
                self.browse_controls.update(cx, |controls, cx| {
                    controls.open_panel(BrowsePanel::Sort, anchor, window, cx)
                });
            }
            HeaderItem::Filter => self.browse_controls.update(cx, |controls, cx| {
                controls.add_condition(Some(name.as_str()), window, cx)
            }),
            HeaderItem::Hide => column(self, ColumnAction::Hide, cx),
            HeaderItem::TogglePin => column(self, ColumnAction::TogglePin, cx),
            HeaderItem::MoveLeft => column(self, ColumnAction::Left, cx),
            HeaderItem::MoveRight => column(self, ColumnAction::Right, cx),
            HeaderItem::Narrow => column(self, ColumnAction::Narrow, cx),
            HeaderItem::Widen => column(self, ColumnAction::Widen, cx),
            HeaderItem::AutoFit => self
                .grid
                .update(cx, |grid, cx| grid.auto_fit_source(source, cx)),
        }
    }

    // ---- Cell context menu -----------------------------------------------

    pub(super) fn open_cell_menu(
        &mut self,
        cell: CellRef,
        source: usize,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let CellRef::Page(row) = cell {
            // Keep a multi-row selection that already covers the cell so Bulk
            // edit stays meaningful; otherwise select the clicked cell.
            let grid = self.grid.read(cx);
            let covered = grid.selected_rows().is_some_and(|rows| rows.contains(&row))
                && grid
                    .selected_cell()
                    .is_some_and(|(_, column)| column == source);
            if !covered {
                self.grid
                    .update(cx, |grid, cx| grid.select_source_cell(row, source, cx));
            }
        }
        let len = self.cell_entries(cell, source, cx).len();
        self.popover = Some(Popover::Cell {
            cell,
            source,
            anchor: point_anchor(position),
            nav: MenuNav::new(len),
        });
        cx.notify();
    }

    pub(super) fn cell_entries(
        &self,
        cell: CellRef,
        source: usize,
        cx: &gpui::App,
    ) -> Vec<Entry<CellItem>> {
        let stage = self.can_stage(cx).is_ok();
        let changes = self.changes.read(cx);
        let overlay = changes.overlay();
        let settled = !changes.pending();
        let (cell_staged, row_staged) = match cell {
            CellRef::Page(row) => (
                overlay.cell(row, source).is_some(),
                overlay.mark(row).is_some(),
            ),
            CellRef::Insert(id) => {
                let insert = overlay.inserts.iter().find(|insert| insert.change == id);
                (
                    insert.is_some_and(|insert| {
                        matches!(insert.cells.get(source), Some(InsertCell::Value(_)))
                    }),
                    insert.is_some(),
                )
            }
        };
        let page = matches!(cell, CellRef::Page(_));
        let navigate = page
            && self.editable
            && !self.busy
            && self.controls.is_some()
            && !changes.navigation_blocked()
            && self
                .model
                .as_ref()
                .is_some_and(|model| model.page_is_current());
        let grid = self.grid.read(cx);
        let spans_rows = grid.selected_rows().is_some_and(|rows| rows.len() > 1);
        CELL_ITEMS
            .into_iter()
            .map(|item| {
                let (label, icon, hint, enabled): (&str, Option<&'static str>, Option<&str>, bool) =
                    match item {
                        CellItem::Edit => ("Edit cell", Some("icons/pencil.svg"), Some("↵"), stage),
                        CellItem::SetNull => ("Set NULL", None, None, stage),
                        CellItem::RevertCell => (
                            "Revert cell",
                            Some("icons/undo.svg"),
                            None,
                            settled && cell_staged,
                        ),
                        CellItem::RevertRow if !page => (
                            "Remove new row",
                            Some("icons/close.svg"),
                            None,
                            settled && row_staged,
                        ),
                        CellItem::RevertRow => ("Revert row", None, None, settled && row_staged),
                        CellItem::Copy => ("Copy", Some("icons/copy.svg"), Some("⌘C"), page),
                        CellItem::Inspect => ("Inspect value", None, Some("Space"), page),
                        CellItem::FollowForeignKey => (
                            "Follow foreign key",
                            Some("icons/link.svg"),
                            None,
                            navigate
                                && self.after_analysis.is_none()
                                && self.pending_preferences.is_none(),
                        ),
                        CellItem::Duplicate => ("Duplicate row", None, None, page && stage),
                        CellItem::Delete => {
                            ("Delete row", Some("icons/trash.svg"), None, page && stage)
                        }
                        CellItem::BulkEdit => {
                            ("Bulk edit column…", None, None, page && stage && spans_rows)
                        }
                    };
                Entry {
                    item,
                    label: label.into(),
                    icon,
                    hint: hint.map(SharedString::from),
                    enabled,
                    divider: item.divider_before(),
                }
            })
            .collect()
    }

    pub(super) fn run_cell(
        &mut self,
        cell: CellRef,
        source: usize,
        item: CellItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result: Result<(), SharedString> = match item {
            CellItem::Edit => self.can_stage(cx).and_then(|()| {
                self.changes.update(cx, |changes, cx| {
                    changes.begin_edit(cell, source, EditSeed::Keep, window, cx)
                })
            }),
            CellItem::SetNull => self.can_stage(cx).and_then(|()| {
                self.changes
                    .update(cx, |changes, cx| changes.set_null(cell, source, cx))
            }),
            CellItem::RevertCell => self
                .changes
                .update(cx, |changes, cx| changes.revert(cell, Some(source), cx)),
            CellItem::RevertRow => match cell {
                CellRef::Insert(change) => {
                    self.changes
                        .update(cx, |changes, cx| changes.remove_insert(change, cx));
                    Ok(())
                }
                CellRef::Page(_) => self
                    .changes
                    .update(cx, |changes, cx| changes.revert(cell, None, cx)),
            },
            CellItem::Copy | CellItem::Inspect => {
                let focus = self.grid.focus_handle(cx);
                window.focus(&focus, cx);
                if item == CellItem::Copy {
                    focus.dispatch_action(&crate::grid::CopyCells, window, cx);
                } else {
                    focus.dispatch_action(&crate::grid::InspectCell, window, cx);
                }
                Ok(())
            }
            CellItem::FollowForeignKey => {
                self.activate(Action::ForeignKeys, window, cx);
                Ok(())
            }
            CellItem::Duplicate => match cell {
                CellRef::Page(row) => self.can_stage(cx).and_then(|()| {
                    self.changes
                        .update(cx, |changes, cx| changes.duplicate(row, cx))
                        .map(|_| ())
                }),
                CellRef::Insert(_) => Err("Only loaded rows can be duplicated".into()),
            },
            CellItem::Delete => match cell {
                CellRef::Page(row) => self.can_stage(cx).and_then(|()| {
                    // Deleting a checked row deletes the whole checked set.
                    let checked = self.grid.read(cx).checked_rows();
                    let rows = if checked.contains(&row) {
                        checked
                    } else {
                        vec![row]
                    };
                    let staged = self
                        .changes
                        .update(cx, |changes, cx| changes.stage_deletes(&rows, cx));
                    if staged.is_ok() {
                        self.grid.update(cx, |grid, cx| grid.clear_checked(cx));
                    }
                    staged.map(|_| ())
                }),
                CellRef::Insert(_) => Err("Use Remove new row for an unsaved row".into()),
            },
            CellItem::BulkEdit => {
                self.activate(Action::Bulk, window, cx);
                Ok(())
            }
        };
        if let Err(error) = result {
            self.status = error.to_string();
        }
    }

    // ---- Overflow menu ---------------------------------------------------

    pub(super) fn toggle_overflow(&mut self, cx: &mut Context<Self>) {
        if matches!(self.popover, Some(Popover::Overflow { .. })) {
            self.close_popover(cx);
            return;
        }
        let Some(anchor) = self.anchors.overflow.get() else {
            return;
        };
        self.popover = Some(Popover::Overflow {
            anchor,
            nav: MenuNav::new(OVERFLOW_ITEMS.len()),
        });
        cx.notify();
    }

    pub(super) fn overflow_entries(&self, cx: &gpui::App) -> Vec<Entry<OverflowItem>> {
        let navigation_blocked = self.changes.read(cx).navigation_blocked();
        // Matches the pre-032 gating of each former toolbar button.
        let tools = self.editable && self.connection.is_some() && !self.busy && !navigation_blocked;
        let connected =
            self.editable && !self.busy && !navigation_blocked && self.controls.is_some();
        let browse = self.can_browse(cx);
        OVERFLOW_ITEMS
            .into_iter()
            .map(|item| {
                let enabled = match item {
                    OverflowItem::Backup
                    | OverflowItem::Restore
                    | OverflowItem::ImportCsv
                    | OverflowItem::ExportCsv
                    | OverflowItem::ExportTable
                    | OverflowItem::CopyTable
                    | OverflowItem::SeedTable => tools,
                    OverflowItem::Count | OverflowItem::ReloadPreferences => connected,
                    OverflowItem::VirtualKey => self.editable && self.controls.is_some(),
                    OverflowItem::FilterHistory
                    | OverflowItem::Presets
                    | OverflowItem::InspectQuery
                    | OverflowItem::ShowAllColumns
                    | OverflowItem::AutoFitAll => browse,
                    OverflowItem::ExportRows => self
                        .model
                        .as_ref()
                        .is_some_and(|model| model.result().is_some()),
                };
                Entry {
                    item,
                    label: item.label().into(),
                    icon: item.icon(),
                    hint: None,
                    enabled,
                    divider: item.divider_before(),
                }
            })
            .collect()
    }

    pub(super) fn run_overflow(
        &mut self,
        item: OverflowItem,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel =
            |this: &mut Self, panel: BrowsePanel, window: &mut Window, cx: &mut Context<Self>| {
                this.browse_controls.update(cx, |controls, cx| {
                    controls.open_panel(panel, anchor, window, cx)
                });
            };
        match item {
            OverflowItem::Backup => self.activate(
                Action::FileJob(crate::pg_tool_jobs::Operation::Backup),
                window,
                cx,
            ),
            OverflowItem::Restore => self.activate(
                Action::FileJob(crate::pg_tool_jobs::Operation::Restore),
                window,
                cx,
            ),
            OverflowItem::ImportCsv => {
                self.activate(Action::CsvTransfer(CsvDirection::Import), window, cx)
            }
            OverflowItem::ExportCsv => {
                self.activate(Action::CsvTransfer(CsvDirection::Export), window, cx)
            }
            OverflowItem::ExportTable => self.activate(Action::WholeExport, window, cx),
            OverflowItem::CopyTable => self.activate(Action::TableCopy, window, cx),
            OverflowItem::SeedTable => self.activate(Action::TableSeed, window, cx),
            OverflowItem::Count => self.activate(Action::Count, window, cx),
            OverflowItem::VirtualKey => self.changes.update(cx, |changes, cx| {
                changes.command(ChangesCommand::OpenVirtualKey, window, cx)
            }),
            OverflowItem::FilterHistory => panel(self, BrowsePanel::History, window, cx),
            OverflowItem::Presets => panel(self, BrowsePanel::Presets, window, cx),
            OverflowItem::InspectQuery => panel(self, BrowsePanel::Inspect, window, cx),
            OverflowItem::ExportRows => self
                .grid
                .update(cx, |grid, cx| grid.open_export(window, cx)),
            OverflowItem::ReloadPreferences => self.activate(Action::Preferences, window, cx),
            OverflowItem::ShowAllColumns => {
                self.activate(Action::Column(ColumnAction::ShowAll), window, cx)
            }
            OverflowItem::AutoFitAll => self.activate(Action::AutoFit(true), window, cx),
        }
    }

    // ---- Shared menu rendering and keyboard ------------------------------

    /// Routes a key to the open menu. Returns true when the key was consumed.
    pub(super) fn menu_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcome = match &mut self.popover {
            Some(Popover::Header { nav, .. })
            | Some(Popover::Cell { nav, .. })
            | Some(Popover::Overflow { nav, .. }) => nav.key(key),
            _ => return false,
        };
        match outcome {
            MenuKey::Moved => cx.notify(),
            MenuKey::Dismiss => self.close_popover(cx),
            MenuKey::Activate(index) => self.activate_menu_index(index, window, cx),
            MenuKey::Ignored => return false,
        }
        true
    }

    fn activate_menu_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.popover.as_ref() {
            Some(Popover::Header { source, .. }) => {
                let source = *source;
                if let Some(entry) = self
                    .header_entries(source, cx)
                    .into_iter()
                    .nth(index)
                    .filter(|entry| entry.enabled)
                {
                    self.close_popover(cx);
                    self.run_header(source, entry.item, window, cx);
                }
            }
            Some(Popover::Cell { cell, source, .. }) => {
                let (cell, source) = (*cell, *source);
                if let Some(entry) = self
                    .cell_entries(cell, source, cx)
                    .into_iter()
                    .nth(index)
                    .filter(|entry| entry.enabled)
                {
                    self.close_popover(cx);
                    self.run_cell(cell, source, entry.item, window, cx);
                }
            }
            Some(Popover::Overflow { anchor, .. }) => {
                let anchor = *anchor;
                if let Some(entry) = self
                    .overflow_entries(cx)
                    .into_iter()
                    .nth(index)
                    .filter(|entry| entry.enabled)
                {
                    self.close_popover(cx);
                    self.run_overflow(entry.item, anchor, window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// The deferred layer for whichever menu or popover is open.
    pub(super) fn render_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        enum Open {
            Header(usize, MenuNav),
            Cell(CellRef, usize, MenuNav),
            Overflow(MenuNav),
            Columns,
            Pager,
        }
        let popover = self.popover.as_ref()?;
        let anchor = popover.anchor();
        let open = match popover {
            Popover::Header { source, nav, .. } => Open::Header(*source, *nav),
            Popover::Cell {
                cell, source, nav, ..
            } => Open::Cell(*cell, *source, *nav),
            Popover::Overflow { nav, .. } => Open::Overflow(*nav),
            Popover::Columns { .. } => Open::Columns,
            Popover::Pager { .. } => Open::Pager,
        };
        let element = match open {
            Open::Header(source, nav) => {
                let entries = self.header_entries(source, cx);
                let label = self
                    .column_name(source, cx)
                    .map_or("Column menu".to_owned(), |name| {
                        format!("{name} column menu")
                    });
                let panel = self.menu_panel(
                    "header-menu",
                    label,
                    entries,
                    nav,
                    cx,
                    move |this, item, window, cx| this.run_header(source, item, window, cx),
                );
                popover::layer(anchor, Placement::Below, panel).into_any_element()
            }
            Open::Cell(cell, source, nav) => {
                let entries = self.cell_entries(cell, source, cx);
                let panel = self.menu_panel(
                    "cell-menu",
                    "Cell actions".to_owned(),
                    entries,
                    nav,
                    cx,
                    move |this, item, window, cx| this.run_cell(cell, source, item, window, cx),
                );
                popover::layer(anchor, Placement::AtPoint, panel).into_any_element()
            }
            Open::Overflow(nav) => {
                let entries = self.overflow_entries(cx);
                let panel = self.menu_panel(
                    "overflow-menu",
                    "More table actions".to_owned(),
                    entries,
                    nav,
                    cx,
                    move |this, item, window, cx| this.run_overflow(item, anchor, window, cx),
                );
                popover::layer(anchor, Placement::BelowEnd, panel).into_any_element()
            }
            Open::Columns => self.render_columns_popover(window, cx)?,
            Open::Pager => self.render_pager_popover(cx)?,
        };
        Some(element)
    }

    /// Closes the popover on an outside press, except a press on its own
    /// trigger, which toggles through the trigger's click handler instead.
    pub(super) fn dismiss_outside(
        &self,
        anchor: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            if !anchor.contains(&event.position) {
                this.close_popover(cx);
            }
        })
    }

    fn menu_panel<I: Copy + 'static>(
        &self,
        id: &'static str,
        label: String,
        entries: Vec<Entry<I>>,
        nav: MenuNav,
        cx: &mut Context<Self>,
        run: impl Fn(&mut Self, I, &mut Window, &mut Context<Self>) + Clone + 'static,
    ) -> impl IntoElement {
        let anchor = self
            .popover
            .as_ref()
            .map(Popover::anchor)
            .unwrap_or_default();
        let mut panel = popover::panel(id, Role::Menu, label)
            .min_w(px(170.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .on_mouse_down_out(self.dismiss_outside(anchor, cx));
        for (index, entry) in entries.into_iter().enumerate() {
            if entry.divider && index > 0 {
                panel = panel.child(popover::divider());
            }
            let Entry {
                item,
                label,
                icon,
                hint,
                enabled,
                ..
            } = entry;
            let weak = cx.weak_entity();
            let (click, a11y) = (run.clone(), run.clone());
            panel = panel.child(
                popover::item(
                    (id, index),
                    label,
                    icon,
                    hint,
                    nav.highlighted == index,
                    enabled,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if enabled {
                        this.close_popover(cx);
                        click(this, item, window, cx);
                        this.sync_grid(cx);
                        cx.notify();
                    }
                }))
                .on_a11y_action(
                    gpui::accesskit::Action::Click,
                    move |_, window, cx| {
                        if enabled {
                            let _ = weak.update(cx, |this, cx| {
                                this.close_popover(cx);
                                a11y(this, item, window, cx);
                                this.sync_grid(cx);
                                cx.notify();
                            });
                        }
                    },
                ),
            );
        }
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        columns_popover::COLUMNS_ITEMS, pager::PAGER_ITEMS, toolbar::TOOLBAR_ITEMS,
    };
    use super::*;
    use std::collections::BTreeSet;

    /// Union of every surface that replaced the old toolbar, column row and
    /// footer pager.
    fn reachable_actions() -> BTreeSet<LegacyAction> {
        let mut reachable = BTreeSet::new();
        reachable.extend(TOOLBAR_ITEMS.iter().flat_map(|item| item.covers()));
        reachable.extend(OVERFLOW_ITEMS.iter().flat_map(|item| item.covers()));
        reachable.extend(HEADER_ITEMS.iter().flat_map(|item| item.covers()));
        reachable.extend(CELL_ITEMS.iter().flat_map(|item| item.covers()));
        reachable.extend(COLUMNS_ITEMS.iter().flat_map(|item| item.covers()));
        reachable.extend(PAGER_ITEMS.iter().flat_map(|item| item.covers()));
        reachable
    }

    #[test]
    fn every_legacy_table_action_stays_reachable() {
        let all = LEGACY_ACTIONS.into_iter().collect::<BTreeSet<_>>();
        assert_eq!(all.len(), 34, "19 toolbar + 10 column + 5 pager actions");
        let reachable = reachable_actions();
        let missing = all.difference(&reachable).collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "unreachable legacy actions: {missing:?}"
        );
    }

    #[test]
    fn width_actions_have_a_keyboard_path_in_the_header_menu() {
        let header = HEADER_ITEMS
            .iter()
            .flat_map(|item| item.covers())
            .copied()
            .collect::<BTreeSet<_>>();
        assert!(header.contains(&LegacyAction::NarrowColumn));
        assert!(header.contains(&LegacyAction::WidenColumn));
    }

    #[test]
    fn overflow_holds_the_former_table_tool_buttons() {
        let overflow = OVERFLOW_ITEMS
            .iter()
            .flat_map(|item| item.covers())
            .copied()
            .collect::<BTreeSet<_>>();
        for action in [
            LegacyAction::Backup,
            LegacyAction::Restore,
            LegacyAction::ImportCsv,
            LegacyAction::ExportCsv,
            LegacyAction::ExportTable,
            LegacyAction::CopyTable,
            LegacyAction::SeedTable,
            LegacyAction::Count,
        ] {
            assert!(overflow.contains(&action), "{action:?} missing from ⋯");
        }
    }

    #[test]
    fn menu_tables_have_no_duplicates() {
        fn unique<T: PartialEq + std::fmt::Debug>(items: &[T]) {
            for (index, item) in items.iter().enumerate() {
                assert!(!items[..index].contains(item), "duplicate {item:?}");
            }
        }
        unique(&HEADER_ITEMS);
        unique(&CELL_ITEMS);
        unique(&OVERFLOW_ITEMS);
    }
}
