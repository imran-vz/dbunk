//! Function and procedure operations. Signatures are validated fragments;
//! bodies are opaque and sealed in a body-derived dollar quote (ADR-0027).

use serde::{Deserialize, Serialize};

use super::super::sql_lex::{lex_sql, SqlToken};
use super::common::*;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgVolatility {
    Immutable,
    Stable,
    Volatile,
}

impl PgVolatility {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Immutable => "IMMUTABLE",
            Self::Stable => "STABLE",
            Self::Volatile => "VOLATILE",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgParallelSafety {
    Safe,
    Restricted,
    Unsafe,
}

impl PgParallelSafety {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Safe => "PARALLEL SAFE",
            Self::Restricted => "PARALLEL RESTRICTED",
            Self::Unsafe => "PARALLEL UNSAFE",
        }
    }
}

/// Keywords that would start a routine clause if they appeared at the top
/// level of a signature fragment. Refusing them keeps `arguments` and
/// `returns` from smuggling a body, a language, or an attribute.
const ROUTINE_CLAUSE_KEYWORDS: &[&str] = &[
    "as",
    "language",
    "returns",
    "begin",
    "atomic",
    "window",
    "immutable",
    "stable",
    "volatile",
    "leakproof",
    "called",
    "strict",
    "security",
    "parallel",
    "cost",
    "rows",
    "support",
    "set",
    "transform",
    "with",
];

pub(super) fn validate_routine_signature(
    op_index: usize,
    label: &str,
    fragment: &str,
    allow_empty: bool,
    allow_top_level_comma: bool,
    allow_opaque: bool,
) -> Result<(), PgObjectError> {
    if fragment.trim().is_empty() {
        if allow_empty {
            return Ok(());
        }
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    // A dollar sign has no role in a signature: dollar quotes would open a
    // body, and positional parameters do not exist in declarations. Refusing
    // the byte outright is simpler and stricter than classifying tokens.
    if fragment.contains('$') {
        return invalid(op_index, format!("{label} cannot contain a dollar sign"));
    }
    validate_fragment(op_index, label, fragment, FragmentContext::RoutineSignature)?;
    let tokens = lex_sql(fragment).map_err(|()| PgObjectError::InvalidOp {
        op_index,
        reason: format!("{label} escapes its typed SQL context"),
    })?;
    let mut depth = 0usize;
    for token in &tokens {
        match token {
            SqlToken::Symbol('(') => depth += 1,
            SqlToken::Symbol(')') => depth = depth.saturating_sub(1),
            SqlToken::Symbol(',') if depth == 0 && !allow_top_level_comma => {
                return invalid(op_index, format!("{label} cannot contain a list"));
            }
            SqlToken::Symbol(';') => {
                return invalid(op_index, "fragment contains a statement boundary");
            }
            SqlToken::Opaque if !allow_opaque => {
                return invalid(op_index, format!("{label} cannot contain a literal"));
            }
            SqlToken::Identifier(identifier)
                if depth == 0
                    && !identifier.quoted
                    && ROUTINE_CLAUSE_KEYWORDS
                        .contains(&identifier.value.to_ascii_lowercase().as_str()) =>
            {
                return invalid(
                    op_index,
                    format!("{label} contains a routine clause keyword"),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn validate_language(op_index: usize, language: &str) -> Result<(), PgObjectError> {
    // Quoted and unquoted identifiers are both valid `LANGUAGE` names and are
    // rendered exactly as typed; PostgreSQL folds the unquoted form.
    let valid = lex_sql(language)
        .is_ok_and(|tokens| matches!(tokens.as_slice(), [SqlToken::Identifier(_)]));
    if !valid {
        return invalid(op_index, "language must be a single identifier");
    }
    Ok(())
}

pub(super) fn validate_routine_body(op_index: usize, body: &str) -> Result<(), PgObjectError> {
    if body.trim().is_empty() {
        return invalid(op_index, "routine body cannot be empty");
    }
    Ok(())
}

/// Picks the first of `$dbunk$`, `$dbunk1$`, `$dbunk2$`, … that does not
/// occur in the body, so nothing inside the body can close the quote. The
/// choice is a pure function of the body, which keeps preview and apply
/// rendering identical SQL.
pub(super) fn dollar_tag(body: &str) -> String {
    let mut tag = "$dbunk$".to_string();
    let mut suffix = 0usize;
    while body.contains(&tag) {
        suffix += 1;
        tag = format!("$dbunk{suffix}$");
    }
    tag
}

/// `arguments` and `returns` are signature fragments; `body` is opaque
/// and rendered inside a renderer-chosen dollar quote.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateFunctionOp {
    pub schema: String,
    pub name: String,
    pub or_replace: bool,
    pub arguments: String,
    pub returns: String,
    pub language: String,
    pub body: String,
    pub volatility: PgVolatility,
    pub strict: bool,
    pub security_definer: bool,
    pub parallel: Option<PgParallelSafety>,
}

impl ObjectOperation for CreateFunctionOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            arguments,
            returns,
            language,
            body,
            ..
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "function", name)?;
            validate_routine_signature(
                op_index,
                "function arguments",
                arguments,
                true,
                true,
                true,
            )?;
            validate_routine_signature(op_index, "return type", returns, false, false, false)?;
            validate_language(op_index, language)?;
            validate_routine_body(op_index, body)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        vec![self.arguments.as_str(), self.returns.as_str()]
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            or_replace,
            arguments,
            returns,
            language,
            body,
            volatility,
            strict,
            security_definer,
            parallel,
        } = self;
        let rendered = {
            let mut attributes = vec![volatility.sql().to_string()];
            if *strict {
                attributes.push("STRICT".into());
            }
            if *security_definer {
                attributes.push("SECURITY DEFINER".into());
            }
            if let Some(parallel) = parallel {
                attributes.push(parallel.sql().into());
            }
            let tag = dollar_tag(body);
            RenderedOp {
            sql: format!(
                "CREATE {}FUNCTION {}({})\nRETURNS {}\nLANGUAGE {language}\n{}\nAS {tag}\n{}\n{tag};",
                if *or_replace { "OR REPLACE " } else { "" },
                qualified(schema, name),
                arguments.trim(),
                returns.trim(),
                attributes.join(" "),
                body.trim_end()
            ),
            summary: format!(
                "{} function {schema}.{name}({})",
                if *or_replace { "Create or replace" } else { "Create" },
                arguments.trim()
            ),
            destructive: *or_replace,
            transactional: true,
        }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateProcedureOp {
    pub schema: String,
    pub name: String,
    pub or_replace: bool,
    pub arguments: String,
    pub language: String,
    pub body: String,
    pub security_definer: bool,
}

impl ObjectOperation for CreateProcedureOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            name,
            arguments,
            language,
            body,
            ..
        } = self;
        {
            require_name(op_index, "schema", schema)?;
            require_name(op_index, "procedure", name)?;
            validate_routine_signature(
                op_index,
                "procedure arguments",
                arguments,
                true,
                true,
                true,
            )?;
            validate_language(op_index, language)?;
            validate_routine_body(op_index, body)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        vec![self.arguments.as_str()]
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            name,
            or_replace,
            arguments,
            language,
            body,
            security_definer,
        } = self;
        let rendered = {
            let tag = dollar_tag(body);
            RenderedOp {
                sql: format!(
                    "CREATE {}PROCEDURE {}({})\nLANGUAGE {language}{}\nAS {tag}\n{}\n{tag};",
                    if *or_replace { "OR REPLACE " } else { "" },
                    qualified(schema, name),
                    arguments.trim(),
                    if *security_definer {
                        "\nSECURITY DEFINER"
                    } else {
                        ""
                    },
                    body.trim_end()
                ),
                summary: format!(
                    "{} procedure {schema}.{name}({})",
                    if *or_replace {
                        "Create or replace"
                    } else {
                        "Create"
                    },
                    arguments.trim()
                ),
                destructive: *or_replace,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}
