//! View and materialized view creation.

use serde::{Deserialize, Serialize};

use super::common::*;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateViewOp {
    pub schema: String,
    pub name: String,
    pub or_replace: bool,
    pub sql_body: String,
}

impl ObjectOperation for CreateViewOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            sql_body,
            ..
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "view", name)?;
            validate_fragment(
                op_index,
                "view SQL body",
                sql_body,
                FragmentContext::StatementBody,
            )
        }
    }

    fn fragments(&self) -> Vec<&str> {
        vec![self.sql_body.as_str()]
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            or_replace,
            sql_body,
        } = self;
        let rendered = RenderedOp {
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
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateMaterializedViewOp {
    pub schema: String,
    pub name: String,
    pub sql_body: String,
    pub with_data: bool,
}

impl ObjectOperation for CreateMaterializedViewOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            sql_body,
            ..
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "view", name)?;
            validate_fragment(
                op_index,
                "view SQL body",
                sql_body,
                FragmentContext::StatementBody,
            )
        }
    }

    fn fragments(&self) -> Vec<&str> {
        vec![self.sql_body.as_str()]
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            sql_body,
            with_data,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}
