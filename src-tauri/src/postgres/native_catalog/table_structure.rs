//! One repeatable-read catalog capture. No user rows, FDW routes or DDL execute.
use super::*;
mod bounds;
mod queries;
mod reader;
#[cfg(test)]
mod tests;
mod types;
pub use types::*;
pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: TableStructureRequest,
) -> Result<TableStructureSnapshot, CatalogError> {
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
