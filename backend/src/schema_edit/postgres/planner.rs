//! The PostgreSQL planner: `plan_steps` mapped onto typed `PgObjectOp`s and
//! rendered through `generate_object_ddl`, so validation, fragment checks and
//! destructive classification are shared with the object DDL flows.
//!
//! Pure and deterministic. Statements run in one atomic group, except
//! `VALIDATE CONSTRAINT` (with `validate_separately`) and `CREATE INDEX
//! CONCURRENTLY` (per index), which follow as standalone statements.

use crate::postgres::object_ddl::{
    generate_object_ddl, AddCheckOp, AddColumnOp, AddForeignKeyOp, AddPrimaryKeyOp, AddUniqueOp,
    AlterColumnCollationOp, AlterColumnTypeOp, CreateIndexOp, DropColumnOp, DropConstraintOp,
    DropIndexOp, NewColumnSpec, PgCommentTarget, PgDefaultValue, PgGeneratedColumn, PgIdentity,
    PgIdentityChange, PgIndexColumn, PgObjectError, PgObjectOp, PgReferentialAction,
    RenameColumnOp, RenameConstraintOp, RenameIndexOp, RenameObjectOp, SetColumnDefaultOp,
    SetColumnExpressionOp, SetColumnIdentityOp, SetColumnNullableOp, SetCommentOp,
    ValidateConstraintOp,
};
use crate::postgres::objects::{PgObjectKind, PgObjectRef};
use crate::postgres::sql_lex::{lex_sql, SqlToken};
use crate::quote_double;
use crate::schema_edit::{
    normalize_type, plan_steps, BaselineId, ColumnField, ColumnModel, DefaultValue, Deferrable,
    EngineNote, EnginePlan, Identity, IndexTarget, NoteKind, Origin, PlanError, PlanOptions,
    PlanStep, PreviewGroup, PreviewStatement, ReferentialAction, SchemaBaseline, SchemaDraft,
    SchemaObject, TableModel,
};

/// PostgreSQL identifiers are at most 63 bytes.
const MAX_IDENTIFIER_BYTES: usize = 63;
/// Above this estimate the review suggests NOT VALID and CONCURRENTLY.
pub(crate) const LARGE_TABLE_ROWS: u64 = 1_000_000;

/// How a statement runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// Inside the single atomic group.
    Atomic,
    /// `VALIDATE CONSTRAINT`, standalone after the atomic group commits.
    Validate,
    /// `CREATE INDEX CONCURRENTLY`, standalone and never in a transaction.
    ConcurrentIndex,
}

/// The planner's full output. Apply rebuilds it to learn each statement's
/// role and which indexes the atomic group leaves to concurrent builds.
#[derive(Clone, Debug)]
pub(crate) struct Built {
    pub(crate) statements: Vec<PreviewStatement>,
    pub(crate) roles: Vec<Role>,
    pub(crate) notes: Vec<EngineNote>,
    /// Draft indexes built `CONCURRENTLY` after the atomic group.
    #[cfg_attr(not(feature = "isolated-profile"), allow(dead_code))]
    pub(crate) deferred_indexes: Vec<Origin>,
}

impl Built {
    pub(crate) fn atomic_len(&self) -> usize {
        self.roles
            .iter()
            .take_while(|role| **role == Role::Atomic)
            .count()
    }

    pub(crate) fn groups(&self) -> Vec<PreviewGroup> {
        let atomic = self.atomic_len();
        let mut groups = Vec::new();
        if atomic > 0 {
            groups.push(PreviewGroup::Atomic {
                statements: (0..atomic).collect(),
            });
        }
        groups.extend(
            (atomic..self.statements.len()).map(|statement| PreviewGroup::Standalone { statement }),
        );
        groups
    }

    pub(crate) fn into_plan(self) -> EnginePlan {
        let groups = self.groups();
        EnginePlan {
            statements: self.statements,
            groups,
            notes: self.notes,
        }
    }
}

pub(crate) fn plan(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    build(baseline, draft, options).map(Built::into_plan)
}

fn unsupported(reason: impl Into<String>) -> PlanError {
    PlanError::Unsupported {
        reason: reason.into(),
    }
}

fn internal(reason: &str) -> PlanError {
    PlanError::Internal(reason.into())
}

/// The leading number of a version string: `16.4` → 16, `13.2 (Debian)` → 13.
pub(crate) fn server_major(version: &str) -> u32 {
    let digits: String = version
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap_or(0)
}

pub(crate) fn build(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    options: &PlanOptions,
) -> Result<Built, PlanError> {
    let steps = plan_steps(baseline, draft);
    let major = server_major(&baseline.server.version);
    precheck(baseline, draft, &steps, major)?;

    let mut emitter = Emitter {
        baseline,
        draft,
        options,
        major,
        schema: baseline.table.schema.clone(),
        table: baseline.table.name.clone(),
        atomic: Vec::new(),
        validate: Vec::new(),
        concurrent: Vec::new(),
        deferred: Vec::new(),
        reserved: Vec::new(),
    };

    // A dropped column whose name the draft reuses (a rename target or a
    // new column) is dropped before the renames and adds, not in phase 5.
    let early: Vec<BaselineId> = baseline
        .table
        .columns
        .iter()
        .filter(|column| draft.table.column(column.origin).is_none())
        .filter(|column| draft.table.columns.iter().any(|c| c.name == column.name))
        .filter_map(|column| column.origin.existing())
        .collect();

    for step in steps.iter().filter(|step| step.phase() == 1) {
        emitter.drop_step(step)?;
    }
    for id in &early {
        emitter.drop_column(*id)?;
    }
    emitter.renames(&steps, &early)?;
    for step in steps.iter().filter(|step| step.phase() >= 3) {
        emitter.step(step, &early)?;
    }

    let Emitter {
        atomic,
        validate,
        concurrent,
        deferred,
        ..
    } = emitter;
    let roles: Vec<Role> = std::iter::repeat_n(Role::Atomic, atomic.len())
        .chain(std::iter::repeat_n(Role::Validate, validate.len()))
        .chain(std::iter::repeat_n(Role::ConcurrentIndex, concurrent.len()))
        .collect();
    let ops: Vec<PgObjectOp> = atomic
        .into_iter()
        .chain(validate)
        .chain(concurrent)
        .collect();
    let rendered = generate_object_ddl(&ops).map_err(object_error)?;
    let statements: Vec<PreviewStatement> = rendered
        .statements
        .into_iter()
        .map(|statement| PreviewStatement {
            sql: statement.sql,
            summary: statement.summary,
            destructive: statement.destructive,
        })
        .collect();
    let mut built = Built {
        statements,
        roles,
        notes: Vec::new(),
        deferred_indexes: deferred,
    };
    built.notes = notes(baseline, draft, &steps, &built, options);
    Ok(built)
}

fn object_error(error: PgObjectError) -> PlanError {
    match error {
        PgObjectError::InvalidOp { reason, .. } => {
            let mut chars = reason.chars();
            let reason = match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => reason,
            };
            unsupported(reason)
        }
        other => PlanError::Internal(format!("{other:?}")),
    }
}

const SERIAL_TYPES: [&str; 6] = [
    "serial",
    "bigserial",
    "smallserial",
    "serial2",
    "serial4",
    "serial8",
];

/// Refusals that need the whole draft rather than one step.
fn precheck(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    steps: &[PlanStep],
    major: u32,
) -> Result<(), PlanError> {
    for step in steps {
        match step {
            PlanStep::ReorderColumns => {
                return Err(unsupported("Column reorder is not supported on PostgreSQL"))
            }
            PlanStep::SetTableOptions { .. } => {
                return Err(unsupported("PostgreSQL has no editable table options"))
            }
            _ => {}
        }
    }
    let old = &baseline.table;
    let new = &draft.table;
    for column in &new.columns {
        if column.extra != Default::default() {
            return Err(unsupported(format!(
                "Column {} uses MySQL-only attributes (character set, ON UPDATE or INVISIBLE)",
                column.name
            )));
        }
        let previous = old
            .column(column.origin)
            .filter(|_| !column.origin.is_new());
        let retyped = previous.is_none_or(|p| p.data_type != column.data_type);
        if retyped && SERIAL_TYPES.contains(&normalize_type(&column.data_type).as_str()) {
            return Err(unsupported(format!(
                "Column {}: serial is not a real type; use an identity column instead",
                column.name
            )));
        }
        if previous.is_none() && column.generated.as_ref().is_some_and(|g| !g.stored) && major < 18
        {
            return Err(unsupported(
                "Virtual generated columns need PostgreSQL 18 or later",
            ));
        }
        if previous.is_some_and(|p| !p.nullable)
            && column.nullable
            && new
                .primary_key
                .as_ref()
                .is_some_and(|key| key.columns.contains(&column.origin))
        {
            return Err(unsupported(format!(
                "Column {} is in the primary key and must stay NOT NULL",
                column.name
            )));
        }
    }
    // PostgreSQL drops a check with the column it uses; keeping the check in
    // the draft would fail verification, so say so before anything runs.
    for dropped in old
        .columns
        .iter()
        .filter(|column| new.column(column.origin).is_none())
    {
        if new.columns.iter().any(|c| c.name == dropped.name) {
            continue;
        }
        if let Some(check) = new
            .checks
            .iter()
            .find(|check| references_identifier(check.expression.as_str(), &dropped.name))
        {
            return Err(unsupported(format!(
                "Check {} uses column {}, which is being dropped; remove or edit the check first",
                check.name.as_deref().unwrap_or(check.expression.as_str()),
                dropped.name
            )));
        }
    }
    Ok(())
}

/// True when `expression` names `column` as an identifier (unquoted names
/// fold to lower case).
pub(crate) fn references_identifier(expression: &str, column: &str) -> bool {
    lex_sql(expression).is_ok_and(|tokens| {
        tokens.iter().any(|token| match token {
            SqlToken::Identifier(identifier) if identifier.quoted => identifier.value == column,
            SqlToken::Identifier(identifier) => identifier.value.to_lowercase() == column,
            _ => false,
        })
    })
}

fn truncate_identifier(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn temp_name(from: &str, current: &[String], targets: &[String]) -> String {
    let mut attempt = 0u32;
    loop {
        let suffix = if attempt == 0 {
            "_dbunk_tmp".to_owned()
        } else {
            format!("_dbunk_tmp{attempt}")
        };
        let name = format!(
            "{}{suffix}",
            truncate_identifier(from, MAX_IDENTIFIER_BYTES - suffix.len())
        );
        if !current.contains(&name) && !targets.contains(&name) {
            return name;
        }
        attempt += 1;
    }
}

/// Orders renames so none targets a name still in use, breaking cycles
/// (swapped names) with a temporary name. A target held by an object that
/// is not renamed is emitted as is; the server refuses it inside the
/// transaction.
pub(crate) fn resolve_renames<K: Clone>(
    mut current: Vec<String>,
    mut pending: Vec<(K, String, String)>,
) -> Vec<(K, String, String)> {
    let targets: Vec<String> = pending.iter().map(|(_, _, to)| to.clone()).collect();
    let mut out = Vec::new();
    let mut budget = pending.len() * 4 + 4;
    while !pending.is_empty() {
        let ready = pending.iter().position(|(_, from, to)| {
            from == to || !current.contains(to) || !pending.iter().any(|(_, other, _)| other == to)
        });
        let ready = if budget == 0 { Some(0) } else { ready };
        budget = budget.saturating_sub(1);
        match ready {
            Some(position) => {
                let (key, from, to) = pending.remove(position);
                if from != to {
                    current.retain(|name| *name != from);
                    current.push(to.clone());
                    out.push((key, from, to));
                }
            }
            None => {
                let (key, from, to) = pending.remove(0);
                let temp = temp_name(&from, &current, &targets);
                current.retain(|name| *name != from);
                current.push(temp.clone());
                out.push((key.clone(), from, temp.clone()));
                pending.push((key, temp, to));
            }
        }
    }
    out
}

fn default_value(value: &DefaultValue) -> Option<PgDefaultValue> {
    match value {
        DefaultValue::None => None,
        DefaultValue::Literal(value) => Some(PgDefaultValue::Literal {
            value: value.clone(),
        }),
        DefaultValue::Expression(sql) => Some(PgDefaultValue::Expression {
            sql: sql.as_str().to_owned(),
        }),
    }
}

fn identity(value: Identity) -> Option<PgIdentity> {
    match value {
        Identity::Always => Some(PgIdentity::Always),
        Identity::ByDefault => Some(PgIdentity::ByDefault),
        Identity::None | Identity::AutoIncrement => None,
    }
}

fn referential(value: ReferentialAction) -> PgReferentialAction {
    match value {
        ReferentialAction::NoAction => PgReferentialAction::NoAction,
        ReferentialAction::Restrict => PgReferentialAction::Restrict,
        ReferentialAction::Cascade => PgReferentialAction::Cascade,
        ReferentialAction::SetNull => PgReferentialAction::SetNull,
        ReferentialAction::SetDefault => PgReferentialAction::SetDefault,
    }
}

/// Every constraint and index name of a table (they share PostgreSQL's
/// relation and constraint namespaces closely enough to rename as one set).
fn object_names(table: &TableModel) -> Vec<String> {
    table
        .primary_key
        .iter()
        .chain(table.uniques.iter())
        .filter_map(|key| key.name.clone())
        .chain(table.foreign_keys.iter().filter_map(|key| key.name.clone()))
        .chain(table.checks.iter().filter_map(|check| check.name.clone()))
        .chain(table.indexes.iter().map(|index| index.name.clone()))
        .collect()
}

struct Emitter<'a> {
    baseline: &'a SchemaBaseline,
    draft: &'a SchemaDraft,
    options: &'a PlanOptions,
    major: u32,
    schema: String,
    /// The table's name at this point of the plan (it changes at the rename).
    table: String,
    atomic: Vec<PgObjectOp>,
    validate: Vec<PgObjectOp>,
    concurrent: Vec<PgObjectOp>,
    deferred: Vec<Origin>,
    /// Constraint names this plan derived, so two never collide.
    reserved: Vec<String>,
}

impl<'a> Emitter<'a> {
    fn before(&self) -> &'a TableModel {
        &self.baseline.table
    }

    fn after(&self) -> &'a TableModel {
        &self.draft.table
    }

    fn old_column(&self, id: BaselineId) -> Result<&'a ColumnModel, PlanError> {
        self.before()
            .column(Origin::Existing(id))
            .ok_or_else(|| internal("a planned column is not in the baseline"))
    }

    fn new_column(&self, origin: Origin) -> Result<&'a ColumnModel, PlanError> {
        self.after()
            .column(origin)
            .ok_or_else(|| internal("a planned column is not in the draft"))
    }

    fn new_names(&self, origins: &[Origin]) -> Result<Vec<String>, PlanError> {
        origins
            .iter()
            .map(|origin| self.new_column(*origin).map(|column| column.name.clone()))
            .collect()
    }

    fn table_ref(&self) -> PgObjectRef {
        PgObjectRef {
            kind: PgObjectKind::Table,
            schema: Some(self.schema.clone()),
            name: self.table.clone(),
            identity_args: None,
        }
    }

    fn push(&mut self, op: PgObjectOp) {
        self.atomic.push(op);
    }

    fn drop_constraint(&mut self, name: &Option<String>) -> Result<(), PlanError> {
        let name = name
            .clone()
            .ok_or_else(|| internal("a captured constraint has no name"))?;
        let op = PgObjectOp::DropConstraint(DropConstraintOp {
            schema: self.schema.clone(),
            table: self.table.clone(),
            name,
            cascade: false,
        });
        self.push(op);
        Ok(())
    }

    /// Names of baseline objects that phase 1 drops.
    fn dropped_names(&self, steps: &[PlanStep]) -> Vec<String> {
        let old = self.before();
        steps
            .iter()
            .filter_map(|step| match step {
                PlanStep::DropForeignKey { id } => old
                    .foreign_key(Origin::Existing(*id))
                    .and_then(|key| key.name.clone()),
                PlanStep::DropCheck { id } => old
                    .check(Origin::Existing(*id))
                    .and_then(|check| check.name.clone()),
                PlanStep::DropUnique { id } | PlanStep::DropPrimaryKey { id } => old
                    .key(Origin::Existing(*id))
                    .and_then(|key| key.name.clone()),
                PlanStep::DropIndex { id } => old
                    .index(Origin::Existing(*id))
                    .map(|index| index.name.clone()),
                _ => None,
            })
            .collect()
    }

    fn drop_step(&mut self, step: &PlanStep) -> Result<(), PlanError> {
        let old = self.before();
        match step {
            PlanStep::DropForeignKey { id } => {
                let key = old
                    .foreign_key(Origin::Existing(*id))
                    .ok_or_else(|| internal("a dropped foreign key is not in the baseline"))?;
                self.drop_constraint(&key.name)
            }
            PlanStep::DropCheck { id } => {
                let check = old
                    .check(Origin::Existing(*id))
                    .ok_or_else(|| internal("a dropped check is not in the baseline"))?;
                self.drop_constraint(&check.name)
            }
            PlanStep::DropUnique { id } | PlanStep::DropPrimaryKey { id } => {
                let key = old
                    .key(Origin::Existing(*id))
                    .ok_or_else(|| internal("a dropped key is not in the baseline"))?;
                self.drop_constraint(&key.name)
            }
            PlanStep::DropIndex { id } => {
                let index = old
                    .index(Origin::Existing(*id))
                    .ok_or_else(|| internal("a dropped index is not in the baseline"))?;
                let op = PgObjectOp::DropIndex(DropIndexOp {
                    schema: self.schema.clone(),
                    name: index.name.clone(),
                    concurrently: false,
                    cascade: false,
                });
                self.push(op);
                Ok(())
            }
            _ => Err(internal("a non-drop step in phase 1")),
        }
    }

    fn drop_column(&mut self, id: BaselineId) -> Result<(), PlanError> {
        let name = self.old_column(id)?.name.clone();
        let op = PgObjectOp::DropColumn(DropColumnOp {
            schema: self.schema.clone(),
            table: self.table.clone(),
            name,
            cascade: false,
        });
        self.push(op);
        Ok(())
    }

    /// Phase 2: constraint and index renames, column renames, then the
    /// table rename.
    fn renames(&mut self, steps: &[PlanStep], early: &[BaselineId]) -> Result<(), PlanError> {
        let old = self.before();
        let new = self.after();

        let dropped = self.dropped_names(steps);
        let current: Vec<String> = object_names(old)
            .into_iter()
            .filter(|name| !dropped.contains(name))
            .collect();
        let mut pending = Vec::new();
        for step in steps {
            let PlanStep::RenameConstraint { object, id } = step else {
                continue;
            };
            let origin = Origin::Existing(*id);
            let names = match object {
                SchemaObject::PrimaryKey | SchemaObject::Unique => old
                    .key(origin)
                    .map(|key| key.name.clone())
                    .zip(new.key(origin).map(|key| key.name.clone())),
                SchemaObject::ForeignKey => old
                    .foreign_key(origin)
                    .map(|key| key.name.clone())
                    .zip(new.foreign_key(origin).map(|key| key.name.clone())),
                SchemaObject::Check => old
                    .check(origin)
                    .map(|check| check.name.clone())
                    .zip(new.check(origin).map(|check| check.name.clone())),
                SchemaObject::Index => old
                    .index(origin)
                    .map(|index| Some(index.name.clone()))
                    .zip(new.index(origin).map(|index| Some(index.name.clone()))),
                SchemaObject::Table | SchemaObject::Column => None,
            };
            match names {
                Some((Some(from), Some(to))) => pending.push((*object, from, to)),
                _ => return Err(internal("a renamed constraint has no name")),
            }
        }
        for (object, from, to) in resolve_renames(current, pending) {
            let op = if object == SchemaObject::Index {
                PgObjectOp::RenameIndex(RenameIndexOp {
                    schema: self.schema.clone(),
                    name: from,
                    new_name: to,
                })
            } else {
                PgObjectOp::RenameConstraint(RenameConstraintOp {
                    schema: self.schema.clone(),
                    table: self.table.clone(),
                    name: from,
                    new_name: to,
                })
            };
            self.push(op);
        }

        let current: Vec<String> = old
            .columns
            .iter()
            .filter(|column| {
                column
                    .origin
                    .existing()
                    .is_none_or(|id| !early.contains(&id))
            })
            .map(|column| column.name.clone())
            .collect();
        let mut pending = Vec::new();
        for step in steps {
            if let PlanStep::RenameColumn { id } = step {
                let from = self.old_column(*id)?.name.clone();
                let to = self.new_column(Origin::Existing(*id))?.name.clone();
                pending.push(((), from, to));
            }
        }
        for ((), from, to) in resolve_renames(current, pending) {
            let op = PgObjectOp::RenameColumn(RenameColumnOp {
                schema: self.schema.clone(),
                table: self.table.clone(),
                name: from,
                new_name: to,
            });
            self.push(op);
        }

        if steps.contains(&PlanStep::RenameTable) {
            let op = PgObjectOp::RenameObject(RenameObjectOp {
                reference: self.table_ref(),
                new_name: new.name.clone(),
            });
            self.push(op);
            self.table = new.name.clone();
        }
        Ok(())
    }

    fn step(&mut self, step: &PlanStep, early: &[BaselineId]) -> Result<(), PlanError> {
        match step {
            PlanStep::AlterColumn { id, fields } => self.alter_column(*id, fields),
            PlanStep::AddColumn { origin } => self.add_column(*origin),
            PlanStep::DropColumn { id } if early.contains(id) => Ok(()),
            PlanStep::DropColumn { id } => self.drop_column(*id),
            PlanStep::AddPrimaryKey { origin } => {
                let key = self
                    .after()
                    .primary_key
                    .as_ref()
                    .filter(|key| key.origin == *origin)
                    .ok_or_else(|| internal("an added primary key is not in the draft"))?;
                let op = PgObjectOp::AddPrimaryKey(AddPrimaryKeyOp {
                    schema: self.schema.clone(),
                    table: self.table.clone(),
                    name: key.name.clone(),
                    columns: self.new_names(&key.columns)?,
                });
                self.push(op);
                Ok(())
            }
            PlanStep::AddUnique { origin } => {
                let key = self
                    .after()
                    .uniques
                    .iter()
                    .find(|key| key.origin == *origin)
                    .ok_or_else(|| internal("an added unique key is not in the draft"))?;
                let op = PgObjectOp::AddUnique(AddUniqueOp {
                    schema: self.schema.clone(),
                    table: self.table.clone(),
                    name: key.name.clone(),
                    columns: self.new_names(&key.columns)?,
                });
                self.push(op);
                Ok(())
            }
            PlanStep::AddCheck { origin } => self.add_check(*origin),
            PlanStep::CreateIndex { origin } => self.create_index(*origin),
            PlanStep::AddForeignKey { origin } => self.add_foreign_key(*origin),
            PlanStep::SetTableComment => {
                let op = PgObjectOp::SetComment(SetCommentOp {
                    target: PgCommentTarget::Object {
                        reference: self.table_ref(),
                    },
                    comment: self.after().comment.clone(),
                });
                self.push(op);
                Ok(())
            }
            PlanStep::SetColumnComment { origin } => {
                let column = self.new_column(*origin)?;
                let op = PgObjectOp::SetComment(SetCommentOp {
                    target: PgCommentTarget::Column {
                        schema: self.schema.clone(),
                        table: self.table.clone(),
                        column: column.name.clone(),
                    },
                    comment: column.comment.clone(),
                });
                self.push(op);
                Ok(())
            }
            PlanStep::ReorderColumns => {
                Err(unsupported("Column reorder is not supported on PostgreSQL"))
            }
            PlanStep::SetTableOptions { .. } => {
                Err(unsupported("PostgreSQL has no editable table options"))
            }
            _ => Err(internal("a phase 1 or 2 step after the renames")),
        }
    }

    fn add_column(&mut self, origin: Origin) -> Result<(), PlanError> {
        let column = self.new_column(origin)?;
        let op = PgObjectOp::AddColumn(AddColumnOp {
            schema: self.schema.clone(),
            table: self.table.clone(),
            column: NewColumnSpec {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
                nullable: column.nullable,
                default: default_value(&column.default),
                identity: identity(column.identity),
                generated: column
                    .generated
                    .as_ref()
                    .map(|generated| PgGeneratedColumn {
                        expression: generated.expression.as_str().to_owned(),
                        stored: generated.stored,
                    }),
                collation: column.collation.clone(),
            },
        });
        self.push(op);
        Ok(())
    }

    /// One existing column's alterations, ordered so each statement is valid
    /// after the previous one: generation and identity are removed first, a
    /// replaced default is dropped before a type change, identity and
    /// generation are added after NOT NULL, and a new default comes last.
    fn alter_column(&mut self, id: BaselineId, fields: &[ColumnField]) -> Result<(), PlanError> {
        let old = self.old_column(id)?;
        let new = self.new_column(Origin::Existing(id))?;
        if fields.iter().any(|field| {
            matches!(
                field,
                ColumnField::Charset | ColumnField::OnUpdate | ColumnField::Invisible
            )
        }) {
            return Err(unsupported(format!(
                "Column {} changes MySQL-only attributes",
                new.name
            )));
        }
        let schema = self.schema.clone();
        let table = self.table.clone();
        let name = new.name.clone();
        let retype = fields
            .iter()
            .any(|field| matches!(field, ColumnField::DataType | ColumnField::Collation));
        let default_changed = old.default != new.default;
        let mut ops = Vec::new();

        match (&old.generated, &new.generated) {
            (None, Some(_)) => {
                return Err(unsupported(format!(
                    "PostgreSQL cannot turn the existing column {name} into a generated column; add a new generated column instead"
                )))
            }
            (Some(before), Some(after)) if before.stored != after.stored => {
                return Err(unsupported(format!(
                    "Column {name} cannot switch between stored and virtual generation"
                )))
            }
            (Some(before), None) => {
                if !before.stored {
                    return Err(unsupported(format!(
                        "Column {name} is a virtual generated column; PostgreSQL cannot keep its values as a plain column"
                    )));
                }
                if self.major < 13 {
                    return Err(unsupported(
                        "Making a generated column plain needs PostgreSQL 13 or later",
                    ));
                }
                ops.push(PgObjectOp::SetColumnExpression(SetColumnExpressionOp {
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    expression: None,
                }));
            }
            _ => {}
        }

        if identity(old.identity).is_some() && identity(new.identity).is_none() {
            ops.push(PgObjectOp::SetColumnIdentity(SetColumnIdentityOp {
                schema: schema.clone(),
                table: table.clone(),
                name: name.clone(),
                change: PgIdentityChange::Drop,
            }));
        }

        if !old.default.is_none() && default_changed && (new.default.is_none() || retype) {
            ops.push(PgObjectOp::SetColumnDefault(SetColumnDefaultOp {
                schema: schema.clone(),
                table: table.clone(),
                name: name.clone(),
                default: None,
            }));
        }

        if retype {
            // A type change without COLLATE resets the collation, so a kept
            // collation is restated.
            ops.push(match &new.collation {
                Some(collation) => PgObjectOp::AlterColumnCollation(AlterColumnCollationOp {
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    data_type: new.data_type.clone(),
                    collation: collation.clone(),
                    using: None,
                }),
                None => PgObjectOp::AlterColumnType(AlterColumnTypeOp {
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    new_type: new.data_type.clone(),
                    using: None,
                }),
            });
        }

        if old.nullable && !new.nullable {
            ops.push(PgObjectOp::SetColumnNullable(SetColumnNullableOp {
                schema: schema.clone(),
                table: table.clone(),
                name: name.clone(),
                nullable: false,
            }));
        }

        match (identity(old.identity), identity(new.identity)) {
            (None, Some(after)) => ops.push(PgObjectOp::SetColumnIdentity(SetColumnIdentityOp {
                schema: schema.clone(),
                table: table.clone(),
                name: name.clone(),
                change: PgIdentityChange::Add { identity: after },
            })),
            (Some(before), Some(after)) if before != after => {
                ops.push(PgObjectOp::SetColumnIdentity(SetColumnIdentityOp {
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    change: PgIdentityChange::Set { identity: after },
                }))
            }
            _ => {}
        }

        if let (Some(before), Some(after)) = (&old.generated, &new.generated) {
            if before.expression != after.expression {
                if self.major < 17 {
                    return Err(unsupported(format!(
                        "Changing the generated expression of {name} needs PostgreSQL 17 or later"
                    )));
                }
                ops.push(PgObjectOp::SetColumnExpression(SetColumnExpressionOp {
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    expression: Some(after.expression.as_str().to_owned()),
                }));
            }
        }

        if default_changed {
            if let Some(default) = default_value(&new.default) {
                ops.push(PgObjectOp::SetColumnDefault(SetColumnDefaultOp {
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    default: Some(default),
                }));
            }
        }

        if !old.nullable && new.nullable {
            ops.push(PgObjectOp::SetColumnNullable(SetColumnNullableOp {
                schema,
                table,
                name,
                nullable: true,
            }));
        }

        self.atomic.extend(ops);
        Ok(())
    }

    fn taken(&self, name: &str) -> bool {
        self.reserved.iter().any(|other| other == name)
            || object_names(self.before())
                .iter()
                .any(|other| other == name)
            || object_names(self.after()).iter().any(|other| other == name)
    }

    /// A constraint name for an unnamed foreign key or check that must be
    /// validated by name later: `{table}_{columns}_{suffix}`, numbered on
    /// collision.
    fn derived_name(&mut self, columns: &[String], suffix: &str) -> String {
        let mut parts = vec![self.after().name.clone()];
        parts.extend(columns.iter().cloned());
        parts.push(suffix.to_owned());
        let raw = parts.join("_");
        let mut attempt = 0u32;
        loop {
            let candidate = if attempt == 0 {
                truncate_identifier(&raw, MAX_IDENTIFIER_BYTES).to_owned()
            } else {
                let number = attempt.to_string();
                format!(
                    "{}{number}",
                    truncate_identifier(&raw, MAX_IDENTIFIER_BYTES - number.len())
                )
            };
            if !self.taken(&candidate) {
                self.reserved.push(candidate.clone());
                return candidate;
            }
            attempt += 1;
        }
    }

    fn validate_later(&mut self, name: String) {
        self.validate
            .push(PgObjectOp::ValidateConstraint(ValidateConstraintOp {
                schema: self.schema.clone(),
                table: self.table.clone(),
                name,
            }));
    }

    fn add_check(&mut self, origin: Origin) -> Result<(), PlanError> {
        let check = self
            .after()
            .check(origin)
            .ok_or_else(|| internal("an added check is not in the draft"))?;
        if !check.enforced {
            return Err(unsupported(
                "NOT ENFORCED checks are not supported on PostgreSQL",
            ));
        }
        let later = self.options.validate_separately;
        let name = match (&check.name, later) {
            (Some(name), _) => Some(name.clone()),
            (None, true) => Some(self.derived_name(&[], "check")),
            (None, false) => None,
        };
        let op = PgObjectOp::AddCheck(AddCheckOp {
            schema: self.schema.clone(),
            table: self.table.clone(),
            name: name.clone(),
            expression: check.expression.as_str().to_owned(),
            not_valid: later,
        });
        self.push(op);
        if let (true, Some(name)) = (later, name) {
            self.validate_later(name);
        }
        Ok(())
    }

    fn add_foreign_key(&mut self, origin: Origin) -> Result<(), PlanError> {
        let key = self
            .after()
            .foreign_key(origin)
            .ok_or_else(|| internal("an added foreign key is not in the draft"))?;
        let columns = self.new_names(&key.columns)?;
        let later = self.options.validate_separately;
        let name = match (&key.name, later) {
            (Some(name), _) => Some(name.clone()),
            (None, true) => Some(self.derived_name(&columns, "fkey")),
            (None, false) => None,
        };
        let (deferrable, initially_deferred) = match key.deferrable {
            Deferrable::Never => (false, false),
            Deferrable::InitiallyImmediate => (true, false),
            Deferrable::InitiallyDeferred => (true, true),
        };
        let op = PgObjectOp::AddForeignKey(AddForeignKeyOp {
            schema: self.schema.clone(),
            table: self.table.clone(),
            name: name.clone(),
            columns,
            referenced_schema: key
                .referenced_schema
                .clone()
                .unwrap_or_else(|| self.schema.clone()),
            referenced_table: key.referenced_table.clone(),
            referenced_columns: key.referenced_columns.clone(),
            on_update: referential(key.on_update),
            on_delete: referential(key.on_delete),
            deferrable,
            initially_deferred,
            not_valid: later,
        });
        self.push(op);
        if let (true, Some(name)) = (later, name) {
            self.validate_later(name);
        }
        Ok(())
    }

    fn create_index(&mut self, origin: Origin) -> Result<(), PlanError> {
        let index = self
            .after()
            .index(origin)
            .ok_or_else(|| internal("a created index is not in the draft"))?;
        let mut columns = Vec::with_capacity(index.members.len());
        for member in &index.members {
            if member.prefix_length.is_some() || member.collation.is_some() {
                return Err(unsupported(format!(
                    "Index {} uses a key prefix or per-column collation, which the editor does not support on PostgreSQL",
                    index.name
                )));
            }
            let expression = match &member.target {
                IndexTarget::Column(origin) => quote_double(&self.new_column(*origin)?.name),
                IndexTarget::Expression(sql) => sql.as_str().to_owned(),
            };
            columns.push(PgIndexColumn {
                expression,
                descending: member.descending,
            });
        }
        let concurrently = self.options.concurrent_indexes.contains(&origin);
        let op = PgObjectOp::CreateIndex(CreateIndexOp {
            schema: self.schema.clone(),
            table: self.table.clone(),
            name: Some(index.name.clone()),
            unique: index.unique,
            method: index.method.clone().unwrap_or_else(|| "btree".into()),
            columns,
            include: self.new_names(&index.include)?,
            where_predicate: index.predicate.as_ref().map(|p| p.as_str().to_owned()),
            concurrently,
        });
        if concurrently {
            self.concurrent.push(op);
            self.deferred.push(origin);
        } else {
            self.push(op);
        }
        Ok(())
    }
}

fn type_parts(text: &str) -> (String, Vec<u64>) {
    let normalized = normalize_type(text);
    let (base, args) = match normalized.find('(') {
        Some(open) if normalized.ends_with(')') => {
            let args: Option<Vec<u64>> = normalized[open + 1..normalized.len() - 1]
                .split(',')
                .map(|arg| arg.trim().parse().ok())
                .collect();
            match args {
                Some(args) => (normalized[..open].trim().to_owned(), args),
                None => return (normalized, Vec::new()),
            }
        }
        _ => (normalized, Vec::new()),
    };
    let base = match base.as_str() {
        "character varying" | "varchar" => "varchar".to_owned(),
        "decimal" | "numeric" => "numeric".to_owned(),
        _ => base,
    };
    (base, args)
}

/// Type changes PostgreSQL makes without rewriting the table: a longer or
/// unbounded `varchar`, `varchar` ↔ `text`, and a wider `numeric` with the
/// same scale.
pub(crate) fn binary_coercible(before: &str, after: &str) -> bool {
    let (b, b_args) = type_parts(before);
    let (a, a_args) = type_parts(after);
    match (b.as_str(), b_args.as_slice(), a.as_str(), a_args.as_slice()) {
        _ if normalize_type(before) == normalize_type(after) => true,
        ("varchar", [n], "varchar", [m]) => m >= n,
        ("varchar", _, "varchar" | "text", []) => true,
        ("text", [], "varchar", []) => true,
        ("numeric", [p, s], "numeric", [q, t]) => q >= p && s == t,
        ("numeric", [p], "numeric", [q]) => q >= p,
        ("numeric", [p], "numeric", [q, 0]) => q >= p,
        ("numeric", [_] | [_, _], "numeric", []) => true,
        _ => false,
    }
}

fn notes(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    steps: &[PlanStep],
    built: &Built,
    options: &PlanOptions,
) -> Vec<EngineNote> {
    let old = &baseline.table;
    let new = &draft.table;
    let display = format!("{}.{}", new.schema, new.name);
    let mut notes = Vec::new();

    if built.atomic_len() > 0 {
        notes.push(EngineNote::new(
            NoteKind::Lock,
            format!(
                "Takes an ACCESS EXCLUSIVE lock on {display} until the change commits, blocking reads and writes; waits at most 10 s for the lock (lock_timeout)"
            ),
        ));
    }

    let mut rewrites = false;
    let mut column_drops_or_retypes = false;
    for step in steps {
        match step {
            PlanStep::AlterColumn { id, fields } => {
                let origin = Origin::Existing(*id);
                let (Some(before), Some(after)) = (old.column(origin), new.column(origin)) else {
                    continue;
                };
                if fields.contains(&ColumnField::DataType) {
                    column_drops_or_retypes = true;
                    if binary_coercible(&before.data_type, &after.data_type) {
                        notes.push(EngineNote::new(
                            NoteKind::Info,
                            format!(
                                "Changing {} from {} to {} needs no table rewrite",
                                after.name, before.data_type, after.data_type
                            ),
                        ));
                    } else {
                        rewrites = true;
                        notes.push(EngineNote::new(
                            NoteKind::Rewrite,
                            format!(
                                "Changing {} from {} to {} rewrites the table and rebuilds its indexes",
                                after.name, before.data_type, after.data_type
                            ),
                        ));
                    }
                } else if fields.contains(&ColumnField::Collation) {
                    notes.push(EngineNote::new(
                        NoteKind::Info,
                        format!(
                            "Changing the collation of {} rebuilds the indexes that use it",
                            after.name
                        ),
                    ));
                }
                if fields.contains(&ColumnField::Generated) && after.generated.is_some() {
                    rewrites = true;
                    notes.push(EngineNote::new(
                        NoteKind::Rewrite,
                        format!("Recomputes {} for every row (table rewrite)", after.name),
                    ));
                }
                if fields.contains(&ColumnField::Nullable) && !after.nullable {
                    notes.push(EngineNote::new(
                        NoteKind::Info,
                        format!("Setting NOT NULL on {} scans the whole table", after.name),
                    ));
                }
            }
            PlanStep::AddColumn { origin } => {
                let Some(column) = new.column(*origin) else {
                    continue;
                };
                if column.identity != Identity::None
                    || column.generated.as_ref().is_some_and(|g| g.stored)
                {
                    rewrites = true;
                    notes.push(EngineNote::new(
                        NoteKind::Rewrite,
                        format!("Adding {} rewrites the table", column.name),
                    ));
                } else if matches!(column.default, DefaultValue::Expression(_)) {
                    notes.push(EngineNote::new(
                        NoteKind::Info,
                        format!(
                            "Adding {} with an expression default rewrites the table if the expression is volatile",
                            column.name
                        ),
                    ));
                }
            }
            PlanStep::DropColumn { .. } => column_drops_or_retypes = true,
            _ => {}
        }
    }

    let drops_keys = steps.iter().any(|step| {
        matches!(
            step,
            PlanStep::DropPrimaryKey { .. } | PlanStep::DropUnique { .. }
        )
    });
    if !baseline.inbound.is_empty() && (column_drops_or_retypes || drops_keys) {
        let mut names: Vec<String> = baseline
            .inbound
            .iter()
            .take(5)
            .map(|key| format!("{}.{}.{}", key.schema, key.table, key.name))
            .collect();
        if baseline.inbound.len() > 5 {
            names.push(format!("and {} more", baseline.inbound.len() - 5));
        }
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "Referenced by foreign keys {}; dropping or retyping referenced keys or columns fails while they exist",
                names.join(", ")
            ),
        ));
    }
    if !baseline.dependents.is_empty() && column_drops_or_retypes {
        notes.push(EngineNote::new(
            NoteKind::Dependents,
            format!(
                "{} triggers, rules or policies depend on {display}; dropping or retyping a column they use fails",
                baseline.dependents.len()
            ),
        ));
    }

    let adds_constraints = steps.iter().any(|step| {
        matches!(
            step,
            PlanStep::AddForeignKey { .. } | PlanStep::AddCheck { .. }
        )
    });
    let blocking_indexes = steps.iter().any(|step| {
        matches!(step, PlanStep::CreateIndex { origin } if !options.concurrent_indexes.contains(origin))
    });
    if let Some(rows) = baseline
        .estimated_rows
        .filter(|rows| *rows > LARGE_TABLE_ROWS)
    {
        let mut advice = Vec::new();
        if adds_constraints && !options.validate_separately {
            advice.push("add foreign keys and checks NOT VALID and validate them separately");
        }
        if blocking_indexes {
            advice.push("build new indexes CONCURRENTLY");
        }
        if rewrites {
            advice.push("expect the rewrite to hold the lock for the whole copy");
        }
        let mut text = format!("{display} has about {rows} rows; the change may take a while");
        if !advice.is_empty() {
            text.push_str(": ");
            text.push_str(&advice.join("; "));
        }
        notes.push(EngineNote::new(NoteKind::LargeTable, text));
    }

    if built.roles.contains(&Role::Validate) {
        notes.push(EngineNote::new(
            NoteKind::Info,
            "New foreign keys and checks are added NOT VALID and validated after the change commits",
        ));
    }
    if built.atomic_len() < built.statements.len() {
        notes.push(EngineNote::new(
            NoteKind::NonAtomic,
            "Statements after the main change commit one at a time; if one fails, the earlier ones stay applied",
        ));
    }
    notes
}
