//! Bounded explicit overview reads. Each page owns a fresh read-only capture;
//! filesystem sizes and statistics are observations, not an atomic database truth.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::DriverJoins,
    native_catalog::{begin_snapshot, owned_read, CatalogError},
};
use std::time::Duration;
use tokio::sync::watch;
mod bounds;
mod queries;
mod reader;
#[cfg(test)]
mod tests;
mod types;
pub use types::*;

pub(crate) async fn database(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
) -> Result<DatabaseOverviewSnapshot, CatalogError> {
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        |client, timeout| Box::pin(reader::database(client, timeout)),
    )
    .await
}

pub(crate) async fn relations(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: RelationStatsRequest,
    connection: String,
    document: String,
) -> Result<RelationStatsSnapshot, CatalogError> {
    request.validate()?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| {
            Box::pin(reader::relations(
                client, timeout, request, connection, document,
            ))
        },
    )
    .await
}
pub(crate) async fn overview(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: RelationStatsRequest,
    connection: String,
    document: String,
) -> Result<OverviewSnapshot, CatalogError> {
    request.validate()?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| {
            Box::pin(reader::overview(
                client, timeout, request, connection, document,
            ))
        },
    )
    .await
}
