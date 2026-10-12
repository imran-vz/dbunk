//! PostgreSQL schema edits (Plan 033 lane A): capture from the table
//! structure snapshot, a planner onto the typed `PgObjectOp`s, and an
//! executor that runs the plan on one owned connection.
//!
//! - `capture`: the Structure reader's snapshot, converted by the pure
//!   `baseline_from_snapshot`, bracketed by two drift-token reads.
//! - `planner`: `plan_steps` onto `PgObjectOp`s, rendered by
//!   `generate_object_ddl`. One atomic group; `VALIDATE CONSTRAINT` and
//!   `CREATE INDEX CONCURRENTLY` follow as standalone statements.
//! - `executor`: lock, drift recheck, statements, in-transaction
//!   re-capture and verification, then COMMIT.
use super::{
    DriftToken, EnginePlan, PlanError, PlanOptions, SchemaBaseline, SchemaChangeSet, SchemaDraft,
};

#[cfg(feature = "isolated-profile")]
mod capture;
#[cfg(feature = "isolated-profile")]
mod catalog;
#[cfg(feature = "isolated-profile")]
mod executor;
mod planner;
#[cfg(test)]
mod tests;
#[cfg(feature = "isolated-profile")]
mod verify;

/// The drift token: the relation OID, then a digest of its catalog rows.
pub(crate) fn drift_token(oid: u32, digest: &str) -> DriftToken {
    DriftToken(format!("pg:{oid}:{digest}"))
}

/// The relation OID a drift token was read from, if it is a PostgreSQL one.
pub(crate) fn drift_oid(token: &DriftToken) -> Option<u32> {
    let mut parts = token.0.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("pg"), Some(oid), Some(_)) => oid.parse().ok().filter(|oid| *oid != 0),
        _ => None,
    }
}

/// Plans `draft` as ordered PostgreSQL statements. Pure and deterministic.
pub fn plan(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    _changes: &SchemaChangeSet,
    options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    planner::plan(baseline, draft, options)
}

#[cfg(feature = "isolated-profile")]
mod native {
    use crate::backend::table_ddl::WritePermit;
    use crate::postgres::{
        connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins,
        native_catalog::CatalogError,
    };
    use crate::schema_edit::{
        SchemaBaseline, SchemaDraft, SchemaEditOutcome, SchemaEditPreview, SchemaEditRequest,
    };
    use tokio::sync::watch;

    /// One complete, bounded, read-only capture of `request` on dedicated
    /// connections.
    pub(crate) async fn capture(
        spec: &ResolvedPostgresConnectSpec,
        drivers: &DriverJoins,
        cancellation: watch::Receiver<u64>,
        request: SchemaEditRequest,
    ) -> Result<SchemaBaseline, CatalogError> {
        super::catalog::capture(spec, drivers, cancellation, request).await
    }

    /// Runs a reviewed plan. The caller has already re-planned and compared
    /// `preview`, checked the policy and journaled the attempt.
    pub(crate) async fn execute(
        spec: ResolvedPostgresConnectSpec,
        drivers: DriverJoins,
        permit: WritePermit,
        cancellation: watch::Receiver<u64>,
        baseline: SchemaBaseline,
        draft: SchemaDraft,
        preview: SchemaEditPreview,
    ) -> SchemaEditOutcome {
        super::executor::execute(
            spec,
            drivers,
            permit,
            cancellation,
            baseline,
            draft,
            preview,
        )
        .await
    }
}

#[cfg(feature = "isolated-profile")]
pub(crate) use native::{capture, execute};
