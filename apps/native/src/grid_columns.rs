//! Display coordinates never replace source column indices. Preferences retain
//! unknown fields and absent columns so schema changes do not destroy settings.
use dbunk_lib::backend::data::TableGridPrefs;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

pub const DEFAULT_WIDTH: f32 = 160.;
/// Bounds for a dragged or stored column width.
pub const RESIZE_MIN: f32 = 48.;
pub const RESIZE_MAX: f32 = 1200.;
const PREFS_BYTES: usize = 64 * 1024;

/// One source column as the Columns popover lists it. Hidden columns are
/// included; `cast_type` is filled by the grid, which owns the page.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnEntry {
    pub source: usize,
    pub name: String,
    pub cast_type: String,
    pub visible: bool,
    pub pinned: bool,
}

#[derive(Clone, Copy)]
pub enum ColumnAction {
    Narrow,
    Widen,
    Left,
    Right,
    Hide,
    ShowAll,
    TogglePin,
}

#[derive(Clone)]
pub struct GridColumns {
    names: Vec<String>,
    initial_widths: Vec<f32>,
    visible: Vec<usize>,
    pinned_count: usize,
    widths: Vec<f32>,
    offsets: Vec<f32>,
    prefs: TableGridPrefs,
    /// Live (unsaved) drag widths by source name. The stored record wins on
    /// `load`; a new page keeps them so widths do not snap back mid-save.
    overrides: HashMap<String, f32>,
}
impl Default for GridColumns {
    fn default() -> Self {
        Self {
            names: Vec::new(),
            initial_widths: Vec::new(),
            visible: Vec::new(),
            pinned_count: 0,
            widths: Vec::new(),
            offsets: vec![0.],
            prefs: TableGridPrefs(json!({"version": 1})),
            overrides: HashMap::new(),
        }
    }
}
impl GridColumns {
    pub fn load(&mut self, prefs: Option<TableGridPrefs>) -> Result<(), &'static str> {
        let prefs = prefs.unwrap_or_else(|| TableGridPrefs(json!({"version": 1})));
        if !prefs.0.is_object() || prefs.0.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("Unsupported table preferences; stored settings were preserved");
        }
        if crate::results::encoded_size(&prefs) > PREFS_BYTES {
            return Err("Table preferences exceed 64 KiB; stored settings were preserved");
        }
        crate::browse_preferences::validate_pins(&prefs)?;
        self.prefs = prefs;
        self.overrides.clear();
        self.rebuild();
        Ok(())
    }
    #[cfg(test)]
    pub fn columns(&mut self, names: impl Iterator<Item = String>) {
        self.names = names.collect();
        self.initial_widths.clear();
        self.rebuild();
    }
    pub fn table_columns(&mut self, page: &dbunk_lib::backend::data::BrowseTableResult) {
        self.names = page
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        self.initial_widths = page
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                // The header shows the type beside the name, so fit both.
                crate::column_widths::fit(
                    &format!("{} {}", column.name, column.cast_type),
                    page.rows
                        .iter()
                        .take(crate::column_widths::SAMPLE_ROWS)
                        .map(|row| row.get(index).and_then(Option::as_deref)),
                    crate::column_widths::INITIAL_MAX,
                )
            })
            .collect();
        self.rebuild();
    }
    pub fn len(&self) -> usize {
        self.visible.len()
    }
    pub fn pinned_count(&self) -> usize {
        self.pinned_count
    }
    pub fn source(&self, display: usize) -> Option<usize> {
        self.visible.get(display).copied()
    }
    pub fn display(&self, source: usize) -> Option<usize> {
        self.visible.iter().position(|index| *index == source)
    }
    pub fn width(&self, display: usize) -> f32 {
        self.widths[display]
    }
    pub fn offset(&self, display: usize) -> f32 {
        self.offsets[display]
    }
    pub fn total_width(&self) -> f32 {
        *self.offsets.last().unwrap()
    }
    #[cfg(test)]
    pub fn prefs(&self) -> TableGridPrefs {
        self.prefs.clone()
    }
    pub fn visible_range(&self, left: f32, width: f32) -> Range<usize> {
        let first = self
            .offsets
            .partition_point(|offset| *offset < left.max(0.))
            .saturating_sub(2);
        let last = self
            .offsets
            .partition_point(|offset| *offset <= left.max(0.) + width.max(0.));
        first.min(self.len())..last.saturating_add(1).min(self.len())
    }
    fn strings(&self, key: &str) -> Vec<String> {
        self.prefs
            .0
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }
    fn rebuild(&mut self) {
        let hidden: HashSet<_> = self.strings("hiddenColumns").into_iter().collect();
        let mut seen = HashSet::new();
        // Names persist for tables, but an ambiguous name never picks one source
        // column arbitrarily. Query-result pins are owned separately by index.
        self.visible = self
            .strings("pinnedColumns")
            .iter()
            .filter_map(|name| {
                let mut matches = self
                    .names
                    .iter()
                    .enumerate()
                    .filter(|(_, value)| *value == name);
                let (index, _) = matches.next()?;
                matches.next().is_none().then_some(index)
            })
            .filter(|index| seen.insert(*index) && !hidden.contains(&self.names[*index]))
            .collect();
        self.pinned_count = self.visible.len();
        let order = self.strings("columnOrder");
        self.visible.extend(
            order
                .iter()
                .filter_map(|name| self.names.iter().position(|item| item == name))
                .chain(0..self.names.len())
                .filter(|index| seen.insert(*index) && !hidden.contains(&self.names[*index])),
        );
        // A malformed/imported all-hidden preference must leave a way to inspect data.
        if self.visible.is_empty() && !self.names.is_empty() {
            self.visible.push(0);
        }
        self.widths = self
            .visible
            .iter()
            .map(|index| {
                self.prefs
                    .0
                    .get("columnWidths")
                    .and_then(|widths| widths.get(&self.names[*index]))
                    .and_then(Value::as_f64)
                    .filter(|width| width.is_finite())
                    .map_or_else(
                        || {
                            self.initial_widths
                                .get(*index)
                                .copied()
                                .unwrap_or(DEFAULT_WIDTH)
                        },
                        |width| width.clamp(f64::from(RESIZE_MIN), f64::from(RESIZE_MAX)) as f32,
                    )
            })
            .collect();
        // Live drag widths apply last, so a new page never snaps them back.
        for (display, source) in self.visible.iter().enumerate() {
            if let Some(width) = self.overrides.get(&self.names[*source]) {
                self.widths[display] = *width;
            }
        }
        self.rebuild_offsets();
    }
    fn rebuild_offsets(&mut self) {
        self.offsets.clear();
        self.offsets.push(0.);
        for width in &self.widths {
            self.offsets.push(self.offsets.last().unwrap() + width);
        }
    }
    /// Applies a dragged width locally (no preference write). Returns whether
    /// the geometry changed.
    pub fn set_live_width(&mut self, display: usize, width: f32) -> bool {
        let Some(source) = self.source(display) else {
            return false;
        };
        if !width.is_finite() {
            return false;
        }
        let width = width.clamp(RESIZE_MIN, RESIZE_MAX);
        if self.widths[display] == width {
            return false;
        }
        self.widths[display] = width;
        self.overrides.insert(self.names[source].clone(), width);
        self.rebuild_offsets();
        true
    }
    /// Drops every live width; the stored record is authoritative again.
    pub fn clear_overrides(&mut self) -> bool {
        if self.overrides.is_empty() {
            return false;
        }
        self.overrides.clear();
        self.rebuild();
        true
    }
    fn unambiguous_name(&self, source: usize) -> Result<&str, &'static str> {
        let name = self
            .names
            .get(source)
            .ok_or("Column is no longer available")?;
        if self.names.iter().filter(|item| *item == name).count() != 1
            || !crate::browse_preferences::valid_column_name(name)
        {
            return Err("Column settings require an unambiguous PostgreSQL column name");
        }
        Ok(name)
    }
    /// The current width of one visible column as a stored preference.
    pub fn width_patch(
        &self,
        display: usize,
    ) -> Result<crate::browse_preferences::PreferencePatch, &'static str> {
        let source = self.source(display).ok_or("Select a column first")?;
        let name = self.unambiguous_name(source)?;
        Ok(crate::browse_preferences::PreferencePatch::ColumnWidth {
            name: name.to_owned(),
            width: self.widths[display].clamp(RESIZE_MIN, RESIZE_MAX),
        })
    }
    /// Visible columns in display order, then hidden ones in source order.
    pub fn entries(&self) -> Vec<ColumnEntry> {
        let mut entries = self
            .visible
            .iter()
            .enumerate()
            .map(|(display, source)| ColumnEntry {
                source: *source,
                name: self.names[*source].clone(),
                cast_type: String::new(),
                visible: true,
                pinned: display < self.pinned_count,
            })
            .collect::<Vec<_>>();
        entries.extend(
            (0..self.names.len())
                .filter(|source| !self.visible.contains(source))
                .map(|source| ColumnEntry {
                    source,
                    name: self.names[source].clone(),
                    cast_type: String::new(),
                    visible: false,
                    pinned: false,
                }),
        );
        entries
    }
    /// Shows or hides one source column. Hiding the last visible column is
    /// refused so the grid always keeps something to inspect.
    pub fn visibility_patch(
        &self,
        source: usize,
        visible: bool,
    ) -> Result<crate::browse_preferences::PreferencePatch, &'static str> {
        let name = self.unambiguous_name(source)?;
        if !visible && self.display(source).is_some() && self.len() <= 1 {
            return Err("Keep at least one column visible");
        }
        Ok(
            crate::browse_preferences::PreferencePatch::ColumnVisibility {
                name: name.to_owned(),
                visible,
            },
        )
    }
    /// Refuse over-budget changes atomically; the caller publishes only after
    /// SQLite acknowledges the complete preference record.
    pub fn patch(
        &self,
        selected: Option<usize>,
        action: ColumnAction,
    ) -> Result<crate::browse_preferences::PreferencePatch, &'static str> {
        use crate::browse_preferences::PreferencePatch;
        if matches!(action, ColumnAction::TogglePin) && selected.is_none() {
            return Err("Select a column first");
        }
        let display = selected.unwrap_or(0);
        let source = self.source(display);
        if matches!(action, ColumnAction::ShowAll) {
            return Ok(PreferencePatch::Column {
                action,
                selected: String::new(),
                order: vec![],
                width: DEFAULT_WIDTH,
            });
        }
        let source = source.ok_or("Select a column first")?;
        let name = &self.names[source];
        if matches!(
            action,
            ColumnAction::TogglePin | ColumnAction::Left | ColumnAction::Right
        ) && self.strings("hiddenColumns").contains(name)
        {
            return Err(
                "Choose Show all columns before pinning or moving a recovery-visible column",
            );
        }
        if matches!(
            action,
            ColumnAction::TogglePin | ColumnAction::Left | ColumnAction::Right
        ) && (self.names.iter().filter(|item| *item == name).count() != 1
            || !crate::browse_preferences::valid_column_name(name))
        {
            return Err(
                "Column pinning and ordering require an unambiguous PostgreSQL column name",
            );
        }
        let pinned = display < self.pinned_count;
        match action {
            ColumnAction::TogglePin => Ok(PreferencePatch::PinColumn {
                selected: name.clone(),
                pinned: !pinned,
            }),
            ColumnAction::Left | ColumnAction::Right => {
                let group = if pinned {
                    0..self.pinned_count
                } else {
                    self.pinned_count..self.len()
                };
                let target = if matches!(action, ColumnAction::Left) {
                    display.saturating_sub(1).max(group.start)
                } else {
                    (display + 1).min(group.end - 1)
                };
                let adjacent = &self.names[self.visible[target]];
                if self.names.iter().filter(|item| *item == adjacent).count() != 1 {
                    return Err("Column ordering requires unambiguous source names");
                }
                // Preflight before cloning the fallback source order.
                if crate::results::encoded_size(&self.names) > PREFS_BYTES {
                    return Err("Column order exceeds 64 KiB");
                }
                Ok(PreferencePatch::MoveColumn {
                    selected: name.clone(),
                    adjacent: adjacent.clone(),
                    left: matches!(action, ColumnAction::Left),
                    pinned,
                    source_order: self.names.clone(),
                })
            }
            ColumnAction::Hide if self.len() <= 1 => Err("Keep at least one column visible"),
            _ => {
                let width = match action {
                    ColumnAction::Narrow => (self.width(display) - 32.).clamp(48., 1200.),
                    ColumnAction::Widen => (self.width(display) + 32.).clamp(48., 1200.),
                    _ => self.width(display),
                };
                Ok(PreferencePatch::Column {
                    action,
                    selected: name.clone(),
                    order: vec![],
                    width,
                })
            }
        }
    }
    #[cfg(test)]
    pub fn change(
        &self,
        selected: Option<usize>,
        action: ColumnAction,
    ) -> Result<Self, &'static str> {
        let patch = self.patch(selected, action)?;
        let mut next = self.clone();
        next.prefs = patch.apply(Some(next.prefs), "")?;
        next.rebuild();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample_columns() -> GridColumns {
        let mut columns = GridColumns::default();
        columns.columns(["id", "value", "amount"].into_iter().map(str::to_owned));
        columns
    }
    #[test]
    fn reordered_and_hidden_cells_keep_source_identity_and_variable_geometry() {
        let columns = sample_columns()
            .change(Some(1), ColumnAction::Left)
            .unwrap()
            .change(Some(0), ColumnAction::Widen)
            .unwrap()
            .change(Some(1), ColumnAction::Hide)
            .unwrap();
        assert_eq!((columns.source(0), columns.source(1)), (Some(1), Some(2)));
        assert_eq!((columns.offset(1), columns.total_width()), (192., 352.));
        assert_eq!(columns.visible_range(195., 20.), 0..2);
        let mut restored = sample_columns();
        restored.load(Some(columns.prefs())).unwrap();
        assert_eq!(restored.visible, columns.visible);
        assert_eq!(restored.widths, columns.widths);
    }
    #[test]
    fn preferences_preserve_unrelated_fields_and_reconcile_schema_changes() {
        let mut columns = sample_columns();
        columns.load(Some(TableGridPrefs(json!({"version":1,"presets":[{"name":"keep"}],"columnOrder":["missing","value","value"],"columnWidths":{"value":-5,"missing":200},"hiddenColumns":["id"]})))).unwrap();
        assert_eq!(columns.source(0), Some(1));
        assert_eq!(columns.width(0), 48.);
        let columns = columns.change(Some(0), ColumnAction::Right).unwrap();
        assert_eq!(columns.prefs.0["presets"][0]["name"], "keep");
        assert_eq!(columns.prefs.0["columnWidths"]["missing"], 200);
        let mut restored = columns.clone();
        restored.columns(
            ["missing", "id", "value", "new"]
                .into_iter()
                .map(str::to_owned),
        );
        assert_eq!(restored.len(), 3);
        assert!(restored.visible.contains(&3));
    }
    #[test]
    fn invalid_or_oversized_preferences_do_not_replace_the_last_valid_state() {
        let mut columns = sample_columns();
        let before = columns.prefs();
        assert!(
            columns
                .load(Some(TableGridPrefs(json!({"version":2}))))
                .is_err()
        );
        assert!(
            columns
                .load(Some(TableGridPrefs(
                    json!({"version":1,"large":"x".repeat(PREFS_BYTES)})
                )))
                .is_err()
        );
        assert_eq!(columns.prefs(), before);
        let columns = columns
            .change(Some(0), ColumnAction::Hide)
            .unwrap()
            .change(Some(0), ColumnAction::Hide)
            .unwrap();
        assert!(columns.change(Some(0), ColumnAction::Hide).is_err());
        assert_eq!(
            columns.change(None, ColumnAction::ShowAll).unwrap().len(),
            3
        );
    }

    fn page(names: &[&str]) -> dbunk_lib::backend::data::BrowseTableResult {
        use dbunk_lib::backend::data::*;
        BrowseTableResult {
            request_id: 1,
            columns: names
                .iter()
                .map(|name| BrowseColumn {
                    name: (*name).into(),
                    cast_type: "text".into(),
                    nullable: true,
                })
                .collect(),
            rows: vec![names.iter().map(|_| Some("x".into())).collect()],
            identity: BrowseIdentity {
                kind: BrowseIdentityKind::None,
                columns: vec![],
            },
            row_identity: None,
            page_info: BrowsePageInfo {
                mode: BrowsePageMode::Offset,
                page: Some(1),
                has_more: false,
                next_cursor: None,
            },
            count: BrowseCount {
                kind: BrowseCountKind::Exact,
                value: Some(1),
            },
            inspection: BrowseInspection {
                sql: String::new(),
                params: vec![],
            },
            omitted_rows: 0,
            truncated_cells: 0,
            runtime_ms: 0,
        }
    }

    #[test]
    fn live_width_is_clamped_and_updates_offsets_and_total() {
        let mut columns = sample_columns();
        let before = columns.total_width();
        let first = columns.width(0);
        assert!(columns.set_live_width(0, 5.));
        assert_eq!(columns.width(0), RESIZE_MIN);
        assert_eq!(columns.offset(1), RESIZE_MIN);
        assert_eq!(columns.total_width(), before - first + RESIZE_MIN);
        assert!(columns.set_live_width(1, 10_000.));
        assert_eq!(columns.width(1), RESIZE_MAX);
        assert_eq!(columns.offset(2), RESIZE_MIN + RESIZE_MAX);
        // Unchanged, non-finite and unknown columns report no change.
        assert!(!columns.set_live_width(1, 2_000.));
        assert!(!columns.set_live_width(0, f32::NAN));
        assert!(!columns.set_live_width(9, 100.));
        assert_eq!(columns.width(0), RESIZE_MIN);
    }

    #[test]
    fn live_width_survives_a_new_page_and_load_restores_the_stored_width() {
        let mut columns = GridColumns::default();
        columns
            .load(Some(TableGridPrefs(
                json!({"version":1,"columnWidths":{"value":90}}),
            )))
            .unwrap();
        columns.table_columns(&page(&["id", "value"]));
        assert_eq!(columns.width(1), 90.);
        assert!(columns.set_live_width(1, 300.));
        columns.table_columns(&page(&["id", "value", "added"]));
        assert_eq!(columns.width(1), 300.);
        assert_eq!(columns.offset(2), columns.width(0) + 300.);
        let stored = columns.prefs();
        columns.load(Some(stored)).unwrap();
        assert_eq!(columns.width(1), 90.);
        // clear_overrides also returns to the stored record.
        assert!(columns.set_live_width(1, 250.));
        assert!(columns.clear_overrides());
        assert_eq!(columns.width(1), 90.);
        assert!(!columns.clear_overrides());
    }

    #[test]
    fn width_patch_names_the_source_and_refuses_ambiguous_names() {
        use crate::browse_preferences::PreferencePatch;
        let mut columns = sample_columns()
            .change(Some(2), ColumnAction::Left)
            .unwrap();
        assert!(columns.set_live_width(1, 222.));
        match columns.width_patch(1).unwrap() {
            PreferencePatch::ColumnWidth { name, width } => {
                assert_eq!(name, "amount");
                assert_eq!(width, 222.);
            }
            _ => panic!("expected a width patch"),
        }
        assert!(columns.width_patch(9).is_err());
        let mut duplicate = GridColumns::default();
        duplicate.columns(["x", "x"].into_iter().map(str::to_owned));
        assert!(duplicate.width_patch(0).is_err());
        assert!(duplicate.visibility_patch(1, false).is_err());
    }

    #[test]
    fn entries_include_hidden_columns_and_last_visible_cannot_hide() {
        use crate::browse_preferences::PreferencePatch;
        let columns = sample_columns()
            .change(Some(1), ColumnAction::Hide)
            .unwrap()
            .change(Some(1), ColumnAction::TogglePin)
            .unwrap();
        let entries = columns.entries();
        let summary = entries
            .iter()
            .map(|entry| {
                (
                    entry.source,
                    entry.name.as_str(),
                    entry.visible,
                    entry.pinned,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                (2, "amount", true, true),
                (0, "id", true, false),
                (1, "value", false, false)
            ]
        );
        match columns.visibility_patch(1, true).unwrap() {
            PreferencePatch::ColumnVisibility { name, visible } => {
                assert_eq!((name.as_str(), visible), ("value", true));
            }
            _ => panic!("expected a visibility patch"),
        }
        let single = columns.change(Some(0), ColumnAction::Hide).unwrap();
        assert_eq!(single.len(), 1);
        let last = single.source(0).unwrap();
        assert_eq!(
            single.visibility_patch(last, false).err(),
            Some("Keep at least one column visible")
        );
        // Hiding an already hidden column is still allowed (idempotent).
        assert!(single.visibility_patch(1, false).is_ok());
    }
}

#[cfg(test)]
mod pinning_tests;
