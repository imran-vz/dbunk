//! Immutable server inspection with a shared retention lease. Filtering never
//! searches settings values and selection resolves stable names after changes.
use dbunk_lib::backend::server_details::{
    MAX_SERVER_DETAILS_BYTES, ServerDetailsSnapshot, ServerExtension, ServerLimit, ServerRows,
    ServerSection as LoadedSection, ServerSetting, ServerText,
};
use std::{cell::Cell, rc::Rc};

mod search;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const RETAINED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SEARCH_BYTES: usize = 1024;
const FACT_KEYS: [&str; 4] = ["server_version", "encoding", "locale", "timezone"];
const FACT_LABELS: [&str; 4] = [
    "Server version",
    "Encoding",
    "Database LC_COLLATE",
    "Timezone",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerSection {
    Facts,
    Settings,
    Extensions,
}
impl ServerSection {
    pub const ALL: [Self; 3] = [Self::Facts, Self::Settings, Self::Extensions];
    pub fn index(self) -> usize {
        match self {
            Self::Facts => 0,
            Self::Settings => 1,
            Self::Extensions => 2,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Facts => "Server facts",
            Self::Settings => "Settings",
            Self::Extensions => "Extensions",
        }
    }
    pub fn headings(self) -> &'static str {
        match self {
            Self::Facts => "Fact · Reader-session value",
            Self::Settings => "Name · Category · Source",
            Self::Extensions => "Name · Schema",
        }
    }
}

pub fn validate_filter_query(query: &str) -> Result<(), &'static str> {
    if query.len() > MAX_SEARCH_BYTES || query.chars().any(char::is_control) {
        return Err("Settings search must fit 1 KiB and contain no control characters");
    }
    Ok(())
}

pub struct Capture {
    data: ServerDetailsSnapshot,
    budget: Rc<Cell<usize>>,
    visible_settings: Vec<usize>,
    query: String,
    non_default: bool,
}
impl Capture {
    pub fn new(data: ServerDetailsSnapshot, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if data
            .checked_heap_bytes()
            .is_none_or(|bytes| bytes > MAX_SERVER_DETAILS_BYTES)
            || crate::results::encoded_size(&data) > MAX_SERVER_DETAILS_BYTES
        {
            return Err(
                "Server capture exceeds its limits or has invalid identities; previous capture retained",
            );
        }
        if RETAINED_BYTES > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Server inspection needs 2 MiB of shared allowance; clear a capture or close another tool",
            );
        }
        // Admission precedes index/search allocations. The remaining 1 MiB
        // covers bounded indices, filter scratch and selected detail strings.
        budget.set(budget.get() + RETAINED_BYTES);
        let mut capture = Self {
            data,
            budget,
            visible_settings: Vec::new(),
            query: String::new(),
            non_default: false,
        };
        capture.visible_settings = (0..capture.settings().len()).collect();
        Ok(capture)
    }
    fn settings(&self) -> &[ServerSetting] {
        match &self.data.settings {
            LoadedSection::Loaded(rows) => &rows.rows,
            _ => &[],
        }
    }
    fn extensions(&self) -> &[ServerExtension] {
        match &self.data.extensions {
            LoadedSection::Loaded(rows) => &rows.rows,
            _ => &[],
        }
    }
    pub fn set_filter(&mut self, query: &str, non_default: bool) -> Result<(), &'static str> {
        validate_filter_query(query)?;
        let search = search::Search::new(query);
        let visible = self
            .settings()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let source_matches =
                    !non_default || (row.source != "default" && !row.inspection_override);
                let text_matches = search.matches(&row.name)
                    || search.matches(&row.category)
                    || search.matches(&row.source)
                    || matches!(&row.short_desc, ServerText::Value(value) if search.matches(value));
                (source_matches && text_matches).then_some(index)
            })
            .collect();
        self.visible_settings = visible;
        self.query = query.into();
        self.non_default = non_default;
        Ok(())
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn non_default(&self) -> bool {
        self.non_default
    }
    pub fn count(&self, section: ServerSection) -> usize {
        match section {
            ServerSection::Facts => {
                if matches!(self.data.facts, LoadedSection::Loaded(_)) {
                    4
                } else {
                    0
                }
            }
            ServerSection::Settings => self.visible_settings.len(),
            ServerSection::Extensions => self.extensions().len(),
        }
    }
    pub fn key(&self, section: ServerSection, index: usize) -> Option<&str> {
        match section {
            ServerSection::Facts => (index < self.count(section)).then(|| FACT_KEYS[index]),
            ServerSection::Settings => self
                .settings()
                .get(*self.visible_settings.get(index)?)
                .map(|row| row.name.as_str()),
            ServerSection::Extensions => self.extensions().get(index).map(|row| row.name.as_str()),
        }
    }
    pub fn index_for_key(&self, section: ServerSection, key: &str) -> Option<usize> {
        (0..self.count(section)).find(|index| self.key(section, *index) == Some(key))
    }
    fn fact(&self, index: usize) -> Option<&ServerText> {
        let LoadedSection::Loaded(facts) = &self.data.facts else {
            return None;
        };
        [
            &facts.server_version,
            &facts.encoding,
            &facts.locale,
            &facts.timezone,
        ]
        .get(index)
        .copied()
    }
    pub fn row_label(&self, section: ServerSection, index: usize) -> Option<String> {
        Some(match section {
            ServerSection::Facts => format!(
                "{} · {}",
                FACT_LABELS.get(index)?,
                summary(self.fact(index)?)
            ),
            ServerSection::Settings => {
                let row = self.settings().get(*self.visible_settings.get(index)?)?;
                format!(
                    "{} · {} · {}{}",
                    row.name,
                    row.category,
                    row.source,
                    if row.inspection_override {
                        " (inspection override)"
                    } else {
                        ""
                    }
                )
            }
            ServerSection::Extensions => {
                let row = self.extensions().get(index)?;
                format!("{} · {}", row.name, row.schema)
            }
        })
    }
    pub fn details(&self, section: ServerSection, index: usize) -> Option<String> {
        let detail = match section {
            ServerSection::Facts => format!(
                "{}:\n{}",
                FACT_LABELS.get(index)?,
                display(self.fact(index)?)
            ),
            ServerSection::Settings => {
                let row = self.settings().get(*self.visible_settings.get(index)?)?;
                format!(
                    "Name: {}\nCategory: {}\nSource: {}\nInspection override: {}\nSetting:\n{}\nUnit: {}\nDescription:\n{}\nBoot value:\n{}\nReset value:\n{}",
                    row.name,
                    row.category,
                    row.source,
                    row.inspection_override,
                    display(&row.setting),
                    display(&row.unit),
                    display(&row.short_desc),
                    display(&row.boot_val),
                    display(&row.reset_val)
                )
            }
            ServerSection::Extensions => {
                let row = self.extensions().get(index)?;
                format!(
                    "Name: {}\nSchema: {}\nVersion: {}\nDescription:\n{}",
                    row.name,
                    row.schema,
                    display(&row.version),
                    display(&row.description)
                )
            }
        };
        let result = format!("{}\n{}\n{detail}", self.interval(), self.reader_context());
        Some(if result.len() <= 96 * 1024 {
            result
        } else {
            "Selected details exceed 96 KiB; capture retained".into()
        })
    }
    pub fn interval(&self) -> String {
        format!(
            "Database {} · reader PID {} · collected {} to {}. Reader-session observations; readings may change during collection",
            self.data.database,
            self.data.reader_pid,
            self.data.collected_start,
            self.data.collected_end
        )
    }
    fn reader_context(&self) -> String {
        let reader = &self.data.reader;
        format!(
            "Current user: {}\nSession user: {}\nReader search path: {}\nInspection timeouts: statement {} ms; lock {} ms. These are inspection-session overrides, not SQL-tab or server-wide settings",
            display(&reader.current_user),
            display(&reader.session_user),
            display(&reader.search_path),
            reader.statement_timeout_ms,
            reader.lock_timeout_ms
        )
    }
    pub fn limits(&self, section: ServerSection) -> String {
        let mut notice = String::from(
            "Reader-session settings include configured role/search path and inspection timeout overrides. Non-default source excludes inspection overrides; it does not compare values to boot defaults.",
        );
        let section_notice = match section {
            ServerSection::Facts => section_state(&self.data.facts),
            ServerSection::Settings => rows_state(&self.data.settings),
            ServerSection::Extensions => rows_state(&self.data.extensions),
        };
        if !section_notice.is_empty() {
            notice.push(' ');
            notice.push_str(section_notice);
        }
        if section == ServerSection::Settings {
            notice.push_str(&format!(" Showing {} of {} captured settings. Search covers name, category, source and description; values are excluded.", self.visible_settings.len(), self.settings().len()));
        }
        notice.push_str(" NULL and omitted fields are shown explicitly in selected details.");
        notice
    }
    pub fn empty_label(&self, section: ServerSection) -> String {
        let state = match section {
            ServerSection::Facts => section_state(&self.data.facts),
            ServerSection::Settings => rows_state(&self.data.settings),
            ServerSection::Extensions => rows_state(&self.data.extensions),
        };
        if !state.is_empty() {
            return state.into();
        }
        if section == ServerSection::Settings && (!self.query.is_empty() || self.non_default) {
            "No captured settings match this filter".into()
        } else {
            "No rows in this captured section".into()
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(RETAINED_BYTES));
    }
}
fn section_state<T>(section: &LoadedSection<T>) -> &'static str {
    match section {
        LoadedSection::Loaded(_) => "",
        LoadedSection::Restricted => "Section restricted by server permissions",
        LoadedSection::Unavailable => "Section unavailable; absence of rows is unknown",
    }
}
fn rows_state<T>(section: &LoadedSection<ServerRows<T>>) -> &'static str {
    match section {
        LoadedSection::Loaded(rows) => match rows.limit {
            Some(ServerLimit::RowLimit) => {
                "Capture incomplete: row limit reached; undisplayed rows are unknown"
            }
            Some(ServerLimit::ByteLimit) => {
                "Capture incomplete: byte limit reached; undisplayed rows are unknown"
            }
            None => "",
        },
        _ => section_state(section),
    }
}
fn display(text: &ServerText) -> String {
    match text {
        ServerText::Value(value) => value.clone(),
        ServerText::Null => "NULL".into(),
        ServerText::Omitted { bytes } => format!("Omitted ({bytes} bytes; exceeds field limit)"),
    }
}
fn summary(text: &ServerText) -> String {
    match text {
        ServerText::Value(value) => {
            let mut chars = value.chars();
            let mut output: String = chars
                .by_ref()
                .take(256)
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect();
            if chars.next().is_some() {
                output.push_str("… (inspect full value)");
            }
            output
        }
        _ => display(text),
    }
}

#[cfg(test)]
mod tests;
