//! MySQL and MariaDB schema edits (Plan 033 lane C): full
//! `information_schema` introspection cross-checked against `SHOW CREATE
//! TABLE`, a version gate, a combined-ALTER planner and an executor on a
//! dedicated short connection.
use super::{
    EnginePlan, PlanError, PlanOptions, SchemaBaseline, SchemaChangeSet, SchemaDraft,
    SchemaEditFailure, SchemaEditOutcome, SchemaEditPreview,
};
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection};

/// Plans `draft` as ordered MySQL statements. Pure and deterministic.
pub fn plan(
    _baseline: &SchemaBaseline,
    _draft: &SchemaDraft,
    _changes: &SchemaChangeSet,
    _options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    Err(PlanError::Unsupported {
        reason: "MySQL schema editing is not available yet".into(),
    })
}

/// Captures `database.table` on the session connection. `server_version` is
/// `VERSION()`. A server below MySQL 8.0 / MariaDB 10.6, or any catalog and
/// `SHOW CREATE TABLE` disagreement, yields a `ReadOnly` baseline.
pub(crate) async fn capture(
    _connection: &mut MySqlConnection,
    _server_version: &str,
    _database: &str,
    _table: &str,
) -> Result<SchemaBaseline, String> {
    Err("MySQL schema editing is not available yet".into())
}

/// Runs a reviewed plan on a dedicated short connection opened from
/// `options`, so DDL's implicit commit never touches the user's session
/// transaction. The caller has already re-planned, compared `preview` and
/// checked the policy.
pub(crate) async fn execute(
    _options: &MySqlConnectOptions,
    _baseline: &SchemaBaseline,
    _draft: &SchemaDraft,
    _preview: &SchemaEditPreview,
) -> SchemaEditOutcome {
    SchemaEditOutcome::NotDispatched {
        reason: SchemaEditFailure::Unsupported {
            reason: "MySQL schema editing is not available yet".into(),
        },
    }
}
