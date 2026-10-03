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
pub(crate) struct Plan {
    pub description: Option<TableSeedDescription>,
    pub intent: TableSeedIntent,
    pub columns: Vec<TableSeedColumn>,
    pub issue: Option<TableSeedError>,
    pub seed_used: u64,
    clock: i64,
    catalog: catalog::Catalog,
}
impl Plan {
    pub(crate) fn checked_heap_bytes(&self) -> Option<usize> {
        retention::checked(self)
    }
}
#[derive(Debug)]
pub(crate) struct Failure {
    pub error: TableSeedError,
    pub diagnostic: Option<Box<TableSeedDiagnostic>>,
}
impl From<TableSeedError> for Failure {
    fn from(error: TableSeedError) -> Self {
        Self {
            error,
            diagnostic: None,
        }
    }
}
pub(crate) struct Execution {
    pub outcome: TableSeedOutcome,
    pub failure: Option<Failure>,
}
fn database_error(error: tokio_postgres::Error) -> Failure {
    let bounded = |s: Option<&str>| {
        s.filter(|s| s.len() <= 63 && !s.contains('\0'))
            .map(str::to_owned)
    };
    Failure {
        error: TableSeedError::Database,
        diagnostic: error.as_db_error().map(|e| {
            Box::new(TableSeedDiagnostic {
                sqlstate: Some(e.code().code().to_owned()),
                constraint: bounded(e.constraint()),
                column: bounded(e.column()),
                parent_schema: None,
                parent_table: None,
            })
        }),
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
        let draft = planning::draft(&catalog, &intent);
        let columns = planning::columns(&catalog, draft.as_ref().ok());
        let inserted = columns
            .iter()
            .filter(|c| c.action != TableSeedColumnAction::Default)
            .count();
        let issue = draft
            .err()
            .or_else(|| (inserted == 0).then_some(TableSeedError::InvalidRecipe));
        let seed_used = intent.seed.unwrap_or_else(|| {
            let bytes = uuid::Uuid::new_v4().into_bytes();
            u64::from_le_bytes(bytes[..8].try_into().unwrap())
        });
        let clock = chrono::Utc::now().timestamp();
        let description = if issue.is_none() {
            let encoded =
                serde_json::to_string(&intent.columns).map_err(|_| TableSeedError::Limit)?;
            let recipe_sha256:String=c.client.query_one("SELECT pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to($1::text,'UTF8')),'hex')",&[&encoded]).await.map_err(database_error)?.get(0);
            let (recipe_summary, recipe_summary_truncated) = planning::summary(&intent);
            let d = TableSeedDescription {
                endpoint: intent.endpoint.clone(),
                connection: target,
                database_oid: catalog.database,
                relation_oid: catalog.relation.oid,
                row_count: intent.row_count,
                seed_used,
                clock_epoch_seconds: clock,
                recipe_sha256,
                recipe_summary,
                recipe_summary_truncated,
                inserted_columns: inserted as u16,
                defaulted_columns: (columns.len() - inserted) as u16,
            };
            d.validate()?;
            Some(d)
        } else {
            None
        };
        c.close().await;
        Ok(Plan {
            description,
            intent,
            columns,
            issue,
            seed_used,
            clock,
            catalog,
        })
    };
    let result = tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled.into()),r=tokio::time::timeout(Duration::from_secs(30),operation)=>r.map_err(|_|Failure::from(TableSeedError::Timeout)).and_then(|r|r)};
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
    let deadline = Instant::now() + Duration::from_secs(600);
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
        if catalog::inspect(c, &plan.intent.endpoint).await? != plan.catalog {
            return Err(TableSeedError::TargetChanged.into());
        }
        let seed = planning::finalize(c, &plan).await?;
        let names = crate::seed::insert_columns(&seed);
        let types = names
            .iter()
            .map(|n| {
                plan.catalog
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
        let batch = planning::batch_size(&seed, &types, prefix.len())?;
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
        if catalog::inspect(c, &plan.intent.endpoint).await? != plan.catalog {
            return Err(TableSeedError::TargetChanged.into());
        }
        Ok::<u64, Failure>(inserted)
    };
    let ready = tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled.into()),_=tokio::time::sleep_until(deadline)=>Err(TableSeedError::Timeout.into()),r=operation=>r};
    let execution = match ready {
        Ok(rows) => {
            commit_result(
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
        Err(error) => Execution {
            outcome: if began {
                TableSeedOutcome::RolledBack
            } else {
                TableSeedOutcome::NotStarted
            },
            failure: Some(error),
        },
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
async fn commit_result(
    future: impl std::future::Future<Output = Result<(), Failure>>,
    control: &Control,
    rows: u64,
    deadline: Instant,
) -> Execution {
    if Instant::now() >= deadline {
        return Execution {
            outcome: TableSeedOutcome::RolledBack,
            failure: Some(TableSeedError::Timeout.into()),
        };
    }
    if !control.admit_commit() {
        return Execution {
            outcome: TableSeedOutcome::RolledBack,
            failure: Some(TableSeedError::Cancelled.into()),
        };
    }
    tokio::pin!(future);
    let deadline = deadline.min(Instant::now() + Duration::from_secs(10));
    let result = tokio::select! {biased;r=&mut future=>r,_=tokio::time::sleep_until(deadline)=>Err(TableSeedError::Timeout.into()),_=control.cancelled()=>match tokio::time::timeout_at(deadline.min(Instant::now()+Duration::from_secs(1)),&mut future).await{Ok(r)=>r,Err(_)=>Err(TableSeedError::Cancelled.into())}};
    match result {
        Ok(()) => Execution {
            outcome: TableSeedOutcome::Completed { rows },
            failure: None,
        },
        Err(e) => Execution {
            outcome: TableSeedOutcome::OutcomeUnknown,
            failure: Some(e),
        },
    }
}
