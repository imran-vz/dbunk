//! Display coordinates never replace source column indices. Preferences retain
//! unknown fields and absent columns so schema changes do not destroy settings.
use dbunk_lib::backend::data::TableGridPrefs;
use serde_json::{Value, json};
use std::{collections::HashSet, ops::Range};

pub const DEFAULT_WIDTH: f32 = 160.;
const PREFS_BYTES: usize = 64 * 1024;

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
                crate::column_widths::fit(
                    &column.name,
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
                        |width| width.clamp(48., 1200.) as f32,
                    )
            })
            .collect();
        self.offsets.clear();
        self.offsets.push(0.);
        for width in &self.widths {
            self.offsets.push(self.offsets.last().unwrap() + width);
        }
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
}

#[cfg(test)]
mod pinning_tests;
