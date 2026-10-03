//! Row-level security and policy operations (ADR-0027).

use serde::{Deserialize, Serialize};

use super::common::*;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::quote_double;

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgPolicyCommand {
    All,
    Select,
    Insert,
    Update,
    Delete,
}

impl PgPolicyCommand {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetRowLevelSecurityOp {
    pub schema: String,
    pub table: String,
    pub enabled: bool,
    pub force: Option<bool>,
}

impl ObjectOperation for SetRowLevelSecurityOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self { schema, table, .. } = self;
        {
            require_table(op_index, schema, table)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            enabled,
            force,
        } = self;
        let rendered = {
            let mut actions = vec![if *enabled {
                "ENABLE ROW LEVEL SECURITY"
            } else {
                "DISABLE ROW LEVEL SECURITY"
            }];
            if let Some(force) = force {
                actions.push(if *force {
                    "FORCE ROW LEVEL SECURITY"
                } else {
                    "NO FORCE ROW LEVEL SECURITY"
                });
            }
            RenderedOp {
                sql: format!(
                    "ALTER TABLE {} {};",
                    qualified(schema, table),
                    actions.join(", ")
                ),
                summary: format!(
                    "{} row-level security on {schema}.{table}{}",
                    if *enabled { "Enable" } else { "Disable" },
                    match force {
                        Some(true) => " (forced)",
                        Some(false) => " (not forced)",
                        None => "",
                    }
                ),
                // Disabling row security exposes every row to every role
                // with table privileges.
                destructive: !enabled,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatePolicyOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub permissive: bool,
    pub command: PgPolicyCommand,
    pub roles: Vec<PgGrantee>,
    pub using: Option<String>,
    pub with_check: Option<String>,
}

impl ObjectOperation for CreatePolicyOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            command,
            roles,
            using,
            with_check,
            ..
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_name(op_index, "policy", name)?;
            if roles.is_empty() {
                return invalid(op_index, "policy roles cannot be empty");
            }
            for role in roles {
                validate_grantee(op_index, role)?;
            }
            if *command == PgPolicyCommand::Insert && using.is_some() {
                return invalid(op_index, "INSERT policies cannot have a USING expression");
            }
            if matches!(command, PgPolicyCommand::Select | PgPolicyCommand::Delete)
                && with_check.is_some()
            {
                return invalid(
                    op_index,
                    format!(
                        "{} policies cannot have a WITH CHECK expression",
                        command.sql()
                    ),
                );
            }
            if let Some(using) = using {
                validate_fragment(
                    op_index,
                    "policy USING expression",
                    using,
                    FragmentContext::Embedded,
                )?;
            }
            if let Some(with_check) = with_check {
                validate_fragment(
                    op_index,
                    "policy WITH CHECK expression",
                    with_check,
                    FragmentContext::Embedded,
                )?;
            }
            Ok(())
        }
    }

    fn fragments(&self) -> Vec<&str> {
        self.using
            .iter()
            .chain(self.with_check.iter())
            .map(String::as_str)
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            permissive,
            command,
            roles,
            using,
            with_check,
        } = self;
        let rendered = {
            let roles_sql = roles
                .iter()
                .map(PgGrantee::sql)
                .collect::<Vec<_>>()
                .join(", ");
            let using_sql = using
                .as_ref()
                .map(|using| format!(" USING ({using})"))
                .unwrap_or_default();
            let check_sql = with_check
                .as_ref()
                .map(|check| format!(" WITH CHECK ({check})"))
                .unwrap_or_default();
            RenderedOp {
                sql: format!(
                    "CREATE POLICY {} ON {} AS {} FOR {} TO {roles_sql}{using_sql}{check_sql};",
                    quote_double(name),
                    qualified(schema, table),
                    if *permissive {
                        "PERMISSIVE"
                    } else {
                        "RESTRICTIVE"
                    },
                    command.sql()
                ),
                summary: format!("Create policy {name} on {schema}.{table}"),
                destructive: false,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropPolicyOp {
    pub schema: String,
    pub table: String,
    pub name: String,
}

impl ObjectOperation for DropPolicyOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_name(op_index, "policy", name)
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
        } = self;
        let rendered = RenderedOp {
            sql: format!(
                "DROP POLICY {} ON {};",
                quote_double(name),
                qualified(schema, table)
            ),
            summary: format!("Drop policy {name} on {schema}.{table}"),
            destructive: true,
            transactional: true,
        };
        Ok(rendered)
    }
}
