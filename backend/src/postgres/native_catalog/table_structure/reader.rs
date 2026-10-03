use super::*;
use serde::de::DeserializeOwned;
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    database_oid: u32,
    relation_oid: u32,
    schema: String,
    table: String,
    kind: String,
    owner: String,
    comment: Option<String>,
    server_version: u32,
    captured_at: String,
    rls_enabled: bool,
    rls_forced: bool,
    partition_key: Option<String>,
    is_partition: bool,
    partition_bound: Option<String>,
}
#[derive(Default)]
pub(super) struct Budget {
    components: usize,
    wire: usize,
    working: usize,
}
impl Budget {
    pub(super) fn admit(&mut self, payload: &str, components: i32) -> Result<(), CatalogError> {
        let components = usize::try_from(components).map_err(|_| CatalogError::InvalidResponse)?;
        if components == 0 || components > MAX_STRUCTURE_COMPONENTS.saturating_sub(self.components)
        {
            return Err(CatalogError::StructureLimit);
        }
        // All nested arrays are counted by SQL. Strings cannot decode longer
        // than their JSON bytes; 512 bytes/component covers Vec growth/headers,
        // scalar fields and the simultaneously retained typed row. Refuse before
        // deserialization, rather than allocating an oversized capture first.
        let working = payload
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(components.checked_mul(512)?))
            .ok_or(CatalogError::StructureLimit)?;
        if payload.len() > MAX_STRUCTURE_BYTES.saturating_sub(self.wire)
            || working > MAX_STRUCTURE_BYTES.saturating_sub(self.working)
        {
            return Err(CatalogError::StructureLimit);
        }
        self.components += components;
        self.wire += payload.len();
        self.working += working;
        Ok(())
    }
}
fn database(error: tokio_postgres::Error) -> CatalogError {
    classify_sqlstate(error.code().map(tokio_postgres::error::SqlState::code))
}
pub(super) fn classify_sqlstate(code: Option<&str>) -> CatalogError {
    match code {
        Some("42501") => CatalogError::StructurePermission,
        _ => CatalogError::Database,
    }
}
pub(super) fn supported_version(version: i32) -> Result<u32, CatalogError> {
    let version = u32::try_from(version).map_err(|_| CatalogError::InvalidResponse)?;
    if version < MIN_STRUCTURE_SERVER_VERSION {
        return Err(CatalogError::StructureVersion);
    }
    Ok(version)
}
/// Guard every string recursively on the server; metadata paths use narrower
/// bounds. Constants form SQL, while endpoint values always remain parameters.
async fn rows<T: DeserializeOwned>(
    client: &Client,
    source: &str,
    params: &[&(dyn ToSql + Sync)],
    names: &[&str],
    metadata: &[&str],
    limit: usize,
    budget: &mut Budget,
) -> Result<Vec<T>, CatalogError> {
    let paths = names
        .iter()
        .map(|p| (*p, 63))
        .chain(metadata.iter().map(|p| (*p, MAX_STRUCTURE_METADATA_BYTES)));
    // JSON text extraction measures decoded UTF-8, not JSON escaping.
    let mut guards = String::from(
        r#"EXISTS(SELECT FROM pg_catalog.jsonb_path_query(doc,'strict $.** ? (@.type() == "string")')v WHERE octet_length(convert_to(v #>> '{}','UTF8'))>1048576)"#,
    );
    for (path, cap) in paths {
        guards.push_str(&format!(" OR EXISTS(SELECT FROM pg_catalog.jsonb_path_query(doc,'lax {path}')v WHERE jsonb_typeof(v)='string' AND octet_length(convert_to(v #>> '{{}}','UTF8'))>{cap})"));
    }
    let take = limit.min(MAX_STRUCTURE_COMPONENTS.saturating_sub(budget.components)) + 1;
    let sql=format!("WITH source AS ({source} LIMIT {take}), encoded AS(SELECT to_jsonb(source)-'components'-'array_oversized' AS doc,components,coalesce((to_jsonb(source)->>'array_oversized')::boolean,false) AS array_oversized FROM source), checked AS(SELECT doc::text AS payload,components,(array_oversized OR ({guards}))AS oversized FROM encoded)SELECT CASE WHEN NOT oversized AND octet_length(convert_to(payload,'UTF8'))<={MAX_STRUCTURE_BYTES} THEN payload END AS payload,oversized OR octet_length(convert_to(payload,'UTF8'))>{MAX_STRUCTURE_BYTES} AS oversized,components FROM checked");
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
            return Err(CatalogError::StructureLimit);
        }
        let payload: &str = row
            .try_get("payload")
            .map_err(|_| CatalogError::StructureLimit)?;
        let components: i32 = row
            .try_get("components")
            .map_err(|_| CatalogError::InvalidResponse)?;
        budget.admit(payload, components)?;
        result.push(serde_json::from_str(payload).map_err(|_| CatalogError::InvalidResponse)?);
    }
    Ok(result)
}
pub(super) async fn read(
    client: &Client,
    timeout: Option<u32>,
    request: TableStructureRequest,
) -> Result<TableStructureSnapshot, CatalogError> {
    begin_snapshot(client, timeout).await?;
    // Run before JSON-path or version-specific catalog statements. The legacy
    // Structure reader explicitly uses PG13 tgparentid; older connections may
    // still use other catalog capabilities without this complete capture.
    let version = client
        .query_one("SELECT current_setting('server_version_num')::integer", &[])
        .await
        .map_err(database)?
        .try_get::<_, i32>(0)
        .map_err(|_| CatalogError::InvalidResponse)?;
    let version = supported_version(version)?;
    client
        .batch_execute("SET LOCAL search_path TO pg_catalog; SET LOCAL TIME ZONE 'UTC'")
        .await
        .map_err(database)?;
    let mut budget = Budget::default();
    let mut headers = rows::<Header>(
        client,
        queries::HEADER,
        &[&request.schema, &request.table],
        &["$.schema", "$.table", "$.owner"],
        &["$.captured_at", "$.comment"],
        1,
        &mut budget,
    )
    .await?;
    let h = headers.pop().ok_or(CatalogError::ObjectNotFound)?;
    if h.server_version != version {
        return Err(CatalogError::InvalidResponse);
    }
    let identity = TableIdentity {
        database_oid: h.database_oid,
        relation_oid: h.relation_oid,
    };
    if request
        .expected
        .is_some_and(|expected| expected != identity)
    {
        return Err(CatalogError::StructureIdentityChanged);
    }
    let kind = match h.kind.as_str() {
        "table" => StructureRelationKind::Table,
        "partitioned_table" => StructureRelationKind::PartitionedTable,
        "view" => StructureRelationKind::View,
        "materialized_view" => StructureRelationKind::MaterializedView,
        "foreign_table" => StructureRelationKind::ForeignTable,
        _ => return Err(CatalogError::UnsupportedObjectKind),
    };
    let oid = identity.relation_oid;
    let columns = rows(
        client,
        queries::COLUMNS,
        &[&oid],
        &["$.name", "$.collation_schema", "$.collation_name"],
        &["$.data_type", "$.comment"],
        MAX_STRUCTURE_COLUMNS,
        &mut budget,
    )
    .await?;
    let mut pk = rows(
        client,
        queries::PRIMARY_KEY,
        &[&oid],
        &["$.name", "$.columns[*].name"],
        &[],
        1,
        &mut budget,
    )
    .await?;
    let names = [
        "$.name",
        "$.source_schema",
        "$.source_table",
        "$.target_schema",
        "$.target_table",
        "$.columns[*].source",
        "$.columns[*].target",
    ];
    let outbound = rows(
        client,
        queries::FOREIGN_KEYS,
        &[&oid, &false],
        &names,
        &["$.match_type"],
        4096,
        &mut budget,
    )
    .await?;
    let inbound = rows(
        client,
        queries::FOREIGN_KEYS,
        &[&oid, &true],
        &names,
        &["$.match_type"],
        4096,
        &mut budget,
    )
    .await?;
    let indexes = rows(
        client,
        queries::INDEXES,
        &[&oid],
        &["$.name", "$.method", "$.keys[*].column_name"],
        &[],
        4096,
        &mut budget,
    )
    .await?;
    let constraints = rows(
        client,
        queries::CONSTRAINTS,
        &[&oid],
        &["$.name"],
        &["$.kind"],
        4096,
        &mut budget,
    )
    .await?;
    let triggers = rows(
        client,
        queries::TRIGGERS,
        &[&oid],
        &[
            "$.name",
            "$.function_schema",
            "$.function_name",
            "$.update_columns[*].name",
        ],
        &["$.timing", "$.level", "$.events[*]"],
        4096,
        &mut budget,
    )
    .await?;
    let policies = rows(
        client,
        queries::POLICIES,
        &[&oid],
        &["$.name", "$.roles[*]"],
        &[],
        4096,
        &mut budget,
    )
    .await?;
    let privileges = rows(
        client,
        queries::PRIVILEGES,
        &[&oid],
        &["$.grantor", "$.grantee"],
        &["$.privilege"],
        4096,
        &mut budget,
    )
    .await?;
    let rules = rows(
        client,
        queries::RULES,
        &[&oid],
        &["$.name"],
        &["$.event"],
        4096,
        &mut budget,
    )
    .await?;
    let parents = rows(
        client,
        queries::RELATIVES,
        &[&oid, &true],
        &["$.schema", "$.name"],
        &[],
        4096,
        &mut budget,
    )
    .await?;
    let partitions = rows(
        client,
        queries::RELATIVES,
        &[&oid, &false],
        &["$.schema", "$.name"],
        &[],
        4096,
        &mut budget,
    )
    .await?;
    let snapshot = TableStructureSnapshot {
        identity,
        schema: h.schema,
        table: h.table,
        kind,
        owner: h.owner,
        comment: h.comment,
        server_version: h.server_version,
        captured_at: h.captured_at,
        columns,
        primary_key: pk.pop(),
        outbound,
        inbound,
        indexes,
        constraints,
        triggers,
        row_security: StructureRowSecurity {
            enabled: h.rls_enabled,
            forced: h.rls_forced,
        },
        policies,
        privileges,
        rules,
        partition_key: h.partition_key,
        is_partition: h.is_partition,
        partition_bound: h.partition_bound,
        parents,
        partitions,
    };
    snapshot
        .checked_heap_bytes()
        .ok_or(CatalogError::StructureLimit)?;
    client.batch_execute("COMMIT").await.map_err(database)?;
    Ok(snapshot)
}
