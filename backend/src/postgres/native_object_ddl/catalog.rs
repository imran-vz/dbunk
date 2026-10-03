//! Exact claim capture and post-statement verification. All identity
//! components are bound values; catalog row versions detect ALTER/recreate.
use super::*;
use crate::backend::object_ddl::{
    ObjectAddress, PgDropImpact, MAX_OBJECT_DDL_IMPACT_BYTES, MAX_OBJECT_DDL_OPERATIONS,
};
use crate::postgres::native_catalog::{self, dependencies, CatalogError};
use crate::postgres::objects::{PgObjectKind, PgObjectRef};
use std::sync::LazyLock;
use tokio_postgres::Client;

/// Reuses the drop-impact resolver: exact kind, schema, name and routine
/// signature, at most one row, plus the catalog row version.
static EXISTING: LazyLock<String> = LazyLock::new(|| {
    format!(
        r#"SELECT a.class_id, a.object_id,
 CASE a.class_id
  WHEN 'pg_catalog.pg_class'::regclass::oid::bigint THEN (SELECT x.xmin::text FROM pg_catalog.pg_class x WHERE x.oid=a.object_id::oid)
  WHEN 'pg_catalog.pg_proc'::regclass::oid::bigint THEN (SELECT x.xmin::text FROM pg_catalog.pg_proc x WHERE x.oid=a.object_id::oid)
  WHEN 'pg_catalog.pg_type'::regclass::oid::bigint THEN (SELECT x.xmin::text FROM pg_catalog.pg_type x WHERE x.oid=a.object_id::oid)
  WHEN 'pg_catalog.pg_namespace'::regclass::oid::bigint THEN (SELECT x.xmin::text FROM pg_catalog.pg_namespace x WHERE x.oid=a.object_id::oid)
 END AS version
FROM ({}) a"#,
        dependencies::RESOLVE
    )
});
const SCHEMA: &str = "SELECT n.oid, n.xmin::text AS version, 'pg_catalog.pg_namespace'::regclass::oid AS class FROM pg_catalog.pg_namespace n WHERE n.nspname=$1::text";
/// A view or materialized view also owns a row type with its name, so a type
/// of the same name blocks creation as surely as a relation does.
const OCCUPANT: &str = r#"
SELECT (SELECT c.relkind::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
  WHERE n.nspname=$1::text AND c.relname=$2::text) AS relkind,
 EXISTS(SELECT 1 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace
  WHERE n.nspname=$1::text AND t.typname=$2::text) AS type_exists
"#;
const DATABASE: &str =
    "SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()";
const ADDRESS_EXISTS: &str = r#"
SELECT CASE $1::oid
 WHEN 'pg_catalog.pg_class'::regclass::oid THEN EXISTS(SELECT 1 FROM pg_catalog.pg_class x WHERE x.oid=$2::oid)
 WHEN 'pg_catalog.pg_proc'::regclass::oid THEN EXISTS(SELECT 1 FROM pg_catalog.pg_proc x WHERE x.oid=$2::oid)
 WHEN 'pg_catalog.pg_type'::regclass::oid THEN EXISTS(SELECT 1 FROM pg_catalog.pg_type x WHERE x.oid=$2::oid)
 WHEN 'pg_catalog.pg_namespace'::regclass::oid THEN EXISTS(SELECT 1 FROM pg_catalog.pg_namespace x WHERE x.oid=$2::oid)
END
"#;
/// DROP and CREATE OR REPLACE lock the object they resolved until COMMIT:
/// relations as `relation` locks, other objects as `object` locks. Holding the
/// observed address proves this transaction's statement targeted that OID.
const LOCKED: &str = r#"
SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_locks l
 WHERE l.pid=pg_catalog.pg_backend_pid() AND l.granted AND l.mode='AccessExclusiveLock'
 AND l.database=(SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database())
 AND CASE WHEN $1::oid='pg_catalog.pg_class'::regclass::oid
  THEN l.locktype='relation' AND l.relation=$2::oid
  ELSE l.locktype='object' AND l.classid=$1::oid AND l.objid=$2::oid AND l.objsubid=0 END)
"#;
const CREATED_RELATION: &str = "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c WHERE c.relnamespace=$1::oid AND c.relname=$2::text AND c.relkind::text=$3::text)";
const CREATED_INDEX: &str = "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_index i ON i.indexrelid=c.oid WHERE c.relnamespace=$1::oid AND c.relname=$2::text AND i.indrelid=$3::oid)";
const ENUM_LABEL: &str = "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_enum e WHERE e.enumtypid=$1::oid AND e.enumlabel=$2::text)";
const INVALID_INDEX: &str = "SELECT NOT i.indisvalid FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class c ON c.oid=i.indexrelid WHERE c.relnamespace=$1::oid AND c.relname=$2::text";

fn oid(value: i64) -> Result<u32, Failure> {
    u32::try_from(value)
        .ok()
        .filter(|oid| *oid != 0)
        .ok_or(Failure::UnsupportedTarget)
}
fn version(value: Option<String>) -> Result<String, Failure> {
    value
        .filter(|v| !v.is_empty() && v.len() <= 10 && v.bytes().all(|b| b.is_ascii_digit()))
        .ok_or(Failure::TargetChanged)
}

pub(super) async fn database_oid(client: &Client) -> Result<u32, Failure> {
    client
        .query_one(DATABASE, &[])
        .await
        .map_err(failure)?
        .try_get::<_, u32>(0)
        .map_err(|_| Failure::UnsupportedTarget)
}

async fn existing(client: &Client, reference: &PgObjectRef) -> Result<ObjectDdlClaim, Failure> {
    let rows = client
        .query(
            EXISTING.as_str(),
            &[
                &dependencies::resolve_kind(reference.kind),
                &reference.schema,
                &reference.name,
                &reference.identity_args,
            ],
        )
        .await
        .map_err(failure)?;
    let row = match rows.as_slice() {
        [] => return Err(Failure::TargetChanged),
        [row] => row,
        _ => return Err(Failure::UnsupportedTarget),
    };
    let class_oid = oid(row
        .try_get("class_id")
        .map_err(|_| Failure::UnsupportedTarget)?)?;
    let object_oid = oid(row
        .try_get("object_id")
        .map_err(|_| Failure::UnsupportedTarget)?)?;
    let row_version: Option<String> = row.try_get("version").map_err(|_| Failure::Limit)?;
    Ok(ObjectDdlClaim::Existing {
        reference: reference.clone(),
        address: ObjectAddress {
            class_oid,
            object_oid,
            row_version: version(row_version)?,
        },
    })
}

pub(super) async fn capture(client: &Client, spec: &ClaimSpec) -> Result<ObjectDdlClaim, Failure> {
    match spec {
        ClaimSpec::Existing(reference) => existing(client, reference).await,
        ClaimSpec::Schema(name) => {
            let row = client
                .query_opt(SCHEMA, &[name])
                .await
                .map_err(failure)?
                .ok_or(Failure::TargetChanged)?;
            Ok(ObjectDdlClaim::Schema {
                name: name.clone(),
                address: ObjectAddress {
                    class_oid: row.try_get(2).map_err(|_| Failure::UnsupportedTarget)?,
                    object_oid: row.try_get(0).map_err(|_| Failure::UnsupportedTarget)?,
                    row_version: version(row.try_get(1).map_err(|_| Failure::Limit)?)?,
                },
            })
        }
        ClaimSpec::Absent { schema, name } | ClaimSpec::ViewOrAbsent { schema, name } => {
            let row = client
                .query_one(OCCUPANT, &[schema, name])
                .await
                .map_err(failure)?;
            let relkind: Option<String> = row.try_get(0).map_err(|_| Failure::Limit)?;
            let type_exists: bool = row.try_get(1).map_err(|_| Failure::Limit)?;
            match (spec, relkind.as_deref(), type_exists) {
                (_, None, false) => Ok(ObjectDdlClaim::Absent {
                    schema: schema.clone(),
                    name: name.clone(),
                }),
                (ClaimSpec::ViewOrAbsent { .. }, Some("v"), _) => {
                    existing(
                        client,
                        &PgObjectRef {
                            kind: PgObjectKind::View,
                            schema: Some(schema.clone()),
                            name: name.clone(),
                            identity_args: None,
                        },
                    )
                    .await
                }
                _ => Err(Failure::TargetChanged),
            }
        }
    }
}

async fn flag(
    client: &Client,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<bool, Failure> {
    client
        .query_one(sql, params)
        .await
        .map_err(failure)?
        .try_get::<_, Option<bool>>(0)
        .map_err(|_| Failure::UnsupportedTarget)
        .map(|value| value.unwrap_or(false))
}

fn schema_oid(claims: &[ObjectDdlClaim]) -> Option<u32> {
    claims.iter().find_map(|claim| match claim {
        ObjectDdlClaim::Schema { address, .. } => Some(address.object_oid),
        _ => None,
    })
}

/// True only when the statement's exact effect is visible in this transaction.
pub(super) async fn verify(
    client: &Client,
    operation: &ObjectDdlOperation,
    claims: &[ObjectDdlClaim],
) -> Result<bool, Failure> {
    match (operation, claims) {
        (ObjectDdlOperation::DropObject { .. }, [ObjectDdlClaim::Existing { address, .. }]) => {
            Ok(!flag(
                client,
                ADDRESS_EXISTS,
                &[&address.class_oid, &address.object_oid],
            )
            .await?
                && flag(client, LOCKED, &[&address.class_oid, &address.object_oid]).await?)
        }
        (
            ObjectDdlOperation::CreateView { name, .. },
            [ObjectDdlClaim::Schema { .. }, ObjectDdlClaim::Existing { address, .. }],
        ) => Ok(flag(
            client,
            ADDRESS_EXISTS,
            &[&address.class_oid, &address.object_oid],
        )
        .await?
            && flag(client, LOCKED, &[&address.class_oid, &address.object_oid]).await?
            && created(client, claims, name, "v").await?),
        (ObjectDdlOperation::CreateView { name, .. }, [_, ObjectDdlClaim::Absent { .. }]) => {
            created(client, claims, name, "v").await
        }
        (
            ObjectDdlOperation::CreateMaterializedView { name, .. },
            [_, ObjectDdlClaim::Absent { .. }],
        ) => created(client, claims, name, "m").await,
        (
            ObjectDdlOperation::CreateIndex { name, .. },
            [ObjectDdlClaim::Schema {
                address: schema, ..
            }, ObjectDdlClaim::Existing { address: table, .. }, ObjectDdlClaim::Absent { .. }],
        ) => {
            flag(
                client,
                CREATED_INDEX,
                &[&schema.object_oid, name, &table.object_oid],
            )
            .await
        }
        (
            ObjectDdlOperation::AddEnumValue { value, .. },
            [ObjectDdlClaim::Existing { address, .. }],
        ) => flag(client, ENUM_LABEL, &[&address.object_oid, value]).await,
        _ => Err(Failure::UnsupportedTarget),
    }
}

async fn created(
    client: &Client,
    claims: &[ObjectDdlClaim],
    name: &String,
    relkind: &str,
) -> Result<bool, Failure> {
    let schema = schema_oid(claims).ok_or(Failure::UnsupportedTarget)?;
    flag(client, CREATED_RELATION, &[&schema, name, &relkind]).await
}

/// Only a failed concurrent index build leaves a named catalog residue.
pub(super) async fn residue(
    client: &Client,
    operation: &ObjectDdlOperation,
    claims: &[ObjectDdlClaim],
) -> Option<ObjectDdlResidue> {
    let ObjectDdlOperation::CreateIndex {
        schema,
        name,
        concurrently: true,
        ..
    } = operation
    else {
        return None;
    };
    let Some(namespace) = schema_oid(claims) else {
        return Some(ObjectDdlResidue::Unverified);
    };
    match client.query_opt(INVALID_INDEX, &[&namespace, name]).await {
        Ok(Some(row)) => match row.try_get::<_, bool>(0) {
            Ok(true) => Some(ObjectDdlResidue::InvalidIndex {
                schema: schema.clone(),
                name: name.clone(),
            }),
            Ok(false) => None,
            Err(_) => Some(ObjectDdlResidue::Unverified),
        },
        Ok(None) => None,
        Err(_) => Some(ObjectDdlResidue::Unverified),
    }
}

fn catalog_error(failure: Failure) -> CatalogError {
    match failure {
        Failure::TargetChanged => CatalogError::StructureIdentityChanged,
        Failure::UnsupportedTarget => CatalogError::UnsupportedObjectKind,
        Failure::Limit => CatalogError::DescriptionLimit,
        _ => CatalogError::Database,
    }
}

/// One read-only snapshot: database, every claim, and each drop's impact.
pub(crate) async fn observe(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    operations: Vec<ObjectDdlOperation>,
) -> Result<(ObjectDdlDescription, Vec<Option<PgDropImpact>>), CatalogError> {
    if operations.is_empty() || operations.len() > MAX_OBJECT_DDL_OPERATIONS {
        return Err(CatalogError::InvalidReference);
    }
    native_catalog::owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| {
            Box::pin(async move {
                native_catalog::begin_snapshot(client, timeout).await?;
                let database_oid = database_oid(client).await.map_err(catalog_error)?;
                let mut claims = Vec::with_capacity(operations.len());
                let mut impacts = Vec::with_capacity(operations.len());
                let mut impact_bytes = 0usize;
                for operation in &operations {
                    let specs = operation.claim_specs();
                    let mut captured = Vec::with_capacity(specs.len());
                    for spec in &specs {
                        captured.push(capture(client, spec).await.map_err(catalog_error)?);
                    }
                    claims.push(captured);
                    impacts.push(match operation {
                        ObjectDdlOperation::DropObject { reference, .. } => {
                            let impact =
                                dependencies::impact_in_snapshot(client, reference).await?;
                            impact_bytes = impact_bytes
                                .saturating_add(crate::backend::schema_ddl::encoded_bytes(&impact));
                            if impact_bytes > MAX_OBJECT_DDL_IMPACT_BYTES {
                                return Err(CatalogError::DropImpactLimit);
                            }
                            Some(impact)
                        }
                        _ => None,
                    });
                }
                client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(|_| CatalogError::Database)?;
                Ok((
                    ObjectDdlDescription {
                        database_oid,
                        claims,
                    },
                    impacts,
                ))
            })
        },
    )
    .await
}
