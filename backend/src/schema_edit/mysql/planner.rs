//! The MySQL planner: every column, key and index change for the table goes
//! into one `ALTER TABLE`, which InnoDB applies atomically. Foreign key drops
//! run first in their own `ALTER` and foreign key adds last in theirs, since
//! the server refuses some combinations with other clauses (re-creating a
//! key under the same name, adding a key whose index is built in the same
//! statement). Each statement commits on its own (`Standalone`).
//!
//! `MODIFY`/`CHANGE` restate the whole column from the draft
//! ([`column_definition`]), so no attribute is dropped by omission. Column
//! order is reached with `FIRST`/`AFTER`: existing columns outside a longest
//! increasing run of their old positions, and new columns that do not trail
//! the table, each go after their draft predecessor. Column clauses are
//! emitted in draft order, so every `AFTER` names a column the server has
//! already placed.
use super::sql::{column_definition, ident, plain_word, qualified, string_literal};
use crate::schema_edit::{
    columns_reordered, group_statements, is_widening, plan_steps, CheckModel, ColumnField,
    ColumnModel, Deferrable, EngineNote, EnginePlan, Field, ForeignKeyModel, Identity, IndexModel,
    IndexTarget, KeyModel, NoteKind, Origin, PlanError, PlanOptions, PlanStep, PreviewStatement,
    ReferentialAction, SchemaBaseline, SchemaChangeSet, SchemaDraft, SchemaEngine, SchemaObject,
    ServerFeatures, ServerFlavor, TableModel,
};
use std::collections::{HashMap, HashSet};

/// Rows above which a rebuilding change gets a large-table note.
const LARGE_TABLE_ROWS: u64 = 1_000_000;

#[derive(Clone, Debug)]
struct Clause {
    sql: String,
    summary: String,
    destructive: bool,
    /// Qualifies for `ALGORITHM=INSTANT` (conservative).
    instant: bool,
    /// May copy or rebuild the table.
    rewrite: bool,
}

impl Clause {
    fn new(sql: String, summary: String) -> Self {
        Self {
            sql,
            summary,
            destructive: false,
            instant: false,
            rewrite: false,
        }
    }
    fn destructive(mut self, value: bool) -> Self {
        self.destructive = value;
        self
    }
    fn instant(mut self, value: bool) -> Self {
        self.instant = value;
        self
    }
    fn rewrite(mut self, value: bool) -> Self {
        self.rewrite = value;
        self
    }
}

fn internal(what: &str) -> PlanError {
    PlanError::Internal(format!("MySQL planner: {what}"))
}

fn unsupported(reason: impl Into<String>) -> PlanError {
    PlanError::Unsupported {
        reason: reason.into(),
    }
}

fn column_name(table: &TableModel, origin: Origin) -> Result<&str, PlanError> {
    table
        .column_name(origin)
        .ok_or_else(|| internal("a key references a column that is not in the table"))
}

fn column_list(table: &TableModel, origins: &[Origin]) -> Result<String, PlanError> {
    let names = origins
        .iter()
        .map(|origin| column_name(table, *origin).map(ident))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.join(", "))
}

fn field_label(field: ColumnField) -> &'static str {
    match field {
        ColumnField::DataType => "type",
        ColumnField::Collation => "collation",
        ColumnField::Nullable => "nullability",
        ColumnField::Default => "default",
        ColumnField::Generated => "generation",
        ColumnField::Identity => "auto-increment",
        ColumnField::Charset => "character set",
        ColumnField::OnUpdate => "on update",
        ColumnField::Invisible => "visibility",
    }
}

/// Plans `draft` as MySQL statements. Pure and deterministic.
pub(super) fn plan(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    _changes: &SchemaChangeSet,
    options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    if !options.concurrent_indexes.is_empty()
        || options.validate_separately
        || !options.type_conversions.is_empty()
    {
        return Err(unsupported(
            "Concurrent indexes, separate validation and USING are PostgreSQL options",
        ));
    }
    let old = &baseline.table;
    let new = &draft.table;
    let features = baseline.server.features;
    let mariadb = baseline.server.flavor == ServerFlavor::MariaDb;
    if !features.column_reorder && columns_reordered(old, new) {
        return Err(unsupported("This server cannot reorder columns"));
    }

    let mut foreign_drops = Vec::new();
    let mut drops = Vec::new();
    let mut renames = Vec::new();
    let mut adds = Vec::new();
    let mut foreign_adds = Vec::new();
    let mut table_clauses = Vec::new();
    let mut rename_table = None;

    for step in plan_steps(baseline, draft) {
        match step {
            PlanStep::DropForeignKey { id } => {
                foreign_drops.push(drop_foreign_key(old, Origin::Existing(id))?);
            }
            PlanStep::DropCheck { id } => {
                drops.push(drop_check(old, Origin::Existing(id), mariadb)?)
            }
            PlanStep::DropUnique { id } => {
                let name = old
                    .key(Origin::Existing(id))
                    .and_then(|key| key.name.clone())
                    .ok_or_else(|| internal("a dropped unique key has no name"))?;
                drops.push(
                    Clause::new(
                        format!("DROP INDEX {}", ident(&name)),
                        format!("drop unique key {name}"),
                    )
                    .destructive(true),
                );
            }
            PlanStep::DropPrimaryKey { .. } => drops.push(
                Clause::new("DROP PRIMARY KEY".into(), "drop primary key".into())
                    .destructive(true)
                    .rewrite(true),
            ),
            PlanStep::DropIndex { id } => {
                let name = old
                    .index(Origin::Existing(id))
                    .map(|index| index.name.clone())
                    .ok_or_else(|| internal("a dropped index is not in the baseline"))?;
                drops.push(
                    Clause::new(
                        format!("DROP INDEX {}", ident(&name)),
                        format!("drop index {name}"),
                    )
                    .destructive(true),
                );
            }
            PlanStep::RenameConstraint { object, id } => {
                let origin = Origin::Existing(id);
                match object {
                    SchemaObject::Index => {
                        let from = old.index(origin).map(|i| i.name.clone());
                        let to = new.index(origin).map(|i| i.name.clone());
                        renames.push(rename_index(from, to, features)?);
                    }
                    SchemaObject::Unique => {
                        let from = old.key(origin).and_then(|k| k.name.clone());
                        let to = new.key(origin).and_then(|k| k.name.clone());
                        renames.push(rename_index(from, to, features)?);
                    }
                    SchemaObject::Check => {
                        // MySQL cannot rename a check; drop and re-add it in
                        // the same atomic statement.
                        drops.push(drop_check(old, origin, mariadb)?.destructive(false));
                        let check = new
                            .check(origin)
                            .ok_or_else(|| internal("a renamed check is not in the draft"))?;
                        adds.push(add_check(check, features, mariadb)?);
                    }
                    SchemaObject::ForeignKey => {
                        foreign_drops.push(drop_foreign_key(old, origin)?);
                        let key = new
                            .foreign_key(origin)
                            .ok_or_else(|| internal("a renamed foreign key is not in the draft"))?;
                        foreign_adds.push(add_foreign_key(key, new)?);
                    }
                    SchemaObject::PrimaryKey => {
                        return Err(unsupported(
                            "MySQL primary keys are always named PRIMARY and cannot be renamed",
                        ))
                    }
                    SchemaObject::Table | SchemaObject::Column => {
                        return Err(internal("unexpected constraint rename"))
                    }
                }
            }
            PlanStep::RenameTable => {
                rename_table = Some(
                    Clause::new(
                        // Qualified: an unqualified target resolves in the connection's
                        // default database, which may be another database or none.
                        format!("RENAME TO {}", qualified(&new.schema, &new.name)),
                        format!("rename table to {}", new.name),
                    )
                    .instant(true),
                );
            }
            PlanStep::AddPrimaryKey { origin } => {
                let key = new
                    .primary_key
                    .as_ref()
                    .filter(|key| key.origin == origin)
                    .ok_or_else(|| internal("an added primary key is not in the draft"))?;
                adds.push(
                    Clause::new(
                        format!("ADD PRIMARY KEY ({})", column_list(new, &key.columns)?),
                        "add primary key".into(),
                    )
                    .rewrite(true),
                );
            }
            PlanStep::AddUnique { origin } => {
                let key = new
                    .uniques
                    .iter()
                    .find(|key| key.origin == origin)
                    .ok_or_else(|| internal("an added unique key is not in the draft"))?;
                adds.push(add_unique(key, new)?);
            }
            PlanStep::AddCheck { origin } => {
                let check = new
                    .check(origin)
                    .ok_or_else(|| internal("an added check is not in the draft"))?;
                adds.push(add_check(check, features, mariadb)?);
            }
            PlanStep::CreateIndex { origin } => {
                let index = new
                    .index(origin)
                    .ok_or_else(|| internal("an added index is not in the draft"))?;
                adds.push(add_index(index, new)?);
            }
            PlanStep::AddForeignKey { origin } => {
                let key = new
                    .foreign_key(origin)
                    .ok_or_else(|| internal("an added foreign key is not in the draft"))?;
                foreign_adds.push(add_foreign_key(key, new)?);
            }
            PlanStep::SetTableComment => {
                let comment = new.comment.clone().unwrap_or_default();
                table_clauses.push(Clause::new(
                    format!("COMMENT = {}", string_literal(&comment)),
                    "set table comment".into(),
                ));
            }
            PlanStep::SetTableOptions { fields } => {
                for field in fields {
                    table_clauses.push(table_option(new, field)?);
                }
            }
            // Columns are planned below, in draft order.
            PlanStep::RenameColumn { .. }
            | PlanStep::AlterColumn { .. }
            | PlanStep::AddColumn { .. }
            | PlanStep::DropColumn { .. }
            | PlanStep::ReorderColumns
            | PlanStep::SetColumnComment { .. } => {}
        }
    }

    let (columns, column_drops) = column_clauses(old, new, features)?;

    let mut main = Vec::new();
    main.extend(drops);
    main.extend(renames);
    main.extend(columns);
    main.extend(column_drops);
    main.extend(adds);
    main.extend(table_clauses);
    main.extend(rename_table);

    let instant_capable = !mariadb && features.instant_add_column;
    let old_name = qualified(&old.schema, &old.name);
    let new_name = qualified(&new.schema, &new.name);
    let mut statements = Vec::new();
    let mut rewrite = false;
    for (table, label, clauses) in [
        (&old_name, "Drop foreign keys", &foreign_drops),
        (&old_name, "Alter table", &main),
        (&new_name, "Add foreign keys", &foreign_adds),
    ] {
        if clauses.is_empty() {
            continue;
        }
        rewrite |= clauses.iter().any(|clause| clause.rewrite);
        statements.push(statement(
            table,
            label,
            clauses,
            options.online_only,
            instant_capable,
        ));
    }
    if statements.is_empty() {
        return Err(PlanError::NoChanges);
    }

    let notes = notes(
        baseline,
        statements.len(),
        rewrite,
        options.online_only,
        !foreign_adds.is_empty(),
    );
    let groups = group_statements(&vec![false; statements.len()]);
    Ok(EnginePlan {
        statements,
        groups,
        notes,
    })
}

fn statement(
    table: &str,
    label: &str,
    clauses: &[Clause],
    online_only: bool,
    instant_capable: bool,
) -> PreviewStatement {
    let mut parts: Vec<String> = clauses.iter().map(|clause| clause.sql.clone()).collect();
    if online_only {
        if instant_capable && clauses.iter().all(|clause| clause.instant) {
            parts.push("ALGORITHM=INSTANT".into());
        } else {
            parts.push("ALGORITHM=INPLACE".into());
            parts.push("LOCK=NONE".into());
        }
    }
    PreviewStatement {
        sql: format!("ALTER TABLE {table} {}", parts.join(", ")),
        summary: format!(
            "{label}: {}",
            clauses
                .iter()
                .map(|clause| clause.summary.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        destructive: clauses.iter().any(|clause| clause.destructive),
    }
}

fn notes(
    baseline: &SchemaBaseline,
    statements: usize,
    rewrite: bool,
    online_only: bool,
    adds_foreign_keys: bool,
) -> Vec<EngineNote> {
    let mut notes = Vec::new();
    if statements > 1 {
        notes.push(EngineNote::new(
            NoteKind::NonAtomic,
            format!(
                "This change runs as {statements} ALTER TABLE statements. MySQL commits each \
                 one on its own, so a failure part way leaves the earlier statements committed."
            ),
        ));
    }
    if rewrite {
        notes.push(EngineNote::new(
            NoteKind::Rewrite,
            "MySQL may copy or rebuild the whole table for type, character set, nullability, \
             column order, primary key or storage changes.",
        ));
    }
    if online_only {
        notes.push(EngineNote::new(
            NoteKind::Online,
            "Online only: each statement asks for ALGORITHM=INSTANT or ALGORITHM=INPLACE, \
             LOCK=NONE, so the server refuses the change instead of copying the table or \
             blocking writes.",
        ));
        if adds_foreign_keys {
            notes.push(EngineNote::new(
                NoteKind::Online,
                "Adding a foreign key copies the table unless foreign_key_checks is off, so \
                 online mode refuses it.",
            ));
        }
    } else {
        notes.push(EngineNote::new(
            NoteKind::Lock,
            "ALTER TABLE waits at most 10 seconds for a metadata lock on the table; a copying \
             change blocks writes until it finishes.",
        ));
    }
    if !baseline.inbound.is_empty() {
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "{} foreign key(s) in other tables reference this table; changing or dropping \
                 referenced columns can fail.",
                baseline.inbound.len()
            ),
        ));
    }
    if !baseline.dependents.is_empty() {
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "{} trigger(s) or view(s) depend on this table and are not changed with it.",
                baseline.dependents.len()
            ),
        ));
    }
    if rewrite
        && baseline
            .estimated_rows
            .is_some_and(|rows| rows >= LARGE_TABLE_ROWS)
    {
        notes.push(EngineNote::new(
            NoteKind::LargeTable,
            format!(
                "The table has about {} rows; rebuilding it may take a long time.",
                baseline.estimated_rows.unwrap_or_default()
            ),
        ));
    }
    notes
}

fn drop_foreign_key(old: &TableModel, origin: Origin) -> Result<Clause, PlanError> {
    let name = old
        .foreign_key(origin)
        .and_then(|key| key.name.clone())
        .ok_or_else(|| internal("a dropped foreign key has no name"))?;
    Ok(Clause::new(
        format!("DROP FOREIGN KEY {}", ident(&name)),
        format!("drop foreign key {name}"),
    )
    .destructive(true))
}

fn drop_check(old: &TableModel, origin: Origin, mariadb: bool) -> Result<Clause, PlanError> {
    let name = old
        .check(origin)
        .and_then(|check| check.name.clone())
        .ok_or_else(|| internal("a dropped check has no name"))?;
    let keyword = if mariadb { "CONSTRAINT" } else { "CHECK" };
    Ok(Clause::new(
        format!("DROP {keyword} {}", ident(&name)),
        format!("drop check {name}"),
    )
    .destructive(true))
}

fn rename_index(
    from: Option<String>,
    to: Option<String>,
    features: ServerFeatures,
) -> Result<Clause, PlanError> {
    let (Some(from), Some(to)) = (from, to) else {
        return Err(internal("a renamed index has no name"));
    };
    Ok(Clause::new(
        format!("RENAME INDEX {} TO {}", ident(&from), ident(&to)),
        format!("rename index {from} to {to}"),
    )
    .instant(features.instant_add_column))
}

fn add_unique(key: &KeyModel, table: &TableModel) -> Result<Clause, PlanError> {
    let columns = column_list(table, &key.columns)?;
    Ok(match &key.name {
        Some(name) => Clause::new(
            format!("ADD UNIQUE INDEX {} ({columns})", ident(name)),
            format!("add unique key {name}"),
        ),
        None => Clause::new(format!("ADD UNIQUE ({columns})"), "add unique key".into()),
    })
}

fn add_check(
    check: &CheckModel,
    features: ServerFeatures,
    mariadb: bool,
) -> Result<Clause, PlanError> {
    if !features.enforced_check {
        return Err(unsupported(
            "This server parses CHECK constraints but does not enforce them",
        ));
    }
    let mut sql = match &check.name {
        Some(name) => format!("ADD CONSTRAINT {} CHECK ", ident(name)),
        None => "ADD CHECK ".to_owned(),
    };
    sql.push('(');
    sql.push_str(check.expression.as_str().trim());
    sql.push(')');
    if !check.enforced {
        if mariadb {
            return Err(unsupported("MariaDB checks are always enforced"));
        }
        sql.push_str(" NOT ENFORCED");
    }
    let summary = match &check.name {
        Some(name) => format!("add check {name}"),
        None => "add check".to_owned(),
    };
    Ok(Clause::new(sql, summary))
}

fn add_index(index: &IndexModel, table: &TableModel) -> Result<Clause, PlanError> {
    if !index.include.is_empty() {
        return Err(unsupported("MySQL indexes have no INCLUDE columns"));
    }
    if index.predicate.is_some() {
        return Err(unsupported("MySQL has no partial indexes"));
    }
    let method = index.method.as_deref().map(str::to_ascii_uppercase);
    let (kind, using) = match method.as_deref() {
        None => (
            if index.unique {
                "UNIQUE INDEX"
            } else {
                "INDEX"
            },
            None,
        ),
        Some("BTREE" | "HASH") => (
            if index.unique {
                "UNIQUE INDEX"
            } else {
                "INDEX"
            },
            method.clone(),
        ),
        Some("FULLTEXT") if !index.unique => ("FULLTEXT INDEX", None),
        Some("SPATIAL") if !index.unique => ("SPATIAL INDEX", None),
        Some(other) => {
            return Err(unsupported(format!(
                "Index method {other} is not supported on MySQL"
            )))
        }
    };
    let mut parts = Vec::with_capacity(index.members.len());
    for member in &index.members {
        if member.collation.is_some() {
            return Err(unsupported("MySQL index members have no collation"));
        }
        let mut part = match &member.target {
            IndexTarget::Column(origin) => ident(column_name(table, *origin)?),
            IndexTarget::Expression(expression) => format!("({})", expression.as_str().trim()),
        };
        if let Some(length) = member.prefix_length {
            if matches!(member.target, IndexTarget::Expression(_)) {
                return Err(unsupported("An expression key part has no prefix length"));
            }
            part.push_str(&format!("({length})"));
        }
        if member.descending {
            part.push_str(" DESC");
        }
        parts.push(part);
    }
    let mut sql = format!("ADD {kind} {} ({})", ident(&index.name), parts.join(", "));
    if let Some(using) = using {
        sql.push_str(" USING ");
        sql.push_str(&using);
    }
    Ok(Clause::new(sql, format!("add index {}", index.name)))
}

fn action(value: ReferentialAction) -> Result<&'static str, PlanError> {
    match value {
        ReferentialAction::SetDefault => Err(unsupported(
            "InnoDB does not support SET DEFAULT foreign key actions",
        )),
        other => Ok(other.sql()),
    }
}

fn add_foreign_key(key: &ForeignKeyModel, table: &TableModel) -> Result<Clause, PlanError> {
    if key.deferrable != Deferrable::Never {
        return Err(unsupported("MySQL foreign keys cannot be deferred"));
    }
    if key.referenced_columns.is_empty() {
        return Err(unsupported(
            "A MySQL foreign key needs its referenced columns",
        ));
    }
    let referenced = qualified(
        key.referenced_schema.as_deref().unwrap_or(&table.schema),
        &key.referenced_table,
    );
    let referenced_columns = key
        .referenced_columns
        .iter()
        .map(|name| ident(name))
        .collect::<Vec<_>>()
        .join(", ");
    let head = match &key.name {
        Some(name) => format!("ADD CONSTRAINT {} FOREIGN KEY", ident(name)),
        None => "ADD FOREIGN KEY".to_owned(),
    };
    let sql = format!(
        "{head} ({}) REFERENCES {referenced} ({referenced_columns}) ON DELETE {} ON UPDATE {}",
        column_list(table, &key.columns)?,
        action(key.on_delete)?,
        action(key.on_update)?,
    );
    let summary = match &key.name {
        Some(name) => format!("add foreign key {name}"),
        None => "add foreign key".to_owned(),
    };
    Ok(Clause::new(sql, summary).rewrite(true))
}

fn table_option(table: &TableModel, field: Field) -> Result<Clause, PlanError> {
    let word = |what: &str, value: &Option<String>| -> Result<String, PlanError> {
        match value {
            Some(value) if plain_word(value) => Ok(value.clone()),
            Some(value) => Err(unsupported(format!("{what} {value:?} is not a plain name"))),
            None => Err(unsupported(format!("The table {what} cannot be unset"))),
        }
    };
    let options = &table.options;
    Ok(match field {
        Field::Engine => {
            let engine = word("engine", &options.engine)?;
            Clause::new(format!("ENGINE = {engine}"), format!("set engine {engine}")).rewrite(true)
        }
        Field::Charset => {
            let charset = word("character set", &options.charset)?;
            Clause::new(
                format!("DEFAULT CHARACTER SET = {charset}"),
                format!("set default character set {charset}"),
            )
        }
        Field::Collation => {
            let collation = word("collation", &options.collation)?;
            Clause::new(
                format!("COLLATE = {collation}"),
                format!("set default collation {collation}"),
            )
        }
        Field::RowFormat => {
            let row_format = match &options.row_format {
                None => "DEFAULT".to_owned(),
                value => word("row format", value)?,
            };
            Clause::new(
                format!("ROW_FORMAT = {row_format}"),
                format!("set row format {row_format}"),
            )
            .rewrite(true)
        }
        Field::Strict | Field::WithoutRowid => {
            return Err(unsupported("STRICT and WITHOUT ROWID are SQLite options"))
        }
        _ => return Err(internal("unexpected table option")),
    })
}

/// Indexes (into `values`) of one longest strictly increasing subsequence,
/// chosen deterministically (the earliest end among the longest).
fn longest_increasing(values: &[usize]) -> HashSet<usize> {
    let n = values.len();
    let mut length = vec![1usize; n];
    let mut previous = vec![usize::MAX; n];
    for i in 0..n {
        for j in 0..i {
            if values[j] < values[i] && length[j] + 1 > length[i] {
                length[i] = length[j] + 1;
                previous[i] = j;
            }
        }
    }
    let mut kept = HashSet::new();
    let Some(mut at) = (0..n).max_by_key(|i| (length[*i], usize::MAX - *i)) else {
        return kept;
    };
    loop {
        kept.insert(at);
        if previous[at] == usize::MAX {
            break;
        }
        at = previous[at];
    }
    kept
}

/// Columns that need `FIRST`/`AFTER` to reach the draft order.
fn positioned_columns(old: &TableModel, new: &TableModel) -> HashSet<Origin> {
    let mut positioned = HashSet::new();
    if !columns_reordered(old, new) {
        return positioned;
    }
    let old_position: HashMap<Origin, usize> = old
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| (column.origin, index))
        .collect();
    let existing: Vec<(Origin, usize)> = new
        .columns
        .iter()
        .filter_map(|column| {
            old_position
                .get(&column.origin)
                .map(|position| (column.origin, *position))
        })
        .collect();
    let kept = longest_increasing(&existing.iter().map(|(_, p)| *p).collect::<Vec<_>>());
    for (index, (origin, _)) in existing.iter().enumerate() {
        if !kept.contains(&index) {
            positioned.insert(*origin);
        }
    }
    let last_existing = new
        .columns
        .iter()
        .rposition(|column| !column.origin.is_new());
    for (index, column) in new.columns.iter().enumerate() {
        if column.origin.is_new() && last_existing.is_some_and(|last| index < last) {
            positioned.insert(column.origin);
        }
    }
    positioned
}

/// Column clauses in draft order, then column drops.
fn column_clauses(
    old: &TableModel,
    new: &TableModel,
    features: ServerFeatures,
) -> Result<(Vec<Clause>, Vec<Clause>), PlanError> {
    let positioned = positioned_columns(old, new);
    let mut clauses = Vec::new();
    for (index, column) in new.columns.iter().enumerate() {
        let position = positioned.contains(&column.origin).then(|| {
            if index == 0 {
                " FIRST".to_owned()
            } else {
                format!(" AFTER {}", ident(&new.columns[index - 1].name))
            }
        });
        let clause = match old
            .column(column.origin)
            .filter(|_| !column.origin.is_new())
        {
            None => add_column(column, position, features)?,
            Some(previous) => match alter_column(previous, column, position, features)? {
                Some(clause) => clause,
                None => continue,
            },
        };
        clauses.push(clause);
    }
    let drops = old
        .columns
        .iter()
        .filter(|column| new.column(column.origin).is_none())
        .map(|column| {
            Clause::new(
                format!("DROP COLUMN {}", ident(&column.name)),
                format!("drop column {}", column.name),
            )
            .destructive(true)
            .instant(features.instant_add_column_anywhere)
            .rewrite(!features.instant_add_column_anywhere)
        })
        .collect();
    Ok((clauses, drops))
}

fn add_column(
    column: &ColumnModel,
    position: Option<String>,
    features: ServerFeatures,
) -> Result<Clause, PlanError> {
    let definition = column_definition(column)?;
    let positioned = position.is_some();
    let stored = column.generated.as_ref().is_some_and(|g| g.stored);
    let auto_increment = column.identity == Identity::AutoIncrement;
    let instant = features.instant_add_column
        && !stored
        && !auto_increment
        && (!positioned || features.instant_add_column_anywhere);
    let destructive = !column.nullable
        && column.default.is_none()
        && column.generated.is_none()
        && column.identity == Identity::None;
    Ok(Clause::new(
        format!(
            "ADD COLUMN {} {definition}{}",
            ident(&column.name),
            position.unwrap_or_default()
        ),
        format!("add column {}", column.name),
    )
    .destructive(destructive)
    .instant(instant)
    .rewrite(stored || auto_increment || (positioned && !features.instant_add_column_anywhere)))
}

fn alter_column(
    previous: &ColumnModel,
    column: &ColumnModel,
    position: Option<String>,
    features: ServerFeatures,
) -> Result<Option<Clause>, PlanError> {
    let renamed = previous.name != column.name;
    let fields = crate::schema_edit::column_field_changes(previous, column);
    let comment_changed = previous.comment != column.comment;
    let positioned = position.is_some();
    if fields.is_empty() && !comment_changed && !positioned {
        if !renamed {
            return Ok(None);
        }
        if features.rename_column {
            return Ok(Some(
                Clause::new(
                    format!(
                        "RENAME COLUMN {} TO {}",
                        ident(&previous.name),
                        ident(&column.name)
                    ),
                    format!("rename column {} to {}", previous.name, column.name),
                )
                .instant(features.instant_add_column_anywhere),
            ));
        }
    }
    let definition = column_definition(column)?;
    let position = position.unwrap_or_default();
    let (sql, verb) = if renamed {
        (
            format!(
                "CHANGE COLUMN {} {} {definition}{position}",
                ident(&previous.name),
                ident(&column.name)
            ),
            format!("change column {} to {}", previous.name, column.name),
        )
    } else {
        (
            format!(
                "MODIFY COLUMN {} {definition}{position}",
                ident(&column.name)
            ),
            format!("modify column {}", column.name),
        )
    };
    let mut details: Vec<&str> = fields.iter().map(|field| field_label(*field)).collect();
    if comment_changed {
        details.push("comment");
    }
    if positioned {
        details.push("position");
    }
    let summary = if details.is_empty() {
        verb
    } else {
        format!("{verb} ({})", details.join(", "))
    };
    let narrowing = fields.contains(&ColumnField::DataType)
        && !is_widening(SchemaEngine::MySql, &previous.data_type, &column.data_type);
    let destructive = narrowing
        || (fields.contains(&ColumnField::Nullable) && !column.nullable)
        || (fields.contains(&ColumnField::Generated) && column.generated.is_some())
        || fields.contains(&ColumnField::Charset);
    let rewrite = positioned
        || fields.iter().any(|field| {
            matches!(
                field,
                ColumnField::DataType
                    | ColumnField::Charset
                    | ColumnField::Collation
                    | ColumnField::Nullable
                    | ColumnField::Generated
                    | ColumnField::Identity
            )
        });
    let instant = features.instant_add_column
        && !renamed
        && !positioned
        && !comment_changed
        && fields == [ColumnField::Default];
    Ok(Some(
        Clause::new(sql, summary)
            .destructive(destructive)
            .instant(instant)
            .rewrite(rewrite),
    ))
}
