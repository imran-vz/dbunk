//! Catalog-only column completion. Never reads user/foreign rows or parses SQL.
use super::*;

pub const MAX_COMPLETION_COLUMNS: usize = 4096;
pub const MAX_COMPLETION_BYTES: usize = 1024 * 1024;
pub const MAX_COMPLETION_NAME_BYTES: usize = 63;
pub const MAX_COMPLETION_TYPE_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionRelationKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}
impl CompletionRelationKind {
    fn decode(kind: &str) -> Result<Self, CatalogError> {
        match kind {
            "r" => Ok(Self::Table),
            "p" => Ok(Self::PartitionedTable),
            "v" => Ok(Self::View),
            "m" => Ok(Self::MaterializedView),
            "f" => Ok(Self::ForeignTable),
            _ => Err(CatalogError::UnsupportedObjectKind),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionColumn {
    pub name: String,
    /// PostgreSQL format_type display, including typmod/domain/array identity.
    /// Display metadata only: never an executable cast or edit capability.
    pub data_type: String,
    pub ordinal_position: i32,
    pub is_primary_key: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionColumns {
    pub schema: String,
    pub relation: String,
    pub relation_oid: u32,
    pub kind: CompletionRelationKind,
    pub columns: Vec<CompletionColumn>,
}

pub(crate) fn validate(schema: &str, relation: &str) -> Result<(), CatalogError> {
    for name in [schema, relation] {
        if name.is_empty() || name.contains('\0') {
            return Err(CatalogError::InvalidReference);
        }
        if name.len() > MAX_COMPLETION_NAME_BYTES {
            return Err(CatalogError::CompletionLimit);
        }
    }
    Ok(())
}
pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    schema: String,
    relation: String,
) -> Result<CompletionColumns, CatalogError> {
    validate(&schema, &relation)?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| {
            Box::pin(async move {
                begin_snapshot(client, timeout).await?;
                let identity = client
                    .query_opt(IDENTITY, &[&schema, &relation])
                    .await
                    .map_err(|_| CatalogError::Database)?
                    .ok_or(CatalogError::ObjectNotFound)?;
                let oid: u32 = get(&identity, "oid")?;
                let kind = CompletionRelationKind::decode(get(&identity, "kind")?)?;
                let mut builder = Builder::new(schema, relation, oid, kind)?;
                let limit = MAX_COMPLETION_COLUMNS as i64 + 1;
                let rows = client
                    .query_raw(COLUMNS, [&oid as &(dyn ToSql + Sync), &limit])
                    .await
                    .map_err(|_| CatalogError::Database)?;
                tokio::pin!(rows);
                while let Some(row) = rows.try_next().await.map_err(|_| CatalogError::Database)? {
                    if get::<bool>(&row, "oversized")? {
                        return Err(CatalogError::CompletionLimit);
                    }
                    builder.push(CompletionColumn {
                        name: get::<&str>(&row, "name")?.to_owned(),
                        data_type: get::<&str>(&row, "data_type")?.to_owned(),
                        ordinal_position: get(&row, "ordinal_position")?,
                        is_primary_key: get(&row, "is_primary_key")?,
                    })?;
                }
                let result = builder.finish()?;
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
fn get<'a, T: tokio_postgres::types::FromSql<'a>>(
    row: &'a Row,
    name: &str,
) -> Result<T, CatalogError> {
    row.try_get(name).map_err(|_| CatalogError::InvalidResponse)
}

struct Builder {
    result: CompletionColumns,
    bytes: usize,
}
impl Builder {
    fn new(
        schema: String,
        relation: String,
        relation_oid: u32,
        kind: CompletionRelationKind,
    ) -> Result<Self, CatalogError> {
        validate(&schema, &relation)?;
        if relation_oid == 0 {
            return Err(CatalogError::InvalidResponse);
        }
        Ok(Self {
            result: CompletionColumns {
                schema,
                relation,
                relation_oid,
                kind,
                columns: vec![],
            },
            bytes: 2048,
        })
    }
    fn push(&mut self, column: CompletionColumn) -> Result<(), CatalogError> {
        if self.result.columns.len() >= MAX_COMPLETION_COLUMNS
            || column.name.len() > MAX_COMPLETION_NAME_BYTES
            || column.data_type.len() > MAX_COMPLETION_TYPE_BYTES
        {
            return Err(CatalogError::CompletionLimit);
        }
        if column.name.is_empty()
            || column.name.contains('\0')
            || column.data_type.is_empty()
            || column.data_type.contains('\0')
            || column.ordinal_position <= 0
            || self
                .result
                .columns
                .last()
                .is_some_and(|previous| previous.ordinal_position >= column.ordinal_position)
        {
            return Err(CatalogError::InvalidResponse);
        }
        let bytes = encoded(&column)?.saturating_add(1);
        if bytes > MAX_COMPLETION_BYTES.saturating_sub(self.bytes) {
            return Err(CatalogError::CompletionLimit);
        }
        self.bytes += bytes;
        self.result.columns.push(column);
        Ok(())
    }
    fn finish(self) -> Result<CompletionColumns, CatalogError> {
        encoded(&self.result)?;
        Ok(self.result)
    }
}
fn encoded(value: &impl Serialize) -> Result<usize, CatalogError> {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, value: &[u8]) -> io::Result<usize> {
            if value.len() > MAX_COMPLETION_BYTES.saturating_sub(self.0) {
                return Err(io::Error::other("completion byte limit"));
            }
            self.0 += value.len();
            Ok(value.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_err(|_| CatalogError::CompletionLimit)?;
    Ok(count.0)
}
const IDENTITY: &str = "SELECT c.oid, c.relkind::text AS kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2";
const COLUMNS: &str = r#"
WITH columns AS MATERIALIZED (
 SELECT a.attname::text AS name, pg_catalog.format_type(a.atttypid,a.atttypmod)::text AS data_type,
        a.attnum::int AS ordinal_position,
        EXISTS(SELECT 1 FROM pg_catalog.pg_constraint p WHERE p.conrelid=a.attrelid AND p.contype='p' AND a.attnum=ANY(p.conkey)) AS is_primary_key
 FROM pg_catalog.pg_attribute a
 WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped
 ORDER BY a.attnum LIMIT $2
)
SELECT CASE WHEN pg_catalog.octet_length(name)<=63 THEN name END AS name,
       CASE WHEN pg_catalog.octet_length(data_type)<=8192 THEN data_type END AS data_type,
       ordinal_position,is_primary_key,
       (pg_catalog.octet_length(name)>63 OR pg_catalog.octet_length(data_type)>8192) AS oversized
FROM columns ORDER BY ordinal_position
"#;

#[cfg(test)]
mod tests;
