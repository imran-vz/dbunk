use serde::{Deserialize, Serialize};

use super::objects::{PgObjectKind, PgObjectRef};
use super::sql_class::{classify_script, StatementClass};
use super::sql_lex::{lex_sql, SqlToken};
use crate::{quote_double, quote_literal};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgDefaultValue {
    Literal { value: String },
    Expression { sql: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewColumnSpec {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<PgDefaultValue>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl PgReferentialAction {
    fn sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgIndexColumn {
    pub expression: String,
    pub descending: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgEnumPosition {
    Before { neighbor: String },
    After { neighbor: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgCommentTarget {
    Object {
        reference: PgObjectRef,
    },
    Column {
        schema: String,
        table: String,
        column: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum PgObjectOp {
    CreateSchema {
        name: String,
    },
    RenameObject {
        reference: PgObjectRef,
        new_name: String,
    },
    DropObject {
        reference: PgObjectRef,
        cascade: bool,
    },
    SetComment {
        target: PgCommentTarget,
        comment: Option<String>,
    },
    AddColumn {
        schema: String,
        table: String,
        column: NewColumnSpec,
    },
    DropColumn {
        schema: String,
        table: String,
        name: String,
        cascade: bool,
    },
    RenameColumn {
        schema: String,
        table: String,
        name: String,
        new_name: String,
    },
    AlterColumnType {
        schema: String,
        table: String,
        name: String,
        new_type: String,
        using: Option<String>,
    },
    SetColumnNullable {
        schema: String,
        table: String,
        name: String,
        nullable: bool,
    },
    SetColumnDefault {
        schema: String,
        table: String,
        name: String,
        default: Option<PgDefaultValue>,
    },
    AddPrimaryKey {
        schema: String,
        table: String,
        name: Option<String>,
        columns: Vec<String>,
    },
    AddUnique {
        schema: String,
        table: String,
        name: Option<String>,
        columns: Vec<String>,
    },
    AddForeignKey {
        schema: String,
        table: String,
        name: Option<String>,
        columns: Vec<String>,
        referenced_schema: String,
        referenced_table: String,
        referenced_columns: Vec<String>,
        on_update: PgReferentialAction,
        on_delete: PgReferentialAction,
        deferrable: bool,
        initially_deferred: bool,
        not_valid: bool,
    },
    AddCheck {
        schema: String,
        table: String,
        name: Option<String>,
        expression: String,
        not_valid: bool,
    },
    DropConstraint {
        schema: String,
        table: String,
        name: String,
        cascade: bool,
    },
    CreateIndex {
        schema: String,
        table: String,
        name: Option<String>,
        unique: bool,
        method: String,
        columns: Vec<PgIndexColumn>,
        include: Vec<String>,
        where_predicate: Option<String>,
        concurrently: bool,
    },
    DropIndex {
        schema: String,
        name: String,
        concurrently: bool,
        cascade: bool,
    },
    CreateView {
        schema: String,
        name: String,
        or_replace: bool,
        sql_body: String,
    },
    CreateMaterializedView {
        schema: String,
        name: String,
        sql_body: String,
        with_data: bool,
    },
    CreateSequence {
        schema: String,
        name: String,
        data_type: Option<String>,
        start: Option<String>,
        increment: Option<String>,
        min_value: Option<String>,
        max_value: Option<String>,
        cycle: Option<bool>,
        cache: Option<String>,
    },
    AlterSequence {
        schema: String,
        name: String,
        restart_with: Option<String>,
        increment_by: Option<String>,
        min_value: Option<String>,
        max_value: Option<String>,
        cycle: Option<bool>,
        cache: Option<String>,
    },
    CreateEnum {
        schema: String,
        name: String,
        labels: Vec<String>,
    },
    AddEnumValue {
        schema: String,
        name: String,
        value: String,
        position: Option<PgEnumPosition>,
    },
    RenameEnumValue {
        schema: String,
        name: String,
        from: String,
        to: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlannedStatement {
    pub sql: String,
    pub summary: String,
    pub destructive: bool,
    pub transactional: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum StatementGroup {
    Atomic { statement_indexes: Vec<usize> },
    Standalone { statement_index: usize },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlPlanPreview {
    pub statements: Vec<PlannedStatement>,
    pub groups: Vec<StatementGroup>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlStatementSummary {
    pub index: usize,
    pub summary: String,
    pub destructive: bool,
    pub transactional: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlApplyResult {
    pub applied_statements: usize,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum DdlResidue {
    InvalidIndex { schema: String, name: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgObjectError {
    UnsupportedEngine {
        engine: String,
    },
    ObjectNotFound {
        reference: PgObjectRef,
    },
    InvalidOp {
        op_index: usize,
        reason: String,
    },
    PolicyBlocked {
        reason: String,
    },
    PolicyNeedsConfirmation {
        statements: Vec<DdlStatementSummary>,
    },
    Connection {
        message: String,
    },
    LockTimeout {
        statement_index: usize,
        applied_statements: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        residue: Option<Box<DdlResidue>>,
    },
    Database {
        #[serde(skip_serializing_if = "Option::is_none")]
        statement_index: Option<usize>,
        code: Option<String>,
        message: String,
        position: Option<u32>,
        applied_statements: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        residue: Option<Box<DdlResidue>>,
    },
}

impl PgObjectError {
    /// Statements committed before the failure; zero for errors raised before
    /// any group executed.
    pub(crate) fn applied_statements(&self) -> usize {
        match self {
            Self::LockTimeout {
                applied_statements, ..
            }
            | Self::Database {
                applied_statements, ..
            } => *applied_statements,
            _ => 0,
        }
    }
}

struct RenderedOp {
    sql: String,
    summary: String,
    destructive: bool,
    transactional: bool,
}

pub(crate) fn generate_object_ddl(ops: &[PgObjectOp]) -> Result<DdlPlanPreview, PgObjectError> {
    let mut statements = Vec::with_capacity(ops.len());
    for (op_index, op) in ops.iter().enumerate() {
        validate_op(op_index, op)?;
        let mut rendered = render_op(op_index, op)?;
        rendered.destructive |= fragments(op).into_iter().any(fragment_is_destructive);

        let classes = classify_script(&rendered.sql);
        if classes.len() != 1 {
            return invalid(op_index, "fragment contains a statement boundary");
        }
        if !matches!(classes[0], StatementClass::Ddl { .. }) {
            return invalid(op_index, "generated statement is not DDL");
        }
        if matches!(classes[0], StatementClass::Ddl { destructive: true }) {
            rendered.destructive = true;
        }

        statements.push(PlannedStatement {
            sql: rendered.sql,
            summary: rendered.summary,
            destructive: rendered.destructive,
            transactional: rendered.transactional,
        });
    }

    Ok(DdlPlanPreview {
        groups: group_statements(&statements),
        statements,
    })
}

pub(crate) fn statement_summaries(preview: &DdlPlanPreview) -> Vec<DdlStatementSummary> {
    preview
        .statements
        .iter()
        .enumerate()
        .map(|(index, statement)| DdlStatementSummary {
            index,
            summary: statement.summary.clone(),
            destructive: statement.destructive,
            transactional: statement.transactional,
        })
        .collect()
}

fn group_statements(statements: &[PlannedStatement]) -> Vec<StatementGroup> {
    let mut groups = Vec::new();
    let mut atomic = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        if statement.transactional {
            atomic.push(index);
            continue;
        }
        if !atomic.is_empty() {
            groups.push(StatementGroup::Atomic {
                statement_indexes: std::mem::take(&mut atomic),
            });
        }
        groups.push(StatementGroup::Standalone {
            statement_index: index,
        });
    }
    if !atomic.is_empty() {
        groups.push(StatementGroup::Atomic {
            statement_indexes: atomic,
        });
    }
    groups
}

fn invalid<T>(op_index: usize, reason: impl Into<String>) -> Result<T, PgObjectError> {
    Err(PgObjectError::InvalidOp {
        op_index,
        reason: reason.into(),
    })
}

fn require_name(op_index: usize, label: &str, value: &str) -> Result<(), PgObjectError> {
    if value.trim().is_empty() {
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    Ok(())
}

fn require_names(op_index: usize, label: &str, values: &[String]) -> Result<(), PgObjectError> {
    if values.is_empty() {
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    for value in values {
        require_name(op_index, label, value)?;
    }
    Ok(())
}

fn validate_reference(op_index: usize, reference: &PgObjectRef) -> Result<(), PgObjectError> {
    require_name(op_index, "object name", &reference.name)?;
    match reference.kind {
        PgObjectKind::Schema => {
            if reference.schema.is_some() {
                return invalid(op_index, "schema references cannot carry a parent schema");
            }
        }
        _ => {
            let Some(schema) = &reference.schema else {
                return invalid(op_index, "object schema is required");
            };
            require_name(op_index, "object schema", schema)?;
        }
    }
    match reference.kind {
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => {
            let Some(identity_args) = &reference.identity_args else {
                return invalid(op_index, "routine identity arguments are required");
            };
            validate_fragment(
                op_index,
                "routine identity arguments",
                identity_args,
                FragmentContext::IdentityArguments,
            )?;
        }
        _ if reference.identity_args.is_some() => {
            return invalid(
                op_index,
                "identity arguments are valid only for routines and aggregates",
            );
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum FragmentContext {
    Embedded,
    IdentityArguments,
    StatementBody,
}

fn validate_data_type(op_index: usize, label: &str, data_type: &str) -> Result<(), PgObjectError> {
    validate_fragment(op_index, label, data_type, FragmentContext::Embedded)?;
    let tokens = lex_sql(data_type).map_err(|_| PgObjectError::InvalidOp {
        op_index,
        reason: format!("{label} is not a valid data type"),
    })?;
    let mut depth = 0usize;
    let mut previous_was_dot = false;
    for token in tokens {
        match token {
            SqlToken::Symbol('(') => {
                depth += 1;
                previous_was_dot = false;
            }
            SqlToken::Symbol(')') => {
                depth = depth.saturating_sub(1);
                previous_was_dot = false;
            }
            SqlToken::Symbol('.') => previous_was_dot = true,
            SqlToken::Identifier(identifier) => {
                if depth == 0
                    && !previous_was_dot
                    && !identifier.quoted
                    && matches!(
                        identifier.value.to_ascii_lowercase().as_str(),
                        "not"
                            | "null"
                            | "default"
                            | "check"
                            | "constraint"
                            | "unique"
                            | "primary"
                            | "references"
                            | "generated"
                            | "identity"
                            | "collate"
                            | "storage"
                            | "compression"
                            | "options"
                            | "encoding"
                            | "using"
                    )
                {
                    return invalid(op_index, format!("{label} contains a column option"));
                }
                previous_was_dot = false;
            }
            _ => previous_was_dot = false,
        }
    }
    Ok(())
}

fn validate_sequence_data_type(op_index: usize, data_type: &str) -> Result<(), PgObjectError> {
    validate_data_type(op_index, "sequence data type", data_type)?;
    let tokens = lex_sql(data_type).map_err(|_| PgObjectError::InvalidOp {
        op_index,
        reason: "sequence data type must be smallint, integer, or bigint".into(),
    })?;
    let identifiers = match tokens.as_slice() {
        [SqlToken::Identifier(data_type)] => (None, data_type),
        [SqlToken::Identifier(schema), SqlToken::Symbol('.'), SqlToken::Identifier(data_type)] => {
            (Some(schema), data_type)
        }
        _ => {
            return invalid(
                op_index,
                "sequence data type must be smallint, integer, or bigint",
            );
        }
    };
    let matches_identifier = |identifier: &super::sql_lex::SqlIdentifier, expected: &str| {
        if identifier.quoted {
            identifier.value == expected
        } else {
            identifier.value.eq_ignore_ascii_case(expected)
        }
    };
    if identifiers
        .0
        .is_some_and(|schema| !matches_identifier(schema, "pg_catalog"))
        || ![
            "smallint", "int2", "integer", "int", "int4", "bigint", "int8",
        ]
        .iter()
        .any(|name| matches_identifier(identifiers.1, name))
    {
        return invalid(
            op_index,
            "sequence data type must be smallint, integer, or bigint",
        );
    }
    Ok(())
}

fn validate_i64_string(
    op_index: usize,
    label: &str,
    value: &Option<String>,
) -> Result<(), PgObjectError> {
    if value
        .as_deref()
        .is_some_and(|value| value.parse::<i64>().is_err())
    {
        return invalid(op_index, format!("{label} must be a signed 64-bit integer"));
    }
    Ok(())
}

fn validate_fragment(
    op_index: usize,
    label: &str,
    fragment: &str,
    context: FragmentContext,
) -> Result<(), PgObjectError> {
    let fragment = if matches!(context, FragmentContext::StatementBody) {
        trim_fragment_terminator(fragment)
    } else {
        fragment
    };
    if fragment.trim().is_empty() {
        if matches!(context, FragmentContext::IdentityArguments) {
            return Ok(());
        }
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    if classify_script(fragment).len() != 1 {
        return invalid(op_index, "fragment contains a statement boundary");
    }

    // Appending a sentinel catches a trailing line comment that would swallow
    // the renderer-owned suffix, while still permitting comments that end
    // before the fragment boundary. The same token stream then drives the
    // shape check, so fragment validation and statement classification share
    // one lexer and cannot disagree about where a string or identifier ends.
    let sentinel = "__dbunk_fragment_boundary__";
    let sentinel_sql = format!("{fragment} {sentinel}");
    let mut tokens = lex_sql(&sentinel_sql).map_err(|()| PgObjectError::InvalidOp {
        op_index,
        reason: "fragment escapes its typed SQL context".into(),
    })?;
    let sentinel_visible = matches!(
        tokens.pop(),
        Some(SqlToken::Identifier(identifier))
            if !identifier.quoted && identifier.value == sentinel
    );
    if !sentinel_visible {
        return invalid(op_index, "fragment escapes its typed SQL context");
    }

    let shape = fragment_shape(&tokens).ok_or_else(|| PgObjectError::InvalidOp {
        op_index,
        reason: "fragment escapes its typed SQL context".into(),
    })?;
    if shape.top_level_comma
        && !matches!(
            context,
            FragmentContext::IdentityArguments | FragmentContext::StatementBody
        )
    {
        return invalid(op_index, "fragment escapes its typed SQL context");
    }
    if shape.semicolons > 0
        && (!matches!(context, FragmentContext::StatementBody) || shape.semicolons > 1)
    {
        return invalid(op_index, "fragment contains a statement boundary");
    }
    Ok(())
}

struct FragmentShape {
    top_level_comma: bool,
    semicolons: usize,
}

/// Inspects only lexical structure. PostgreSQL still parses the expression,
/// but this pass makes sure an embedded fragment cannot close renderer-owned
/// delimiters or add a sibling comma-separated ALTER action.
fn fragment_shape(tokens: &[SqlToken]) -> Option<FragmentShape> {
    let mut delimiters = Vec::new();
    let mut top_level_comma = false;
    let mut semicolons = 0usize;
    for token in tokens {
        match token {
            SqlToken::Symbol(open @ ('(' | '[')) => delimiters.push(*open),
            SqlToken::Symbol(')') => {
                if delimiters.pop() != Some('(') {
                    return None;
                }
            }
            SqlToken::Symbol(']') => {
                if delimiters.pop() != Some('[') {
                    return None;
                }
            }
            SqlToken::Symbol(',') => top_level_comma |= delimiters.is_empty(),
            SqlToken::Symbol(';') => semicolons += 1,
            _ => {}
        }
    }
    if !delimiters.is_empty() {
        return None;
    }
    Some(FragmentShape {
        top_level_comma,
        semicolons,
    })
}

fn validate_op(op_index: usize, op: &PgObjectOp) -> Result<(), PgObjectError> {
    match op {
        PgObjectOp::CreateSchema { name } => require_name(op_index, "schema name", name),
        PgObjectOp::RenameObject {
            reference,
            new_name,
        } => {
            validate_reference(op_index, reference)?;
            require_name(op_index, "new name", new_name)?;
            if !matches!(
                reference.kind,
                PgObjectKind::Schema
                    | PgObjectKind::Table
                    | PgObjectKind::View
                    | PgObjectKind::MaterializedView
                    | PgObjectKind::Sequence
            ) {
                return invalid(op_index, "this object kind cannot be renamed");
            }
            Ok(())
        }
        PgObjectOp::DropObject { reference, .. } => {
            validate_reference(op_index, reference)?;
            if reference.kind == PgObjectKind::Extension {
                return invalid(op_index, "dropping extensions is not supported");
            }
            Ok(())
        }
        PgObjectOp::SetComment { target, .. } => match target {
            PgCommentTarget::Object { reference } => validate_reference(op_index, reference),
            PgCommentTarget::Column {
                schema,
                table,
                column,
            } => {
                require_name(op_index, "schema", schema)?;
                require_name(op_index, "table", table)?;
                require_name(op_index, "column", column)
            }
        },
        PgObjectOp::AddColumn {
            schema,
            table,
            column,
        } => {
            require_table(op_index, schema, table)?;
            require_name(op_index, "column", &column.name)?;
            validate_data_type(op_index, "column data type", &column.data_type)?;
            if let Some(PgDefaultValue::Expression { sql }) = &column.default {
                validate_fragment(
                    op_index,
                    "default expression",
                    sql,
                    FragmentContext::Embedded,
                )?;
            }
            Ok(())
        }
        PgObjectOp::DropColumn {
            schema,
            table,
            name,
            ..
        }
        | PgObjectOp::SetColumnNullable {
            schema,
            table,
            name,
            ..
        }
        | PgObjectOp::DropConstraint {
            schema,
            table,
            name,
            ..
        } => {
            require_table(op_index, schema, table)?;
            require_name(op_index, "name", name)
        }
        PgObjectOp::RenameColumn {
            schema,
            table,
            name,
            new_name,
        } => {
            require_table(op_index, schema, table)?;
            require_name(op_index, "column", name)?;
            require_name(op_index, "new column name", new_name)
        }
        PgObjectOp::AlterColumnType {
            schema,
            table,
            name,
            new_type,
            using,
        } => {
            require_table(op_index, schema, table)?;
            require_name(op_index, "column", name)?;
            validate_data_type(op_index, "new type", new_type)?;
            if let Some(using) = using {
                validate_fragment(
                    op_index,
                    "USING expression",
                    using,
                    FragmentContext::Embedded,
                )?;
            }
            Ok(())
        }
        PgObjectOp::SetColumnDefault {
            schema,
            table,
            name,
            default,
        } => {
            require_table(op_index, schema, table)?;
            require_name(op_index, "column", name)?;
            if let Some(PgDefaultValue::Expression { sql }) = default {
                validate_fragment(
                    op_index,
                    "default expression",
                    sql,
                    FragmentContext::Embedded,
                )?;
            }
            Ok(())
        }
        PgObjectOp::AddPrimaryKey {
            schema,
            table,
            name,
            columns,
        }
        | PgObjectOp::AddUnique {
            schema,
            table,
            name,
            columns,
        } => {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "constraint", name)?;
            require_names(op_index, "constraint columns", columns)
        }
        PgObjectOp::AddForeignKey {
            schema,
            table,
            name,
            columns,
            referenced_schema,
            referenced_table,
            referenced_columns,
            deferrable,
            initially_deferred,
            ..
        } => {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "constraint", name)?;
            require_names(op_index, "foreign-key columns", columns)?;
            require_table(op_index, referenced_schema, referenced_table)?;
            require_names(op_index, "referenced columns", referenced_columns)?;
            if columns.len() != referenced_columns.len() {
                return invalid(
                    op_index,
                    "foreign-key and referenced column counts must match",
                );
            }
            if *initially_deferred && !*deferrable {
                return invalid(
                    op_index,
                    "initially deferred requires a deferrable constraint",
                );
            }
            Ok(())
        }
        PgObjectOp::AddCheck {
            schema,
            table,
            name,
            expression,
            ..
        } => {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "constraint", name)?;
            validate_fragment(
                op_index,
                "check expression",
                expression,
                FragmentContext::Embedded,
            )
        }
        PgObjectOp::CreateIndex {
            schema,
            table,
            name,
            method,
            columns,
            include,
            where_predicate,
            ..
        } => {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "index", name)?;
            if name.as_ref().is_some_and(|name| name.len() > 63) {
                return invalid(op_index, "index name exceeds PostgreSQL's 63-byte limit");
            }
            require_name(op_index, "index method", method)?;
            if columns.is_empty() {
                return invalid(op_index, "index columns cannot be empty");
            }
            for column in columns {
                validate_fragment(
                    op_index,
                    "index expression",
                    &column.expression,
                    FragmentContext::Embedded,
                )?;
            }
            for column in include {
                require_name(op_index, "included column", column)?;
            }
            if let Some(predicate) = where_predicate {
                validate_fragment(
                    op_index,
                    "index predicate",
                    predicate,
                    FragmentContext::Embedded,
                )?;
            }
            Ok(())
        }
        PgObjectOp::DropIndex {
            schema,
            name,
            concurrently,
            cascade,
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "index", name)?;
            if *concurrently && *cascade {
                return invalid(op_index, "DROP INDEX CONCURRENTLY cannot use CASCADE");
            }
            Ok(())
        }
        PgObjectOp::CreateView {
            schema,
            name,
            sql_body,
            ..
        }
        | PgObjectOp::CreateMaterializedView {
            schema,
            name,
            sql_body,
            ..
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "view", name)?;
            validate_fragment(
                op_index,
                "view SQL body",
                sql_body,
                FragmentContext::StatementBody,
            )
        }
        PgObjectOp::CreateSequence {
            schema,
            name,
            data_type,
            start,
            increment,
            min_value,
            max_value,
            cache,
            ..
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "sequence", name)?;
            if let Some(data_type) = data_type {
                validate_sequence_data_type(op_index, data_type)?;
            }
            validate_i64_string(op_index, "sequence start", start)?;
            validate_i64_string(op_index, "sequence increment", increment)?;
            validate_i64_string(op_index, "sequence minimum", min_value)?;
            validate_i64_string(op_index, "sequence maximum", max_value)?;
            validate_i64_string(op_index, "sequence cache", cache)?;
            Ok(())
        }
        PgObjectOp::AlterSequence {
            schema,
            name,
            restart_with,
            increment_by,
            min_value,
            max_value,
            cache,
            ..
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "sequence", name)?;
            validate_i64_string(op_index, "sequence restart", restart_with)?;
            validate_i64_string(op_index, "sequence increment", increment_by)?;
            validate_i64_string(op_index, "sequence minimum", min_value)?;
            validate_i64_string(op_index, "sequence maximum", max_value)?;
            validate_i64_string(op_index, "sequence cache", cache)
        }
        PgObjectOp::CreateEnum {
            schema,
            name,
            labels,
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "enum", name)?;
            if labels.is_empty() {
                return invalid(op_index, "enum labels cannot be empty");
            }
            Ok(())
        }
        PgObjectOp::AddEnumValue {
            schema,
            name,
            position,
            ..
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "enum", name)?;
            if let Some(PgEnumPosition::Before { neighbor } | PgEnumPosition::After { neighbor }) =
                position
            {
                require_name(op_index, "neighbor enum label", neighbor)?;
            }
            Ok(())
        }
        PgObjectOp::RenameEnumValue {
            schema,
            name,
            from: _,
            to: _,
        } => {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "enum", name)
        }
    }
}

fn require_table(op_index: usize, schema: &str, table: &str) -> Result<(), PgObjectError> {
    require_name(op_index, "schema", schema)?;
    require_name(op_index, "table", table)
}

fn require_optional_name(
    op_index: usize,
    label: &str,
    value: &Option<String>,
) -> Result<(), PgObjectError> {
    if let Some(value) = value {
        require_name(op_index, label, value)?;
    }
    Ok(())
}

fn fragments(op: &PgObjectOp) -> Vec<&str> {
    match op {
        PgObjectOp::DropObject { reference, .. } | PgObjectOp::RenameObject { reference, .. } => {
            reference.identity_args.iter().map(String::as_str).collect()
        }
        PgObjectOp::SetComment { target, .. } => match target {
            PgCommentTarget::Object { reference } => {
                reference.identity_args.iter().map(String::as_str).collect()
            }
            PgCommentTarget::Column { .. } => Vec::new(),
        },
        PgObjectOp::AddColumn { column, .. } => {
            let mut values = vec![column.data_type.as_str()];
            if let Some(PgDefaultValue::Expression { sql }) = &column.default {
                values.push(sql);
            }
            values
        }
        PgObjectOp::AlterColumnType {
            new_type, using, ..
        } => std::iter::once(new_type.as_str())
            .chain(using.iter().map(String::as_str))
            .collect(),
        PgObjectOp::SetColumnDefault {
            default: Some(PgDefaultValue::Expression { sql }),
            ..
        } => vec![sql],
        PgObjectOp::AddCheck { expression, .. } => vec![expression],
        PgObjectOp::CreateIndex {
            columns,
            where_predicate,
            ..
        } => columns
            .iter()
            .map(|column| column.expression.as_str())
            .chain(where_predicate.iter().map(String::as_str))
            .collect(),
        PgObjectOp::CreateView { sql_body, .. }
        | PgObjectOp::CreateMaterializedView { sql_body, .. } => vec![sql_body],
        PgObjectOp::CreateSequence {
            data_type: Some(data_type),
            ..
        } => vec![data_type],
        _ => Vec::new(),
    }
}

fn fragment_is_destructive(fragment: &str) -> bool {
    classify_script(fragment).iter().any(|class| {
        matches!(
            class,
            StatementClass::Ddl { destructive: true }
                | StatementClass::Dml {
                    destructive: true,
                    ..
                }
        )
    })
}

fn render_op(op_index: usize, op: &PgObjectOp) -> Result<RenderedOp, PgObjectError> {
    let rendered = match op {
        PgObjectOp::CreateSchema { name } => RenderedOp {
            sql: format!("CREATE SCHEMA {};", quote_double(name)),
            summary: format!("Create schema {name}"),
            destructive: false,
            transactional: true,
        },
        PgObjectOp::RenameObject {
            reference,
            new_name,
        } => {
            let keyword = object_keyword(reference.kind);
            RenderedOp {
                sql: format!(
                    "ALTER {keyword} {} RENAME TO {};",
                    render_object_identity(reference),
                    quote_double(new_name)
                ),
                summary: format!(
                    "Rename {} {} to {new_name}",
                    object_label(reference.kind),
                    display_identity(reference)
                ),
                destructive: false,
                transactional: true,
            }
        }
        PgObjectOp::DropObject { reference, cascade } => {
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "DROP {} {} {behavior};",
                    object_keyword(reference.kind),
                    render_object_identity(reference)
                ),
                summary: format!(
                    "Drop {} {} ({behavior})",
                    object_label(reference.kind),
                    display_identity(reference)
                ),
                destructive: true,
                transactional: true,
            }
        }
        PgObjectOp::SetComment { target, comment } => {
            let (target_sql, target_summary) = render_comment_target(target);
            let comment_sql = comment
                .as_deref()
                .map(quote_literal)
                .unwrap_or_else(|| "NULL".to_string());
            RenderedOp {
                sql: format!("COMMENT ON {target_sql} IS {comment_sql};"),
                summary: format!(
                    "{} comment on {target_summary}",
                    if comment.is_some() { "Set" } else { "Remove" }
                ),
                destructive: false,
                transactional: true,
            }
        }
        PgObjectOp::AddColumn {
            schema,
            table,
            column,
        } => {
            let mut definition = format!("{} {}", quote_double(&column.name), column.data_type);
            if !column.nullable {
                definition.push_str(" NOT NULL");
            }
            if let Some(default) = &column.default {
                definition.push_str(" DEFAULT ");
                definition.push_str(&render_default(default));
            }
            RenderedOp {
                sql: format!(
                    "ALTER TABLE {} ADD COLUMN {definition};",
                    qualified(schema, table)
                ),
                summary: format!("Add column {schema}.{table}.{}", column.name),
                destructive: !column.nullable,
                transactional: true,
            }
        }
        PgObjectOp::DropColumn {
            schema,
            table,
            name,
            cascade,
        } => {
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "ALTER TABLE {} DROP COLUMN {} {behavior};",
                    qualified(schema, table),
                    quote_double(name)
                ),
                summary: format!("Drop column {schema}.{table}.{name} ({behavior})"),
                destructive: true,
                transactional: true,
            }
        }
        PgObjectOp::RenameColumn {
            schema,
            table,
            name,
            new_name,
        } => RenderedOp {
            sql: format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {};",
                qualified(schema, table),
                quote_double(name),
                quote_double(new_name)
            ),
            summary: format!("Rename column {schema}.{table}.{name} to {new_name}"),
            destructive: false,
            transactional: true,
        },
        PgObjectOp::AlterColumnType {
            schema,
            table,
            name,
            new_type,
            using,
        } => RenderedOp {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {}{};",
                qualified(schema, table),
                quote_double(name),
                new_type,
                using
                    .as_ref()
                    .map(|expression| format!(" USING {expression}"))
                    .unwrap_or_default()
            ),
            summary: format!("Change type of {schema}.{table}.{name} to {new_type}"),
            destructive: true,
            transactional: true,
        },
        PgObjectOp::SetColumnNullable {
            schema,
            table,
            name,
            nullable,
        } => RenderedOp {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} {};",
                qualified(schema, table),
                quote_double(name),
                if *nullable {
                    "DROP NOT NULL"
                } else {
                    "SET NOT NULL"
                }
            ),
            summary: format!(
                "{} NOT NULL on {schema}.{table}.{name}",
                if *nullable { "Remove" } else { "Set" }
            ),
            destructive: !nullable,
            transactional: true,
        },
        PgObjectOp::SetColumnDefault {
            schema,
            table,
            name,
            default,
        } => RenderedOp {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} {};",
                qualified(schema, table),
                quote_double(name),
                default
                    .as_ref()
                    .map(|value| format!("SET DEFAULT {}", render_default(value)))
                    .unwrap_or_else(|| "DROP DEFAULT".to_string())
            ),
            summary: format!(
                "{} default on {schema}.{table}.{name}",
                if default.is_some() { "Set" } else { "Remove" }
            ),
            destructive: false,
            transactional: true,
        },
        PgObjectOp::AddPrimaryKey {
            schema,
            table,
            name,
            columns,
        } => render_constraint_add(
            schema,
            table,
            name,
            &format!("PRIMARY KEY ({})", render_ident_list(columns)),
            "primary key",
            true,
        ),
        PgObjectOp::AddUnique {
            schema,
            table,
            name,
            columns,
        } => render_constraint_add(
            schema,
            table,
            name,
            &format!("UNIQUE ({})", render_ident_list(columns)),
            "unique constraint",
            true,
        ),
        PgObjectOp::AddForeignKey {
            schema,
            table,
            name,
            columns,
            referenced_schema,
            referenced_table,
            referenced_columns,
            on_update,
            on_delete,
            deferrable,
            initially_deferred,
            not_valid,
        } => {
            let mut clause = format!(
                "FOREIGN KEY ({}) REFERENCES {} ({}) ON UPDATE {} ON DELETE {}",
                render_ident_list(columns),
                qualified(referenced_schema, referenced_table),
                render_ident_list(referenced_columns),
                on_update.sql(),
                on_delete.sql()
            );
            if *deferrable {
                clause.push_str(" DEFERRABLE");
                if *initially_deferred {
                    clause.push_str(" INITIALLY DEFERRED");
                }
            } else {
                clause.push_str(" NOT DEFERRABLE");
            }
            if *not_valid {
                clause.push_str(" NOT VALID");
            }
            render_constraint_add(schema, table, name, &clause, "foreign key", !not_valid)
        }
        PgObjectOp::AddCheck {
            schema,
            table,
            name,
            expression,
            not_valid,
        } => render_constraint_add(
            schema,
            table,
            name,
            &format!(
                "CHECK ({expression}){}",
                if *not_valid { " NOT VALID" } else { "" }
            ),
            "check constraint",
            !not_valid,
        ),
        PgObjectOp::DropConstraint {
            schema,
            table,
            name,
            cascade,
        } => {
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "ALTER TABLE {} DROP CONSTRAINT {} {behavior};",
                    qualified(schema, table),
                    quote_double(name)
                ),
                summary: format!("Drop constraint {schema}.{table}.{name} ({behavior})"),
                destructive: true,
                transactional: true,
            }
        }
        PgObjectOp::CreateIndex {
            schema,
            table,
            name,
            unique,
            method,
            columns,
            include,
            where_predicate,
            concurrently,
        } => {
            let index_name = name
                .clone()
                .unwrap_or_else(|| derived_index_name(table, columns));
            let column_sql = columns
                .iter()
                .map(|column| {
                    format!(
                        "{}{}",
                        render_index_element(&column.expression),
                        if column.descending { " DESC" } else { "" }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let include_sql = if include.is_empty() {
                String::new()
            } else {
                format!(" INCLUDE ({})", render_ident_list(include))
            };
            let predicate_sql = where_predicate
                .as_ref()
                .map(|predicate| format!(" WHERE {predicate}"))
                .unwrap_or_default();
            RenderedOp {
                sql: format!(
                    "CREATE {}INDEX {}{} ON {} USING {} ({column_sql}){include_sql}{predicate_sql};",
                    if *unique { "UNIQUE " } else { "" },
                    if *concurrently { "CONCURRENTLY " } else { "" },
                    quote_double(&index_name),
                    qualified(schema, table),
                    quote_double(method)
                ),
                summary: format!("Create index {schema}.{index_name} on {schema}.{table}"),
                destructive: *unique,
                transactional: !concurrently,
            }
        }
        PgObjectOp::DropIndex {
            schema,
            name,
            concurrently,
            cascade,
        } => {
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "DROP INDEX {}{} {behavior};",
                    if *concurrently { "CONCURRENTLY " } else { "" },
                    qualified(schema, name)
                ),
                summary: format!("Drop index {schema}.{name} ({behavior})"),
                destructive: true,
                transactional: !concurrently,
            }
        }
        PgObjectOp::CreateView {
            schema,
            name,
            or_replace,
            sql_body,
        } => RenderedOp {
            sql: format!(
                "CREATE {}VIEW {} AS {};",
                if *or_replace { "OR REPLACE " } else { "" },
                qualified(schema, name),
                trim_fragment_terminator(sql_body)
            ),
            summary: format!(
                "{} view {schema}.{name}",
                if *or_replace { "Replace" } else { "Create" }
            ),
            destructive: *or_replace,
            transactional: true,
        },
        PgObjectOp::CreateMaterializedView {
            schema,
            name,
            sql_body,
            with_data,
        } => {
            let data_clause = if *with_data {
                "WITH DATA"
            } else {
                "WITH NO DATA"
            };
            RenderedOp {
                sql: format!(
                    "CREATE MATERIALIZED VIEW {} AS {} {data_clause};",
                    qualified(schema, name),
                    trim_fragment_terminator(sql_body),
                ),
                summary: format!("Create materialized view {schema}.{name}"),
                destructive: false,
                transactional: true,
            }
        }
        PgObjectOp::CreateSequence {
            schema,
            name,
            data_type,
            start,
            increment,
            min_value,
            max_value,
            cycle,
            cache,
        } => {
            let mut options = Vec::new();
            if let Some(data_type) = data_type {
                options.push(format!("AS {data_type}"));
            }
            if let Some(increment) = increment {
                options.push(format!("INCREMENT BY {increment}"));
            }
            if let Some(min_value) = min_value {
                options.push(format!("MINVALUE {min_value}"));
            }
            if let Some(max_value) = max_value {
                options.push(format!("MAXVALUE {max_value}"));
            }
            if let Some(start) = start {
                options.push(format!("START WITH {start}"));
            }
            if let Some(cache) = cache {
                options.push(format!("CACHE {cache}"));
            }
            if let Some(cycle) = cycle {
                options.push(if *cycle { "CYCLE" } else { "NO CYCLE" }.to_string());
            }
            RenderedOp {
                sql: render_with_options("CREATE SEQUENCE", schema, name, &options),
                summary: format!("Create sequence {schema}.{name}"),
                destructive: false,
                transactional: true,
            }
        }
        PgObjectOp::AlterSequence {
            schema,
            name,
            restart_with,
            increment_by,
            min_value,
            max_value,
            cycle,
            cache,
        } => {
            let mut options = Vec::new();
            if let Some(restart_with) = restart_with {
                options.push(format!("RESTART WITH {restart_with}"));
            }
            if let Some(increment_by) = increment_by {
                options.push(format!("INCREMENT BY {increment_by}"));
            }
            if let Some(min_value) = min_value {
                options.push(format!("MINVALUE {min_value}"));
            }
            if let Some(max_value) = max_value {
                options.push(format!("MAXVALUE {max_value}"));
            }
            if let Some(cache) = cache {
                options.push(format!("CACHE {cache}"));
            }
            if let Some(cycle) = cycle {
                options.push(if *cycle { "CYCLE" } else { "NO CYCLE" }.to_string());
            }
            if options.is_empty() {
                return invalid(op_index, "alter sequence needs at least one change");
            }
            RenderedOp {
                sql: render_with_options("ALTER SEQUENCE", schema, name, &options),
                summary: format!("Alter sequence {schema}.{name}"),
                destructive: false,
                transactional: true,
            }
        }
        PgObjectOp::CreateEnum {
            schema,
            name,
            labels,
        } => RenderedOp {
            sql: format!(
                "CREATE TYPE {} AS ENUM ({});",
                qualified(schema, name),
                labels
                    .iter()
                    .map(|label| quote_literal(label))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            summary: format!("Create enum {schema}.{name}"),
            destructive: false,
            transactional: true,
        },
        PgObjectOp::AddEnumValue {
            schema,
            name,
            value,
            position,
        } => {
            let position_sql = match position {
                Some(PgEnumPosition::Before { neighbor }) => {
                    format!(" BEFORE {}", quote_literal(neighbor))
                }
                Some(PgEnumPosition::After { neighbor }) => {
                    format!(" AFTER {}", quote_literal(neighbor))
                }
                None => String::new(),
            };
            RenderedOp {
                sql: format!(
                    "ALTER TYPE {} ADD VALUE {}{position_sql};",
                    qualified(schema, name),
                    quote_literal(value)
                ),
                summary: format!("Add value to enum {schema}.{name}"),
                destructive: false,
                // A label added by ALTER TYPE ... ADD VALUE cannot be used
                // inside the same transaction (SQLSTATE 55P04), and servers
                // before 12 reject the statement in a transaction block.
                transactional: false,
            }
        }
        PgObjectOp::RenameEnumValue {
            schema,
            name,
            from,
            to,
        } => RenderedOp {
            sql: format!(
                "ALTER TYPE {} RENAME VALUE {} TO {};",
                qualified(schema, name),
                quote_literal(from),
                quote_literal(to)
            ),
            summary: format!("Rename value in enum {schema}.{name}"),
            destructive: false,
            transactional: true,
        },
    };
    Ok(rendered)
}

fn render_constraint_add(
    schema: &str,
    table: &str,
    name: &Option<String>,
    clause: &str,
    label: &str,
    destructive: bool,
) -> RenderedOp {
    RenderedOp {
        sql: format!(
            "ALTER TABLE {} ADD {}{clause};",
            qualified(schema, table),
            name.as_ref()
                .map(|name| format!("CONSTRAINT {} ", quote_double(name)))
                .unwrap_or_default()
        ),
        summary: format!("Add {label} to {schema}.{table}"),
        destructive,
        transactional: true,
    }
}

fn render_default(default: &PgDefaultValue) -> String {
    match default {
        PgDefaultValue::Literal { value } => quote_literal(value),
        PgDefaultValue::Expression { sql } => format!("({sql})"),
    }
}

fn render_comment_target(target: &PgCommentTarget) -> (String, String) {
    match target {
        PgCommentTarget::Object { reference } => (
            format!(
                "{} {}",
                object_keyword(reference.kind),
                render_object_identity(reference)
            ),
            display_identity(reference),
        ),
        PgCommentTarget::Column {
            schema,
            table,
            column,
        } => (
            format!(
                "COLUMN {}.{}",
                qualified(schema, table),
                quote_double(column)
            ),
            format!("{schema}.{table}.{column}"),
        ),
    }
}

fn object_keyword(kind: PgObjectKind) -> &'static str {
    match kind {
        PgObjectKind::Schema => "SCHEMA",
        PgObjectKind::Table => "TABLE",
        PgObjectKind::View => "VIEW",
        PgObjectKind::MaterializedView => "MATERIALIZED VIEW",
        PgObjectKind::ForeignTable => "FOREIGN TABLE",
        PgObjectKind::Sequence => "SEQUENCE",
        PgObjectKind::Function => "FUNCTION",
        PgObjectKind::Procedure => "PROCEDURE",
        PgObjectKind::Aggregate => "AGGREGATE",
        PgObjectKind::Type => "TYPE",
        PgObjectKind::Domain => "DOMAIN",
        PgObjectKind::Extension => "EXTENSION",
    }
}

fn object_label(kind: PgObjectKind) -> &'static str {
    match kind {
        PgObjectKind::Schema => "schema",
        PgObjectKind::Table => "table",
        PgObjectKind::View => "view",
        PgObjectKind::MaterializedView => "materialized view",
        PgObjectKind::ForeignTable => "foreign table",
        PgObjectKind::Sequence => "sequence",
        PgObjectKind::Function => "function",
        PgObjectKind::Procedure => "procedure",
        PgObjectKind::Aggregate => "aggregate",
        PgObjectKind::Type => "type",
        PgObjectKind::Domain => "domain",
        PgObjectKind::Extension => "extension",
    }
}

fn render_object_identity(reference: &PgObjectRef) -> String {
    match reference.kind {
        PgObjectKind::Schema | PgObjectKind::Extension => quote_double(&reference.name),
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => format!(
            "{}({})",
            qualified(
                reference.schema.as_deref().unwrap_or_default(),
                &reference.name
            ),
            reference.identity_args.as_deref().unwrap_or_default()
        ),
        _ => qualified(
            reference.schema.as_deref().unwrap_or_default(),
            &reference.name,
        ),
    }
}

fn display_identity(reference: &PgObjectRef) -> String {
    let qualified = match &reference.schema {
        Some(schema) => format!("{schema}.{}", reference.name),
        None => reference.name.clone(),
    };
    match &reference.identity_args {
        Some(arguments) => format!("{qualified}({arguments})"),
        None => qualified,
    }
}

fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_double(schema), quote_double(name))
}

/// PostgreSQL's `index_elem` grammar accepts only a bare column name or a
/// function call without parentheses; every other expression must be wrapped.
fn render_index_element(expression: &str) -> String {
    let expression = expression.trim();
    let tokens = lex_sql(expression).unwrap_or_default();
    let bare_column = matches!(tokens.as_slice(), [SqlToken::Identifier(_)]);
    if bare_column || is_function_call(&tokens) {
        expression.to_string()
    } else {
        format!("({expression})")
    }
}

/// Matches `name(...)` or `schema.name(...)` whose closing parenthesis is the
/// final token, so trailing operators such as `lower(a) || b` are not calls.
fn is_function_call(tokens: &[SqlToken]) -> bool {
    let mut index = 0usize;
    loop {
        if !matches!(tokens.get(index), Some(SqlToken::Identifier(_))) {
            return false;
        }
        index += 1;
        if matches!(tokens.get(index), Some(SqlToken::Symbol('.'))) {
            index += 1;
            continue;
        }
        break;
    }
    if !matches!(tokens.get(index), Some(SqlToken::Symbol('('))) {
        return false;
    }
    let mut depth = 0usize;
    for (position, token) in tokens.iter().enumerate().skip(index) {
        match token {
            SqlToken::Symbol('(') => depth += 1,
            SqlToken::Symbol(')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return position + 1 == tokens.len();
                }
            }
            _ => {}
        }
    }
    false
}

fn render_ident_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| quote_double(name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn trim_fragment_terminator(fragment: &str) -> &str {
    fragment.trim().trim_end_matches(';').trim_end()
}

fn render_with_options(prefix: &str, schema: &str, name: &str, options: &[String]) -> String {
    if options.is_empty() {
        format!("{prefix} {};", qualified(schema, name))
    } else {
        format!(
            "{prefix} {} {};",
            qualified(schema, name),
            options.join(" ")
        )
    }
}

pub(crate) fn derived_index_name(table: &str, columns: &[PgIndexColumn]) -> String {
    let mut parts = vec![identifierish(table)];
    parts.extend(
        columns
            .iter()
            .map(|column| identifierish(&column.expression)),
    );
    parts.push("idx".to_string());
    truncate_identifier(&parts.join("_"), 63)
}

fn identifierish(value: &str) -> String {
    let simplified = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = simplified.trim_matches('_');
    if trimmed.is_empty() {
        "expr".to_string()
    } else {
        trimmed.to_string()
    }
}

fn truncate_identifier(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_column(data_type: &str, default: Option<PgDefaultValue>) -> PgObjectOp {
        PgObjectOp::AddColumn {
            schema: "s".into(),
            table: "t".into(),
            column: NewColumnSpec {
                name: "c".into(),
                data_type: data_type.into(),
                nullable: true,
                default,
            },
        }
    }

    fn add_check(expression: &str) -> PgObjectOp {
        PgObjectOp::AddCheck {
            schema: "s".into(),
            table: "t".into(),
            name: None,
            expression: expression.into(),
            not_valid: false,
        }
    }

    fn index_with(expressions: &[&str]) -> PgObjectOp {
        PgObjectOp::CreateIndex {
            schema: "s".into(),
            table: "orders".into(),
            name: Some("orders_idx".into()),
            unique: false,
            method: "btree".into(),
            columns: expressions
                .iter()
                .map(|expression| PgIndexColumn {
                    expression: (*expression).into(),
                    descending: true,
                })
                .collect(),
            include: Vec::new(),
            where_predicate: None,
            concurrently: false,
        }
    }

    #[test]
    fn fragment_shape_uses_the_shared_lexer_for_dollar_identifiers() {
        // `d$x$` is one identifier to PostgreSQL and to lex_sql; a byte scanner
        // that treated `$x$` as a dollar-quote opener would hide the comma.
        let smuggled = add_column("d$x$ , ADD COLUMN \"evil\" text /*$x$*/", None);
        let error = generate_object_ddl(&[smuggled]).expect_err("smuggled action");
        assert!(matches!(error, PgObjectError::InvalidOp { .. }));

        let using = PgObjectOp::AlterColumnType {
            schema: "s".into(),
            table: "t".into(),
            name: "c".into(),
            new_type: "text".into(),
            using: Some("y$q$::text , DROP COLUMN \"secret\" /*$q$*/".into()),
        };
        assert!(generate_object_ddl(&[using]).is_err());
        assert!(generate_object_ddl(&[index_with(&["a$b$ , x$b$"])]).is_err());

        // Real dollar quotes and bracketed lists remain valid fragments.
        let array_default = add_column(
            "integer[]",
            Some(PgDefaultValue::Expression {
                sql: "ARRAY[1, 2]".into(),
            }),
        );
        assert!(generate_object_ddl(&[array_default]).is_ok());
        assert!(generate_object_ddl(&[add_check("name <> $tag$a, b$tag$")]).is_ok());
    }

    #[test]
    fn plain_string_backslashes_are_valid_fragments() {
        let regex = add_check(r"email ~ '^[^@]+@[^@]+\.[a-z]{2,}$'");
        let preview = generate_object_ddl(&[regex]).expect("regex check");
        assert!(preview.statements[0]
            .sql
            .contains(r"'^[^@]+@[^@]+\.[a-z]{2,}$'"));
        let view = PgObjectOp::CreateView {
            schema: "s".into(),
            name: "paths".into(),
            or_replace: false,
            sql_body: r"SELECT 'C:\temp' AS path".into(),
        };
        assert!(generate_object_ddl(&[view]).is_ok());
        // A literal whose end depends on standard_conforming_strings is still
        // refused, because the boundary could move on the server.
        assert!(generate_object_ddl(&[add_check(r"name = 'a\'' OR true")]).is_err());
    }

    #[test]
    fn index_elements_parenthesize_everything_but_columns_and_calls() {
        let preview = generate_object_ddl(&[index_with(&[
            "created_at",
            "lower(email)",
            "public.norm(a, b)",
            "a || b",
            "n::text",
            "price * quantity",
            "data->>'k'",
            "lower(a) || b",
            "(already)",
        ])])
        .expect("index DDL");
        assert!(preview.statements[0].sql.contains(
            "(created_at DESC, lower(email) DESC, public.norm(a, b) DESC, (a || b) DESC, \
             (n::text) DESC, (price * quantity) DESC, (data->>'k') DESC, \
             (lower(a) || b) DESC, ((already)) DESC)"
        ));
    }

    #[test]
    fn add_enum_value_runs_outside_atomic_groups() {
        let ops = vec![
            PgObjectOp::AddEnumValue {
                schema: "s".into(),
                name: "order_status".into(),
                value: "queued".into(),
                position: None,
            },
            PgObjectOp::SetColumnDefault {
                schema: "s".into(),
                table: "orders".into(),
                name: "status".into(),
                default: Some(PgDefaultValue::Literal {
                    value: "queued".into(),
                }),
            },
        ];
        let preview = generate_object_ddl(&ops).expect("enum plan");
        assert!(!preview.statements[0].transactional);
        assert_eq!(
            preview.groups,
            vec![
                StatementGroup::Standalone { statement_index: 0 },
                StatementGroup::Atomic {
                    statement_indexes: vec![1]
                },
            ]
        );
    }

    #[test]
    fn applied_statements_is_read_from_execution_errors() {
        let error = PgObjectError::Database {
            statement_index: Some(2),
            code: None,
            message: "boom".into(),
            position: None,
            applied_statements: 2,
            residue: None,
        };
        assert_eq!(error.applied_statements(), 2);
        assert_eq!(
            PgObjectError::PolicyBlocked {
                reason: "read-only".into()
            }
            .applied_statements(),
            0
        );
    }

    fn table_ref(kind: PgObjectKind, name: &str) -> PgObjectRef {
        PgObjectRef {
            kind,
            schema: Some("lifecycle".into()),
            name: name.into(),
            identity_args: None,
        }
    }

    fn statement(op: PgObjectOp) -> PlannedStatement {
        generate_object_ddl(&[op])
            .expect("valid DDL")
            .statements
            .into_iter()
            .next()
            .expect("one statement")
    }

    #[test]
    fn renders_schema_object_and_comment_operations_with_quoting() {
        assert_eq!(
            statement(PgObjectOp::CreateSchema {
                name: "weird\"name".into(),
            })
            .sql,
            "CREATE SCHEMA \"weird\"\"name\";"
        );
        assert_eq!(
            statement(PgObjectOp::RenameObject {
                reference: table_ref(PgObjectKind::View, "orders"),
                new_name: "renamed".into(),
            })
            .sql,
            "ALTER VIEW \"lifecycle\".\"orders\" RENAME TO \"renamed\";"
        );
        assert_eq!(
            statement(PgObjectOp::DropObject {
                reference: PgObjectRef {
                    kind: PgObjectKind::Function,
                    schema: Some("lifecycle".into()),
                    name: "add_nums".into(),
                    identity_args: Some("integer, integer".into()),
                },
                cascade: false,
            })
            .sql,
            "DROP FUNCTION \"lifecycle\".\"add_nums\"(integer, integer) RESTRICT;"
        );
        assert_eq!(
            statement(PgObjectOp::SetComment {
                target: PgCommentTarget::Object {
                    reference: table_ref(PgObjectKind::Table, "orders"),
                },
                comment: Some("it's 'quoted'".into()),
            })
            .sql,
            "COMMENT ON TABLE \"lifecycle\".\"orders\" IS E'it''s ''quoted''';"
        );
    }

    #[test]
    fn e_literals_escape_apostrophes_and_backslashes_in_every_literal_operation() {
        let preview = generate_object_ddl(&[
            PgObjectOp::SetComment {
                target: PgCommentTarget::Object {
                    reference: table_ref(PgObjectKind::Table, "orders"),
                },
                comment: Some("owner's \\note".into()),
            },
            PgObjectOp::SetColumnDefault {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "status".into(),
                default: Some(PgDefaultValue::Literal {
                    value: "queued\\later".into(),
                }),
            },
            PgObjectOp::CreateEnum {
                schema: "lifecycle".into(),
                name: "escaped_status".into(),
                labels: vec!["it's\\done".into()],
            },
        ])
        .expect("escaped E literals remain one classified DDL statement each");
        assert!(preview.statements[0].sql.ends_with("E'owner''s \\\\note';"));
        assert!(preview.statements[1]
            .sql
            .ends_with("DEFAULT E'queued\\\\later';"));
        assert!(preview.statements[2]
            .sql
            .contains("ENUM (E'it''s\\\\done')"));
    }

    #[test]
    fn renders_column_operations_and_tagged_defaults() {
        let literal = statement(PgObjectOp::SetColumnDefault {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "status".into(),
            default: Some(PgDefaultValue::Literal {
                value: "now()".into(),
            }),
        });
        let expression = statement(PgObjectOp::SetColumnDefault {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "created_at".into(),
            default: Some(PgDefaultValue::Expression {
                sql: "now()".into(),
            }),
        });
        assert!(literal.sql.ends_with("SET DEFAULT E'now()';"));
        assert!(expression.sql.ends_with("SET DEFAULT (now());"));

        assert!(statement(PgObjectOp::AlterColumnType {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            new_type: "numeric(12, 2)".into(),
            using: Some("amount::numeric".into()),
        })
        .sql
        .contains("TYPE numeric(12, 2) USING amount::numeric"));

        let variants = vec![
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "note".into(),
                    data_type: "text".into(),
                    nullable: false,
                    default: Some(PgDefaultValue::Literal {
                        value: "'quoted'".into(),
                    }),
                },
            },
            PgObjectOp::DropColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "note".into(),
                cascade: false,
            },
            PgObjectOp::RenameColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "note".into(),
                new_name: "memo".into(),
            },
            PgObjectOp::SetColumnNullable {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "note".into(),
                nullable: false,
            },
            PgObjectOp::SetColumnDefault {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "note".into(),
                default: None,
            },
        ];
        let preview = generate_object_ddl(&variants).expect("column DDL");
        assert_eq!(preview.statements.len(), variants.len());
        assert!(preview.statements[0].sql.contains("DEFAULT E'''quoted'''"));
        assert!(preview.statements[1].sql.ends_with("RESTRICT;"));
    }

    #[test]
    fn renders_constraint_and_index_operations() {
        let ops = vec![
            PgObjectOp::AddPrimaryKey {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: Some("orders_pk".into()),
                columns: vec!["id".into()],
            },
            PgObjectOp::AddUnique {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                columns: vec!["external_id".into()],
            },
            PgObjectOp::AddForeignKey {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: Some("orders_customer_fk".into()),
                columns: vec!["customer_id".into()],
                referenced_schema: "lifecycle".into(),
                referenced_table: "customers".into(),
                referenced_columns: vec!["id".into()],
                on_update: PgReferentialAction::Cascade,
                on_delete: PgReferentialAction::SetNull,
                deferrable: true,
                initially_deferred: true,
                not_valid: true,
            },
            PgObjectOp::AddCheck {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: Some("amount_positive".into()),
                expression: "amount > 0".into(),
                not_valid: false,
            },
            PgObjectOp::DropConstraint {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount_positive".into(),
                cascade: false,
            },
            PgObjectOp::CreateIndex {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                unique: false,
                method: "btree".into(),
                columns: vec![PgIndexColumn {
                    expression: "created_at".into(),
                    descending: true,
                }],
                include: vec!["status".into()],
                where_predicate: Some("status = 'open'".into()),
                concurrently: true,
            },
            PgObjectOp::DropIndex {
                schema: "lifecycle".into(),
                name: "orders_created_at_idx".into(),
                concurrently: true,
                cascade: false,
            },
        ];
        let preview = generate_object_ddl(&ops).expect("constraint DDL");
        assert!(preview.statements[2].sql.contains(
            "ON UPDATE CASCADE ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED NOT VALID"
        ));
        assert!(preview.statements[5]
            .sql
            .starts_with("CREATE INDEX CONCURRENTLY \"orders_created_at_idx\""));
        assert!(!preview.statements[2].destructive);
        assert!(preview.statements[3].destructive);
        assert!(!preview.statements[5].transactional);
        assert!(!preview.statements[6].transactional);
    }

    #[test]
    fn renders_views_sequences_and_enums() {
        let ops = vec![
            PgObjectOp::CreateView {
                schema: "lifecycle".into(),
                name: "orders_view".into(),
                or_replace: true,
                sql_body: "SELECT * FROM lifecycle.orders;".into(),
            },
            PgObjectOp::CreateMaterializedView {
                schema: "lifecycle".into(),
                name: "orders_mat".into(),
                sql_body: "SELECT 'WITH  DATA' AS marker FROM lifecycle.orders_view".into(),
                with_data: false,
            },
            PgObjectOp::CreateSequence {
                schema: "lifecycle".into(),
                name: "order_seq".into(),
                data_type: Some("bigint".into()),
                start: Some("10".into()),
                increment: Some("5".into()),
                min_value: Some("10".into()),
                max_value: Some("10000".into()),
                cycle: Some(false),
                cache: Some("20".into()),
            },
            PgObjectOp::AlterSequence {
                schema: "lifecycle".into(),
                name: "order_seq".into(),
                restart_with: Some("50".into()),
                increment_by: None,
                min_value: None,
                max_value: None,
                cycle: Some(true),
                cache: None,
            },
            PgObjectOp::CreateEnum {
                schema: "lifecycle".into(),
                name: "order_status".into(),
                labels: vec!["new".into(), "it's done".into()],
            },
            PgObjectOp::AddEnumValue {
                schema: "lifecycle".into(),
                name: "order_status".into(),
                value: "queued".into(),
                position: Some(PgEnumPosition::Before {
                    neighbor: "done".into(),
                }),
            },
            PgObjectOp::RenameEnumValue {
                schema: "lifecycle".into(),
                name: "order_status".into(),
                from: "new".into(),
                to: "fresh".into(),
            },
        ];
        let preview = generate_object_ddl(&ops).expect("lifecycle DDL");
        assert_eq!(
            preview.statements[0].sql,
            "CREATE OR REPLACE VIEW \"lifecycle\".\"orders_view\" AS SELECT * FROM lifecycle.orders;"
        );
        assert!(preview.statements[1].sql.ends_with("WITH NO DATA;"));
        assert!(preview.statements[1].sql.contains("'WITH  DATA'"));
        assert!(preview.statements[2]
            .sql
            .contains("START WITH 10 CACHE 20 NO CYCLE"));
        assert!(preview.statements[4].sql.contains("E'it''s done'"));
        assert!(preview.statements[5].sql.contains("BEFORE E'done'"));
    }

    #[test]
    fn groups_transactional_runs_around_concurrent_indexes() {
        let ops = vec![
            PgObjectOp::CreateSchema { name: "one".into() },
            PgObjectOp::CreateSchema { name: "two".into() },
            PgObjectOp::CreateIndex {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: Some("orders_idx".into()),
                unique: false,
                method: "btree".into(),
                columns: vec![PgIndexColumn {
                    expression: "id".into(),
                    descending: false,
                }],
                include: Vec::new(),
                where_predicate: None,
                concurrently: true,
            },
            PgObjectOp::CreateSchema {
                name: "three".into(),
            },
        ];
        assert_eq!(
            generate_object_ddl(&ops).expect("grouped plan").groups,
            vec![
                StatementGroup::Atomic {
                    statement_indexes: vec![0, 1]
                },
                StatementGroup::Standalone { statement_index: 2 },
                StatementGroup::Atomic {
                    statement_indexes: vec![3]
                },
            ]
        );
    }

    #[test]
    fn destructive_classification_matches_data_risk() {
        let cases = vec![
            (
                PgObjectOp::DropObject {
                    reference: table_ref(PgObjectKind::Table, "orders"),
                    cascade: false,
                },
                true,
            ),
            (
                PgObjectOp::SetColumnNullable {
                    schema: "lifecycle".into(),
                    table: "orders".into(),
                    name: "amount".into(),
                    nullable: false,
                },
                true,
            ),
            (
                PgObjectOp::AddColumn {
                    schema: "lifecycle".into(),
                    table: "orders".into(),
                    column: NewColumnSpec {
                        name: "required_value".into(),
                        data_type: "integer".into(),
                        nullable: false,
                        default: None,
                    },
                },
                true,
            ),
            (
                PgObjectOp::CreateIndex {
                    schema: "lifecycle".into(),
                    table: "orders".into(),
                    name: Some("orders_unique_amount".into()),
                    unique: true,
                    method: "btree".into(),
                    columns: vec![PgIndexColumn {
                        expression: "amount".into(),
                        descending: false,
                    }],
                    include: Vec::new(),
                    where_predicate: None,
                    concurrently: false,
                },
                true,
            ),
            (
                PgObjectOp::AddCheck {
                    schema: "lifecycle".into(),
                    table: "orders".into(),
                    name: None,
                    expression: "amount > 0".into(),
                    not_valid: true,
                },
                false,
            ),
            (
                PgObjectOp::AddCheck {
                    schema: "lifecycle".into(),
                    table: "orders".into(),
                    name: None,
                    expression: "amount > 0".into(),
                    not_valid: false,
                },
                true,
            ),
            (
                PgObjectOp::CreateView {
                    schema: "lifecycle".into(),
                    name: "danger".into(),
                    or_replace: false,
                    sql_body: "SELECT pg_terminate_backend(1)".into(),
                },
                true,
            ),
        ];
        for (op, expected) in cases {
            assert_eq!(statement(op).destructive, expected);
        }
    }

    #[test]
    fn rejects_every_fragment_boundary_and_invalid_shape() {
        let boundary_ops = vec![
            PgObjectOp::CreateView {
                schema: "lifecycle".into(),
                name: "bad".into(),
                or_replace: false,
                sql_body: "SELECT 1; DROP TABLE x".into(),
            },
            PgObjectOp::AddCheck {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                expression: "amount > 0; DROP TABLE x".into(),
                not_valid: false,
            },
            PgObjectOp::CreateIndex {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                unique: false,
                method: "btree".into(),
                columns: vec![PgIndexColumn {
                    expression: "amount; DROP TABLE x".into(),
                    descending: false,
                }],
                include: Vec::new(),
                where_predicate: None,
                concurrently: false,
            },
            PgObjectOp::AlterColumnType {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount".into(),
                new_type: "numeric".into(),
                using: Some("amount::numeric; DROP TABLE x".into()),
            },
            PgObjectOp::SetColumnDefault {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount".into(),
                default: Some(PgDefaultValue::Expression {
                    sql: "1; DROP TABLE x".into(),
                }),
            },
        ];
        for op in boundary_ops {
            assert!(matches!(
                generate_object_ddl(&[op]),
                Err(PgObjectError::InvalidOp { reason, .. })
                    if reason == "fragment contains a statement boundary"
            ));
        }

        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::DropObject {
                reference: PgObjectRef {
                    kind: PgObjectKind::Function,
                    schema: Some("lifecycle".into()),
                    name: "add_nums".into(),
                    identity_args: None,
                },
                cascade: false,
            }]),
            Err(PgObjectError::InvalidOp { .. })
        ));
        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::RenameObject {
                reference: table_ref(PgObjectKind::Function, "f"),
                new_name: "g".into(),
            }]),
            Err(PgObjectError::InvalidOp { .. })
        ));
        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::AddUnique {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                columns: Vec::new(),
            }]),
            Err(PgObjectError::InvalidOp { .. })
        ));
    }

    #[test]
    fn embedded_fragments_cannot_escape_typed_operations() {
        let attacks = vec![
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "constraint_in_type".into(),
                    data_type: "integer NOT NULL".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "check_in_type".into(),
                    data_type: "integer CHECK(false)".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::CreateSequence {
                schema: "lifecycle".into(),
                name: "unsafe_sequence".into(),
                data_type: Some("bigint OWNED BY lifecycle.orders.id".into()),
                start: None,
                increment: None,
                min_value: None,
                max_value: None,
                cycle: None,
                cache: None,
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "unsafe_column".into(),
                    data_type: "integer, DROP COLUMN amount".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "unsafe_default".into(),
                    data_type: "integer".into(),
                    nullable: true,
                    default: Some(PgDefaultValue::Expression {
                        sql: "0, DROP COLUMN amount".into(),
                    }),
                },
            },
            PgObjectOp::AlterColumnType {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount".into(),
                new_type: "numeric".into(),
                using: Some("amount::numeric, DROP COLUMN status".into()),
            },
            PgObjectOp::AlterColumnType {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount".into(),
                new_type: "numeric USING amount::numeric".into(),
                using: None,
            },
            PgObjectOp::DropObject {
                reference: PgObjectRef {
                    kind: PgObjectKind::Function,
                    schema: Some("lifecycle".into()),
                    name: "add_nums".into(),
                    identity_args: Some("integer) CASCADE --".into()),
                },
                cascade: false,
            },
        ];
        for attack in attacks {
            assert!(matches!(
                generate_object_ddl(&[attack]),
                Err(PgObjectError::InvalidOp { .. })
            ));
        }

        let legitimate = vec![
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "created_in_zone".into(),
                    data_type: "timestamp with time zone".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "ratio".into(),
                    data_type: "double precision".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "elapsed".into(),
                    data_type: "interval day to second".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "custom_value".into(),
                    data_type: "\"Custom.Schema\".\"Type.Name\"[]".into(),
                    nullable: true,
                    default: None,
                },
            },
            PgObjectOp::CreateSequence {
                schema: "lifecycle".into(),
                name: "qualified_sequence".into(),
                data_type: Some("pg_catalog.int8".into()),
                start: None,
                increment: None,
                min_value: None,
                max_value: None,
                cycle: None,
                cache: None,
            },
            PgObjectOp::AddColumn {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "amounts".into(),
                    data_type: "numeric(12, 2)[]".into(),
                    nullable: true,
                    default: Some(PgDefaultValue::Expression {
                        sql: "ARRAY[1, 2]".into(),
                    }),
                },
            },
            PgObjectOp::AlterColumnType {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount".into(),
                new_type: "numeric(12, 2)".into(),
                using: Some("coalesce(amount, 0)::numeric".into()),
            },
            PgObjectOp::SetComment {
                target: PgCommentTarget::Object {
                    reference: PgObjectRef {
                        kind: PgObjectKind::Function,
                        schema: Some("lifecycle".into()),
                        name: "mixed_args".into(),
                        identity_args: Some("numeric(12, 2), text[]".into()),
                    },
                },
                comment: Some("safe".into()),
            },
        ];
        assert_eq!(
            generate_object_ddl(&legitimate)
                .expect("legitimate nested commas")
                .statements
                .len(),
            legitimate.len()
        );

        let isolated_default = generate_object_ddl(&[PgObjectOp::AddColumn {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "typed_default".into(),
                data_type: "integer".into(),
                nullable: true,
                default: Some(PgDefaultValue::Expression {
                    sql: "0 NOT NULL".into(),
                }),
            },
        }])
        .expect("expression default remains inside its renderer-owned boundary");
        assert_eq!(
            isolated_default.statements[0].sql,
            "ALTER TABLE \"lifecycle\".\"orders\" ADD COLUMN \"typed_default\" integer DEFAULT (0 NOT NULL);"
        );

        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::AlterSequence {
                schema: "lifecycle".into(),
                name: "unsafe_sequence".into(),
                restart_with: Some("1; DROP TABLE lifecycle.orders".into()),
                increment_by: None,
                min_value: None,
                max_value: None,
                cycle: None,
                cache: None,
            }]),
            Err(PgObjectError::InvalidOp { reason, .. })
                if reason == "sequence restart must be a signed 64-bit integer"
        ));
    }

    #[test]
    fn statement_body_validation_uses_the_renderer_normalized_boundary() {
        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::CreateMaterializedView {
                schema: "lifecycle".into(),
                name: "unsafe_body".into(),
                sql_body: "SELECT 1 -- swallows renderer suffix\n".into(),
                with_data: false,
            }]),
            Err(PgObjectError::InvalidOp { reason, .. })
                if reason == "fragment escapes its typed SQL context"
        ));
    }

    #[test]
    fn explicitly_deferred_extension_drop_is_rejected() {
        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::DropObject {
                reference: PgObjectRef {
                    kind: PgObjectKind::Extension,
                    schema: Some("lifecycle".into()),
                    name: "hstore".into(),
                    identity_args: None,
                },
                cascade: false,
            }]),
            Err(PgObjectError::InvalidOp { reason, .. })
                if reason == "dropping extensions is not supported"
        ));
    }

    #[test]
    fn derived_index_names_are_explicit_and_utf8_safe() {
        let columns = vec![PgIndexColumn {
            expression: "customer_id".into(),
            descending: false,
        }];
        assert_eq!(
            derived_index_name("orders", &columns),
            "orders_customer_id_idx"
        );
        let long = derived_index_name(&"é".repeat(40), &columns);
        assert!(long.len() <= 63);
        assert!(long.is_char_boundary(long.len()));

        let too_long = "é".repeat(32);
        assert_eq!(too_long.len(), 64);
        assert!(matches!(
            generate_object_ddl(&[PgObjectOp::CreateIndex {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: Some(too_long),
                unique: false,
                method: "btree".into(),
                columns,
                include: Vec::new(),
                where_predicate: None,
                concurrently: true,
            }]),
            Err(PgObjectError::InvalidOp { reason, .. })
                if reason == "index name exceeds PostgreSQL's 63-byte limit"
        ));
    }

    #[test]
    fn wire_shapes_are_camel_case_and_tagged() {
        let value = serde_json::to_value(PgObjectOp::AlterColumnType {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            new_type: "numeric".into(),
            using: None,
        })
        .expect("serialize op");
        assert_eq!(value["op"], "alterColumnType");
        assert_eq!(value["newType"], "numeric");

        let sequence = serde_json::to_value(PgObjectOp::CreateSequence {
            schema: "lifecycle".into(),
            name: "wire_sequence".into(),
            data_type: Some("bigint".into()),
            start: Some(i64::MAX.to_string()),
            increment: Some("1".into()),
            min_value: None,
            max_value: Some(i64::MAX.to_string()),
            cycle: Some(false),
            cache: Some("1".into()),
        })
        .expect("serialize sequence op");
        assert!(sequence["start"].is_string());
        assert_eq!(sequence["start"], i64::MAX.to_string());
        assert!(sequence["maxValue"].is_string());

        let error = serde_json::to_value(PgObjectError::PolicyNeedsConfirmation {
            statements: vec![DdlStatementSummary {
                index: 0,
                summary: "Drop table lifecycle.orders (RESTRICT)".into(),
                destructive: true,
                transactional: true,
            }],
        })
        .expect("serialize error");
        assert_eq!(error["kind"], "policyNeedsConfirmation");
        assert_eq!(error["statements"][0]["transactional"], true);

        let lock_without_residue = serde_json::to_value(PgObjectError::LockTimeout {
            statement_index: 1,
            applied_statements: 0,
            residue: None,
        })
        .expect("serialize lock timeout");
        assert!(lock_without_residue.get("residue").is_none());
        let lock_with_residue = serde_json::to_value(PgObjectError::LockTimeout {
            statement_index: 1,
            applied_statements: 0,
            residue: Some(Box::new(DdlResidue::InvalidIndex {
                schema: "lifecycle".into(),
                name: "orders_idx".into(),
            })),
        })
        .expect("serialize lock residue");
        assert_eq!(lock_with_residue["residue"]["kind"], "invalidIndex");
    }
}
