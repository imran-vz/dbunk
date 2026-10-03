//! Bounded ordered FK metadata for native navigation. Reads only pg_catalog;
//! never selects user/foreign rows or contacts a foreign server.
use super::*;
use serde::Serialize;

pub const MAX_FOREIGN_KEYS: usize = 256;
pub const MAX_FOREIGN_KEY_PAIRS: usize = 4096;
pub const MAX_FOREIGN_KEY_BYTES: usize = 1024 * 1024;
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}
pub(crate) fn validate(schema: &str, table: &str) -> Result<(), CatalogError> {
    for name in [schema, table] {
        if name.is_empty() || name.len() > MAX_TEXT_BYTES || name.contains('\0') {
            return Err(CatalogError::InvalidReference);
        }
    }
    Ok(())
}
pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    schema: String,
    table: String,
) -> Result<Vec<ForeignKey>, CatalogError> {
    validate(&schema, &table)?;
    owned_read(spec, drivers, cancellation, Duration::from_secs(30), move |client, timeout| {
        Box::pin(async move {
            begin_snapshot(client, timeout).await?;
            let exists = client.query_opt("SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('r','p','f','v','m')", &[&schema,&table]).await.map_err(|_|CatalogError::Database)?;
            if exists.is_none() { return Err(CatalogError::ObjectNotFound); }
            let rows=client.query_raw(QUERY, [&schema as &(dyn ToSql+Sync), &table]).await.map_err(|_|CatalogError::Database)?;
            tokio::pin!(rows);
            let mut builder=Builder::default();
            while let Some(row)=rows.try_next().await.map_err(|_|CatalogError::Database)? {
                builder.push(Pair::decode(&row)?)?;
            }
            let result=builder.finish()?;
            client.batch_execute("COMMIT").await.map_err(|_|CatalogError::Database)?;
            Ok(result)
        })
    }).await
}
struct Pair {
    id: i64,
    ordinal: i64,
    count: i32,
    name: String,
    schema: String,
    table: String,
    source: String,
    target: String,
    update: String,
    delete: String,
}
impl Pair {
    fn decode(row: &tokio_postgres::Row) -> Result<Self, CatalogError> {
        fn text(row: &tokio_postgres::Row, key: &str) -> Result<String, CatalogError> {
            let value: &str = row
                .try_get(key)
                .map_err(|_| CatalogError::InvalidResponse)?;
            if value.is_empty() || value.len() > MAX_TEXT_BYTES {
                return Err(CatalogError::InvalidResponse);
            }
            Ok(value.into())
        }
        let count: i32 = row
            .try_get("key_count")
            .map_err(|_| CatalogError::InvalidResponse)?;
        let target_count: i32 = row
            .try_get("target_count")
            .map_err(|_| CatalogError::InvalidResponse)?;
        if count != target_count {
            return Err(CatalogError::InvalidResponse);
        }
        Ok(Self {
            id: row
                .try_get("constraint_id")
                .map_err(|_| CatalogError::InvalidResponse)?,
            ordinal: row
                .try_get("ordinal")
                .map_err(|_| CatalogError::InvalidResponse)?,
            count,
            name: text(row, "name")?,
            schema: text(row, "referenced_schema")?,
            table: text(row, "referenced_table")?,
            source: text(row, "source")?,
            target: text(row, "target")?,
            update: text(row, "on_update")?,
            delete: text(row, "on_delete")?,
        })
    }
}
#[derive(Default)]
struct Builder {
    keys: Vec<ForeignKey>,
    last: Option<(i64, i32)>,
    pairs: usize,
    bytes: usize,
}
impl Builder {
    fn push(&mut self, pair: Pair) -> Result<(), CatalogError> {
        if pair.id <= 0
            || pair.ordinal <= 0
            || pair.count <= 0
            || pair.count > 64
            || pair.ordinal > i64::from(pair.count)
        {
            return Err(CatalogError::InvalidResponse);
        }
        self.pairs += 1;
        if self.pairs > MAX_FOREIGN_KEY_PAIRS {
            return Err(CatalogError::ForeignKeyLimit);
        }
        let update = action(&pair.update)?;
        let delete = action(&pair.delete)?;
        if self.last.map(|(id, _)| id) != Some(pair.id) {
            self.complete()?;
            if self.last.is_some_and(|(id, _)| pair.id <= id) || pair.ordinal != 1 {
                return Err(CatalogError::InvalidResponse);
            }
            if self.keys.len() >= MAX_FOREIGN_KEYS {
                return Err(CatalogError::ForeignKeyLimit);
            }
            let key = ForeignKey {
                name: pair.name,
                columns: vec![pair.source],
                referenced_schema: pair.schema,
                referenced_table: pair.table,
                referenced_columns: vec![pair.target],
                on_update: Some(update.into()),
                on_delete: Some(delete.into()),
            };
            self.bytes = self.bytes.saturating_add(
                serde_json::to_vec(&key)
                    .map_err(|_| CatalogError::InvalidResponse)?
                    .len()
                    + 1,
            );
            self.keys.push(key);
            self.last = Some((pair.id, pair.count));
        } else {
            let key = self.keys.last_mut().ok_or(CatalogError::InvalidResponse)?;
            if self.last.map(|(_, count)| count) != Some(pair.count)
                || pair.ordinal != key.columns.len() as i64 + 1
                || key.name != pair.name
                || key.referenced_schema != pair.schema
                || key.referenced_table != pair.table
                || key.on_update.as_deref() != Some(update)
                || key.on_delete.as_deref() != Some(delete)
                || key.columns.contains(&pair.source)
                || key.referenced_columns.contains(&pair.target)
            {
                return Err(CatalogError::InvalidResponse);
            }
            self.bytes = self.bytes.saturating_add(
                serde_json::to_vec(&(&pair.source, &pair.target))
                    .map_err(|_| CatalogError::InvalidResponse)?
                    .len(),
            );
            key.columns.push(pair.source);
            key.referenced_columns.push(pair.target);
        }
        // Count outer brackets and commas conservatively before accepting more.
        if self.bytes > MAX_FOREIGN_KEY_BYTES - 2 {
            return Err(CatalogError::ForeignKeyLimit);
        }
        Ok(())
    }
    fn complete(&self) -> Result<(), CatalogError> {
        if self.last.is_some_and(|(_, count)| {
            self.keys
                .last()
                .is_none_or(|key| key.columns.len() != count as usize)
        }) {
            return Err(CatalogError::InvalidResponse);
        }
        Ok(())
    }
    fn finish(self) -> Result<Vec<ForeignKey>, CatalogError> {
        self.complete()?;
        if serde_json::to_vec(&self.keys)
            .map_err(|_| CatalogError::InvalidResponse)?
            .len()
            > MAX_FOREIGN_KEY_BYTES
        {
            return Err(CatalogError::ForeignKeyLimit);
        }
        Ok(self.keys)
    }
}
fn action(code: &str) -> Result<&'static str, CatalogError> {
    match code {
        "a" => Ok("NO ACTION"),
        "r" => Ok("RESTRICT"),
        "c" => Ok("CASCADE"),
        "n" => Ok("SET NULL"),
        "d" => Ok("SET DEFAULT"),
        _ => Err(CatalogError::InvalidResponse),
    }
}
// PostgreSQL name fields are bounded by the server's identifier length. This
// query returns one pair at a time, preserving conkey/confkey ordinality. The
// extra row is an overflow sentinel, never a successful partial result.
const QUERY: &str = r#"
SELECT con.oid::bigint AS constraint_id, con.conname::text AS name,
 nr.nspname::text AS referenced_schema, cr.relname::text AS referenced_table,
 con.confupdtype::text AS on_update, con.confdeltype::text AS on_delete,
 cardinality(con.conkey) AS key_count, cardinality(con.confkey) AS target_count,
 pair.ordinality AS ordinal, source.attname::text AS source, target.attname::text AS target
FROM pg_constraint con
JOIN pg_class c ON c.oid=con.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace
JOIN pg_class cr ON cr.oid=con.confrelid JOIN pg_namespace nr ON nr.oid=cr.relnamespace
CROSS JOIN LATERAL unnest(con.conkey) WITH ORDINALITY AS pair(attnum,ordinality)
LEFT JOIN pg_attribute source ON source.attrelid=con.conrelid AND source.attnum=pair.attnum AND NOT source.attisdropped
LEFT JOIN pg_attribute target ON target.attrelid=con.confrelid AND target.attnum=con.confkey[pair.ordinality::int] AND NOT target.attisdropped
WHERE con.contype='f' AND n.nspname=$1 AND c.relname=$2
ORDER BY con.oid,pair.ordinality LIMIT 4097
"#;
#[cfg(test)]
mod tests {
    use super::*;
    fn pair(id: i64, ordinal: i64, count: i32) -> Pair {
        Pair {
            id,
            ordinal,
            count,
            name: format!("fk{id}"),
            schema: "odd\"schema".into(),
            table: "target.table".into(),
            source: format!("source{ordinal}"),
            target: format!("target{}", count as i64 - ordinal),
            update: "a".into(),
            delete: "c".into(),
        }
    }
    #[test]
    fn composite_order_and_literal_identity_are_preserved() {
        let mut builder = Builder::default();
        builder.push(pair(1, 1, 2)).unwrap();
        builder.push(pair(1, 2, 2)).unwrap();
        builder.push(pair(2, 1, 1)).unwrap();
        let keys = builder.finish().unwrap();
        assert_eq!(keys[0].columns, ["source1", "source2"]);
        assert_eq!(keys[0].referenced_columns, ["target1", "target0"]);
        assert_eq!(keys[0].referenced_schema, "odd\"schema");
        assert_eq!(keys[0].on_delete.as_deref(), Some("CASCADE"));
        assert_eq!(keys.len(), 2);
    }
    #[test]
    fn partial_and_duplicate_pairs_refuse_the_whole_read() {
        let mut builder = Builder::default();
        builder.push(pair(1, 1, 2)).unwrap();
        assert!(matches!(
            builder.finish(),
            Err(CatalogError::InvalidResponse)
        ));
        let mut builder = Builder::default();
        builder.push(pair(1, 1, 2)).unwrap();
        let mut second = pair(1, 2, 2);
        second.source = "source1".into();
        assert_eq!(builder.push(second), Err(CatalogError::InvalidResponse));
        assert_eq!(action("unknown"), Err(CatalogError::InvalidResponse));
    }
    #[test]
    fn count_and_encoded_limits_do_not_return_prefixes() {
        let mut builder = Builder::default();
        for id in 1..=MAX_FOREIGN_KEYS {
            builder.push(pair(id as i64, 1, 1)).unwrap()
        }
        assert_eq!(
            builder.push(pair(MAX_FOREIGN_KEYS as i64 + 1, 1, 1)),
            Err(CatalogError::ForeignKeyLimit)
        );
        let mut builder = Builder::default();
        let mut row = pair(1, 1, 1);
        row.source = "\u{1}".repeat(MAX_FOREIGN_KEY_BYTES / 6);
        assert_eq!(builder.push(row), Err(CatalogError::ForeignKeyLimit));
    }
}
