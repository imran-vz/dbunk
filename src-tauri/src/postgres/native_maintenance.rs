//! Dedicated maintenance execution. Transactional target changes use a COMMIT
//! fence; potentially partial commands use a dispatch fence and are never retried.
mod observation;
#[cfg(test)]
mod tests;
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, Notice, NoticeSink},
};
use crate::backend::maintenance::{
    MaintenanceFailure as Failure, MaintenanceNotice, MaintenanceOutcome as Outcome,
    MaintenancePreview as Preview, MaintenanceRelationKind as Kind,
    MaintenanceSemantics as Semantics, MaintenanceTarget as Target, WritePermit,
};
use futures_util::future::BoxFuture;
pub(crate) use observation::observe;
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, watch},
    time::Instant,
};
const CLEANUP_GRACE: Duration = Duration::from_secs(1);
const NOTICE_COUNT: usize = 8;
const NOTICE_MESSAGE_BYTES: usize = 128;
const NOTICE_SEVERITY_BYTES: usize = 16;

pub(crate) struct Execution {
    pub outcome: Outcome,
    pub notices: Vec<MaintenanceNotice>,
    pub notices_truncated: bool,
    pub runtime_ms: u64,
}
impl Execution {
    pub(crate) fn not_dispatched(reason: Failure) -> Self {
        Self {
            outcome: Outcome::NotDispatched { reason },
            notices: vec![],
            notices_truncated: false,
            runtime_ms: 0,
        }
    }
}
fn database_error(error: tokio_postgres::Error) -> Failure {
    match error.as_db_error() {
        Some(db) => Failure::Database {
            code: Some(db.code().code().into()),
        },
        None => Failure::Connection,
    }
}
trait Transport: Sized {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
    fn matches<'a>(&'a mut self, target: &'a Target) -> BoxFuture<'a, Result<bool, Failure>>;
    fn cleanup(self, cancel: bool, deadline: Instant) -> BoxFuture<'static, Diagnostics>;
}
#[derive(Default)]
struct Diagnostics {
    notices: Vec<MaintenanceNotice>,
    truncated: bool,
}
impl Diagnostics {
    fn drain(notices: &mut mpsc::Receiver<Notice>, dropped: &AtomicU32) -> Self {
        let mut retained = Vec::with_capacity(NOTICE_COUNT);
        while let Ok(notice) = notices.try_recv() {
            retained.push(MaintenanceNotice {
                severity: notice.severity,
                message: notice.message,
            });
        }
        Self {
            notices: retained,
            truncated: dropped.load(Ordering::Relaxed) != 0,
        }
    }
}
struct Socket {
    connection: DedicatedConnection,
    drivers: DriverJoins,
    notices: mpsc::Receiver<Notice>,
    dropped: Arc<AtomicU32>,
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
    fn matches<'a>(&'a mut self, target: &'a Target) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            let current =
                observation::load(&self.connection.client, target.schema(), target.name()).await?;
            Ok(current.as_ref() == Some(target))
        })
    }
    fn cleanup(mut self, cancel: bool, deadline: Instant) -> BoxFuture<'static, Diagnostics> {
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
            Diagnostics::drain(&mut self.notices, &self.dropped)
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
    spec: ResolvedPostgresConnectSpec,
    drivers: DriverJoins,
    permit: WritePermit,
    mut cancelled: watch::Receiver<u64>,
    target: Target,
    preview: Preview,
) -> Execution {
    let deadline = Instant::now() + Duration::from_millis(u64::from(preview.operation_timeout_ms));
    let (tx, mut notices) = mpsc::channel(NOTICE_COUNT);
    let dropped = Arc::new(AtomicU32::new(0));
    let sink = NoticeSink::Capped {
        tx,
        dropped: dropped.clone(),
        message_bytes: NOTICE_MESSAGE_BYTES,
        severity_bytes: NOTICE_SEVERITY_BYTES,
    };
    let connected = tokio::select! {
        biased;
        _ = interrupted(&permit, &mut cancelled) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = dedicated::connect_tracked(&spec, sink, Some(&drivers)) => result.map_err(|_| Failure::Connection),
    };
    match connected {
        Ok(connection) => {
            run(
                Socket {
                    connection,
                    drivers,
                    notices,
                    dropped,
                },
                &permit,
                cancelled,
                &target,
                &preview,
                deadline,
            )
            .await
        }
        Err(reason) => {
            join(&drivers, deadline.min(Instant::now() + CLEANUP_GRACE)).await;
            let diagnostics = Diagnostics::drain(&mut notices, &dropped);
            Execution {
                notices: diagnostics.notices,
                notices_truncated: diagnostics.truncated,
                ..Execution::not_dispatched(reason)
            }
        }
    }
}
async fn run(
    mut socket: impl Transport,
    permit: &WritePermit,
    mut cancelled: watch::Receiver<u64>,
    target: &Target,
    preview: &Preview,
    deadline: Instant,
) -> Execution {
    let start = Instant::now();
    let outcome = operation(
        &mut socket,
        permit,
        &mut cancelled,
        target,
        preview,
        deadline,
    )
    .await;
    let diagnostics = socket
        .cleanup(
            outcome != Outcome::Completed,
            deadline.min(Instant::now() + CLEANUP_GRACE),
        )
        .await;
    Execution {
        outcome,
        notices: diagnostics.notices,
        notices_truncated: diagnostics.truncated,
        runtime_ms: start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    }
}
async fn preparing<T>(
    future: impl std::future::Future<Output = Result<T, Failure>>,
    permit: &WritePermit,
    cancelled: &mut watch::Receiver<u64>,
    deadline: Instant,
) -> Result<T, Failure> {
    if Instant::now() >= deadline {
        return Err(Failure::Timeout);
    }
    tokio::select! {
        biased;
        _ = interrupted(permit, cancelled) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = future => result,
    }
}
async fn operation(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancelled: &mut watch::Receiver<u64>,
    target: &Target,
    preview: &Preview,
    deadline: Instant,
) -> Outcome {
    let transaction = preview.semantics == Semantics::Transactional;
    // The inherited statement_timeout is installed by dedicated::connect_tracked.
    // This independent lock bound never relaxes that configured statement limit.
    let setup = if transaction {
        "BEGIN; SET LOCAL lock_timeout = '10s'"
    } else {
        "SET lock_timeout = '10s'"
    };
    if let Err(reason) = preparing(socket.execute(setup), permit, cancelled, deadline).await {
        return Outcome::NotDispatched { reason };
    }
    match preparing(socket.matches(target), permit, cancelled, deadline).await {
        Ok(true) => (),
        Ok(false) => return Outcome::TargetChanged,
        Err(reason) => return Outcome::NotDispatched { reason },
    }
    if transaction && target.kind() == Kind::Table {
        // Relation locks narrow drop/rename races. They do not lock the namespace
        // name, so the review explicitly discloses the remaining schema-name race.
        let lock = format!(
            "LOCK TABLE ONLY \"{}\".\"{}\" IN ACCESS SHARE MODE",
            target.schema().replace('"', "\"\""),
            target.name().replace('"', "\"\"")
        );
        if let Err(reason) = preparing(socket.execute(&lock), permit, cancelled, deadline).await {
            return Outcome::NotDispatched { reason };
        }
        match preparing(socket.matches(target), permit, cancelled, deadline).await {
            Ok(true) => (),
            Ok(false) => return Outcome::TargetChanged,
            Err(reason) => return Outcome::NotDispatched { reason },
        }
    }
    if !transaction {
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
        return match settle(socket.execute(&preview.sql), cancelled, deadline).await {
            Ok(()) => Outcome::Completed,
            Err(reason @ Failure::Database { .. }) => {
                Outcome::InterruptedEffectsPossible { reason }
            }
            Err(reason) => Outcome::OutcomeUnknown { reason },
        };
    }
    if let Err(reason) = preparing(socket.execute(&preview.sql), permit, cancelled, deadline).await
    {
        return Outcome::RolledBack { reason };
    }
    if Instant::now() >= deadline {
        return Outcome::RolledBack {
            reason: Failure::Timeout,
        };
    }
    if !permit.admit_commit() {
        return Outcome::RolledBack {
            reason: Failure::Cancelled,
        };
    }
    match settle(socket.execute("COMMIT"), cancelled, deadline).await {
        Ok(()) => Outcome::Completed,
        Err(reason) => Outcome::OutcomeUnknown { reason },
    }
}
/// Once admitted, an available terminal reply wins. Cancellation gives this
/// same future one short bounded settlement window, never a fresh execution.
async fn settle(
    future: impl std::future::Future<Output = Result<(), Failure>>,
    cancelled: &mut watch::Receiver<u64>,
    deadline: Instant,
) -> Result<(), Failure> {
    tokio::pin!(future);
    tokio::select! {
        biased;
        result = &mut future => result,
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        _ = cancelled.changed() => match tokio::time::timeout_at(deadline.min(Instant::now() + CLEANUP_GRACE), &mut future).await {
            Ok(result) => result, Err(_) => Err(Failure::Cancelled),
        },
    }
}
