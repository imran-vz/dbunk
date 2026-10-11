//! The SQLite planner: a fast path of `ALTER TABLE` and index statements
//! when every step qualifies, otherwise the documented table rebuild
//! rendered from the draft. Pure and deterministic; always one atomic group.
use super::capture::{REFERENCING_TRIGGER, ROW_ESTIMATE_CAP, TRIGGER, VIEW};
use super::definition::qualify_trigger;
use super::lexer::{
    all_names, dequote, is_column_reference, mentioned_names, tokenize, trim_space, Kind,
};
use super::{qualified, quote, quote_literal};
use crate::schema_edit::{
    normalize_type, plan_steps, ColumnExtra, ColumnModel, DefaultValue, Deferrable, Dependent,
    EngineNote, EnginePlan, ForeignKeyModel, Identity, IndexModel, IndexTarget, KeyModel, NoteKind,
    Origin, PlanError, PlanStep, PreviewGroup, PreviewStatement, ReferentialAction, SchemaBaseline,
    SchemaDraft, SchemaObject, SqlFragment, TableModel,
};

/// The rebuild creates the new table under this prefix plus the old name.
pub(super) const REBUILD_PREFIX: &str = "_dbunk_rebuild_";
/// A case-only table rename goes through this intermediate name.
const RENAME_PREFIX: &str = "_dbunk_rename_";
/// Rows from which a rebuild or column drop gets a large-table note.
pub(super) const LARGE_TABLE_ROWS: u64 = 100_000;

fn unsupported(reason: impl Into<String>) -> PlanError {
    PlanError::Unsupported {
        reason: reason.into(),
    }
}

fn statement(sql: String, summary: impl Into<String>, destructive: bool) -> PreviewStatement {
    PreviewStatement {
        sql,
        summary: summary.into(),
        destructive,
    }
}

pub(super) fn plan(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
) -> Result<EnginePlan, PlanError> {
    refuse_unsupported(draft)?;
    let expected = expected_table(baseline, draft);
    let steps = plan_steps(baseline, draft);
    let mut plan = match fast_path(baseline, &expected, &steps) {
        Some(statements) if !statements.is_empty() => EnginePlan {
            notes: fast_notes(baseline, &steps),
            statements,
            groups: Vec::new(),
        },
        _ => rebuild(baseline, &expected)?,
    };
    plan.groups = vec![PreviewGroup::Atomic {
        statements: (0..plan.statements.len()).collect(),
    }];
    Ok(plan)
}

/// True when the reviewed plan is a rebuild (the executor relaxes
/// `legacy_alter_table` only then).
pub(super) fn is_rebuild(notes: &[EngineNote]) -> bool {
    notes.iter().any(|note| note.kind == NoteKind::Rebuild)
}

/// Draft attributes SQLite cannot hold, refused before anything is planned.
fn refuse_unsupported(draft: &SchemaDraft) -> Result<(), PlanError> {
    let table = &draft.table;
    if table.comment.is_some() || table.columns.iter().any(|c| c.comment.is_some()) {
        return Err(unsupported("SQLite has no table or column comments"));
    }
    if table
        .name
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sqlite_"))
    {
        return Err(unsupported(
            "Table names starting with sqlite_ are reserved",
        ));
    }
    let options = &table.options;
    if options.engine.is_some()
        || options.charset.is_some()
        || options.collation.is_some()
        || options.row_format.is_some()
    {
        return Err(unsupported(
            "SQLite has no engine, character set, collation or row format table options",
        ));
    }
    for column in &table.columns {
        if column.extra != ColumnExtra::default() {
            return Err(unsupported(format!(
                "Column {}: character sets, ON UPDATE and invisible columns are MySQL only",
                column.name
            )));
        }
        match column.identity {
            Identity::None => {}
            Identity::AutoIncrement => {
                let alone_in_key = table
                    .primary_key
                    .as_ref()
                    .is_some_and(|key| key.columns == [column.origin]);
                if !alone_in_key
                    || normalize_type(&column.data_type) != "integer"
                    || options.without_rowid
                {
                    return Err(unsupported(format!(
                        "Column {}: AUTOINCREMENT needs a single INTEGER PRIMARY KEY column in a \
                         rowid table",
                        column.name
                    )));
                }
            }
            Identity::Always | Identity::ByDefault => {
                return Err(unsupported("Identity columns are PostgreSQL only"))
            }
        }
    }
    if options.without_rowid && table.primary_key.is_none() {
        return Err(unsupported("A WITHOUT ROWID table needs a primary key"));
    }
    for key in &table.foreign_keys {
        if key.referenced_schema.is_some() {
            return Err(unsupported(
                "SQLite foreign keys can only reference tables in the same schema",
            ));
        }
    }
    if table.checks.iter().any(|check| !check.enforced) {
        return Err(unsupported("SQLite always enforces CHECK constraints"));
    }
    for index in &table.indexes {
        if index.method.is_some()
            || !index.include.is_empty()
            || index.members.iter().any(|m| m.prefix_length.is_some())
        {
            return Err(unsupported(format!(
                "Index {}: SQLite indexes have no method, INCLUDE columns or prefix lengths",
                index.name
            )));
        }
    }
    Ok(())
}

/// Columns renamed by the draft, as (old name, new name).
fn renamed_columns(baseline: &SchemaBaseline, draft: &TableModel) -> Vec<(String, String)> {
    baseline
        .table
        .columns
        .iter()
        .filter_map(|old| {
            let new = draft.column(old.origin)?;
            (new.name != old.name).then(|| (old.name.clone(), new.name.clone()))
        })
        .collect()
}

/// `text` with references to renamed columns rewritten to the new names.
fn rename_references(text: &str, renames: &[(String, String)]) -> String {
    let Ok(tokens) = tokenize(text) else {
        return text.to_owned();
    };
    let mut out = String::with_capacity(text.len());
    let mut copied = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if !is_column_reference(text, &tokens, index) {
            continue;
        }
        let name = dequote(token.text(text));
        if let Some((_, new)) = renames
            .iter()
            .find(|(old, _)| old.eq_ignore_ascii_case(&name))
        {
            out.push_str(&text[copied..token.start]);
            out.push_str(&quote(new));
            copied = token.end;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// The table the plan produces: the draft, with untouched baseline
/// expressions and self-references following column and table renames the
/// way SQLite's own `RENAME` rewrites them. Edited and new expressions are
/// taken as written against the draft's names. Used to render the rebuild
/// and to verify the applied structure.
pub(super) fn expected_table(baseline: &SchemaBaseline, draft: &SchemaDraft) -> TableModel {
    let old = &baseline.table;
    let mut table = draft.table.clone();
    let renames = renamed_columns(baseline, &table);
    let table_renamed = old.name != table.name;
    if renames.is_empty() && !table_renamed {
        return table;
    }
    let rewrite = |fragment: &mut SqlFragment| {
        *fragment = SqlFragment::new(rename_references(fragment.as_str(), &renames));
    };
    if !renames.is_empty() {
        for check in &mut table.checks {
            if old
                .check(check.origin)
                .is_some_and(|before| before.expression == check.expression)
            {
                rewrite(&mut check.expression);
            }
        }
        for column in &mut table.columns {
            let before = old
                .column(column.origin)
                .and_then(|before| before.generated.as_ref());
            if let (Some(before), Some(generated)) = (before, column.generated.as_mut()) {
                if before.expression == generated.expression {
                    rewrite(&mut generated.expression);
                }
            }
        }
        for index in &mut table.indexes {
            let Some(before) = old.index(index.origin) else {
                continue;
            };
            for (position, member) in index.members.iter_mut().enumerate() {
                let untouched = before
                    .members
                    .get(position)
                    .is_some_and(|previous| previous.target == member.target);
                if let (true, IndexTarget::Expression(expression)) = (untouched, &mut member.target)
                {
                    rewrite(expression);
                }
            }
            if let (Some(previous), Some(predicate)) = (&before.predicate, index.predicate.as_mut())
            {
                if previous == predicate {
                    rewrite(predicate);
                }
            }
        }
    }
    for key in &mut table.foreign_keys {
        let Some(before) = old.foreign_key(key.origin) else {
            continue;
        };
        let self_reference = before.referenced_table.eq_ignore_ascii_case(&old.name)
            && key.referenced_table == before.referenced_table;
        if !self_reference {
            continue;
        }
        if table_renamed {
            key.referenced_table = table.name.clone();
        }
        if key.referenced_columns == before.referenced_columns {
            for column in &mut key.referenced_columns {
                if let Some((_, new)) = renames
                    .iter()
                    .find(|(old, _)| old.eq_ignore_ascii_case(column))
                {
                    *column = new.clone();
                }
            }
        }
    }
    table
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// One literal or name token, or a signed number: SQLite takes these as a
/// default without parentheses.
fn is_simple_term(text: &str) -> bool {
    let Ok(tokens) = tokenize(text) else {
        return false;
    };
    // Nothing but whitespace around and between the tokens: a comment would
    // swallow whatever follows a bare default.
    let bare = |range: &str| trim_space(range).is_empty();
    let covered = match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => {
            bare(&text[..first.start])
                && bare(&text[last.end..])
                && tokens
                    .windows(2)
                    .all(|pair| bare(&text[pair[0].end..pair[1].start]))
        }
        _ => false,
    };
    if !covered {
        return false;
    }
    match tokens.as_slice() {
        [token] => matches!(
            token.kind,
            Kind::Number | Kind::String | Kind::Blob | Kind::Word | Kind::Quoted
        ),
        [sign, number] => {
            (sign.is_punct(text, "+") || sign.is_punct(text, "-")) && number.kind == Kind::Number
        }
        _ => false,
    }
}

/// The text after `DEFAULT`, or `None` for no default.
pub(super) fn render_default(value: &DefaultValue) -> Option<String> {
    match value {
        DefaultValue::None => None,
        DefaultValue::Literal(text) => Some(quote_literal(text)),
        DefaultValue::Expression(sql) if is_simple_term(sql.as_str()) => Some(sql.0.clone()),
        DefaultValue::Expression(sql) => Some(format!("({})", closed(sql.as_str()))),
    }
}

/// A fragment followed by a closing token: a trailing line comment would
/// swallow it, so end the line first.
fn closed(text: &str) -> String {
    if text.contains("--") {
        format!("{text}\n")
    } else {
        text.to_owned()
    }
}

fn collation_name(name: &str) -> String {
    let plain = name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if plain {
        name.to_owned()
    } else {
        quote(name)
    }
}

fn constraint_prefix(name: &Option<String>) -> String {
    match name {
        Some(name) => format!("CONSTRAINT {} ", quote(name)),
        None => String::new(),
    }
}

fn name_list(table: &TableModel, origins: &[Origin]) -> String {
    table
        .column_names(origins)
        .iter()
        .map(|name| quote(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One column definition. `primary_key` is set when the column carries the
/// table's single-column primary key inline (needed for AUTOINCREMENT).
fn column_sql(column: &ColumnModel, primary_key: Option<&KeyModel>) -> String {
    let mut sql = quote(&column.name);
    let data_type = column.data_type.trim();
    if !data_type.is_empty() {
        sql.push(' ');
        sql.push_str(data_type);
    }
    if let Some(key) = primary_key {
        sql.push(' ');
        sql.push_str(&constraint_prefix(&key.name));
        sql.push_str("PRIMARY KEY");
        if column.identity == Identity::AutoIncrement {
            sql.push_str(" AUTOINCREMENT");
        }
    }
    if !column.nullable {
        sql.push_str(" NOT NULL");
    }
    if let Some(default) = render_default(&column.default) {
        sql.push_str(" DEFAULT ");
        sql.push_str(&default);
    }
    if let Some(collation) = &column.collation {
        sql.push_str(" COLLATE ");
        sql.push_str(&collation_name(collation));
    }
    if let Some(generated) = &column.generated {
        sql.push_str(" GENERATED ALWAYS AS (");
        sql.push_str(&closed(generated.expression.as_str()));
        sql.push_str(if generated.stored {
            ") STORED"
        } else {
            ") VIRTUAL"
        });
    }
    sql
}

fn action_sql(key: &ForeignKeyModel) -> String {
    let mut sql = String::new();
    if key.on_update != ReferentialAction::NoAction {
        sql.push_str(" ON UPDATE ");
        sql.push_str(key.on_update.sql());
    }
    if key.on_delete != ReferentialAction::NoAction {
        sql.push_str(" ON DELETE ");
        sql.push_str(key.on_delete.sql());
    }
    match key.deferrable {
        Deferrable::Never => {}
        Deferrable::InitiallyImmediate => sql.push_str(" DEFERRABLE INITIALLY IMMEDIATE"),
        Deferrable::InitiallyDeferred => sql.push_str(" DEFERRABLE INITIALLY DEFERRED"),
    }
    sql
}

/// `CREATE TABLE schema.name (…)` for the whole table, one item per line.
pub(super) fn create_table_sql(schema: &str, name: &str, table: &TableModel) -> String {
    let inline_key = table
        .primary_key
        .as_ref()
        .filter(|key| key.columns.len() == 1);
    let mut items: Vec<String> = table
        .columns
        .iter()
        .map(|column| {
            let key = inline_key.filter(|key| key.columns[0] == column.origin);
            column_sql(column, key)
        })
        .collect();
    if let Some(key) = table
        .primary_key
        .as_ref()
        .filter(|key| key.columns.len() > 1)
    {
        items.push(format!(
            "{}PRIMARY KEY ({})",
            constraint_prefix(&key.name),
            name_list(table, &key.columns)
        ));
    }
    for key in &table.uniques {
        items.push(format!(
            "{}UNIQUE ({})",
            constraint_prefix(&key.name),
            name_list(table, &key.columns)
        ));
    }
    for check in &table.checks {
        items.push(format!(
            "{}CHECK ({})",
            constraint_prefix(&check.name),
            closed(check.expression.as_str())
        ));
    }
    for key in &table.foreign_keys {
        let referenced = if key.referenced_columns.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                key.referenced_columns
                    .iter()
                    .map(|name| quote(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        items.push(format!(
            "{}FOREIGN KEY ({}) REFERENCES {}{referenced}{}",
            constraint_prefix(&key.name),
            name_list(table, &key.columns),
            quote(&key.referenced_table),
            action_sql(key)
        ));
    }
    let mut sql = format!(
        "CREATE TABLE {} (\n  {}\n)",
        qualified(schema, name),
        items.join(",\n  ")
    );
    let mut options = Vec::new();
    if table.options.without_rowid {
        options.push("WITHOUT ROWID");
    }
    if table.options.strict {
        options.push("STRICT");
    }
    if !options.is_empty() {
        sql.push(' ');
        sql.push_str(&options.join(", "));
    }
    sql
}

/// `CREATE [UNIQUE] INDEX schema.index ON table (…) [WHERE …]`.
pub(super) fn create_index_sql(
    schema: &str,
    table_name: &str,
    index: &IndexModel,
    table: &TableModel,
) -> String {
    let members = index
        .members
        .iter()
        .map(|member| {
            let mut text = match &member.target {
                IndexTarget::Column(origin) => quote(table.column_name(*origin).unwrap_or("?")),
                IndexTarget::Expression(sql) => closed(sql.as_str()),
            };
            if let Some(collation) = &member.collation {
                text.push_str(" COLLATE ");
                text.push_str(&collation_name(collation));
            }
            if member.descending {
                text.push_str(" DESC");
            }
            text
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "CREATE {}INDEX {} ON {} ({members})",
        if index.unique { "UNIQUE " } else { "" },
        qualified(schema, &index.name),
        quote(table_name)
    );
    if let Some(predicate) = &index.predicate {
        sql.push_str(" WHERE ");
        sql.push_str(predicate.as_str());
    }
    sql
}

fn drop_index_sql(schema: &str, index: &IndexModel) -> String {
    format!("DROP INDEX {}", qualified(schema, &index.name))
}

/// The column SQLite uses as the rowid: a rowid table's single-column
/// primary key declared exactly `INTEGER`.
pub(super) fn rowid_alias(table: &TableModel) -> Option<Origin> {
    if table.options.without_rowid {
        return None;
    }
    let key = table.primary_key.as_ref()?;
    let [origin] = key.columns.as_slice() else {
        return None;
    };
    let column = table.column(*origin)?;
    (normalize_type(&column.data_type) == "integer").then_some(*origin)
}

// ---------------------------------------------------------------------------
// Fast path
// ---------------------------------------------------------------------------

/// `ALTER TABLE … ADD COLUMN` restrictions: no key, a constant default,
/// NOT NULL only with a non-NULL default, no STORED generated column.
fn add_column_ok(column: &ColumnModel, table: &TableModel) -> bool {
    let in_key = table
        .primary_key
        .iter()
        .chain(table.uniques.iter())
        .any(|key| key.columns.contains(&column.origin))
        || table
            .foreign_keys
            .iter()
            .any(|key| key.columns.contains(&column.origin));
    let constant_default = match &column.default {
        DefaultValue::None | DefaultValue::Literal(_) => true,
        DefaultValue::Expression(sql) => {
            let text = sql.as_str().trim();
            is_simple_term(text)
                && !["CURRENT_TIME", "CURRENT_DATE", "CURRENT_TIMESTAMP"]
                    .iter()
                    .any(|word| text.eq_ignore_ascii_case(word))
        }
    };
    let null_default = match &column.default {
        DefaultValue::None => true,
        DefaultValue::Literal(_) => false,
        DefaultValue::Expression(sql) => sql.as_str().trim().eq_ignore_ascii_case("NULL"),
    };
    !in_key
        && column.identity == Identity::None
        && constant_default
        && !column.generated.as_ref().is_some_and(|g| g.stored)
        && (column.nullable || (!null_default && column.generated.is_none()))
}

/// `DROP COLUMN` is only planned when nothing could mention the column:
/// no key or foreign key, and its name appears in no check, generated
/// expression, index expression, trigger or view.
fn drop_column_ok(baseline: &SchemaBaseline, table: &TableModel, column: &ColumnModel) -> bool {
    let old = &baseline.table;
    if old
        .primary_key
        .iter()
        .chain(old.uniques.iter())
        .any(|key| key.columns.contains(&column.origin))
        || old
            .foreign_keys
            .iter()
            .any(|key| key.columns.contains(&column.origin))
    {
        return false;
    }
    let name = column.name.to_ascii_lowercase();
    let mentions = |text: &str| mentioned_names(text).contains(&name);
    let expressions = old
        .checks
        .iter()
        .chain(table.checks.iter())
        .map(|check| check.expression.as_str())
        .chain(
            old.columns
                .iter()
                .chain(table.columns.iter())
                .filter_map(|c| c.generated.as_ref())
                .map(|g| g.expression.as_str()),
        )
        .chain(
            old.indexes
                .iter()
                .chain(table.indexes.iter())
                .flat_map(|index| {
                    index
                        .members
                        .iter()
                        .filter_map(|member| match &member.target {
                            IndexTarget::Expression(sql) => Some(sql.as_str()),
                            IndexTarget::Column(_) => None,
                        })
                        .chain(index.predicate.iter().map(SqlFragment::as_str))
                }),
        )
        .collect::<Vec<_>>();
    if expressions.into_iter().any(mentions) {
        return false;
    }
    !baseline.dependents.iter().any(|dependent| {
        dependent
            .definition
            .as_deref()
            .is_some_and(|sql| all_names(sql).contains(&name))
    })
}

/// The fast-path statements, or `None` when any step needs the rebuild.
fn fast_path(
    baseline: &SchemaBaseline,
    table: &TableModel,
    steps: &[PlanStep],
) -> Option<Vec<PreviewStatement>> {
    let features = baseline.server.features;
    let old = &baseline.table;
    let schema = old.schema.as_str();
    let mut current = old.name.clone();
    let mut statements = Vec::new();
    let mut recreate = Vec::new();
    for step in steps {
        match step {
            PlanStep::DropIndex { id } => {
                let index = old.index(Origin::Existing(*id))?;
                statements.push(statement(
                    drop_index_sql(schema, index),
                    format!("Drop index {}", index.name),
                    true,
                ));
            }
            PlanStep::RenameConstraint {
                object: SchemaObject::Index,
                id,
            } => {
                // SQLite cannot rename an index: drop it now, create it
                // under the new name at the end.
                let index = old.index(Origin::Existing(*id))?;
                statements.push(statement(
                    drop_index_sql(schema, index),
                    format!("Drop index {} to re-create it renamed", index.name),
                    true,
                ));
                recreate.push(Origin::Existing(*id));
            }
            PlanStep::RenameColumn { id } => {
                if !features.rename_column {
                    return None;
                }
                let before = old.column(Origin::Existing(*id))?;
                let after = table.column(Origin::Existing(*id))?;
                statements.push(statement(
                    format!(
                        "ALTER TABLE {} RENAME COLUMN {} TO {}",
                        qualified(schema, &current),
                        quote(&before.name),
                        quote(&after.name)
                    ),
                    format!("Rename column {} to {}", before.name, after.name),
                    false,
                ));
            }
            PlanStep::RenameTable => {
                if current.eq_ignore_ascii_case(&table.name) {
                    // SQLite refuses a rename to the same name in another case.
                    let through = format!("{RENAME_PREFIX}{}", table.name);
                    statements.push(statement(
                        format!(
                            "ALTER TABLE {} RENAME TO {}",
                            qualified(schema, &current),
                            quote(&through)
                        ),
                        format!("Rename table {current} to a temporary name"),
                        false,
                    ));
                    current = through;
                }
                statements.push(statement(
                    format!(
                        "ALTER TABLE {} RENAME TO {}",
                        qualified(schema, &current),
                        quote(&table.name)
                    ),
                    format!("Rename table to {}", table.name),
                    false,
                ));
                current = table.name.clone();
            }
            PlanStep::AddColumn { origin } => {
                let column = table.column(*origin)?;
                if !add_column_ok(column, table) {
                    return None;
                }
                statements.push(statement(
                    format!(
                        "ALTER TABLE {} ADD COLUMN {}",
                        qualified(schema, &current),
                        column_sql(column, None)
                    ),
                    format!("Add column {}", column.name),
                    false,
                ));
            }
            PlanStep::DropColumn { id } => {
                let column = old.column(Origin::Existing(*id))?;
                if !features.drop_column || !drop_column_ok(baseline, table, column) {
                    return None;
                }
                statements.push(statement(
                    format!(
                        "ALTER TABLE {} DROP COLUMN {}",
                        qualified(schema, &current),
                        quote(&column.name)
                    ),
                    format!("Drop column {}", column.name),
                    true,
                ));
            }
            PlanStep::CreateIndex { origin } => {
                let index = table.index(*origin)?;
                statements.push(statement(
                    create_index_sql(schema, &current, index, table),
                    format!("Create index {}", index.name),
                    false,
                ));
            }
            _ => return None,
        }
    }
    for origin in recreate {
        let index = table.index(origin)?;
        statements.push(statement(
            create_index_sql(schema, &current, index, table),
            format!("Create index {}", index.name),
            false,
        ));
    }
    Some(statements)
}

fn names_of(dependents: &[&Dependent]) -> String {
    dependents
        .iter()
        .map(|dependent| dependent.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn large_table_note(baseline: &SchemaBaseline, what: &str) -> Option<EngineNote> {
    let rows = baseline.estimated_rows?;
    if rows < LARGE_TABLE_ROWS {
        return None;
    }
    let count = if rows >= ROW_ESTIMATE_CAP {
        format!("More than {LARGE_TABLE_ROWS} rows")
    } else {
        format!("About {rows} rows")
    };
    Some(EngineNote::new(
        NoteKind::LargeTable,
        format!("{count} are {what} while the database is locked for writing"),
    ))
}

fn dependent_notes(baseline: &SchemaBaseline, rebuild: bool) -> Vec<EngineNote> {
    let mut notes = Vec::new();
    let of_kind = |kind: &str| -> Vec<&Dependent> {
        baseline
            .dependents
            .iter()
            .filter(|dependent| dependent.kind == kind)
            .collect()
    };
    let triggers = of_kind(TRIGGER);
    if rebuild && !triggers.is_empty() {
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "Triggers re-created from their saved definitions: {}",
                names_of(&triggers)
            ),
        ));
    }
    let mut others = of_kind(VIEW);
    others.extend(of_kind(REFERENCING_TRIGGER));
    if !others.is_empty() {
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "Views and triggers that mention this table: {}. {}",
                names_of(&others),
                if rebuild {
                    "They are not changed; check them if columns are renamed or dropped"
                } else {
                    "SQLite updates their references to renamed columns and tables"
                }
            ),
        ));
    }
    if !baseline.inbound.is_empty() {
        let mut tables: Vec<&str> = baseline
            .inbound
            .iter()
            .map(|reference| reference.table.as_str())
            .collect();
        tables.dedup();
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "Foreign keys in {} reference this table; when foreign keys are enforced they \
                 are checked before commit",
                tables.join(", ")
            ),
        ));
    }
    notes
}

fn fast_notes(baseline: &SchemaBaseline, steps: &[PlanStep]) -> Vec<EngineNote> {
    let mut notes = Vec::new();
    if steps
        .iter()
        .any(|step| matches!(step, PlanStep::DropColumn { .. }))
    {
        notes.push(EngineNote::new(
            NoteKind::Rewrite,
            "Dropping a column rewrites every row of the table",
        ));
        notes.extend(large_table_note(baseline, "rewritten"));
    }
    notes.extend(dependent_notes(baseline, false));
    notes
}

// ---------------------------------------------------------------------------
// Rebuild
// ---------------------------------------------------------------------------

/// The 12-step rebuild (https://sqlite.org/lang_altertable.html#otheralter):
/// create the new table from the draft, copy every row, drop the old table,
/// rename the new one, then re-create indexes and triggers. The executor
/// handles `foreign_keys`, the transaction and verification around it.
fn rebuild(baseline: &SchemaBaseline, table: &TableModel) -> Result<EnginePlan, PlanError> {
    let old = &baseline.table;
    let schema = old.schema.as_str();
    let temporary = format!("{REBUILD_PREFIX}{}", old.name);
    let triggers: Vec<&Dependent> = baseline
        .dependents
        .iter()
        .filter(|dependent| dependent.kind == TRIGGER)
        .collect();
    let renames = renamed_columns(baseline, table);
    if !triggers.is_empty() && (old.name != table.name || !renames.is_empty()) {
        return Err(unsupported(
            "This table has triggers, which a rebuild re-creates from their saved text. Rename \
             the table or its columns on their own first, then make the other changes",
        ));
    }
    // A rebuild runs with legacy_alter_table on, so SQLite does not rewrite
    // other tables' REFERENCES or views that name the old table or columns.
    let referenced = !baseline.inbound.is_empty()
        || baseline
            .dependents
            .iter()
            .any(|dependent| dependent.kind == VIEW || dependent.kind == REFERENCING_TRIGGER);
    if referenced && (old.name != table.name || !renames.is_empty()) {
        return Err(unsupported(
            "Other tables, views or triggers refer to this table, and a rebuild would leave them \
             pointing at the old names. Rename the table or its columns on their own first, then \
             make the other changes",
        ));
    }
    for column in &old.columns {
        if table.column(column.origin).is_some() {
            continue;
        }
        let name = column.name.to_ascii_lowercase();
        if let Some(trigger) = triggers.iter().find(|trigger| {
            trigger
                .definition
                .as_deref()
                .is_some_and(|sql| all_names(sql).contains(&name))
        }) {
            return Err(unsupported(format!(
                "Trigger {} uses column {}, which this change drops; drop or change the trigger \
                 first",
                trigger.name, column.name
            )));
        }
    }

    // Copy lists: surviving columns by origin, plus the rowid.
    let mut targets = Vec::new();
    let mut sources = Vec::new();
    for column in &table.columns {
        if column.generated.is_some() {
            continue;
        }
        if let Some(before) = old
            .column(column.origin)
            .filter(|_| !column.origin.is_new())
        {
            targets.push(quote(&column.name));
            sources.push(quote(&before.name));
        }
    }
    let rowid_name = ["rowid", "_rowid_", "oid"].into_iter().find(|keyword| {
        !old.columns
            .iter()
            .chain(table.columns.iter())
            .any(|column| column.name.eq_ignore_ascii_case(keyword))
    });
    let old_alias = rowid_alias(old);
    let mut rowids_preserved = false;
    let rowid_tables = !old.options.without_rowid && !table.options.without_rowid;
    if rowid_tables {
        match rowid_alias(table) {
            None => {
                if let Some(keyword) = rowid_name {
                    targets.push(keyword.to_owned());
                    sources.push(keyword.to_owned());
                    rowids_preserved = true;
                }
            }
            Some(alias) => match old.column(alias).filter(|_| !alias.is_new()) {
                Some(_) => rowids_preserved = old_alias == Some(alias),
                None => {
                    if let (Some(keyword), Some(column)) = (rowid_name, table.column(alias)) {
                        targets.push(quote(&column.name));
                        sources.push(keyword.to_owned());
                        rowids_preserved = true;
                    }
                }
            },
        }
    }
    if targets.is_empty() {
        return Err(unsupported(
            "The new table keeps none of the old columns, so no rows can be copied",
        ));
    }

    let mut statements = vec![
        statement(
            create_table_sql(schema, &temporary, table),
            format!("Create the new table as {temporary}"),
            true,
        ),
        statement(
            format!(
                "INSERT INTO {} ({}) SELECT {} FROM {}",
                qualified(schema, &temporary),
                targets.join(", "),
                sources.join(", "),
                qualified(schema, &old.name)
            ),
            "Copy every row into the new table",
            true,
        ),
    ];
    let autoincrement = |t: &TableModel| {
        t.columns
            .iter()
            .any(|c| c.identity == Identity::AutoIncrement)
    };
    if autoincrement(old) && autoincrement(table) {
        // Keep AUTOINCREMENT's high-water mark, so deleted ids are never
        // reused after the rebuild.
        let sequence = format!("{}.sqlite_sequence", quote(schema));
        let (new_name, old_name) = (quote_literal(&temporary), quote_literal(&old.name));
        statements.push(statement(
            format!(
                "INSERT INTO {sequence} (name, seq) SELECT {new_name}, 0 WHERE NOT EXISTS \
                 (SELECT 1 FROM {sequence} WHERE name = {new_name})"
            ),
            "Prepare the AUTOINCREMENT counter of the new table",
            true,
        ));
        statements.push(statement(
            format!(
                "UPDATE {sequence} SET seq = max(seq, coalesce((SELECT seq FROM {sequence} \
                 WHERE name = {old_name}), 0)) WHERE name = {new_name}"
            ),
            "Carry over the AUTOINCREMENT counter",
            true,
        ));
    }
    statements.push(statement(
        format!("DROP TABLE {}", qualified(schema, &old.name)),
        format!("Drop the old table {}", old.name),
        true,
    ));
    statements.push(statement(
        format!(
            "ALTER TABLE {} RENAME TO {}",
            qualified(schema, &temporary),
            quote(&table.name)
        ),
        format!("Rename the new table to {}", table.name),
        true,
    ));
    for index in &table.indexes {
        statements.push(statement(
            create_index_sql(schema, &table.name, index, table),
            format!("Create index {}", index.name),
            true,
        ));
    }
    for trigger in &triggers {
        let sql = trigger
            .definition
            .as_deref()
            .and_then(|sql| qualify_trigger(sql, schema))
            .ok_or_else(|| {
                unsupported(format!(
                    "Trigger {} cannot be re-created from its saved definition",
                    trigger.name
                ))
            })?;
        statements.push(statement(
            sql,
            format!("Re-create trigger {}", trigger.name),
            true,
        ));
    }

    let mut notes = vec![EngineNote::new(
        NoteKind::Rebuild,
        format!(
            "SQLite cannot make this change in place: it creates a new table, copies every row \
             and drops the old one. {}",
            if !rowid_tables {
                "The table has no rowids to keep."
            } else if rowids_preserved {
                "Rowids are preserved."
            } else {
                "Rowids are not preserved."
            }
        ),
    )];
    notes.extend(large_table_note(baseline, "copied"));
    notes.extend(dependent_notes(baseline, true));
    Ok(EnginePlan {
        statements,
        groups: Vec::new(),
        notes,
    })
}

#[cfg(test)]
#[path = "planner_tests.rs"]
mod tests;
