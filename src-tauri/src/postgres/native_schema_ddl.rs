//! One atomic native create-schema transaction on an owned dedicated socket.
//! Read cancellation helpers cannot be used here: COMMIT success is terminal.
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
};
use crate::backend::schema_ddl::{
    CreateSchemaFailure as Failure, CreateSchemaOutcome as Outcome, CreateSchemaPreview,
    WritePermit,
};
use futures_util::future::BoxFuture;
use std::time::Duration;
use tokio::{sync::watch, time::Instant};

/// Finite native operation bound, in addition to the configured server statement
/// timeout. Synchronous TLS material reads are outside Tokio preemption.
pub(crate) const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const CLEANUP_GRACE: Duration = Duration::from_secs(1);

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
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
    fn cleanup(self, cancel: bool, deadline: Instant) -> BoxFuture<'static, ()>;
}
struct Socket {
    connection: DedicatedConnection,
    drivers: DriverJoins,
}
impl Transport for Socket {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.connection
                .client
                .batch_execute(sql)
                .await
                .map_err(database_error)
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
async fn interrupted(permit: &WritePermit, cancelled: &mut watch::Receiver<u64>) {
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
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    permit: &WritePermit,
    mut cancelled: watch::Receiver<u64>,
    preview: &CreateSchemaPreview,
) -> Outcome {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let connection = tokio::select! {
        biased;
        _ = interrupted(permit, &mut cancelled) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        connection = dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers)) => connection.map_err(|_| Failure::Connection),
    };
    match connection {
        Ok(connection) => {
            run(
                Socket {
                    connection,
                    drivers: drivers.clone(),
                },
                permit,
                cancelled,
                preview,
                deadline,
            )
            .await
        }
        Err(reason) => {
            join(drivers, deadline.min(Instant::now() + CLEANUP_GRACE)).await;
            Outcome::NotApplied { reason }
        }
    }
}

/// The transport seam tests real transaction ordering and cleanup, with explicit
/// barriers instead of duplicating the state machine in mocks.
async fn run(
    mut socket: impl Transport,
    permit: &WritePermit,
    mut cancelled: watch::Receiver<u64>,
    preview: &CreateSchemaPreview,
    deadline: Instant,
) -> Outcome {
    let started = Instant::now();
    let outcome = transaction(&mut socket, permit, &mut cancelled, preview, deadline).await;
    let committed = outcome.is_ok();
    socket
        .cleanup(!committed, deadline.min(Instant::now() + CLEANUP_GRACE))
        .await;
    match outcome {
        Ok(()) => Outcome::Applied {
            statements: preview.statements.len() as u8,
            runtime_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        },
        Err((false, reason)) => Outcome::NotApplied { reason },
        Err((true, reason)) => Outcome::OutcomeUnknown { reason },
    }
}
async fn transaction(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancelled: &mut watch::Receiver<u64>,
    preview: &CreateSchemaPreview,
    deadline: Instant,
) -> Result<(), (bool, Failure)> {
    // No autocommit DDL and no standalone groups. Loss before COMMIT admission
    // cannot commit these transactional schema changes; socket teardown rolls back.
    for sql in ["BEGIN", "SET LOCAL lock_timeout = '10s'"]
        .into_iter()
        .chain(
            preview
                .statements
                .iter()
                .map(|statement| statement.sql.as_str()),
        )
    {
        if Instant::now() >= deadline {
            return Err((false, Failure::Timeout));
        }
        let result = tokio::select! {
            biased;
            _ = interrupted(permit, cancelled) => Err(Failure::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
            result = socket.execute(sql) => result,
        };
        result.map_err(|reason| (false, reason))?;
    }
    if Instant::now() >= deadline {
        return Err((false, Failure::Timeout));
    }
    if !permit.admit_commit() {
        return Err((false, Failure::Cancelled));
    }
    let commit = socket.execute("COMMIT");
    tokio::pin!(commit);
    // Once admitted, prefer an available terminal COMMIT reply. Cancel/retire
    // gives this same in-flight future a short joined grace, never a fresh COMMIT.
    tokio::select! {
        biased;
        result = &mut commit => result.map_err(|reason| (true, reason)),
        _ = tokio::time::sleep_until(deadline) => Err((true, Failure::Timeout)),
        _ = cancelled.changed() => {
            match tokio::time::timeout_at(deadline.min(Instant::now() + CLEANUP_GRACE), &mut commit).await {
                Ok(result) => result.map_err(|reason| (true, reason)),
                Err(_) => Err((true, Failure::Cancelled)),
            }
        }
    }
}

#[cfg(test)]
mod tests;
