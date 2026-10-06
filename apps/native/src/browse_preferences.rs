//! Typed browse preferences patch the latest profile record. Column geometry,
//! unknown fields and other tabs' history/presets are never rebuilt from defaults.
use crate::{data_model::TableQuery, grid_columns::ColumnAction, results::encoded_size};
use dbunk_lib::backend::{WorkspaceTableState, data::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PREFS_BYTES: usize = 64 * 1024;
pub const PAGE_SIZES: [u32; 7] = [10, 25, 50, 100, 250, 500, 1000];
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterMode {
    #[default]
    Typed,
    Raw,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowseState {
    pub typed_filters: Vec<BrowseFilter>,
    pub raw_filter_text: String,
    pub filter_mode: FilterMode,
    pub sort: Vec<BrowseSortKey>,
    pub page_size: u32,
}
impl Default for BrowseState {
    fn default() -> Self {
        Self {
            typed_filters: vec![],
            raw_filter_text: String::new(),
            filter_mode: FilterMode::Typed,
            sort: vec![],
            page_size: 100,
        }
    }
}
impl BrowseState {
    pub fn workspace(state: &WorkspaceTableState) -> Self {
        let raw = state
            .filters
            .iter()
            .filter_map(|filter| match filter {
                BrowseFilter::RawSql { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        Self {
            typed_filters: state
                .filters
                .iter()
                .filter(|filter| !matches!(filter, BrowseFilter::RawSql { .. }))
                .cloned()
                .collect(),
            raw_filter_text: if raw.len() == 1 {
                raw[0].into()
            } else {
                raw.iter()
                    .map(|text| format!("({text})"))
                    .collect::<Vec<_>>()
                    .join(" AND ")
            },
            sort: state.sort.clone(),
            page_size: state.page_size,
            ..Default::default()
        }
    }
    pub fn query(&self) -> TableQuery {
        TableQuery {
            filters: self.typed_filters.clone(),
            sort: self.sort.clone(),
            page_size: self.page_size,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.typed_filters.len() > 256
            || self.sort.len() > 256
            || encoded_size(self) > PREFS_BYTES
        {
            return Err("Browse settings exceed 64 KiB or 256 predicates/sort keys");
        }
        if !(1..=1000).contains(&self.page_size) {
            return Err("Page size must be 1 to 1000");
        }
        Ok(())
    }
    pub fn apply_filter(&mut self, filter: BrowseFilter) {
        if let Some(column) = filter_column(&filter) {
            self.typed_filters
                .retain(|old| filter_column(old) != Some(column));
        }
        self.typed_filters.push(filter);
    }
}
pub fn filter_column(filter: &BrowseFilter) -> Option<&str> {
    match filter {
        BrowseFilter::Comparison { column, .. }
        | BrowseFilter::TextMatch { column, .. }
        | BrowseFilter::IsNull { column }
        | BrowseFilter::IsNotNull { column }
        | BrowseFilter::InList { column, .. } => Some(column),
        BrowseFilter::RawSql { .. } => None,
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub applied_at: String,
    pub typed_filters: Vec<BrowseFilter>,
    pub raw_filter_text: String,
    pub filter_mode: FilterMode,
    pub sort: Vec<BrowseSortKey>,
}
impl HistoryEntry {
    pub fn state(&self, page_size: u32) -> BrowseState {
        BrowseState {
            typed_filters: self.typed_filters.clone(),
            raw_filter_text: self.raw_filter_text.clone(),
            filter_mode: self.filter_mode,
            sort: self.sort.clone(),
            page_size,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(flatten)]
    pub state: BrowseState,
}
#[derive(Clone)]
pub struct BrowsePreferences {
    pub state: BrowseState,
    pub history: Vec<HistoryEntry>,
    pub presets: Vec<Preset>,
}
impl BrowsePreferences {
    pub fn parse(prefs: &TableGridPrefs) -> Result<Self, &'static str> {
        validate_record(prefs)?;
        let mut state: BrowseState = serde_json::from_value(prefs.0.clone())
            .map_err(|_| "Stored browse settings are invalid; preferences preserved")?;
        if !PAGE_SIZES.contains(&state.page_size) {
            state.page_size = 100;
        }
        state.validate()?;
        let history = prefs
            .0
            .get("filterHistory")
            .map(|value| serde_json::from_value::<Vec<HistoryEntry>>(value.clone()))
            .transpose()
            .map_err(|_| "Stored filter history is invalid; preferences preserved")?
            .unwrap_or_default();
        let presets = prefs
            .0
            .get("presets")
            .map(|value| serde_json::from_value::<Vec<Preset>>(value.clone()))
            .transpose()
            .map_err(|_| "Stored presets are invalid; preferences preserved")?
            .unwrap_or_default();
        if history
            .iter()
            .any(|entry| entry.state(100).validate().is_err())
            || presets
                .iter()
                .any(|preset| preset.name.trim().is_empty() || preset.state.validate().is_err())
        {
            return Err("Stored history or presets are invalid; preferences preserved");
        }
        Ok(Self {
            state,
            history: history.into_iter().take(20).collect(),
            presets,
        })
    }
}
/// The change, rather than a stale complete record, crosses the worker boundary.
#[derive(Clone)]
pub enum PreferencePatch {
    Browse {
        state: BrowseState,
        history: bool,
    },
    Mode(FilterMode),
    Preset(Preset),
    /// All widths in one acknowledged record; names are source identities.
    AutoFit(Vec<(String, f32)>),
    /// Desired state from the reviewed layout; merge only this name into the latest pins.
    PinColumn {
        selected: String,
        pinned: bool,
    },
    /// Swap exact visible neighbors only while both remain in the reviewed group.
    MoveColumn {
        selected: String,
        adjacent: String,
        left: bool,
        pinned: bool,
        source_order: Vec<String>,
    },
    Column {
        action: ColumnAction,
        selected: String,
        order: Vec<String>,
        width: f32,
    },
    /// A dragged width for one source name, merged into the latest widths.
    ColumnWidth {
        name: String,
        width: f32,
    },
    /// Desired visibility for one source name: hide adds it to
    /// `hiddenColumns`, show removes it.
    ColumnVisibility {
        name: String,
        visible: bool,
    },
}
impl PreferencePatch {
    pub fn apply(
        &self,
        current: Option<TableGridPrefs>,
        timestamp: &str,
    ) -> Result<TableGridPrefs, &'static str> {
        let mut prefs = current.unwrap_or_else(|| TableGridPrefs(json!({"version":1})));
        validate_record(&prefs)?;
        match self {
            Self::PinColumn { selected, pinned } => {
                set_pin(&mut prefs, selected, *pinned)?;
            }
            Self::ColumnWidth { name, width } => {
                if !valid_column_name(name)
                    || !width.is_finite()
                    || !(crate::grid_columns::RESIZE_MIN..=crate::grid_columns::RESIZE_MAX)
                        .contains(width)
                {
                    return Err("Column width is invalid");
                }
                if !prefs.0["columnWidths"].is_object() {
                    if !prefs.0["columnWidths"].is_null() {
                        return Err("Stored column widths are invalid");
                    }
                    prefs.0["columnWidths"] = json!({});
                }
                prefs.0["columnWidths"][name] = json!(width);
            }
            Self::ColumnVisibility { name, visible } => {
                if !valid_column_name(name) {
                    return Err("Select an unambiguous PostgreSQL column first");
                }
                let mut hidden = array(&prefs, "hiddenColumns")?;
                if *visible {
                    hidden.retain(|entry| entry.as_str() != Some(name.as_str()));
                } else if !hidden
                    .iter()
                    .any(|entry| entry.as_str() == Some(name.as_str()))
                {
                    hidden.push(json!(name));
                }
                prefs.0["hiddenColumns"] = json!(hidden);
            }
            Self::MoveColumn {
                selected,
                adjacent,
                left,
                pinned,
                source_order,
            } => {
                if !valid_column_name(selected)
                    || !valid_column_name(adjacent)
                    || encoded_size(source_order) > PREFS_BYTES
                {
                    return Err("Column order exceeds its supported bounds");
                }
                let pins = pin_names(&prefs)?;
                let contains = |name: &str| pins.iter().any(|pin| pin == name);
                let hidden = array(&prefs, "hiddenColumns")?;
                if contains(selected) != *pinned
                    || contains(adjacent) != *pinned
                    || hidden.iter().any(|name| {
                        name.as_str()
                            .is_some_and(|name| name == selected || name == adjacent)
                    })
                {
                    return Err("Column layout changed; refresh before reordering");
                }
                if selected != adjacent {
                    let key = if *pinned {
                        "pinnedColumns"
                    } else {
                        "columnOrder"
                    };
                    let mut order = if *pinned {
                        pins.clone()
                    } else {
                        let mut order = Vec::new();
                        for name in array(&prefs, key)?
                            .iter()
                            .filter_map(Value::as_str)
                            .chain(source_order.iter().map(String::as_str))
                        {
                            if !order.iter().any(|old| old == name) {
                                order.push(name.to_owned());
                            }
                        }
                        order
                    };
                    let first = order
                        .iter()
                        .position(|name| name == selected)
                        .ok_or("Selected column is no longer available")?;
                    let second = order
                        .iter()
                        .position(|name| name == adjacent)
                        .ok_or("Adjacent column is no longer available")?;
                    if (*left && first < second) || (!*left && first > second) {
                        return Err("Column order changed; refresh before reordering");
                    }
                    let between = first.min(second) + 1..first.max(second);
                    if order[between].iter().any(|name| {
                        source_order.contains(name)
                            && contains(name) == *pinned
                            && !hidden.iter().any(|hidden| hidden.as_str() == Some(name))
                    }) {
                        return Err("Column neighbors changed; refresh before reordering");
                    }
                    order.swap(first, second);
                    prefs.0[key] = json!(order);
                }
            }
            Self::AutoFit(widths) => {
                if widths.is_empty()
                    || encoded_size(widths) > PREFS_BYTES
                    || widths.iter().any(|(_, width)| {
                        !width.is_finite()
                            || !(crate::column_widths::MIN_WIDTH
                                ..=crate::column_widths::AUTO_FIT_MAX)
                                .contains(width)
                    })
                {
                    return Err("Auto-fit widths are invalid or exceed 64 KiB");
                }
                if !prefs.0["columnWidths"].is_object() {
                    if !prefs.0["columnWidths"].is_null() {
                        return Err("Stored column widths are invalid");
                    }
                    prefs.0["columnWidths"] = json!({});
                }
                for (name, width) in widths {
                    prefs.0["columnWidths"][name] = json!(width);
                }
            }
            Self::Browse { state, history } => {
                state.validate()?;
                if *history {
                    let entry = HistoryEntry {
                        applied_at: timestamp.into(),
                        typed_filters: state.typed_filters.clone(),
                        raw_filter_text: state.raw_filter_text.clone(),
                        filter_mode: state.filter_mode,
                        sort: state.sort.clone(),
                    };
                    let mut entries = array(&prefs, "filterHistory")?;
                    entries.insert(
                        0,
                        serde_json::to_value(entry).map_err(|_| "Invalid history")?,
                    );
                    entries.truncate(20);
                    prefs.0["filterHistory"] = Value::Array(entries);
                }
                for (key, value) in serde_json::to_value(state)
                    .map_err(|_| "Invalid browse settings")?
                    .as_object()
                    .unwrap()
                {
                    prefs.0[key] = value.clone();
                }
            }
            Self::Mode(mode) => prefs.0["filterMode"] = serde_json::to_value(mode).unwrap(),
            Self::Preset(preset) => {
                preset.state.validate()?;
                if preset.name.trim().is_empty() {
                    return Err("Enter a preset name");
                }
                let mut presets = array(&prefs, "presets")?;
                let mut preset = preset.clone();
                preset.name = preset.name.trim().into();
                presets.retain(|entry| {
                    entry.get("name").and_then(Value::as_str) != Some(&preset.name)
                });
                presets.insert(
                    0,
                    serde_json::to_value(preset).map_err(|_| "Invalid preset")?,
                );
                prefs.0["presets"] = Value::Array(presets);
            }
            Self::Column {
                action,
                selected,
                order,
                width,
            } => match action {
                ColumnAction::Narrow | ColumnAction::Widen => {
                    if !prefs.0["columnWidths"].is_object() {
                        if !prefs.0["columnWidths"].is_null() {
                            return Err("Stored column widths are invalid");
                        }
                        prefs.0["columnWidths"] = json!({});
                    }
                    prefs.0["columnWidths"][selected] = json!(width);
                }
                ColumnAction::Left | ColumnAction::Right => {
                    let mut next = order.clone();
                    for old in array(&prefs, "columnOrder")? {
                        if let Some(name) = old.as_str()
                            && !next.iter().any(|item| item == name)
                        {
                            next.push(name.into());
                        }
                    }
                    prefs.0["columnOrder"] = json!(next);
                }
                ColumnAction::Hide => {
                    let mut hidden = array(&prefs, "hiddenColumns")?;
                    if !hidden.iter().any(|name| name.as_str() == Some(selected)) {
                        hidden.push(json!(selected));
                    }
                    prefs.0["hiddenColumns"] = json!(hidden);
                }
                ColumnAction::ShowAll => prefs.0["hiddenColumns"] = json!([]),
                // GridColumns emits desired-state PinColumn, never this legacy
                // shape, so a later preference record cannot invert the intent.
                ColumnAction::TogglePin => {
                    return Err("Pinning requires a desired-state preference patch");
                }
            },
        }
        // Mirror backend normalization before returning an exact committed ACK.
        for key in ["filterHistory", "sortHistory"] {
            if let Some(Value::Array(entries)) = prefs.0.get_mut(key) {
                entries.truncate(20);
            }
        }
        validate_record(&prefs)?;
        Ok(prefs)
    }
}
pub(crate) fn valid_column_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 63 && !name.contains('\0')
}
pub(crate) fn validate_pins(prefs: &TableGridPrefs) -> Result<(), &'static str> {
    if let Some(value) = prefs.0.get("pinnedColumns") {
        let names = value
            .as_array()
            .ok_or("Stored column pins are invalid; settings preserved")?;
        if names
            .iter()
            .any(|name| !name.as_str().is_some_and(valid_column_name))
        {
            return Err("Stored column pins are invalid; settings preserved");
        }
    }
    Ok(())
}
fn pin_names(prefs: &TableGridPrefs) -> Result<Vec<String>, &'static str> {
    validate_pins(prefs)?;
    let mut names = Vec::new();
    for name in prefs
        .0
        .get("pinnedColumns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !names.iter().any(|old| old == name) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}
fn set_pin(prefs: &mut TableGridPrefs, selected: &str, pinned: bool) -> Result<(), &'static str> {
    if !valid_column_name(selected) {
        return Err("Select an unambiguous PostgreSQL column first");
    }
    let mut pins = pin_names(prefs)?;
    if pinned {
        if !pins.iter().any(|name| name == selected) {
            pins.push(selected.into());
        }
    } else {
        pins.retain(|name| name != selected);
    }
    prefs.0["pinnedColumns"] = json!(pins);
    Ok(())
}
fn array(prefs: &TableGridPrefs, key: &str) -> Result<Vec<Value>, &'static str> {
    match prefs.0.get(key) {
        None => Ok(vec![]),
        Some(Value::Array(values)) => Ok(values.clone()),
        _ => Err("Stored table preference list is invalid; settings preserved"),
    }
}
fn validate_record(prefs: &TableGridPrefs) -> Result<(), &'static str> {
    if prefs.0.get("version").and_then(Value::as_u64) != Some(1) || !prefs.0.is_object() {
        return Err("Unsupported table preferences; stored settings preserved");
    }
    validate_pins(prefs)?;
    if encoded_size(prefs) > PREFS_BYTES {
        return Err("Table preferences exceed 64 KiB; stored settings preserved");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(value: &str) -> BrowseState {
        BrowseState {
            raw_filter_text: value.into(),
            ..Default::default()
        }
    }
    #[test]
    fn auto_fit_merges_latest_widths_without_losing_hidden_columns_or_filters() {
        let current = TableGridPrefs(
            json!({"version":1,"columnWidths":{"hidden":230},"hiddenColumns":["hidden"],"rawFilterText":"id > 1","future":{"keep":true}}),
        );
        let patch = PreferencePatch::AutoFit(vec![("visible".into(), 91.), ("雪".into(), 500.)]);
        let saved = patch.apply(Some(current.clone()), "now").unwrap();
        assert_eq!(
            saved.0["columnWidths"],
            json!({"hidden":230,"visible":91.0,"雪":500.0})
        );
        assert_eq!(saved.0["rawFilterText"], "id > 1");
        assert_eq!(saved.0["hiddenColumns"], json!(["hidden"]));
        assert_eq!(saved.0["future"], json!({"keep":true}));
        assert!(
            PreferencePatch::AutoFit(vec![("x".into(), f32::NAN)])
                .apply(Some(current.clone()), "later")
                .is_err()
        );
        assert!(
            PreferencePatch::AutoFit(vec![("x".repeat(PREFS_BYTES), 60.)])
                .apply(Some(current.clone()), "later")
                .is_err()
        );
        assert_eq!(current.0["columnWidths"], json!({"hidden":230}));
    }
    #[test]
    fn patches_merge_latest_unrelated_fields_and_named_presets_without_lost_updates() {
        let current = TableGridPrefs(
            json!({"version":1,"future":{"keep":true},"columnWidths":{"id":150},"sortHistory":[{"legacy":"keep"}]}),
        );
        let saved = PreferencePatch::Browse {
            state: state("id > 0"),
            history: true,
        }
        .apply(Some(current), "now")
        .unwrap();
        let saved = PreferencePatch::Preset(Preset {
            name: " Saved ".into(),
            state: state("id > 1"),
        })
        .apply(Some(saved), "later")
        .unwrap();
        let saved = PreferencePatch::Column {
            action: ColumnAction::Widen,
            selected: "other".into(),
            order: vec![],
            width: 192.,
        }
        .apply(Some(saved), "later")
        .unwrap();
        assert_eq!(saved.0["future"]["keep"], true);
        assert_eq!(saved.0["columnWidths"]["id"], 150);
        assert_eq!(saved.0["rawFilterText"], "id > 0");
        assert_eq!(saved.0["presets"][0]["name"], "Saved");
        assert_eq!(saved.0["sortHistory"][0]["legacy"], "keep");
    }
    #[test]
    fn histories_cap_twenty_replays_keep_page_size_and_oversize_preserves_prior_record() {
        let mut prefs = None;
        for index in 0..21 {
            prefs = Some(
                PreferencePatch::Browse {
                    state: state(&index.to_string()),
                    history: true,
                }
                .apply(prefs, "now")
                .unwrap(),
            );
        }
        let prefs = prefs.unwrap();
        let parsed = BrowsePreferences::parse(&prefs).unwrap();
        assert_eq!(parsed.history.len(), 20);
        assert_eq!(parsed.history[0].raw_filter_text, "20");
        assert_eq!(parsed.history[19].raw_filter_text, "1");
        assert_eq!(parsed.history[0].state(250).page_size, 250);
        assert!(
            PreferencePatch::Browse {
                state: state(&"x".repeat(PREFS_BYTES)),
                history: true
            }
            .apply(Some(prefs.clone()), "now")
            .is_err()
        );
        assert_eq!(prefs.0["rawFilterText"], "20");
    }
}

#[cfg(test)]
mod pinning_tests;
