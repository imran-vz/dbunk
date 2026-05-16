//! Process-wide cache of `redis-rs` `ConnectionManager`s, keyed by
//! `connection_id`. Auto-reconnect with exponential backoff is the
//! default behaviour of `ConnectionManager`; reusing one per
//! `StoredConnection` amortises TLS handshakes across the
//! keyspace-explorer fan-out.
//!
//! The Pub/Sub tab does NOT share managers from here — it opens its
//! own dedicated connection per tab (Phase 1.3) because `redis-rs`'s
//! pub/sub API can't be multiplexed with regular commands.
//!
//! ## Diagnostics
//!
//! `redis-rs::aio::MultiplexedConnection` swallows the underlying
//! cause of connect failures: any TCP / TLS / protocol error
//! surfaces as a generic "Multiplexed connection driver unexpectedly
//! terminated- IoError" string, which makes "wrong port" look
//! identical to "TLS required on server" look identical to "auth
//! rejected mid-handshake".
//!
//! Mitigation: every connect path here runs a **pre-flight TCP
//! check** via `tokio::net::TcpStream::connect` with a short timeout
//! before handing off to redis-rs. If the TCP connect fails we
//! surface a tight, OS-error-aware message; if TCP succeeds but
//! redis-rs still fails, we suggest TLS / auth / protocol as the
//! culprits in the returned error message.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use once_cell::sync::Lazy;
use redis::aio::ConnectionManager;
use redis::{Client, RedisError};

use crate::redis::url::{self, RedisUrl};
use crate::RedisStoredConnection;

const TCP_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Multiplexed connection (auto-reconnect) cached by connection ID.
///
/// Standard library `Mutex` works here because the critical section
/// never spans an `.await` — we lock, look up or insert, drop. The
/// `ConnectionManager` itself is `Clone`, so callers receive their
/// own clone without holding the lock.
static MANAGER_CACHE: Lazy<Mutex<HashMap<String, ConnectionManager>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Session-scoped cache of "is this Redis instance a replica?". The
/// answer drives `assert_writable` in `key_ops.rs` and only changes
/// across a failover — well outside the lifetime of an editor
/// session — so a per-`connection_id` cache means one `INFO
/// replication` round trip per session instead of one per write.
///
/// Cleared by [`drop_cached`] alongside the manager cache, so a
/// reconnect re-probes.
static REPLICA_ROLE_CACHE: Lazy<Mutex<HashMap<String, bool>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Returns a clone of the cached `ConnectionManager` for this
/// connection, opening and caching one if necessary.
pub async fn manager_for(connection: &RedisStoredConnection) -> Result<ConnectionManager, String> {
    if let Some(manager) = lookup(&connection.id) {
        log::debug!("manager_for: cache hit for connection_id={}", connection.id);
        return Ok(manager);
    }

    log::debug!(
        "manager_for: cache miss for connection_id={}, opening new manager",
        connection.id
    );
    let manager = open_manager(connection).await?;
    insert(connection.id.clone(), manager.clone());
    Ok(manager)
}

/// Drop the cached manager and replica-role for a connection — used
/// when the user disconnects or deletes the connection record. A
/// subsequent connect re-opens the manager and re-probes the role.
/// Best-effort.
#[allow(dead_code)]
pub fn drop_cached(connection_id: &str) {
    if let Ok(mut cache) = MANAGER_CACHE.lock() {
        if cache.remove(connection_id).is_some() {
            log::debug!("drop_cached: removed manager for {}", connection_id);
        }
    }
    if let Ok(mut cache) = REPLICA_ROLE_CACHE.lock() {
        cache.remove(connection_id);
    }
}

/// Look up the cached "is this connection a replica?" flag.
pub fn cached_replica_role(connection_id: &str) -> Option<bool> {
    REPLICA_ROLE_CACHE.lock().ok()?.get(connection_id).copied()
}

/// Cache the replica-role flag for this connection.
pub fn cache_replica_role(connection_id: &str, is_replica: bool) {
    if let Ok(mut cache) = REPLICA_ROLE_CACHE.lock() {
        cache.insert(connection_id.to_string(), is_replica);
    }
}

fn lookup(connection_id: &str) -> Option<ConnectionManager> {
    let cache = MANAGER_CACHE.lock().ok()?;
    cache.get(connection_id).cloned()
}

fn insert(connection_id: String, manager: ConnectionManager) {
    if let Ok(mut cache) = MANAGER_CACHE.lock() {
        cache.entry(connection_id).or_insert(manager);
    }
}

/// Open a new, single-shot connection without caching it. Used by
/// `test_connection` so a "Test" click doesn't leave a stale manager
/// hanging around in the cache (the saved connection may differ).
pub async fn open_oneshot(
    connection: &RedisStoredConnection,
) -> Result<redis::aio::MultiplexedConnection, String> {
    log_connect_attempt("open_oneshot", connection);
    preflight_tcp(connection).await?;

    let info = client_info(connection)?;
    let client = Client::open(info).map_err(redis_err)?;
    match client.get_multiplexed_async_connection().await {
        Ok(conn) => {
            log::info!(
                "open_oneshot: success for {}:{}",
                connection.host,
                effective_port(connection)
            );
            Ok(conn)
        }
        Err(err) => {
            let mapped = handshake_err(connection, err);
            log::warn!("open_oneshot: handshake failure → {}", mapped);
            Err(mapped)
        }
    }
}

async fn open_manager(connection: &RedisStoredConnection) -> Result<ConnectionManager, String> {
    log_connect_attempt("open_manager", connection);
    preflight_tcp(connection).await?;

    let info = client_info(connection)?;
    let client = Client::open(info).map_err(redis_err)?;
    match ConnectionManager::new(client).await {
        Ok(manager) => {
            log::info!(
                "open_manager: success for {}:{}",
                connection.host,
                effective_port(connection)
            );
            Ok(manager)
        }
        Err(err) => {
            let mapped = handshake_err(connection, err);
            log::warn!("open_manager: handshake failure → {}", mapped);
            Err(mapped)
        }
    }
}

fn client_info(connection: &RedisStoredConnection) -> Result<redis::ConnectionInfo, String> {
    let RedisUrl { url, .. } = url::build(connection)?;
    redis::IntoConnectionInfo::into_connection_info(url.as_str()).map_err(redis_err)
}

fn effective_port(connection: &RedisStoredConnection) -> u16 {
    if connection.port == 0 {
        6379
    } else {
        connection.port
    }
}

fn log_connect_attempt(site: &str, connection: &RedisStoredConnection) {
    log::info!(
        "{}: host={} port={} db={} tls={} verify_cert={} user_present={} password_present={}",
        site,
        connection.host,
        effective_port(connection),
        connection.db_number,
        connection.use_tls,
        connection.verify_tls_cert,
        !connection.user.is_empty(),
        !connection.password.is_empty(),
    );
}

/// TCP-level reachability check before handing off to redis-rs. Gives
/// us a clear OS error when the host/port is wrong, instead of the
/// opaque "multiplexed driver terminated" message redis-rs returns
/// for everything below the protocol layer.
async fn preflight_tcp(connection: &RedisStoredConnection) -> Result<(), String> {
    let host = connection.host.clone();
    let port = effective_port(connection);
    let target = format!("{host}:{port}");

    log::debug!("preflight_tcp: connecting to {}", target);
    let started = std::time::Instant::now();

    match tokio::time::timeout(TCP_PROBE_TIMEOUT, tokio::net::TcpStream::connect(&target)).await {
        Ok(Ok(_stream)) => {
            log::debug!(
                "preflight_tcp: TCP reachable in {}ms",
                started.elapsed().as_millis()
            );
            Ok(())
        }
        Ok(Err(err)) => {
            let mapped = tcp_io_error(&host, port, &err);
            log::warn!("preflight_tcp: TCP failed → {} (raw: {})", mapped, err);
            Err(mapped)
        }
        Err(_) => {
            let msg = format!(
                "Timed out after {}s connecting to {}:{}. The host may be unreachable, blocked by a firewall, or the port is wrong.",
                TCP_PROBE_TIMEOUT.as_secs(),
                host,
                port
            );
            log::warn!("preflight_tcp: timeout → {}", msg);
            Err(msg)
        }
    }
}

fn tcp_io_error(host: &str, port: u16, err: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::ConnectionRefused => format!(
            "Could not connect to {host}:{port}: connection refused. Is Redis running on this port?"
        ),
        ErrorKind::TimedOut => format!(
            "Connection to {host}:{port} timed out. The host may be unreachable or blocked by a firewall."
        ),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable | ErrorKind::NetworkDown => {
            format!("Host \"{host}\" is unreachable from this network.")
        }
        _ => {
            let text = err.to_string();
            let lower = text.to_lowercase();
            if lower.contains("nodename nor servname")
                || lower.contains("name or service not known")
                || lower.contains("no such host")
                || lower.contains("failed to lookup address")
                || lower.contains("temporary failure in name resolution")
            {
                format!(
                    "Could not resolve hostname \"{host}\". Check the address and your DNS settings."
                )
            } else {
                format!("Network error connecting to {host}:{port}: {text}")
            }
        }
    }
}

/// Build the user-facing error for a redis-rs failure that happened
/// AFTER the TCP probe succeeded. By construction this is no longer
/// "host unreachable" — it's protocol-layer (TLS mismatch, AUTH
/// rejected, server speaks a different protocol).
///
/// IoError messages are matched against distinctive substrings so the
/// hint can point at the actual cause rather than listing every
/// possibility:
/// - rustls's `InvalidContentType` / "corrupt message" → server is
///   plaintext, the user has TLS on
/// - redis-rs's "driver unexpectedly terminated" with no creds set →
///   server most likely requires AUTH and closed the socket
///
/// Anything we can't pattern-match falls through to the generic
/// three-cause hint.
fn handshake_err(connection: &RedisStoredConnection, error: RedisError) -> String {
    use redis::ErrorKind;

    let host = connection.host.as_str();
    let port = effective_port(connection);
    let scheme = if connection.use_tls {
        "rediss"
    } else {
        "redis"
    };

    match error.kind() {
        ErrorKind::AuthenticationFailed => format!(
            "Authentication rejected by {host}:{port}. \
             Check the username and password. \
             If the server uses ACLs (Redis 6+), the username must match a configured user."
        ),
        ErrorKind::IoError => io_handshake_err(connection, &error, host, port, scheme),
        ErrorKind::ResponseError => {
            // Redis 6+ AUTH failures surface here as a `ResponseError`
            // with a NOAUTH/WRONGPASS prefix on the detail line. Lift
            // the canonical hints to the top so the user sees them
            // before the raw detail.
            let detail = error.detail().unwrap_or("(no detail)");
            let upper = detail.to_uppercase();
            if upper.starts_with("NOAUTH") {
                format!(
                    "Authentication required by {host}:{port}. \
                     Add a password (and a username for ACL-based Redis 6+) under the form."
                )
            } else if upper.starts_with("WRONGPASS") {
                format!(
                    "Authentication rejected by {host}:{port}: wrong password. \
                     Check the credentials under the form."
                )
            } else {
                format!("Redis rejected the connection: {detail}")
            }
        }
        _ => format!(
            "Redis handshake with {host}:{port} failed ({}): {}",
            error.category(),
            error
        ),
    }
}

fn io_handshake_err(
    connection: &RedisStoredConnection,
    error: &RedisError,
    host: &str,
    port: u16,
    scheme: &str,
) -> String {
    let text = error.to_string();
    let lower = text.to_lowercase();

    // Distinctive rustls signal: the server replied with bytes that
    // weren't TLS records. The most common cause is talking TLS to a
    // plaintext server (`rediss://` against `redis://`-only port).
    if connection.use_tls
        && (lower.contains("invalidcontenttype")
            || lower.contains("corrupt message")
            || lower.contains("not enough data"))
    {
        return format!(
            "Connected to {host}:{port} over rediss:// but the server returned non-TLS data. \
             The server is most likely plaintext — disable \"Use TLS\" under Advanced Options."
        );
    }

    // Distinctive redis-rs signal: the MultiplexedConnection driver
    // died mid-handshake. With no credentials on the client side this
    // is almost always the server requiring AUTH and closing the
    // socket on the anonymous client.
    if !connection.use_tls && lower.contains("driver unexpectedly terminated") {
        let auth_hint = if connection.password.is_empty() {
            "The server may require a password (Redis `requirepass`) or a named ACL user — add credentials under the form."
        } else {
            "AUTH may have been rejected; double-check the username and password."
        };
        return format!(
            "Connected to {host}:{port} over redis:// but the server closed the connection during the handshake. \
             {auth_hint} Other possibilities: the server requires TLS (enable \"Use TLS\" under Advanced Options), \
             or the port is reachable but is not a Redis server."
        );
    }

    // Fall-through: keep the generic three-cause hint with the
    // TLS-direction toggle suggestion.
    let tls_hint = if connection.use_tls {
        "If you're using a self-signed TLS cert, disable \"Verify TLS certificate\" under Advanced Options."
    } else {
        "If the server requires TLS, enable \"Use TLS\" under Advanced Options."
    };
    format!(
        "Connected to {host}:{port} over {scheme}:// but the handshake failed: {error}. \
         Possible causes: TLS mismatch (server requires the opposite of what's set), \
         auth rejected and the server closed the socket, \
         or the port is reachable but is not a Redis server. \
         {tls_hint}"
    )
}

/// Generic Redis-error mapper used outside the connect path (per-key
/// reads, CLI commands, pub/sub). Less context than `handshake_err`
/// because we're past the connect phase here.
pub fn redis_err(error: RedisError) -> String {
    use redis::ErrorKind;

    log::debug!(
        "redis_err: kind={:?} category={} detail={:?} display={}",
        error.kind(),
        error.category(),
        error.detail(),
        error
    );

    match error.kind() {
        ErrorKind::IoError => format!(
            "Could not reach the Redis server: {}. Check that the host is reachable and the port is open.",
            error
        ),
        ErrorKind::AuthenticationFailed => {
            "Redis authentication failed — check the username and password.".to_string()
        }
        ErrorKind::NoScriptError => format!("Redis returned NOSCRIPT: {}", error),
        ErrorKind::TypeError => format!("Redis type error: {}", error),
        ErrorKind::ResponseError => error
            .detail()
            .unwrap_or("Redis returned an error")
            .to_string(),
        _ => error.to_string(),
    }
}
