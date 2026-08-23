//! Connection pool cache for PostgreSQL.

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use sqlx::{
    pool::PoolConnection,
    postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions},
    Postgres,
};

use crate::StoredConnection;

/// Per-connection pool cache. Keyed by `connection_id`. Each pool allows
/// up to 5 concurrent connections (enough for health checks, schema loads,
/// and queries without exhausting PG's connection slots). Idle connections
/// are reaped after 5 minutes.
static POOL_CACHE: Lazy<Mutex<HashMap<String, PgPool>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Maximum connections per pool. Desktop app typically only needs a few
/// concurrent operations against the same database.
const MAX_POOL_SIZE: u32 = 5;

/// Idle timeout before a pooled connection is closed.
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Build `PgConnectOptions` from a stored connection record.
fn build_connect_options(connection: &crate::PgStoredConnection) -> PgConnectOptions {
    let port = if connection.port == 0 {
        5432
    } else {
        connection.port
    };
    let ssl_mode = if connection.ssl {
        sqlx::postgres::PgSslMode::Prefer
    } else {
        sqlx::postgres::PgSslMode::Disable
    };
    let mut options = PgConnectOptions::new()
        .host(&connection.host)
        .username(&connection.user)
        .database(&connection.database)
        .port(port)
        .ssl_mode(ssl_mode);

    if !connection.password.is_empty() {
        options = options.password(&connection.password);
    }
    options
}

/// Get or create a `PgPool` for the given connection. Pools are cached
/// per `connection_id` so repeated operations reuse TCP connections.
async fn pool_for(connection: &StoredConnection) -> Result<PgPool, String> {
    let StoredConnection::PostgreSQL(pg) = connection else {
        return Err(
            "postgres::pool_for reached with a non-PostgreSQL connection — dispatch bug"
                .to_string(),
        );
    };

    let id = pg.id.clone();

    // Fast path: pool already exists. Connection saves run through the socket
    // lifecycle fence and evict this entry before the next operation.
    {
        let cache = POOL_CACHE.lock().expect("pg pool cache poisoned");
        if let Some(pool) = cache.get(&id) {
            return Ok(pool.clone());
        }
    }

    // Slow path: build a new pool.
    let options = build_connect_options(pg);
    let driver_options = pg.driver_options.clone();
    let read_only = pg.read_only;
    let connect_timeout = driver_options
        .as_ref()
        .and_then(|opts| opts.connect_timeout_ms)
        .map(|ms| std::time::Duration::from_millis(u64::from(ms)));
    let host_for_err = pg.host.clone();
    let port = if pg.port == 0 { 5432 } else { pg.port };

    let build = PgPoolOptions::new()
        .max_connections(MAX_POOL_SIZE)
        .idle_timeout(IDLE_TIMEOUT)
        .after_connect(move |conn, _meta| {
            let driver_options = driver_options.clone();
            Box::pin(async move {
                // Driver options remain best-effort as a batch. A requested
                // read-only GUC must apply before this socket enters the pool.
                let options = driver_options.clone().unwrap_or_default();
                apply_driver_options_raw(conn, &options, read_only).await?;
                Ok(())
            })
        })
        .connect_with(options);

    // ADR-0013 `connect_timeout_ms`. `PgConnectOptions` has no connect
    // deadline of its own, so the bound is a wrapper around the initial
    // handshake — which is the hang users actually hit (an unreachable
    // host otherwise waits on the OS TCP timeout, minutes on some
    // platforms). Deliberately *not* mapped to sqlx's `acquire_timeout`:
    // that also covers waiting for a free slot on a saturated pool, so a
    // short connect deadline would start failing healthy queries.
    let pool = match connect_timeout {
        Some(limit) => tokio::time::timeout(limit, build)
            .await
            .map_err(|_| {
                format!(
                    "Connection to {host_for_err}:{port} timed out after {} ms. \
                     Raise the connect timeout in the connection's advanced options, \
                     or check that the host is reachable.",
                    limit.as_millis()
                )
            })?
            .map_err(|error| crate::dispatch::friendly_sqlx_error(error, &host_for_err, port))?,
        None => build
            .await
            .map_err(|error| crate::dispatch::friendly_sqlx_error(error, &host_for_err, port))?,
    };

    // Store in cache. If another task raced us, prefer its pool and close ours
    // so we do not leak connections.
    {
        let mut cache = POOL_CACHE.lock().expect("pg pool cache poisoned");
        if let Some(existing) = cache.get(&id) {
            let winner = existing.clone();
            tokio::spawn(async move { pool.close().await });
            return Ok(winner);
        }
        cache.insert(id, pool.clone());
    }

    Ok(pool)
}

/// Acquire a connection from the cached pool.
pub(crate) async fn connect(
    connection: &StoredConnection,
) -> Result<PoolConnection<Postgres>, String> {
    let pool = pool_for(connection).await?;
    pool.acquire().await.map_err(|error| error.to_string())
}

/// Evict a connection's pool from the cache. Called when the connection
/// is saved (credentials/config may have changed) or deleted.
pub fn drop_pool(connection_id: &str) {
    let mut cache = POOL_CACHE.lock().expect("pg pool cache poisoned");
    if let Some(pool) = cache.remove(connection_id) {
        // Close the pool asynchronously — existing checked-out
        // connections finish their work, but no new ones are issued.
        tokio::spawn(async move { pool.close().await });
    }
}

/// ADR-0013: apply the post-connect SET statements that mirror the
/// driver knobs. Returns `sqlx::Error` for use inside the pool's
/// `after_connect` callback.
async fn apply_driver_options_raw(
    conn: &mut PgConnection,
    options: &crate::types::PgDriverOptions,
    read_only: bool,
) -> Result<(), sqlx::Error> {
    for statement in super::options::driver_option_sql(options, false) {
        if sqlx::query(&statement).execute(&mut *conn).await.is_err() {
            // Preserve the original best-effort batch semantics: the first
            // failed driver option stops the remaining optional statements.
            break;
        }
    }
    if read_only {
        sqlx::query(super::options::READ_ONLY_SESSION_SQL)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
