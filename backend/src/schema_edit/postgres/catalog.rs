//! Catalog reads for capture and apply: the drift token, and the
//! in-transaction re-capture used to verify an applied plan. SQL is internal;
//! names and OIDs are always bound parameters.
use super::capture::{baseline_from_snapshot, drift_token, estimated_rows};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::dedicated::{self, DriverJoins, NoticeSink};
use crate::postgres::native_catalog::{
    table_structure::{
        self, StructureColumn, StructureConstraint, StructureForeignKey, StructureIndex,
        StructurePrimaryKey, StructureRelationKind, StructureRowSecurity, TableIdentity,
        TableStructureRequest, TableStructureSnapshot, MAX_STRUCTURE_BYTES,
    },
    CatalogError,
};
use crate::schema_edit::{DriftToken, SchemaBaseline, SchemaEditFailure, SchemaEditRequest, TableModel};
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::{sync::watch, time::Instant};
use tokio_postgres::Client;

/// One statement, so one snapshot: the relation OID and a digest of the
/// catalog row versions (xmin) of the table and everything the editor reads
/// from it. Any committed ALTER, constraint, index, comment, trigger, rule,
/// policy or inheritance change moves at least one xmin.
pub(super) const DRIFT: &str = r#"SELECT c.oid::bigint AS oid,c.reltuples::float8 AS reltuples,
pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(pg_catalog.concat_ws('|',c.xmin::text,
coalesce((SELECT pg_catalog.string_agg(a.attnum::text||':'||a.xmin::text,',' ORDER BY a.attnum) FROM pg_catalog.pg_attribute a WHERE a.attrelid=c.oid AND a.attnum>0),''),
coalesce((SELECT pg_catalog.string_agg(d.oid::text||':'||d.xmin::text,',' ORDER BY d.oid) FROM pg_catalog.pg_attrdef d WHERE d.adrelid=c.oid),''),
coalesce((SELECT pg_catalog.string_agg(k.oid::text||':'||k.xmin::text,',' ORDER BY k.oid) FROM pg_catalog.pg_constraint k WHERE k.conrelid=c.oid OR k.confrelid=c.oid),''),
coalesce((SELECT pg_catalog.string_agg(i.indexrelid::text||':'||i.xmin::text||':'||ic.xmin::text,',' ORDER BY i.indexrelid) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid=i.indexrelid WHERE i.indrelid=c.oid),''),
coalesce((SELECT pg_catalog.string_agg(s.objsubid::text||':'||s.xmin::text,',' ORDER BY s.objsubid) FROM pg_catalog.pg_description s WHERE s.objoid=c.oid AND s.classoid='pg_catalog.pg_class'::pg_catalog.regclass),''),
coalesce((SELECT pg_catalog.string_agg(t.oid::text||':'||t.xmin::text,',' ORDER BY t.oid) FROM pg_catalog.pg_trigger t WHERE t.tgrelid=c.oid),''),
coalesce((SELECT pg_catalog.string_agg(r.oid::text||':'||r.xmin::text,',' ORDER BY r.oid) FROM pg_catalog.pg_rewrite r WHERE r.ev_class=c.oid),''),
coalesce((SELECT pg_catalog.string_agg(p.oid::text||':'||p.xmin::text,',' ORDER BY p.oid) FROM pg_catalog.pg_policy p WHERE p.polrelid=c.oid),''),
coalesce((SELECT pg_catalog.string_agg(h.inhrelid::text||'>'||h.inhparent::text||':'||h.xmin::text,',' ORDER BY h.inhrelid,h.inhparent) FROM pg_catalog.pg_inherits h WHERE h.inhrelid=c.oid OR h.inhparent=c.oid),'')
),'UTF8')),'hex') AS digest
FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1::text AND c.relname=$2::text"#;

const RELATION: &str = "SELECT c.oid::bigint FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1::text AND c.relname=$2::text";

const HEADER: &str = "SELECT c.oid::bigint AS oid,c.relkind::text AS kind,pg_catalog.obj_description(c.oid,'pg_class') AS comment FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1::text AND c.relname=$2::text";

const CANONICAL_TYPES: &str = "SELECT pg_catalog.format_type(t.oid,t.typmod) FROM pg_catalog.unnest($1::oid[],$2::int4[]) WITH ORDINALITY AS t(oid,typmod,ord) ORDER BY t.ord";

// The same shapes as the Structure reader (`table_structure::queries`), one
// JSON document per row, read inside the apply transaction.
const COLUMNS: &str = r#"SELECT pg_catalog.to_jsonb(s)::text FROM (SELECT a.attnum::integer AS number,a.attname::text AS name,pg_catalog.format_type(a.atttypid,a.atttypmod) AS data_type,NOT a.attnotnull AS nullable,
pg_catalog.pg_get_expr(d.adbin,d.adrelid) AS default_expression,pg_catalog.col_description(a.attrelid,a.attnum) AS comment,
CASE a.attidentity WHEN '' THEN 'none' WHEN 'a' THEN 'always' WHEN 'd' THEN 'by_default' ELSE 'unknown' END AS identity,
CASE a.attgenerated WHEN '' THEN 'none' WHEN 's' THEN 'stored' WHEN 'v' THEN 'virtual' ELSE 'unknown' END AS generated,
(SELECT k.o::integer FROM pg_catalog.pg_constraint p CROSS JOIN LATERAL pg_catalog.unnest(p.conkey) WITH ORDINALITY k(n,o) WHERE p.conrelid=a.attrelid AND p.contype='p' AND k.n=a.attnum) AS primary_key_position,
cn.nspname::text AS collation_schema,co.collname::text AS collation_name
FROM pg_catalog.pg_attribute a LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum LEFT JOIN pg_catalog.pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_catalog.pg_namespace cn ON cn.oid=co.collnamespace
WHERE a.attrelid=$1::oid AND a.attnum>0 AND NOT a.attisdropped) s ORDER BY s.number"#;

const PRIMARY_KEY: &str = r#"SELECT pg_catalog.to_jsonb(s)::text FROM (SELECT c.oid::bigint AS oid,c.conname::text AS name,
coalesce((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('number',a.attnum::int,'name',a.attname::text) ORDER BY k.o) FROM pg_catalog.unnest(c.conkey) WITH ORDINALITY k(n,o) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.n),'[]'::jsonb) AS columns,
c.condeferrable AS deferrable,c.condeferred AS initially_deferred FROM pg_catalog.pg_constraint c WHERE c.conrelid=$1::oid AND c.contype='p') s ORDER BY s.oid"#;

const FOREIGN_KEYS: &str = r#"SELECT pg_catalog.to_jsonb(s)::text FROM (SELECT c.oid::bigint AS oid,c.conname::text AS name,c.conrelid::bigint AS source_oid,ns.nspname::text AS source_schema,src.relname::text AS source_table,c.confrelid::bigint AS target_oid,nt.nspname::text AS target_schema,t.relname::text AS target_table,
coalesce((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('source_number',a.attnum::int,'source',a.attname::text,'target_number',b.attnum::int,'target',b.attname::text) ORDER BY k.o) FROM pg_catalog.unnest(c.conkey,c.confkey) WITH ORDINALITY k(an,bn,o) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.an JOIN pg_catalog.pg_attribute b ON b.attrelid=c.confrelid AND b.attnum=k.bn),'[]'::jsonb) AS columns,
CASE c.confupdtype WHEN 'a' THEN 'no_action' WHEN 'r' THEN 'restrict' WHEN 'c' THEN 'cascade' WHEN 'n' THEN 'set_null' WHEN 'd' THEN 'set_default' ELSE 'unknown' END AS on_update,
CASE c.confdeltype WHEN 'a' THEN 'no_action' WHEN 'r' THEN 'restrict' WHEN 'c' THEN 'cascade' WHEN 'n' THEN 'set_null' WHEN 'd' THEN 'set_default' ELSE 'unknown' END AS on_delete,
CASE c.confmatchtype WHEN 'f' THEN 'FULL' WHEN 'p' THEN 'PARTIAL' WHEN 's' THEN 'SIMPLE' ELSE 'unknown' END AS match_type,c.condeferrable AS deferrable,c.condeferred AS initially_deferred,c.convalidated AS validated
FROM pg_catalog.pg_constraint c JOIN pg_catalog.pg_class src ON src.oid=c.conrelid JOIN pg_catalog.pg_namespace ns ON ns.oid=src.relnamespace JOIN pg_catalog.pg_class t ON t.oid=c.confrelid JOIN pg_catalog.pg_namespace nt ON nt.oid=t.relnamespace
WHERE c.contype='f' AND c.conrelid=$1::oid) s ORDER BY s.name,s.oid"#;

const INDEXES: &str = r#"SELECT pg_catalog.to_jsonb(s)::text FROM (SELECT i.indexrelid::bigint AS oid,c.relname::text AS name,am.amname::text AS method,i.indisunique AS unique,i.indisprimary AS primary,i.indisvalid AS valid,i.indisready AS ready,
coalesce((SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_object('position',k.o::int,'column_number',NULLIF(k.n,0)::int,'column_name',a.attname::text,'definition',pg_catalog.pg_get_indexdef(i.indexrelid,k.o::int,true),'included',k.o>i.indnkeyatts) ORDER BY k.o) FROM pg_catalog.unnest(i.indkey::smallint[]) WITH ORDINALITY k(n,o) LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.n),'[]'::jsonb) AS keys,
pg_catalog.pg_get_expr(i.indpred,i.indrelid) AS predicate,pg_catalog.pg_get_indexdef(i.indexrelid) AS definition,
(SELECT con.oid::bigint FROM pg_catalog.pg_constraint con WHERE con.conindid=i.indexrelid AND con.contype IN('p','u','x') LIMIT 1) AS constraint_oid
FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid JOIN pg_catalog.pg_am am ON am.oid=c.relam WHERE i.indrelid=$1::oid) s ORDER BY s.name,s.oid"#;

const CONSTRAINTS: &str = r#"SELECT pg_catalog.to_jsonb(s)::text FROM (SELECT c.oid::bigint AS oid,c.conname::text AS name,
CASE c.contype WHEN 'c' THEN 'check' WHEN 'u' THEN 'unique' WHEN 'x' THEN 'exclusion' WHEN 'p' THEN 'primary key' WHEN 'f' THEN 'foreign key' WHEN 'n' THEN 'not null' ELSE 'constraint' END AS kind,
pg_catalog.pg_get_constraintdef(c.oid,true) AS definition,c.condeferrable AS deferrable,c.condeferred AS initially_deferred,c.convalidated AS validated FROM pg_catalog.pg_constraint c WHERE c.conrelid=$1::oid) s ORDER BY s.name,s.oid"#;

fn message(text: &str) -> String {
    const MAX: usize = 2048;
    if text.len() <= MAX {
        return text.to_owned();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// A server error reply is definite; anything else (I/O, closed socket) is
/// not. `lock_not_available` is the lock timeout and `query_canceled` the
/// statement timeout (or a cancel this executor sent, which the caller
/// reports as Cancelled).
pub(super) fn failure(error: tokio_postgres::Error) -> SchemaEditFailure {
    match error.as_db_error() {
        Some(database) => match database.code().code() {
            "55P03" => SchemaEditFailure::LockTimeout,
            "57014" => SchemaEditFailure::Timeout,
            code => SchemaEditFailure::Database {
                code: Some(code)
                    .filter(|s| s.len() == 5 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
                    .map(str::to_owned),
                message: message(database.message()),
            },
        },
        None => SchemaEditFailure::Connection,
    }
}

fn unreadable() -> SchemaEditFailure {
    SchemaEditFailure::VerificationFailed {
        detail: "the catalog reply could not be read".into(),
    }
}

/// The current drift token of `schema.table`, or `None` when it is gone.
pub(super) async fn drift(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Option<(DriftToken, Option<u64>)>, tokio_postgres::Error> {
    let Some(row) = client.query_opt(DRIFT, &[&schema, &table]).await? else {
        return Ok(None);
    };
    let oid: i64 = row.try_get("oid")?;
    let reltuples: f64 = row.try_get("reltuples")?;
    let digest: String = row.try_get("digest")?;
    Ok(u32::try_from(oid)
        .ok()
        .map(|oid| (drift_token(oid, &digest), estimated_rows(reltuples))))
}

pub(super) async fn relation(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<Option<u32>, SchemaEditFailure> {
    let row = client
        .query_opt(RELATION, &[&schema, &table])
        .await
        .map_err(failure)?;
    match row {
        None => Ok(None),
        Some(row) => {
            let oid: i64 = row.try_get(0).map_err(|_| unreadable())?;
            Ok(u32::try_from(oid).ok())
        }
    }
}

async fn json_rows<T: DeserializeOwned>(
    client: &Client,
    sql: &str,
    oid: u32,
    budget: &mut usize,
) -> Result<Vec<T>, SchemaEditFailure> {
    let rows = client.query(sql, &[&oid]).await.map_err(failure)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let text: String = row.try_get(0).map_err(|_| unreadable())?;
        *budget = budget.saturating_add(text.len());
        if *budget > MAX_STRUCTURE_BYTES {
            return Err(SchemaEditFailure::VerificationFailed {
                detail: "the table structure is too large to verify".into(),
            });
        }
        out.push(serde_json::from_str(&text).map_err(|_| unreadable())?);
    }
    Ok(out)
}

/// The applied table, read inside the still-open apply transaction, plus
/// the server's rendering of each requested draft type. Types resolve with
/// the session's search_path (as the planned statements did); every read
/// then runs with `search_path = pg_catalog`, as the capture did, so
/// untouched types and expressions render byte for byte as captured.
pub(super) async fn observe(
    client: &Client,
    schema: &str,
    table: &str,
    types: &[String],
) -> Result<(TableModel, Vec<String>), SchemaEditFailure> {
    let mut resolved: Vec<(u32, i32)> = Vec::with_capacity(types.len());
    if !types.is_empty() {
        let select = types
            .iter()
            .enumerate()
            .map(|(index, data_type)| format!("NULL::{data_type} AS c{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let statement = client
            .prepare(&format!("SELECT {select}"))
            .await
            .map_err(failure)?;
        resolved = statement
            .columns()
            .iter()
            .map(|column| (column.type_().oid(), column.type_modifier()))
            .collect();
        if resolved.len() != types.len() {
            return Err(unreadable());
        }
    }
    client
        .batch_execute("SET LOCAL search_path TO pg_catalog")
        .await
        .map_err(failure)?;
    let mut canonical = Vec::with_capacity(resolved.len());
    if !resolved.is_empty() {
        let oids: Vec<u32> = resolved.iter().map(|(oid, _)| *oid).collect();
        let modifiers: Vec<i32> = resolved.iter().map(|(_, modifier)| *modifier).collect();
        for row in client
            .query(CANONICAL_TYPES, &[&oids, &modifiers])
            .await
            .map_err(failure)?
        {
            canonical.push(row.try_get::<_, String>(0).map_err(|_| unreadable())?);
        }
        if canonical.len() != types.len() {
            return Err(unreadable());
        }
    }

    let Some(header) = client
        .query_opt(HEADER, &[&schema, &table])
        .await
        .map_err(failure)?
    else {
        return Err(SchemaEditFailure::VerificationFailed {
            detail: format!("table {schema}.{table} is missing after the change"),
        });
    };
    let oid = header
        .try_get::<_, i64>("oid")
        .ok()
        .and_then(|oid| u32::try_from(oid).ok())
        .ok_or_else(unreadable)?;
    let kind: String = header.try_get("kind").map_err(|_| unreadable())?;
    let comment: Option<String> = header.try_get("comment").map_err(|_| unreadable())?;
    if kind != "r" {
        return Err(SchemaEditFailure::VerificationFailed {
            detail: format!("{schema}.{table} is no longer an ordinary table"),
        });
    }
    let mut budget = 0usize;
    let columns: Vec<StructureColumn> = json_rows(client, COLUMNS, oid, &mut budget).await?;
    let mut keys: Vec<StructurePrimaryKey> =
        json_rows(client, PRIMARY_KEY, oid, &mut budget).await?;
    let outbound: Vec<StructureForeignKey> =
        json_rows(client, FOREIGN_KEYS, oid, &mut budget).await?;
    let indexes: Vec<StructureIndex> = json_rows(client, INDEXES, oid, &mut budget).await?;
    let constraints: Vec<StructureConstraint> =
        json_rows(client, CONSTRAINTS, oid, &mut budget).await?;
    let snapshot = TableStructureSnapshot {
        identity: TableIdentity {
            database_oid: 0,
            relation_oid: oid,
        },
        schema: schema.to_owned(),
        table: table.to_owned(),
        kind: StructureRelationKind::Table,
        owner: String::new(),
        comment,
        server_version: 0,
        captured_at: String::new(),
        columns,
        primary_key: keys.pop(),
        outbound,
        inbound: Vec::new(),
        indexes,
        constraints,
        triggers: Vec::new(),
        row_security: StructureRowSecurity {
            enabled: false,
            forced: false,
        },
        policies: Vec::new(),
        privileges: Vec::new(),
        rules: Vec::new(),
        partition_key: None,
        is_partition: false,
        partition_bound: None,
        parents: Vec::new(),
        partitions: Vec::new(),
    };
    let table = baseline_from_snapshot(&snapshot, DriftToken(String::new()), None).table;
    Ok((table, canonical))
}

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_GRACE: Duration = Duration::from_secs(1);

/// The drift token on its own short connection, with the same cancel,
/// deadline and driver-join discipline as the other owned catalog reads.
async fn owned_drift(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    mut cancellation: watch::Receiver<u64>,
    request: &SchemaEditRequest,
) -> Result<Option<(DriftToken, Option<u64>)>, CatalogError> {
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let connected = tokio::select! {
        biased;
        _ = cancellation.changed() => Err(CatalogError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
        result = dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers)) => result.map_err(|_| CatalogError::Connection),
    };
    let cleanup = || deadline.min(Instant::now() + CLEANUP_GRACE);
    let result = match connected {
        Ok(connection) => {
            let result = tokio::select! {
                biased;
                _ = cancellation.changed() => Err(CatalogError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
                result = drift(&connection.client, &request.schema, &request.table) => result.map_err(|error| match error.code().map(|code| code.code()) {
                    Some("42501") => CatalogError::StructurePermission,
                    _ => CatalogError::Database,
                }),
            };
            let grace = cleanup();
            if result.is_err() {
                let _ = tokio::time::timeout_at(
                    grace,
                    dedicated::cancel(connection.cancel.clone(), connection.tls.clone()),
                )
                .await;
            }
            if tokio::time::timeout_at(grace, connection.close())
                .await
                .is_err()
            {
                drivers.abort_all();
            }
            result
        }
        Err(error) => Err(error),
    };
    if tokio::time::timeout_at(cleanup(), drivers.drain())
        .await
        .is_err()
    {
        drivers.abort_all();
        drivers.drain().await;
    }
    if cancellation.has_changed().unwrap_or(true) {
        return Err(CatalogError::Cancelled);
    }
    result
}

/// Captures one table: drift token, structure, drift token again. The
/// structure read is its own snapshot, so equal tokens on both sides prove
/// the structure belongs to that token. A change in between retries once.
pub(crate) async fn capture(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    request: SchemaEditRequest,
) -> Result<SchemaBaseline, CatalogError> {
    request
        .validate()
        .map_err(|_| CatalogError::InvalidReference)?;
    for _ in 0..2 {
        let before = owned_drift(spec, drivers, cancellation.clone(), &request)
            .await?
            .ok_or(CatalogError::ObjectNotFound)?;
        let snapshot = table_structure::read(
            spec,
            drivers,
            cancellation.clone(),
            TableStructureRequest {
                schema: request.schema.clone(),
                table: request.table.clone(),
                expected: None,
            },
        )
        .await?;
        let after = owned_drift(spec, drivers, cancellation.clone(), &request)
            .await?
            .ok_or(CatalogError::ObjectNotFound)?;
        let (token, rows) = before;
        if token == after.0
            && super::capture::drift_oid(&token) == Some(snapshot.identity.relation_oid)
        {
            return Ok(baseline_from_snapshot(&snapshot, token, rows));
        }
    }
    Err(CatalogError::StructureIdentityChanged)
}
