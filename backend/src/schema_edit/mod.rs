//! Plan 033: schema editor and schema change diff (ADR-0016).
//!
//! ```text
//! Baseline (captured, per engine) ──┐
//!                                   ├─ diff() ─▶ SchemaChangeSet ─▶ engine planner ─▶ SchemaEditPreview ─▶ apply
//! SchemaDraft (user edits)        ──┘
//! ```
//!
//! Shared here: the engine-neutral model, `diff`, operation ordering,
//! destructive classification, validation and the preview type. Each engine
//! owns its capture, planner and executor (`postgres`, `sqlite`, `mysql`);
//! there is no shared engine trait (ADR-0002). [`plan_review`] routes with a
//! `match` on the engine and is the only way a preview is produced, at review
//! and again at apply.
pub mod change;
pub mod diff;
pub mod fragment;
pub mod model;
pub mod mysql;
pub mod outcome;
pub mod postgres;
pub mod preview;
pub mod sqlite;

pub use change::*;
pub use diff::{
    column_field_changes, columns_reordered, describe_default, describe_generated,
    describe_identity, describe_members, diff, is_widening, plan_steps, same_name, types_equal,
    validate,
};
pub use fragment::{check_data_type, check_fragment, normalize_type, FragmentError};
pub use model::*;
pub use outcome::*;
pub use preview::*;

use serde::{Deserialize, Serialize};

/// Which table to capture. Names are exact (never case-folded or trimmed).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaEditRequest {
    /// PostgreSQL schema, MySQL database, SQLite schema (`main`, …).
    pub schema: String,
    pub table: String,
}

impl SchemaEditRequest {
    pub fn new(schema: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            schema: schema.into(),
            table: table.into(),
        }
    }

    pub fn validate(&self) -> Result<(), PlanError> {
        let bad = |name: &str| {
            name.is_empty() || name.len() > MAX_SCHEMA_EDIT_NAME_BYTES || name.contains('\0')
        };
        if bad(&self.schema) || bad(&self.table) {
            return Err(PlanError::Unsupported {
                reason: "The table name is empty, too long or contains NUL".into(),
            });
        }
        Ok(())
    }
}

/// What one engine planner returns: statements in execution order, their
/// grouping and engine notes. [`plan_review`] adds the change set and the
/// options to form the preview.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnginePlan {
    pub statements: Vec<PreviewStatement>,
    pub groups: Vec<PreviewGroup>,
    pub notes: Vec<EngineNote>,
}

/// The single planning entry point, pure and deterministic: equal inputs
/// always give an equal preview, which is what lets apply re-plan and compare
/// with the reviewed preview.
///
/// Refuses a read-only baseline, an invalid draft and an empty change set
/// before any engine planner runs.
pub fn plan_review(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    options: &PlanOptions,
) -> Result<SchemaEditPreview, PlanError> {
    if let Editability::ReadOnly { reasons } = &baseline.editable {
        return Err(PlanError::ReadOnly {
            reasons: reasons.clone(),
        });
    }
    let issues = validate(baseline, draft);
    if !issues.is_empty() {
        return Err(PlanError::Invalid { issues });
    }
    let changes = diff(baseline, draft);
    if changes.is_empty() {
        return Err(PlanError::NoChanges);
    }
    let plan = match baseline.engine {
        SchemaEngine::Postgres => postgres::plan(baseline, draft, &changes, options)?,
        SchemaEngine::Sqlite => sqlite::plan(baseline, draft, &changes, options)?,
        SchemaEngine::MySql => mysql::plan(baseline, draft, &changes, options)?,
    };
    let preview = SchemaEditPreview {
        engine: baseline.engine,
        changes,
        statements: plan.statements,
        groups: plan.groups,
        notes: plan.notes,
        options: options.clone(),
    };
    preview.check()?;
    Ok(preview)
}

#[cfg(test)]
pub(crate) mod fixtures;
