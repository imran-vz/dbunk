//! Helpers shared by every operation domain: validation primitives, identity
//! rendering, and the grantee type used by policies and privileges.

use serde::{Deserialize, Serialize};

use super::super::objects::{PgObjectKind, PgObjectRef};
use super::fragment::{validate_fragment, FragmentContext};
use super::PgObjectError;
use crate::quote_double;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgGrantee {
    Public,
    Role { name: String },
}

impl PgGrantee {
    /// `PUBLIC` is a keyword; a role that happens to be named `public` is a
    /// quoted identifier, so the two can never be confused.
    pub(super) fn sql(&self) -> String {
        match self {
            Self::Public => "PUBLIC".to_string(),
            Self::Role { name } => quote_double(name),
        }
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Public => "PUBLIC".to_string(),
            Self::Role { name } => name.clone(),
        }
    }
}

pub(super) fn invalid<T>(op_index: usize, reason: impl Into<String>) -> Result<T, PgObjectError> {
    Err(PgObjectError::InvalidOp {
        op_index,
        reason: reason.into(),
    })
}

pub(super) fn require_name(op_index: usize, label: &str, value: &str) -> Result<(), PgObjectError> {
    if value.trim().is_empty() {
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    Ok(())
}

pub(super) fn require_names(
    op_index: usize,
    label: &str,
    values: &[String],
) -> Result<(), PgObjectError> {
    if values.is_empty() {
        return invalid(op_index, format!("{label} cannot be empty"));
    }
    for value in values {
        require_name(op_index, label, value)?;
    }
    Ok(())
}

pub(super) fn require_optional_name(
    op_index: usize,
    label: &str,
    value: &Option<String>,
) -> Result<(), PgObjectError> {
    if let Some(value) = value {
        require_name(op_index, label, value)?;
    }
    Ok(())
}

pub(super) fn require_table(
    op_index: usize,
    schema: &str,
    table: &str,
) -> Result<(), PgObjectError> {
    require_name(op_index, "schema", schema)?;
    require_name(op_index, "table", table)
}

pub(super) fn validate_reference(
    op_index: usize,
    reference: &PgObjectRef,
) -> Result<(), PgObjectError> {
    require_name(op_index, "object name", &reference.name)?;
    match reference.kind {
        PgObjectKind::Schema => {
            if reference.schema.is_some() {
                return invalid(op_index, "schema references cannot carry a parent schema");
            }
        }
        _ => {
            let Some(schema) = &reference.schema else {
                return invalid(op_index, "object schema is required");
            };
            require_name(op_index, "object schema", schema)?;
        }
    }
    match reference.kind {
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => {
            let Some(identity_args) = &reference.identity_args else {
                return invalid(op_index, "routine identity arguments are required");
            };
            validate_fragment(
                op_index,
                "routine identity arguments",
                identity_args,
                FragmentContext::IdentityArguments,
            )?;
        }
        _ if reference.identity_args.is_some() => {
            return invalid(
                op_index,
                "identity arguments are valid only for routines and aggregates",
            );
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_grantee(op_index: usize, grantee: &PgGrantee) -> Result<(), PgObjectError> {
    match grantee {
        PgGrantee::Public => Ok(()),
        PgGrantee::Role { name } => require_name(op_index, "role", name),
    }
}

pub(super) fn object_keyword(kind: PgObjectKind) -> &'static str {
    match kind {
        PgObjectKind::Schema => "SCHEMA",
        PgObjectKind::Table => "TABLE",
        PgObjectKind::View => "VIEW",
        PgObjectKind::MaterializedView => "MATERIALIZED VIEW",
        PgObjectKind::ForeignTable => "FOREIGN TABLE",
        PgObjectKind::Sequence => "SEQUENCE",
        PgObjectKind::Function => "FUNCTION",
        PgObjectKind::Procedure => "PROCEDURE",
        PgObjectKind::Aggregate => "AGGREGATE",
        PgObjectKind::Type => "TYPE",
        PgObjectKind::Domain => "DOMAIN",
        PgObjectKind::Extension => "EXTENSION",
    }
}

pub(super) fn object_label(kind: PgObjectKind) -> &'static str {
    match kind {
        PgObjectKind::Schema => "schema",
        PgObjectKind::Table => "table",
        PgObjectKind::View => "view",
        PgObjectKind::MaterializedView => "materialized view",
        PgObjectKind::ForeignTable => "foreign table",
        PgObjectKind::Sequence => "sequence",
        PgObjectKind::Function => "function",
        PgObjectKind::Procedure => "procedure",
        PgObjectKind::Aggregate => "aggregate",
        PgObjectKind::Type => "type",
        PgObjectKind::Domain => "domain",
        PgObjectKind::Extension => "extension",
    }
}

pub(super) fn render_object_identity(reference: &PgObjectRef) -> String {
    match reference.kind {
        PgObjectKind::Schema | PgObjectKind::Extension => quote_double(&reference.name),
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate => format!(
            "{}({})",
            qualified(
                reference.schema.as_deref().unwrap_or_default(),
                &reference.name
            ),
            reference.identity_args.as_deref().unwrap_or_default()
        ),
        _ => qualified(
            reference.schema.as_deref().unwrap_or_default(),
            &reference.name,
        ),
    }
}

pub(super) fn display_identity(reference: &PgObjectRef) -> String {
    let qualified = match &reference.schema {
        Some(schema) => format!("{schema}.{}", reference.name),
        None => reference.name.clone(),
    };
    match &reference.identity_args {
        Some(arguments) => format!("{qualified}({arguments})"),
        None => qualified,
    }
}

pub(super) fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_double(schema), quote_double(name))
}

pub(super) fn render_ident_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| quote_double(name))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn trim_fragment_terminator(fragment: &str) -> &str {
    fragment.trim().trim_end_matches(';').trim_end()
}

pub(super) fn render_with_options(
    prefix: &str,
    schema: &str,
    name: &str,
    options: &[String],
) -> String {
    if options.is_empty() {
        format!("{prefix} {};", qualified(schema, name))
    } else {
        format!(
            "{prefix} {} {};",
            qualified(schema, name),
            options.join(" ")
        )
    }
}

pub(super) fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        noun.to_string()
    } else {
        format!("{noun}s")
    }
}
