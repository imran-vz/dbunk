//! Sequence operations. Numeric options travel as decimal strings and are
//! checked as signed 64-bit integers before rendering.

use serde::{Deserialize, Serialize};

use super::super::sql_lex::{lex_sql, SqlToken};
use super::common::*;
use super::fragment::validate_data_type;
use super::{ObjectOperation, PgObjectError, RenderedOp};

pub(super) fn validate_sequence_data_type(
    op_index: usize,
    data_type: &str,
) -> Result<(), PgObjectError> {
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
    let matches_identifier = |identifier: &super::super::sql_lex::SqlIdentifier, expected: &str| {
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

pub(super) fn validate_i64_string(
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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateSequenceOp {
    pub schema: String,
    pub name: String,
    pub data_type: Option<String>,
    pub start: Option<String>,
    pub increment: Option<String>,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub cycle: Option<bool>,
    pub cache: Option<String>,
}

impl ObjectOperation for CreateSequenceOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            data_type,
            start,
            increment,
            min_value,
            max_value,
            cache,
            ..
        } = self;
        {
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
    }

    fn fragments(&self) -> Vec<&str> {
        self.data_type.iter().map(String::as_str).collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            data_type,
            start,
            increment,
            min_value,
            max_value,
            cycle,
            cache,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AlterSequenceOp {
    pub schema: String,
    pub name: String,
    pub restart_with: Option<String>,
    pub increment_by: Option<String>,
    pub min_value: Option<String>,
    pub max_value: Option<String>,
    pub cycle: Option<bool>,
    pub cache: Option<String>,
}

impl ObjectOperation for AlterSequenceOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            restart_with,
            increment_by,
            min_value,
            max_value,
            cache,
            ..
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "sequence", name)?;
            validate_i64_string(op_index, "sequence restart", restart_with)?;
            validate_i64_string(op_index, "sequence increment", increment_by)?;
            validate_i64_string(op_index, "sequence minimum", min_value)?;
            validate_i64_string(op_index, "sequence maximum", max_value)?;
            validate_i64_string(op_index, "sequence cache", cache)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            restart_with,
            increment_by,
            min_value,
            max_value,
            cycle,
            cache,
        } = self;
        let rendered = {
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
        };
        Ok(rendered)
    }
}
