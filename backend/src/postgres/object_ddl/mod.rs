//! Backend-owned PostgreSQL object DDL (ADR-0026, ADR-0027).
//!
//! `PgObjectOp` is the serde dispatcher that crosses IPC. Each variant wraps a
//! payload struct owned by its domain module, and every payload implements
//! [`ObjectOperation`] so validation, destructiveness scanning, and rendering
//! for one operation live together. Preview and apply share
//! [`generate_object_ddl`], which regenerates from operations at the trust
//! boundary and never accepts statement text from the frontend.

mod column;
mod common;
mod constraint;
mod enum_type;
mod fragment;
mod index;
mod object;
mod policy;
mod privilege;
mod routine;
mod sequence;
mod table;
#[cfg(test)]
mod tests;
mod trigger;
mod view;

use serde::{Deserialize, Serialize};

use super::objects::PgObjectRef;
use super::sql_class::{classify_script, StatementClass};
use common::invalid;
use fragment::fragment_is_destructive;

// The operation vocabulary is re-exported here so callers never reach into a
// domain module; most of it is consumed only by command tests and IPC decoding.
#[allow(unused_imports)]
pub(crate) use column::{
    AddColumnOp, AlterColumnTypeOp, DropColumnOp, NewColumnSpec, PgDefaultValue, PgIdentity,
    RenameColumnOp, SetColumnDefaultOp, SetColumnNullableOp,
};
#[allow(unused_imports)]
pub(crate) use common::PgGrantee;
#[allow(unused_imports)]
pub(crate) use constraint::{
    AddCheckOp, AddForeignKeyOp, AddPrimaryKeyOp, AddUniqueOp, DropConstraintOp,
    PgReferentialAction,
};
#[allow(unused_imports)]
pub(crate) use enum_type::{AddEnumValueOp, CreateEnumOp, PgEnumPosition, RenameEnumValueOp};
#[allow(unused_imports)]
pub(crate) use index::{derived_index_name, CreateIndexOp, DropIndexOp, PgIndexColumn};
#[allow(unused_imports)]
pub(crate) use object::{
    CreateSchemaOp, DropObjectOp, PgCommentTarget, RenameObjectOp, SetCommentOp,
};
#[allow(unused_imports)]
pub(crate) use policy::{CreatePolicyOp, DropPolicyOp, PgPolicyCommand, SetRowLevelSecurityOp};
#[allow(unused_imports)]
pub(crate) use privilege::{GrantPrivilegesOp, PgPrivilege, RevokePrivilegesOp};
#[allow(unused_imports)]
pub(crate) use routine::{CreateFunctionOp, CreateProcedureOp, PgParallelSafety, PgVolatility};
#[allow(unused_imports)]
pub(crate) use sequence::{AlterSequenceOp, CreateSequenceOp};
#[allow(unused_imports)]
pub(crate) use table::{CreateTableOp, PgCheckSpec, PgForeignKeySpec, PgKeySpec};
#[allow(unused_imports)]
pub(crate) use trigger::{
    CreateTriggerOp, DropTriggerOp, PgTriggerEvent, PgTriggerLevel, PgTriggerMode, PgTriggerTiming,
    SetTriggerEnabledOp,
};
#[allow(unused_imports)]
pub(crate) use view::{CreateMaterializedViewOp, CreateViewOp};

/// One typed operation's contract. `validate` runs before rendering and names
/// the operation index in every refusal; `fragments` lists the SQL-bearing
/// text that is scanned for destructive statements; `render` produces exactly
/// one statement.
pub(crate) trait ObjectOperation {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError>;
    fn fragments(&self) -> Vec<&str>;
    fn render(&self, op_index: usize) -> Result<RenderedOp, PgObjectError>;
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum PgObjectOp {
    CreateSchema(CreateSchemaOp),
    RenameObject(RenameObjectOp),
    DropObject(DropObjectOp),
    SetComment(SetCommentOp),
    AddColumn(AddColumnOp),
    DropColumn(DropColumnOp),
    RenameColumn(RenameColumnOp),
    AlterColumnType(AlterColumnTypeOp),
    SetColumnNullable(SetColumnNullableOp),
    SetColumnDefault(SetColumnDefaultOp),
    AddPrimaryKey(AddPrimaryKeyOp),
    AddUnique(AddUniqueOp),
    AddForeignKey(AddForeignKeyOp),
    AddCheck(AddCheckOp),
    DropConstraint(DropConstraintOp),
    CreateIndex(CreateIndexOp),
    DropIndex(DropIndexOp),
    CreateView(CreateViewOp),
    CreateMaterializedView(CreateMaterializedViewOp),
    CreateSequence(CreateSequenceOp),
    AlterSequence(AlterSequenceOp),
    CreateEnum(CreateEnumOp),
    AddEnumValue(AddEnumValueOp),
    RenameEnumValue(RenameEnumValueOp),
    /// One `CREATE TABLE` statement. Comments and indexes are separate
    /// operations that the designer appends after this one.
    CreateTable(CreateTableOp),
    /// `arguments` and `returns` are signature fragments; `body` is opaque
    /// and rendered inside a renderer-chosen dollar quote.
    CreateFunction(CreateFunctionOp),
    CreateProcedure(CreateProcedureOp),
    CreateTrigger(CreateTriggerOp),
    DropTrigger(DropTriggerOp),
    SetTriggerEnabled(SetTriggerEnabledOp),
    SetRowLevelSecurity(SetRowLevelSecurityOp),
    CreatePolicy(CreatePolicyOp),
    DropPolicy(DropPolicyOp),
    GrantPrivileges(GrantPrivilegesOp),
    RevokePrivileges(RevokePrivilegesOp),
}

impl PgObjectOp {
    /// Every variant dispatches to its payload; the match is exhaustive, so a
    /// new operation cannot compile without validation, fragments, and
    /// rendering.
    fn operation(&self) -> &dyn ObjectOperation {
        match self {
            Self::CreateSchema(op) => op,
            Self::RenameObject(op) => op,
            Self::DropObject(op) => op,
            Self::SetComment(op) => op,
            Self::AddColumn(op) => op,
            Self::DropColumn(op) => op,
            Self::RenameColumn(op) => op,
            Self::AlterColumnType(op) => op,
            Self::SetColumnNullable(op) => op,
            Self::SetColumnDefault(op) => op,
            Self::AddPrimaryKey(op) => op,
            Self::AddUnique(op) => op,
            Self::AddForeignKey(op) => op,
            Self::AddCheck(op) => op,
            Self::DropConstraint(op) => op,
            Self::CreateIndex(op) => op,
            Self::DropIndex(op) => op,
            Self::CreateView(op) => op,
            Self::CreateMaterializedView(op) => op,
            Self::CreateSequence(op) => op,
            Self::AlterSequence(op) => op,
            Self::CreateEnum(op) => op,
            Self::AddEnumValue(op) => op,
            Self::RenameEnumValue(op) => op,
            Self::CreateTable(op) => op,
            Self::CreateFunction(op) => op,
            Self::CreateProcedure(op) => op,
            Self::CreateTrigger(op) => op,
            Self::DropTrigger(op) => op,
            Self::SetTriggerEnabled(op) => op,
            Self::SetRowLevelSecurity(op) => op,
            Self::CreatePolicy(op) => op,
            Self::DropPolicy(op) => op,
            Self::GrantPrivileges(op) => op,
            Self::RevokePrivileges(op) => op,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlannedStatement {
    pub sql: String,
    pub summary: String,
    pub destructive: bool,
    pub transactional: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum StatementGroup {
    Atomic { statement_indexes: Vec<usize> },
    Standalone { statement_index: usize },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlPlanPreview {
    pub statements: Vec<PlannedStatement>,
    pub groups: Vec<StatementGroup>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlStatementSummary {
    pub index: usize,
    pub summary: String,
    pub destructive: bool,
    pub transactional: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DdlApplyResult {
    pub applied_statements: usize,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum DdlResidue {
    InvalidIndex { schema: String, name: String },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgObjectError {
    UnsupportedEngine {
        engine: String,
    },
    ObjectNotFound {
        reference: PgObjectRef,
    },
    InvalidOp {
        op_index: usize,
        reason: String,
    },
    PolicyBlocked {
        reason: String,
    },
    PolicyNeedsConfirmation {
        statements: Vec<DdlStatementSummary>,
    },
    Connection {
        message: String,
    },
    LockTimeout {
        statement_index: usize,
        applied_statements: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        residue: Option<Box<DdlResidue>>,
    },
    Database {
        #[serde(skip_serializing_if = "Option::is_none")]
        statement_index: Option<usize>,
        code: Option<String>,
        message: String,
        position: Option<u32>,
        applied_statements: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        residue: Option<Box<DdlResidue>>,
    },
}

impl PgObjectError {
    /// Statements committed before the failure; zero for errors raised before
    /// any group executed.
    pub(crate) fn applied_statements(&self) -> usize {
        match self {
            Self::LockTimeout {
                applied_statements, ..
            }
            | Self::Database {
                applied_statements, ..
            } => *applied_statements,
            _ => 0,
        }
    }
}

pub(crate) struct RenderedOp {
    pub(crate) sql: String,
    pub(crate) summary: String,
    pub(crate) destructive: bool,
    pub(crate) transactional: bool,
}

pub(crate) fn generate_object_ddl(ops: &[PgObjectOp]) -> Result<DdlPlanPreview, PgObjectError> {
    let mut statements = Vec::with_capacity(ops.len());
    for (op_index, op) in ops.iter().enumerate() {
        let operation = op.operation();
        operation.validate(op_index)?;
        let mut rendered = operation.render(op_index)?;
        rendered.destructive |= operation
            .fragments()
            .into_iter()
            .any(fragment_is_destructive);

        let classes = classify_script(&rendered.sql);
        if classes.len() != 1 {
            return invalid(op_index, "fragment contains a statement boundary");
        }
        if !matches!(classes[0], StatementClass::Ddl { .. }) {
            return invalid(op_index, "generated statement is not DDL");
        }
        if matches!(classes[0], StatementClass::Ddl { destructive: true }) {
            rendered.destructive = true;
        }

        statements.push(PlannedStatement {
            sql: rendered.sql,
            summary: rendered.summary,
            destructive: rendered.destructive,
            transactional: rendered.transactional,
        });
    }

    Ok(DdlPlanPreview {
        groups: group_statements(&statements),
        statements,
    })
}

pub(crate) fn statement_summaries(preview: &DdlPlanPreview) -> Vec<DdlStatementSummary> {
    preview
        .statements
        .iter()
        .enumerate()
        .map(|(index, statement)| DdlStatementSummary {
            index,
            summary: statement.summary.clone(),
            destructive: statement.destructive,
            transactional: statement.transactional,
        })
        .collect()
}

fn group_statements(statements: &[PlannedStatement]) -> Vec<StatementGroup> {
    let mut groups = Vec::new();
    let mut atomic = Vec::new();
    for (index, statement) in statements.iter().enumerate() {
        if statement.transactional {
            atomic.push(index);
            continue;
        }
        if !atomic.is_empty() {
            groups.push(StatementGroup::Atomic {
                statement_indexes: std::mem::take(&mut atomic),
            });
        }
        groups.push(StatementGroup::Standalone {
            statement_index: index,
        });
    }
    if !atomic.is_empty() {
        groups.push(StatementGroup::Atomic {
            statement_indexes: atomic,
        });
    }
    groups
}
