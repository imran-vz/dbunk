//! One owned read-only capture; all cells share one SELECT snapshot. Row/cell guards bound protocol values;
//! server type-output execution and filesystem/DNS work are not RSS guarantees.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::DriverJoins,
    native_catalog::{begin_snapshot, owned_read, CatalogError},
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
mod bounds;
mod reader;
mod sql;
#[cfg(test)]
mod tests;
mod types;
pub use types::*;
pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    connection: String,
    request: TableExportRequest,
) -> Result<TableExportCapture, CatalogError> {
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
