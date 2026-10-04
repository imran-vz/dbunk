//! Sidebar tree over one ClickHouse catalog: databases → tables, views,
//! materialized views and dictionaries, each kind its own group. Pure and
//! bounded; rows carry the identity they open, never a parsed label.
use dbunk_lib::backend::clickhouse::{ClickHouseCatalog, ClickHouseDatabase};
use std::collections::HashSet;

/// Objects shown per group before a "filter to see more" note.
pub const GROUP_LIMIT: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Dictionary,
}

impl ObjectKind {
    pub const ALL: [Self; 4] = [
        Self::Table,
        Self::View,
        Self::MaterializedView,
        Self::Dictionary,
    ];
    pub fn group_label(self) -> &'static str {
        match self {
            Self::Table => "Tables",
            Self::View => "Views",
            Self::MaterializedView => "Materialized Views",
            Self::Dictionary => "Dictionaries",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::View => "view",
            Self::MaterializedView => "materialized view",
            Self::Dictionary => "dictionary",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Database {
        expanded: bool,
    },
    Group {
        kind: ObjectKind,
        expanded: bool,
    },
    Object {
        kind: ObjectKind,
        name: String,
    },
    /// Informational row: truncation, unlisted database, unreadable source.
    Note {
        warning: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    /// Database the row belongs to; empty for server-level rows.
    pub database: String,
    pub level: usize,
    pub label: String,
    /// Engine, materialized-view target, dictionary status or a count.
    pub detail: Option<String>,
    pub kind: RowKind,
}

impl Row {
    pub fn expandable(&self) -> bool {
        matches!(self.kind, RowKind::Database { .. } | RowKind::Group { .. })
    }
}

/// Expansion state, kept across reloads of the same connection.
#[derive(Default)]
pub struct TreeState {
    /// Expanded databases. `None` until the first catalog chooses a default.
    databases: Option<HashSet<String>>,
    /// Collapsed groups (`database\0kind`); groups start expanded.
    collapsed: HashSet<String>,
}

fn database_key(name: &str) -> String {
    format!("db\0{name}")
}
fn group_key(database: &str, kind: ObjectKind) -> String {
    format!("group\0{database}\0{kind:?}")
}

impl TreeState {
    /// The first catalog expands its first user database.
    pub fn adopt(&mut self, catalog: &ClickHouseCatalog) {
        if self.databases.is_none() {
            self.databases = Some(
                catalog
                    .databases
                    .iter()
                    .find(|database| !database.system && database.listed)
                    .map(|database| database.name.clone())
                    .into_iter()
                    .collect(),
            );
        }
    }

    /// Toggles a database or group row; other rows are unchanged.
    pub fn toggle(&mut self, row: &Row) -> bool {
        match row.kind {
            RowKind::Database { expanded } => {
                let databases = self.databases.get_or_insert_with(HashSet::new);
                if expanded {
                    databases.remove(&row.database);
                } else {
                    databases.insert(row.database.clone());
                }
                true
            }
            RowKind::Group { kind, expanded } => {
                let key = group_key(&row.database, kind);
                if expanded {
                    self.collapsed.insert(key);
                } else {
                    self.collapsed.remove(&key);
                }
                true
            }
            _ => false,
        }
    }

    fn database_open(&self, name: &str) -> bool {
        self.databases
            .as_ref()
            .is_some_and(|databases| databases.contains(name))
    }

    /// Visible rows. A non-empty filter (case-insensitive substring of the
    /// object or database name) expands every match and hides the rest.
    pub fn rows(&self, catalog: &ClickHouseCatalog, filter: &str) -> Vec<Row> {
        let filter = filter.trim().to_lowercase();
        let filtering = !filter.is_empty();
        let mut rows = Vec::new();
        if catalog.truncated {
            rows.push(note(
                "truncated",
                "",
                0,
                "Catalog stopped at its bound; filter or query system.tables".into(),
                true,
            ));
        }
        for database in &catalog.databases {
            let database_match = filtering && database.name.to_lowercase().contains(&filter);
            let groups = groups(database);
            let matches =
                |name: &str| !filtering || database_match || name.to_lowercase().contains(&filter);
            if filtering
                && !database_match
                && !groups
                    .iter()
                    .any(|(_, objects)| objects.iter().any(|(name, _)| matches(name)))
            {
                continue;
            }
            let expanded = filtering || self.database_open(&database.name);
            let total = groups
                .iter()
                .map(|(_, objects)| objects.len())
                .sum::<usize>();
            rows.push(Row {
                key: database_key(&database.name),
                database: database.name.clone(),
                level: 0,
                label: database.name.clone(),
                detail: Some(if database.listed {
                    total.to_string()
                } else {
                    database.engine.clone()
                }),
                kind: RowKind::Database { expanded },
            });
            if !expanded {
                continue;
            }
            if !database.listed {
                rows.push(note(
                    "unlisted",
                    &database.name,
                    1,
                    format!("{} database; objects are not listed", database.engine),
                    false,
                ));
                continue;
            }
            for (kind, objects) in groups {
                let visible = objects
                    .iter()
                    .filter(|(name, _)| matches(name))
                    .collect::<Vec<_>>();
                if visible.is_empty() {
                    continue;
                }
                let open = filtering || !self.collapsed.contains(&group_key(&database.name, kind));
                rows.push(Row {
                    key: group_key(&database.name, kind),
                    database: database.name.clone(),
                    level: 1,
                    label: kind.group_label().into(),
                    detail: Some(visible.len().to_string()),
                    kind: RowKind::Group {
                        kind,
                        expanded: open,
                    },
                });
                if !open {
                    continue;
                }
                for (name, detail) in visible.iter().take(GROUP_LIMIT) {
                    rows.push(Row {
                        key: format!("obj\0{}\0{kind:?}\0{name}", database.name),
                        database: database.name.clone(),
                        level: 2,
                        label: (*name).clone(),
                        detail: detail.clone(),
                        kind: RowKind::Object {
                            kind,
                            name: (*name).clone(),
                        },
                    });
                }
                if visible.len() > GROUP_LIMIT {
                    rows.push(note(
                        "more",
                        &database.name,
                        2,
                        format!(
                            "{} more {}s; filter to find them",
                            visible.len() - GROUP_LIMIT,
                            kind.label()
                        ),
                        false,
                    ));
                }
            }
        }
        if !catalog.config_dictionaries.is_empty() {
            let visible = catalog
                .config_dictionaries
                .iter()
                .filter(|dictionary| !filtering || dictionary.name.to_lowercase().contains(&filter))
                .take(GROUP_LIMIT)
                .collect::<Vec<_>>();
            if !visible.is_empty() {
                rows.push(note(
                    "config",
                    "",
                    0,
                    format!("Server-config dictionaries: {}", visible.len()),
                    false,
                ));
                for dictionary in visible {
                    rows.push(Row {
                        key: format!("config\0{}", dictionary.name),
                        database: String::new(),
                        level: 1,
                        label: dictionary.name.clone(),
                        detail: dictionary.status.clone(),
                        kind: RowKind::Object {
                            kind: ObjectKind::Dictionary,
                            name: dictionary.name.clone(),
                        },
                    });
                }
            }
        }
        if let Some(error) = &catalog.dictionaries_error {
            rows.push(note(
                "dictionaries",
                "",
                0,
                format!("Dictionary status unavailable: {}", first_line(error)),
                true,
            ));
        }
        rows
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}

fn note(key: &str, database: &str, level: usize, label: String, warning: bool) -> Row {
    Row {
        key: format!("note\0{key}\0{database}"),
        database: database.into(),
        level,
        label,
        detail: None,
        kind: RowKind::Note { warning },
    }
}

type Objects = Vec<(String, Option<String>)>;

fn groups(database: &ClickHouseDatabase) -> Vec<(ObjectKind, Objects)> {
    ObjectKind::ALL
        .into_iter()
        .map(|kind| {
            let objects = match kind {
                ObjectKind::Table => database
                    .tables
                    .iter()
                    .map(|table| (table.name.clone(), Some(table.engine.clone())))
                    .collect(),
                ObjectKind::View => database
                    .views
                    .iter()
                    .map(|view| {
                        let engine = (view.engine != "View").then(|| view.engine.clone());
                        (view.name.clone(), engine)
                    })
                    .collect(),
                ObjectKind::MaterializedView => database
                    .materialized_views
                    .iter()
                    .map(|view| {
                        (
                            view.name.clone(),
                            view.target.as_ref().map(|target| format!("→ {target}")),
                        )
                    })
                    .collect(),
                ObjectKind::Dictionary => database
                    .dictionaries
                    .iter()
                    .map(|dictionary| (dictionary.name.clone(), dictionary.status.clone()))
                    .collect(),
            };
            (kind, objects)
        })
        .collect()
}

/// Index of the nearest expandable ancestor of `index`, for Left.
pub fn parent(rows: &[Row], index: usize) -> Option<usize> {
    let level = rows.get(index)?.level;
    (0..index)
        .rev()
        .find(|&candidate| rows[candidate].level < level && rows[candidate].expandable())
}

/// Index of the row with `key` after a rebuild, else a clamped position.
pub fn reselect(rows: &[Row], key: Option<&str>, fallback: usize) -> usize {
    key.and_then(|key| rows.iter().position(|row| row.key == key))
        .unwrap_or_else(|| fallback.min(rows.len().saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::clickhouse::{
        ClickHouseDictionary, ClickHouseMaterializedView, ClickHouseTable,
    };

    fn table(name: &str, engine: &str) -> ClickHouseTable {
        ClickHouseTable {
            name: name.into(),
            engine: engine.into(),
        }
    }

    fn catalog() -> ClickHouseCatalog {
        ClickHouseCatalog {
            databases: vec![
                ClickHouseDatabase {
                    name: "analytics".into(),
                    engine: "Atomic".into(),
                    listed: true,
                    tables: vec![
                        table("daily", "SummingMergeTree"),
                        table("events", "Distributed"),
                    ],
                    views: vec![table("recent", "View")],
                    materialized_views: vec![ClickHouseMaterializedView {
                        name: "daily_mv".into(),
                        target: Some("analytics.daily".into()),
                    }],
                    dictionaries: vec![ClickHouseDictionary {
                        name: "geo".into(),
                        status: Some("LOADED".into()),
                    }],
                    ..Default::default()
                },
                ClickHouseDatabase {
                    name: "pg".into(),
                    engine: "PostgreSQL".into(),
                    listed: false,
                    ..Default::default()
                },
                ClickHouseDatabase {
                    name: "system".into(),
                    engine: "Atomic".into(),
                    system: true,
                    listed: true,
                    tables: vec![table("parts", "SystemParts")],
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| format!("{}{}", "  ".repeat(row.level), row.label))
            .collect()
    }

    #[test]
    fn first_user_database_opens_with_each_kind_in_its_own_group() {
        let catalog = catalog();
        let mut state = TreeState::default();
        state.adopt(&catalog);
        let rows = state.rows(&catalog, "");
        assert_eq!(
            labels(&rows),
            [
                "analytics",
                "  Tables",
                "    daily",
                "    events",
                "  Views",
                "    recent",
                "  Materialized Views",
                "    daily_mv",
                "  Dictionaries",
                "    geo",
                "pg",
                "system",
            ]
        );
        assert_eq!(rows[2].detail.as_deref(), Some("SummingMergeTree"));
        assert_eq!(rows[7].detail.as_deref(), Some("→ analytics.daily"));
        assert_eq!(rows[9].detail.as_deref(), Some("LOADED"));
        assert_eq!(rows[10].detail.as_deref(), Some("PostgreSQL"));
        assert_eq!(
            rows[7].kind,
            RowKind::Object {
                kind: ObjectKind::MaterializedView,
                name: "daily_mv".into()
            }
        );
        // A later reload keeps the user's expansion instead of re-defaulting.
        state.toggle(&rows[0]);
        state.adopt(&catalog);
        assert_eq!(state.rows(&catalog, "").len(), 3);
    }

    #[test]
    fn groups_collapse_and_unlisted_databases_explain_themselves() {
        let catalog = catalog();
        let mut state = TreeState::default();
        state.adopt(&catalog);
        let rows = state.rows(&catalog, "");
        state.toggle(&rows[1]);
        state.toggle(&rows[10]);
        let rows = state.rows(&catalog, "");
        assert_eq!(
            rows[1].kind,
            RowKind::Group {
                kind: ObjectKind::Table,
                expanded: false
            }
        );
        assert_eq!(rows[2].label, "Views");
        let unlisted = rows.iter().position(|row| row.label == "pg").unwrap();
        assert!(rows[unlisted + 1].label.contains("not listed"));
        assert_eq!(parent(&rows, unlisted + 1), Some(unlisted));
    }

    #[test]
    fn filter_expands_matches_and_hides_everything_else() {
        let catalog = catalog();
        let state = TreeState::default();
        assert_eq!(
            labels(&state.rows(&catalog, "DAI")),
            [
                "analytics",
                "  Tables",
                "    daily",
                "  Materialized Views",
                "    daily_mv"
            ]
        );
        assert_eq!(
            labels(&state.rows(&catalog, "system")),
            ["system", "  Tables", "    parts"]
        );
        assert!(state.rows(&catalog, "nothing-matches").is_empty());
    }

    #[test]
    fn bounds_and_unreadable_sources_are_visible_rows() {
        let mut catalog = catalog();
        catalog.truncated = true;
        catalog.dictionaries_error = Some("Code: 497. Not enough privileges\nstack".into());
        catalog.databases[0].tables = (0..GROUP_LIMIT + 5)
            .map(|n| table(&format!("t{n:05}"), "MergeTree"))
            .collect();
        let mut state = TreeState::default();
        state.adopt(&catalog);
        let rows = state.rows(&catalog, "");
        assert_eq!(rows[0].kind, RowKind::Note { warning: true });
        assert!(
            rows.iter()
                .any(|row| row.label == "5 more tables; filter to find them")
        );
        let last = rows.last().unwrap();
        assert_eq!(
            last.label,
            "Dictionary status unavailable: Code: 497. Not enough privileges"
        );
        assert_eq!(reselect(&rows, Some(&rows[3].key), 0), 3);
        assert_eq!(reselect(&rows, Some("gone"), 99_999), rows.len() - 1);
    }
}
