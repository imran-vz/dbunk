use super::*;
use futures_util::TryStreamExt;
use serde::{de::DeserializeOwned, Deserialize};
use tokio_postgres::{types::ToSql, Client};
mod assemble;
pub(super) fn sqlstate(code: Option<&str>) -> CatalogError {
    match code {
        Some("42501") => CatalogError::SchemaMapPermission,
        _ => CatalogError::Database,
    }
}
fn database(error: tokio_postgres::Error) -> CatalogError {
    sqlstate(error.code().map(|c| c.code()))
}
#[derive(Default)]
pub(super) struct Budget {
    working: usize,
}
impl Budget {
    pub(super) fn admit(&mut self, bytes: usize, components: i32) -> Result<(), CatalogError> {
        let count = usize::try_from(components)
            .ok()
            .filter(|n| *n > 0)
            .ok_or(CatalogError::InvalidResponse)?;
        // Includes typed and wire text overlap, Vec growth/headers, attachment
        // into node vectors and bounded classification maps. Charge before JSON
        // decode. Names/comments/types are guarded before serialization in SQL.
        let added = bytes
            .checked_mul(2)
            .and_then(|n| n.checked_add(count.checked_mul(512)?))
            .ok_or(CatalogError::SchemaMapLimit)?;
        if added > MAX_SCHEMA_MAP_BYTES.saturating_sub(self.working) {
            return Err(CatalogError::SchemaMapLimit);
        }
        self.working += added;
        Ok(())
    }
}
async fn rows<T: DeserializeOwned>(
    client: &Client,
    source: &str,
    params: &[&(dyn ToSql + Sync)],
    names: &[&str],
    limit: usize,
    budget: &mut Budget,
) -> Result<Vec<T>, CatalogError> {
    let mut guards = String::from("false");
    for name in names {
        guards.push_str(&format!(
            " OR octet_length(convert_to(doc->>'{name}','UTF8'))>63"
        ));
    }
    // Query templates preguard variable-width text and arrays. The source limit
    // comes before to_jsonb; no whole-graph aggregate is ever serialized.
    // Explicit whole-row syntax avoids a `source` column shadowing the CTE row
    // in foreign-key queries and turning the JSON object into a scalar.
    let sql=format!("WITH source AS ({source} LIMIT {}), encoded AS (SELECT to_jsonb(source_row.*)-'components'-'array_oversized' AS doc,components,coalesce((to_jsonb(source_row.*)->>'array_oversized')::boolean,false)AS oversized FROM source AS source_row), checked AS (SELECT doc::text AS payload,components,(oversized OR coalesce(({guards}),false))AS oversized FROM encoded) SELECT CASE WHEN NOT oversized AND octet_length(convert_to(payload,'UTF8'))<=131072 THEN payload END AS payload,oversized OR octet_length(convert_to(payload,'UTF8'))>131072 AS oversized,components FROM checked",limit+1);
    let stream = client
        .query_raw(&sql, params.iter().copied())
        .await
        .map_err(database)?;
    tokio::pin!(stream);
    let mut result = Vec::new();
    while let Some(row) = stream.try_next().await.map_err(database)? {
        if result.len() == limit
            || row
                .try_get::<_, bool>("oversized")
                .map_err(|_| CatalogError::InvalidResponse)?
        {
            return Err(CatalogError::SchemaMapLimit);
        }
        let payload: &str = row
            .try_get("payload")
            .map_err(|_| CatalogError::SchemaMapLimit)?;
        budget.admit(
            payload.len(),
            row.try_get("components")
                .map_err(|_| CatalogError::InvalidResponse)?,
        )?;
        result.push(serde_json::from_str(payload).map_err(|_| CatalogError::InvalidResponse)?);
    }
    Ok(result)
}
#[derive(Deserialize)]
struct Header {
    database_oid: u32,
    database: String,
    server_version: u32,
    captured_at: String,
}
#[derive(Deserialize)]
struct Table {
    oid: u32,
    schema_oid: u32,
    schema: String,
    name: String,
    kind: SchemaMapTableKind,
    external: bool,
}
#[derive(Deserialize)]
struct Column {
    table_oid: u32,
    #[serde(flatten)]
    column: SchemaMapColumn,
}
#[derive(Deserialize)]
pub(super) struct Key {
    pub table_oid: u32,
    pub oid: u32,
    pub columns: Vec<i16>,
}
#[derive(Deserialize)]
struct ForeignKey {
    oid: u32,
    name: String,
    source: u32,
    target: u32,
    source_columns: Vec<i16>,
    target_columns: Vec<i16>,
    on_update: String,
    on_delete: String,
    match_type: String,
    validated: bool,
    deferrable: bool,
}
#[derive(Deserialize)]
struct Trigger {
    table_oid: u32,
    oid: u32,
    name: String,
    trigger_type: i16,
    enabled: String,
    columns: Vec<i16>,
    function_oid: u32,
    function_schema: String,
    function_name: String,
}
async fn resolve(
    client: &Client,
    request: &SchemaMapRequest,
    database_oid: u32,
) -> Result<(Option<u32>, Option<u32>), CatalogError> {
    if request
        .expected_database_oid
        .is_some_and(|expected| expected != database_oid)
    {
        return Err(CatalogError::SchemaMapIdentityChanged);
    }
    match &request.scope {
        SchemaMapScope::Database => Ok((None, None)),
        SchemaMapScope::Schema { name, expected_oid } => {
            let row = client
                .query_opt(queries::SCHEMA, &[name])
                .await
                .map_err(database)?
                .ok_or(CatalogError::ObjectNotFound)?;
            let oid = decode_oid(&row, "oid")?;
            if expected_oid.is_some_and(|expected| expected != oid) {
                return Err(CatalogError::SchemaMapIdentityChanged);
            }
            Ok((Some(oid), None))
        }
        SchemaMapScope::Relation {
            schema,
            table,
            expected,
        } => {
            let row = client
                .query_opt(queries::RELATION, &[schema, table])
                .await
                .map_err(database)?
                .ok_or(CatalogError::ObjectNotFound)?;
            let kind: &str = row
                .try_get("kind")
                .map_err(|_| CatalogError::InvalidResponse)?;
            if !matches!(kind, "r" | "p") {
                return Err(CatalogError::UnsupportedObjectKind);
            }
            let oid = decode_oid(&row, "oid")?;
            if expected.is_some_and(|expected| {
                expected
                    != SchemaMapIdentity {
                        database_oid,
                        relation_oid: oid,
                    }
            }) {
                return Err(CatalogError::SchemaMapIdentityChanged);
            }
            Ok((Some(decode_oid(&row, "schema_oid")?), Some(oid)))
        }
    }
}
fn decode_oid(row: &tokio_postgres::Row, field: &str) -> Result<u32, CatalogError> {
    row.try_get::<_, i64>(field)
        .ok()
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id != 0)
        .ok_or(CatalogError::InvalidResponse)
}
pub(super) async fn read(
    client: &Client,
    timeout: Option<u32>,
    request: SchemaMapRequest,
) -> Result<SchemaMapSnapshot, CatalogError> {
    begin_snapshot(client, timeout).await?;
    let version: i32 = client
        .query_one("SELECT current_setting('server_version_num')::int", &[])
        .await
        .map_err(database)?
        .try_get(0)
        .map_err(|_| CatalogError::InvalidResponse)?;
    if version < i32::try_from(MIN_SCHEMA_MAP_SERVER_VERSION).unwrap() {
        return Err(CatalogError::SchemaMapVersion);
    }
    let mut budget = Budget::default();
    let mut header =
        rows::<Header>(client, queries::HEADER, &[], &["database"], 1, &mut budget).await?;
    let header = header.pop().ok_or(CatalogError::InvalidResponse)?;
    let (schema_oid, focus_oid) = resolve(client, &request, header.database_oid).await?;
    let schema = schema_oid.map(i64::from);
    let focus = focus_oid.map(i64::from);
    let tables = rows::<Table>(
        client,
        &queries::tables(),
        &[&schema, &focus],
        &["schema", "name"],
        MAX_SCHEMA_MAP_TABLES,
        &mut budget,
    )
    .await?;
    let ids = tables.iter().map(|t| t.oid).collect::<Vec<_>>();
    let foreign_keys = rows::<ForeignKey>(
        client,
        &queries::foreign_keys(),
        &[&schema, &focus],
        &["name"],
        MAX_SCHEMA_MAP_FOREIGN_KEYS,
        &mut budget,
    )
    .await?;
    let columns = rows::<Column>(
        client,
        queries::COLUMNS,
        &[&ids],
        &["name"],
        MAX_SCHEMA_MAP_COLUMNS,
        &mut budget,
    )
    .await?;
    let unique = rows::<Key>(
        client,
        queries::UNIQUE_KEYS,
        &[&ids],
        &[],
        MAX_SCHEMA_MAP_UNIQUE_KEYS,
        &mut budget,
    )
    .await?;
    let outgoing = rows::<Key>(
        client,
        queries::OUTGOING,
        &[&ids],
        &[],
        MAX_SCHEMA_MAP_FOREIGN_KEYS,
        &mut budget,
    )
    .await?;
    let triggers = rows::<Trigger>(
        client,
        queries::TRIGGERS,
        &[&ids],
        &["name", "function_schema", "function_name"],
        MAX_SCHEMA_MAP_TRIGGERS,
        &mut budget,
    )
    .await?;
    let snapshot = assemble::assemble(
        header,
        request.scope,
        schema_oid,
        focus_oid,
        Parts {
            tables,
            columns,
            foreign_keys,
            unique,
            outgoing,
            triggers,
        },
    )?;
    snapshot
        .checked_heap_bytes()
        .ok_or(CatalogError::SchemaMapLimit)?;
    client.batch_execute("COMMIT").await.map_err(database)?;
    Ok(snapshot)
}

struct Parts {
    tables: Vec<Table>,
    columns: Vec<Column>,
    foreign_keys: Vec<ForeignKey>,
    unique: Vec<Key>,
    outgoing: Vec<Key>,
    triggers: Vec<Trigger>,
}
