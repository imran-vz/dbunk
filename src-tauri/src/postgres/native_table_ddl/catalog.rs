use super::*;
use crate::postgres::native_catalog::{self, CatalogError};
use tokio_postgres::Client;
const CAPTURE: &str = r#"
SELECT d.oid AS database_oid,n.oid AS schema_oid,c.oid AS relation_oid,
 n.nspname::text AS schema,n.xmin::text AS xmin,n.ctid::text AS ctid,c.relname::text AS name,
 a.attnum,a.attname::text AS column_name,
 c.relkind='r' AND c.relpersistence IN ('p','u') AND NOT c.relispartition
 AND n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
 AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhrelid=c.oid OR i.inhparent=c.oid) AS supported,
 pg_catalog.octet_length(text.comment)>4096 AS oversized,
 CASE WHEN pg_catalog.octet_length(text.comment)<=4096 THEN text.comment END AS comment
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
JOIN pg_catalog.pg_database d ON d.datname=pg_catalog.current_database()
LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid AND a.attname=$3::text AND a.attnum>0 AND NOT a.attisdropped
CROSS JOIN LATERAL (SELECT CASE WHEN $3::text IS NULL THEN pg_catalog.obj_description(c.oid,'pg_class') ELSE pg_catalog.col_description(c.oid,a.attnum) END AS comment) text
WHERE n.nspname=$1::text AND c.relname=$2::text
"#;
pub(super) async fn capture(
    client: &Client,
    request: &TableDdlRequest,
) -> Result<TableDdlDescription, Failure> {
    let row = client
        .query_opt(CAPTURE, &[&request.schema, &request.table, &request.column])
        .await
        .map_err(database_error)?
        .ok_or(Failure::TargetChanged)?;
    if !row.get::<_, bool>("supported") {
        return Err(Failure::UnsupportedTarget);
    }
    if row.get::<_, Option<bool>>("oversized") == Some(true) {
        return Err(Failure::Limit);
    }
    let identity = TableIdentity {
        database_oid: row.get("database_oid"),
        relation_oid: row.get("relation_oid"),
    };
    if request
        .expected
        .is_some_and(|expected| expected != identity)
    {
        return Err(Failure::TargetChanged);
    }
    let column = match (&request.column, row.get::<_, Option<i16>>("attnum")) {
        (Some(_), Some(attnum)) => Some(TableDdlColumn {
            attnum,
            name: row.get("column_name"),
        }),
        (None, _) => None,
        _ => return Err(Failure::TargetChanged),
    };
    let description = TableDdlDescription {
        identity,
        schema_oid: row.get("schema_oid"),
        schema: row.get("schema"),
        namespace_xmin: row.get("xmin"),
        namespace_ctid: row.get("ctid"),
        table: row.get("name"),
        column,
        comment: row.get("comment"),
    };
    description.checked_heap_bytes().ok_or(Failure::Limit)?;
    Ok(description)
}
pub(crate) async fn observe(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: TableDdlRequest,
) -> Result<TableDdlDescription, CatalogError> {
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
pub(super) async fn lock_held(client: &Client, identity: TableIdentity) -> Result<bool, Failure> {
    client.query_one("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_locks WHERE locktype='relation' AND pid=pg_catalog.pg_backend_pid() AND database=$1::oid AND relation=$2::oid AND mode='AccessExclusiveLock' AND granted)", &[&identity.database_oid, &identity.relation_oid]).await.map(|r| r.get(0)).map_err(database_error)
}
