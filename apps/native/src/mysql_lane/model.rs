//! MySQL sidebar tree as plain data: databases → tables, views, routines,
//! events, triggers. Objects load per database on first expansion; the
//! flattened rows feed a virtualized list.
use crate::document_view::ConnectionPhase;
use dbunk_lib::backend::mysql_sessions::{MySqlObjectKind, MySqlObjects, MySqlRoutineKind};
use std::collections::{BTreeMap, BTreeSet};

/// Connect and disconnect bookkeeping. Every attempt and every disconnect
/// bumps the epoch, and a reply applies only while its epoch is current, so
/// a late session from an abandoned attempt is closed instead of adopted.
/// Nothing here ever starts an attempt on its own.
pub struct Lifecycle {
    epoch: u64,
    phase: ConnectionPhase,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            epoch: 0,
            phase: ConnectionPhase::Idle,
        }
    }
}

impl Lifecycle {
    pub fn phase(&self) -> &ConnectionPhase {
        &self.phase
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Starts an attempt, unless one is open or opening.
    pub fn begin(&mut self) -> Option<u64> {
        if matches!(
            self.phase,
            ConnectionPhase::Connecting | ConnectionPhase::Connected
        ) {
            return None;
        }
        self.epoch += 1;
        self.phase = ConnectionPhase::Connecting;
        Some(self.epoch)
    }

    /// An attempt finished. False when it is stale: its session must be
    /// closed, not used.
    pub fn opened(&mut self, epoch: u64, result: Result<(), String>) -> bool {
        if epoch != self.epoch || self.phase != ConnectionPhase::Connecting {
            return false;
        }
        match result {
            Ok(()) => {
                self.phase = ConnectionPhase::Connected;
                true
            }
            Err(error) => {
                self.phase = ConnectionPhase::Failed(error);
                false
            }
        }
    }

    /// The backend closed the current session; `None` is a normal close.
    pub fn closed(&mut self, epoch: u64, reason: Option<String>) -> bool {
        if epoch != self.epoch || self.phase != ConnectionPhase::Connected {
            return false;
        }
        self.epoch += 1;
        self.phase = match reason {
            Some(reason) => ConnectionPhase::Failed(reason),
            None => ConnectionPhase::Idle,
        };
        true
    }

    /// User disconnect: abandons any attempt and the open session.
    pub fn disconnect(&mut self) {
        self.epoch += 1;
        self.phase = ConnectionPhase::Idle;
    }

    pub fn is_current(&self, epoch: u64) -> bool {
        epoch == self.epoch
    }
}

/// One asynchronous value.
#[derive(Clone, Debug, PartialEq)]
pub enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Tables,
    Views,
    Routines,
    Events,
    Triggers,
}

impl Group {
    pub const ALL: [Group; 5] = [
        Group::Tables,
        Group::Views,
        Group::Routines,
        Group::Events,
        Group::Triggers,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::Tables => "Tables",
            Group::Views => "Views",
            Group::Routines => "Routines",
            Group::Events => "Events",
            Group::Triggers => "Triggers",
        }
    }
}

/// An object a document can open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRef {
    pub database: String,
    pub kind: MySqlObjectKind,
    pub name: String,
}

impl ObjectRef {
    /// Tables and views browse rows; everything else shows its definition.
    pub fn has_rows(&self) -> bool {
        matches!(self.kind, MySqlObjectKind::Table | MySqlObjectKind::View)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Database {
        expanded: bool,
    },
    Group {
        database: String,
        group: Group,
        expanded: bool,
    },
    Object(ObjectRef),
    /// Loading, empty, failure or truncation note under a node.
    Note {
        text: String,
        error: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub depth: u8,
    pub label: String,
    /// Secondary text (trigger table, routine kind).
    pub detail: Option<String>,
    pub count: Option<usize>,
    pub kind: RowKind,
}

#[derive(Default)]
pub struct Tree {
    pub databases: Option<Load<(Vec<String>, bool)>>,
    objects: BTreeMap<String, Load<MySqlObjects>>,
    expanded: BTreeSet<String>,
    groups: BTreeSet<(String, Group)>,
}

impl Tree {
    /// Clears everything for a new session.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Databases arrived; the connection's default database opens with its
    /// tables so the common case needs no clicks.
    pub fn set_databases(
        &mut self,
        result: Result<(Vec<String>, bool), String>,
        default: Option<&str>,
    ) -> Option<String> {
        // Only the first load opens the default; a refresh keeps the user's
        // expansion as it was.
        let first = matches!(self.databases, None | Some(Load::Failed(_)));
        match result {
            Ok(list) => {
                let open = default
                    .filter(|default| first && list.0.iter().any(|name| name == default))
                    .map(str::to_owned);
                self.databases = Some(Load::Ready(list));
                let open = open?;
                self.expanded.insert(open.clone());
                self.groups.insert((open.clone(), Group::Tables));
                if self.objects.contains_key(&open) {
                    return None;
                }
                self.objects.insert(open.clone(), Load::Loading);
                Some(open)
            }
            Err(error) => {
                self.databases = Some(Load::Failed(error));
                None
            }
        }
    }

    pub fn set_objects(&mut self, database: &str, result: Result<MySqlObjects, String>) {
        self.objects.insert(
            database.to_owned(),
            match result {
                Ok(objects) => Load::Ready(objects),
                Err(error) => Load::Failed(error),
            },
        );
    }

    /// Toggles a database; returns it when its objects must be loaded.
    pub fn toggle_database(&mut self, database: &str) -> Option<String> {
        if !self.expanded.remove(database) {
            self.expanded.insert(database.to_owned());
            if matches!(
                self.objects.get(database),
                None | Some(Load::Failed(_) | Load::Idle)
            ) {
                self.objects.insert(database.to_owned(), Load::Loading);
                return Some(database.to_owned());
            }
        }
        None
    }

    pub fn toggle_group(&mut self, database: &str, group: Group) {
        let key = (database.to_owned(), group);
        if !self.groups.remove(&key) {
            self.groups.insert(key);
        }
    }

    /// Marks loaded databases stale and returns the expanded ones to reload.
    pub fn refresh(&mut self) -> Vec<String> {
        self.databases = Some(Load::Loading);
        self.objects.clear();
        let open: Vec<String> = self.expanded.iter().cloned().collect();
        for database in &open {
            self.objects.insert(database.clone(), Load::Loading);
        }
        open
    }

    /// Databases known to the tree, for pickers.
    pub fn database_names(&self) -> &[String] {
        match &self.databases {
            Some(Load::Ready((names, _))) => names,
            _ => &[],
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let note = |depth, text: &str, error| Row {
            depth,
            label: text.to_owned(),
            detail: None,
            count: None,
            kind: RowKind::Note {
                text: text.to_owned(),
                error,
            },
        };
        let (names, truncated) = match &self.databases {
            None | Some(Load::Idle) => return rows,
            Some(Load::Loading) => {
                rows.push(note(0, "Loading databases…", false));
                return rows;
            }
            Some(Load::Failed(error)) => {
                rows.push(note(0, error, true));
                return rows;
            }
            Some(Load::Ready((names, truncated))) => (names, *truncated),
        };
        if names.is_empty() {
            rows.push(note(0, "No databases visible to this account", false));
        }
        for database in names {
            let expanded = self.expanded.contains(database);
            rows.push(Row {
                depth: 0,
                label: database.clone(),
                detail: None,
                count: None,
                kind: RowKind::Database { expanded },
            });
            if !expanded {
                continue;
            }
            match self.objects.get(database) {
                None | Some(Load::Idle | Load::Loading) => {
                    rows.push(note(1, "Loading objects…", false))
                }
                Some(Load::Failed(error)) => rows.push(note(1, error, true)),
                Some(Load::Ready(objects)) => self.object_rows(database, objects, &mut rows),
            }
        }
        if truncated {
            rows.push(note(0, "More databases exist than are listed", true));
        }
        rows
    }

    fn object_rows(&self, database: &str, objects: &MySqlObjects, rows: &mut Vec<Row>) {
        for group in Group::ALL {
            let items = group_items(database, objects, group);
            let expanded = self.groups.contains(&(database.to_owned(), group));
            rows.push(Row {
                depth: 1,
                label: group.label().into(),
                detail: None,
                count: Some(items.len()),
                kind: RowKind::Group {
                    database: database.to_owned(),
                    group,
                    expanded,
                },
            });
            if expanded {
                rows.extend(items.into_iter().map(|(object, detail)| Row {
                    depth: 2,
                    label: object.name.clone(),
                    detail,
                    count: None,
                    kind: RowKind::Object(object),
                }));
            }
        }
        if objects.truncated {
            rows.push(Row {
                depth: 1,
                label: "Some lists were cut at the catalog limit".into(),
                detail: None,
                count: None,
                kind: RowKind::Note {
                    text: "Some lists were cut at the catalog limit".into(),
                    error: true,
                },
            });
        }
    }
}

fn group_items(
    database: &str,
    objects: &MySqlObjects,
    group: Group,
) -> Vec<(ObjectRef, Option<String>)> {
    let object = |kind, name: &String| ObjectRef {
        database: database.to_owned(),
        kind,
        name: name.clone(),
    };
    match group {
        Group::Tables => objects
            .tables
            .iter()
            .map(|name| (object(MySqlObjectKind::Table, name), None))
            .collect(),
        Group::Views => objects
            .views
            .iter()
            .map(|name| (object(MySqlObjectKind::View, name), None))
            .collect(),
        Group::Routines => objects
            .routines
            .iter()
            .map(|routine| match routine.kind {
                MySqlRoutineKind::Procedure => (
                    object(MySqlObjectKind::Procedure, &routine.name),
                    Some("procedure".into()),
                ),
                MySqlRoutineKind::Function => (
                    object(MySqlObjectKind::Function, &routine.name),
                    Some("function".into()),
                ),
            })
            .collect(),
        Group::Events => objects
            .events
            .iter()
            .map(|name| (object(MySqlObjectKind::Event, name), None))
            .collect(),
        Group::Triggers => objects
            .triggers
            .iter()
            .map(|trigger| {
                (
                    object(MySqlObjectKind::Trigger, &trigger.name),
                    Some(trigger.table.clone()),
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::mysql_sessions::{MySqlRoutine, MySqlTrigger};

    #[test]
    fn lifecycle_connects_once_and_never_retries_on_failure() {
        let mut life = Lifecycle::default();
        let first = life.begin().unwrap();
        // A second click while connecting starts nothing.
        assert_eq!(life.begin(), None);
        assert!(!life.opened(first, Err("Access denied".into())));
        assert_eq!(
            life.phase(),
            &ConnectionPhase::Failed("Access denied".into())
        );
        // Failure stays until the user retries explicitly.
        let retry = life.begin().unwrap();
        assert!(retry > first);
        assert!(life.opened(retry, Ok(())));
        assert_eq!(life.phase(), &ConnectionPhase::Connected);
        assert_eq!(life.begin(), None);
    }

    #[test]
    fn lifecycle_drops_late_sessions_and_stale_close_reports() {
        let mut life = Lifecycle::default();
        let attempt = life.begin().unwrap();
        life.disconnect();
        // The session that arrives after a disconnect must be closed.
        assert!(!life.opened(attempt, Ok(())));
        assert_eq!(life.phase(), &ConnectionPhase::Idle);

        let attempt = life.begin().unwrap();
        assert!(life.opened(attempt, Ok(())));
        assert!(life.is_current(attempt));
        // A lost connection fails the row; its later duplicate is ignored.
        assert!(life.closed(attempt, Some("Connection lost".into())));
        assert_eq!(
            life.phase(),
            &ConnectionPhase::Failed("Connection lost".into())
        );
        assert!(!life.closed(attempt, None));
        assert!(!life.is_current(attempt));

        // A normal close (retired by an edit or disconnect) goes idle.
        let attempt = life.begin().unwrap();
        assert!(life.opened(attempt, Ok(())));
        assert!(life.closed(attempt, None));
        assert_eq!(life.phase(), &ConnectionPhase::Idle);
    }

    fn objects(database: &str) -> MySqlObjects {
        MySqlObjects {
            database: database.into(),
            tables: vec!["orders".into(), "users".into()],
            views: vec!["big_orders".into()],
            routines: vec![MySqlRoutine {
                name: "answer".into(),
                kind: MySqlRoutineKind::Function,
            }],
            events: vec![],
            triggers: vec![MySqlTrigger {
                name: "orders_bi".into(),
                table: "orders".into(),
            }],
            truncated: false,
        }
    }

    fn labels(tree: &Tree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth as usize), row.label))
            .collect()
    }

    #[test]
    fn default_database_opens_with_its_tables_and_others_load_on_expand() {
        let mut tree = Tree::default();
        assert!(tree.rows().is_empty());
        let load = tree.set_databases(Ok((vec!["app".into(), "shop".into()], false)), Some("shop"));
        assert_eq!(load.as_deref(), Some("shop"));
        assert_eq!(labels(&tree), ["app", "shop", "  Loading objects…"]);
        tree.set_objects("shop", Ok(objects("shop")));
        assert_eq!(
            labels(&tree),
            [
                "app",
                "shop",
                "  Tables",
                "    orders",
                "    users",
                "  Views",
                "  Routines",
                "  Events",
                "  Triggers"
            ]
        );
        let rows = tree.rows();
        assert_eq!(rows[2].count, Some(2));
        assert_eq!(rows[8].count, Some(1));

        // Expanding loads once; collapsing and re-expanding reuses the result.
        assert_eq!(tree.toggle_database("app").as_deref(), Some("app"));
        tree.set_objects("app", Ok(objects("app")));
        assert_eq!(tree.toggle_database("app"), None);
        assert_eq!(tree.toggle_database("app"), None);
        tree.toggle_group("shop", Group::Triggers);
        let trigger = tree
            .rows()
            .into_iter()
            .find(|row| row.label == "orders_bi")
            .unwrap();
        assert_eq!(trigger.detail.as_deref(), Some("orders"));
        assert_eq!(
            trigger.kind,
            RowKind::Object(ObjectRef {
                database: "shop".into(),
                kind: MySqlObjectKind::Trigger,
                name: "orders_bi".into()
            })
        );
    }

    #[test]
    fn failures_and_truncation_are_visible_notes_and_failed_loads_retry() {
        let mut tree = Tree::default();
        assert_eq!(
            tree.set_databases(Err("Access denied (1044)".into()), None),
            None
        );
        assert!(matches!(
            tree.rows()[0].kind,
            RowKind::Note { error: true, .. }
        ));

        // A missing default database opens nothing.
        let load = tree.set_databases(Ok((vec!["app".into()], true)), Some("gone"));
        assert_eq!(load, None);
        assert_eq!(
            labels(&tree),
            ["app", "More databases exist than are listed"]
        );
        assert_eq!(tree.toggle_database("app").as_deref(), Some("app"));
        tree.set_objects("app", Err("Lost".into()));
        assert!(labels(&tree).contains(&"  Lost".to_owned()));
        // Collapse, then expand again: a failed load is retried explicitly.
        assert_eq!(tree.toggle_database("app"), None);
        assert_eq!(tree.toggle_database("app").as_deref(), Some("app"));
    }

    #[test]
    fn refresh_reloads_only_expanded_databases() {
        let mut tree = Tree::default();
        tree.set_databases(Ok((vec!["a".into(), "b".into()], false)), Some("a"));
        tree.set_objects("a", Ok(objects("a")));
        tree.toggle_database("b");
        tree.toggle_database("a");
        assert_eq!(tree.refresh(), ["b"]);
        assert_eq!(tree.rows()[0].label, "Loading databases…");
        assert!(tree.database_names().is_empty());
        // The reply after a refresh does not reopen the collapsed default.
        assert_eq!(
            tree.set_databases(Ok((vec!["a".into(), "b".into()], false)), Some("a")),
            None
        );
        assert_eq!(tree.rows()[0].kind, RowKind::Database { expanded: false });
        tree.reset();
        assert!(tree.rows().is_empty());
    }

    #[test]
    fn only_tables_and_views_browse_rows() {
        let object = |kind| ObjectRef {
            database: "a".into(),
            kind,
            name: "x".into(),
        };
        assert!(object(MySqlObjectKind::Table).has_rows());
        assert!(object(MySqlObjectKind::View).has_rows());
        assert!(!object(MySqlObjectKind::Event).has_rows());
        assert!(!object(MySqlObjectKind::Procedure).has_rows());
    }
}
