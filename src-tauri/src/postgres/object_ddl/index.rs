//! Index operations. Concurrent builds and drops are non-transactional.

use serde::{Deserialize, Serialize};

use super::super::sql_lex::{lex_sql, SqlToken};
use super::common::*;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::quote_double;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgIndexColumn {
    pub expression: String,
    pub descending: bool,
}

/// PostgreSQL's `index_elem` grammar accepts only a bare column name or a
/// function call without parentheses; every other expression must be wrapped.
pub(super) fn render_index_element(expression: &str) -> String {
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
pub(super) fn is_function_call(tokens: &[SqlToken]) -> bool {
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

pub(super) fn identifierish(value: &str) -> String {
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

pub(super) fn truncate_identifier(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateIndexOp {
    pub schema: String,
    pub table: String,
    pub name: Option<String>,
    pub unique: bool,
    pub method: String,
    pub columns: Vec<PgIndexColumn>,
    pub include: Vec<String>,
    pub where_predicate: Option<String>,
    pub concurrently: bool,
}

impl ObjectOperation for CreateIndexOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            method,
            columns,
            include,
            where_predicate,
            ..
        } = self;
        {
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
    }

    fn fragments(&self) -> Vec<&str> {
        self.columns
            .iter()
            .map(|column| column.expression.as_str())
            .chain(self.where_predicate.iter().map(String::as_str))
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            unique,
            method,
            columns,
            include,
            where_predicate,
            concurrently,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropIndexOp {
    pub schema: String,
    pub name: String,
    pub concurrently: bool,
    pub cascade: bool,
}

impl ObjectOperation for DropIndexOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            concurrently,
            cascade,
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "index", name)?;
            if *concurrently && *cascade {
                return invalid(op_index, "DROP INDEX CONCURRENTLY cannot use CASCADE");
            }
            Ok(())
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            concurrently,
            cascade,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}
