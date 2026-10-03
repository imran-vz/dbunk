use super::*;
use futures_util::TryStreamExt;
use tokio_postgres::{
    types::{FromSql, ToSql},
    Client, Row,
};

pub(super) fn sqlstate(code: Option<&str>) -> CatalogError {
    if code == Some("42501") {
        CatalogError::OverviewPermission
    } else {
        CatalogError::Database
    }
}
fn db_error(e: tokio_postgres::Error) -> CatalogError {
    sqlstate(e.code().map(|c| c.code()))
}
fn get<'a, T: FromSql<'a>>(row: &'a Row, field: &str) -> Result<T, CatalogError> {
    row.try_get(field)
        .map_err(|_| CatalogError::InvalidResponse)
}
fn number(row: &Row, field: &str) -> Result<i64, CatalogError> {
    let value: i64 = get(row, field)?;
    if value < 0 {
        Err(CatalogError::InvalidResponse)
    } else {
        Ok(value)
    }
}
fn oid(row: &Row, field: &str) -> Result<u32, CatalogError> {
    u32::try_from(number(row, field)?)
        .ok()
        .filter(|id| *id != 0)
        .ok_or(CatalogError::InvalidResponse)
}
fn text(row: &Row, field: &str) -> Result<String, CatalogError> {
    let s: Option<&str> = get(row, field)?;
    let s = s
        .filter(|s| bounds::name(s))
        .ok_or(CatalogError::OverviewLimit)?;
    Ok(s.to_owned())
}
pub(super) fn estimate(known: i64, unknown: i64) -> OverviewMetric {
    if unknown == 0 {
        OverviewMetric::Value(known)
    } else {
        OverviewMetric::Unknown
    }
}
fn metric(row: &Row, field: &str) -> Result<OverviewMetric, CatalogError> {
    match get::<Option<i64>>(row, field)? {
        Some(n) if n >= 0 => Ok(OverviewMetric::Value(n)),
        None => Ok(OverviewMetric::Unknown),
        _ => Err(CatalogError::InvalidResponse),
    }
}
fn kind(row: &Row) -> Result<OverviewRelationKind, CatalogError> {
    match get::<&str>(row, "kind")? {
        "r" => Ok(OverviewRelationKind::Table),
        "p" => Ok(OverviewRelationKind::PartitionedTable),
        "v" => Ok(OverviewRelationKind::View),
        "m" => Ok(OverviewRelationKind::MaterializedView),
        _ => Err(CatalogError::UnsupportedObjectKind),
    }
}
async fn capture(client: &Client, timeout: Option<u32>) -> Result<OverviewCapture, CatalogError> {
    let collected_start = chrono::Utc::now().to_rfc3339();
    begin_snapshot(client, timeout).await?;
    let row = client
        .query_one(queries::IDENTITY, &[])
        .await
        .map_err(db_error)?;
    Ok(OverviewCapture {
        database: text(&row, "database")?,
        database_oid: oid(&row, "database_oid")?,
        reader_pid: get(&row, "pid")?,
        collected_start,
        collected_end: String::new(),
    })
}
/// Only an explicit insufficient-privilege SQLSTATE is an optional restricted
/// metric. Missing objects, overflow, timeout and other failures stay errors.
async fn optional(
    client: &Client,
    query: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Option<Vec<Row>>, CatalogError> {
    client
        .batch_execute("SAVEPOINT overview_metric")
        .await
        .map_err(db_error)?;
    let rows = match client.query(query, params).await {
        Ok(rows) => Some(rows),
        Err(e) if sqlstate(e.code().map(|c| c.code())) == CatalogError::OverviewPermission => {
            client
                .batch_execute("ROLLBACK TO SAVEPOINT overview_metric")
                .await
                .map_err(db_error)?;
            None
        }
        Err(e) => return Err(db_error(e)),
    };
    client
        .batch_execute("RELEASE SAVEPOINT overview_metric")
        .await
        .map_err(db_error)?;
    Ok(rows)
}
async fn optional_metric(
    client: &Client,
    query: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<OverviewMetric, CatalogError> {
    match optional(client, query, params).await? {
        Some(rows) if rows.len() == 1 => metric(&rows[0], "value"),
        Some(_) => Err(CatalogError::InvalidResponse),
        None => Ok(OverviewMetric::Restricted),
    }
}
async fn load_database(
    client: &Client,
    capture: OverviewCapture,
) -> Result<DatabaseOverviewSnapshot, CatalogError> {
    let counts = client
        .query_one(queries::DATABASE_COUNTS, &[])
        .await
        .map_err(db_error)?;
    let known = number(&counts, "known_rows")?;
    let unknown = number(&counts, "unknown_rows")?;
    let sizes = optional(client, queries::DATABASE_RELATION_SIZES, &[]).await?;
    let (table_size_bytes, index_size_bytes) = match sizes {
        Some(rows) if rows.len() == 1 => (
            metric(&rows[0], "table_bytes")?,
            metric(&rows[0], "index_bytes")?,
        ),
        Some(_) => return Err(CatalogError::InvalidResponse),
        None => (OverviewMetric::Restricted, OverviewMetric::Restricted),
    };
    let mut result = DatabaseOverviewSnapshot {
        capture,
        database_size_bytes: optional_metric(client, queries::DATABASE_SIZE, &[]).await?,
        table_size_bytes,
        index_size_bytes,
        table_count: number(&counts, "table_count")?,
        schema_count: number(&counts, "schema_count")?,
        row_count_estimate: estimate(known, unknown),
        known_row_count_estimate: known,
        unknown_estimate_relations: unknown,
        index_count: number(&counts, "index_count")?,
        connection_count: optional_metric(client, queries::CONNECTIONS, &[]).await?,
    };
    result.capture.collected_end = chrono::Utc::now().to_rfc3339();
    result
        .checked_heap_bytes()
        .ok_or(CatalogError::OverviewLimit)?;
    Ok(result)
}
async fn resolve(
    client: &Client,
    request: &RelationStatsRequest,
    database_oid: u32,
) -> Result<(Option<u32>, Option<u32>), CatalogError> {
    if request
        .expected_database_oid
        .is_some_and(|oid| oid != database_oid)
    {
        return Err(CatalogError::OverviewIdentityChanged);
    }
    match &request.scope {
        RelationStatsScope::Database => Ok((None, None)),
        RelationStatsScope::Schema { name, expected_oid } => {
            let row = client
                .query_opt(queries::SCHEMA, &[name])
                .await
                .map_err(db_error)?
                .ok_or(CatalogError::ObjectNotFound)?;
            let actual = oid(&row, "oid")?;
            if expected_oid.is_some_and(|expected| expected != actual) {
                return Err(CatalogError::OverviewIdentityChanged);
            }
            Ok((Some(actual), None))
        }
        RelationStatsScope::Relation {
            schema,
            name,
            expected,
        } => {
            let row = client
                .query_opt(queries::RELATION, &[schema, name])
                .await
                .map_err(db_error)?
                .ok_or(CatalogError::ObjectNotFound)?;
            kind(&row)?;
            let actual = oid(&row, "oid")?;
            if expected.is_some_and(|e| e.database_oid != database_oid || e.relation_oid != actual)
            {
                return Err(CatalogError::OverviewIdentityChanged);
            }
            Ok((Some(oid(&row, "schema_oid")?), Some(actual)))
        }
    }
}
pub(super) fn check_cursor(
    request: &RelationStatsRequest,
    connection: &str,
    database_oid: u32,
    database: &str,
    schema_oid: Option<u32>,
    relation_oid: Option<u32>,
) -> Result<(), CatalogError> {
    if request.cursor.as_ref().is_some_and(|c| {
        c.connection != connection
            || c.database_oid != database_oid
            || c.database != database
            || c.scope != request.scope
            || c.schema_oid != schema_oid
            || c.relation_oid != relation_oid
    }) {
        return Err(CatalogError::OverviewIdentityChanged);
    }
    Ok(())
}
async fn load_relations(
    client: &Client,
    capture: OverviewCapture,
    request: RelationStatsRequest,
    connection: String,
    document: String,
) -> Result<RelationStatsSnapshot, CatalogError> {
    if connection.is_empty() || connection.len() > 256 {
        return Err(CatalogError::InvalidReference);
    }
    let (schema_oid, relation_oid) = resolve(client, &request, capture.database_oid).await?;
    check_cursor(
        &request,
        &connection,
        capture.database_oid,
        &capture.database,
        schema_oid,
        relation_oid,
    )?;
    let schema = schema_oid.map(i64::from);
    let relation = relation_oid.map(i64::from);
    let totals = client
        .query_one(&queries::totals(), &[&schema, &relation])
        .await
        .map_err(db_error)?;
    let known = number(&totals, "known_rows")?;
    let unknown = number(&totals, "unknown_rows")?;
    let totals = RelationStatsTotals {
        relation_count: number(&totals, "relation_count")?,
        table_count: number(&totals, "table_count")?,
        view_count: number(&totals, "view_count")?,
        materialized_view_count: number(&totals, "materialized_view_count")?,
        row_count_estimate: estimate(known, unknown),
        known_row_count_estimate: known,
        unknown_estimate_relations: unknown,
        total_size_bytes: optional_metric(client, &queries::scope_size(), &[&schema, &relation])
            .await?,
    };
    let after_schema = request.cursor.as_ref().map(|c| c.schema.as_str());
    let after_name = request.cursor.as_ref().map(|c| c.name.as_str());
    let after_oid = request.cursor.as_ref().map(|c| i64::from(c.oid));
    let page_sql = queries::page();
    let params: [&(dyn ToSql + Sync); 5] =
        [&schema, &relation, &after_schema, &after_name, &after_oid];
    let stream = client
        .query_raw(&page_sql, params)
        .await
        .map_err(db_error)?;
    tokio::pin!(stream);
    // All wire names are capped before decode, scalar fields are fixed-width,
    // and this fixed capacity is charged by checked_heap_bytes before publishing.
    let mut rows = Vec::with_capacity(MAX_RELATION_STATS_ROWS);
    let mut more = false;
    while let Some(row) = stream.try_next().await.map_err(db_error)? {
        if rows.len() == MAX_RELATION_STATS_ROWS {
            more = true;
            continue;
        }
        let kind = kind(&row)?;
        rows.push(RelationStats {
            identity: OverviewRelationIdentity {
                database_oid: capture.database_oid,
                relation_oid: oid(&row, "oid")?,
            },
            schema_oid: oid(&row, "schema_oid")?,
            schema: text(&row, "schema")?,
            name: text(&row, "name")?,
            kind,
            is_partition: get(&row, "is_partition")?,
            row_count_estimate: if kind == OverviewRelationKind::View {
                OverviewMetric::NotApplicable
            } else {
                metric(&row, "estimate")?
            },
            total_size_bytes: if kind == OverviewRelationKind::View {
                OverviewMetric::NotApplicable
            } else {
                OverviewMetric::Unknown
            },
        });
    }
    let ids: Vec<u32> = rows
        .iter()
        .filter(|r| r.kind != OverviewRelationKind::View)
        .map(|r| r.identity.relation_oid)
        .collect();
    if !ids.is_empty() {
        match optional(client, queries::PAGE_SIZES, &[&ids]).await? {
            None => {
                for row in &mut rows {
                    if row.kind != OverviewRelationKind::View {
                        row.total_size_bytes = OverviewMetric::Restricted;
                    }
                }
            }
            Some(sizes) => {
                if sizes.len() != ids.len() {
                    return Err(CatalogError::InvalidResponse);
                }
                for (size, expected) in sizes.iter().zip(ids) {
                    if oid(size, "oid")? != expected {
                        return Err(CatalogError::InvalidResponse);
                    }
                    let row = rows
                        .iter_mut()
                        .find(|r| r.identity.relation_oid == expected)
                        .ok_or(CatalogError::InvalidResponse)?;
                    row.total_size_bytes = metric(size, "value")?;
                }
            }
        }
    }
    let next_cursor = if more {
        let last = rows.last().ok_or(CatalogError::InvalidResponse)?;
        Some(RelationStatsCursor {
            connection,
            document,
            database: capture.database.clone(),
            database_oid: capture.database_oid,
            scope: request.scope.clone(),
            schema_oid,
            relation_oid,
            schema: last.schema.clone(),
            name: last.name.clone(),
            oid: last.identity.relation_oid,
        })
    } else {
        None
    };
    let mut result = RelationStatsSnapshot {
        capture,
        scope: request.scope,
        schema_oid,
        relation_oid,
        totals,
        rows,
        next_cursor,
    };
    result.capture.collected_end = chrono::Utc::now().to_rfc3339();
    result
        .checked_heap_bytes()
        .ok_or(CatalogError::OverviewLimit)?;
    Ok(result)
}

pub(super) async fn database(
    client: &Client,
    timeout: Option<u32>,
) -> Result<DatabaseOverviewSnapshot, CatalogError> {
    let capture = capture(client, timeout).await?;
    let result = load_database(client, capture).await?;
    client.batch_execute("COMMIT").await.map_err(db_error)?;
    Ok(result)
}
pub(super) async fn relations(
    client: &Client,
    timeout: Option<u32>,
    request: RelationStatsRequest,
    connection: String,
    document: String,
) -> Result<RelationStatsSnapshot, CatalogError> {
    let capture = capture(client, timeout).await?;
    let result = load_relations(client, capture, request, connection, document).await?;
    client.batch_execute("COMMIT").await.map_err(db_error)?;
    Ok(result)
}
pub(super) async fn overview(
    client: &Client,
    timeout: Option<u32>,
    request: RelationStatsRequest,
    connection: String,
    document: String,
) -> Result<OverviewSnapshot, CatalogError> {
    let capture = capture(client, timeout).await?;
    let database = if request.scope == RelationStatsScope::Database && request.cursor.is_none() {
        Some(load_database(client, capture.clone()).await?)
    } else {
        None
    };
    let relations = load_relations(client, capture, request, connection, document).await?;
    client.batch_execute("COMMIT").await.map_err(db_error)?;
    let result = OverviewSnapshot {
        database,
        relations,
    };
    result
        .checked_heap_bytes()
        .ok_or(CatalogError::OverviewLimit)?;
    Ok(result)
}
