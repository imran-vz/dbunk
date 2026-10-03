//! Native table copy uses a frozen source snapshot and one destination transaction.
//! Only the CSV catalog and guarded text projection are shared with file transfers.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
    transfer::{
        csv::CsvOptions,
        runner::{
            catalog::{self, RelationState},
            sql,
        },
    },
};
use crate::backend::table_copy::*;
use futures_util::{SinkExt, TryStreamExt};
use std::sync::Arc;
use tokio::time::{Duration, Instant};

pub(crate) struct Plan {
    pub description: TableCopyDescription,
    pub columns: Vec<TableCopyColumn>,
    source: RelationState,
    destination: RelationState,
    names: Vec<String>,
}
impl Plan {
    pub(crate) fn retained_bytes(&self) -> usize {
        // Catalog admission is <=256 KiB text per endpoint/1600 columns. Include
        // row structs, duplicate public types/names and both SQL preparations.
        MAX_TABLE_COPY_REVIEW_BYTES
    }
}
#[derive(Debug)]
pub(crate) struct Failure {
    pub error: TableCopyError,
    pub diagnostic: Option<TableCopyDiagnostic>,
}
impl From<TableCopyError> for Failure {
    fn from(error: TableCopyError) -> Self {
        Self {
            error,
            diagnostic: None,
        }
    }
}
fn database(error: tokio_postgres::Error, side: TableCopySide) -> Failure {
    Failure {
        error: TableCopyError::Database,
        diagnostic: Some(TableCopyDiagnostic {
            side,
            sqlstate: error.code().map(|c| c.code().to_owned()),
            field_limit: error
                .as_db_error()
                .is_some_and(|e| e.message().contains("dbunk_csv_export_field_limit_")),
            record_limit: error
                .as_db_error()
                .is_some_and(|e| e.message().contains("dbunk_csv_export_record_limit_")),
        }),
    }
}
fn catalog_error(error: super::transfer::protocol::TransferError, side: TableCopySide) -> Failure {
    use super::transfer::protocol::TransferError as E;
    match error {
        E::TargetChanged => TableCopyError::TargetChanged.into(),
        E::UnsupportedTarget { .. } => TableCopyError::UnsupportedTarget.into(),
        E::InvalidRequest { .. } => TableCopyError::Limit.into(),
        E::Database { code, .. } => Failure {
            error: TableCopyError::Database,
            diagnostic: Some(TableCopyDiagnostic {
                side,
                sqlstate: code,
                field_limit: false,
                record_limit: false,
            }),
        },
        _ => TableCopyError::Database.into(),
    }
}
const SESSION:&str="SET LOCAL DateStyle TO ISO; SET LOCAL TIME ZONE 'UTC'; SET LOCAL IntervalStyle TO iso_8601; SET LOCAL extra_float_digits TO 3; SET LOCAL bytea_output TO hex; SET LOCAL lc_monetary TO 'C'";
fn session(spec: &ResolvedPostgresConnectSpec, ceiling: u32) -> String {
    let timeout = spec
        .driver_options
        .statement_timeout_ms
        .filter(|ms| *ms > 0)
        .unwrap_or(ceiling)
        .min(ceiling);
    // Lower finite inherited role/database timeouts as necessary; never widen
    // them. PostgreSQL's displayed timeout values are accepted interval input.
    let cap = |name: &str, ms: u32| {
        format!("SELECT pg_catalog.set_config('{name}', CASE WHEN pg_catalog.current_setting('{name}')::interval=interval '0' THEN '{ms}' ELSE LEAST(EXTRACT(EPOCH FROM pg_catalog.current_setting('{name}')::interval)*1000,{ms})::bigint::text END, true)")
    };
    format!(
        "{SESSION}; {}; {}",
        cap("statement_timeout", timeout),
        cap("lock_timeout", 10_000)
    )
}
fn qualified(endpoint: &TableCopyEndpoint) -> String {
    format!(
        "{}.{}",
        crate::quote_double(&endpoint.schema),
        crate::quote_double(&endpoint.table)
    )
}
async fn metadata(
    c: &DedicatedConnection,
    e: &TableCopyEndpoint,
    side: TableCopySide,
) -> Result<(RelationState, u32), Failure> {
    let relation = catalog::inspect(c, &e.schema, &e.table, true)
        .await
        .map_err(|e| catalog_error(e, side))?;
    if !matches!(relation.kind.as_str(), "r" | "p")
        || matches!(side, TableCopySide::Destination)
            && (relation.row_security || relation.force_row_security)
    {
        return Err(TableCopyError::UnsupportedTarget.into());
    }
    let oid = c
        .client
        .query_one(
            "SELECT oid FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database()",
            &[],
        )
        .await
        .map_err(|e| database(e, side))?
        .get(0);
    Ok((relation, oid))
}
pub(crate) async fn inspect(
    specs: &[ResolvedPostgresConnectSpec; 2],
    intent: TableCopyIntent,
    targets: [TableCopyConnection; 2],
    drivers: &DriverJoins,
    control: &Control,
) -> Result<Plan, Failure> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let operation = async {
        let source = dedicated::connect_tracked(&specs[0], NoticeSink::Ignore, Some(drivers))
            .await
            .map_err(|_| TableCopyError::Database)?;
        source
            .client
            .batch_execute(&format!("BEGIN READ ONLY; {}", session(&specs[0], 20_000)))
            .await
            .map_err(|e| database(e, TableCopySide::Source))?;
        let (source_relation, source_db) =
            metadata(&source, &intent.source, TableCopySide::Source).await?;
        source.close().await;
        let destination = dedicated::connect_tracked(&specs[1], NoticeSink::Ignore, Some(drivers))
            .await
            .map_err(|_| TableCopyError::Database)?;
        destination
            .client
            .batch_execute(&format!("BEGIN READ ONLY; {}", session(&specs[1], 20_000)))
            .await
            .map_err(|e| database(e, TableCopySide::Destination))?;
        let (destination_relation, destination_db) = metadata(
            &destination,
            &intent.destination,
            TableCopySide::Destination,
        )
        .await?;
        let columns = mapping(&source_relation, &destination_relation)?;
        // Server SHA-256 avoids a new dependency. Metadata bytes are already
        // admitted; this parameter contains names/types/actions, never row data.
        let encoded = serde_json::to_string(&columns).map_err(|_| TableCopyError::Limit)?;
        if encoded.len() > MAX_TABLE_COPY_REVIEW_BYTES / 2 {
            return Err(TableCopyError::Limit.into());
        }
        let digest:String=destination.client.query_one("SELECT pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to($1::text,'UTF8')),'hex')",&[&encoded]).await.map_err(|e|database(e,TableCopySide::Destination))?.get(0);
        destination.close().await;
        let names = columns
            .iter()
            .filter(|c| {
                matches!(
                    c.action,
                    TableCopyColumnAction::Copy | TableCopyColumnAction::CopyIdentity
                )
            })
            .map(|c| c.name.clone())
            .collect();
        let description = TableCopyDescription {
            intent,
            source_connection: targets[0].clone(),
            destination_connection: targets[1].clone(),
            source_relation: TableCopyRelation {
                database_oid: source_db,
                relation_oid: source_relation.oid,
                kind: source_relation.kind.clone(),
            },
            destination_relation: TableCopyRelation {
                database_oid: destination_db,
                relation_oid: destination_relation.oid,
                kind: destination_relation.kind.clone(),
            },
            mapping_sha256: digest,
            copied_columns: columns
                .iter()
                .filter(|c| {
                    matches!(
                        c.action,
                        TableCopyColumnAction::Copy | TableCopyColumnAction::CopyIdentity
                    )
                })
                .count() as u16,
            defaulted_columns: columns
                .iter()
                .filter(|c| c.action == TableCopyColumnAction::DefaultOrNull)
                .count() as u16,
            generated_columns: columns
                .iter()
                .filter(|c| c.action == TableCopyColumnAction::Generated)
                .count() as u16,
            identity_columns: columns
                .iter()
                .filter(|c| c.action == TableCopyColumnAction::CopyIdentity)
                .count() as u16,
        };
        description.validate()?;
        Ok(Plan {
            description,
            columns,
            source: source_relation,
            destination: destination_relation,
            names,
        })
    };
    let result = tokio::select! {biased;_=control.cancelled()=>Err(TableCopyError::Cancelled.into()),_=tokio::time::sleep_until(deadline)=>Err(TableCopyError::Timeout.into()),result=operation=>result};
    drivers.abort_all();
    drivers.drain().await;
    result
}
fn mapping(
    source: &RelationState,
    destination: &RelationState,
) -> Result<Vec<TableCopyColumn>, Failure> {
    let mut columns = Vec::with_capacity(destination.columns.len());
    for target in &destination.columns {
        let s = source
            .columns
            .iter()
            .find(|c| c.public.name == target.public.name);
        let action = if target.public.generated {
            TableCopyColumnAction::Generated
        } else if s.is_some() {
            if target.public.identity {
                TableCopyColumnAction::CopyIdentity
            } else {
                TableCopyColumnAction::Copy
            }
        } else if target.public.has_default || target.public.nullable || target.public.identity {
            TableCopyColumnAction::DefaultOrNull
        } else {
            return Err(TableCopyError::MissingRequiredColumn.into());
        };
        columns.push(TableCopyColumn {
            name: target.public.name.clone(),
            source_type: s.map(|s| s.public.data_type.clone()),
            destination_type: target.public.data_type.clone(),
            action,
        });
    }
    if !columns.iter().any(|c| {
        matches!(
            c.action,
            TableCopyColumnAction::Copy | TableCopyColumnAction::CopyIdentity
        )
    }) {
        return Err(TableCopyError::UnsupportedTarget.into());
    }
    Ok(columns)
}
pub(crate) struct Execution {
    pub outcome: TableCopyOutcome,
    pub failure: Option<Failure>,
}
pub(crate) async fn execute(
    specs: &[ResolvedPostgresConnectSpec; 2],
    plan: Arc<Plan>,
    drivers: &DriverJoins,
    control: &Control,
) -> Execution {
    let deadline = Instant::now() + Duration::from_secs(3600);
    let mut source = None;
    let mut destination = None;
    let mut began = false;
    let prepare = async {
        source = Some(
            dedicated::connect_tracked(&specs[0], NoticeSink::Ignore, Some(drivers))
                .await
                .map_err(|_| TableCopyError::Database)?,
        );
        destination = Some(
            dedicated::connect_tracked(&specs[1], NoticeSink::Ignore, Some(drivers))
                .await
                .map_err(|_| TableCopyError::Database)?,
        );
        let s = source.as_ref().unwrap();
        let d = destination.as_ref().unwrap();
        s.client
            .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(|e| database(e, TableCopySide::Source))?;
        s.client
            .batch_execute(&session(&specs[0], 60_000))
            .await
            .map_err(|e| database(e, TableCopySide::Source))?;
        s.client
            .batch_execute(&format!(
                "SELECT FROM {} LIMIT 0",
                qualified(&plan.description.intent.source)
            ))
            .await
            .map_err(|e| database(e, TableCopySide::Source))?;
        let (current, oid) =
            metadata(s, &plan.description.intent.source, TableCopySide::Source).await?;
        if current != plan.source || oid != plan.description.source_relation.database_oid {
            return Err(TableCopyError::TargetChanged.into());
        }
        // The source RR snapshot is now established before any destination write,
        // including self-copy. Source locks last until destination COMMIT settles.
        d.client
            .batch_execute("BEGIN")
            .await
            .map_err(|e| database(e, TableCopySide::Destination))?;
        began = true;
        d.client
            .batch_execute(&session(&specs[1], 60_000))
            .await
            .map_err(|e| database(e, TableCopySide::Destination))?;
        d.client
            .batch_execute(&format!(
                "LOCK TABLE {} IN ROW EXCLUSIVE MODE",
                qualified(&plan.description.intent.destination)
            ))
            .await
            .map_err(|e| database(e, TableCopySide::Destination))?;
        let (current, oid) = metadata(
            d,
            &plan.description.intent.destination,
            TableCopySide::Destination,
        )
        .await?;
        if current != plan.destination || oid != plan.description.destination_relation.database_oid
        {
            return Err(TableCopyError::TargetChanged.into());
        }
        let names = plan.names.iter().map(String::as_str).collect::<Vec<_>>();
        let options = CsvOptions {
            header: false,
            ..Default::default()
        };
        let output_sql = sql::export_columns(
            &plan.description.intent.source.schema,
            &plan.description.intent.source.table,
            &names,
            &options,
        );
        let input_sql = sql::import_columns(
            &plan.description.intent.destination.schema,
            &plan.description.intent.destination.table,
            &names,
            &options,
        );
        let output = s
            .client
            .copy_out(&output_sql)
            .await
            .map_err(|e| database(e, TableCopySide::Source))?;
        let input = d
            .client
            .copy_in(&input_sql)
            .await
            .map_err(|e| database(e, TableCopySide::Destination))?;
        futures_util::pin_mut!(output, input);
        while let Some(chunk) = output
            .try_next()
            .await
            .map_err(|e| database(e, TableCopySide::Source))?
        {
            if chunk.len() > super::transfer::csv::MAX_RECORD_BYTES {
                return Err(TableCopyError::Limit.into());
            }
            control.add_bytes(chunk.len())?;
            // No decoded row arrays. The server's pre-wire field/record guard and
            // backpressure bound the one retained CopyData message per endpoint.
            input
                .as_mut()
                .send(chunk)
                .await
                .map_err(|e| database(e, TableCopySide::Destination))?;
        }
        let rows = input
            .as_mut()
            .finish()
            .await
            .map_err(|e| database(e, TableCopySide::Destination))?;
        Ok::<_, Failure>(rows)
    };
    let ready = tokio::select! {biased;_=control.cancelled()=>Err(TableCopyError::Cancelled.into()),_=tokio::time::sleep_until(deadline)=>Err(TableCopyError::Timeout.into()),r=prepare=>r};
    let execution = match ready {
        Ok(rows) => {
            commit(
                destination.as_ref().unwrap().client.batch_execute("COMMIT"),
                control,
                rows,
                deadline,
            )
            .await
        }
        result => Execution {
            outcome: if began {
                TableCopyOutcome::RolledBack
            } else {
                TableCopyOutcome::NotStarted
            },
            failure: Some(
                result
                    .err()
                    .unwrap_or_else(|| TableCopyError::Cancelled.into()),
            ),
        },
    };
    // Never issue a second COMMIT. An uncommitted destination transaction is
    // rolled back by teardown; sequence/trigger external effects are not undone.
    let cleanup = deadline.min(Instant::now() + Duration::from_secs(1));
    for connection in [source, destination].into_iter().flatten() {
        if !matches!(execution.outcome, TableCopyOutcome::Completed { .. }) {
            let _ = tokio::time::timeout_at(
                cleanup,
                dedicated::cancel(connection.cancel.clone(), connection.tls.clone()),
            )
            .await;
        }
        let _ = tokio::time::timeout_at(cleanup, connection.close()).await;
    }
    drivers.abort_all();
    drivers.drain().await;
    execution
}

#[cfg(test)]
pub(crate) mod tests;

async fn commit(
    future: impl std::future::Future<Output = Result<(), tokio_postgres::Error>>,
    control: &Control,
    rows: u64,
    deadline: Instant,
) -> Execution {
    commit_result(
        async move {
            future
                .await
                .map_err(|e| database(e, TableCopySide::Destination))
        },
        control,
        rows,
        deadline,
    )
    .await
}
async fn commit_result(
    future: impl std::future::Future<Output = Result<(), Failure>>,
    control: &Control,
    rows: u64,
    deadline: Instant,
) -> Execution {
    if Instant::now() >= deadline {
        return Execution {
            outcome: TableCopyOutcome::RolledBack,
            failure: Some(TableCopyError::Timeout.into()),
        };
    }
    if !control.admit_commit() {
        return Execution {
            outcome: TableCopyOutcome::RolledBack,
            failure: Some(TableCopyError::Cancelled.into()),
        };
    }
    tokio::pin!(future);
    let commit_deadline = deadline.min(Instant::now() + Duration::from_secs(10));
    let result = tokio::select! {biased;r=&mut future=>r,_=tokio::time::sleep_until(commit_deadline)=>Err(TableCopyError::Timeout.into()),_=control.cancelled()=>match tokio::time::timeout_at(commit_deadline.min(Instant::now()+Duration::from_secs(1)),&mut future).await{Ok(r)=>r,Err(_)=>Err(TableCopyError::Cancelled.into())}};
    match result {
        Ok(()) => Execution {
            outcome: TableCopyOutcome::Completed { rows },
            failure: None,
        },
        Err(e) => Execution {
            outcome: TableCopyOutcome::OutcomeUnknown,
            failure: Some(e),
        },
    }
}
