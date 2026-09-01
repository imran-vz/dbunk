//! Lexical validation of SQL fragments embedded in generated statements.
//! PostgreSQL still parses every fragment; this pass only proves a fragment
//! cannot close renderer-owned delimiters or add a sibling statement.

use super::super::sql_class::{classify_script, StatementClass};
use super::super::sql_lex::{lex_sql, SqlToken};
use super::common::{invalid, trim_fragment_terminator};
use super::PgObjectError;

#[derive(Clone, Copy)]
pub(super) enum FragmentContext {
    Embedded,
    IdentityArguments,
    StatementBody,
    /// A routine argument list or return clause: top-level commas are part
    /// of the grammar, but clause keywords that would start a routine option
    /// (`AS`, `LANGUAGE`, …) are refused separately.
    RoutineSignature,
}

pub(super) fn validate_data_type(
    op_index: usize,
    label: &str,
    data_type: &str,
) -> Result<(), PgObjectError> {
    validate_fragment(op_index, label, data_type, FragmentContext::Embedded)?;
    let tokens = lex_sql(data_type).map_err(|_| PgObjectError::InvalidOp {
        op_index,
        reason: format!("{label} is not a valid data type"),
    })?;
    let mut depth = 0usize;
    let mut previous_was_dot = false;
    for token in tokens {
        match token {
            SqlToken::Symbol('(') => {
                depth += 1;
                previous_was_dot = false;
            }
            SqlToken::Symbol(')') => {
                depth = depth.saturating_sub(1);
                previous_was_dot = false;
            }
            SqlToken::Symbol('.') => previous_was_dot = true,
            SqlToken::Identifier(identifier) => {
                if depth == 0
                    && !previous_was_dot
                    && !identifier.quoted
                    && matches!(
                        identifier.value.to_ascii_lowercase().as_str(),
                        "not"
                            | "null"
                            | "default"
                            | "check"
                            | "constraint"
                            | "unique"
                            | "primary"
                            | "references"
                            | "generated"
                            | "identity"
                            | "collate"
                            | "storage"
                            | "compression"
                            | "options"
                            | "encoding"
                            | "using"
                    )
                {
                    return invalid(op_index, format!("{label} contains a column option"));
                }
                previous_was_dot = false;
            }
            _ => previous_was_dot = false,
        }
    }
    Ok(())
}

pub(super) fn validate_fragment(
    op_index: usize,
    label: &str,
    fragment: &str,
    context: FragmentContext,
) -> Result<(), PgObjectError> {
    let fragment = if matches!(context, FragmentContext::StatementBody) {
        trim_fragment_terminator(fragment)
    } else {
        fragment
    };
    if fragment.trim().is_empty() {
        if matches!(context, FragmentContext::IdentityArguments) {
            return Ok(());
        }
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    if classify_script(fragment).len() != 1 {
        return invalid(op_index, "fragment contains a statement boundary");
    }

    // Appending a sentinel catches a trailing line comment that would swallow
    // the renderer-owned suffix, while still permitting comments that end
    // before the fragment boundary. The same token stream then drives the
    // shape check, so fragment validation and statement classification share
    // one lexer and cannot disagree about where a string or identifier ends.
    let sentinel = "__dbunk_fragment_boundary__";
    let sentinel_sql = format!("{fragment} {sentinel}");
    let mut tokens = lex_sql(&sentinel_sql).map_err(|()| PgObjectError::InvalidOp {
        op_index,
        reason: "fragment escapes its typed SQL context".into(),
    })?;
    let sentinel_visible = matches!(
        tokens.pop(),
        Some(SqlToken::Identifier(identifier))
            if !identifier.quoted && identifier.value == sentinel
    );
    if !sentinel_visible {
        return invalid(op_index, "fragment escapes its typed SQL context");
    }

    let shape = fragment_shape(&tokens).ok_or_else(|| PgObjectError::InvalidOp {
        op_index,
        reason: "fragment escapes its typed SQL context".into(),
    })?;
    if shape.top_level_comma
        && !matches!(
            context,
            FragmentContext::IdentityArguments
                | FragmentContext::StatementBody
                | FragmentContext::RoutineSignature
        )
    {
        return invalid(op_index, "fragment escapes its typed SQL context");
    }
    if shape.semicolons > 0
        && (!matches!(context, FragmentContext::StatementBody) || shape.semicolons > 1)
    {
        return invalid(op_index, "fragment contains a statement boundary");
    }
    Ok(())
}

pub(super) struct FragmentShape {
    top_level_comma: bool,
    semicolons: usize,
}

/// Inspects only lexical structure. PostgreSQL still parses the expression,
/// but this pass makes sure an embedded fragment cannot close renderer-owned
/// delimiters or add a sibling comma-separated ALTER action.
pub(super) fn fragment_shape(tokens: &[SqlToken]) -> Option<FragmentShape> {
    let mut delimiters = Vec::new();
    let mut top_level_comma = false;
    let mut semicolons = 0usize;
    for token in tokens {
        match token {
            SqlToken::Symbol(open @ ('(' | '[')) => delimiters.push(*open),
            SqlToken::Symbol(')') => {
                if delimiters.pop() != Some('(') {
                    return None;
                }
            }
            SqlToken::Symbol(']') => {
                if delimiters.pop() != Some('[') {
                    return None;
                }
            }
            SqlToken::Symbol(',') => top_level_comma |= delimiters.is_empty(),
            SqlToken::Symbol(';') => semicolons += 1,
            _ => {}
        }
    }
    if !delimiters.is_empty() {
        return None;
    }
    Some(FragmentShape {
        top_level_comma,
        semicolons,
    })
}

pub(super) fn fragment_is_destructive(fragment: &str) -> bool {
    classify_script(fragment).iter().any(|class| {
        matches!(
            class,
            StatementClass::Ddl { destructive: true }
                | StatementClass::Dml {
                    destructive: true,
                    ..
                }
        )
    })
}
