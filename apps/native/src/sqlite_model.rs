//! Plan 031 step 4: pure state for the native SQLite workspace. The object
//! tree flattening, tab identities, table paging and the translation of a
//! bounded SQLite execution into the result grid's event stream live here so
//! they can be tested without a window.
use std::collections::BTreeSet;

use dbunk_lib::backend::sqlite_session::{
    SqliteDatabase, SqliteExecution, SqliteObjects, SqlitePage, SqliteResultSet,
};
use dbunk_lib::backend::{
    QueryEvent, QueryTransactionSnapshot, RowLimitOutcome, StatementClassKind,
    StatementClassSummary,
};

/// Rows per table page.
pub const PAGE_ROWS: u32 = 200;
/// Rows per grid batch, so one event never carries a whole page.
const BATCH_ROWS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectKind {
    Table,
    View,
    Index,
    Trigger,
}

impl ObjectKind {
    pub const ALL: [ObjectKind; 4] = [Self::Table, Self::View, Self::Index, Self::Trigger];

    pub fn plural(self) -> &'static str {
        match self {
            Self::Table => "Tables",
            Self::View => "Views",
            Self::Index => "Indexes",
            Self::Trigger => "Triggers",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Table => "tables",
            Self::View => "views",
            Self::Index => "indexes",
            Self::Trigger => "triggers",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Table => "icons/table.svg",
            Self::View => "icons/eye.svg",
            Self::Index => "icons/hash.svg",
            Self::Trigger => "icons/bolt_outlined.svg",
        }
    }

    /// Tables and views have rows and structure; indexes and triggers open
    /// their owning table's structure.
    pub fn browsable(self) -> bool {
        matches!(self, Self::Table | Self::View)
    }

    fn objects(
        self,
        database: &SqliteDatabase,
    ) -> &[dbunk_lib::backend::sqlite_session::SqliteObject] {
        match self {
            Self::Table => &database.tables,
            Self::View => &database.views,
            Self::Index => &database.indexes,
            Self::Trigger => &database.triggers,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Database {
        schema: String,
        file: String,
    },
    Group {
        schema: String,
        kind: ObjectKind,
    },
    Object {
        schema: String,
        kind: ObjectKind,
        name: String,
        /// Owning table (itself for tables and views).
        table: String,
    },
    /// A trailing note that the kind lists only its first objects.
    Truncated {
        schema: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub key: String,
    pub depth: usize,
    pub label: String,
    pub kind: RowKind,
    pub count: Option<usize>,
    /// `Some` for expandable rows.
    pub expanded: Option<bool>,
}

pub fn database_key(schema: &str) -> String {
    format!("db\u{1f}{schema}")
}

pub fn group_key(schema: &str, kind: ObjectKind) -> String {
    format!("group\u{1f}{schema}\u{1f}{}", kind.key())
}

/// `main` and its tables start open; everything else starts closed.
pub fn default_expanded() -> BTreeSet<String> {
    [database_key("main"), group_key("main", ObjectKind::Table)]
        .into_iter()
        .collect()
}

/// Flattens the tree for rendering. `filter` keeps objects whose name
/// contains it (case-insensitive) and opens every group with a match.
pub fn tree_rows(
    objects: &SqliteObjects,
    expanded: &BTreeSet<String>,
    filter: &str,
) -> Vec<TreeRow> {
    let needle = filter.trim().to_lowercase();
    let mut rows = Vec::new();
    for database in &objects.databases {
        let schema = &database.name;
        let db_key = database_key(schema);
        let matches = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
        let total: usize = ObjectKind::ALL
            .iter()
            .map(|kind| {
                kind.objects(database)
                    .iter()
                    .filter(|o| matches(&o.name))
                    .count()
            })
            .sum();
        if !needle.is_empty() && total == 0 {
            continue;
        }
        let db_open = !needle.is_empty() || expanded.contains(&db_key);
        rows.push(TreeRow {
            key: db_key,
            depth: 0,
            label: schema.clone(),
            kind: RowKind::Database {
                schema: schema.clone(),
                file: database.file.clone(),
            },
            count: None,
            expanded: Some(db_open),
        });
        if !db_open {
            continue;
        }
        for kind in ObjectKind::ALL {
            let members: Vec<_> = kind
                .objects(database)
                .iter()
                .filter(|object| matches(&object.name))
                .collect();
            if !needle.is_empty() && members.is_empty() {
                continue;
            }
            let key = group_key(schema, kind);
            let open = !needle.is_empty() || expanded.contains(&key);
            rows.push(TreeRow {
                key: key.clone(),
                depth: 1,
                label: kind.plural().into(),
                kind: RowKind::Group {
                    schema: schema.clone(),
                    kind,
                },
                count: Some(members.len()),
                expanded: Some(open),
            });
            if !open {
                continue;
            }
            for object in members {
                rows.push(TreeRow {
                    key: format!("{key}\u{1f}{}", object.name),
                    depth: 2,
                    label: object.name.clone(),
                    kind: RowKind::Object {
                        schema: schema.clone(),
                        kind,
                        name: object.name.clone(),
                        table: object.table.clone(),
                    },
                    count: None,
                    expanded: None,
                });
            }
        }
        if database.truncated {
            rows.push(TreeRow {
                key: format!("{}\u{1f}truncated", database_key(schema)),
                depth: 1,
                label: "Only the first objects of each kind are listed".into(),
                kind: RowKind::Truncated {
                    schema: schema.clone(),
                },
                count: None,
                expanded: None,
            });
        }
    }
    rows
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Select(usize),
    Toggle(usize),
}

/// Tree keyboard model: arrows move, Right opens then descends, Left closes
/// then climbs to the parent.
pub fn navigate(rows: &[TreeRow], selected: usize, key: Key) -> Option<Move> {
    if rows.is_empty() {
        return None;
    }
    let selected = selected.min(rows.len() - 1);
    let row = &rows[selected];
    match key {
        Key::Up => selected.checked_sub(1).map(Move::Select),
        Key::Down => (selected + 1 < rows.len()).then_some(Move::Select(selected + 1)),
        Key::Home => Some(Move::Select(0)),
        Key::End => Some(Move::Select(rows.len() - 1)),
        Key::Right => match row.expanded {
            Some(false) => Some(Move::Toggle(selected)),
            Some(true) => rows
                .get(selected + 1)
                .filter(|next| next.depth > row.depth)
                .map(|_| Move::Select(selected + 1)),
            None => None,
        },
        Key::Left => match row.expanded {
            Some(true) => Some(Move::Toggle(selected)),
            _ => rows[..selected]
                .iter()
                .rposition(|candidate| candidate.depth < row.depth)
                .map(Move::Select),
        },
    }
}

/// What a SQLite tab shows. Data and structure tabs are unique per object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TabKind {
    Query(u32),
    Data { schema: String, name: String },
    Structure { schema: String, name: String },
}

impl TabKind {
    pub fn title(&self) -> String {
        let object = |schema: &str, name: &str| {
            if schema == "main" {
                name.to_owned()
            } else {
                format!("{schema}.{name}")
            }
        };
        match self {
            Self::Query(1) => "Query".into(),
            Self::Query(number) => format!("Query {number}"),
            Self::Data { schema, name } => object(schema, name),
            Self::Structure { schema, name } => format!("{} · structure", object(schema, name)),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Query(_) => "icons/terminal.svg",
            Self::Data { .. } => "icons/table.svg",
            Self::Structure { .. } => "icons/list_tree.svg",
        }
    }
}

/// Offset arithmetic for table pages; never underflows or skips rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paging {
    pub offset: u64,
    pub limit: u32,
}

impl Default for Paging {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: PAGE_ROWS,
        }
    }
}

impl Paging {
    pub fn next(self) -> Self {
        Self {
            offset: self.offset.saturating_add(u64::from(self.limit)),
            ..self
        }
    }

    pub fn previous(self) -> Self {
        Self {
            offset: self.offset.saturating_sub(u64::from(self.limit)),
            ..self
        }
    }

    pub fn label(self, page: &SqlitePage) -> String {
        let rows = page.set.rows.len() as u64;
        if rows == 0 {
            if self.offset == 0 {
                "No rows".into()
            } else {
                format!("No rows after {}", self.offset)
            }
        } else {
            format!("Rows {}–{}", self.offset + 1, self.offset + rows)
        }
    }
}

fn events_for_set(index: u32, set: SqliteResultSet, events: &mut Vec<QueryEvent>) {
    events.push(QueryEvent::ResultSetStarted {
        result_set_index: index,
        columns: set.columns.into_iter().map(Some).collect(),
    });
    let mut rows = set.rows.into_iter().peekable();
    while rows.peek().is_some() {
        events.push(QueryEvent::RowBatch {
            result_set_index: index,
            rows: rows.by_ref().take(BATCH_ROWS).collect(),
        });
    }
    events.push(QueryEvent::ResultSetCompleted {
        result_set_index: index,
        row_count: set.row_count,
        partial: set.omitted_rows > 0,
        limit: (set.omitted_rows > 0).then_some(RowLimitOutcome::Drained),
    });
}

fn completion(
    status: &str,
    omitted_rows: u64,
    omitted_sets: u32,
    reasons: Vec<String>,
) -> QueryEvent {
    QueryEvent::ExecutionCompleted {
        status: status.into(),
        transaction: QueryTransactionSnapshot::default(),
        omitted_rows,
        omitted_result_sets: omitted_sets,
        omitted_notices: 0,
        omitted_metadata_bytes: 0,
        truncation_reasons: reasons,
        error: None,
        refusal: None,
        context: None,
    }
}

/// The grid's view of a completed execution: every retained set, then one
/// completion carrying the bounds that applied.
pub fn execution_events(
    sets: Vec<SqliteResultSet>,
    omitted_sets: u32,
    byte_limited: bool,
) -> Vec<QueryEvent> {
    let mut events = vec![QueryEvent::ExecutionStarted];
    let omitted_rows: u64 = sets.iter().map(|set| set.omitted_rows).sum();
    let truncated_cells: u64 = sets.iter().map(|set| set.truncated_cells).sum();
    for (index, set) in sets.into_iter().enumerate() {
        events_for_set(index as u32, set, &mut events);
    }
    let mut reasons = Vec::new();
    if omitted_rows > 0 {
        reasons.push(if byte_limited {
            "SQLite results are limited to 16 MiB per run".to_string()
        } else {
            format!(
                "SQLite results keep {} rows per result",
                dbunk_lib::backend::sqlite_session::SQLITE_MAX_ROWS_PER_SET
            )
        });
    }
    if truncated_cells > 0 {
        reasons.push(format!("{truncated_cells} long values were shortened"));
    }
    events.push(completion("completed", omitted_rows, omitted_sets, reasons));
    events
}

/// One table page as a single result set.
pub fn page_events(page: &SqlitePage) -> Vec<QueryEvent> {
    let mut set = page.set.clone();
    // Rows past the page are not omitted; they are on the next page.
    set.omitted_rows = 0;
    let truncated = set.truncated_cells;
    let mut events = vec![QueryEvent::ExecutionStarted];
    events_for_set(0, set, &mut events);
    let reasons = if truncated > 0 {
        vec![format!("{truncated} long values were shortened")]
    } else {
        Vec::new()
    };
    events.push(completion("completed", 0, 0, reasons));
    events
}

/// One line for the query footer after a run.
pub fn execution_summary(execution: &SqliteExecution) -> String {
    match execution {
        SqliteExecution::Completed {
            sets,
            rows_affected,
            elapsed_ms,
            ..
        } => {
            let mut parts = Vec::new();
            match sets.len() {
                0 => {}
                1 => parts.push(plural(sets[0].row_count, "row", "rows")),
                count => parts.push(format!(
                    "{count} results, {}",
                    plural(sets.iter().map(|set| set.row_count).sum(), "row", "rows")
                )),
            }
            if *rows_affected > 0 || sets.is_empty() {
                parts.push(format!(
                    "{} affected",
                    plural(*rows_affected, "row", "rows")
                ));
            }
            parts.push(format!("{elapsed_ms} ms"));
            parts.join(" · ")
        }
        SqliteExecution::NeedsConfirmation { .. } => "Waiting for confirmation".into(),
        SqliteExecution::Blocked { reason } => reason.clone(),
    }
}

fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What a confirmation is for, e.g. "2 statements: 1 write, 1 destructive".
pub fn confirmation_text(statements: &[StatementClassSummary]) -> String {
    let writes = statements
        .iter()
        .filter(|statement| {
            matches!(
                statement.class,
                StatementClassKind::Dml | StatementClassKind::Ddl | StatementClassKind::Unknown
            )
        })
        .count();
    let destructive = statements
        .iter()
        .filter(|statement| statement.destructive || statement.unbounded)
        .count();
    let mut text = format!(
        "This connection's safety policy requires confirmation: {} that may write",
        plural(writes as u64, "statement", "statements")
    );
    if destructive > 0 {
        text.push_str(&format!(", {destructive} destructive or unbounded"));
    }
    text.push('.');
    text
}

/// Whether a run may have changed the object tree (DDL, ATTACH, …).
pub fn changes_objects(sql: &str) -> bool {
    sql.split(|c: char| !c.is_ascii_alphabetic()).any(|word| {
        ["create", "drop", "alter", "attach", "detach"]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::sqlite_session::SqliteObject;

    fn object(name: &str, table: &str) -> SqliteObject {
        SqliteObject {
            name: name.into(),
            table: table.into(),
        }
    }

    fn objects() -> SqliteObjects {
        SqliteObjects {
            databases: vec![
                SqliteDatabase {
                    name: "main".into(),
                    file: "/tmp/app.db".into(),
                    tables: vec![object("authors", "authors"), object("books", "books")],
                    views: vec![object("cheap", "cheap")],
                    indexes: vec![object("books_title", "books")],
                    triggers: vec![],
                    truncated: false,
                },
                SqliteDatabase {
                    name: "extra".into(),
                    file: "/tmp/extra.db".into(),
                    tables: vec![object("notes", "notes")],
                    truncated: true,
                    ..Default::default()
                },
            ],
        }
    }

    fn labels(rows: &[TreeRow]) -> Vec<(usize, &str)> {
        rows.iter()
            .map(|row| (row.depth, row.label.as_str()))
            .collect()
    }

    #[test]
    fn default_tree_opens_main_tables_only() {
        let rows = tree_rows(&objects(), &default_expanded(), "");
        assert_eq!(
            labels(&rows),
            [
                (0, "main"),
                (1, "Tables"),
                (2, "authors"),
                (2, "books"),
                (1, "Views"),
                (1, "Indexes"),
                (1, "Triggers"),
                (0, "extra"),
            ]
        );
        assert_eq!(rows[1].count, Some(2));
        assert_eq!(rows[7].expanded, Some(false));
    }

    #[test]
    fn indexes_keep_their_owning_table_and_truncation_is_shown() {
        let mut expanded = default_expanded();
        expanded.insert(group_key("main", ObjectKind::Index));
        expanded.insert(database_key("extra"));
        let rows = tree_rows(&objects(), &expanded, "");
        let index = rows.iter().find(|row| row.label == "books_title").unwrap();
        assert_eq!(
            index.kind,
            RowKind::Object {
                schema: "main".into(),
                kind: ObjectKind::Index,
                name: "books_title".into(),
                table: "books".into(),
            }
        );
        assert!(matches!(
            rows.last().unwrap().kind,
            RowKind::Truncated { ref schema } if schema == "extra"
        ));
    }

    #[test]
    fn filter_opens_matching_groups_and_hides_the_rest() {
        let rows = tree_rows(&objects(), &BTreeSet::new(), "BOO");
        assert_eq!(
            labels(&rows),
            [
                (0, "main"),
                (1, "Tables"),
                (2, "books"),
                (1, "Indexes"),
                (2, "books_title")
            ]
        );
        assert!(tree_rows(&objects(), &BTreeSet::new(), "zzz").is_empty());
    }

    #[test]
    fn keys_do_not_collide_across_schemas_and_kinds() {
        let mut expanded = default_expanded();
        expanded.insert(database_key("extra"));
        expanded.insert(group_key("extra", ObjectKind::Table));
        let rows = tree_rows(&objects(), &expanded, "");
        let keys: BTreeSet<_> = rows.iter().map(|row| row.key.clone()).collect();
        assert_eq!(keys.len(), rows.len());
    }

    #[test]
    fn keyboard_opens_descends_closes_and_climbs() {
        let rows = tree_rows(&objects(), &default_expanded(), "");
        assert_eq!(navigate(&rows, 0, Key::Right), Some(Move::Select(1)));
        assert_eq!(navigate(&rows, 2, Key::Left), Some(Move::Select(1)));
        assert_eq!(navigate(&rows, 1, Key::Left), Some(Move::Toggle(1)));
        assert_eq!(navigate(&rows, 4, Key::Right), Some(Move::Toggle(4)));
        assert_eq!(navigate(&rows, 2, Key::Right), None);
        assert_eq!(navigate(&rows, 0, Key::Up), None);
        assert_eq!(navigate(&rows, rows.len() - 1, Key::Down), None);
        assert_eq!(
            navigate(&rows, 3, Key::End),
            Some(Move::Select(rows.len() - 1))
        );
        assert_eq!(navigate(&[], 0, Key::Down), None);
    }

    #[test]
    fn paging_never_underflows() {
        let paging = Paging::default();
        assert_eq!(paging.previous().offset, 0);
        assert_eq!(paging.next().offset, u64::from(PAGE_ROWS));
        assert_eq!(paging.next().previous(), paging);
        let page = SqlitePage {
            set: SqliteResultSet {
                rows: vec![vec![None]; 3],
                ..Default::default()
            },
            offset: 200,
            has_more: false,
            elapsed_ms: 1,
        };
        assert_eq!(paging.next().label(&page), "Rows 201–203");
        let empty = SqlitePage {
            set: SqliteResultSet::default(),
            ..page
        };
        assert_eq!(paging.label(&empty), "No rows");
    }

    #[test]
    fn tab_titles_hide_main_but_name_attached_databases() {
        let data = TabKind::Data {
            schema: "main".into(),
            name: "books".into(),
        };
        assert_eq!(data.title(), "books");
        let attached = TabKind::Structure {
            schema: "extra".into(),
            name: "notes".into(),
        };
        assert_eq!(attached.title(), "extra.notes · structure");
        assert_eq!(TabKind::Query(1).title(), "Query");
        assert_eq!(TabKind::Query(3).title(), "Query 3");
    }

    #[test]
    fn executions_become_bounded_grid_events() {
        let set = SqliteResultSet {
            columns: vec!["n".into()],
            rows: (0..600).map(|n| vec![Some(n.to_string())]).collect(),
            row_count: 2_100,
            omitted_rows: 1_500,
            truncated_cells: 2,
        };
        let events = execution_events(vec![set, SqliteResultSet::default()], 1, false);
        let batches = events
            .iter()
            .filter(|event| matches!(event, QueryEvent::RowBatch { .. }))
            .count();
        assert_eq!(batches, 3, "600 rows in batches of 256");
        let mut model = crate::results::ResultModel::default();
        for event in events {
            model.consume(event);
        }
        assert_eq!(model.sets.len(), 2);
        assert_eq!(model.sets[0].rows.len(), 600);
        assert_eq!(model.sets[0].row_count, Some(2_100));
        assert!(model.sets[0].partial);
        let completion = model.completion.unwrap();
        assert_eq!(completion.status, crate::results::TerminalStatus::Completed);
        assert_eq!(completion.omitted_rows, 1_500);
        assert_eq!(completion.omitted_result_sets, 1);
        assert_eq!(completion.truncation_reasons.len(), 2);
    }

    #[test]
    fn pages_do_not_report_next_page_rows_as_omitted() {
        let page = SqlitePage {
            set: SqliteResultSet {
                columns: vec!["id".into()],
                rows: vec![vec![Some("1".into())], vec![None]],
                row_count: 2,
                omitted_rows: 0,
                truncated_cells: 0,
            },
            offset: 0,
            has_more: true,
            elapsed_ms: 0,
        };
        let mut model = crate::results::ResultModel::default();
        for event in page_events(&page) {
            model.consume(event);
        }
        assert_eq!(model.sets[0].rows[1].as_ref(), [None]);
        assert!(!model.sets[0].partial);
        assert_eq!(model.completion.unwrap().omitted_rows, 0);
    }

    #[test]
    fn summaries_name_rows_results_and_changes() {
        let set = |rows: u64| SqliteResultSet {
            row_count: rows,
            ..Default::default()
        };
        let done = |sets, rows_affected| SqliteExecution::Completed {
            sets,
            rows_affected,
            omitted_sets: 0,
            byte_limited: false,
            elapsed_ms: 4,
        };
        assert_eq!(execution_summary(&done(vec![set(1)], 0)), "1 row · 4 ms");
        assert_eq!(
            execution_summary(&done(vec![set(2), set(3)], 1)),
            "2 results, 5 rows · 1 row affected · 4 ms"
        );
        assert_eq!(
            execution_summary(&done(vec![], 0)),
            "0 rows affected · 4 ms"
        );
    }

    #[test]
    fn confirmation_counts_writes_and_destructive_statements() {
        let summary = |class, destructive| StatementClassSummary {
            index: 0,
            class,
            unbounded: false,
            destructive,
        };
        let text = confirmation_text(&[
            summary(StatementClassKind::Read, false),
            summary(StatementClassKind::Dml, false),
            summary(StatementClassKind::Ddl, true),
        ]);
        assert_eq!(
            text,
            "This connection's safety policy requires confirmation: 2 statements that may write, 1 destructive or unbounded."
        );
    }

    #[test]
    fn object_changes_are_detected_by_keyword() {
        assert!(changes_objects("create table t (x)"));
        assert!(changes_objects("SELECT 1; DROP VIEW v"));
        assert!(changes_objects("attach 'x.db' as x"));
        assert!(!changes_objects("SELECT created_at FROM t"));
        assert!(!changes_objects("UPDATE t SET dropped = 1"));
    }
}
