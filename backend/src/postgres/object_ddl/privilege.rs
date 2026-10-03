//! GRANT and REVOKE over a closed privilege vocabulary (ADR-0027).

use serde::{Deserialize, Serialize};

use super::super::objects::{PgObjectKind, PgObjectRef};
use super::common::*;
use super::{ObjectOperation, PgObjectError, RenderedOp};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgPrivilege {
    Select,
    Insert,
    Update,
    Delete,
    Truncate,
    References,
    Trigger,
    Usage,
    Create,
    Execute,
    Maintain,
}

impl PgPrivilege {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Select => "SELECT",
            Self::Insert => "INSERT",
            Self::Update => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
            Self::References => "REFERENCES",
            Self::Trigger => "TRIGGER",
            Self::Usage => "USAGE",
            Self::Create => "CREATE",
            Self::Execute => "EXECUTE",
            Self::Maintain => "MAINTAIN",
        }
    }

    /// The privileges PostgreSQL defines for each grantable object kind.
    pub(super) fn valid_for(self, kind: PgObjectKind) -> bool {
        match kind {
            PgObjectKind::Table
            | PgObjectKind::View
            | PgObjectKind::MaterializedView
            | PgObjectKind::ForeignTable => matches!(
                self,
                Self::Select
                    | Self::Insert
                    | Self::Update
                    | Self::Delete
                    | Self::Truncate
                    | Self::References
                    | Self::Trigger
                    | Self::Maintain
            ),
            PgObjectKind::Sequence => matches!(self, Self::Usage | Self::Select | Self::Update),
            PgObjectKind::Schema => matches!(self, Self::Usage | Self::Create),
            PgObjectKind::Function | PgObjectKind::Procedure => matches!(self, Self::Execute),
            PgObjectKind::Aggregate
            | PgObjectKind::Type
            | PgObjectKind::Domain
            | PgObjectKind::Extension => false,
        }
    }
}

pub(super) fn validate_privilege_set(
    op_index: usize,
    target: &PgObjectRef,
    privileges: &[PgPrivilege],
    all_privileges: bool,
    grantee: &PgGrantee,
) -> Result<(), PgObjectError> {
    validate_reference(op_index, target)?;
    if !matches!(
        target.kind,
        PgObjectKind::Table
            | PgObjectKind::View
            | PgObjectKind::MaterializedView
            | PgObjectKind::ForeignTable
            | PgObjectKind::Sequence
            | PgObjectKind::Schema
            | PgObjectKind::Function
            | PgObjectKind::Procedure
    ) {
        return invalid(
            op_index,
            format!(
                "privileges on {}s are not supported",
                object_label(target.kind)
            ),
        );
    }
    validate_grantee(op_index, grantee)?;
    if all_privileges && !privileges.is_empty() {
        return invalid(
            op_index,
            "all privileges cannot be combined with a privilege list",
        );
    }
    if !all_privileges && privileges.is_empty() {
        return invalid(op_index, "privilege list cannot be empty");
    }
    let mut seen = Vec::with_capacity(privileges.len());
    for privilege in privileges {
        if !privilege.valid_for(target.kind) {
            return invalid(
                op_index,
                format!(
                    "{} is not a {} privilege",
                    privilege.sql(),
                    object_label(target.kind)
                ),
            );
        }
        if seen.contains(privilege) {
            return invalid(
                op_index,
                format!("{} is listed more than once", privilege.sql()),
            );
        }
        seen.push(*privilege);
    }
    Ok(())
}

pub(super) fn render_privileges(privileges: &[PgPrivilege], all_privileges: bool) -> String {
    if all_privileges {
        "ALL PRIVILEGES".to_string()
    } else {
        privileges
            .iter()
            .map(|privilege| privilege.sql())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// `GRANT … ON TABLE` covers every relation kind; the other kinds use their
/// own keyword, and routines carry their identity arguments.
pub(super) fn render_privilege_target(target: &PgObjectRef) -> String {
    let keyword = match target.kind {
        PgObjectKind::Table
        | PgObjectKind::View
        | PgObjectKind::MaterializedView
        | PgObjectKind::ForeignTable => "TABLE",
        other => object_keyword(other),
    };
    format!("{keyword} {}", render_object_identity(target))
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantPrivilegesOp {
    pub target: PgObjectRef,
    pub privileges: Vec<PgPrivilege>,
    pub all_privileges: bool,
    pub grantee: PgGrantee,
    pub with_grant_option: bool,
}

impl ObjectOperation for GrantPrivilegesOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            target,
            privileges,
            all_privileges,
            grantee,
            ..
        } = self;
        validate_privilege_set(op_index, target, privileges, *all_privileges, grantee)
    }

    fn fragments(&self) -> Vec<&str> {
        self.target
            .identity_args
            .iter()
            .map(String::as_str)
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            target,
            privileges,
            all_privileges,
            grantee,
            with_grant_option,
        } = self;
        let rendered = {
            let privileges_sql = render_privileges(privileges, *all_privileges);
            RenderedOp {
                sql: format!(
                    "GRANT {privileges_sql} ON {} TO {}{};",
                    render_privilege_target(target),
                    grantee.sql(),
                    if *with_grant_option {
                        " WITH GRANT OPTION"
                    } else {
                        ""
                    }
                ),
                summary: format!(
                    "Grant {privileges_sql} on {} {} to {}",
                    object_label(target.kind),
                    display_identity(target),
                    grantee.label()
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
pub(crate) struct RevokePrivilegesOp {
    pub target: PgObjectRef,
    pub privileges: Vec<PgPrivilege>,
    pub all_privileges: bool,
    pub grantee: PgGrantee,
    pub grant_option_for: bool,
    pub cascade: bool,
}

impl ObjectOperation for RevokePrivilegesOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            target,
            privileges,
            all_privileges,
            grantee,
            ..
        } = self;
        validate_privilege_set(op_index, target, privileges, *all_privileges, grantee)
    }

    fn fragments(&self) -> Vec<&str> {
        self.target
            .identity_args
            .iter()
            .map(String::as_str)
            .collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            target,
            privileges,
            all_privileges,
            grantee,
            grant_option_for,
            cascade,
        } = self;
        let rendered = {
            let privileges_sql = render_privileges(privileges, *all_privileges);
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "REVOKE {}{privileges_sql} ON {} FROM {} {behavior};",
                    if *grant_option_for {
                        "GRANT OPTION FOR "
                    } else {
                        ""
                    },
                    render_privilege_target(target),
                    grantee.sql()
                ),
                summary: format!(
                    "Revoke {}{privileges_sql} on {} {} from {} ({behavior})",
                    if *grant_option_for {
                        "grant option for "
                    } else {
                        ""
                    },
                    object_label(target.kind),
                    display_identity(target),
                    grantee.label()
                ),
                // Losing a privilege breaks whatever depended on it; the
                // classifier treats REVOKE as ordinary non-destructive DDL.
                destructive: true,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}
