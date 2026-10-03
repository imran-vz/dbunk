//! Streamed bounded pg_catalog administration reads on the shared owned socket.
use super::*;
use serde::Serialize;

mod queries;
pub const MAX_ADMIN_BYTES: usize = 1024 * 1024;
pub const MAX_ADMIN_ROWS: usize = 200;
pub const MAX_ADMIN_BLOCKERS: usize = 64;
pub const MAX_ADMIN_QUERY_CHARS: usize = 500;
pub const MAX_ADMIN_TEXT_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "state", content = "value", rename_all = "camelCase")]
pub enum AdminMetric<T> {
    Value(T),
    Null,
    Restricted,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminStats {
    pub database_size_bytes: AdminMetric<i64>,
    pub cache_hit_ratio: AdminMetric<f64>,
    pub active_sessions: AdminMetric<i64>,
    pub idle_in_transaction: AdminMetric<i64>,
    /// Cluster-wide, unlike the database-scoped activity counters above.
    pub blocked_locks: AdminMetric<i64>,
}
impl AdminStats {
    fn unavailable() -> Self {
        Self {
            database_size_bytes: AdminMetric::Unavailable,
            cache_hit_ratio: AdminMetric::Unavailable,
            active_sessions: AdminMetric::Unavailable,
            idle_in_transaction: AdminMetric::Unavailable,
            blocked_locks: AdminMetric::Unavailable,
        }
    }
    fn restricted() -> Self {
        Self {
            database_size_bytes: AdminMetric::Restricted,
            cache_hit_ratio: AdminMetric::Restricted,
            active_sessions: AdminMetric::Restricted,
            idle_in_transaction: AdminMetric::Restricted,
            blocked_locks: AdminMetric::Restricted,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminSession {
    pub pid: i32,
    pub user: Option<String>,
    pub database: Option<String>,
    pub application_name: Option<String>,
    pub client_addr: Option<String>,
    pub state: Option<String>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub query_age_seconds: Option<i64>,
    pub transaction_age_seconds: Option<i64>,
    pub query: Option<String>,
    pub query_clipped: bool,
    pub details_restricted: bool,
    /// UTC microseconds, nullable when PostgreSQL withholds identity details.
    pub backend_start: Option<String>,
    pub query_start: Option<String>,
    pub xact_start: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminLock {
    /// Prepared transactions can hold a lock without a live PID.
    pub pid: Option<i32>,
    pub lock_type: String,
    pub relation: Option<String>,
    pub mode: String,
    pub granted: bool,
    pub blocked_by: Vec<i32>,
    pub blocked_by_clipped: bool,
    pub blocked_by_unavailable: bool,
    pub query: Option<String>,
    pub query_clipped: bool,
    pub details_restricted: bool,
    pub backend_start: Option<String>,
    pub query_start: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminSnapshot {
    pub database: String,
    pub reader_pid: i32,
    pub activity_restricted: bool,
    pub sessions: Vec<AdminSession>,
    pub locks: Vec<AdminLock>,
    pub pending_transactions: Vec<AdminSession>,
    pub stats: AdminStats,
    pub collected_start: String,
    pub collected_end: String,
    pub scope_note: String,
    pub sessions_truncated: bool,
    pub locks_truncated: bool,
    pub pending_transactions_truncated: bool,
}
impl std::fmt::Debug for AdminSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminSnapshot")
            .field("sessions", &self.sessions.len())
            .field("locks", &self.locks.len())
            .field("pending_transactions", &self.pending_transactions.len())
            .finish_non_exhaustive()
    }
}

pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
) -> Result<AdminSnapshot, CatalogError> {
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        |client, timeout| Box::pin(load(client, timeout)),
    )
    .await
}

async fn load(client: &Client, timeout: Option<u32>) -> Result<AdminSnapshot, CatalogError> {
    let mut builder = Builder::new();
    begin_snapshot(client, timeout).await?;
    let identity = client.query_one("SELECT pg_catalog.current_database()::text AS database, pg_catalog.pg_backend_pid() AS pid, EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE (datname=pg_catalog.current_database() OR datname IS NULL) AND query='<insufficient privilege>') AS activity_restricted", &[])
        .await.map_err(|_| CatalogError::Database)?;
    builder.snapshot.database =
        optional(&identity, "database")?.ok_or(CatalogError::InvalidResponse)?;
    builder.snapshot.reader_pid = get(&identity, "pid")?;
    builder.snapshot.activity_restricted = get(&identity, "activity_restricted")?;
    for pending in [false, true] {
        let limit = MAX_ADMIN_ROWS as i64 + 1;
        let rows = client
            .query_raw(queries::SESSIONS, [&pending as &(dyn ToSql + Sync), &limit])
            .await
            .map_err(|_| CatalogError::Database)?;
        tokio::pin!(rows);
        while let Some(row) = rows.try_next().await.map_err(|_| CatalogError::Database)? {
            if !builder.session(session(&row)?, pending)? {
                break;
            }
        }
    }
    let limit = MAX_ADMIN_ROWS as i64 + 1;
    let rows = client
        .query_raw(queries::LOCKS, [&limit as &(dyn ToSql + Sync)])
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(rows);
    while let Some(row) = rows.try_next().await.map_err(|_| CatalogError::Database)? {
        if !builder.lock(lock(&row)?)? {
            break;
        }
    }
    // A permission failure in these optional counters must not become zero or
    // discard the already readable activity rows. Other failures remain errors.
    client
        .batch_execute("SAVEPOINT admin_metrics")
        .await
        .map_err(|_| CatalogError::Database)?;
    builder.snapshot.stats = match client.query_opt(queries::STATS, &[]).await {
        Ok(Some(row)) => stats(&row)?,
        Ok(None) => AdminStats::unavailable(),
        Err(error) if error.code().is_some_and(|code| code.code() == "42501") => {
            client
                .batch_execute("ROLLBACK TO SAVEPOINT admin_metrics")
                .await
                .map_err(|_| CatalogError::Database)?;
            AdminStats::restricted()
        }
        Err(_) => return Err(CatalogError::Database),
    };
    client
        .batch_execute("RELEASE SAVEPOINT admin_metrics; COMMIT")
        .await
        .map_err(|_| CatalogError::Database)?;
    builder.finish()
}

fn get<'a, T: tokio_postgres::types::FromSql<'a>>(
    row: &'a Row,
    key: &str,
) -> Result<T, CatalogError> {
    row.try_get(key).map_err(|_| CatalogError::InvalidResponse)
}
fn optional(row: &Row, key: &str) -> Result<Option<String>, CatalogError> {
    let value: Option<&str> = get(row, key)?;
    if value.is_some_and(|value| value.len() > MAX_ADMIN_TEXT_BYTES) {
        return Err(CatalogError::AdminLimit);
    }
    Ok(value.map(str::to_owned))
}
fn checked(row: &Row) -> Result<(), CatalogError> {
    if get::<bool>(row, "too_large")? {
        Err(CatalogError::AdminLimit)
    } else {
        Ok(())
    }
}
fn query_text(
    value: Option<&str>,
    restricted: bool,
    clipped: bool,
) -> Result<Option<String>, CatalogError> {
    let count = value.map(|value| value.chars().count());
    if count.is_some_and(|count| count > MAX_ADMIN_QUERY_CHARS) {
        return Err(CatalogError::AdminLimit);
    }
    if clipped && count != Some(MAX_ADMIN_QUERY_CHARS) {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(if restricted {
        None
    } else {
        value.map(str::to_owned)
    })
}
fn session(row: &Row) -> Result<AdminSession, CatalogError> {
    checked(row)?;
    let restricted = get(row, "details_restricted")?;
    Ok(AdminSession {
        pid: get(row, "pid")?,
        user: optional(row, "usename")?,
        database: optional(row, "datname")?,
        application_name: optional(row, "application_name")?,
        client_addr: optional(row, "client_addr")?,
        state: optional(row, "state")?,
        wait_event_type: optional(row, "wait_event_type")?,
        wait_event: optional(row, "wait_event")?,
        query_age_seconds: get(row, "query_age_seconds")?,
        transaction_age_seconds: get(row, "transaction_age_seconds")?,
        query: query_text(get(row, "query")?, restricted, get(row, "query_clipped")?)?,
        query_clipped: get(row, "query_clipped")?,
        details_restricted: restricted,
        backend_start: optional(row, "backend_start")?,
        query_start: optional(row, "query_start")?,
        xact_start: optional(row, "xact_start")?,
    })
}
fn lock(row: &Row) -> Result<AdminLock, CatalogError> {
    checked(row)?;
    let blocked_by: Option<Vec<i32>> = get(row, "blocked_by")?;
    validate_blockers(blocked_by.as_deref(), get(row, "blocked_by_clipped")?)?;
    let restricted = get(row, "details_restricted")?;
    Ok(AdminLock {
        pid: get(row, "pid")?,
        lock_type: optional(row, "locktype")?.ok_or(CatalogError::InvalidResponse)?,
        relation: optional(row, "relation")?,
        mode: optional(row, "mode")?.ok_or(CatalogError::InvalidResponse)?,
        granted: get(row, "granted")?,
        blocked_by_unavailable: blocked_by.is_none(),
        blocked_by: blocked_by.unwrap_or_default(),
        blocked_by_clipped: get(row, "blocked_by_clipped")?,
        query: query_text(get(row, "query")?, restricted, get(row, "query_clipped")?)?,
        query_clipped: get(row, "query_clipped")?,
        details_restricted: restricted,
        backend_start: optional(row, "backend_start")?,
        query_start: optional(row, "query_start")?,
    })
}
fn validate_blockers(ids: Option<&[i32]>, clipped: bool) -> Result<(), CatalogError> {
    if ids.is_some_and(|ids| ids.len() > MAX_ADMIN_BLOCKERS || ids.iter().any(|pid| *pid < 0))
        || (clipped && ids.map(<[i32]>::len) != Some(MAX_ADMIN_BLOCKERS))
    {
        return Err(CatalogError::InvalidResponse);
    }
    Ok(())
}
fn counter(value: Option<i64>, restricted: bool) -> Result<AdminMetric<i64>, CatalogError> {
    if restricted {
        return Ok(AdminMetric::Restricted);
    }
    match value {
        Some(value) if value < 0 => Err(CatalogError::InvalidResponse),
        Some(value) => Ok(AdminMetric::Value(value)),
        None => Ok(AdminMetric::Null),
    }
}
fn ratio(value: Option<f64>) -> Result<AdminMetric<f64>, CatalogError> {
    match value {
        Some(value) if !value.is_finite() || !(0.0..=1.0).contains(&value) => {
            Err(CatalogError::InvalidResponse)
        }
        Some(value) => Ok(AdminMetric::Value(value)),
        None => Ok(AdminMetric::Null),
    }
}
fn stats(row: &Row) -> Result<AdminStats, CatalogError> {
    let restricted: bool = get(row, "activity_restricted")?;
    Ok(AdminStats {
        database_size_bytes: counter(get(row, "database_size_bytes")?, false)?,
        cache_hit_ratio: ratio(get(row, "cache_hit_ratio")?)?,
        active_sessions: counter(get(row, "active_sessions")?, restricted)?,
        idle_in_transaction: counter(get(row, "idle_in_transaction")?, restricted)?,
        blocked_locks: counter(get(row, "blocked_locks")?, false)?,
    })
}

struct Builder {
    snapshot: AdminSnapshot,
    charged: usize,
}
impl Builder {
    fn new() -> Self {
        Self { charged: 4096, snapshot: AdminSnapshot { database: String::new(), reader_pid: 0, activity_restricted: false, sessions: vec![], locks: vec![], pending_transactions: vec![],
            stats: AdminStats::unavailable(), collected_start: chrono::Utc::now().to_rfc3339(), collected_end: String::new(),
            scope_note: "Sessions and transactions: current database and background processes. Locks: current database and unattributed holders. Blocked-lock metric: cluster-wide. Statistics are collected over an interval, not atomically. Restricted activity details can hide pending transactions.".into(),
            sessions_truncated: false, locks_truncated: false, pending_transactions_truncated: false } }
    }
    fn charge(&mut self, value: &impl Serialize) -> Result<(), CatalogError> {
        let bytes = encoded(value)?;
        if bytes > MAX_ADMIN_BYTES.saturating_sub(self.charged) {
            return Err(CatalogError::AdminLimit);
        }
        self.charged += bytes;
        Ok(())
    }
    fn session(&mut self, session: AdminSession, pending: bool) -> Result<bool, CatalogError> {
        if (if pending {
            self.snapshot.pending_transactions.len()
        } else {
            self.snapshot.sessions.len()
        }) >= MAX_ADMIN_ROWS
        {
            if pending {
                self.snapshot.pending_transactions_truncated = true;
            } else {
                self.snapshot.sessions_truncated = true;
            }
            return Ok(false);
        }
        self.charge(&session)?;
        if pending {
            self.snapshot.pending_transactions.push(session);
        } else {
            self.snapshot.sessions.push(session);
        }
        Ok(true)
    }
    fn lock(&mut self, lock: AdminLock) -> Result<bool, CatalogError> {
        if self.snapshot.locks.len() >= MAX_ADMIN_ROWS {
            self.snapshot.locks_truncated = true;
            return Ok(false);
        }
        self.charge(&lock)?;
        self.snapshot.locks.push(lock);
        Ok(true)
    }
    fn finish(mut self) -> Result<AdminSnapshot, CatalogError> {
        self.snapshot.collected_end = chrono::Utc::now().to_rfc3339();
        encoded(&self.snapshot)?;
        Ok(self.snapshot)
    }
}
fn encoded(value: &impl Serialize) -> Result<usize, CatalogError> {
    struct Count(usize);
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_ADMIN_BYTES.saturating_sub(self.0) {
                return Err(io::Error::other("admin byte limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    serde_json::to_writer(&mut count, value).map_err(|_| CatalogError::AdminLimit)?;
    Ok(count.0)
}

#[cfg(test)]
mod tests;
