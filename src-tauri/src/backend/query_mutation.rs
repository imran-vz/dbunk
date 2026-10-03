//! Query mutation provenance contains SQL and parameter names, never bindings.
//! It is not execution authority: hosts must attach it to the matching result.
use crate::postgres::{
    sql_class::describe_script,
    sql_lex::{lex_sql_spanned_bounded, SqlLexError},
    sql_params::{plan_execution, scan_parameters, ParameterValue, MAX_NAME_BYTES, MAX_PARAMETERS},
};
use crate::result_mutation::protocol::AnalyzeSource;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub const MAX_QUERY_SOURCE_BYTES: usize = 1024 * 1024;
pub const MAX_QUERY_PARAMETER_NAMES: usize = MAX_PARAMETERS;
pub const MAX_QUERY_PARAMETER_NAME_BYTES: usize = MAX_NAME_BYTES;
pub const MAX_QUERY_SOURCE_TOKENS: usize = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMutationSourceError {
    TooLarge,
    TooManyTokens,
    InvalidSql,
    NotSingleStatement,
    TooManyParameters,
    ParameterNameTooLong,
    ParametersRejected,
    ProvenanceMismatch,
    SessionDependentTarget,
}
impl fmt::Display for QueryMutationSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "Query mutation source exceeds 1 MiB",
            Self::TooManyTokens => "Query mutation source exceeds 16,384 SQL tokens",
            Self::InvalidSql => "Query mutation source cannot be lexed",
            Self::NotSingleStatement => "Query mutations require one statement",
            Self::TooManyParameters => "Query mutation source exceeds 256 parameter names",
            Self::ParameterNameTooLong => "Query parameter name exceeds 63 bytes",
            Self::ParametersRejected => "Query mutation parameter rewrite was refused",
            Self::ProvenanceMismatch => "Query mutation source does not match its planner rewrite",
            Self::SessionDependentTarget => "Result editing requires a supported SELECT with every FROM/JOIN table fully schema-qualified outside temporary schemas; execution and analysis use separate session contexts",
        })
    }
}
impl std::error::Error for QueryMutationSourceError {}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryMutationSource {
    original_sql: String,
    statement_sql: String,
    parameter_mode: bool,
    parameter_names: Vec<String>,
}
impl fmt::Debug for QueryMutationSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryMutationSource")
            .field("original_bytes", &self.original_sql.len())
            .field("statement_bytes", &self.statement_sql.len())
            .field("parameter_mode", &self.parameter_mode)
            .field("parameter_count", &self.parameter_names.len())
            .finish_non_exhaustive()
    }
}
impl QueryMutationSource {
    pub fn new(
        original_sql: String,
        parameter_mode: bool,
    ) -> Result<Self, QueryMutationSourceError> {
        let (statement_sql, parameter_names) = planned(&original_sql, parameter_mode)?;
        Ok(Self {
            original_sql,
            statement_sql,
            parameter_mode,
            parameter_names,
        })
    }
    pub fn original_sql(&self) -> &str {
        &self.original_sql
    }
    pub fn statement_sql(&self) -> &str {
        &self.statement_sql
    }
    pub fn parameter_mode(&self) -> bool {
        self.parameter_mode
    }
    pub fn parameter_names(&self) -> &[String] {
        &self.parameter_names
    }
    /// Recompute rather than trusting durable derived text or parameter order.
    pub fn validate(&self) -> Result<(), QueryMutationSourceError> {
        if self.statement_sql.len() > MAX_QUERY_SOURCE_BYTES {
            return Err(QueryMutationSourceError::TooLarge);
        }
        check_names(&self.parameter_names)?;
        let (statement, names) = planned(&self.original_sql, self.parameter_mode)?;
        if statement != self.statement_sql || names != self.parameter_names {
            return Err(QueryMutationSourceError::ProvenanceMismatch);
        }
        Ok(())
    }
    /// Constructors and deserialization already validated this immutable source.
    /// Analysis only needs an owned statement for the dedicated data socket.
    pub fn analysis_source(&self) -> Result<AnalyzeSource, QueryMutationSourceError> {
        Ok(AnalyzeSource::NativeStatement {
            sql: self.statement_sql.clone(),
        })
    }
}
impl<'de> Deserialize<'de> for QueryMutationSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Stored {
            original_sql: String,
            statement_sql: String,
            parameter_mode: bool,
            parameter_names: Vec<String>,
        }
        let stored = Stored::deserialize(deserializer)?;
        let source = Self {
            original_sql: stored.original_sql,
            statement_sql: stored.statement_sql,
            parameter_mode: stored.parameter_mode,
            parameter_names: stored.parameter_names,
        };
        source.validate().map_err(serde::de::Error::custom)?;
        Ok(source)
    }
}
fn check_names(names: &[String]) -> Result<(), QueryMutationSourceError> {
    if names.len() > MAX_QUERY_PARAMETER_NAMES {
        return Err(QueryMutationSourceError::TooManyParameters);
    }
    if names
        .iter()
        .any(|name| name.len() > MAX_QUERY_PARAMETER_NAME_BYTES)
    {
        return Err(QueryMutationSourceError::ParameterNameTooLong);
    }
    Ok(())
}
fn planned(sql: &str, mode: bool) -> Result<(String, Vec<String>), QueryMutationSourceError> {
    if sql.len() > MAX_QUERY_SOURCE_BYTES {
        return Err(QueryMutationSourceError::TooLarge);
    }
    if sql.contains('\0') {
        return Err(QueryMutationSourceError::InvalidSql);
    }
    // Drop this bounded preflight before the planner's own tokenization. A
    // recognized :name pair becomes one $k token, so rewriting cannot increase
    // the token count. Existing execution APIs retain their original limits.
    lex_sql_spanned_bounded(sql, MAX_QUERY_SOURCE_TOKENS).map_err(|error| match error {
        SqlLexError::InvalidSql => QueryMutationSourceError::InvalidSql,
        SqlLexError::TooManyTokens => QueryMutationSourceError::TooManyTokens,
    })?;
    let names = if mode {
        let scan = scan_parameters(sql).map_err(|()| QueryMutationSourceError::InvalidSql)?;
        check_names(scan.names())?;
        scan.names().to_vec()
    } else {
        Vec::new()
    };
    // NULL stand-ins satisfy the planner's binding-shape check only. They are
    // never sent anywhere, persisted, or substituted into SQL text.
    let placeholders = names
        .iter()
        .map(|name| ParameterValue {
            name: name.clone(),
            value: None,
        })
        .collect::<Vec<_>>();
    let plan = plan_execution(
        sql.to_owned(),
        mode.then_some(placeholders.as_slice()),
        None,
    )
    .map_err(|_| QueryMutationSourceError::ParametersRejected)?;
    let rewritten = plan.policy_sql();
    let statements =
        describe_script(rewritten).map_err(|()| QueryMutationSourceError::InvalidSql)?;
    let [statement] = statements.as_slice() else {
        return Err(QueryMutationSourceError::NotSingleStatement);
    };
    let text = &rewritten[statement.start..statement.end];
    if text.len() > MAX_QUERY_SOURCE_BYTES {
        return Err(QueryMutationSourceError::TooLarge);
    }
    crate::result_mutation::require_qualified_query_targets(text)
        .map_err(|()| QueryMutationSourceError::SessionDependentTarget)?;
    Ok((text.to_owned(), names))
}

#[cfg(test)]
mod tests;
