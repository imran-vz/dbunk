use super::*;
use futures_util::TryStreamExt;
use std::mem::size_of;
use tokio_postgres::{types::FromSql, Client, Row};
fn db(error: tokio_postgres::Error) -> CatalogError {
    match error.code().map(|c| c.code()) {
        Some("42501") => CatalogError::TableExportPermission,
        Some("42P01" | "42703" | "3F000") => CatalogError::TableExportIdentityChanged,
        Some("57014") => CatalogError::Timeout,
        _ => CatalogError::Database,
    }
}
fn get<'a, T: FromSql<'a>>(row: &'a Row, index: usize) -> Result<T, CatalogError> {
    row.try_get(index)
        .map_err(|_| CatalogError::InvalidResponse)
}
fn oid(row: &Row, index: usize) -> Result<u32, CatalogError> {
    u32::try_from(get::<i64>(row, index)?)
        .ok()
        .filter(|n| *n != 0)
        .ok_or(CatalogError::InvalidResponse)
}
const RELATION: &str = "SELECT c.oid::bigint,c.reltype::bigint,n.oid::bigint,c.relkind::text,c.relrowsecurity OR c.relforcerowsecurity,c.relispopulated FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2";
const COLUMNS: &str = "SELECT attnum::smallint,CASE WHEN octet_length(convert_to(attname::text,'UTF8'))<=63 THEN attname::text END,atttypid::bigint,atttypmod,attcollation::bigint FROM pg_catalog.pg_attribute WHERE attrelid=$1 AND attnum>0 AND NOT attisdropped ORDER BY attnum LIMIT 1025";
#[derive(PartialEq, Eq)]
struct Relation {
    oid: u32,
    row_type: u32,
    schema: u32,
    kind: TableExportKind,
    rls: bool,
}
async fn relation(client: &Client, request: &TableExportRequest) -> Result<Relation, CatalogError> {
    let row = client
        .query_opt(RELATION, &[&request.schema, &request.table])
        .await
        .map_err(db)?
        .ok_or(CatalogError::ObjectNotFound)?;
    let kind = match get::<&str>(&row, 3)? {
        "r" => TableExportKind::Table,
        "p" => TableExportKind::PartitionedTable,
        "v" => TableExportKind::View,
        "m" if get::<bool>(&row, 5)? => TableExportKind::MaterializedView,
        "f" => TableExportKind::ForeignTable,
        _ => return Err(CatalogError::UnsupportedObjectKind),
    };
    Ok(Relation {
        oid: oid(&row, 0)?,
        row_type: oid(&row, 1)?,
        schema: oid(&row, 2)?,
        kind,
        rls: get(&row, 4)?,
    })
}
async fn columns(
    client: &Client,
    relation_oid: u32,
) -> Result<Vec<TableExportColumn>, CatalogError> {
    let stream = client
        .query_raw(COLUMNS, [&relation_oid])
        .await
        .map_err(db)?;
    tokio::pin!(stream);
    let mut columns = Vec::new();
    while let Some(row) = stream.try_next().await.map_err(db)? {
        if columns.len() == MAX_TABLE_EXPORT_COLUMNS {
            return Err(CatalogError::TableExportLimit);
        }
        let name: Option<&str> = get(&row, 1)?;
        let name = name
            .filter(|n| bounds::name(n))
            .ok_or(CatalogError::TableExportLimit)?;
        columns.push(TableExportColumn {
            name: name.to_owned(),
            attnum: get(&row, 0)?,
            type_oid: oid(&row, 2)?,
            type_modifier: get(&row, 3)?,
            collation_oid: u32::try_from(get::<i64>(&row, 4)?)
                .map_err(|_| CatalogError::InvalidResponse)?,
        });
    }
    Ok(columns)
}
pub(super) async fn read(
    client: &Client,
    timeout: Option<u32>,
    connection: String,
    request: TableExportRequest,
) -> Result<TableExportCapture, CatalogError> {
    let started = chrono::Utc::now().to_rfc3339();
    begin_snapshot(client, timeout).await?;
    client.batch_execute("SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL DateStyle TO ISO; SET LOCAL TIME ZONE 'UTC'; SET LOCAL bytea_output='hex'; SET LOCAL client_encoding='UTF8'").await.map_err(db)?;
    let header = client.query_one("SELECT oid::bigint, CASE WHEN octet_length(convert_to(datname::text,'UTF8'))<=63 THEN datname::text END FROM pg_catalog.pg_database WHERE datname=current_database()", &[]).await.map_err(db)?;
    let database_oid = oid(&header, 0)?;
    let database: Option<&str> = get(&header, 1)?;
    let database = database
        .filter(|n| bounds::name(n))
        .ok_or(CatalogError::TableExportLimit)?
        .to_owned();
    let target = relation(client, &request).await?;
    let identity = TableExportIdentity {
        database_oid,
        relation_oid: target.oid,
    };
    if request
        .expected
        .is_some_and(|expected| expected != identity)
    {
        return Err(CatalogError::TableExportIdentityChanged);
    }
    // All exportable kinds support SELECT and its transaction-held access-share
    // lock; explicit LOCK TABLE excludes materialized/foreign tables.
    client
        .batch_execute(&format!(
            "SELECT FROM {}.{} LIMIT 0",
            crate::quote_double(&request.schema),
            crate::quote_double(&request.table)
        ))
        .await
        .map_err(db)?;
    // Metadata must be read with a fresh command snapshot AFTER locking. An
    // earlier RR catalog snapshot could otherwise describe a pre-lock ALTER.
    // Qualified namespace lookup may race a rename, so also prove that this
    // backend actually holds the expected relation's lock, not only its name.
    let locked: bool = client
        .query_one(
            "SELECT EXISTS(SELECT FROM pg_catalog.pg_locks WHERE pid=pg_catalog.pg_backend_pid() AND locktype='relation' AND database=(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database()) AND relation=$1 AND granted AND mode IN ('AccessShareLock','RowShareLock','RowExclusiveLock','ShareUpdateExclusiveLock','ShareLock','ShareRowExclusiveLock','ExclusiveLock','AccessExclusiveLock'))",
            &[&target.oid],
        )
        .await
        .map_err(db)?
        .try_get(0)
        .map_err(|_| CatalogError::InvalidResponse)?;
    if !locked || relation(client, &request).await? != target {
        return Err(CatalogError::TableExportIdentityChanged);
    }
    // AccessShare now fences relation-definition changes through completion.
    // All cells are read by one SELECT and therefore one coherent MVCC snapshot.
    let metadata_columns = columns(client, target.oid).await?;
    let statement = client
        .prepare(&sql::capture(&request, &metadata_columns)?)
        .await
        .map_err(db)?;
    let mut data = TableExportData {
        connection_id: connection,
        database,
        schema: request.schema.clone(),
        table: request.table.clone(),
        identity,
        schema_oid: target.schema,
        kind: target.kind,
        row_security: target.rls,
        captured_start: started,
        captured_end: String::new(),
        columns: metadata_columns,
        rows: Vec::new(),
    };
    let mut heap = bounds::header_heap(&data).ok_or(CatalogError::TableExportLimit)?;
    let mut text = data.columns.iter().map(|c| c.name.len()).sum::<usize>();
    // End timestamp is populated after the stream; reserve its largest admitted
    // encoded/retained size before any row copies.
    let mut encoded = data.encoded_bytes().ok_or(CatalogError::TableExportLimit)? + 64;
    heap += 64;
    let stream = client
        .query_raw(
            &statement,
            std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
        )
        .await
        .map_err(db)?;
    tokio::pin!(stream);
    while let Some(row) = stream.try_next().await.map_err(db)? {
        // A schema rename can redirect qualified names between catalog commands. This witness comes from the same bound FROM source as cells.
        // Reject before retaining any mismatched values; not a claim that the
        // server never evaluated that read-only source.
        if get::<u32>(&row, 0)? != target.row_type {
            return Err(CatalogError::TableExportIdentityChanged);
        }
        if get::<Option<bool>>(&row, 1)?.is_none() {
            continue;
        }
        if get::<bool>(&row, 2)? || data.rows.len() == bounds::row_limit(data.columns.len()) {
            return Err(CatalogError::TableExportLimit);
        }
        let cells = (0..data.columns.len())
            .map(|i| get::<Option<&str>>(&row, i + 3))
            .collect::<Result<Vec<_>, _>>()?;
        let row_text = cells
            .iter()
            .flatten()
            .try_fold(0usize, |n, s| n.checked_add(s.len()))
            .ok_or(CatalogError::TableExportLimit)?;
        text = text
            .checked_add(row_text)
            .filter(|n| *n <= MAX_TABLE_EXPORT_TEXT_BYTES)
            .ok_or(CatalogError::TableExportLimit)?;
        encoded = encoded
            .checked_add(
                bounds::encoded(&cells, MAX_TABLE_EXPORT_ENCODED_BYTES)
                    .ok_or(CatalogError::TableExportLimit)?
                    + usize::from(!data.rows.is_empty()),
            )
            .filter(|n| *n <= MAX_TABLE_EXPORT_ENCODED_BYTES)
            .ok_or(CatalogError::TableExportLimit)?;
        let extra_capacity = if data.rows.len() == data.rows.capacity() {
            512.min(bounds::row_limit(data.columns.len()) - data.rows.len())
        } else {
            0
        };
        heap = heap
            .checked_add(extra_capacity * size_of::<Vec<Option<String>>>())
            .and_then(|n| n.checked_add(cells.len() * size_of::<Option<String>>() + row_text))
            .filter(|n| *n <= MAX_TABLE_EXPORT_HEAP_BYTES)
            .ok_or(CatalogError::TableExportLimit)?;
        if extra_capacity > 0 {
            data.rows
                .try_reserve_exact(extra_capacity)
                .map_err(|_| CatalogError::TableExportLimit)?;
        }
        data.rows.push(
            cells
                .into_iter()
                .map(|value| value.map(str::to_owned))
                .collect(),
        );
    }
    if relation(client, &request).await? != target
        || columns(client, target.oid).await? != data.columns
    {
        return Err(CatalogError::TableExportIdentityChanged);
    }
    client.batch_execute("COMMIT").await.map_err(db)?;
    data.captured_end = chrono::Utc::now().to_rfc3339();
    data.checked_heap_bytes()
        .ok_or(CatalogError::TableExportLimit)?;
    Ok(TableExportCapture(Arc::new(data)))
}
