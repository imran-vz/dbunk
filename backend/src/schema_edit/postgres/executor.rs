//! Runs a reviewed plan on one owned connection.
//!
//! The atomic group: BEGIN, lower `lock_timeout` to 10 s for that
//! transaction only, `LOCK TABLE … IN ACCESS EXCLUSIVE MODE`, recompute the
//! drift token and compare, run the statements, re-capture the table inside
//! the transaction and verify it against the draft, then COMMIT. Any failure
//! rolls back. Standalone statements (`VALIDATE CONSTRAINT`, `CREATE INDEX
//! CONCURRENTLY`) follow under the connection's inherited settings, each
//! after re-checking the table OID; a failure there reports the committed
//! prefix, and a failed concurrent build's leftover invalid index is dropped.
//! Once they all succeed the table is re-captured in a read-only
//! transaction and verified again; a mismatch or an unanswered read is
//! OutcomeUnknown, since the statements committed and cannot be rolled back.
//!
//! Only a real ROLLBACK acknowledgement establishes RolledBack; a lost reply
//! after dispatch is always OutcomeUnknown.
use super::capture::drift_oid;
use super::catalog;
use super::planner::{self, Built};
use super::verify::{type_requests, Expectation};
use crate::backend::table_ddl::WritePermit;
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink};
use crate::quote_double;
use crate::schema_edit::{
    DriftToken, Origin, SchemaBaseline, SchemaDraft, SchemaEditFailure as Failure,
    SchemaEditOutcome as Outcome, SchemaEditPreview, TableModel,
};
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};

/// Outer bound for one apply, including table rewrites and concurrent index
/// builds. Each statement is also bounded by the connection's inherited
/// statement_timeout, and cancellation is honoured throughout.
pub(super) const OPERATION_TIMEOUT: Duration = Duration::from_secs(600);
const CLEANUP_GRACE: Duration = Duration::from_secs(1);
/// Bounds one cleanup of a failed concurrent build's invalid index.
const INDEX_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Lowers `lock_timeout` to at most 10 s for the atomic group's transaction
/// only (`is_local`), so it neither bounds the standalone statements nor
/// leaks into a pooled session. Lower only: a shorter inherited timeout is
/// kept, never widened, and never rounded down to 0 (which disables it).
pub(super) const LOCK_TIMEOUT_LOCAL: &str = "SELECT pg_catalog.set_config('lock_timeout', CASE WHEN pg_catalog.current_setting('lock_timeout')::interval=interval '0' THEN '10000' ELSE GREATEST(LEAST(ROUND(EXTRACT(EPOCH FROM pg_catalog.current_setting('lock_timeout')::interval)*1000),10000),1)::bigint::text END, true)";
pub(super) const BEGIN: &str = "BEGIN ISOLATION LEVEL READ COMMITTED";
/// Wraps the final verification so `SET LOCAL` in the capture applies.
pub(super) const BEGIN_VERIFY: &str = "BEGIN ISOLATION LEVEL READ COMMITTED READ ONLY";
/// Bound for the verification read after standalone statements.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) type Canceller = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

pub(super) trait Transport: Sized + Send {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
    /// The current drift token of `schema.table`; `None` when it is gone.
    fn drift<'a>(
        &'a mut self,
        schema: &'a str,
        table: &'a str,
    ) -> BoxFuture<'a, Result<Option<DriftToken>, Failure>>;
    fn relation<'a>(
        &'a mut self,
        schema: &'a str,
        table: &'a str,
    ) -> BoxFuture<'a, Result<Option<u32>, Failure>>;
    /// True when `schema.index` exists on table `table` and is invalid.
    fn invalid_index<'a>(
        &'a mut self,
        schema: &'a str,
        index: &'a str,
        table: u32,
    ) -> BoxFuture<'a, Result<bool, Failure>>;
    /// Re-captures `schema.table` inside the open transaction, and the
    /// server's rendering of each draft type in `types`.
    fn observe<'a>(
        &'a mut self,
        schema: &'a str,
        table: &'a str,
        types: &'a [String],
    ) -> BoxFuture<'a, Result<(TableModel, Vec<String>), Failure>>;
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
                .map_err(catalog::failure)
        })
    }
    fn drift<'a>(
        &'a mut self,
        schema: &'a str,
        table: &'a str,
    ) -> BoxFuture<'a, Result<Option<DriftToken>, Failure>> {
        Box::pin(async move {
            catalog::drift(&self.connection.client, schema, table)
                .await
                .map(|found| found.map(|(token, _)| token))
                .map_err(catalog::failure)
        })
    }
    fn relation<'a>(
        &'a mut self,
        schema: &'a str,
        table: &'a str,
    ) -> BoxFuture<'a, Result<Option<u32>, Failure>> {
        Box::pin(catalog::relation(&self.connection.client, schema, table))
    }
    fn invalid_index<'a>(
        &'a mut self,
        schema: &'a str,
        index: &'a str,
        table: u32,
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(catalog::invalid_index(
            &self.connection.client,
            schema,
            index,
            table,
        ))
    }
    fn observe<'a>(
        &'a mut self,
        schema: &'a str,
        table: &'a str,
        types: &'a [String],
    ) -> BoxFuture<'a, Result<(TableModel, Vec<String>), Failure>> {
        Box::pin(catalog::observe(
            &self.connection.client,
            schema,
            table,
            types,
        ))
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    spec: ResolvedPostgresConnectSpec,
    drivers: DriverJoins,
    permit: WritePermit,
    mut cancellation: watch::Receiver<u64>,
    baseline: SchemaBaseline,
    draft: SchemaDraft,
    preview: SchemaEditPreview,
) -> Outcome {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
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
                &baseline,
                &draft,
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

/// The plan as rebuilt here must be exactly the reviewed one: the facade has
/// already compared previews, and this is what tells each statement's role.
fn rebuild(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    preview: &SchemaEditPreview,
) -> Result<Built, Failure> {
    let built = planner::build(baseline, draft, &preview.options).map_err(|error| {
        Failure::Unsupported {
            reason: error.to_string(),
        }
    })?;
    if built.statements != preview.statements || built.groups() != preview.groups {
        return Err(Failure::Unsupported {
            reason: "The reviewed plan no longer matches the draft; review again".into(),
        });
    }
    Ok(built)
}

/// Every socket driver joins before return, whatever the outcome.
pub(super) async fn run(
    mut socket: impl Transport,
    permit: &WritePermit,
    mut cancellation: watch::Receiver<u64>,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    preview: &SchemaEditPreview,
    deadline: Instant,
) -> Outcome {
    let started = Instant::now();
    let outcome = match rebuild(baseline, draft, preview) {
        Ok(built) => {
            apply(
                &mut socket,
                permit,
                &mut cancellation,
                baseline,
                draft,
                &built,
                deadline,
            )
            .await
        }
        Err(reason) => Outcome::NotDispatched { reason },
    };
    let outcome = match outcome {
        Outcome::Applied { .. } => Outcome::Applied {
            runtime_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        },
        other => other,
    };
    let clean = matches!(
        outcome,
        Outcome::Applied { .. } | Outcome::RolledBack { .. }
    );
    socket.cleanup(!clean, Instant::now() + CLEANUP_GRACE).await;
    outcome
}

async fn apply(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    built: &Built,
    deadline: Instant,
) -> Outcome {
    let schema = baseline.table.schema.as_str();
    let atomic = built.atomic_len();
    let mut committed = 0usize;
    if atomic > 0 {
        if let Err(outcome) = atomic_group(
            socket,
            permit,
            cancellation,
            baseline,
            draft,
            built,
            deadline,
        )
        .await
        {
            return outcome;
        }
        committed = atomic;
    } else {
        // Nothing to lock for: a concurrent build must not take the strong
        // lock. The drift check still runs before anything is sent.
        let checked = tokio::select! {
            biased;
            _ = interrupted(permit, cancellation) => Err(Failure::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
            result = socket.drift(schema, &baseline.table.name) => result,
        };
        match checked {
            Ok(Some(token)) if token == baseline.drift => {}
            Ok(_) => {
                return Outcome::NotDispatched {
                    reason: Failure::BaselineChanged,
                }
            }
            Err(reason) => return Outcome::NotDispatched { reason },
        }
    }

    let oid = drift_oid(&baseline.drift);
    for index in atomic..built.statements.len() {
        let stopped = |reason| {
            if committed == 0 && index == 0 {
                Outcome::NotDispatched { reason }
            } else {
                Outcome::Stopped {
                    committed,
                    failed_statement: index,
                    reason,
                }
            }
        };
        // A cancel that arrived during the previous COMMIT or statement is
        // never swallowed by the next dispatch.
        if index > 0 && !permit.readmit() {
            return stopped(Failure::Cancelled);
        }
        if Instant::now() >= deadline {
            return stopped(Failure::Timeout);
        }
        let checked = tokio::select! {
            biased;
            _ = interrupted(permit, cancellation) => Err(Failure::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
            result = socket.relation(schema, &draft.table.name) => result,
        };
        match checked {
            Ok(Some(found)) if Some(found) == oid => {}
            Ok(_) => return stopped(Failure::BaselineChanged),
            Err(reason) => return stopped(reason),
        }
        if !permit.admit_dispatch() {
            return stopped(Failure::Cancelled);
        }
        match standalone(socket, cancellation, &built.statements[index].sql, deadline).await {
            Ok(()) => committed += 1,
            Err((true, reason)) => {
                // A definite server rejection: this statement did not apply,
                // though a concurrent build may have left an invalid index.
                let leftover = built
                    .concurrent_index(index)
                    .and_then(|origin| draft.table.index(origin))
                    .zip(oid);
                let reason = match leftover {
                    Some((created, table)) => {
                        drop_invalid_index(
                            socket,
                            cancellation,
                            schema,
                            &created.name,
                            table,
                            reason,
                        )
                        .await
                    }
                    None => reason,
                };
                return Outcome::Stopped {
                    committed,
                    failed_statement: index,
                    reason,
                };
            }
            Err((false, reason)) => return Outcome::OutcomeUnknown { reason },
        }
    }
    if built.statements.len() > atomic {
        let verified = tokio::time::timeout(VERIFY_TIMEOUT, async {
            socket.execute(BEGIN_VERIFY).await?;
            let result = verify(socket, baseline, draft, &[]).await;
            socket.execute("ROLLBACK").await?;
            result
        })
        .await
        .unwrap_or(Err(Failure::Timeout));
        if let Err(reason) = verified {
            return Outcome::OutcomeUnknown { reason };
        }
    }
    Outcome::Applied { runtime_ms: 0 }
}

/// Re-captures the table in the open transaction and compares it with the
/// draft. `deferred` lists indexes not built yet.
async fn verify(
    socket: &mut impl Transport,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    deferred: &[Origin],
) -> Result<(), Failure> {
    let requests = type_requests(baseline, draft);
    let types: Vec<String> = requests.iter().map(|(_, text)| text.clone()).collect();
    let (actual, canonical) = socket
        .observe(&baseline.table.schema, &draft.table.name, &types)
        .await?;
    if canonical.len() != requests.len() {
        return Err(Failure::VerificationFailed {
            detail: "the server did not resolve every column type".into(),
        });
    }
    let canonical: Vec<_> = requests
        .iter()
        .map(|(origin, _)| *origin)
        .zip(canonical)
        .collect();
    Expectation {
        baseline,
        draft,
        deferred_indexes: deferred,
    }
    .compare(&actual, &canonical)
    .map_err(|detail| Failure::VerificationFailed { detail })
}

/// After a definite failure of `CREATE INDEX CONCURRENTLY`, drops the
/// invalid index it left on `table`, if any, and says so in the reason. When
/// the cleanup cannot be confirmed the reason says the index may remain.
async fn drop_invalid_index(
    socket: &mut impl Transport,
    cancellation: &mut watch::Receiver<u64>,
    schema: &str,
    index: &str,
    table: u32,
    reason: Failure,
) -> Failure {
    let deadline = Instant::now() + INDEX_CLEANUP_TIMEOUT;
    let qualified = format!("{}.{}", quote_double(schema), quote_double(index));
    let remains = |reason: Failure| {
        annotate(
            reason,
            &format!(
                "the failed build may have left the invalid index {qualified}; drop it with DROP INDEX CONCURRENTLY"
            ),
        )
    };
    let found = tokio::select! {
        biased;
        _ = cancellation.changed() => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = socket.invalid_index(schema, index, table) => result,
    };
    match found {
        Ok(false) => return reason,
        Ok(true) => {}
        Err(_) => return remains(reason),
    }
    let sql = format!("DROP INDEX CONCURRENTLY IF EXISTS {qualified}");
    match standalone(socket, cancellation, &sql, deadline).await {
        // A database error carries text; other reasons stay as they are
        // since the table is back to its committed state.
        Ok(()) if matches!(reason, Failure::Database { .. }) => annotate(
            reason,
            &format!("the invalid index {qualified} left by the failed build was dropped"),
        ),
        Ok(()) => reason,
        Err(_) => remains(reason),
    }
}

/// `reason` with `note` appended. Reasons without text become a database
/// failure that keeps the SQLSTATE they stand for.
pub(super) fn annotate(reason: Failure, note: &str) -> Failure {
    match reason {
        Failure::Database { code, message } => Failure::Database {
            code,
            message: format!("{message}; {note}"),
        },
        other => {
            let code = match &other {
                Failure::LockTimeout => Some("55P03".to_owned()),
                Failure::Timeout => Some("57014".to_owned()),
                _ => None,
            };
            Failure::Database {
                code,
                message: format!("{other}; {note}"),
            }
        }
    }
}

/// `Err((definite, reason))`: `definite` when the server answered with an
/// error, so the statement did not apply.
async fn standalone(
    socket: &mut impl Transport,
    cancellation: &mut watch::Receiver<u64>,
    sql: &str,
    deadline: Instant,
) -> Result<(), (bool, Failure)> {
    let cancel = socket.canceller();
    let statement = socket.execute(sql);
    tokio::pin!(statement);
    let interruption = tokio::select! {
        biased;
        result = &mut statement => Ok(result),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        _ = cancellation.changed() => Err(Failure::Cancelled),
    };
    let (result, interrupted_by) = match interruption {
        Ok(result) => (Some(result), None),
        Err(reason) => {
            // A bounded grace, only to learn whether the server answered the
            // cancel with an error or finished the statement.
            let grace = Instant::now() + CLEANUP_GRACE;
            let _ = tokio::time::timeout_at(grace, cancel()).await;
            (
                tokio::time::timeout_at(grace, &mut statement).await.ok(),
                Some(reason),
            )
        }
    };
    match result {
        Some(Ok(())) => Ok(()),
        Some(Err(reason)) if definite(&reason) => Err((true, interrupted_by.unwrap_or(reason))),
        Some(Err(reason)) => Err((false, interrupted_by.unwrap_or(reason))),
        None => Err((false, interrupted_by.unwrap_or(Failure::Cancelled))),
    }
}

/// Failures that come from a server error reply.
fn definite(reason: &Failure) -> bool {
    matches!(
        reason,
        Failure::Database { .. } | Failure::LockTimeout | Failure::Timeout
    )
}

async fn atomic_group(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    built: &Built,
    deadline: Instant,
) -> Result<(), Outcome> {
    let cancel = socket.canceller();
    let mut dispatched = false;
    let result = transaction(
        socket,
        permit,
        cancellation,
        baseline,
        draft,
        built,
        deadline,
        &mut dispatched,
    )
    .await;
    match result {
        Ok(()) => Ok(()),
        Err((true, reason)) => Err(Outcome::OutcomeUnknown { reason }),
        // Only reads and the lock ran; closing the socket ends the
        // transaction.
        Err((false, reason)) if !dispatched => Err(Outcome::NotDispatched { reason }),
        Err((false, reason)) => {
            let grace = Instant::now() + CLEANUP_GRACE;
            if matches!(reason, Failure::Cancelled | Failure::Timeout) {
                // A dropped statement future may still run server-side.
                let _ = tokio::time::timeout_at(grace, cancel()).await;
            }
            match tokio::time::timeout_at(grace, socket.execute("ROLLBACK")).await {
                Ok(Ok(())) => Err(Outcome::RolledBack { reason }),
                _ => Err(Outcome::OutcomeUnknown { reason }),
            }
        }
    }
}

/// `Err((true, _))` means COMMIT was sent and its result is unknown.
#[allow(clippy::too_many_arguments)]
async fn transaction(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancellation: &mut watch::Receiver<u64>,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    built: &Built,
    deadline: Instant,
    dispatched: &mut bool,
) -> Result<(), (bool, Failure)> {
    let schema = baseline.table.schema.as_str();
    let preparing = async {
        socket.execute(BEGIN).await?;
        socket.execute(LOCK_TIMEOUT_LOCAL).await?;
        let lock = format!(
            "LOCK TABLE ONLY {}.{} IN ACCESS EXCLUSIVE MODE",
            quote_double(schema),
            quote_double(&baseline.table.name)
        );
        socket
            .execute(&lock)
            .await
            .map_err(|failure| match failure {
                // The table was dropped or renamed since capture.
                Failure::Database {
                    code: Some(code), ..
                } if code == "42P01" => Failure::BaselineChanged,
                other => other,
            })?;
        match socket.drift(schema, &baseline.table.name).await? {
            Some(token) if token == baseline.drift => {}
            _ => return Err(Failure::BaselineChanged),
        }
        // The flag precedes polling dispatch, so a dropped or failed reply
        // never reports NotDispatched for SQL that might have reached the
        // server.
        if !permit.check_preparing() {
            return Err(Failure::Cancelled);
        }
        *dispatched = true;
        for statement in &built.statements[..built.atomic_len()] {
            socket.execute(&statement.sql).await?;
        }
        verify(socket, baseline, draft, &built.deferred_indexes).await
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
        _ = cancellation.changed() => match tokio::time::timeout_at(Instant::now() + CLEANUP_GRACE, &mut commit).await {
            Ok(result) => result.map_err(|failure| (true, failure)),
            Err(_) => Err((true, Failure::Cancelled)),
        }
    }
}
