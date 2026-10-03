//! Typed object DDL on one owned connection. Identity guards detect changes;
//! they are not a namespace lock or an external-effect sandbox.
//!
//! Atomic groups recheck every claim after BEGIN and verify each statement's
//! exact effect (for existing objects: the observed OID is the one this
//! transaction locked) before COMMIT. Standalone statements are rechecked
//! immediately before dispatch and commit alone. Only a real ROLLBACK
//! acknowledgement establishes RolledBack and only a server error reply
//! establishes Rejected; a lost reply after dispatch is always unknown.
mod catalog;
#[cfg(test)]
mod tests;
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
};
use crate::backend::object_ddl::{
    ClaimSpec, ObjectDdlClaim, ObjectDdlDescription, ObjectDdlFailure as Failure, ObjectDdlGroup,
    ObjectDdlOperation, ObjectDdlOutcome as Outcome, ObjectDdlPreview, ObjectDdlResidue,
    ObjectDdlStop as Stop, WritePermit, OBJECT_DDL_OPERATION_TIMEOUT_MS,
};
pub(crate) use catalog::observe;
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};
const CLEANUP_GRACE: Duration = Duration::from_secs(1);
/// Lower only; never widen a shorter inherited lock timeout.
const LOCK_TIMEOUT: &str = "SELECT pg_catalog.set_config('lock_timeout', CASE WHEN pg_catalog.current_setting('lock_timeout')::interval=interval '0' THEN '10000' ELSE LEAST(EXTRACT(EPOCH FROM pg_catalog.current_setting('lock_timeout')::interval)*1000,10000)::bigint::text END, false)";

/// A server error reply is definite; anything else (I/O, closed socket) is not.
fn failure(error: tokio_postgres::Error) -> Failure {
    match error.as_db_error() {
        Some(database) => Failure::Database {
            code: Some(database.code().code())
                .filter(|s| s.len() == 5 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
                .map(str::to_owned),
        },
        None => Failure::Connection,
    }
}

type Canceller = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

trait Transport: Sized + Send {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
    fn database_oid(&mut self) -> BoxFuture<'_, Result<u32, Failure>>;
    fn capture<'a>(
        &'a mut self,
        spec: &'a ClaimSpec,
    ) -> BoxFuture<'a, Result<ObjectDdlClaim, Failure>>;
    /// Exact post-statement effect inside the still-open transaction.
    fn verify<'a>(
        &'a mut self,
        operation: &'a ObjectDdlOperation,
        claims: &'a [ObjectDdlClaim],
    ) -> BoxFuture<'a, Result<bool, Failure>>;
    fn residue<'a>(
        &'a mut self,
        operation: &'a ObjectDdlOperation,
        claims: &'a [ObjectDdlClaim],
    ) -> BoxFuture<'a, Option<ObjectDdlResidue>>;
    /// A server-side cancel request usable while a statement borrows the socket.
    fn canceller(&self) -> Canceller;
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
                .map_err(failure)
        })
    }
    fn database_oid(&mut self) -> BoxFuture<'_, Result<u32, Failure>> {
        Box::pin(catalog::database_oid(&self.connection.client))
    }
    fn capture<'a>(
        &'a mut self,
        spec: &'a ClaimSpec,
    ) -> BoxFuture<'a, Result<ObjectDdlClaim, Failure>> {
        Box::pin(catalog::capture(&self.connection.client, spec))
    }
    fn verify<'a>(
        &'a mut self,
        operation: &'a ObjectDdlOperation,
        claims: &'a [ObjectDdlClaim],
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(catalog::verify(&self.connection.client, operation, claims))
    }
    fn residue<'a>(
        &'a mut self,
        operation: &'a ObjectDdlOperation,
        claims: &'a [ObjectDdlClaim],
    ) -> BoxFuture<'a, Option<ObjectDdlResidue>> {
        Box::pin(catalog::residue(&self.connection.client, operation, claims))
    }
    fn canceller(&self) -> Canceller {
        let token = self.connection.cancel.clone();
        let tls = self.connection.tls.clone();
        Arc::new(move || {
            let token = token.clone();
            let tls = tls.clone();
            Box::pin(async move {
                let _ = dedicated::cancel(token, tls).await;
            })
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

pub(crate) async fn execute(
    spec: ResolvedPostgresConnectSpec,
    drivers: DriverJoins,
    permit: WritePermit,
    mut cancellation: watch::Receiver<u64>,
    description: ObjectDdlDescription,
    operations: Vec<ObjectDdlOperation>,
    preview: ObjectDdlPreview,
) -> Outcome {
    let deadline = Instant::now() + Duration::from_millis(OBJECT_DDL_OPERATION_TIMEOUT_MS.into());
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
                &description,
                &operations,
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

/// How the run ended, before the committed prefix is attached.
enum End {
    Done,
    /// The group starting at `at` did not commit; nothing after it was sent.
    Stopped {
        at: usize,
        stop: Stop,
        reason: Failure,
        residue: Option<ObjectDdlResidue>,
    },
    /// Statements before `end` may have committed.
    Unknown {
        end: usize,
        reason: Failure,
    },
}

fn consistent(
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
) -> bool {
    let mut next = 0usize;
    description.claims.len() == operations.len()
        && preview.statements.len() == operations.len()
        && preview.groups.iter().all(|group| {
            group.statements().into_iter().all(|index| {
                let ordered = index == next;
                next += 1;
                ordered
            })
        })
        && next == operations.len()
        && preview.groups.iter().all(|group| match group {
            ObjectDdlGroup::Atomic { statements } => statements
                .iter()
                .all(|i| preview.statements[usize::from(*i)].transactional),
            ObjectDdlGroup::Standalone { statement } => {
                !preview.statements[usize::from(*statement)].transactional
            }
        })
}

/// Every socket driver joins before return, whatever the outcome.
async fn run(
    mut socket: impl Transport,
    permit: &WritePermit,
    mut cancellation: watch::Receiver<u64>,
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
    deadline: Instant,
) -> Outcome {
    let started = Instant::now();
    let mut committed = 0usize;
    let end = if consistent(description, operations, preview) {
        groups(
            &mut socket,
            permit,
            &mut cancellation,
            description,
            operations,
            preview,
            deadline,
            &mut committed,
        )
        .await
    } else {
        End::Stopped {
            at: 0,
            stop: Stop::NotDispatched,
            reason: Failure::Limit,
            residue: None,
        }
    };
    let clean = matches!(
        end,
        End::Done
            | End::Stopped {
                stop: Stop::RolledBack | Stop::Rejected,
                ..
            }
    );
    let narrow = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    let outcome = match end {
        End::Done => Outcome::Applied {
            runtime_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        },
        End::Stopped {
            stop: Stop::NotDispatched,
            reason,
            residue: None,
            ..
        } if committed == 0 => Outcome::NotDispatched { reason },
        End::Stopped {
            at,
            stop,
            reason,
            residue,
        } => Outcome::Stopped {
            committed: narrow(committed),
            stopped_at: narrow(at),
            stop,
            reason,
            residue,
        },
        End::Unknown { end, reason } => Outcome::OutcomeUnknown {
            committed: narrow(committed),
            uncertain_end: narrow(end),
            reason,
        },
    };
    socket
        .cleanup(!clean, deadline.min(Instant::now() + CLEANUP_GRACE))
        .await;
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn groups(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
    deadline: Instant,
    committed: &mut usize,
) -> End {
    let stopped = |at, reason| End::Stopped {
        at,
        stop: Stop::NotDispatched,
        reason,
        residue: None,
    };
    let session = tokio::select! {
        biased;
        _ = interrupted(permit, cancellation) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = socket.execute(LOCK_TIMEOUT) => result,
    };
    if let Err(reason) = session {
        return stopped(0, reason);
    }
    for (position, group) in preview.groups.iter().enumerate() {
        let indexes = group.statements();
        let at = indexes[0];
        // The previous group's admission must not swallow a cancel that
        // arrived during its COMMIT or standalone statement.
        if position > 0 && !permit.readmit() {
            return stopped(at, Failure::Cancelled);
        }
        if Instant::now() >= deadline {
            return stopped(at, Failure::Timeout);
        }
        let end = match group {
            ObjectDdlGroup::Atomic { .. } => {
                atomic(
                    socket,
                    permit,
                    cancellation,
                    description,
                    operations,
                    preview,
                    &indexes,
                    deadline,
                )
                .await
            }
            ObjectDdlGroup::Standalone { .. } => {
                standalone(
                    socket,
                    permit,
                    cancellation,
                    description,
                    operations,
                    preview,
                    at,
                    deadline,
                )
                .await
            }
        };
        match end {
            End::Done => *committed += indexes.len(),
            other => return other,
        }
    }
    End::Done
}

async fn recheck(
    socket: &mut impl Transport,
    description: &ObjectDdlDescription,
    indexes: &[usize],
) -> Result<(), Failure> {
    if socket.database_oid().await? != description.database_oid {
        return Err(Failure::TargetChanged);
    }
    for index in indexes {
        for claim in &description.claims[*index] {
            if socket.capture(&claim.spec()).await? != *claim {
                return Err(Failure::TargetChanged);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn atomic(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
    indexes: &[usize],
    deadline: Instant,
) -> End {
    let at = indexes[0];
    let end = at + indexes.len();
    let cancel = socket.canceller();
    let mut dispatched = false;
    let result = transaction(
        socket,
        permit,
        cancellation,
        description,
        operations,
        preview,
        indexes,
        deadline,
        &mut dispatched,
    )
    .await;
    match result {
        Ok(()) => End::Done,
        Err((true, reason)) => End::Unknown { end, reason },
        Err((false, reason)) if !dispatched => {
            // Only reads ran in this transaction; closing the socket ends it.
            End::Stopped {
                at,
                stop: Stop::NotDispatched,
                reason,
                residue: None,
            }
        }
        Err((false, reason)) => {
            let grace = deadline.min(Instant::now() + CLEANUP_GRACE);
            if matches!(reason, Failure::Cancelled | Failure::Timeout) {
                // A dropped statement future may still run server-side.
                let _ = tokio::time::timeout_at(grace, cancel()).await;
            }
            match tokio::time::timeout_at(grace, socket.execute("ROLLBACK")).await {
                Ok(Ok(())) => End::Stopped {
                    at,
                    stop: Stop::RolledBack,
                    reason,
                    residue: None,
                },
                _ => End::Unknown {
                    end,
                    reason: Failure::RollbackUnconfirmed,
                },
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn transaction(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
    indexes: &[usize],
    deadline: Instant,
    dispatched: &mut bool,
) -> Result<(), (bool, Failure)> {
    let preparing = async {
        socket
            .execute("BEGIN ISOLATION LEVEL READ COMMITTED")
            .await?;
        recheck(socket, description, indexes).await?;
        // The flag precedes polling dispatch, so a dropped/failed reply never
        // reports NotDispatched for SQL that might have reached the server.
        if !permit.check_preparing() {
            return Err(Failure::Cancelled);
        }
        *dispatched = true;
        for index in indexes {
            socket.execute(&preview.statements[*index].sql).await?;
        }
        for index in indexes {
            if !socket
                .verify(&operations[*index], &description.claims[*index])
                .await?
            {
                return Err(Failure::TargetChanged);
            }
        }
        Ok(())
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

#[allow(clippy::too_many_arguments)]
async fn standalone(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
    index: usize,
    deadline: Instant,
) -> End {
    let stopped = |reason| End::Stopped {
        at: index,
        stop: Stop::NotDispatched,
        reason,
        residue: None,
    };
    let checked = tokio::select! {
        biased;
        _ = interrupted(permit, cancellation) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = recheck(socket, description, std::slice::from_ref(&index)) => result,
    };
    if let Err(reason) = checked {
        return stopped(reason);
    }
    if Instant::now() >= deadline {
        return stopped(Failure::Timeout);
    }
    // The effect boundary is dispatch: the same fence as COMMIT admission.
    if !permit.admit_dispatch() {
        return stopped(Failure::Cancelled);
    }
    let cancel = socket.canceller();
    let (result, interrupted_by) = {
        let statement = socket.execute(&preview.statements[index].sql);
        tokio::pin!(statement);
        let interruption = tokio::select! {
            biased;
            result = &mut statement => Ok(result),
            _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
            _ = cancellation.changed() => Err(Failure::Cancelled),
        };
        match interruption {
            Ok(result) => (Some(result), None),
            Err(reason) => {
                // A bounded grace beyond the deadline, only to learn whether
                // the server answered the cancel with an error or a commit.
                let grace = Instant::now() + CLEANUP_GRACE;
                let _ = tokio::time::timeout_at(grace, cancel()).await;
                (
                    tokio::time::timeout_at(grace, &mut statement).await.ok(),
                    Some(reason),
                )
            }
        }
    };
    match (result, interrupted_by) {
        (Some(Ok(())), _) => End::Done,
        (Some(Err(reason @ Failure::Database { .. })), interrupted_by) => {
            // A server error reply is definite: this statement did not apply,
            // though a failed concurrent index build can leave residue.
            let residue = tokio::time::timeout_at(
                Instant::now() + CLEANUP_GRACE,
                socket.residue(&operations[index], &description.claims[index]),
            )
            .await
            .unwrap_or(Some(ObjectDdlResidue::Unverified));
            End::Stopped {
                at: index,
                stop: Stop::Rejected,
                reason: interrupted_by.unwrap_or(reason),
                residue,
            }
        }
        (Some(Err(reason)), interrupted_by) => End::Unknown {
            end: index + 1,
            reason: interrupted_by.unwrap_or(reason),
        },
        (None, interrupted_by) => End::Unknown {
            end: index + 1,
            reason: interrupted_by.unwrap_or(Failure::Cancelled),
        },
    }
}
