//! SQLite schema edits (Plan 033 lane B): capture by parsing
//! `sqlite_master.sql` and cross-checking PRAGMAs, a fast path for ADD /
//! RENAME / DROP COLUMN, and the documented 12-step table rebuild for
//! everything else. Runs on the session worker's connection.
use super::{
    EnginePlan, PlanError, PlanOptions, SchemaBaseline, SchemaChangeSet, SchemaDraft,
    SchemaEditFailure, SchemaEditOutcome, SchemaEditPreview,
};
use sqlx::SqliteConnection;

/// Plans `draft` as ordered SQLite statements. Pure and deterministic.
pub fn plan(
    _baseline: &SchemaBaseline,
    _draft: &SchemaDraft,
    _changes: &SchemaChangeSet,
    _options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    Err(PlanError::Unsupported {
        reason: "SQLite schema editing is not available yet".into(),
    })
}

/// Captures `schema.table`. Errors are user-facing text; a definition that
/// cannot be parsed or disagrees with the PRAGMAs yields a `ReadOnly`
/// baseline, not an error.
pub(crate) async fn capture(
    _connection: &mut SqliteConnection,
    _schema: &str,
    _table: &str,
    _sqlite_version: &str,
) -> Result<SchemaBaseline, String> {
    Err("SQLite schema editing is not available yet".into())
}

/// Runs a reviewed plan on the session connection. The caller has already
/// re-planned and compared `preview` and checked the policy.
pub(crate) async fn execute(
    _connection: &mut SqliteConnection,
    _baseline: &SchemaBaseline,
    _draft: &SchemaDraft,
    _preview: &SchemaEditPreview,
) -> SchemaEditOutcome {
    SchemaEditOutcome::NotDispatched {
        reason: SchemaEditFailure::Unsupported {
            reason: "SQLite schema editing is not available yet".into(),
        },
    }
}
