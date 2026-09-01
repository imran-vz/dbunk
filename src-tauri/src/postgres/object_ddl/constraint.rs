//! Table constraint operations on existing tables.

use serde::{Deserialize, Serialize};

use super::common::*;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::quote_double;

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
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::NoAction => "NO ACTION",
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::SetDefault => "SET DEFAULT",
        }
    }
}

pub(super) fn render_constraint_add(
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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddPrimaryKeyOp {
    pub schema: String,
    pub table: String,
    pub name: Option<String>,
    pub columns: Vec<String>,
}

impl ObjectOperation for AddPrimaryKeyOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            columns,
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "constraint", name)?;
            require_names(op_index, "constraint columns", columns)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            columns,
        } = self;
        let rendered = render_constraint_add(
            schema,
            table,
            name,
            &format!("PRIMARY KEY ({})", render_ident_list(columns)),
            "primary key",
            true,
        );
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddUniqueOp {
    pub schema: String,
    pub table: String,
    pub name: Option<String>,
    pub columns: Vec<String>,
}

impl ObjectOperation for AddUniqueOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            columns,
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "constraint", name)?;
            require_names(op_index, "constraint columns", columns)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            columns,
        } = self;
        let rendered = render_constraint_add(
            schema,
            table,
            name,
            &format!("UNIQUE ({})", render_ident_list(columns)),
            "unique constraint",
            true,
        );
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddForeignKeyOp {
    pub schema: String,
    pub table: String,
    pub name: Option<String>,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: PgReferentialAction,
    pub on_delete: PgReferentialAction,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub not_valid: bool,
}

impl ObjectOperation for AddForeignKeyOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
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
        } = self;
        {
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
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
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
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddCheckOp {
    pub schema: String,
    pub table: String,
    pub name: Option<String>,
    pub expression: String,
    pub not_valid: bool,
}

impl ObjectOperation for AddCheckOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            expression,
            ..
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_optional_name(op_index, "constraint", name)?;
            validate_fragment(
                op_index,
                "check expression",
                expression,
                FragmentContext::Embedded,
            )
        }
    }

    fn fragments(&self) -> Vec<&str> {
        vec![self.expression.as_str()]
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            expression,
            not_valid,
        } = self;
        let rendered = render_constraint_add(
            schema,
            table,
            name,
            &format!(
                "CHECK ({expression}){}",
                if *not_valid { " NOT VALID" } else { "" }
            ),
            "check constraint",
            !not_valid,
        );
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropConstraintOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub cascade: bool,
}

impl ObjectOperation for DropConstraintOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            ..
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_name(op_index, "name", name)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            cascade,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}
