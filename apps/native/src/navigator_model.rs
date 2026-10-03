//! Persistent Navigator tree over one admitted catalog. Pure and bounded:
//! rows reference catalog indices, so object identity never comes from labels.
use crate::catalog::{Catalog, Kind};
use dbunk_lib::backend::objects::PgObjectKind;
use std::collections::HashSet;

/// Baseline `INITIAL_GROUP_LIMIT`; "Show more" lifts it for one group.
pub const INITIAL_GROUP_LIMIT: usize = 200;
/// Unquoted, so it cannot collide with any JSON-quoted schema ID.
const DATABASE_SCOPE: &str = "database";

struct Group {
    key: &'static str,
    label: &'static str,
    kind: Kind,
    truncation: &'static str,
}
const SCHEMA_GROUPS: [Group; 11] = [
    group("tables", "Tables", PgObjectKind::Table, "table"),
    group("views", "Views", PgObjectKind::View, "view"),
    group(
        "materializedViews",
        "Materialized Views",
        PgObjectKind::MaterializedView,
        "materialized-view",
    ),
    group(
        "foreignTables",
        "Foreign Tables",
        PgObjectKind::ForeignTable,
        "foreign-table",
    ),
    group("sequences", "Sequences", PgObjectKind::Sequence, "sequence"),
    group("functions", "Functions", PgObjectKind::Function, "function"),
    group(
        "procedures",
        "Procedures",
        PgObjectKind::Procedure,
        "procedure",
    ),
    group(
        "aggregates",
        "Aggregates",
        PgObjectKind::Aggregate,
        "aggregate",
    ),
    group("types", "Types", PgObjectKind::Type, "type"),
    group("domains", "Domains", PgObjectKind::Domain, "domain"),
    group(
        "extensions",
        "Extensions",
        PgObjectKind::Extension,
        "extension",
    ),
];
const DATABASE_GROUPS: [Group; 3] = [
    Group {
        key: "eventTriggers",
        label: "Event Triggers",
        kind: Kind::EventTrigger,
        truncation: "event-trigger",
    },
    Group {
        key: "roles",
        label: "Roles",
        kind: Kind::Role,
        truncation: "role",
    },
    Group {
        key: "tablespaces",
        label: "Tablespaces",
        kind: Kind::Tablespace,
        truncation: "tablespace",
    },
];
const fn group(
    key: &'static str,
    label: &'static str,
    kind: PgObjectKind,
    truncation: &'static str,
) -> Group {
    Group {
        key,
        label,
        kind: Kind::Object(kind),
        truncation,
    }
}
fn same_kind(left: Kind, right: Kind) -> bool {
    match (left, right) {
        (Kind::Object(left), Kind::Object(right)) => left == right,
        (Kind::EventTrigger, Kind::EventTrigger)
        | (Kind::Role, Kind::Role)
        | (Kind::Tablespace, Kind::Tablespace) => true,
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Schema {
        expanded: bool,
    },
    Database,
    Group {
        expanded: bool,
    },
    /// Index into `Catalog::rows`.
    Object(usize),
    ShowMore {
        remaining: usize,
    },
    Truncated,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub parent: Option<String>,
    pub level: usize,
    pub label: String,
    pub count: Option<usize>,
    pub kind: RowKind,
}

/// Expansion is view state for one connection's capture; it never authorizes
/// a read or a database action.
#[derive(Default)]
pub struct Tree {
    schemas: HashSet<String>,
    /// Tables default open; toggling records a departure from the default.
    toggled_groups: HashSet<String>,
    unlimited: HashSet<String>,
}
impl Tree {
    pub fn toggle(&mut self, row: &Row) {
        let set = match row.kind {
            RowKind::Schema { .. } => &mut self.schemas,
            RowKind::Group { .. } => &mut self.toggled_groups,
            _ => return,
        };
        if !set.remove(&row.id) {
            set.insert(row.id.clone());
        }
    }
    pub fn expand_schema(&mut self, name: &str) -> String {
        let id = schema_id(name);
        self.schemas.insert(id.clone());
        id
    }
    pub fn show_more(&mut self, row: &Row) {
        if let (RowKind::ShowMore { .. }, Some(parent)) = (&row.kind, &row.parent) {
            self.unlimited.insert(parent.clone());
        }
    }
    fn group_expanded(&self, id: &str, key: &str) -> bool {
        (key == "tables") != self.toggled_groups.contains(id)
    }
}

/// JSON quoting keeps names containing `:` or `$` from forging another row ID.
fn schema_id(name: &str) -> String {
    format!(
        "schema:{}",
        serde_json::to_string(name).unwrap_or_else(|_| name.to_owned())
    )
}
fn display_name(catalog: &Catalog, index: usize) -> String {
    let entry = &catalog.rows[index].entry;
    match &entry.identity_args {
        Some(args) => format!("{}({args})", entry.name),
        None => entry.name.clone(),
    }
}
fn clipped(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(256).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

/// Builds visible rows in baseline order. A non-empty filter expands every
/// matching branch without changing stored expansion.
pub fn rows(catalog: &Catalog, filter: &str, tree: &Tree) -> Vec<Row> {
    let needle = filter.trim().to_lowercase();
    let filtering = !needle.is_empty();
    let matches = |index: usize| {
        !filtering
            || display_name(catalog, index)
                .to_lowercase()
                .contains(&needle)
    };
    // Catalog rows arrive grouped: each schema row precedes its objects.
    let mut schemas: Vec<(usize, Vec<Vec<usize>>)> = Vec::new();
    let mut database: Vec<Vec<usize>> = vec![Vec::new(); DATABASE_GROUPS.len()];
    for (index, row) in catalog.rows.iter().enumerate() {
        if matches!(row.kind, Kind::Object(PgObjectKind::Schema)) {
            schemas.push((index, vec![Vec::new(); SCHEMA_GROUPS.len()]));
        } else if row.schema.is_some() {
            if let (Some((_, groups)), Some(slot)) = (
                schemas.last_mut(),
                SCHEMA_GROUPS
                    .iter()
                    .position(|group| same_kind(group.kind, row.kind)),
            ) {
                groups[slot].push(index);
            }
        } else if let Some(slot) = DATABASE_GROUPS
            .iter()
            .position(|group| same_kind(group.kind, row.kind))
        {
            database[slot].push(index);
        }
    }
    let truncated = |schema: Option<&str>, kind: &str| {
        catalog
            .truncated
            .iter()
            .any(|item| item.schema.as_deref() == schema && item.kind == kind)
    };
    let mut out = Vec::new();
    let push_entries = |out: &mut Vec<Row>,
                        group: &Group,
                        group_id: &str,
                        schema: Option<&str>,
                        entries: &[usize],
                        level: usize| {
        let limit = if tree.unlimited.contains(group_id) {
            entries.len()
        } else {
            entries.len().min(INITIAL_GROUP_LIMIT)
        };
        for &index in &entries[..limit] {
            let row = &catalog.rows[index];
            out.push(Row {
                // Overload identity, not display text, keeps IDs unique.
                id: format!(
                    "{group_id}:{}",
                    serde_json::to_string(&(
                        &row.entry.name,
                        row.entry.identity_args.as_deref().unwrap_or("")
                    ))
                    .unwrap_or_default()
                ),
                parent: Some(group_id.to_owned()),
                level,
                label: clipped(&display_name(catalog, index)),
                count: None,
                kind: RowKind::Object(index),
            });
        }
        if entries.len() > limit {
            let remaining = entries.len() - limit;
            out.push(Row {
                id: format!("{group_id}:show-more"),
                parent: Some(group_id.to_owned()),
                level,
                label: format!("Show {remaining} more"),
                count: None,
                kind: RowKind::ShowMore { remaining },
            });
        }
        if truncated(schema, group.truncation) {
            out.push(Row {
                id: format!("{group_id}:truncated"),
                parent: Some(group_id.to_owned()),
                level,
                label: format!("{} list cut at 2000 on the server", group.label),
                count: None,
                kind: RowKind::Truncated,
            });
        }
    };
    for (schema_index, groups) in &schemas {
        let name = &catalog.rows[*schema_index].entry.name;
        let schema_matches = !filtering || name.to_lowercase().contains(&needle);
        let visible: Vec<(usize, Vec<usize>)> = groups
            .iter()
            .enumerate()
            .map(|(slot, entries)| {
                let entries = if schema_matches {
                    entries.clone()
                } else {
                    entries.iter().copied().filter(|&i| matches(i)).collect()
                };
                (slot, entries)
            })
            .filter(|(_, entries)| !entries.is_empty())
            .collect();
        if filtering && !schema_matches && visible.is_empty() {
            continue;
        }
        let schema_id = schema_id(name);
        let expanded = filtering || tree.schemas.contains(&schema_id);
        out.push(Row {
            id: schema_id.clone(),
            parent: None,
            level: 1,
            label: clipped(name),
            count: Some(groups.iter().map(Vec::len).sum()),
            kind: RowKind::Schema { expanded },
        });
        if !expanded {
            continue;
        }
        for (slot, entries) in visible {
            let group = &SCHEMA_GROUPS[slot];
            let group_id = format!("{schema_id}:{}", group.key);
            let group_expanded = filtering || tree.group_expanded(&group_id, group.key);
            out.push(Row {
                id: group_id.clone(),
                parent: Some(schema_id.clone()),
                level: 2,
                label: group.label.into(),
                count: Some(entries.len()),
                kind: RowKind::Group {
                    expanded: group_expanded,
                },
            });
            if group_expanded {
                push_entries(&mut out, group, &group_id, Some(name), &entries, 3);
            }
        }
    }
    let database_groups: Vec<(usize, Vec<usize>)> = database
        .iter()
        .enumerate()
        .map(|(slot, entries)| {
            (
                slot,
                entries.iter().copied().filter(|&i| matches(i)).collect(),
            )
        })
        .filter(|(_, entries): &(usize, Vec<usize>)| !entries.is_empty())
        .collect();
    if !database_groups.is_empty() {
        let database_id = DATABASE_SCOPE.to_owned();
        out.push(Row {
            id: database_id.clone(),
            parent: None,
            level: 1,
            label: "Database objects".into(),
            count: Some(database_groups.iter().map(|(_, e)| e.len()).sum()),
            kind: RowKind::Database,
        });
        for (slot, entries) in database_groups {
            let group = &DATABASE_GROUPS[slot];
            let group_id = format!("{database_id}:{}", group.key);
            let group_expanded = filtering || tree.group_expanded(&group_id, group.key);
            out.push(Row {
                id: group_id.clone(),
                parent: Some(database_id.clone()),
                level: 2,
                label: group.label.into(),
                count: Some(entries.len()),
                kind: RowKind::Group {
                    expanded: group_expanded,
                },
            });
            if group_expanded {
                push_entries(&mut out, group, &group_id, None, &entries, 3);
            }
        }
    }
    if truncated(None, "schema") {
        out.push(Row {
            id: "schemas:truncated".into(),
            parent: None,
            level: 1,
            label: "Schema list cut at 2000 on the server".into(),
            count: None,
            kind: RowKind::Truncated,
        });
    }
    out
}

/// Arrow keys follow the baseline tree: Right expands or descends, Left
/// collapses or moves to the parent.
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}
pub enum Move {
    Select(usize),
    Toggle(usize),
}
pub fn navigate(rows: &[Row], selected: usize, key: Key) -> Option<Move> {
    if rows.is_empty() {
        return None;
    }
    let selected = selected.min(rows.len() - 1);
    let expanded = |row: &Row| match row.kind {
        RowKind::Schema { expanded } | RowKind::Group { expanded } => Some(expanded),
        _ => None,
    };
    Some(match key {
        Key::Up => Move::Select(selected.saturating_sub(1)),
        Key::Down => Move::Select((selected + 1).min(rows.len() - 1)),
        Key::Home => Move::Select(0),
        Key::End => Move::Select(rows.len() - 1),
        Key::Right => match expanded(&rows[selected]) {
            Some(false) => Move::Toggle(selected),
            _ => Move::Select((selected + 1).min(rows.len() - 1)),
        },
        Key::Left => match expanded(&rows[selected]) {
            Some(true) => Move::Toggle(selected),
            _ => Move::Select(
                rows[selected]
                    .parent
                    .as_ref()
                    .and_then(|parent| rows.iter().position(|row| &row.id == parent))
                    .unwrap_or(selected),
            ),
        },
    })
}

/// Case-insensitive type-ahead from the row after `selected`, wrapping.
pub fn type_ahead(rows: &[Row], selected: usize, prefix: &str, restart: bool) -> Option<usize> {
    let prefix = prefix.to_lowercase();
    let start = if restart { selected + 1 } else { selected };
    (0..rows.len())
        .map(|step| (start + step) % rows.len())
        .find(|&index| rows[index].label.to_lowercase().starts_with(&prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::objects::{
        PgCatalogEntry, PgCatalogTruncation, PgObjectCatalog, PgSchemaObjects,
    };
    use std::{cell::Cell, rc::Rc};

    fn entry(name: &str, args: Option<&str>) -> PgCatalogEntry {
        PgCatalogEntry {
            name: name.into(),
            identity_args: args.map(Into::into),
            comment: None,
            type_class: None,
        }
    }
    fn schema(name: &str) -> PgSchemaObjects {
        PgSchemaObjects {
            name: name.into(),
            tables: vec![],
            views: vec![],
            materialized_views: vec![],
            foreign_tables: vec![],
            sequences: vec![],
            functions: vec![],
            procedures: vec![],
            aggregates: vec![],
            types: vec![],
            domains: vec![],
            extensions: vec![],
        }
    }
    fn catalog(schemas: Vec<PgSchemaObjects>, truncated: Vec<PgCatalogTruncation>) -> Catalog {
        Catalog::new(
            PgObjectCatalog {
                schemas,
                event_triggers: vec![],
                roles: vec![entry("app_owner", None)],
                tablespaces: vec![],
                truncated,
            },
            Rc::new(Cell::new(0)),
        )
        .unwrap()
    }
    fn fixture() -> Catalog {
        let mut public = schema("public");
        public.tables = vec![entry("orders", None), entry("Ünïcode", None)];
        public.views = vec![entry("recent_orders", None)];
        public.functions = vec![
            entry("total", Some("integer")),
            entry("total", Some("text")),
        ];
        let mut audit = schema("audit");
        audit.tables = vec![entry("events", None)];
        catalog(vec![audit, public], vec![])
    }
    fn labels(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn collapsed_by_default_with_tables_open_once_schema_expands() {
        let catalog = fixture();
        let mut tree = Tree::default();
        let initial = rows(&catalog, "", &tree);
        assert_eq!(
            labels(&initial),
            ["audit", "public", "Database objects", "Roles"]
        );
        assert_eq!(initial[1].count, Some(5));
        tree.toggle(&initial[1]);
        let expanded = rows(&catalog, "", &tree);
        assert_eq!(
            labels(&expanded),
            [
                "audit",
                "public",
                "Tables",
                "orders",
                "Ünïcode",
                "Views",
                "Functions",
                "Database objects",
                "Roles"
            ]
        );
        let functions = expanded
            .iter()
            .find(|row| row.label == "Functions")
            .unwrap();
        tree.toggle(functions);
        let with_functions = rows(&catalog, "", &tree);
        let overloads: Vec<_> = with_functions
            .iter()
            .filter(|row| row.label.starts_with("total("))
            .collect();
        assert_eq!(overloads.len(), 2);
        assert_ne!(overloads[0].id, overloads[1].id);
        let RowKind::Object(index) = overloads[1].kind else {
            panic!("object row");
        };
        let reference = catalog.rows[index].reference().unwrap();
        assert_eq!(reference.identity_args.as_deref(), Some("text"));
        assert_eq!(reference.schema.as_deref(), Some("public"));
    }

    #[test]
    fn filter_expands_matches_without_changing_stored_expansion() {
        let catalog = fixture();
        let tree = Tree::default();
        let filtered = rows(&catalog, "  ÜNÏ ", &tree);
        assert_eq!(labels(&filtered), ["public", "Tables", "Ünïcode"]);
        assert!(matches!(
            filtered[0].kind,
            RowKind::Schema { expanded: true }
        ));
        // A schema-name match shows all its groups, collapsed groups included.
        let by_schema = rows(&catalog, "audit", &tree);
        assert_eq!(labels(&by_schema), ["audit", "Tables", "events"]);
        assert!(rows(&catalog, "no such object", &tree).is_empty());
        assert_eq!(rows(&catalog, "", &tree).len(), 4);
    }

    #[test]
    fn group_limit_show_more_and_server_truncation_are_explicit() {
        let mut big = schema("big");
        big.tables = (0..INITIAL_GROUP_LIMIT + 5)
            .map(|i| entry(&format!("t{i:04}"), None))
            .collect();
        let catalog = catalog(
            vec![big],
            vec![
                PgCatalogTruncation {
                    schema: Some("big".into()),
                    kind: "table".into(),
                },
                PgCatalogTruncation {
                    schema: None,
                    kind: "schema".into(),
                },
            ],
        );
        let mut tree = Tree::default();
        let first = rows(&catalog, "", &tree);
        tree.toggle(&first[0]);
        let limited = rows(&catalog, "", &tree);
        let objects = limited
            .iter()
            .filter(|row| matches!(row.kind, RowKind::Object(_)))
            .count();
        assert_eq!(objects, INITIAL_GROUP_LIMIT);
        let more = limited
            .iter()
            .find(|row| matches!(row.kind, RowKind::ShowMore { remaining: 5 }))
            .unwrap();
        assert!(
            limited
                .iter()
                .any(|row| row.label == "Tables list cut at 2000 on the server")
        );
        assert_eq!(
            limited.last().unwrap().label,
            "Schema list cut at 2000 on the server"
        );
        tree.show_more(more);
        let all = rows(&catalog, "", &tree);
        assert_eq!(
            all.iter()
                .filter(|row| matches!(row.kind, RowKind::Object(_)))
                .count(),
            INITIAL_GROUP_LIMIT + 5
        );
        assert!(
            !all.iter()
                .any(|row| matches!(row.kind, RowKind::ShowMore { .. }))
        );
    }

    #[test]
    fn hostile_schema_names_cannot_forge_row_ids() {
        let mut first = schema("a:tables");
        first.tables = vec![entry("x", None)];
        let mut second = schema("a");
        second.tables = vec![entry("y", None)];
        let catalog = catalog(
            vec![first, second, schema("database"), schema("$database")],
            vec![],
        );
        let mut tree = Tree::default();
        for name in ["a:tables", "a", "database", "$database"] {
            tree.expand_schema(name);
        }
        let rows = rows(&catalog, "", &tree);
        let mut ids: Vec<_> = rows.iter().map(|row| row.id.as_str()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate navigator row IDs");
    }

    #[test]
    fn keyboard_moves_follow_tree_structure() {
        let catalog = fixture();
        let mut tree = Tree::default();
        let collapsed = rows(&catalog, "", &tree);
        assert!(matches!(
            navigate(&collapsed, 1, Key::Right),
            Some(Move::Toggle(1))
        ));
        tree.toggle(&collapsed[1]);
        let expanded = rows(&catalog, "", &tree);
        // From "orders" Left goes to its group, then the group collapses.
        assert!(matches!(
            navigate(&expanded, 3, Key::Left),
            Some(Move::Select(2))
        ));
        assert!(matches!(
            navigate(&expanded, 2, Key::Left),
            Some(Move::Toggle(2))
        ));
        assert!(matches!(
            navigate(&expanded, 99, Key::Down),
            Some(Move::Select(8))
        ));
        assert!(matches!(
            navigate(&expanded, 0, Key::Up),
            Some(Move::Select(0))
        ));
        assert!(matches!(
            navigate(&expanded, 4, Key::End),
            Some(Move::Select(8))
        ));
        assert!(navigate(&[], 0, Key::Down).is_none());
        assert_eq!(type_ahead(&expanded, 0, "p", true), Some(1));
        assert_eq!(type_ahead(&expanded, 1, "pu", false), Some(1));
        assert_eq!(type_ahead(&expanded, 3, "o", true), Some(3));
        assert_eq!(type_ahead(&expanded, 0, "zzz", true), None);
    }
}
