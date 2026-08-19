use tokio_postgres::error::ErrorPosition;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Row};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::query_session::postgres::{self as session_postgres, SessionConnection};

use super::builder::{
    BoundParam, BuiltBrowseQuery, RelationColumn, RelationDescriptor, UniqueIndexCandidate,
};
use super::protocol::*;

const MAX_CELL_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct BrowseConnection {
    pub inner: SessionConnection,
    pub tls: bool,
}

pub(crate) async fn connect(
    spec: &ResolvedPostgresConnectSpec,
) -> Result<BrowseConnection, TableBrowseError> {
    let inner = session_postgres::connect(spec)
        .await
        .map_err(map_session_error)?;
    inner
        .client
        .batch_execute("SET default_transaction_read_only = on")
        .await
        .map_err(database_error)?;
    Ok(BrowseConnection {
        tls: spec.tls_prefer,
        inner,
    })
}

pub(crate) async fn load_descriptor(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<RelationDescriptor, TableBrowseError> {
    let header = client
        .query_opt(
            r#"
            SELECT c.relkind::text, current_setting('server_version_num')::int
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = $2
              AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
            "#,
            &[&schema, &table],
        )
        .await
        .map_err(database_error)?;
    let Some(header) = header else {
        return Err(undefined_table());
    };
    let relkind: String = header.get(0);
    let server_version_num: i32 = header.get(1);
    let relkind = relkind.chars().next().unwrap_or('r');

    let column_rows = client
        .query(
            r#"
            SELECT a.attname::text,
                   pg_catalog.format_type(a.atttypid, a.atttypmod) AS cast_type,
                   NOT a.attnotnull AS nullable
            FROM pg_attribute a
            JOIN pg_class c ON c.oid = a.attrelid
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = $2
              AND a.attnum > 0 AND NOT a.attisdropped
            ORDER BY a.attnum
            "#,
            &[&schema, &table],
        )
        .await
        .map_err(database_error)?;
    let columns = column_rows
        .into_iter()
        .map(|row| RelationColumn {
            name: row.get(0),
            cast_type: row.get(1),
            nullable: row.get(2),
        })
        .collect::<Vec<_>>();

    let pk_rows = client
        .query(
            r#"
            SELECT a.attname::text
            FROM pg_index ix
            JOIN pg_class t ON t.oid = ix.indrelid
            JOIN pg_namespace n ON n.oid = t.relnamespace
            JOIN unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord) ON true
            JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = u.attnum
            WHERE n.nspname = $1 AND t.relname = $2 AND ix.indisprimary
            ORDER BY u.ord
            "#,
            &[&schema, &table],
        )
        .await
        .map_err(database_error)?;
    let primary_key = pk_rows
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();

    let unique_rows = client
        .query(
            r#"
            SELECT i.relname::text,
                   array_agg(a.attname::text ORDER BY u.ord) AS columns
            FROM pg_index ix
            JOIN pg_class t ON t.oid = ix.indrelid
            JOIN pg_namespace n ON n.oid = t.relnamespace
            JOIN pg_class i ON i.oid = ix.indexrelid
            JOIN unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord) ON true
            JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = u.attnum
            WHERE n.nspname = $1 AND t.relname = $2
              AND ix.indisvalid AND ix.indisunique AND NOT ix.indisprimary
              AND ix.indpred IS NULL AND ix.indexprs IS NULL
            GROUP BY i.relname
            "#,
            &[&schema, &table],
        )
        .await
        .map_err(database_error)?;
    let non_nullable = columns
        .iter()
        .filter(|column| !column.nullable)
        .map(|column| column.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let unique_indexes = unique_rows
        .into_iter()
        .filter_map(|row| {
            let name: String = row.get(0);
            let index_columns: Vec<String> = row.get(1);
            if index_columns.is_empty()
                || !index_columns
                    .iter()
                    .all(|column| non_nullable.contains(column.as_str()))
            {
                return None;
            }
            Some(UniqueIndexCandidate {
                name,
                columns: index_columns,
            })
        })
        .collect();

    Ok(RelationDescriptor {
        schema: schema.into(),
        table: table.into(),
        relkind,
        server_version_num,
        columns,
        primary_key,
        unique_indexes,
    })
}

pub(crate) struct ExecutedBrowse {
    pub rows: Vec<Vec<Option<String>>>,
    pub row_identity: Option<Vec<Vec<String>>>,
    pub has_more: bool,
    pub omitted_rows: u64,
    pub truncated_cells: u64,
}

pub(crate) async fn execute_browse(
    client: &Client,
    built: &BuiltBrowseQuery,
) -> Result<ExecutedBrowse, TableBrowseError> {
    let rows = query_rows(client, &built.sql, &built.params).await?;
    decode_browse_rows(rows, built)
}

pub(crate) async fn execute_count(
    client: &Client,
    sql: &str,
    params: &[BoundParam],
) -> Result<u64, TableBrowseError> {
    let rows = query_rows(client, sql, params).await?;
    let Some(row) = rows.first() else {
        return Ok(0);
    };
    parse_count_cell(row)
}

pub(crate) async fn estimated_unfiltered_count(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<BrowseCount, TableBrowseError> {
    let rows = client
        .query(
            r#"
            SELECT c.reltuples
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = $2
            "#,
            &[&schema, &table],
        )
        .await
        .map_err(database_error)?;
    let Some(row) = rows.first() else {
        return Ok(BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        });
    };
    let reltuples: f32 = row.get(0);
    if reltuples <= 0.0 {
        Ok(BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        })
    } else {
        Ok(BrowseCount {
            kind: BrowseCountKind::Estimated,
            value: Some(reltuples.round() as u64),
        })
    }
}

pub(crate) async fn estimated_filtered_count(
    client: &Client,
    sql: &str,
    params: &[BoundParam],
) -> Result<BrowseCount, TableBrowseError> {
    let rows = query_rows(client, sql, params).await?;
    let Some(row) = rows.first() else {
        return Ok(BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        });
    };
    let plan = explain_json(row)?;
    let plan_rows = plan
        .get(0)
        .and_then(|entry| entry.get("Plan"))
        .and_then(|plan| plan.get("Plan Rows"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_f64().map(|number| number.round() as u64))
        });
    Ok(BrowseCount {
        kind: if plan_rows.is_some() {
            BrowseCountKind::Estimated
        } else {
            BrowseCountKind::Unknown
        },
        value: plan_rows,
    })
}

pub(crate) fn is_undefined_object(error: &TableBrowseError) -> bool {
    matches!(error.sqlstate(), Some("42703" | "42P01"))
}

pub(crate) fn is_query_canceled(error: &TableBrowseError) -> bool {
    error.sqlstate() == Some("57014")
}

pub(crate) fn database_error(error: tokio_postgres::Error) -> TableBrowseError {
    if let Some(db) = error.as_db_error() {
        TableBrowseError::Database {
            code: Some(db.code().code().into()),
            message: db.message().into(),
            severity: Some(db.severity().into()),
            position: match db.position() {
                Some(ErrorPosition::Original(pos)) => Some(*pos),
                _ => None,
            },
        }
    } else {
        TableBrowseError::ConnectionLost
    }
}

fn map_session_error(error: crate::query_session::protocol::QuerySessionError) -> TableBrowseError {
    use crate::query_session::protocol::QuerySessionError;
    match error {
        QuerySessionError::UnsupportedEngine => TableBrowseError::UnsupportedEngine,
        QuerySessionError::ConnectionClosing => TableBrowseError::ConnectionClosing,
        QuerySessionError::ConnectionLost => TableBrowseError::ConnectionLost,
        QuerySessionError::Timeout { operation } => TableBrowseError::Timeout { operation },
        QuerySessionError::Database {
            code,
            message,
            severity,
            position,
        } => TableBrowseError::Database {
            code,
            message,
            severity,
            position,
        },
        _ => TableBrowseError::ConnectionLost,
    }
}

fn undefined_table() -> TableBrowseError {
    TableBrowseError::Database {
        code: Some("42P01".into()),
        message: "undefined table".into(),
        severity: Some("ERROR".into()),
        position: None,
    }
}

async fn query_rows(
    client: &Client,
    sql: &str,
    params: &[BoundParam],
) -> Result<Vec<Row>, TableBrowseError> {
    let refs = param_refs(params);
    client.query(sql, &refs).await.map_err(database_error)
}

fn param_refs(params: &[BoundParam]) -> Vec<&(dyn ToSql + Sync)> {
    params
        .iter()
        .map(|param| match param {
            BoundParam::Text(value) => value as &(dyn ToSql + Sync),
            BoundParam::TextArray(values) => values as &(dyn ToSql + Sync),
        })
        .collect()
}

fn decode_browse_rows(
    rows: Vec<Row>,
    built: &BuiltBrowseQuery,
) -> Result<ExecutedBrowse, TableBrowseError> {
    let extra = rows.len() as u32 > built.page_size;
    let take = built.page_size as usize;
    let mut decoded = Vec::new();
    let mut identities = Vec::new();
    let mut omitted_rows = 0_u64;
    let mut truncated_cells = 0_u64;
    let mut retained_bytes = 0_usize;
    let mut has_more = extra;
    for (index, row) in rows.into_iter().enumerate() {
        if index >= take {
            break;
        }
        let (cells, identity) = decode_row(&row, built, &mut truncated_cells);
        let bytes = serde_json::to_vec(&cells)
            .map(|json| json.len())
            .unwrap_or(usize::MAX);
        if retained_bytes.saturating_add(bytes) > MAX_RESPONSE_BYTES {
            omitted_rows += (take.saturating_sub(index)) as u64;
            has_more = true;
            break;
        }
        retained_bytes += bytes;
        if let Some(identity) = identity {
            identities.push(identity);
        }
        decoded.push(cells);
    }
    Ok(ExecutedBrowse {
        rows: decoded,
        row_identity: built.identity.exists().then_some(identities),
        has_more,
        omitted_rows,
        truncated_cells,
    })
}

fn decode_row(
    row: &Row,
    built: &BuiltBrowseQuery,
    truncated_cells: &mut u64,
) -> (Vec<Option<String>>, Option<Vec<String>>) {
    let visible = built.visible_columns.len();
    let mut cells = (0..visible)
        .map(|index| cell_text(row, index))
        .collect::<Vec<_>>();
    let identity = if built.projects_ctid {
        Some(vec![cell_text(row, visible).unwrap_or_default()])
    } else if built.identity.exists() {
        Some(
            built
                .identity
                .columns
                .iter()
                .map(|name| {
                    built
                        .visible_columns
                        .iter()
                        .position(|column| column.name == *name)
                        .and_then(|index| cells.get(index).cloned().flatten())
                        .unwrap_or_default()
                })
                .collect(),
        )
    } else {
        None
    };
    let mut reasons = Vec::new();
    for value in cells.iter_mut().flatten() {
        let truncated = session_postgres::truncate_utf8(value, MAX_CELL_BYTES, &mut reasons);
        if truncated.len() != value.len() {
            *truncated_cells += 1;
        }
        *value = truncated;
    }
    let before = reasons.len();
    session_postgres::shrink_row(&mut cells, &mut reasons);
    if reasons.len() > before {
        *truncated_cells += 1;
    }
    (cells, identity)
}

fn cell_text(row: &Row, index: usize) -> Option<String> {
    row.try_get::<_, Option<String>>(index)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<_, Option<&str>>(index)
                .ok()
                .flatten()
                .map(str::to_string)
        })
}

fn parse_count_cell(row: &Row) -> Result<u64, TableBrowseError> {
    if let Ok(value) = row.try_get::<_, i64>(0) {
        return Ok(value.max(0) as u64);
    }
    if let Ok(Some(value)) = row.try_get::<_, Option<i64>>(0) {
        return Ok(value.max(0) as u64);
    }
    if let Ok(Some(text)) = row.try_get::<_, Option<String>>(0) {
        return text
            .parse::<u64>()
            .map_err(|_| TableBrowseError::ConnectionLost);
    }
    Ok(0)
}

fn explain_json(row: &Row) -> Result<serde_json::Value, TableBrowseError> {
    if let Ok(value) = row.try_get::<_, serde_json::Value>(0) {
        return Ok(value);
    }
    if let Ok(text) = row.try_get::<_, String>(0) {
        return serde_json::from_str(&text).map_err(|_| TableBrowseError::ConnectionLost);
    }
    if let Ok(Some(text)) = row.try_get::<_, Option<String>>(0) {
        return serde_json::from_str(&text).map_err(|_| TableBrowseError::ConnectionLost);
    }
    Err(TableBrowseError::ConnectionLost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undefined_object_sqlstates_trigger_retry() {
        let missing_column = TableBrowseError::Database {
            code: Some("42703".into()),
            message: "undefined column".into(),
            severity: Some("ERROR".into()),
            position: None,
        };
        let missing_table = TableBrowseError::Database {
            code: Some("42P01".into()),
            message: "undefined table".into(),
            severity: Some("ERROR".into()),
            position: None,
        };
        assert!(is_undefined_object(&missing_column));
        assert!(is_undefined_object(&missing_table));
        assert!(!is_undefined_object(&TableBrowseError::Cancelled));
        assert!(is_query_canceled(&TableBrowseError::Database {
            code: Some("57014".into()),
            message: "canceling statement due to user request".into(),
            severity: Some("ERROR".into()),
            position: None,
        }));
    }
}
