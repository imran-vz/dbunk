use super::super::protocol::{Direction, TargetColumn, TransferError};
use super::{cancellable_pg, sql, JobContext, Review};
use crate::postgres::dedicated::DedicatedConnection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RelationState {
    pub(super) oid: u32,
    pub(super) kind: String,
    pub(super) row_security: bool,
    pub(super) force_row_security: bool,
    pub(super) populated: bool,
    pub(super) columns: Vec<CatalogColumn>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CatalogColumn {
    pub(super) number: i32,
    pub(super) type_oid: u32,
    pub(super) type_modifier: i32,
    pub(super) collation_oid: u32,
    // Retain bounded DDL identity without storing a potentially large or sensitive literal.
    pub(super) default_fingerprint: Option<String>,
    pub(super) public: TargetColumn,
}

pub(super) async fn lock_and_validate(
    context: &JobContext,
    connection: &DedicatedConnection,
    review: &Review,
) -> Result<(), TransferError> {
    let relation = sql::qualified(review);
    let statement = match review.payload.direction {
        Direction::Import => format!("LOCK TABLE {relation} IN ROW EXCLUSIVE MODE"),
        // SELECT acquires a transaction-held AccessShareLock for every exportable
        // relation kind, including materialized views and foreign tables that
        // reject LOCK TABLE. No source rows are read before catalog validation.
        Direction::Export => format!("SELECT FROM {relation} LIMIT 0"),
    };
    cancellable_pg(
        context,
        connection,
        connection.client.batch_execute(&statement),
    )
    .await
    .map_err(|error| match error {
        TransferError::Database {
            code: Some(ref code),
            ..
        } if code == "42P01" || code == "42809" => TransferError::TargetChanged,
        other => other,
    })?;
    validate(context, connection, review).await
}

pub(super) async fn validate(
    context: &JobContext,
    connection: &DedicatedConnection,
    review: &Review,
) -> Result<(), TransferError> {
    let current = cancellable(
        context,
        connection,
        &review.payload.schema,
        &review.payload.table,
    )
    .await?;
    if current != review.relation {
        return Err(TransferError::TargetChanged);
    }
    current.ensure_supported(review.payload.direction)
}

async fn cancellable(
    context: &JobContext,
    connection: &DedicatedConnection,
    schema: &str,
    table: &str,
) -> Result<RelationState, TransferError> {
    let relation = cancellable_pg(
        context,
        connection,
        connection
            .client
            .query_opt(RELATION_SQL, &[&schema, &table]),
    )
    .await?
    .ok_or(TransferError::TargetChanged)?;
    let oid = relation.get::<_, u32>(0);
    let columns = cancellable_pg(
        context,
        connection,
        connection.client.query(COLUMNS_SQL, &[&oid]),
    )
    .await?;
    Ok(from_rows(relation, columns))
}

pub(super) async fn inspect(
    connection: &DedicatedConnection,
    schema: &str,
    table: &str,
) -> Result<RelationState, TransferError> {
    let relation = connection
        .client
        .query_opt(RELATION_SQL, &[&schema, &table])
        .await
        .map_err(|error| TransferError::database(&error))?
        .ok_or_else(|| TransferError::UnsupportedTarget {
            reason: "The target relation does not exist".into(),
        })?;
    let oid = relation.get::<_, u32>(0);
    let columns = connection
        .client
        .query(COLUMNS_SQL, &[&oid])
        .await
        .map_err(|error| TransferError::database(&error))?;
    Ok(from_rows(relation, columns))
}

fn from_rows(relation: tokio_postgres::Row, columns: Vec<tokio_postgres::Row>) -> RelationState {
    RelationState {
        oid: relation.get(0),
        kind: relation.get(1),
        row_security: relation.get(2),
        force_row_security: relation.get(3),
        populated: relation.get(4),
        columns: columns
            .into_iter()
            .map(|column| {
                let default_fingerprint = column.get::<_, Option<String>>(7);
                CatalogColumn {
                    number: column.get(0),
                    type_oid: column.get(3),
                    type_modifier: column.get(4),
                    collation_oid: column.get(5),
                    public: TargetColumn {
                        name: column.get(1),
                        data_type: column.get(2),
                        nullable: !column.get::<_, bool>(6),
                        has_default: default_fingerprint.is_some(),
                        generated: !column.get::<_, String>(8).is_empty(),
                        identity: !column.get::<_, String>(9).is_empty(),
                    },
                    default_fingerprint,
                }
            })
            .collect(),
    }
}

impl RelationState {
    pub(super) fn ensure_supported(&self, direction: Direction) -> Result<(), TransferError> {
        match direction {
            Direction::Import if !matches!(self.kind.as_str(), "r" | "p") => {
                Err(TransferError::UnsupportedTarget {
                    reason: "Imports require an ordinary or partitioned table".into(),
                })
            }
            Direction::Import if self.row_security || self.force_row_security => {
                Err(TransferError::UnsupportedTarget {
                    reason: "Imports into row-level-security tables are not supported".into(),
                })
            }
            Direction::Export if !matches!(self.kind.as_str(), "r" | "p" | "v" | "m" | "f") => {
                Err(TransferError::UnsupportedTarget {
                    reason: "This relation type cannot be exported".into(),
                })
            }
            Direction::Export if self.kind == "m" && !self.populated => {
                Err(TransferError::UnsupportedTarget {
                    reason: "An unpopulated materialized view cannot be exported".into(),
                })
            }
            _ => Ok(()),
        }
    }
}

const RELATION_SQL: &str = "
SELECT c.oid, c.relkind::text, c.relrowsecurity, c.relforcerowsecurity, c.relispopulated
FROM pg_catalog.pg_class AS c
JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relname = $2
";

const COLUMNS_SQL: &str = "
SELECT a.attnum::integer,
       a.attname,
       pg_catalog.format_type(a.atttypid, a.atttypmod),
       a.atttypid,
       a.atttypmod,
       a.attcollation,
       a.attnotnull,
       pg_catalog.encode(
         pg_catalog.sha256(pg_catalog.convert_to(d.adbin::text, 'UTF8')),
         'hex'
       ),
       a.attgenerated::text,
       a.attidentity::text
FROM pg_catalog.pg_attribute AS a
LEFT JOIN pg_catalog.pg_attrdef AS d
  ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
";
