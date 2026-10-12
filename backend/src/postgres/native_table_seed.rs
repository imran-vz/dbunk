//! A seed review freezes recipe, catalog identity, seed and clock. FK samples and
//! current integer maxima are read only inside the eventual write transaction.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
};
use crate::backend::table_seed::*;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
mod catalog;
mod planning;
mod retention;
#[cfg(test)]
pub(crate) mod tests;
use crate::backend::table_seed::engine::{self, EngineCatalog, Execution, Failure, Identity, Plan};
pub(crate) use catalog::Catalog;
fn database_error(error: tokio_postgres::Error) -> Failure {
    match error.as_db_error() {
        Some(e) => Failure::database(Some(e.code().code()), e.constraint(), e.column()),
        None => TableSeedError::Database.into(),
    }
}
/// The reviewed PostgreSQL catalog of a plan; plans reach this runner only
/// for PostgreSQL targets.
fn catalog_of(plan: &Plan) -> Result<&Catalog, Failure> {
    match &plan.catalog {
        EngineCatalog::Postgres(catalog) => Ok(catalog),
        _ => Err(TableSeedError::UnsupportedTarget.into()),
    }
}
fn qualified(e: &TableSeedEndpoint) -> String {
    format!(
        "{}.{}",
        crate::quote_double(&e.schema),
        crate::quote_double(&e.table)
    )
}
fn session(spec: &ResolvedPostgresConnectSpec) -> String {
    let timeout = spec
        .driver_options
        .statement_timeout_ms
        .filter(|ms| *ms > 0)
        .unwrap_or(60_000)
        .min(60_000);
    let cap = |name: &str, ms: u32| {
        format!("SELECT pg_catalog.set_config('{name}',CASE WHEN pg_catalog.current_setting('{name}')::interval=interval '0' THEN '{ms}' ELSE LEAST(EXTRACT(EPOCH FROM pg_catalog.current_setting('{name}')::interval)*1000,{ms})::bigint::text END,true)")
    };
    format!("SET LOCAL search_path TO pg_catalog; SET LOCAL DateStyle TO ISO; SET LOCAL TIME ZONE 'UTC'; SET LOCAL IntervalStyle TO iso_8601; SET LOCAL extra_float_digits TO 3; SET LOCAL bytea_output TO hex; {}; {}",cap("statement_timeout",timeout),cap("lock_timeout",10_000))
}
pub(crate) async fn inspect(
    spec: &ResolvedPostgresConnectSpec,
    intent: TableSeedIntent,
    target: TableSeedConnection,
    drivers: &DriverJoins,
    control: &Control,
) -> Result<Plan, Failure> {
    let operation = async {
        let c = dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers))
            .await
            .map_err(|_| TableSeedError::Database)?;
        c.client
            .batch_execute(&format!("BEGIN READ ONLY; {}", session(spec)))
            .await
            .map_err(database_error)?;
        let catalog = catalog::inspect(&c, &intent.endpoint).await?;
        c.close().await;
        let identity = Identity::Postgres {
            database_oid: catalog.database,
            relation_oid: catalog.relation.oid,
        };
        Plan::build(EngineCatalog::Postgres(catalog), intent, target, identity)
    };
    let result = tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled.into()),r=tokio::time::timeout(engine::INSPECTION_DEADLINE,operation)=>r.map_err(|_|Failure::from(TableSeedError::Timeout)).and_then(|r|r)};
    drivers.abort_all();
    drivers.drain().await;
    result
}
pub(crate) async fn execute(
    spec: &ResolvedPostgresConnectSpec,
    plan: Arc<Plan>,
    drivers: &DriverJoins,
    control: &Control,
) -> Execution {
    let deadline = engine::deadline();
    let mut connection = None;
    let mut began = false;
    let operation = async {
        connection = Some(
            dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers))
                .await
                .map_err(|_| TableSeedError::Database)?,
        );
        let c = connection.as_ref().unwrap();
        c.client
            .batch_execute("BEGIN")
            .await
            .map_err(database_error)?;
        began = true;
        c.client
            .batch_execute(&session(spec))
            .await
            .map_err(database_error)?;
        // Serialize local MAX-based generation against concurrent target writes.
        c.client
            .batch_execute(&format!(
                "LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE",
                qualified(&plan.intent.endpoint)
            ))
            .await
            .map_err(database_error)?;
        let reviewed = catalog_of(&plan)?;
        if &catalog::inspect(c, &plan.intent.endpoint).await? != reviewed {
            return Err(TableSeedError::TargetChanged.into());
        }
        let seed = planning::finalize(c, &plan, reviewed).await?;
        let names = crate::seed::insert_columns(&seed);
        let types = names
            .iter()
            .map(|n| {
                reviewed
                    .relation
                    .columns
                    .iter()
                    .find(|c| &c.public.name == n)
                    .unwrap()
                    .public
                    .data_type
                    .clone()
            })
            .collect::<Vec<_>>();
        let prefix = format!(
            "INSERT INTO {} ({}) VALUES ",
            qualified(&plan.intent.endpoint),
            names
                .iter()
                .map(|n| crate::quote_double(n))
                .collect::<Vec<_>>()
                .join(",")
        );
        // Each value is `$n::text::type`.
        let placeholders: Vec<usize> = types.iter().map(|t| t.len() + 16).collect();
        let batch = engine::batch_size(&seed, &placeholders, prefix.len(), 60_000)?;
        let mut rng = crate::seed::SeedRng::new(plan.seed_used);
        let mut offset = 0;
        let mut inserted = 0;
        while offset < plan.intent.row_count {
            if control.is_cancelled() {
                return Err(TableSeedError::Cancelled.into());
            }
            let count = batch.min(plan.intent.row_count - offset);
            let rows = crate::seed::generate_rows_from(&seed, offset, count, &mut rng);
            let sql = insert_sql(&prefix, &types, count);
            if sql.len() > 1024 * 1024 {
                return Err(TableSeedError::Limit.into());
            }
            let params = rows
                .iter()
                .flatten()
                .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect::<Vec<_>>();
            inserted += c
                .client
                .execute(&sql, &params)
                .await
                .map_err(database_error)?;
            offset += count;
            control.progress(u64::from(offset));
        }
        if &catalog::inspect(c, &plan.intent.endpoint).await? != reviewed {
            return Err(TableSeedError::TargetChanged.into());
        }
        Ok::<u64, Failure>(inserted)
    };
    let ready = tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled.into()),_=tokio::time::sleep_until(deadline)=>Err(TableSeedError::Timeout.into()),r=operation=>r};
    let execution = match ready {
        Ok(rows) => {
            engine::commit_result(
                async {
                    connection
                        .as_ref()
                        .unwrap()
                        .client
                        .batch_execute("COMMIT")
                        .await
                        .map_err(database_error)
                },
                control,
                rows,
                deadline,
            )
            .await
        }
        Err(error) => Execution::failed(began, error),
    };
    // Driver teardown aborts any uncommitted transaction. Sequence advancement
    // and external trigger side effects are not transactional rollback promises.
    if let Some(c) = connection {
        let cleanup = Instant::now() + Duration::from_secs(1);
        if !matches!(execution.outcome, TableSeedOutcome::Completed { .. }) {
            let _ = tokio::time::timeout_at(
                cleanup,
                dedicated::cancel(c.cancel.clone(), c.tls.clone()),
            )
            .await;
        }
        let _ = tokio::time::timeout_at(cleanup, c.close()).await;
    }
    drivers.abort_all();
    drivers.drain().await;
    execution
}
fn insert_sql(prefix: &str, types: &[String], rows: u32) -> String {
    let mut sql = String::from(prefix);
    let mut parameter = 0;
    for row in 0..rows {
        if row > 0 {
            sql.push(',');
        }
        sql.push('(');
        for (index, kind) in types.iter().enumerate() {
            if index > 0 {
                sql.push(',');
            }
            parameter += 1;
            use std::fmt::Write;
            let _ = write!(sql, "${parameter}::text::{kind}");
        }
        sql.push(')');
    }
    sql
}
