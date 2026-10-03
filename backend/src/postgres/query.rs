//! Ad-hoc query execution against PostgreSQL.

use std::time::Instant;

use sqlx::{Column, Executor, Row};

use crate::{dispatch::should_fetch_rows, QueryResult, StoredConnection};

use super::{connect, row_to_strings};

/// Run an ad-hoc query against PG using the native driver.
///
/// Selects (and other row-returning statements) come back with column
/// metadata + row strings; everything else returns `rows_affected`. Routing
/// SELECT vs DML lives in `should_fetch_rows`.
pub async fn run_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();

    if should_fetch_rows(query) {
        let rows = sqlx::query(query)
            .fetch_all(&mut *conn)
            .await
            .map_err(|error| error.to_string())?;
        let columns = if let Some(row) = rows.first() {
            row.columns()
                .iter()
                .map(|column| column.name().to_string())
                .collect::<Vec<_>>()
        } else {
            let describe = (&mut *conn)
                .describe(query)
                .await
                .map_err(|error| error.to_string())?;
            describe
                .columns()
                .iter()
                .map(|column| column.name().to_string())
                .collect::<Vec<_>>()
        };
        let values = rows.iter().map(row_to_strings).collect::<Vec<_>>();
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns,
            rows: values,
            runtime_ms,
            row_count: rows.len() as u64,
        })
    } else {
        let result = sqlx::query(query)
            .execute(&mut *conn)
            .await
            .map_err(|error| error.to_string())?;
        let runtime_ms = start.elapsed().as_millis() as u64;
        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            runtime_ms,
            row_count: result.rows_affected(),
        })
    }
}
