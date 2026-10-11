//! MySQL and MariaDB schema edits (Plan 033 lane C): full
//! `information_schema` introspection cross-checked against `SHOW CREATE
//! TABLE`, a version gate, a combined-ALTER planner and an executor on a
//! dedicated short connection.
//!
//! - [`version`]: `VERSION()` parsing, the MySQL 8.0 / MariaDB 10.6 gate and
//!   feature flags.
//! - [`catalog`]: raw catalog rows and the definition text.
//! - [`definition`]: the `SHOW CREATE TABLE` parser and the `sqlparser`
//!   cross-check.
//! - [`build`]: rows plus definition into a baseline, ids and drift token.
//! - [`planner`]: the draft as ordered `ALTER TABLE` statements.
//! - [`executor`]: apply, partial-apply outcomes and verification.
use super::{
    EnginePlan, PlanError, PlanOptions, SchemaBaseline, SchemaChangeSet, SchemaDraft,
    SchemaEditOutcome, SchemaEditPreview,
};
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection};

mod build;
mod catalog;
mod definition;
mod executor;
mod planner;
mod sql;
mod version;

/// Plans `draft` as ordered MySQL statements. Pure and deterministic.
pub fn plan(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    changes: &SchemaChangeSet,
    options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    planner::plan(baseline, draft, changes, options)
}

/// Captures `database.table` on the session connection. `server_version` is
/// `VERSION()`. A server below MySQL 8.0 / MariaDB 10.6, or any catalog and
/// `SHOW CREATE TABLE` disagreement, yields a `ReadOnly` baseline.
pub(crate) async fn capture(
    connection: &mut MySqlConnection,
    server_version: &str,
    database: &str,
    table: &str,
) -> Result<SchemaBaseline, String> {
    let version = version::Version::parse(server_version);
    let catalog = catalog::read(connection, version, database, table).await?;
    Ok(build::baseline(server_version, database, table, &catalog))
}

/// Runs a reviewed plan on a dedicated short connection opened from
/// `options`, so DDL's implicit commit never touches the user's session
/// transaction. The caller has already re-planned, compared `preview` and
/// checked the policy.
pub(crate) async fn execute(
    options: &MySqlConnectOptions,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    preview: &SchemaEditPreview,
) -> SchemaEditOutcome {
    executor::execute(options, baseline, draft, preview).await
}

#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod planner_tests;
