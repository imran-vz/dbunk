//! PostgreSQL schema edits (Plan 033 lane A): capture from the table
//! structure snapshot, a planner onto the typed `PgObjectOp`s, and an
//! executor that runs the plan on one owned connection with identity claims
//! and per-statement verification.
use super::{EnginePlan, PlanError, PlanOptions, SchemaBaseline, SchemaChangeSet, SchemaDraft};

/// Plans `draft` as ordered PostgreSQL statements. Pure and deterministic.
pub fn plan(
    _baseline: &SchemaBaseline,
    _draft: &SchemaDraft,
    _changes: &SchemaChangeSet,
    _options: &PlanOptions,
) -> Result<EnginePlan, PlanError> {
    Err(PlanError::Unsupported {
        reason: "PostgreSQL schema editing is not available yet".into(),
    })
}

#[cfg(feature = "isolated-profile")]
mod native {
    use crate::backend::table_ddl::WritePermit;
    use crate::postgres::{
        connect_spec::ResolvedPostgresConnectSpec, dedicated::DriverJoins,
        native_catalog::CatalogError,
    };
    use crate::schema_edit::{
        SchemaBaseline, SchemaDraft, SchemaEditFailure, SchemaEditOutcome, SchemaEditPreview,
        SchemaEditRequest,
    };
    use tokio::sync::watch;

    /// One complete, bounded, read-only capture of `request` on a dedicated
    /// connection.
    pub(crate) async fn capture(
        _spec: &ResolvedPostgresConnectSpec,
        _drivers: &DriverJoins,
        _cancellation: watch::Receiver<u64>,
        _request: SchemaEditRequest,
    ) -> Result<SchemaBaseline, CatalogError> {
        Err(CatalogError::UnsupportedObjectKind)
    }

    /// Runs a reviewed plan. The caller has already re-planned and compared
    /// `preview`, checked the policy and journaled the attempt.
    pub(crate) async fn execute(
        _spec: ResolvedPostgresConnectSpec,
        _drivers: DriverJoins,
        _permit: WritePermit,
        _cancellation: watch::Receiver<u64>,
        _baseline: SchemaBaseline,
        _draft: SchemaDraft,
        _preview: SchemaEditPreview,
    ) -> SchemaEditOutcome {
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Unsupported {
                reason: "PostgreSQL schema editing is not available yet".into(),
            },
        }
    }
}

#[cfg(feature = "isolated-profile")]
pub(crate) use native::{capture, execute};
