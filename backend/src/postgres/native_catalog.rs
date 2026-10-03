//! Bounded catalog reads on one owned dedicated socket. SQL guards bound each
//! wire text value; streamed rows are charged before cloning into the catalog.
use super::connect_spec::ResolvedPostgresConnectSpec;
use super::dedicated::{self, DriverJoins, NoticeSink};
use super::objects::{
    PgCatalogEntry, PgCatalogTruncation, PgObjectCatalog, PgSchemaObjects, PgTypeClass,
    CATALOG_KIND_CAP,
};
use futures_util::{future::BoxFuture, TryStreamExt};

pub(crate) mod admin;
pub(crate) mod completion;
pub(crate) mod dependencies;
pub(crate) mod description;
pub(crate) mod foreign_keys;
pub(crate) mod server_details;
pub(crate) mod table_structure;
use serde::Serialize;
use std::{collections::BTreeMap, io, time::Duration};
use tokio::sync::watch;
use tokio_postgres::{types::ToSql, Client, Row};

pub const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CATALOG_NODES: usize = 10_000;
const MAX_TEXT_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogError {
    Cancelled,
    Timeout,
    Connection,
    UnsupportedEngine,
    Database,
    TextLimit,
    NodeLimit,
    ByteLimit,
    InvalidResponse,
    UnsupportedObjectKind,
    InvalidReference,
    ObjectNotFound,
    DescriptionLimit,
    ForeignKeyLimit,
    DropImpactLimit,
    AdminLimit,
    CompletionLimit,
    ServerDetailsLimit,
    StructureLimit,
    StructurePermission,
    StructureIdentityChanged,
    StructureVersion,
    OverviewLimit,
    OverviewIdentityChanged,
    OverviewPermission,
    DdlExportLimit,
    DdlExportIdentityChanged,
    DdlExportPermission,
    SchemaMapLimit,
    SchemaMapPermission,
    SchemaMapIdentityChanged,
    SchemaMapVersion,
    TableExportLimit,
    TableExportIdentityChanged,
    TableExportPermission,
}
impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Catalog read cancelled",
            Self::Timeout => "Catalog read timed out",
            Self::Connection => "Catalog connection unavailable",
            Self::UnsupportedEngine => "Catalog requires PostgreSQL",
            Self::Database => "PostgreSQL refused the catalog read",
            Self::TextLimit => "Catalog text exceeds 8 KiB; no complete catalog returned",
            Self::NodeLimit => {
                "Catalog exceeds 10,000 schemas/objects; no complete catalog returned"
            }
            Self::ByteLimit => "Catalog exceeds 8 MiB; no complete catalog returned",
            Self::InvalidResponse => "Object metadata response was inconsistent",
            Self::UnsupportedObjectKind => {
                "Native description is not available for this object kind"
            }
            Self::InvalidReference => "Object reference is invalid or exceeds 8 KiB per field",
            Self::ObjectNotFound => {
                "Object no longer exists at this exact schema/name/overload identity"
            }
            Self::ForeignKeyLimit=>"Foreign-key metadata exceeds 256 constraints, 4096 column pairs or 1 MiB; no partial result returned",
            Self::DropImpactLimit => "Drop-impact metadata exceeds 8 KiB per text field or 1 MiB encoded; no result returned",
            Self::DescriptionLimit => {
                "Object description exceeds its text or 8 MiB result limit; no description returned"
            }
            Self::CompletionLimit => "Completion metadata exceeds 4096 columns, 1 MiB, 63-byte names or 8 KiB types; no columns returned",
            Self::AdminLimit => "Administration snapshot exceeds its text or 1 MiB result limit; no snapshot returned",
            Self::ServerDetailsLimit => "Server details exceed their identity or 1 MiB bounds; previous capture remains available",
            Self::StructureLimit => "Table structure exceeds its text, component or 4 MiB limit; no partial capture returned",
            Self::StructurePermission => "PostgreSQL denied access to table structure metadata",
            Self::StructureIdentityChanged => "Table identity changed; explicitly inspect the new relation before continuing",
            Self::StructureVersion => "Table Structure requires PostgreSQL 13 or newer",
            Self::OverviewLimit => "Overview exceeds 256 rows per page or 1 MiB; previous capture retained",
            Self::OverviewIdentityChanged => "Overview target identity changed; refresh the target explicitly",
            Self::OverviewPermission => "PostgreSQL denied access to overview metadata",
            Self::DdlExportLimit => "DDL export exceeds relation, SQL or retained text bounds; no partial artifact returned",
            Self::DdlExportIdentityChanged => "DDL export target identity changed; explicitly inspect the replacement",
            Self::DdlExportPermission => "PostgreSQL denied DDL export metadata",
            Self::SchemaMapLimit => "Schema map exceeds complete graph or text bounds; no partial capture returned",
            Self::SchemaMapPermission => "PostgreSQL denied schema-map metadata",
            Self::SchemaMapIdentityChanged => "Schema-map target identity changed; explicitly inspect the replacement",
            Self::SchemaMapVersion => "Schema map requires PostgreSQL 13 or newer",
            Self::TableExportLimit => "Complete table export exceeds native row, cell, text or delivery bounds; no partial capture returned",
            Self::TableExportIdentityChanged => "Table export source identity changed; explicitly inspect the replacement",
            Self::TableExportPermission => "PostgreSQL denied access to table export rows or metadata",
        })
    }
}
impl std::error::Error for CatalogError {}

pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    timeout: Duration,
) -> Result<PgObjectCatalog, CatalogError> {
    owned_read(
        spec,
        drivers,
        cancellation,
        timeout,
        |client, configured| Box::pin(load(client, configured)),
    )
    .await
}

/// One document-owned lifecycle for catalog and bounded single-object metadata.
pub(super) async fn owned_read<T>(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    mut cancellation: watch::Receiver<u64>,
    timeout: Duration,
    operation: impl for<'a> FnOnce(&'a Client, Option<u32>) -> BoxFuture<'a, Result<T, CatalogError>>,
) -> Result<T, CatalogError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let connected = tokio::select! {
        biased;
        _ = cancellation.changed() => Err(CatalogError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
        result = dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers)) => result.map_err(|_| CatalogError::Connection),
    };
    let (result, cleanup_deadline) = match connected {
        Ok(connection) => {
            let result = tokio::select! {
                biased;
                _ = cancellation.changed() => Err(CatalogError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
                result = operation(&connection.client, spec.driver_options.statement_timeout_ms) => result,
            };
            let cleanup_deadline = cleanup_deadline(deadline);
            // These are metadata reads. Dropping the socket on any refusal is
            // safe; cancel its server statement first, using the verified TLS.
            if result.is_err() {
                let _ = tokio::time::timeout_at(
                    cleanup_deadline,
                    dedicated::cancel(connection.cancel.clone(), connection.tls.clone()),
                )
                .await;
            }
            // Never start a fresh cleanup deadline after cancellation/timeout.
            if tokio::time::timeout_at(cleanup_deadline, connection.close())
                .await
                .is_err()
            {
                drivers.abort_all();
            }
            (result, cleanup_deadline)
        }
        Err(error) => (Err(error), cleanup_deadline(deadline)),
    };
    // This barrier is joined, even if the caller drops its facade future. An
    // expired grace aborts first, then waits for actual driver termination.
    join_drivers(drivers, cleanup_deadline).await;
    if cancellation.has_changed().unwrap_or(true) {
        return Err(CatalogError::Cancelled);
    }
    result
}

// DataDocument retirement has a three-second grace/five-second hard gate.
// Cancel, graceful socket close and driver drain share one shorter deadline.
fn cleanup_deadline(operation_deadline: tokio::time::Instant) -> tokio::time::Instant {
    operation_deadline.min(tokio::time::Instant::now() + Duration::from_secs(1))
}
async fn join_drivers(drivers: &DriverJoins, deadline: tokio::time::Instant) {
    if tokio::time::timeout_at(deadline, drivers.drain())
        .await
        .is_err()
    {
        drivers.abort_all();
        drivers.drain().await;
    }
}

async fn load(
    client: &Client,
    configured_timeout: Option<u32>,
) -> Result<PgObjectCatalog, CatalogError> {
    begin_snapshot(client, configured_timeout).await?;
    let mut catalog = Builder::new();
    let kind_limit = (CATALOG_KIND_CAP + 1) as i64;
    let schemas = client
        .query_raw(SCHEMAS, [&kind_limit as &(dyn ToSql + Sync)])
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(schemas);
    while let Some(row) = schemas
        .try_next()
        .await
        .map_err(|_| CatalogError::Database)?
    {
        checked_row(&row)?;
        catalog.schema(text(&row, "name")?)?;
    }
    let names: Vec<_> = catalog.schemas.keys().cloned().collect();
    let rows_limit = MAX_CATALOG_NODES as i64 + 1;
    let objects = client
        .query_raw(
            OBJECTS,
            [&kind_limit as &(dyn ToSql + Sync), &names, &rows_limit],
        )
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(objects);
    while let Some(row) = objects
        .try_next()
        .await
        .map_err(|_| CatalogError::Database)?
    {
        checked_row(&row)?;
        let schema = optional_text(&row, "schema_name")?;
        let kind = text(&row, "kind")?;
        let class = match optional_text(&row, "type_class")? {
            None => None,
            Some("enum") => Some(PgTypeClass::Enum),
            Some("composite") => Some(PgTypeClass::Composite),
            Some("range") => Some(PgTypeClass::Range),
            Some("multirange") => Some(PgTypeClass::Multirange),
            _ => return Err(CatalogError::InvalidResponse),
        };
        catalog.entry(
            schema,
            kind,
            text(&row, "name")?,
            optional_text(&row, "identity_args")?,
            optional_text(&row, "comment")?,
            class,
        )?;
    }
    client
        .batch_execute("COMMIT")
        .await
        .map_err(|_| CatalogError::Database)?;
    catalog.finish()
}

pub(super) async fn begin_snapshot(
    client: &Client,
    configured_timeout: Option<u32>,
) -> Result<(), CatalogError> {
    // Bound unlimited/long statements without relaxing a stricter stored limit.
    let statement_ms = configured_timeout
        .filter(|ms| *ms > 0)
        .unwrap_or(10_000)
        .min(10_000);
    client.batch_execute(&format!("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; SET LOCAL statement_timeout = {statement_ms}; SET LOCAL lock_timeout = '2s'"))
        .await.map_err(|_| CatalogError::Database)?;
    Ok(())
}

fn text<'a>(row: &'a Row, field: &str) -> Result<&'a str, CatalogError> {
    row.try_get(field)
        .map_err(|_| CatalogError::InvalidResponse)
}
fn optional_text<'a>(row: &'a Row, field: &str) -> Result<Option<&'a str>, CatalogError> {
    row.try_get(field)
        .map_err(|_| CatalogError::InvalidResponse)
}
fn checked_row(row: &Row) -> Result<(), CatalogError> {
    if row
        .try_get::<_, bool>("oversized")
        .map_err(|_| CatalogError::InvalidResponse)?
    {
        Err(CatalogError::TextLimit)
    } else {
        Ok(())
    }
}

struct Builder {
    schemas: BTreeMap<String, PgSchemaObjects>,
    catalog: PgObjectCatalog,
    scanned: usize,
    bytes: usize,
}
impl Builder {
    fn new() -> Self {
        let catalog = PgObjectCatalog {
            schemas: Vec::new(),
            event_triggers: Vec::new(),
            roles: Vec::new(),
            tablespaces: Vec::new(),
            truncated: Vec::new(),
        };
        Self {
            schemas: BTreeMap::new(),
            bytes: json_size(&catalog).unwrap(),
            catalog,
            scanned: 0,
        }
    }
    fn scan(&mut self) -> Result<(), CatalogError> {
        self.scanned += 1;
        if self.scanned > MAX_CATALOG_NODES {
            Err(CatalogError::NodeLimit)
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, bytes: usize) -> Result<(), CatalogError> {
        let next = self
            .bytes
            .checked_add(bytes)
            .ok_or(CatalogError::ByteLimit)?;
        if next > MAX_CATALOG_BYTES {
            return Err(CatalogError::ByteLimit);
        }
        self.bytes = next;
        Ok(())
    }
    fn truncated(&mut self, schema: Option<&str>, kind: &str) -> Result<(), CatalogError> {
        if self
            .catalog
            .truncated
            .iter()
            .any(|item| item.schema.as_deref() == schema && item.kind == kind)
        {
            return Ok(());
        }
        // Charge the borrowed shape before cloning strings.
        #[derive(Serialize)]
        struct Marker<'a> {
            schema: Option<&'a str>,
            kind: &'a str,
        }
        self.charge(json_size(&Marker { schema, kind })? + 1)?;
        self.catalog.truncated.push(PgCatalogTruncation {
            schema: schema.map(str::to_owned),
            kind: kind.into(),
        });
        Ok(())
    }
    fn schema(&mut self, name: &str) -> Result<(), CatalogError> {
        self.scan()?;
        check_text(Some(name))?;
        if self.schemas.len() == CATALOG_KIND_CAP {
            return self.truncated(None, "schema");
        }
        if self.schemas.contains_key(name) {
            return Err(CatalogError::InvalidResponse);
        }
        // Names are capped before this single bounded allocation.
        let schema = PgSchemaObjects::empty(name.into());
        self.charge(json_size(&schema)? + 1)?;
        self.schemas.insert(name.into(), schema);
        Ok(())
    }
    fn entry(
        &mut self,
        schema: Option<&str>,
        kind: &str,
        name: &str,
        identity_args: Option<&str>,
        comment: Option<&str>,
        type_class: Option<PgTypeClass>,
    ) -> Result<(), CatalogError> {
        self.scan()?;
        for value in [schema, Some(kind), Some(name), identity_args, comment] {
            check_text(value)?;
        }
        if self.entries(schema, kind)?.len() == CATALOG_KIND_CAP {
            return self.truncated(schema, kind);
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry<'a> {
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            identity_args: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            comment: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            type_class: Option<PgTypeClass>,
        }
        self.charge(
            json_size(&Entry {
                name,
                identity_args,
                comment,
                type_class,
            })? + 1,
        )?;
        self.entries(schema, kind)?.push(PgCatalogEntry {
            name: name.into(),
            identity_args: identity_args.map(str::to_owned),
            comment: comment.map(str::to_owned),
            type_class,
        });
        Ok(())
    }
    fn entries(
        &mut self,
        schema: Option<&str>,
        kind: &str,
    ) -> Result<&mut Vec<PgCatalogEntry>, CatalogError> {
        if let Some(name) = schema {
            let schema = self
                .schemas
                .get_mut(name)
                .ok_or(CatalogError::InvalidResponse)?;
            Ok(match kind {
                "table" => &mut schema.tables,
                "view" => &mut schema.views,
                "materialized-view" => &mut schema.materialized_views,
                "foreign-table" => &mut schema.foreign_tables,
                "sequence" => &mut schema.sequences,
                "function" => &mut schema.functions,
                "procedure" => &mut schema.procedures,
                "aggregate" => &mut schema.aggregates,
                "type" => &mut schema.types,
                "domain" => &mut schema.domains,
                "extension" => &mut schema.extensions,
                _ => return Err(CatalogError::InvalidResponse),
            })
        } else {
            Ok(match kind {
                "event-trigger" => &mut self.catalog.event_triggers,
                "role" => &mut self.catalog.roles,
                "tablespace" => &mut self.catalog.tablespaces,
                _ => return Err(CatalogError::InvalidResponse),
            })
        }
    }
    fn finish(mut self) -> Result<PgObjectCatalog, CatalogError> {
        self.catalog.schemas = self.schemas.into_values().collect();
        if json_size(&self.catalog)? > MAX_CATALOG_BYTES {
            return Err(CatalogError::ByteLimit);
        }
        Ok(self.catalog)
    }
}
fn check_text(value: Option<&str>) -> Result<(), CatalogError> {
    if value.is_some_and(|value| value.len() > MAX_TEXT_BYTES) {
        Err(CatalogError::TextLimit)
    } else {
        Ok(())
    }
}
fn json_size(value: &impl Serialize) -> Result<usize, CatalogError> {
    struct Counter(usize);
    impl io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| io::Error::other("size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Counter(0);
    serde_json::to_writer(&mut writer, value).map_err(|_| CatalogError::ByteLimit)?;
    Ok(writer.0)
}

const SCHEMAS: &str = r#"
SELECT CASE WHEN octet_length(n.nspname::text) <= 8192 THEN n.nspname::text END AS name,
       octet_length(n.nspname::text) > 8192 AS oversized
FROM pg_namespace n
WHERE n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%' ESCAPE '\'
  AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_namespace'::regclass AND d.objid = n.oid AND d.deptype = 'e')
ORDER BY n.nspname LIMIT $1
"#;

// As in the baseline, schemas are capped first and bound into every scoped
// query. The outer LIMIT bounds the whole wire result in addition to per-kind
// sentinels. Oversized text becomes NULL plus a refusal bit, never a truncated
// overload identity or an apparently complete comment.
const OBJECTS: &str = r#"
WITH entries AS (
 SELECT n.nspname::text AS schema_name, c.relname::text AS name,
   CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'table' WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized-view' WHEN 'f' THEN 'foreign-table' ELSE 'sequence' END AS kind,
   NULL::text AS identity_args, obj_description(c.oid, 'pg_class') AS comment, NULL::text AS type_class
 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
 WHERE c.relkind IN ('r','p','v','m','f','S') AND n.nspname = ANY($2::text[])
   AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
 UNION ALL
 SELECT n.nspname::text, p.proname::text, CASE p.prokind WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure' ELSE 'aggregate' END,
   pg_get_function_identity_arguments(p.oid)::text, obj_description(p.oid, 'pg_proc'), NULL::text
 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
 WHERE p.prokind IN ('f','p','a') AND n.nspname = ANY($2::text[])
   AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
 UNION ALL
 SELECT n.nspname::text, t.typname::text, CASE t.typtype WHEN 'd' THEN 'domain' ELSE 'type' END,
   NULL::text, obj_description(t.oid, 'pg_type'),
   CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite' WHEN 'r' THEN 'range' WHEN 'm' THEN 'multirange' ELSE NULL END
 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace LEFT JOIN pg_class tr ON tr.oid = t.typrelid
 WHERE t.typtype IN ('c','e','r','m','d') AND n.nspname = ANY($2::text[]) AND (t.typtype <> 'c' OR tr.relkind = 'c')
   AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e')
 UNION ALL
 SELECT n.nspname::text, e.extname::text, 'extension', NULL::text, obj_description(e.oid, 'pg_extension'), NULL::text
 FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace WHERE n.nspname = ANY($2::text[])
 UNION ALL
 SELECT NULL::text, evtname::text, 'event-trigger', NULL::text, obj_description(oid, 'pg_event_trigger'), NULL::text FROM pg_event_trigger
 UNION ALL
 SELECT NULL::text, rolname::text, 'role', NULL::text, shobj_description(oid, 'pg_authid'), NULL::text FROM pg_roles
 UNION ALL
 SELECT NULL::text, spcname::text, 'tablespace', NULL::text, shobj_description(oid, 'pg_tablespace'), NULL::text FROM pg_tablespace
), ranked AS (
 SELECT *, row_number() OVER (PARTITION BY schema_name, kind ORDER BY name, identity_args) AS row_number FROM entries
)
SELECT CASE WHEN octet_length(schema_name) <= 8192 THEN schema_name END AS schema_name,
       CASE WHEN octet_length(name) <= 8192 THEN name END AS name,
       kind,
       CASE WHEN octet_length(identity_args) <= 8192 THEN identity_args END AS identity_args,
       CASE WHEN octet_length(comment) <= 8192 THEN comment END AS comment,
       type_class,
       (coalesce(octet_length(schema_name), 0) > 8192 OR octet_length(name) > 8192 OR coalesce(octet_length(identity_args), 0) > 8192 OR coalesce(octet_length(comment), 0) > 8192) AS oversized
FROM ranked WHERE row_number <= $1 ORDER BY schema_name NULLS FIRST, kind, name, identity_args LIMIT $3
"#;

#[cfg(test)]
mod tests;
