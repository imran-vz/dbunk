use super::*;
use crate::postgres::transfer::runner::catalog::{self, RelationState};
use crate::{ColumnInfo, ForeignKeyInfo, IndexInfo, StructureCapabilities, TableStructure};
use futures_util::{pin_mut, TryStreamExt};
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Key {
    pub name: String,
    pub kind: String,
    pub columns: Vec<String>,
    pub parent_oid: u32,
    pub parent_schema: String,
    pub parent_table: String,
    pub parent_columns: Vec<String>,
    pub fingerprint: String,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub primary: bool,
    pub fingerprint: String,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Catalog {
    pub database: u32,
    pub relation: RelationState,
    pub keys: Vec<Key>,
    pub indexes: Vec<Index>,
    pub serial_columns: Vec<String>,
}
impl Catalog {
    pub fn structure(&self) -> TableStructure {
        let primary_key = self
            .indexes
            .iter()
            .find(|i| i.primary)
            .map(|i| i.columns.clone());
        TableStructure {
            columns: self
                .relation
                .columns
                .iter()
                .map(|c| ColumnInfo {
                    name: c.public.name.clone(),
                    data_type: c.public.data_type.clone(),
                    nullable: c.public.nullable,
                    default_value: c.public.has_default.then(|| {
                        if self.serial_columns.contains(&c.public.name) {
                            "nextval(owned sequence)".into()
                        } else {
                            "database default".into()
                        }
                    }),
                    is_primary_key: primary_key
                        .as_ref()
                        .is_some_and(|p| p.contains(&c.public.name)),
                    ordinal_position: c.number,
                    derivation_kind: (c.public.identity || c.public.generated)
                        .then(|| "database supplied".into()),
                })
                .collect(),
            primary_key,
            foreign_keys: self
                .keys
                .iter()
                .filter(|k| k.kind == "f")
                .map(|k| ForeignKeyInfo {
                    name: k.name.clone(),
                    columns: k.columns.clone(),
                    referenced_schema: k.parent_schema.clone(),
                    referenced_table: k.parent_table.clone(),
                    referenced_columns: k.parent_columns.clone(),
                    on_update: None,
                    on_delete: None,
                })
                .collect(),
            indexes: self
                .indexes
                .iter()
                .map(|i| IndexInfo {
                    name: i.name.clone(),
                    columns: i.columns.clone(),
                    is_unique: true,
                    is_primary: i.primary,
                    method: None,
                })
                .collect(),
            constraints: vec![],
            triggers: vec![],
            policies: vec![],
            privileges: vec![],
            row_security: None,
            table_engine: None,
            partition_by: None,
            sample_by: None,
            capabilities: StructureCapabilities {
                columns: true,
                primary_key: true,
                foreign_keys: true,
                indexes: true,
                constraints: true,
                triggers: false,
                policies: false,
                privileges: false,
                can_insert_rows: true,
                can_update_rows: false,
                can_delete_rows: false,
                can_alter_schema: false,
                uniqueness_guarantee: "database enforced".into(),
            },
        }
    }
}
pub(super) async fn inspect(
    connection: &DedicatedConnection,
    endpoint: &TableSeedEndpoint,
) -> Result<Catalog, Failure> {
    let relation = catalog::inspect(connection, &endpoint.schema, &endpoint.table, true)
        .await
        .map_err(|e| -> Failure {
            match e {
                crate::postgres::transfer::protocol::TransferError::InvalidRequest { .. } => {
                    TableSeedError::Limit.into()
                }
                _ => TableSeedError::UnsupportedTarget.into(),
            }
        })?;
    relation
        .ensure_supported(crate::postgres::transfer::protocol::Direction::Import)
        .map_err(|_| TableSeedError::UnsupportedTarget)?;
    let database = connection
        .client
        .query_one(
            "SELECT oid FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database()",
            &[],
        )
        .await
        .map_err(database_error)?
        .get(0);
    let stream = connection
        .client
        .query_raw(
            KEYS,
            [&relation.oid as &(dyn tokio_postgres::types::ToSql + Sync)],
        )
        .await
        .map_err(database_error)?;
    pin_mut!(stream);
    let mut keys = Vec::new();
    let mut bytes = 0usize;
    while let Some(row) = stream.try_next().await.map_err(database_error)? {
        if keys.len() >= 512 {
            return Err(TableSeedError::Limit.into());
        }
        let columns: Vec<String> = row
            .get::<_, Option<Vec<String>>>(2)
            .ok_or(TableSeedError::Limit)?;
        let parent_columns: Vec<String> = row
            .get::<_, Option<Vec<String>>>(6)
            .ok_or(TableSeedError::Limit)?;
        let key = Key {
            name: row.get(0),
            kind: row.get(1),
            columns,
            parent_oid: row.get(3),
            parent_schema: row.get(4),
            parent_table: row.get(5),
            parent_columns,
            fingerprint: row.get(7),
        };
        if key
            .columns
            .iter()
            .chain(&key.parent_columns)
            .any(|s| s.len() > 63)
            || key.kind == "f"
                && (key.columns.is_empty() || key.columns.len() != key.parent_columns.len())
        {
            return Err(TableSeedError::UnsupportedTarget.into());
        }
        bytes = bytes
            .saturating_add(
                key.columns
                    .iter()
                    .chain(&key.parent_columns)
                    .map(|s| s.capacity() + std::mem::size_of::<String>())
                    .sum::<usize>(),
            )
            .saturating_add(512);
        if bytes > 512 * 1024 {
            return Err(TableSeedError::Limit.into());
        }
        keys.push(key);
    }
    let stream = connection
        .client
        .query_raw(
            INDEXES,
            [&relation.oid as &(dyn tokio_postgres::types::ToSql + Sync)],
        )
        .await
        .map_err(database_error)?;
    pin_mut!(stream);
    let mut indexes = Vec::new();
    while let Some(row) = stream.try_next().await.map_err(database_error)? {
        if indexes.len() >= 256 {
            return Err(TableSeedError::Limit.into());
        }
        let columns = row
            .get::<_, Option<Vec<String>>>(1)
            .ok_or(TableSeedError::Limit)?;
        let index = Index {
            name: row.get(0),
            columns,
            primary: row.get(2),
            fingerprint: row.get(3),
        };
        bytes = bytes
            .saturating_add(
                index
                    .columns
                    .iter()
                    .map(|s| s.capacity() + std::mem::size_of::<String>())
                    .sum::<usize>(),
            )
            .saturating_add(256);
        if bytes > 512 * 1024 {
            return Err(TableSeedError::Limit.into());
        }
        indexes.push(index);
    }
    let serial_rows=connection.client.query("SELECT a.attname::text FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_attrdef d ON d.adrelid=a.attrelid AND d.adnum=a.attnum WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped AND pg_catalog.pg_get_expr(d.adbin,d.adrelid) LIKE 'nextval(%' ORDER BY a.attnum LIMIT 1601",&[&relation.oid]).await.map_err(database_error)?;
    if serial_rows.len() > 1600 {
        return Err(TableSeedError::Limit.into());
    }
    let serial_columns = serial_rows.into_iter().map(|r| r.get(0)).collect();
    Ok(Catalog {
        database,
        relation,
        keys,
        indexes,
        serial_columns,
    })
}
const KEYS: &str = r#"SELECT c.conname::text,c.contype::text,
CASE WHEN cardinality(c.conkey)<=32 OR c.conkey IS NULL THEN ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(n,o) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.n ORDER BY k.o) END,
c.confrelid,coalesce(n.nspname::text,''),coalesce(p.relname::text,''),
CASE WHEN cardinality(c.confkey)<=32 OR c.confkey IS NULL THEN ARRAY(SELECT a.attname::text FROM unnest(c.confkey) WITH ORDINALITY k(n,o) JOIN pg_catalog.pg_attribute a ON a.attrelid=c.confrelid AND a.attnum=k.n ORDER BY k.o) END,
pg_catalog.md5(pg_catalog.pg_get_constraintdef(c.oid,true)||c.convalidated::text||c.condeferrable::text||c.condeferred::text)
FROM pg_catalog.pg_constraint c LEFT JOIN pg_catalog.pg_class p ON p.oid=c.confrelid LEFT JOIN pg_catalog.pg_namespace n ON n.oid=p.relnamespace WHERE c.conrelid=$1 ORDER BY c.oid LIMIT 513"#;
const INDEXES: &str = r#"SELECT x.relname::text,
CASE WHEN i.indnkeyatts<=32 THEN CASE WHEN i.indpred IS NOT NULL OR i.indexprs IS NOT NULL THEN ARRAY[]::text[] ELSE ARRAY(SELECT a.attname::text FROM unnest(i.indkey::smallint[]) WITH ORDINALITY k(n,o) JOIN pg_catalog.pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.n WHERE k.o<=i.indnkeyatts ORDER BY k.o) END END,
i.indisprimary,pg_catalog.md5(pg_catalog.pg_get_indexdef(i.indexrelid)||i.indisvalid::text||i.indisready::text)
FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class x ON x.oid=i.indexrelid WHERE i.indrelid=$1 AND i.indisunique ORDER BY i.indexrelid LIMIT 257"#;
