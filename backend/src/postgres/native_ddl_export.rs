//! One owned catalog snapshot, producing bounded SQL text without executing it.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::DriverJoins,
    native_catalog::{begin_snapshot, description, owned_read, CatalogError},
    objects::{PgObjectKind, PgObjectRef},
};
use std::time::Duration;
use tokio::sync::watch;
mod bounds;
mod reader;
mod render;
#[cfg(test)]
mod tests;
mod types;
pub use types::*;

pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    connection: String,
    request: DdlExportRequest,
) -> Result<DdlExportArtifact, CatalogError> {
    request.validate()?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| Box::pin(reader::read(client, timeout, connection, request)),
    )
    .await
}
