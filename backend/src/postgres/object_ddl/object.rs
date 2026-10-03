//! Schema creation and the kind-generic rename, drop, and comment operations.

use serde::{Deserialize, Serialize};

use super::super::objects::{PgObjectKind, PgObjectRef};
use super::common::*;
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::{quote_double, quote_literal};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgCommentTarget {
    Object {
        reference: PgObjectRef,
    },
    Column {
        schema: String,
        table: String,
        column: String,
    },
}

pub(super) fn render_comment_target(target: &PgCommentTarget) -> (String, String) {
    match target {
        PgCommentTarget::Object { reference } => (
            format!(
                "{} {}",
                object_keyword(reference.kind),
                render_object_identity(reference)
            ),
            display_identity(reference),
        ),
        PgCommentTarget::Column {
            schema,
            table,
            column,
        } => (
            format!(
                "COLUMN {}.{}",
                qualified(schema, table),
                quote_double(column)
            ),
            format!("{schema}.{table}.{column}"),
        ),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateSchemaOp {
    pub name: String,
}

impl ObjectOperation for CreateSchemaOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self { name } = self;
        require_name(op_index, "schema name", name)
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self { name } = self;
        let rendered = RenderedOp {
            sql: format!("CREATE SCHEMA {};", quote_double(name)),
            summary: format!("Create schema {name}"),
            destructive: false,
            transactional: true,
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameObjectOp {
    pub reference: PgObjectRef,
    pub new_name: String,
}

impl ObjectOperation for RenameObjectOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            reference,
            new_name,
        } = self;
        {
            validate_reference(op_index, reference)?;
            require_name(op_index, "new name", new_name)?;
            if !matches!(
                reference.kind,
                PgObjectKind::Schema
                    | PgObjectKind::Table
                    | PgObjectKind::View
                    | PgObjectKind::MaterializedView
                    | PgObjectKind::Sequence
            ) {
                return invalid(op_index, "this object kind cannot be renamed");
            }
            Ok(())
        }
    }

    fn fragments(&self) -> Vec<&str> {
        self.reference
            .identity_args
            .iter()
            .map(String::as_str)
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            reference,
            new_name,
        } = self;
        let rendered = {
            let keyword = object_keyword(reference.kind);
            RenderedOp {
                sql: format!(
                    "ALTER {keyword} {} RENAME TO {};",
                    render_object_identity(reference),
                    quote_double(new_name)
                ),
                summary: format!(
                    "Rename {} {} to {new_name}",
                    object_label(reference.kind),
                    display_identity(reference)
                ),
                destructive: false,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropObjectOp {
    pub reference: PgObjectRef,
    pub cascade: bool,
}

impl ObjectOperation for DropObjectOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self { reference, .. } = self;
        {
            validate_reference(op_index, reference)?;
            if reference.kind == PgObjectKind::Extension {
                return invalid(op_index, "dropping extensions is not supported");
            }
            Ok(())
        }
    }

    fn fragments(&self) -> Vec<&str> {
        self.reference
            .identity_args
            .iter()
            .map(String::as_str)
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self { reference, cascade } = self;
        let rendered = {
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "DROP {} {} {behavior};",
                    object_keyword(reference.kind),
                    render_object_identity(reference)
                ),
                summary: format!(
                    "Drop {} {} ({behavior})",
                    object_label(reference.kind),
                    display_identity(reference)
                ),
                destructive: true,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetCommentOp {
    pub target: PgCommentTarget,
    pub comment: Option<String>,
}

impl ObjectOperation for SetCommentOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self { target, .. } = self;
        match target {
            PgCommentTarget::Object { reference } => validate_reference(op_index, reference),
            PgCommentTarget::Column {
                schema,
                table,
                column,
            } => {
                require_name(op_index, "schema", schema)?;
                require_name(op_index, "table", table)?;
                require_name(op_index, "column", column)
            }
        }
    }

    fn fragments(&self) -> Vec<&str> {
        match &self.target {
            PgCommentTarget::Object { reference } => {
                reference.identity_args.iter().map(String::as_str).collect()
            }
            PgCommentTarget::Column { .. } => Vec::new(),
        }
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self { target, comment } = self;
        let rendered = {
            let (target_sql, target_summary) = render_comment_target(target);
            let comment_sql = comment
                .as_deref()
                .map(quote_literal)
                .unwrap_or_else(|| "NULL".to_string());
            RenderedOp {
                sql: format!("COMMENT ON {target_sql} IS {comment_sql};"),
                summary: format!(
                    "{} comment on {target_summary}",
                    if comment.is_some() { "Set" } else { "Remove" }
                ),
                destructive: false,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}
