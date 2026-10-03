use super::*;
use crate::{
    backend::{admit_connection, Inner},
    postgres::{connect_spec::ResolvedPostgresConnectSpec, native_table_copy as runner},
    safety::{
        gate,
        policy::{assert_permitted, AuditDisposition, WriteIntent},
    },
    StoredConnection,
};
use tokio::time::Duration;
fn target(stored: &StoredConnection) -> Result<TableCopyConnection, TableCopyError> {
    let StoredConnection::PostgreSQL(pg) = stored else {
        return Err(TableCopyError::UnsupportedTarget);
    };
    let target = TableCopyConnection {
        connection_name: pg.name.clone(),
        host: pg.host.clone(),
        port: pg.effective_port(),
        database: pg.database.clone(),
        user: pg.user.clone(),
        environment: format!("{:?}", pg.environment),
        safe_mode: format!("{:?}", pg.safe_mode),
        read_only: pg.read_only,
    };
    target.checked_heap_bytes().ok_or(TableCopyError::Limit)?;
    Ok(target)
}
fn policy(
    stored: &StoredConnection,
    confirmed: bool,
) -> Result<(bool, AuditDisposition), TableCopyError> {
    match assert_permitted(
        &gate::resolved_policy(stored),
        &WriteIntent::CopyDestination,
        confirmed,
    ) {
        Ok(p) => Ok((false, p.audit_disposition())),
        Err(e) => e.fold(
            |_, _| Err(TableCopyError::PolicyBlocked),
            |_| Ok((true, AuditDisposition::NotRequired)),
        ),
    }
}
struct Authorized {
    specs: [ResolvedPostgresConnectSpec; 2],
    targets: [TableCopyConnection; 2],
    requires_confirmation: bool,
    audit: AuditDisposition,
}
/// Caller holds development_gate. Both capabilities and destination policy are
/// checked against stored metadata before either password is hydrated.
async fn authorize(
    inner: &Inner,
    intent: &TableCopyIntent,
    confirmed: bool,
) -> Result<Authorized, TableCopyError> {
    for endpoint in [&intent.source, &intent.destination] {
        admit_connection(
            &inner.state,
            inner.development.as_deref(),
            &endpoint.connection_id,
        )
        .await
        .map_err(|_| TableCopyError::StaleReview)?;
        if super::super::table_seed::destination_write_in_progress(inner, &endpoint.connection_id)
            || inner.tool_jobs.restore_in_progress(&endpoint.connection_id)
            || inner
                .csv_transfers
                .import_in_progress(&endpoint.connection_id)
        {
            return Err(TableCopyError::Busy);
        }
    }
    let source =
        crate::storage::read_connection_by_id(&inner.state.pool, &intent.source.connection_id)
            .await
            .map_err(|_| TableCopyError::StaleReview)?
            .ok_or(TableCopyError::StaleReview)?;
    let destination =
        crate::storage::read_connection_by_id(&inner.state.pool, &intent.destination.connection_id)
            .await
            .map_err(|_| TableCopyError::StaleReview)?
            .ok_or(TableCopyError::StaleReview)?;
    let (requires_confirmation, audit) = policy(&destination, confirmed)?;
    let targets = [target(&source)?, target(&destination)?];
    let source = crate::app::find_connection(&inner.state, &intent.source.connection_id)
        .await
        .map_err(|_| TableCopyError::Credentials)?;
    let destination = crate::app::find_connection(&inner.state, &intent.destination.connection_id)
        .await
        .map_err(|_| TableCopyError::Credentials)?;
    let specs = [
        ResolvedPostgresConnectSpec::from_connection(&source)
            .map_err(|_| TableCopyError::UnsupportedTarget)?,
        ResolvedPostgresConnectSpec::from_connection(&destination)
            .map_err(|_| TableCopyError::UnsupportedTarget)?,
    ];
    Ok(Authorized {
        specs,
        targets,
        requires_confirmation,
        audit,
    })
}
pub(super) async fn prepare(inner: Arc<Inner>, id: TableCopyAttemptId) {
    let (intent, control, drivers) = {
        let state = inner.table_copy.state.lock().unwrap();
        let e = &state.jobs[&id];
        (
            e.observation.intent.clone(),
            e.control.clone(),
            e.drivers.clone(),
        )
    };
    let setup = async {
        let _gate = inner.development_gate.lock().await;
        authorize(&inner, &intent, false).await
    };
    let authorized = tokio::select! {biased;_=control.cancelled()=>Err(TableCopyError::Cancelled),r=tokio::time::timeout(Duration::from_secs(30),setup)=>r.map_err(|_|TableCopyError::Timeout).and_then(|r|r)};
    let result = match authorized {
        Ok(authorized) => inspect(
            &inner,
            &authorized.specs,
            intent,
            authorized.targets,
            &drivers,
            &control,
        )
        .await
        .map(|plan| (plan, authorized.requires_confirmation)),
        Err(error) => Err(error.into()),
    };
    // A connect that failed during session setup still has an owned driver.
    drivers.abort_all();
    drivers.drain().await;
    let mut state = inner.table_copy.state.lock().unwrap();
    let Some(e) = state.jobs.get_mut(&id) else {
        return;
    };
    let result = if control.is_cancelled() {
        Err(TableCopyError::Cancelled.into())
    } else {
        result
    };
    match result {
        Ok((plan, requires_confirmation)) => {
            e.ready = Some(Arc::new(Ready {
                plan: Arc::new(plan),
                _permit: e.permit.take().unwrap(),
                requires_confirmation,
            }));
            e.observation.phase = TableCopyPhase::ReadyReview;
            e.observation.cleanup = TableCopyCleanup::Complete;
            e.created = std::time::Instant::now();
        }
        Err(failure) => {
            e.observation.phase = if failure.error == TableCopyError::Cancelled {
                TableCopyPhase::Cancelled
            } else {
                TableCopyPhase::Failed
            };
            e.observation.failure = Some(failure.error);
            e.observation.diagnostic = failure.diagnostic;
            e.observation.cleanup = TableCopyCleanup::Complete;
            e.finished = Some(std::time::Instant::now());
            e.permit.take();
        }
    }
}
pub(super) async fn execute(
    inner: Arc<Inner>,
    id: TableCopyAttemptId,
    review: TableCopyReview,
    confirmed: bool,
) {
    let (control, drivers) = {
        let state = inner.table_copy.state.lock().unwrap();
        let e = &state.jobs[&id];
        (e.control.clone(), e.drivers.clone())
    };
    let preparation = async {
        let gate = inner.development_gate.lock().await;
        let authorized = authorize(&inner, &review.description().intent, confirmed).await?;
        if authorized.requires_confirmation
            || authorized.targets[0] != review.description().source_connection
            || authorized.targets[1] != review.description().destination_connection
        {
            return Err(TableCopyError::StaleReview);
        }
        Ok((gate, authorized))
    };
    let prepared = tokio::select! {biased;_=control.cancelled()=>Err(TableCopyError::Cancelled),r=tokio::time::timeout(Duration::from_secs(30),preparation)=>r.map_err(|_|TableCopyError::Timeout).and_then(|r|r)};
    let result = match prepared {
        Ok((gate, authorized)) => {
            // Once retirement starts its internal absolute deadline/joins must
            // settle. Dropping it on Cancel could strand a manager teardown fence.
            let retired = crate::backend::data::retire_data(
                &inner,
                &inner.state,
                Some(&review.description().intent.destination.connection_id),
            )
            .await
            .map_err(|_| TableCopyError::Cleanup);
            drop(gate);
            retired.and_then(|()| {
                if control.is_cancelled() {
                    Err(TableCopyError::Cancelled)
                } else {
                    Ok(authorized)
                }
            })
        }
        Err(error) => Err(error),
    };
    let (execution, audit) = match result {
        Ok(authorized) => (
            run(
                &inner,
                &authorized.specs,
                review.ready.plan.clone(),
                &drivers,
                control.clone(),
            )
            .await,
            authorized.audit,
        ),
        Err(error) => (
            runner::Execution {
                outcome: TableCopyOutcome::NotStarted,
                failure: Some(error.into()),
            },
            AuditDisposition::NotRequired,
        ),
    };
    drivers.abort_all();
    drivers.drain().await;
    if matches!(execution.outcome, TableCopyOutcome::Completed { .. })
        && audit == AuditDisposition::RequiredAfterSuccess
    {
        gate::record_override(
            &inner.state.pool,
            &review.description().intent.destination.connection_id,
            "native_table_copy",
            &WriteIntent::CopyDestination,
        )
        .await;
    }
    finish(&inner, id, &review, execution);
}
fn finish(
    inner: &Inner,
    id: TableCopyAttemptId,
    review: &TableCopyReview,
    execution: runner::Execution,
) {
    let mut state = inner.table_copy.state.lock().unwrap();
    let changed = matches!(
        execution.outcome,
        TableCopyOutcome::Completed { .. } | TableCopyOutcome::OutcomeUnknown
    );
    let revision = if changed {
        state.revision = state
            .revision
            .checked_add(1)
            .expect("bounded lifetime copy revision");
        Some(state.revision)
    } else {
        None
    };
    let Some(e) = state.jobs.get_mut(&id) else {
        return;
    };
    let failure = execution.failure.as_ref().map(|f| f.error);
    let diagnostic = execution.failure.and_then(|f| f.diagnostic);
    e.observation.phase = if matches!(execution.outcome, TableCopyOutcome::Completed { .. }) {
        TableCopyPhase::Completed
    } else if failure == Some(TableCopyError::Cancelled)
        && execution.outcome != TableCopyOutcome::OutcomeUnknown
    {
        TableCopyPhase::Cancelled
    } else {
        TableCopyPhase::Failed
    };
    e.observation.outcome = execution.outcome;
    e.observation.failure = failure;
    e.observation.diagnostic = diagnostic.clone();
    e.observation.change_revision = revision;
    e.observation.receipt = Some(TableCopyReceipt {
        attempt_id: id,
        description: review.description().clone(),
        outcome: execution.outcome,
        failure,
        diagnostic,
    });
    e.observation.cleanup = if failure == Some(TableCopyError::Cleanup) {
        TableCopyCleanup::Failed
    } else {
        TableCopyCleanup::Complete
    };
    if e.observation.cleanup == TableCopyCleanup::Complete {
        e.execution.take();
        e.ready.take();
    }
    e.finished = Some(std::time::Instant::now());
}

async fn inspect(
    inner: &Inner,
    specs: &[ResolvedPostgresConnectSpec; 2],
    intent: TableCopyIntent,
    targets: [TableCopyConnection; 2],
    drivers: &crate::postgres::dedicated::DriverJoins,
    control: &Control,
) -> Result<Plan, runner::Failure> {
    #[cfg(test)]
    {
        let injected = inner.table_copy.inspector.lock().unwrap().clone();
        if let Some(inspect) = injected {
            return tokio::select! {biased;_=control.cancelled()=>Err(TableCopyError::Cancelled.into()),r=inspect(intent,targets)=>r};
        }
    }
    let _ = inner;
    runner::inspect(specs, intent, targets, drivers, control).await
}
async fn run(
    inner: &Inner,
    specs: &[ResolvedPostgresConnectSpec; 2],
    plan: Arc<Plan>,
    drivers: &crate::postgres::dedicated::DriverJoins,
    control: Arc<Control>,
) -> runner::Execution {
    #[cfg(test)]
    {
        let injected = inner.table_copy.runner.lock().unwrap().clone();
        if let Some(run) = injected {
            return run(control).await;
        }
    }
    let _ = inner;
    runner::execute(specs, plan, drivers, &control).await
}
