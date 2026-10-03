//! One complete, bounded PostgreSQL relationship capture. All graph selection,
//! columns, constraints, unique keys and triggers share one read-only snapshot.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::DriverJoins,
    native_catalog::{begin_snapshot, owned_read, CatalogError},
};
use std::time::Duration;
use tokio::sync::watch;
mod bounds;
mod classify;
mod queries;
mod reader;
mod types;
pub use types::*;
#[cfg(test)]
mod tests;
pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: SchemaMapRequest,
) -> Result<SchemaMapSnapshot, CatalogError> {
    request.validate()?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| Box::pin(reader::read(client, timeout, request)),
    )
    .await
}
