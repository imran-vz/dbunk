//! Unit tests for the PostgreSQL schema editor. Golden SQL runs through
//! `plan_review`, the same entry point review and apply use.
#[cfg(feature = "isolated-profile")]
mod capture;
#[cfg(feature = "isolated-profile")]
mod executor;
#[cfg(feature = "isolated-profile")]
mod live;
mod planner;
#[cfg(feature = "isolated-profile")]
mod verify;

use crate::schema_edit::fixtures::users;
use crate::schema_edit::{
    plan_review, PlanError, PlanOptions, SchemaBaseline, SchemaDraft, SchemaEditPreview,
    SchemaEngine,
};

pub(super) fn setup() -> (SchemaBaseline, SchemaDraft) {
    let baseline = users(SchemaEngine::Postgres);
    let draft = SchemaDraft::from_baseline(&baseline);
    (baseline, draft)
}

pub(super) fn review(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    options: &PlanOptions,
) -> Result<SchemaEditPreview, PlanError> {
    plan_review(baseline, draft, options)
}

pub(super) fn sql(preview: &SchemaEditPreview) -> Vec<&str> {
    preview
        .statements
        .iter()
        .map(|statement| statement.sql.as_str())
        .collect()
}
