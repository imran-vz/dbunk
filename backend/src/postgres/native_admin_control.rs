//! Native session signals on a dedicated, joined socket. Signals are external
//! effects: BEGIN/ROLLBACK cannot undo them. An acknowledged true means sent,
//! not stopped. PostgreSQL cannot lock backend lifetime through signal delivery;
//! the identity predicate narrows the stale-PID/query race but cannot remove it.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
};
use crate::backend::admin::{
    AdminControlAction as Action, AdminControlFailure as Failure, AdminControlOutcome as Outcome,
    AdminControlTarget as Target, ControlPermit,
};
use futures_util::future::BoxFuture;
use std::time::Duration;
use tokio::{sync::watch, time::Instant};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_GRACE: Duration = Duration::from_secs(1);
const CANCEL: &str = "SELECT pg_catalog.pg_cancel_backend(a.pid) FROM pg_catalog.pg_stat_activity AS a WHERE a.pid=$1 AND a.backend_start=$2::text::timestamptz AND a.datname::text IS NOT DISTINCT FROM $3::text AND a.query_start IS NOT DISTINCT FROM $4::text::timestamptz AND a.pid<>pg_catalog.pg_backend_pid()";
const TERMINATE: &str = "SELECT pg_catalog.pg_terminate_backend(a.pid) FROM pg_catalog.pg_stat_activity AS a WHERE a.pid=$1 AND a.backend_start=$2::text::timestamptz AND a.datname::text IS NOT DISTINCT FROM $3::text AND a.pid<>pg_catalog.pg_backend_pid()";

fn database_error(error: tokio_postgres::Error) -> Failure {
    Failure::Database {
        code: error
            .code()
            .map(|code| code.code())
            .filter(|code| code.len() == 5 && code.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .map(str::to_owned),
    }
}
trait Transport: Sized {
    fn signal<'a>(
        &'a mut self,
        target: &'a Target,
        action: Action,
    ) -> BoxFuture<'a, Result<Option<bool>, Failure>>;
    fn cleanup(self, cancel: bool, deadline: Instant) -> BoxFuture<'static, ()>;
}
struct Socket {
    connection: DedicatedConnection,
    drivers: DriverJoins,
}
impl Transport for Socket {
    fn signal<'a>(
        &'a mut self,
        target: &'a Target,
        action: Action,
    ) -> BoxFuture<'a, Result<Option<bool>, Failure>> {
        Box::pin(async move {
            let pid = target.pid();
            let start = target.backend_start();
            let database = target.database();
            let query_start = target.query_start();
            let row = match action {
                Action::CancelQuery => {
                    self.connection
                        .client
                        .query_opt(CANCEL, &[&pid, &start, &database, &query_start])
                        .await
                }
                Action::TerminateSession => {
                    self.connection
                        .client
                        .query_opt(TERMINATE, &[&pid, &start, &database])
                        .await
                }
            }
            .map_err(database_error)?;
            row.map(|row| row.try_get::<_, bool>(0).map_err(database_error))
                .transpose()
        })
    }
    fn cleanup(self, cancel: bool, deadline: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            if cancel {
                let _ = tokio::time::timeout_at(
                    deadline,
                    dedicated::cancel(self.connection.cancel.clone(), self.connection.tls.clone()),
                )
                .await;
            }
            if tokio::time::timeout_at(deadline, self.connection.close())
                .await
                .is_err()
            {
                self.drivers.abort_all();
            }
            join(&self.drivers, deadline).await;
        })
    }
}
async fn join(drivers: &DriverJoins, deadline: Instant) {
    if tokio::time::timeout_at(deadline, drivers.drain())
        .await
        .is_err()
    {
        drivers.abort_all();
        drivers.drain().await;
    }
}
async fn interrupted(permit: &ControlPermit, cancelled: &mut watch::Receiver<u64>) {
    loop {
        if !permit.check_preparing() {
            return;
        }
        if cancelled.changed().await.is_err() {
            return;
        }
    }
}
pub(crate) async fn execute(
    spec: ResolvedPostgresConnectSpec,
    drivers: DriverJoins,
    permit: ControlPermit,
    mut cancelled: watch::Receiver<u64>,
    target: Target,
    action: Action,
) -> Outcome {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let connected = tokio::select! {
        biased;
        _ = interrupted(&permit, &mut cancelled) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        connected = dedicated::connect_tracked(&spec, NoticeSink::Ignore, Some(&drivers)) => connected.map_err(|_| Failure::Connection),
    };
    match connected {
        Ok(connection) => {
            run(
                Socket {
                    connection,
                    drivers,
                },
                &permit,
                cancelled,
                &target,
                action,
                deadline,
            )
            .await
        }
        Err(reason) => {
            join(&drivers, deadline.min(Instant::now() + CLEANUP_GRACE)).await;
            Outcome::NotDispatched { reason }
        }
    }
}
async fn run(
    mut socket: impl Transport,
    permit: &ControlPermit,
    mut cancelled: watch::Receiver<u64>,
    target: &Target,
    action: Action,
    deadline: Instant,
) -> Outcome {
    let outcome = signal(
        &mut socket,
        permit,
        &mut cancelled,
        target,
        action,
        deadline,
    )
    .await;
    socket
        .cleanup(
            matches!(outcome, Outcome::OutcomeUnknown { .. }),
            deadline.min(Instant::now() + CLEANUP_GRACE),
        )
        .await;
    outcome
}
async fn signal(
    socket: &mut impl Transport,
    permit: &ControlPermit,
    cancelled: &mut watch::Receiver<u64>,
    target: &Target,
    action: Action,
    deadline: Instant,
) -> Outcome {
    if Instant::now() >= deadline {
        return Outcome::NotDispatched {
            reason: Failure::Timeout,
        };
    }
    if !permit.admit_dispatch() {
        return Outcome::NotDispatched {
            reason: Failure::Cancelled,
        };
    }
    let pending = socket.signal(target, action);
    tokio::pin!(pending);
    // Preserve an available reply over late cancellation. Settle this same
    // future once; neither cancellation nor lost delivery can retry the signal.
    let result = tokio::select! {
        biased;
        result = &mut pending => result,
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        _ = cancelled.changed() => match tokio::time::timeout_at(deadline.min(Instant::now() + CLEANUP_GRACE), &mut pending).await {
            Ok(result) => result,
            Err(_) => Err(Failure::Cancelled),
        },
    };
    match result {
        Ok(None) => Outcome::TargetChanged,
        Ok(Some(false)) => Outcome::SignalNotSent,
        Ok(Some(true)) => Outcome::SignalSent,
        Err(reason) => Outcome::OutcomeUnknown { reason },
    }
}

#[cfg(test)]
mod tests;
