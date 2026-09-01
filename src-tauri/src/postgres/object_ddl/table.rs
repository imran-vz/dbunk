//! `CREATE TABLE` as one statement; comments and indexes are separate
//! operations a designer appends (ADR-0027).

use serde::{Deserialize, Serialize};

use super::column::{
    column_fragments, render_column_definition, validate_column_spec, NewColumnSpec,
};
use super::common::*;
use super::constraint::PgReferentialAction;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::quote_double;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgKeySpec {
    pub name: Option<String>,
    pub columns: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgCheckSpec {
    pub name: Option<String>,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgForeignKeySpec {
    pub name: Option<String>,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: PgReferentialAction,
    pub on_delete: PgReferentialAction,
    pub deferrable: bool,
    pub initially_deferred: bool,
}

pub(super) fn validate_key_spec(
    op_index: usize,
    label: &str,
    key: &PgKeySpec,
    declared: &[String],
) -> Result<(), PgObjectError> {
    require_optional_name(op_index, label, &key.name)?;
    require_names(op_index, &format!("{label} columns"), &key.columns)?;
    require_declared_columns(op_index, label, &key.columns, declared)
}

pub(super) fn require_declared_columns(
    op_index: usize,
    label: &str,
    columns: &[String],
    declared: &[String],
) -> Result<(), PgObjectError> {
    for column in columns {
        if !declared.contains(column) {
            return invalid(
                op_index,
                format!("{label} names an undeclared column {column}"),
            );
        }
    }
    Ok(())
}

/// One `CREATE TABLE` statement. Comments and indexes are separate
/// operations that the designer appends after this one.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTableOp {
    pub schema: String,
    pub name: String,
    pub columns: Vec<NewColumnSpec>,
    pub primary_key: Option<PgKeySpec>,
    pub uniques: Vec<PgKeySpec>,
    pub checks: Vec<PgCheckSpec>,
    pub foreign_keys: Vec<PgForeignKeySpec>,
    pub unlogged: bool,
    pub if_not_exists: bool,
}

impl ObjectOperation for CreateTableOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            columns,
            primary_key,
            uniques,
            checks,
            foreign_keys,
            ..
        } = self;
        {
            require_table(op_index, schema, name)?;
            if columns.is_empty() {
                return invalid(op_index, "table columns cannot be empty");
            }
            let mut declared: Vec<String> = Vec::with_capacity(columns.len());
            for column in columns {
                validate_column_spec(op_index, column)?;
                if declared.contains(&column.name) {
                    return invalid(
                        op_index,
                        format!("column {} is declared more than once", column.name),
                    );
                }
                declared.push(column.name.clone());
            }
            if let Some(primary_key) = primary_key {
                validate_key_spec(op_index, "primary key", primary_key, &declared)?;
            }
            for unique in uniques {
                validate_key_spec(op_index, "unique constraint", unique, &declared)?;
            }
            for check in checks {
                require_optional_name(op_index, "check constraint", &check.name)?;
                validate_fragment(
                    op_index,
                    "check expression",
                    &check.expression,
                    FragmentContext::Embedded,
                )?;
            }
            for foreign_key in foreign_keys {
                require_optional_name(op_index, "foreign key", &foreign_key.name)?;
                require_names(op_index, "foreign-key columns", &foreign_key.columns)?;
                require_declared_columns(op_index, "foreign key", &foreign_key.columns, &declared)?;
                require_table(
                    op_index,
                    &foreign_key.referenced_schema,
                    &foreign_key.referenced_table,
                )?;
                require_names(
                    op_index,
                    "referenced columns",
                    &foreign_key.referenced_columns,
                )?;
                if foreign_key.columns.len() != foreign_key.referenced_columns.len() {
                    return invalid(
                        op_index,
                        "foreign-key and referenced column counts must match",
                    );
                }
                if foreign_key.initially_deferred && !foreign_key.deferrable {
                    return invalid(
                        op_index,
                        "initially deferred requires a deferrable constraint",
                    );
                }
            }
            Ok(())
        }
    }

    fn fragments(&self) -> Vec<&str> {
        self.columns
            .iter()
            .flat_map(column_fragments)
            .chain(self.checks.iter().map(|check| check.expression.as_str()))
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            columns,
            primary_key,
            uniques,
            checks,
            foreign_keys,
            unlogged,
            if_not_exists,
        } = self;
        let rendered = {
            let mut elements: Vec<String> = columns.iter().map(render_column_definition).collect();
            let constraint = |name: &Option<String>, clause: String| {
                format!(
                    "{}{clause}",
                    name.as_ref()
                        .map(|name| format!("CONSTRAINT {} ", quote_double(name)))
                        .unwrap_or_default()
                )
            };
            if let Some(primary_key) = primary_key {
                elements.push(constraint(
                    &primary_key.name,
                    format!("PRIMARY KEY ({})", render_ident_list(&primary_key.columns)),
                ));
            }
            for unique in uniques {
                elements.push(constraint(
                    &unique.name,
                    format!("UNIQUE ({})", render_ident_list(&unique.columns)),
                ));
            }
            for check in checks {
                elements.push(constraint(
                    &check.name,
                    format!("CHECK ({})", check.expression),
                ));
            }
            for foreign_key in foreign_keys {
                let mut clause = format!(
                    "FOREIGN KEY ({}) REFERENCES {} ({}) ON UPDATE {} ON DELETE {}",
                    render_ident_list(&foreign_key.columns),
                    qualified(
                        &foreign_key.referenced_schema,
                        &foreign_key.referenced_table
                    ),
                    render_ident_list(&foreign_key.referenced_columns),
                    foreign_key.on_update.sql(),
                    foreign_key.on_delete.sql()
                );
                if foreign_key.deferrable {
                    clause.push_str(" DEFERRABLE");
                    if foreign_key.initially_deferred {
                        clause.push_str(" INITIALLY DEFERRED");
                    }
                } else {
                    clause.push_str(" NOT DEFERRABLE");
                }
                elements.push(constraint(&foreign_key.name, clause));
            }
            let constraint_count = elements.len() - columns.len();
            RenderedOp {
                sql: format!(
                    "CREATE {}TABLE {}{} (\n  {}\n);",
                    if *unlogged { "UNLOGGED " } else { "" },
                    if *if_not_exists { "IF NOT EXISTS " } else { "" },
                    qualified(schema, name),
                    elements.join(",\n  ")
                ),
                summary: format!(
                    "Create table {schema}.{name} ({} {}, {constraint_count} {})",
                    columns.len(),
                    plural(columns.len(), "column"),
                    plural(constraint_count, "constraint")
                ),
                destructive: false,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}
