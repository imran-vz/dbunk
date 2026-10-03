use super::*;
use crate::{
    backend::{admit_connection, Inner},
    postgres::{connect_spec::ResolvedPostgresConnectSpec, native_table_seed as runner},
    safety::{
        gate,
        policy::{assert_permitted, AuditDisposition, WriteIntent},
    },
    StoredConnection,
};
use tokio::time::Duration;
fn target(stored: &StoredConnection) -> Result<TableSeedConnection, TableSeedError> {
    let StoredConnection::PostgreSQL(pg) = stored else {
        return Err(TableSeedError::UnsupportedTarget);
    };
    let target = TableSeedConnection {
        connection_name: pg.name.clone(),
        host: pg.host.clone(),
        port: pg.effective_port(),
        database: pg.database.clone(),
        user: pg.user.clone(),
        environment: format!("{:?}", pg.environment),
        safe_mode: format!("{:?}", pg.safe_mode),
        read_only: pg.read_only,
    };
    target.checked_heap_bytes().ok_or(TableSeedError::Limit)?;
    Ok(target)
}
fn policy(
    stored: &StoredConnection,
    confirmed: bool,
) -> Result<(bool, AuditDisposition), TableSeedError> {
    match assert_permitted(
        &gate::resolved_policy(stored),
        &WriteIntent::Seed,
        confirmed,
    ) {
        Ok(p) => Ok((false, p.audit_disposition())),
        Err(e) => e.fold(
            |_, _| Err(TableSeedError::PolicyBlocked),
            |_| Ok((true, AuditDisposition::NotRequired)),
        ),
    }
}
struct Authorized {
    specs: ResolvedPostgresConnectSpec,
    targets: TableSeedConnection,
    requires_confirmation: bool,
    audit: AuditDisposition,
}
/// Caller holds development_gate. Target policy and interlocks are checked
/// against stored metadata before credentials are hydrated.
async fn authorize(
    inner: &Inner,
    intent: &TableSeedIntent,
    confirmed: bool,
) -> Result<Authorized, TableSeedError> {
    let id = &intent.endpoint.connection_id;
    admit_connection(&inner.state, inner.development.as_deref(), id)
        .await
        .map_err(|_| TableSeedError::StaleReview)?;
    if inner.tool_jobs.restore_in_progress(id)
        || inner.csv_transfers.import_in_progress(id)
        || crate::backend::table_copy::destination_write_in_progress(inner, id)
    {
        return Err(TableSeedError::Busy);
    }
    let stored = crate::storage::read_connection_by_id(&inner.state.pool, id)
        .await
        .map_err(|_| TableSeedError::StaleReview)?
        .ok_or(TableSeedError::StaleReview)?;
    let (requires_confirmation, audit) = policy(&stored, confirmed)?;
    let targets = target(&stored)?;
    let hydrated = crate::app::find_connection(&inner.state, id)
        .await
        .map_err(|_| TableSeedError::Credentials)?;
    let specs = ResolvedPostgresConnectSpec::from_connection(&hydrated)
        .map_err(|_| TableSeedError::UnsupportedTarget)?;
    Ok(Authorized {
        specs,
        targets,
        requires_confirmation,
        audit,
    })
}

pub(super) async fn prepare(inner: Arc<Inner>, id: TableSeedAttemptId) {
    let (intent, control, drivers) = {
        let state = inner.table_seed.state.lock().unwrap();
        let e = &state.jobs[&id];
        (e.intent.clone(), e.control.clone(), e.drivers.clone())
    };
    let setup = async {
        let _gate = inner.development_gate.lock().await;
        authorize(&inner, &intent, false).await
    };
    let authorized = tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled),r=tokio::time::timeout(Duration::from_secs(30),setup)=>r.map_err(|_|TableSeedError::Timeout).and_then(|r|r)};
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
        .and_then(|plan| {
            plan.checked_heap_bytes()
                .ok_or(runner::Failure::from(TableSeedError::Limit))?;
            Ok((plan, authorized.requires_confirmation))
        }),
        Err(error) => Err(error.into()),
    };
    // A connect that failed during session setup still has an owned driver.
    drivers.abort_all();
    drivers.drain().await;
    let mut state = inner.table_seed.state.lock().unwrap();
    let Some(e) = state.jobs.get_mut(&id) else {
        return;
    };
    let result = if control.is_cancelled() {
        Err(TableSeedError::Cancelled.into())
    } else {
        result
    };
    match result {
        Ok((plan, requires_confirmation)) => {
            e.observation.issue = plan.issue;
            e.observation.seed_used = Some(plan.seed_used);
            let runnable = plan.description.is_some();
            e.ready = Some(Arc::new(Ready {
                plan: Arc::new(plan),
                _permit: e.permit.take().unwrap(),
                requires_confirmation,
            }));
            e.observation.phase = if runnable {
                TableSeedPhase::ReadyReview
            } else {
                TableSeedPhase::NeedsRecipe
            };
            e.observation.cleanup = TableSeedCleanup::Complete;
            e.created = std::time::Instant::now();
        }
        Err(failure) => {
            e.observation.phase = if failure.error == TableSeedError::Cancelled {
                TableSeedPhase::Cancelled
            } else {
                TableSeedPhase::Failed
            };
            e.observation.failure = Some(failure.error);
            e.observation.diagnostic = failure.diagnostic.map(|d| *d);
            e.observation.cleanup = TableSeedCleanup::Complete;
            e.finished = Some(std::time::Instant::now());
            e.permit.take();
        }
    }
}
pub(super) async fn execute(
    inner: Arc<Inner>,
    id: TableSeedAttemptId,
    review: TableSeedReview,
    confirmed: bool,
) {
    let (control, drivers) = {
        let state = inner.table_seed.state.lock().unwrap();
        let e = &state.jobs[&id];
        (e.control.clone(), e.drivers.clone())
    };
    let preparation = async {
        let gate = inner.development_gate.lock().await;
        let authorized = authorize(&inner, &review.ready.plan.intent, confirmed).await?;
        if authorized.requires_confirmation || authorized.targets != review.description().connection
        {
            return Err(TableSeedError::StaleReview);
        }
        Ok((gate, authorized))
    };
    let prepared = tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled),r=tokio::time::timeout(Duration::from_secs(30),preparation)=>r.map_err(|_|TableSeedError::Timeout).and_then(|r|r)};
    let result = match prepared {
        Ok((gate, authorized)) => {
            // Once retirement starts its internal absolute deadline/joins must
            // settle. Dropping it on Cancel could strand a manager teardown fence.
            let retired = crate::backend::data::retire_data(
                &inner,
                &inner.state,
                Some(&review.description().endpoint.connection_id),
            )
            .await
            .map_err(|_| TableSeedError::Cleanup);
            drop(gate);
            retired.and_then(|()| {
                if control.is_cancelled() {
                    Err(TableSeedError::Cancelled)
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
                outcome: TableSeedOutcome::NotStarted,
                failure: Some(error.into()),
            },
            AuditDisposition::NotRequired,
        ),
    };
    drivers.abort_all();
    drivers.drain().await;
    if matches!(execution.outcome, TableSeedOutcome::Completed { .. })
        && audit == AuditDisposition::RequiredAfterSuccess
    {
        gate::record_override(
            &inner.state.pool,
            &review.description().endpoint.connection_id,
            "seed_table",
            &WriteIntent::Seed,
        )
        .await;
    }
    finish(&inner, id, &review, execution);
}
fn finish(
    inner: &Inner,
    id: TableSeedAttemptId,
    review: &TableSeedReview,
    execution: runner::Execution,
) {
    let mut state = inner.table_seed.state.lock().unwrap();
    let changed = matches!(
        execution.outcome,
        TableSeedOutcome::Completed { .. } | TableSeedOutcome::OutcomeUnknown
    );
    let revision = if changed {
        state.revision = state
            .revision
            .checked_add(1)
            .expect("bounded lifetime seed revision");
        Some(state.revision)
    } else {
        None
    };
    let Some(e) = state.jobs.get_mut(&id) else {
        return;
    };
    let failure = execution.failure.as_ref().map(|f| f.error);
    let diagnostic = execution.failure.and_then(|f| f.diagnostic).map(|d| *d);
    e.observation.phase = if matches!(execution.outcome, TableSeedOutcome::Completed { .. }) {
        TableSeedPhase::Completed
    } else if failure == Some(TableSeedError::Cancelled)
        && execution.outcome != TableSeedOutcome::OutcomeUnknown
    {
        TableSeedPhase::Cancelled
    } else {
        TableSeedPhase::Failed
    };
    e.observation.outcome = execution.outcome;
    e.observation.failure = failure;
    e.observation.diagnostic = diagnostic.clone();
    e.observation.change_revision = revision;
    e.observation.receipt = Some(TableSeedReceipt {
        attempt_id: id,
        description: review.description().clone(),
        outcome: execution.outcome,
        failure,
        diagnostic,
    });
    e.observation.cleanup = if failure == Some(TableSeedError::Cleanup) {
        TableSeedCleanup::Failed
    } else {
        TableSeedCleanup::Complete
    };
    if e.observation.cleanup == TableSeedCleanup::Complete {
        e.execution.take();
        e.ready.take();
    }
    e.finished = Some(std::time::Instant::now());
}

async fn inspect(
    inner: &Inner,
    specs: &ResolvedPostgresConnectSpec,
    intent: TableSeedIntent,
    targets: TableSeedConnection,
    drivers: &crate::postgres::dedicated::DriverJoins,
    control: &Control,
) -> Result<Plan, runner::Failure> {
    #[cfg(test)]
    {
        let injected = inner.table_seed.inspector.lock().unwrap().clone();
        if let Some(inspect) = injected {
            return tokio::select! {biased;_=control.cancelled()=>Err(TableSeedError::Cancelled.into()),r=inspect(intent,targets)=>r};
        }
    }
    let _ = inner;
    runner::inspect(specs, intent, targets, drivers, control).await
}
async fn run(
    inner: &Inner,
    specs: &ResolvedPostgresConnectSpec,
    plan: Arc<Plan>,
    drivers: &crate::postgres::dedicated::DriverJoins,
    control: Arc<Control>,
) -> runner::Execution {
    #[cfg(test)]
    {
        let injected = inner.table_seed.runner.lock().unwrap().clone();
        if let Some(run) = injected {
            return run(control).await;
        }
    }
    let _ = inner;
    runner::execute(specs, plan, drivers, &control).await
}
