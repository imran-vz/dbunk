//! Enum type operations. Adding a label is deliberately non-transactional.

use serde::{Deserialize, Serialize};

use super::common::*;
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::quote_literal;

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
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateEnumOp {
    pub schema: String,
    pub name: String,
    pub labels: Vec<String>,
}

impl ObjectOperation for CreateEnumOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            labels,
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "enum", name)?;
            if labels.is_empty() {
                return invalid(op_index, "enum labels cannot be empty");
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
            labels,
        } = self;
        let rendered = RenderedOp {
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
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddEnumValueOp {
    pub schema: String,
    pub name: String,
    pub value: String,
    pub position: Option<PgEnumPosition>,
}

impl ObjectOperation for AddEnumValueOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            position,
            ..
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "enum", name)?;
            if let Some(PgEnumPosition::Before { neighbor } | PgEnumPosition::After { neighbor }) =
                position
            {
                require_name(op_index, "neighbor enum label", neighbor)?;
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
            value,
            position,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameEnumValueOp {
    pub schema: String,
    pub name: String,
    pub from: String,
    pub to: String,
}

impl ObjectOperation for RenameEnumValueOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            from: _,
            to: _,
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "enum", name)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            from,
            to,
        } = self;
        let rendered = RenderedOp {
            sql: format!(
                "ALTER TYPE {} RENAME VALUE {} TO {};",
                qualified(schema, name),
                quote_literal(from),
                quote_literal(to)
            ),
            summary: format!("Rename value in enum {schema}.{name}"),
            destructive: false,
            transactional: true,
        };
        Ok(rendered)
    }
}
