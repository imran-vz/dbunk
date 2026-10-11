//! In-place table alterations used by the schema editor (Plan 033): identity
//! and generation changes, collation-carrying type changes, constraint and
//! index renames, and deferred constraint validation.

use serde::{Deserialize, Serialize};

use super::column::PgIdentity;
use super::common::*;
use super::fragment::{validate_data_type, validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::quote_double;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgIdentityChange {
    /// `ADD GENERATED … AS IDENTITY` on a NOT NULL column without a default.
    Add { identity: PgIdentity },
    /// `SET GENERATED ALWAYS|BY DEFAULT` on an existing identity column.
    Set { identity: PgIdentity },
    /// `DROP IDENTITY`; the column keeps its values.
    Drop,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetColumnIdentityOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub change: PgIdentityChange,
}

impl ObjectOperation for SetColumnIdentityOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        require_table(op_index, &self.schema, &self.table)?;
        require_name(op_index, "column", &self.name)
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            change,
        } = self;
        let kind = |identity: &PgIdentity| match identity {
            PgIdentity::Always => "ALWAYS",
            PgIdentity::ByDefault => "BY DEFAULT",
        };
        let (action, summary) = match change {
            PgIdentityChange::Add { identity } => (
                format!("ADD {}", identity.sql()),
                format!("Make {schema}.{table}.{name} an identity column"),
            ),
            PgIdentityChange::Set { identity } => (
                format!("SET GENERATED {}", kind(identity)),
                format!(
                    "Make identity {schema}.{table}.{name} generated {}",
                    kind(identity).to_ascii_lowercase()
                ),
            ),
            PgIdentityChange::Drop => (
                "DROP IDENTITY".to_string(),
                format!("Remove identity from {schema}.{table}.{name}"),
            ),
        };
        Ok(RenderedOp {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} {action};",
                qualified(schema, table),
                quote_double(name)
            ),
            summary,
            destructive: false,
            transactional: true,
        })
    }
}

/// `SET EXPRESSION AS (…)` (PostgreSQL 17+) recomputes every row;
/// `DROP EXPRESSION` (PostgreSQL 13+) keeps the stored values as a plain
/// column. The planner gates both by server version.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetColumnExpressionOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub expression: Option<String>,
}

impl ObjectOperation for SetColumnExpressionOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        require_table(op_index, &self.schema, &self.table)?;
        require_name(op_index, "column", &self.name)?;
        if let Some(expression) = &self.expression {
            validate_fragment(
                op_index,
                "generated expression",
                expression,
                FragmentContext::Embedded,
            )?;
        }
        Ok(())
    }

    fn fragments(&self) -> Vec<&str> {
        self.expression.iter().map(String::as_str).collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            expression,
        } = self;
        let (action, summary, destructive) = match expression {
            Some(expression) => (
                format!("SET EXPRESSION AS ({expression})"),
                format!("Change the generated expression of {schema}.{table}.{name}"),
                true,
            ),
            None => (
                "DROP EXPRESSION".to_string(),
                format!("Make {schema}.{table}.{name} a plain column"),
                false,
            ),
        };
        Ok(RenderedOp {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} {action};",
                qualified(schema, table),
                quote_double(name)
            ),
            summary,
            destructive,
            transactional: true,
        })
    }
}

/// `ALTER COLUMN … TYPE … COLLATE "name"`. A type change without COLLATE
/// resets the column to the new type's default collation, so the planner
/// restates a kept collation with every type change.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlterColumnCollationOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub data_type: String,
    pub collation: String,
    pub using: Option<String>,
}

impl ObjectOperation for AlterColumnCollationOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        require_table(op_index, &self.schema, &self.table)?;
        require_name(op_index, "column", &self.name)?;
        validate_data_type(op_index, "column data type", &self.data_type)?;
        require_name(op_index, "collation", &self.collation)?;
        if let Some(using) = &self.using {
            validate_fragment(
                op_index,
                "USING expression",
                using,
                FragmentContext::Embedded,
            )?;
        }
        Ok(())
    }

    fn fragments(&self) -> Vec<&str> {
        std::iter::once(self.data_type.as_str())
            .chain(self.using.iter().map(String::as_str))
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            data_type,
            collation,
            using,
        } = self;
        Ok(RenderedOp {
            sql: format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {data_type} COLLATE {}{};",
                qualified(schema, table),
                quote_double(name),
                quote_double(collation),
                using
                    .as_ref()
                    .map(|expression| format!(" USING {expression}"))
                    .unwrap_or_default()
            ),
            summary: format!(
                "Change type of {schema}.{table}.{name} to {data_type} collate {collation}"
            ),
            destructive: true,
            transactional: true,
        })
    }
}

/// `ALTER TABLE … RENAME CONSTRAINT`. Renaming a primary key or unique
/// constraint also renames its index.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameConstraintOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub new_name: String,
}

impl ObjectOperation for RenameConstraintOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        require_table(op_index, &self.schema, &self.table)?;
        require_name(op_index, "constraint", &self.name)?;
        require_name(op_index, "new constraint name", &self.new_name)
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            new_name,
        } = self;
        Ok(RenderedOp {
            sql: format!(
                "ALTER TABLE {} RENAME CONSTRAINT {} TO {};",
                qualified(schema, table),
                quote_double(name),
                quote_double(new_name)
            ),
            summary: format!("Rename constraint {schema}.{table}.{name} to {new_name}"),
            destructive: false,
            transactional: true,
        })
    }
}

/// `ALTER INDEX … RENAME TO` for an index that backs no constraint.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameIndexOp {
    pub schema: String,
    pub name: String,
    pub new_name: String,
}

impl ObjectOperation for RenameIndexOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        require_name(op_index, "schema", &self.schema)?;
        require_name(op_index, "index", &self.name)?;
        require_name(op_index, "new index name", &self.new_name)?;
        if self.new_name.len() > 63 {
            return invalid(op_index, "index name exceeds PostgreSQL's 63-byte limit");
        }
        Ok(())
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            new_name,
        } = self;
        Ok(RenderedOp {
            sql: format!(
                "ALTER INDEX {} RENAME TO {};",
                qualified(schema, name),
                quote_double(new_name)
            ),
            summary: format!("Rename index {schema}.{name} to {new_name}"),
            destructive: false,
            transactional: true,
        })
    }
}

/// `ALTER TABLE … VALIDATE CONSTRAINT` for a foreign key or check added
/// `NOT VALID`. It scans the table under a SHARE UPDATE EXCLUSIVE lock.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidateConstraintOp {
    pub schema: String,
    pub table: String,
    pub name: String,
}

impl ObjectOperation for ValidateConstraintOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        require_table(op_index, &self.schema, &self.table)?;
        require_name(op_index, "constraint", &self.name)
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
        } = self;
        Ok(RenderedOp {
            sql: format!(
                "ALTER TABLE {} VALIDATE CONSTRAINT {};",
                qualified(schema, table),
                quote_double(name)
            ),
            summary: format!("Validate constraint {schema}.{table}.{name}"),
            destructive: false,
            transactional: true,
        })
    }
}
