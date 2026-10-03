use super::*;
use crate::postgres::{
    native_catalog::CatalogError,
    objects::{PgObjectKind, PgObjectRef},
};
use tokio_postgres::Client;
// Every returned text field is guarded before it reaches the wire. This query
// addresses exactly one pg_class row; no arbitrary definitions or user rows.
const TARGET: &str = "SELECT d.oid AS database_oid, CASE WHEN octet_length(d.datname::text)<=63 THEN d.datname::text END AS database, n.oid AS namespace_oid, CASE WHEN octet_length(n.nspname::text)<=63 THEN n.nspname::text END AS schema, c.oid AS relation_oid, CASE WHEN octet_length(c.relname::text)<=63 THEN c.relname::text END AS name, c.relkind::text AS kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace JOIN pg_catalog.pg_database d ON d.datname=pg_catalog.current_database() WHERE n.nspname::text=$1 AND c.relname::text=$2 AND c.relkind IN ('r','p','m')";
pub(super) async fn load(
    client: &Client,
    schema: &str,
    name: &str,
) -> Result<Option<Target>, Failure> {
    let row = client
        .query_opt(TARGET, &[&schema, &name])
        .await
        .map_err(database_error)?;
    row.map(|row| {
        let kind: &str = row.try_get("kind").map_err(database_error)?;
        let target = Target {
            database_oid: row.try_get("database_oid").map_err(database_error)?,
            database: row.try_get("database").map_err(database_error)?,
            namespace_oid: row.try_get("namespace_oid").map_err(database_error)?,
            schema: row.try_get("schema").map_err(database_error)?,
            relation_oid: row.try_get("relation_oid").map_err(database_error)?,
            name: row.try_get("name").map_err(database_error)?,
            kind: match kind {
                "r" => Kind::Table,
                "p" => Kind::PartitionedTable,
                "m" => Kind::MaterializedView,
                _ => return Err(Failure::Connection),
            },
        };
        if !target.valid() {
            return Err(Failure::Connection);
        }
        Ok(target)
    })
    .transpose()
}
pub(crate) async fn observe(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    mut cancelled: watch::Receiver<u64>,
    reference: &PgObjectRef,
) -> Result<Target, CatalogError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let connected = tokio::select! {
        biased;
        _ = cancelled.changed() => Err(CatalogError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
        result = dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers)) => result.map_err(|_| CatalogError::Connection),
    };
    let (result, cleanup) = match connected {
        Ok(connection) => {
            let result = tokio::select! {
                biased;
                _ = cancelled.changed() => Err(CatalogError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
                result = load(&connection.client, reference.schema.as_deref().unwrap_or(""), &reference.name) => result.map_err(|_| CatalogError::Database).and_then(|value| value.ok_or(CatalogError::ObjectNotFound)),
            };
            let cleanup = deadline.min(Instant::now() + CLEANUP_GRACE);
            if result.is_err() {
                let _ = tokio::time::timeout_at(
                    cleanup,
                    dedicated::cancel(connection.cancel.clone(), connection.tls.clone()),
                )
                .await;
            }
            if tokio::time::timeout_at(cleanup, connection.close())
                .await
                .is_err()
            {
                drivers.abort_all();
            }
            (result, cleanup)
        }
        Err(error) => (Err(error), deadline.min(Instant::now() + CLEANUP_GRACE)),
    };
    join(drivers, cleanup).await;
    if cancelled.has_changed().unwrap_or(true) {
        return Err(CatalogError::Cancelled);
    }
    let target = result?;
    if (reference.kind == PgObjectKind::MaterializedView)
        != (target.kind() == Kind::MaterializedView)
    {
        return Err(CatalogError::ObjectNotFound);
    }
    Ok(target)
}
