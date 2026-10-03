//! Existing-schema comment and rename on an owned dedicated socket.
//! PostgreSQL has no LOCK SCHEMA statement. The observed namespace row (OID,
//! name, comment and row version) is rechecked inside the transaction before
//! dispatch and again after the statement; any difference rolls back. These
//! guards detect change; they are not a namespace lock or a hook sandbox.
mod catalog;
#[cfg(test)]
mod tests;
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
};
use crate::backend::schema_alter::{
    SchemaAlterDescription, SchemaAlterFailure as Failure, SchemaAlterIntent,
    SchemaAlterOutcome as Outcome, SchemaAlterPreview, SchemaAlterRequest, WritePermit,
    SCHEMA_ALTER_OPERATION_TIMEOUT_MS,
};
pub(crate) use catalog::observe;
use futures_util::future::BoxFuture;
use std::time::Duration;
use tokio::{sync::watch, time::Instant};
const CLEANUP_GRACE: Duration = Duration::from_secs(1);
fn database_error(error: tokio_postgres::Error) -> Failure {
    Failure::Database {
        code: error
            .code()
            .map(|code| code.code())
            .filter(|s| s.len() == 5 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            .map(str::to_owned),
    }
}
trait Transport: Sized {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
    fn capture<'a>(
        &'a mut self,
        request: &'a SchemaAlterRequest,
    ) -> BoxFuture<'a, Result<SchemaAlterDescription, Failure>>;
    /// True only when this transaction wrote the visible namespace row version.
    fn owned_version(&mut self, schema_oid: u32) -> BoxFuture<'_, Result<bool, Failure>>;
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
    fn capture<'a>(
        &'a mut self,
        request: &'a SchemaAlterRequest,
    ) -> BoxFuture<'a, Result<SchemaAlterDescription, Failure>> {
        Box::pin(catalog::capture(&self.connection.client, request))
    }
    fn owned_version(&mut self, schema_oid: u32) -> BoxFuture<'_, Result<bool, Failure>> {
        Box::pin(catalog::owned_version(&self.connection.client, schema_oid))
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
async fn interrupted(permit: &WritePermit, cancellation: &mut watch::Receiver<u64>) {
    loop {
        if !permit.check_preparing() {
            return;
        }
        if cancellation.changed().await.is_err() {
            return;
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    spec: ResolvedPostgresConnectSpec,
    drivers: DriverJoins,
    permit: WritePermit,
    mut cancellation: watch::Receiver<u64>,
    target: SchemaAlterDescription,
    intent: SchemaAlterIntent,
    preview: SchemaAlterPreview,
) -> Outcome {
    let deadline = Instant::now() + Duration::from_millis(SCHEMA_ALTER_OPERATION_TIMEOUT_MS.into());
    let connection = tokio::select! {
        biased;
        _ = interrupted(&permit, &mut cancellation) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        value = dedicated::connect_tracked(&spec, NoticeSink::Ignore, Some(&drivers)) => value.map_err(|_| Failure::Connection),
    };
    match connection {
        Ok(connection) => {
            run(
                Socket {
                    connection,
                    drivers,
                },
                &permit,
                cancellation,
                &target,
                &intent,
                &preview,
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
/// Only a real ROLLBACK acknowledgement can establish RolledBack. Lost rollback
/// or COMMIT delivery remains unknown; every socket driver joins before return.
async fn run(
    mut socket: impl Transport,
    permit: &WritePermit,
    mut cancellation: watch::Receiver<u64>,
    target: &SchemaAlterDescription,
    intent: &SchemaAlterIntent,
    preview: &SchemaAlterPreview,
    deadline: Instant,
) -> Outcome {
    let started = Instant::now();
    let mut dispatched = false;
    let result = transaction(
        &mut socket,
        permit,
        &mut cancellation,
        target,
        intent,
        preview,
        deadline,
        &mut dispatched,
    )
    .await;
    let outcome = match result {
        Ok(()) => Outcome::Applied {
            runtime_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        },
        Err((true, reason)) => Outcome::OutcomeUnknown { reason },
        Err((false, reason)) if !dispatched => Outcome::NotDispatched { reason },
        Err((false, reason)) => match tokio::time::timeout_at(
            deadline.min(Instant::now() + CLEANUP_GRACE),
            socket.execute("ROLLBACK"),
        )
        .await
        {
            Ok(Ok(())) => Outcome::RolledBack { reason },
            _ => Outcome::OutcomeUnknown {
                reason: Failure::RollbackUnconfirmed,
            },
        },
    };
    socket
        .cleanup(
            !matches!(
                outcome,
                Outcome::Applied { .. } | Outcome::RolledBack { .. }
            ),
            deadline.min(Instant::now() + CLEANUP_GRACE),
        )
        .await;
    outcome
}
#[allow(clippy::too_many_arguments)]
async fn transaction(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    target: &SchemaAlterDescription,
    intent: &SchemaAlterIntent,
    preview: &SchemaAlterPreview,
    deadline: Instant,
    dispatched: &mut bool,
) -> Result<(), (bool, Failure)> {
    let preparing = async {
        socket
            .execute("BEGIN ISOLATION LEVEL READ COMMITTED")
            .await?;
        // Lower only; never widen a shorter inherited lock timeout.
        socket.execute("SELECT pg_catalog.set_config('lock_timeout', CASE WHEN pg_catalog.current_setting('lock_timeout')::interval=interval '0' THEN '10000' ELSE LEAST(EXTRACT(EPOCH FROM pg_catalog.current_setting('lock_timeout')::interval)*1000,10000)::bigint::text END, true)").await?;
        // Exact observed OID, name, comment and namespace row version.
        guard(socket, target).await?;
        // The flag precedes polling dispatch, so a dropped/failed reply never
        // reports NotDispatched for SQL that might have reached the server.
        if !permit.check_preparing() {
            return Err(Failure::Cancelled);
        }
        *dispatched = true;
        socket.execute(&preview.sql).await?;
        verify(socket, target, intent).await
    };
    let prepared = tokio::select! {
        biased;
        _ = interrupted(permit, cancellation) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = preparing => result,
    };
    prepared.map_err(|failure| (false, failure))?;
    if Instant::now() >= deadline {
        return Err((false, Failure::Timeout));
    }
    if !permit.admit_commit() {
        return Err((false, Failure::Cancelled));
    }
    let commit = socket.execute("COMMIT");
    tokio::pin!(commit);
    tokio::select! {
        biased;
        result = &mut commit => result.map_err(|failure| (true, failure)),
        _ = tokio::time::sleep_until(deadline) => Err((true, Failure::Timeout)),
        _ = cancellation.changed() => match tokio::time::timeout_at(deadline.min(Instant::now()+CLEANUP_GRACE), &mut commit).await {
            Ok(result) => result.map_err(|failure| (true, failure)),
            Err(_) => Err((true, Failure::Cancelled)),
        }
    }
}
async fn guard(
    socket: &mut impl Transport,
    expected: &SchemaAlterDescription,
) -> Result<(), Failure> {
    let actual = socket.capture(&expected.request()).await?;
    if actual != *expected {
        return Err(Failure::TargetChanged);
    }
    Ok(())
}
/// After the statement, the same OID must carry exactly the intended state.
/// A comment leaves the namespace row untouched, so its version must be the
/// observed one. A rename rewrites the row, so the visible version must be the
/// one this transaction wrote; a concurrent committed writer cannot match.
async fn verify(
    socket: &mut impl Transport,
    target: &SchemaAlterDescription,
    intent: &SchemaAlterIntent,
) -> Result<(), Failure> {
    let expected = post_target(target, intent);
    match intent {
        SchemaAlterIntent::SetComment { .. } => guard(socket, &expected).await,
        SchemaAlterIntent::Rename { .. } => {
            let actual = socket.capture(&expected.request()).await?;
            if actual.identity != expected.identity
                || actual.schema != expected.schema
                || actual.comment != expected.comment
                || !socket.owned_version(target.identity.schema_oid).await?
            {
                return Err(Failure::TargetChanged);
            }
            Ok(())
        }
    }
}
fn post_target(
    target: &SchemaAlterDescription,
    intent: &SchemaAlterIntent,
) -> SchemaAlterDescription {
    let mut expected = target.clone();
    match intent {
        SchemaAlterIntent::SetComment { comment } => {
            // PostgreSQL CreateComments reduces empty text to NULL. Keep the
            // reviewed intent exact, but verify the server's removal semantics.
            expected.comment = comment.clone().filter(|text| !text.is_empty());
        }
        SchemaAlterIntent::Rename { new_name } => expected.schema.clone_from(new_name),
    }
    expected
}
