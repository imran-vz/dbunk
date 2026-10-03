use super::*;
use futures_util::TryStreamExt;
use tokio_postgres::{types::FromSql, Client, Row};
pub(super) fn sqlstate(code: Option<&str>) -> CatalogError {
    if code == Some("42501") {
        CatalogError::DdlExportPermission
    } else {
        CatalogError::Database
    }
}
fn db(error: tokio_postgres::Error) -> CatalogError {
    sqlstate(error.code().map(|state| state.code()))
}
fn get<'a, T: FromSql<'a>>(row: &'a Row, key: &str) -> Result<T, CatalogError> {
    row.try_get(key).map_err(|_| CatalogError::InvalidResponse)
}
fn oid(row: &Row, key: &str) -> Result<u32, CatalogError> {
    u32::try_from(get::<i64>(row, key)?)
        .ok()
        .filter(|id| *id != 0)
        .ok_or(CatalogError::InvalidResponse)
}
fn name(row: &Row, key: &str) -> Result<String, CatalogError> {
    let value: Option<&str> = get(row, key)?;
    value
        .filter(|value| bounds::name(value))
        .map(str::to_owned)
        .ok_or(CatalogError::DdlExportLimit)
}
fn kind(row: &Row) -> Result<DdlExportRelationKind, CatalogError> {
    match get::<&str>(row, "kind")? {
        "r" => Ok(DdlExportRelationKind::Table),
        "p" => Ok(DdlExportRelationKind::PartitionedTable),
        "v" => Ok(DdlExportRelationKind::View),
        "m" => Ok(DdlExportRelationKind::MaterializedView),
        "f" => Ok(DdlExportRelationKind::ForeignTable),
        _ => Err(CatalogError::UnsupportedObjectKind),
    }
}
fn reference(relation: &DdlExportRelation) -> PgObjectRef {
    PgObjectRef {
        kind: match relation.kind {
            DdlExportRelationKind::Table | DdlExportRelationKind::PartitionedTable => {
                PgObjectKind::Table
            }
            DdlExportRelationKind::View => PgObjectKind::View,
            DdlExportRelationKind::MaterializedView => PgObjectKind::MaterializedView,
            DdlExportRelationKind::ForeignTable => PgObjectKind::ForeignTable,
        },
        schema: Some(relation.schema.clone()),
        name: relation.name.clone(),
        identity_args: None,
    }
}
const HEADER:&str="SELECT (SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database())AS oid,CASE WHEN octet_length(convert_to(current_database(),'UTF8'))<=63 THEN current_database()END AS name,pg_backend_pid()AS pid";
const SCHEMA:&str="SELECT oid::bigint AS oid,CASE WHEN octet_length(convert_to(nspname::text,'UTF8'))<=63 THEN nspname::text END AS name FROM pg_catalog.pg_namespace WHERE nspname=$1";
const RELATIONS: &str = r#"SELECT c.oid::bigint AS oid,n.oid::bigint AS schema_oid,c.relkind::text AS kind,
 CASE WHEN octet_length(convert_to(n.nspname::text,'UTF8'))<=63 THEN n.nspname::text END AS schema,
 CASE WHEN octet_length(convert_to(c.relname::text,'UTF8'))<=63 THEN c.relname::text END AS name
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE ($1::bigint IS NULL OR n.oid::bigint=$1) AND ($2::text IS NULL OR c.relname=$2)
 AND ($2::text IS NOT NULL OR c.relkind IN('r','p','v','m','f'))
 AND ($1::bigint IS NOT NULL OR (n.nspname NOT IN('pg_catalog','information_schema')AND n.nspname NOT LIKE 'pg_toast%'))
 ORDER BY n.nspname::text COLLATE "C",c.relname::text COLLATE "C" LIMIT 1025"#;

pub(super) async fn read(
    client: &Client,
    timeout: Option<u32>,
    connection: String,
    request: DdlExportRequest,
) -> Result<DdlExportArtifact, CatalogError> {
    let started = chrono::Utc::now().to_rfc3339();
    begin_snapshot(client, timeout).await?;
    let header = client.query_one(HEADER, &[]).await.map_err(db)?;
    let database_oid = oid(&header, "oid")?;
    if request
        .expected_database_oid
        .is_some_and(|expected| expected != database_oid)
    {
        return Err(CatalogError::DdlExportIdentityChanged);
    }
    let schema_name = match &request.scope {
        DdlExportScope::Database => None,
        DdlExportScope::Schema { name, .. } => Some(name),
        DdlExportScope::Relation { schema, .. } => Some(schema),
    };
    let mut schemas = Vec::new();
    let schema_oid = if let Some(schema) = schema_name {
        let row = client
            .query_opt(SCHEMA, &[schema])
            .await
            .map_err(db)?
            .ok_or(CatalogError::ObjectNotFound)?;
        let actual = oid(&row, "oid")?;
        if matches!(&request.scope,DdlExportScope::Schema{expected_oid:Some(expected),..} if *expected!=actual)
        {
            return Err(CatalogError::DdlExportIdentityChanged);
        }
        schemas.push(DdlExportSchema {
            oid: actual,
            name: name(&row, "name")?,
            declared: matches!(request.scope, DdlExportScope::Schema { .. }),
        });
        Some(i64::from(actual))
    } else {
        None
    };
    let relation_name = match &request.scope {
        DdlExportScope::Relation { name, .. } => Some(name.as_str()),
        _ => None,
    };
    let stream = client
        .query_raw(
            RELATIONS,
            [
                &schema_oid as &(dyn tokio_postgres::types::ToSql + Sync),
                &relation_name,
            ],
        )
        .await
        .map_err(db)?;
    tokio::pin!(stream);
    let mut relations = Vec::new();
    while let Some(row) = stream.try_next().await.map_err(db)? {
        if relations.len() == MAX_DDL_EXPORT_RELATIONS {
            return Err(CatalogError::DdlExportLimit);
        }
        let relation = DdlExportRelation {
            identity: DdlExportIdentity {
                database_oid,
                relation_oid: oid(&row, "oid")?,
            },
            schema_oid: oid(&row, "schema_oid")?,
            schema: name(&row, "schema")?,
            name: name(&row, "name")?,
            kind: kind(&row)?,
            sql_start: 0,
            sql_end: 0,
        };
        if let DdlExportScope::Relation {
            expected: Some(expected),
            ..
        } = &request.scope
        {
            if *expected != relation.identity {
                return Err(CatalogError::DdlExportIdentityChanged);
            }
        }
        if matches!(request.scope, DdlExportScope::Database)
            && schemas
                .last()
                .is_none_or(|schema| schema.oid != relation.schema_oid)
        {
            if schemas.len() == MAX_DDL_EXPORT_SCHEMAS {
                return Err(CatalogError::DdlExportLimit);
            }
            schemas.push(DdlExportSchema {
                oid: relation.schema_oid,
                name: relation.schema.clone(),
                declared: relation.schema != "public",
            });
        }
        relations.push(relation);
    }
    if matches!(request.scope, DdlExportScope::Relation { .. }) && relations.len() != 1 {
        return Err(CatalogError::ObjectNotFound);
    }
    let mut sql = render::Sql::new()?;
    for schema in &schemas {
        sql.schema(schema)?;
    }
    // Reconstruct one bounded description at a time. Do not retain a Vec of
    // descriptions or clone definition SQL. Existing description guards cap
    // component JSON at 8 MiB/4096 rows and reconstructed SQL at 1 MiB per relation;
    // artifact assembly independently admits SQL and JSON escaping before copy.
    for relation in &mut relations {
        let description = description::load(client, reference(relation)).await?;
        let definition = description
            .definition_sql
            .as_deref()
            .ok_or(CatalogError::InvalidResponse)?;
        if definition.is_empty() {
            return Err(CatalogError::InvalidResponse);
        }
        sql.relation(relation, definition)?;
    }
    client.batch_execute("COMMIT").await.map_err(db)?;
    let result = DdlExportArtifact {
        connection_id: connection,
        database: name(&header, "name")?,
        database_oid,
        reader_pid: get(&header, "pid")?,
        collected_start: started,
        collected_end: chrono::Utc::now().to_rfc3339(),
        request,
        schemas,
        relations,
        omissions: types::OMISSIONS.to_vec(),
        sql: sql.finish(),
    };
    result
        .checked_heap_bytes()
        .ok_or(CatalogError::DdlExportLimit)?;
    Ok(result)
}
