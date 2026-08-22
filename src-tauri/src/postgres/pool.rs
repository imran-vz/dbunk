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

/// Per-connection pool cache. Each pool allows up to 5 concurrent connections
/// (enough for health checks, schema loads, and queries without exhausting
/// PG's connection slots). Idle connections are reaped after 5 minutes.
static POOL_CACHE: Lazy<Mutex<PoolCache>> = Lazy::new(|| Mutex::new(PoolCache::default()));

#[derive(Default)]
struct PoolCache {
    pools: HashMap<String, CachedPool>,
    /// Invalidates builds that were in flight when `drop_pool` ran. Entries
    /// intentionally outlive pools because a build may finish much later.
    generations: HashMap<String, u64>,
}

struct CachedPool {
    config: PoolConfig,
    pool: PgPool,
}

/// Only settings that affect pool creation or a socket's session defaults.
/// Equality is deliberately explicit rather than based on the connection ID:
/// callers can hold different snapshots of the same stored connection.
#[derive(Clone, PartialEq, Eq)]
struct PoolConfig {
    host: String,
    port: u16,
    database: String,
    user: String,
    password: String,
    ssl: bool,
    read_only: bool,
    statement_timeout_ms: Option<u32>,
    idle_in_transaction_timeout_ms: Option<u32>,
    connect_timeout_ms: Option<u32>,
    keepalive_seconds: Option<u32>,
    default_search_path: Option<Vec<String>>,
    default_role: Option<String>,
}

impl PoolConfig {
    fn from_connection(connection: &crate::PgStoredConnection) -> Self {
        let options = connection.driver_options.clone().unwrap_or_default();
        Self {
            host: connection.host.clone(),
            port: if connection.port == 0 {
                5432
            } else {
                connection.port
            },
            database: connection.database.clone(),
            user: connection.user.clone(),
            password: connection.password.clone(),
            ssl: connection.ssl,
            read_only: connection.read_only,
            statement_timeout_ms: options.statement_timeout_ms,
            idle_in_transaction_timeout_ms: options.idle_in_transaction_timeout_ms,
            connect_timeout_ms: options.connect_timeout_ms,
            keepalive_seconds: options.keepalive_seconds,
            default_search_path: options.default_search_path,
            default_role: options.default_role,
        }
    }
}

impl PoolCache {
    /// Returns a matching cached pool or a generation token for a new build.
    /// Observing different settings is itself an invalidation so older builds
    /// cannot populate the cache after this caller starts building.
    fn pool_or_generation(
        &mut self,
        connection_id: &str,
        config: &PoolConfig,
    ) -> (Option<PgPool>, u64, Option<PgPool>) {
        if let Some(cached) = self.pools.get(connection_id) {
            if cached.config == *config {
                return (
                    Some(cached.pool.clone()),
                    self.generation(connection_id),
                    None,
                );
            }
        }

        let stale_pool = self.pools.remove(connection_id).map(|cached| cached.pool);
        if stale_pool.is_some() {
            self.advance_generation(connection_id);
        }
        (None, self.generation(connection_id), stale_pool)
    }

    /// Publishes a completed build only if no invalidation occurred while it
    /// was awaiting I/O. A caller never receives an entry for another config.
    fn finish_build(
        &mut self,
        connection_id: &str,
        generation: u64,
        config: PoolConfig,
        pool: PgPool,
    ) -> (PgPool, Option<PgPool>) {
        if self.generation(connection_id) != generation {
            return (pool, None);
        }

        if let Some(existing) = self.pools.get(connection_id) {
            if existing.config == config {
                return (existing.pool.clone(), Some(pool));
            }

            // Concurrent callers can carry different snapshots for the same
            // ID. There is no safe way to infer which snapshot is newer, so
            // use this caller's pool without replacing or reusing the cache.
            return (pool, None);
        }

        self.pools.insert(
            connection_id.to_string(),
            CachedPool {
                config,
                pool: pool.clone(),
            },
        );
        (pool, None)
    }

    fn invalidate(&mut self, connection_id: &str) -> Option<PgPool> {
        self.advance_generation(connection_id);
        self.pools.remove(connection_id).map(|cached| cached.pool)
    }

    fn generation(&self, connection_id: &str) -> u64 {
        self.generations.get(connection_id).copied().unwrap_or(0)
    }

    fn advance_generation(&mut self, connection_id: &str) {
        let next = self
            .generation(connection_id)
            .checked_add(1)
            .expect("PostgreSQL pool generation exhausted");
        self.generations.insert(connection_id.to_string(), next);
    }
}

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
    let config = PoolConfig::from_connection(pg);

    // Fast path: a pool built from these exact settings already exists. The
    // generation token prevents this build from surviving a concurrent save,
    // disconnect, or config mismatch discovered by another caller.
    let (cached, generation, stale_pool) = {
        let mut cache = POOL_CACHE.lock().expect("pg pool cache poisoned");
        cache.pool_or_generation(&id, &config)
    };
    if let Some(stale_pool) = stale_pool {
        tokio::spawn(async move { stale_pool.close().await });
    }
    if let Some(pool) = cached {
        return Ok(pool);
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

    // Publish only if the config generation is still current. A build that
    // lost an invalidation race remains usable by its original operation but
    // can never become a cache hit for a later operation.
    let (pool, losing_pool) = {
        let mut cache = POOL_CACHE.lock().expect("pg pool cache poisoned");
        cache.finish_build(&id, generation, config, pool)
    };
    if let Some(losing_pool) = losing_pool {
        tokio::spawn(async move { losing_pool.close().await });
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
    if let Some(pool) = cache.invalidate(connection_id) {
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
        sqlx::query("SET default_transaction_read_only = on")
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgDriverOptions, PgStoredConnection, SshTunnelConfig};

    fn connection(read_only: bool) -> PgStoredConnection {
        PgStoredConnection {
            id: "same-id".into(),
            name: "Test".into(),
            database: "postgres".into(),
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            password: String::new(),
            role: String::new(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only,
            last_activity_at: None,
            ssl: false,
            driver_options: Some(PgDriverOptions::default()),
            ssh_tunnel: SshTunnelConfig::default(),
        }
    }

    fn lazy_pool(connection: &PgStoredConnection) -> PgPool {
        PgPoolOptions::new().connect_lazy_with(build_connect_options(connection))
    }

    #[tokio::test]
    async fn same_id_with_read_only_config_never_reuses_writable_pool() {
        let writable = connection(false);
        let read_only = connection(true);
        let writable_config = PoolConfig::from_connection(&writable);
        let read_only_config = PoolConfig::from_connection(&read_only);
        let mut cache = PoolCache::default();

        let (_, writable_generation, _) = cache.pool_or_generation(&writable.id, &writable_config);
        cache.finish_build(
            &writable.id,
            writable_generation,
            writable_config,
            lazy_pool(&writable),
        );

        let (cached, read_only_generation, stale) =
            cache.pool_or_generation(&read_only.id, &read_only_config);
        assert!(cached.is_none());
        assert!(stale.is_some());

        cache.finish_build(
            &read_only.id,
            read_only_generation,
            read_only_config.clone(),
            lazy_pool(&read_only),
        );
        assert!(cache
            .pools
            .get(&read_only.id)
            .is_some_and(|entry| entry.config == read_only_config));
    }

    #[tokio::test]
    async fn invalidation_prevents_an_in_flight_pool_from_entering_cache() {
        let writable = connection(false);
        let read_only = connection(true);
        let writable_config = PoolConfig::from_connection(&writable);
        let read_only_config = PoolConfig::from_connection(&read_only);
        let mut cache = PoolCache::default();

        let (_, stale_generation, _) = cache.pool_or_generation(&writable.id, &writable_config);
        cache.invalidate(&writable.id);
        cache.finish_build(
            &writable.id,
            stale_generation,
            writable_config,
            lazy_pool(&writable),
        );
        assert!(!cache.pools.contains_key(&writable.id));

        let (_, current_generation, _) = cache.pool_or_generation(&read_only.id, &read_only_config);
        cache.finish_build(
            &read_only.id,
            current_generation,
            read_only_config.clone(),
            lazy_pool(&read_only),
        );
        assert!(cache
            .pools
            .get(&read_only.id)
            .is_some_and(|entry| entry.config == read_only_config));
    }
}
