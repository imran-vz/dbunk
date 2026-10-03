//! Dedicated sequence inspection and writes.
//!
//! Inspection runs in a READ ONLY transaction and reads `last_value`/`is_called`
//! from the relation; it never calls nextval. nextval/setval are bound to the
//! observed OID by a guarded single statement and have a dispatch fence.
//! RESTART uses a transaction with guard checks before and after ALTER SEQUENCE
//! and a COMMIT fence. Nothing is retried after dispatch.
#[cfg(test)]
mod tests;
use super::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
    native_catalog::CatalogError,
    objects::{PgObjectKind, PgObjectRef},
};
use crate::backend::sequences::{
    restart_sql, SequenceDataType, SequenceDefinition, SequenceFailure as Failure,
    SequenceIntent as Intent, SequenceObservation as Observation, SequenceOutcome as Outcome,
    SequencePreview as Preview, SequenceTarget as Target, SequenceValue, WritePermit, ADVANCE_SQL,
    GUARD_SQL, LOCK_TIMEOUT_SQL, RESTART_BEGIN_SQL, SET_SQL,
};
use futures_util::future::BoxFuture;
use std::time::Duration;
use tokio::{sync::watch, time::Instant};
use tokio_postgres::{types::ToSql, Client};

const CLEANUP_GRACE: Duration = Duration::from_secs(1);
const INSPECT_DEADLINE: Duration = Duration::from_secs(30);

// Every text column is length-guarded before it reaches the wire. The query
// addresses exactly one pg_class sequence row by schema and name.
const OBSERVE_SQL: &str = "SELECT d.oid AS database_oid,
  CASE WHEN octet_length(d.datname::text) <= 63 THEN d.datname::text END AS database,
  n.oid AS namespace_oid,
  CASE WHEN octet_length(n.nspname::text) <= 63 THEN n.nspname::text END AS schema,
  c.oid AS sequence_oid,
  CASE WHEN octet_length(c.relname::text) <= 63 THEN c.relname::text END AS name,
  pg_catalog.format_type(s.seqtypid, NULL) AS data_type,
  s.seqstart AS start_value, s.seqincrement AS increment_by,
  s.seqmin AS min_value, s.seqmax AS max_value, s.seqcache AS cache_size, s.seqcycle AS cycle,
  pg_catalog.has_sequence_privilege(c.oid, 'SELECT') AS readable,
  owned.owned_by, coalesce(owned.identity, false) AS identity
FROM pg_catalog.pg_class c
  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
  JOIN pg_catalog.pg_sequence s ON s.seqrelid = c.oid
  JOIN pg_catalog.pg_database d ON d.datname = pg_catalog.current_database()
  LEFT JOIN LATERAL (
    SELECT CASE WHEN octet_length(x.v) <= 200 THEN x.v END AS owned_by,
           dep.deptype = 'i' AS identity
    FROM pg_catalog.pg_depend dep
      JOIN pg_catalog.pg_class t ON t.oid = dep.refobjid
      JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace
      JOIN pg_catalog.pg_attribute a ON a.attrelid = t.oid AND a.attnum = dep.refobjsubid
      CROSS JOIN LATERAL (SELECT pg_catalog.quote_ident(tn.nspname::text) || '.' ||
        pg_catalog.quote_ident(t.relname::text) || '.' ||
        pg_catalog.quote_ident(a.attname::text) AS v) x
    WHERE dep.classid = 'pg_catalog.pg_class'::pg_catalog.regclass
      AND dep.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
      AND dep.objid = c.oid AND dep.deptype IN ('a', 'i')
    ORDER BY dep.deptype DESC
    LIMIT 1
  ) owned ON true
WHERE n.nspname::text = $1 AND c.relname::text = $2 AND c.relkind = 'S'";

pub(crate) struct Execution {
    pub outcome: Outcome,
    pub runtime_ms: u64,
}
impl Execution {
    pub(crate) fn not_dispatched(reason: Failure) -> Self {
        Self {
            outcome: Outcome::NotDispatched { reason },
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

/// A non-transactional nextval/setval error reply proves no effect only for
/// errors raised before the value changes: data limits/ranges (22), read-only
/// transaction (25), privilege/undefined objects (42) and lock/prerequisite
/// state (55). Cancellation, shutdown and anything else stay unknown.
fn rejected_before_effect(reason: &Failure) -> bool {
    matches!(reason, Failure::Database { code: Some(code) }
        if ["22", "25", "42", "55"].iter().any(|class| code.starts_with(class)))
}

/// Exact guard parameters in `$n` order matching the backend preview.
struct Parameters {
    sequence_oid: u32,
    namespace_oid: u32,
    schema: String,
    name: String,
    database_oid: u32,
    data_type: &'static str,
    start: i64,
    increment: i64,
    min_value: i64,
    max_value: i64,
    cache: i64,
    cycle: bool,
    set: Option<(i64, bool)>,
}
impl Parameters {
    fn new(observation: &Observation, set: Option<(i64, bool)>) -> Self {
        let t = &observation.target;
        let d = &observation.definition;
        Self {
            sequence_oid: t.sequence_oid(),
            namespace_oid: t.namespace_oid(),
            schema: t.schema().to_owned(),
            name: t.name().to_owned(),
            database_oid: t.database_oid(),
            data_type: d.data_type.sql(),
            start: d.start,
            increment: d.increment,
            min_value: d.min_value,
            max_value: d.max_value,
            cache: d.cache,
            cycle: d.cycle,
            set,
        }
    }
    fn values(&self) -> Vec<&(dyn ToSql + Sync)> {
        let mut values: Vec<&(dyn ToSql + Sync)> = vec![
            &self.sequence_oid,
            &self.namespace_oid,
            &self.schema,
            &self.name,
            &self.database_oid,
            &self.data_type,
            &self.start,
            &self.increment,
            &self.min_value,
            &self.max_value,
            &self.cache,
            &self.cycle,
        ];
        if let Some((value, is_called)) = &self.set {
            values.push(value);
            values.push(is_called);
        }
        values
    }
}

trait Transport: Sized {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>>;
    /// Runs a guarded statement; `None` means the guard matched no row.
    fn guarded<'a>(
        &'a mut self,
        sql: &'static str,
        parameters: &'a Parameters,
    ) -> BoxFuture<'a, Result<Option<i64>, Failure>>;
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
    fn guarded<'a>(
        &'a mut self,
        sql: &'static str,
        parameters: &'a Parameters,
    ) -> BoxFuture<'a, Result<Option<i64>, Failure>> {
        Box::pin(async move {
            let row = self
                .connection
                .client
                .query_opt(sql, &parameters.values())
                .await
                .map_err(database_error)?;
            row.map(|row| row.try_get::<_, i64>("value").map_err(database_error))
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
    intent: Intent,
    observation: Observation,
    preview: Preview,
) -> Execution {
    let deadline = Instant::now() + Duration::from_millis(u64::from(preview.operation_timeout_ms));
    let connected = tokio::select! {
        biased;
        _ = interrupted(&permit, &mut cancelled) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(Failure::Timeout),
        result = dedicated::connect_tracked(&spec, NoticeSink::Ignore, Some(&drivers)) => result.map_err(|_| Failure::Connection),
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
                intent,
                &observation,
                deadline,
            )
            .await
        }
        Err(reason) => {
            join(&drivers, deadline.min(Instant::now() + CLEANUP_GRACE)).await;
            Execution::not_dispatched(reason)
        }
    }
}
async fn run(
    mut socket: impl Transport,
    permit: &WritePermit,
    mut cancelled: watch::Receiver<u64>,
    intent: Intent,
    observation: &Observation,
    deadline: Instant,
) -> Execution {
    let start = Instant::now();
    let outcome = operation(
        &mut socket,
        permit,
        &mut cancelled,
        intent,
        observation,
        deadline,
    )
    .await;
    // Closing the socket rolls back any open RESTART transaction.
    socket
        .cleanup(
            !matches!(outcome, Outcome::Completed { .. }),
            deadline.min(Instant::now() + CLEANUP_GRACE),
        )
        .await;
    Execution {
        outcome,
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
    intent: Intent,
    observation: &Observation,
    deadline: Instant,
) -> Outcome {
    let (sql, set) = match intent {
        Intent::Advance => (ADVANCE_SQL, None),
        Intent::Set { value, is_called } => (SET_SQL, Some((value, is_called))),
        Intent::Restart { with } => {
            return restart(socket, permit, cancelled, with, observation, deadline).await;
        }
    };
    let parameters = Parameters::new(observation, set);
    if let Err(reason) = preparing(
        socket.execute(LOCK_TIMEOUT_SQL),
        permit,
        cancelled,
        deadline,
    )
    .await
    {
        return Outcome::NotDispatched { reason };
    }
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
    match settle(socket.guarded(sql, &parameters), cancelled, deadline).await {
        Ok(Some(value)) => Outcome::Completed {
            returned: Some(value),
        },
        // The guard filtered the only candidate row, so the call never ran.
        Ok(None) => Outcome::TargetChanged,
        Err(reason) if rejected_before_effect(&reason) => Outcome::Rejected { reason },
        Err(reason) => Outcome::OutcomeUnknown { reason },
    }
}
async fn restart(
    socket: &mut impl Transport,
    permit: &WritePermit,
    cancelled: &mut watch::Receiver<u64>,
    with: Option<i64>,
    observation: &Observation,
    deadline: Instant,
) -> Outcome {
    let parameters = Parameters::new(observation, None);
    if let Err(reason) = preparing(
        socket.execute(RESTART_BEGIN_SQL),
        permit,
        cancelled,
        deadline,
    )
    .await
    {
        return Outcome::NotDispatched { reason };
    }
    match preparing(
        socket.guarded(GUARD_SQL, &parameters),
        permit,
        cancelled,
        deadline,
    )
    .await
    {
        Ok(Some(_)) => (),
        Ok(None) => return Outcome::TargetChanged,
        Err(reason) => return Outcome::NotDispatched { reason },
    }
    let alter = restart_sql(&observation.target, with);
    if let Err(reason) = preparing(socket.execute(&alter), permit, cancelled, deadline).await {
        return Outcome::RolledBack { reason };
    }
    // ALTER SEQUENCE now holds a lock that blocks renames of the altered
    // relation; the name must still resolve to the observed OID/definition.
    match preparing(
        socket.guarded(GUARD_SQL, &parameters),
        permit,
        cancelled,
        deadline,
    )
    .await
    {
        Ok(Some(_)) => (),
        Ok(None) => return Outcome::TargetChanged,
        Err(reason) => return Outcome::RolledBack { reason },
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
        Ok(()) => Outcome::Completed { returned: None },
        Err(reason) => Outcome::OutcomeUnknown { reason },
    }
}
/// Once admitted, an available terminal reply wins. Cancellation gives this
/// same future one short settlement window, never a fresh execution.
async fn settle<T>(
    future: impl std::future::Future<Output = Result<T, Failure>>,
    cancelled: &mut watch::Receiver<u64>,
    deadline: Instant,
) -> Result<T, Failure> {
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

struct Catalog {
    target: Target,
    definition: SequenceDefinition,
    readable: bool,
    owned_by: Option<String>,
    identity: bool,
}
async fn load(client: &Client, schema: &str, name: &str) -> Result<Option<Catalog>, CatalogError> {
    let row = client
        .query_opt(OBSERVE_SQL, &[&schema, &name])
        .await
        .map_err(|_| CatalogError::Database)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let invalid = |_| CatalogError::InvalidResponse;
    let data_type: String = row.try_get("data_type").map_err(invalid)?;
    let catalog = Catalog {
        target: Target {
            database_oid: row.try_get("database_oid").map_err(invalid)?,
            database: row.try_get("database").map_err(invalid)?,
            namespace_oid: row.try_get("namespace_oid").map_err(invalid)?,
            schema: row.try_get("schema").map_err(invalid)?,
            sequence_oid: row.try_get("sequence_oid").map_err(invalid)?,
            name: row.try_get("name").map_err(invalid)?,
        },
        definition: SequenceDefinition {
            data_type: SequenceDataType::parse(&data_type).ok_or(CatalogError::InvalidResponse)?,
            start: row.try_get("start_value").map_err(invalid)?,
            increment: row.try_get("increment_by").map_err(invalid)?,
            min_value: row.try_get("min_value").map_err(invalid)?,
            max_value: row.try_get("max_value").map_err(invalid)?,
            cache: row.try_get("cache_size").map_err(invalid)?,
            cycle: row.try_get("cycle").map_err(invalid)?,
        },
        readable: row.try_get("readable").map_err(invalid)?,
        owned_by: row.try_get("owned_by").map_err(invalid)?,
        identity: row.try_get("identity").map_err(invalid)?,
    };
    if !catalog.target.valid() || !catalog.definition.valid() {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(Some(catalog))
}
/// Read-only observation. The value is read from the relation itself; a second
/// catalog read after that SELECT (which holds AccessShareLock on the resolved
/// relation) proves the name still maps to the observed OID and definition.
async fn inspect(client: &Client, reference: &PgObjectRef) -> Result<Observation, CatalogError> {
    client
        .batch_execute("BEGIN READ ONLY; SET LOCAL lock_timeout = '10s'")
        .await
        .map_err(|_| CatalogError::Database)?;
    let schema = reference.schema.as_deref().unwrap_or("");
    let first = load(client, schema, &reference.name)
        .await?
        .ok_or(CatalogError::ObjectNotFound)?;
    let value = if first.readable {
        let row = client
            .query_one(
                &format!(
                    "SELECT last_value, is_called FROM {}",
                    first.target.qualified()
                ),
                &[],
            )
            .await
            .map_err(|_| CatalogError::Database)?;
        let value = SequenceValue::Read {
            last_value: row
                .try_get("last_value")
                .map_err(|_| CatalogError::InvalidResponse)?,
            is_called: row
                .try_get("is_called")
                .map_err(|_| CatalogError::InvalidResponse)?,
        };
        let second = load(client, schema, &reference.name)
            .await?
            .ok_or(CatalogError::ObjectNotFound)?;
        if second.target != first.target || second.definition != first.definition {
            return Err(CatalogError::ObjectNotFound);
        }
        value
    } else {
        SequenceValue::NotReadable
    };
    let _ = client.batch_execute("ROLLBACK").await;
    let observation = Observation {
        target: first.target,
        definition: first.definition,
        value,
        owned_by: first.owned_by,
        identity: first.identity,
    };
    if !observation.valid() {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(observation)
}
pub(crate) async fn observe(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    mut cancelled: watch::Receiver<u64>,
    reference: &PgObjectRef,
) -> Result<Observation, CatalogError> {
    if reference.kind != PgObjectKind::Sequence {
        return Err(CatalogError::UnsupportedObjectKind);
    }
    let deadline = Instant::now() + INSPECT_DEADLINE;
    let connected = tokio::select! {
        biased;
        _ = cancelled.changed() => Err(CatalogError::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
        result = dedicated::connect_tracked(spec, NoticeSink::Ignore, Some(drivers)) => result.map_err(|_| CatalogError::Connection),
    };
    let (result, cleanup) = match connected {
        Ok(connection) => {
            let result = tokio::select! {
                biased;
                _ = cancelled.changed() => Err(CatalogError::Cancelled),
                _ = tokio::time::sleep_until(deadline) => Err(CatalogError::Timeout),
                result = inspect(&connection.client, reference) => result,
            };
            let cleanup = deadline.min(Instant::now() + CLEANUP_GRACE);
            if result.is_err() {
                let _ = tokio::time::timeout_at(
                    cleanup,
                    dedicated::cancel(connection.cancel.clone(), connection.tls.clone()),
                )
                .await;
            }
            if tokio::time::timeout_at(cleanup, connection.close())
                .await
                .is_err()
            {
                drivers.abort_all();
            }
            (result, cleanup)
        }
        Err(error) => (Err(error), deadline.min(Instant::now() + CLEANUP_GRACE)),
    };
    join(drivers, cleanup).await;
    if cancelled.has_changed().unwrap_or(true) {
        return Err(CatalogError::Cancelled);
    }
    result
}
