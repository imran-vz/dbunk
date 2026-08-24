//! Connection pool cache for PostgreSQL.

use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use sqlx::{
    pool::PoolConnection,
    postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions},
    Connection as _, Executor as _, Postgres,
};

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
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

/// Build `PgConnectOptions` from the shared connect spec (ADR-0025).
///
/// Two knobs the dedicated driver honours are *not* applied here because
/// SQLx 0.8 has no setter for them: TCP keepalive (`keepalive_seconds`)
/// and a TLS server name distinct from the socket host (so `verify-full`
/// over an SSH tunnel verifies the chain only on this path — disclosed
/// by the diagnosis).
fn build_connect_options(spec: &ResolvedPostgresConnectSpec) -> PgConnectOptions {
    super::tls::prepare_sqlx_environment();
    let mut options = PgConnectOptions::new_without_pgpass()
        .host(&spec.host)
        .username(&spec.user)
        .database(&spec.database)
        .port(spec.port)
        // `Some("")` is intentional: it makes the stored value authoritative
        // and prevents SQLx from consulting `.pgpass` for passwordless records.
        .password(&spec.password);
    options = super::tls::apply_to_pg_options(&spec.tls, &spec.host, &spec.connection_id, options);
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
    let spec = ResolvedPostgresConnectSpec::from_postgres(pg);
    let options = build_connect_options(&spec);
    let driver_options = spec.driver_options.clone();
    let read_only = spec.safety_policy.read_only;
    let connect_timeout = spec.connect_timeout;
    let host_for_err = spec.host.clone();
    let port = spec.port;

    let build = PgPoolOptions::new()
        .max_connections(MAX_POOL_SIZE)
        .idle_timeout(IDLE_TIMEOUT)
        .after_connect(move |conn, _meta| {
            let driver_options = driver_options.clone();
            Box::pin(async move {
                // Driver options remain best-effort as a batch. A requested
                // read-only GUC must apply before this socket enters the pool.
                apply_driver_options_raw(conn, &driver_options, read_only).await?;
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

/// One-shot PostgreSQL probe for the legacy Test Connection command.
/// It deliberately bypasses `POOL_CACHE`: unsaved form values can reuse a
/// stored connection id, and neither those credentials nor an ephemeral
/// tunnel endpoint may survive the probe.
pub(crate) async fn ping_once(connection: &StoredConnection) -> Result<u64, String> {
    let StoredConnection::PostgreSQL(pg) = connection else {
        return Err("PostgreSQL one-shot probe received a different engine".to_string());
    };
    let spec = ResolvedPostgresConnectSpec::from_postgres(pg);
    let options = build_connect_options(&spec);
    let host_for_error = spec.host.clone();
    let port = spec.port;
    let started = std::time::Instant::now();
    let connect = PgConnection::connect_with(&options);
    let mut connection = match spec.connect_timeout {
        Some(limit) => tokio::time::timeout(limit, connect)
            .await
            .map_err(|_| {
                format!(
                    "Connection to {host_for_error}:{port} timed out after {} ms. \
                     Raise the connect timeout in the connection's advanced options, \
                     or check that the host is reachable.",
                    limit.as_millis()
                )
            })?
            .map_err(|error| crate::dispatch::friendly_sqlx_error(error, &host_for_error, port))?,
        None => connect
            .await
            .map_err(|error| crate::dispatch::friendly_sqlx_error(error, &host_for_error, port))?,
    };
    apply_driver_options_raw(
        &mut connection,
        &spec.driver_options,
        spec.safety_policy.read_only,
    )
    .await
    .map_err(|error| error.to_string())?;
    connection
        .execute("SELECT 1")
        .await
        .map_err(|error| crate::dispatch::friendly_sqlx_error(error, &host_for_error, port))?;
    Ok(started.elapsed().as_millis() as u64)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgDriverOptions, PgStoredConnection};
    use sqlx::ConnectOptions as _;

    fn probe(connection_id: &str) -> StoredConnection {
        StoredConnection::PostgreSQL(PgStoredConnection {
            id: connection_id.into(),
            name: "probe".into(),
            database: "postgres".into(),
            host: "127.0.0.1".into(),
            port: 1,
            user: "postgres".into(),
            password: String::new(),
            role: String::new(),
            environment: Default::default(),
            safe_mode: Default::default(),
            read_only: false,
            last_activity_at: None,
            organization: Default::default(),
            ssl: false,
            tls_options: None,
            driver_options: Some(PgDriverOptions {
                connect_timeout_ms: Some(100),
                ..Default::default()
            }),
            ssh_tunnel: Default::default(),
        })
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn one_shot_probe_ignores_a_cached_pool_with_the_same_id() {
        let connection_id = "one-shot-does-not-use-cache";
        let sentinel = PgPoolOptions::new().connect_lazy_with(PgConnectOptions::new());
        sentinel.close().await;
        POOL_CACHE
            .lock()
            .unwrap()
            .insert(connection_id.into(), sentinel);

        let error = ping_once(&probe(connection_id))
            .await
            .expect_err("port 1 should refuse the one-shot probe");

        assert!(
            !error.contains("closed pool"),
            "the one-shot probe consulted the cached sentinel: {error}"
        );
        drop_pool(connection_id);
    }

    #[test]
    fn pooled_options_do_not_inherit_postgres_credentials_tls_or_options() {
        const CHILD: &str = "DBUNK_TEST_AMBIENT_POOL_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "postgres::pool::tests::pooled_options_do_not_inherit_postgres_credentials_tls_or_options",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("PGPASSWORD", "ambient-password")
                .env("PGPASSFILE", "/ambient/pgpass")
                .env("PGOPTIONS", "-c role=ambient_role")
                .env("PGSSLCERT", "/ambient/client.crt")
                .env("PGSSLKEY", "/ambient/client.key")
                .env("PGSSLROOTCERT", "/ambient/root.crt")
                .status()
                .expect("run isolated ambient-environment regression test");
            assert!(status.success(), "isolated regression test failed");
            return;
        }

        assert_eq!(
            std::env::var("PGPASSWORD").as_deref(),
            Ok("ambient-password")
        );

        let StoredConnection::PostgreSQL(pg) = probe("ambient-pool") else {
            unreachable!()
        };
        let options = build_connect_options(&ResolvedPostgresConnectSpec::from_postgres(&pg));
        for key in super::super::tls::SQLX_INHERITED_OVERRIDES {
            assert!(
                std::env::var_os(key).is_none(),
                "{key} was not neutralized before option construction"
            );
        }
        let url = options.to_url_lossy();
        let debug = format!("{options:?}");

        assert_ne!(url.password(), Some("ambient-password"));
        assert!(debug.contains("password: Some(\"\")"), "{debug}");
        assert_eq!(options.get_options(), None);
        for ambient in [
            "ambient-password",
            "/ambient/pgpass",
            "ambient_role",
            "/ambient/client.crt",
            "/ambient/client.key",
            "/ambient/root.crt",
        ] {
            assert!(!debug.contains(ambient), "inherited {ambient}: {debug}");
        }
    }
}
