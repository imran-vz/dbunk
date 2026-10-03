use super::*;
use crate::backend::schema_alter::SchemaIdentity;
use crate::postgres::native_catalog::{self, CatalogError};
use tokio_postgres::Client;
// System schemas are never alterable here; the rest are ordinary namespaces.
const CAPTURE: &str = r#"
SELECT d.oid AS database_oid,n.oid AS schema_oid,n.nspname::text AS schema,
 n.xmin::text AS xmin,n.ctid::text AS ctid,
 n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\' AS supported,
 pg_catalog.octet_length(text.comment)>4096 AS oversized,
 CASE WHEN pg_catalog.octet_length(text.comment)<=4096 THEN text.comment END AS comment
FROM pg_catalog.pg_namespace n
JOIN pg_catalog.pg_database d ON d.datname=pg_catalog.current_database()
CROSS JOIN LATERAL (SELECT pg_catalog.obj_description(n.oid,'pg_namespace') AS comment) text
WHERE n.nspname=$1::text
"#;
// The 32-bit row xmin equals this transaction's xid only for a version it wrote.
const OWNED_VERSION: &str = "SELECT n.xmin::text=(pg_catalog.txid_current()%4294967296)::text FROM pg_catalog.pg_namespace n WHERE n.oid=$1::oid";
pub(super) async fn capture(
    client: &Client,
    request: &SchemaAlterRequest,
) -> Result<SchemaAlterDescription, Failure> {
    let row = client
        .query_opt(CAPTURE, &[&request.schema])
        .await
        .map_err(database_error)?
        .ok_or(Failure::TargetChanged)?;
    if !row.get::<_, bool>("supported") {
        return Err(Failure::UnsupportedTarget);
    }
    if row.get::<_, Option<bool>>("oversized") == Some(true) {
        return Err(Failure::Limit);
    }
    let identity = SchemaIdentity {
        database_oid: row.get("database_oid"),
        schema_oid: row.get("schema_oid"),
    };
    if request
        .expected
        .is_some_and(|expected| expected != identity)
    {
        return Err(Failure::TargetChanged);
    }
    let description = SchemaAlterDescription {
        identity,
        schema: row.get("schema"),
        namespace_xmin: row.get("xmin"),
        namespace_ctid: row.get("ctid"),
        comment: row.get("comment"),
    };
    description.checked_heap_bytes().ok_or(Failure::Limit)?;
    Ok(description)
}
pub(super) async fn owned_version(client: &Client, schema_oid: u32) -> Result<bool, Failure> {
    client
        .query_opt(OWNED_VERSION, &[&schema_oid])
        .await
        .map_err(database_error)?
        .map(|row| row.get::<_, bool>(0))
        .ok_or(Failure::TargetChanged)
}
pub(crate) async fn observe(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: SchemaAlterRequest,
) -> Result<SchemaAlterDescription, CatalogError> {
    native_catalog::owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        |client, _| {
            Box::pin(async move {
                client
                    .batch_execute("BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY")
                    .await
                    .map_err(|_| CatalogError::Database)?;
                let result = capture(client, &request)
                    .await
                    .map_err(|failure| match failure {
                        Failure::Limit => CatalogError::DescriptionLimit,
                        Failure::UnsupportedTarget => CatalogError::UnsupportedObjectKind,
                        Failure::TargetChanged => CatalogError::StructureIdentityChanged,
                        _ => CatalogError::Database,
                    })?;
                client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(|_| CatalogError::Database)?;
                Ok(result)
            })
        },
    )
    .await
}
