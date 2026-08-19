use std::sync::Arc;
use std::time::Instant;

use super::builder::{build_browse_query, BuiltBrowseQuery, RelationDescriptor};
use super::protocol::*;
use super::{builder, postgres};

pub(crate) async fn run_kind(
    executor: &super::executor::Executor,
    job: &super::executor::Job,
) -> Result<super::executor::JobResult, TableBrowseError> {
    match &job.kind {
        super::executor::JobKind::Browse(payload) => run_browse(executor, payload)
            .await
            .map(|result| super::executor::JobResult::Browse(Box::new(result))),
        super::executor::JobKind::Count(payload) => run_count(executor, payload)
            .await
            .map(super::executor::JobResult::Count),
    }
}

async fn ensure_connection(executor: &super::executor::Executor) -> Result<(), TableBrowseError> {
    {
        let mut inner = executor.inner.lock().await;
        if let Some(connection) = inner.connection.as_ref() {
            if !connection.inner.is_closed() {
                return Ok(());
            }
        }
        inner.connection = None;
    }
    let connection = postgres::connect(&executor.spec).await?;
    let mut inner = executor.inner.lock().await;
    if inner.closed {
        return Err(TableBrowseError::ConnectionClosing);
    }
    inner.connection = Some(connection);
    Ok(())
}

async fn browse_client(
    executor: &super::executor::Executor,
) -> Result<Arc<tokio_postgres::Client>, TableBrowseError> {
    executor
        .inner
        .lock()
        .await
        .connection
        .as_ref()
        .map(|connection| connection.inner.client.clone())
        .ok_or(TableBrowseError::ConnectionLost)
}

async fn descriptor(
    executor: &super::executor::Executor,
    schema: &str,
    table: &str,
    refresh: bool,
) -> Result<RelationDescriptor, TableBrowseError> {
    ensure_connection(executor).await?;
    if !refresh {
        if let Some(cached) = executor
            .inner
            .lock()
            .await
            .cache
            .get(&(schema.to_string(), table.to_string()))
            .cloned()
        {
            return Ok(cached);
        }
    }
    let client = browse_client(executor).await?;
    let descriptor = postgres::load_descriptor(client.as_ref(), schema, table).await?;
    executor
        .inner
        .lock()
        .await
        .cache
        .insert((schema.into(), table.into()), descriptor.clone());
    Ok(descriptor)
}

async fn invalidate_descriptor(executor: &super::executor::Executor, schema: &str, table: &str) {
    executor
        .inner
        .lock()
        .await
        .cache
        .remove(&(schema.to_string(), table.to_string()));
}

async fn run_browse(
    executor: &super::executor::Executor,
    payload: &BrowseTableDataPayload,
) -> Result<BrowseTableResult, TableBrowseError> {
    let started = Instant::now();
    let mut retried = false;
    loop {
        let descriptor = descriptor(
            executor,
            &payload.schema,
            &payload.table,
            payload.refresh_structure || retried,
        )
        .await?;
        let built = build_browse_query(&descriptor, payload)?;
        let client = browse_client(executor).await?;
        let executed = match postgres::execute_browse(client.as_ref(), &built).await {
            Ok(executed) => Ok(executed),
            Err(error) if postgres::is_undefined_object(&error) && !retried => Err(error),
            Err(error) => return Err(error),
        };
        match executed {
            Ok(executed) => {
                let count = browse_count(executor, payload, &built).await?;
                let next_cursor = next_cursor(&built, &executed.row_identity, executed.has_more);
                let inspection = built.inspection();
                return Ok(BrowseTableResult {
                    request_id: payload.request_id,
                    columns: built.visible_columns,
                    rows: executed.rows,
                    identity: built.identity,
                    row_identity: executed.row_identity,
                    page_info: BrowsePageInfo {
                        mode: built.page_mode,
                        page: built.page,
                        has_more: executed.has_more,
                        next_cursor,
                    },
                    count,
                    inspection,
                    omitted_rows: executed.omitted_rows,
                    truncated_cells: executed.truncated_cells,
                    runtime_ms: started.elapsed().as_millis() as u64,
                });
            }
            Err(_) => {
                invalidate_descriptor(executor, &payload.schema, &payload.table).await;
                retried = true;
            }
        }
    }
}

async fn browse_count(
    executor: &super::executor::Executor,
    payload: &BrowseTableDataPayload,
    built: &BuiltBrowseQuery,
) -> Result<BrowseCount, TableBrowseError> {
    match payload.count_policy {
        BrowseCountPolicy::None => Ok(BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        }),
        BrowseCountPolicy::Estimated if built.where_sql.is_empty() => {
            let client = browse_client(executor).await?;
            postgres::estimated_unfiltered_count(client.as_ref(), &payload.schema, &payload.table)
                .await
        }
        BrowseCountPolicy::Estimated => {
            let client = browse_client(executor).await?;
            postgres::estimated_filtered_count(
                client.as_ref(),
                &built.explain_sql(),
                &built.where_params,
            )
            .await
        }
    }
}

async fn run_count(
    executor: &super::executor::Executor,
    payload: &CountTableBrowseRowsPayload,
) -> Result<BrowseExactCountResult, TableBrowseError> {
    let mut retried = false;
    loop {
        let descriptor = descriptor(executor, &payload.schema, &payload.table, retried).await?;
        let (sql, params) = builder::build_count_query(&descriptor, &payload.filters)?;
        let client = browse_client(executor).await?;
        match postgres::execute_count(client.as_ref(), &sql, &params).await {
            Ok(value) => {
                return Ok(BrowseExactCountResult {
                    kind: BrowseCountKind::Exact,
                    value,
                    request_id: payload.request_id,
                });
            }
            Err(error) if postgres::is_undefined_object(&error) && !retried => {
                invalidate_descriptor(executor, &payload.schema, &payload.table).await;
                retried = true;
            }
            Err(error) => return Err(error),
        }
    }
}

fn next_cursor(
    built: &BuiltBrowseQuery,
    row_identity: &Option<Vec<Vec<String>>>,
    has_more: bool,
) -> Option<BrowseCursor> {
    if built.page_mode != BrowsePageMode::Keyset || !has_more {
        return None;
    }
    row_identity
        .as_ref()
        .and_then(|rows| rows.last())
        .cloned()
        .map(|values| BrowseCursor { values })
}
