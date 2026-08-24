//! SSH tunnel runtime for ADR-0018.
//!
//! The rest of the engine code should not know how SSH works. It asks
//! this module to resolve a `StoredConnection` into the endpoint it
//! should dial. When the connection has no tunnel, the record is
//! returned unchanged. When a tunnel is enabled, this module fails
//! closed on any SSH/forwarding error and rewrites only the runtime
//! host/port to a local listener.

use std::{
    collections::HashMap,
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;
use sqlx::SqlitePool;
use ssh2::Session;

use crate::{credentials, storage, CredentialStorageMode, StoredConnection, TestBastionResult};

const DEFAULT_LOCAL_BIND_HOST: &str = "127.0.0.1";
const DEFAULT_SSH_PORT: u16 = 22;
const SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(super) struct LocalEndpoint {
    pub(super) host: String,
    pub(super) port: u16,
}

mod endpoint;
mod forwarding;
mod route;
mod session;

use endpoint::{remote_endpoint, rewrite_connection_endpoint};
use forwarding::spawn_forward_accept_loop;
pub use route::validate_connection_tunnel;
use route::{SshRoute, SshSessionKey};
use session::{connect_route_session, ResolvedBastion, RouteSession};

struct ForwardState {
    bastion_ids: Vec<String>,
    session_key: SshSessionKey,
    endpoint: LocalEndpoint,
    stop: Arc<AtomicBool>,
}

struct Runtime {
    sessions: HashMap<SshSessionKey, RouteSession>,
    forwards: HashMap<String, ForwardState>,
}

static RUNTIME: Lazy<Mutex<Runtime>> = Lazy::new(|| {
    Mutex::new(Runtime {
        sessions: HashMap::new(),
        forwards: HashMap::new(),
    })
});

/// Cancellation-safe owner for a short-lived forwarding route. Creating
/// the guard before awaiting route setup ensures a cancelled command cannot
/// leave a listener or tunnel session behind.
pub(crate) struct EphemeralRoute {
    key: String,
}

impl EphemeralRoute {
    pub(crate) fn new(prefix: &str) -> Self {
        Self {
            key: format!("{prefix}-{}", uuid::Uuid::new_v4()),
        }
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for EphemeralRoute {
    fn drop(&mut self) {
        drop_connection(&self.key);
    }
}

pub async fn resolve_connection(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    route_key: &str,
    connection: &StoredConnection,
) -> Result<StoredConnection, String> {
    let Some(config) = connection.ssh_tunnel() else {
        return Ok(connection.clone());
    };
    if !config.enabled {
        return Ok(connection.clone());
    }
    let endpoint = ensure_forward(pool, mode, route_key, connection).await?;
    rewrite_connection_endpoint(connection, &endpoint)
}

pub fn drop_connection(connection_id: &str) {
    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    drop_forward_locked(&mut runtime, connection_id);
    drop_unused_sessions_locked(&mut runtime);
}

pub fn drop_bastion(bastion_id: &str) {
    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    let route_keys = runtime
        .forwards
        .iter()
        .filter_map(|(key, forward)| {
            if forward.bastion_ids.iter().any(|id| id == bastion_id) {
                Some(key.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for route_key in route_keys {
        drop_forward_locked(&mut runtime, &route_key);
    }
    drop_unused_sessions_locked(&mut runtime);
}

pub async fn test_bastion(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    bastion_id: &str,
) -> Result<TestBastionResult, String> {
    let started = Instant::now();
    let bastion = load_bastion(pool, mode, bastion_id).await?;
    let route = SshRoute::from_config(&crate::SshTunnelConfig {
        enabled: true,
        bastion_server_id: Some(bastion_id.to_string()),
        ..crate::SshTunnelConfig::default()
    })?;
    let (mut route_session, accepted_fingerprints) = tokio::task::spawn_blocking(move || {
        let bastions = vec![bastion];
        connect_route_session(&bastions, &route)
    })
    .await
    .map_err(|error| error.to_string())??;
    for (accepted_bastion_id, fingerprint) in accepted_fingerprints {
        storage::bastions::update_bastion_host_key_fingerprint(
            pool,
            &accepted_bastion_id,
            Some(&fingerprint),
        )
        .await?;
    }
    route_session.shutdown("dbunk test complete");
    Ok(TestBastionResult {
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

async fn ensure_forward(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    route_key: &str,
    connection: &StoredConnection,
) -> Result<LocalEndpoint, String> {
    if let Some(endpoint) = lookup_forward(route_key) {
        return Ok(endpoint);
    }

    let config = connection
        .ssh_tunnel()
        .ok_or_else(|| "SQLite connections do not support SSH tunnels".to_string())?;
    let config = config.normalized();
    let route = SshRoute::from_config(&config)?;
    let bind_host = config
        .local_bind_host
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(DEFAULT_LOCAL_BIND_HOST)
        .to_string();
    let bind_port = config.local_port.unwrap_or(0);

    let (remote_host, remote_port) = remote_endpoint(connection)?;
    let listener = TcpListener::bind((bind_host.as_str(), bind_port)).map_err(|error| {
        format!("Failed to bind SSH tunnel listener on {bind_host}:{bind_port}: {error}")
    })?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let local_addr = listener.local_addr().map_err(|error| error.to_string())?;
    let endpoint = LocalEndpoint {
        host: bind_host,
        port: local_addr.port(),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let session = ensure_session(pool, mode, &route).await?;
    spawn_forward_accept_loop(listener, stop.clone(), session, remote_host, remote_port);

    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    if let Some(existing) = runtime.forwards.get(route_key) {
        stop.store(true, Ordering::SeqCst);
        return Ok(existing.endpoint.clone());
    }
    runtime.forwards.insert(
        route_key.to_string(),
        ForwardState {
            bastion_ids: route.bastion_ids,
            session_key: route.session_key,
            endpoint: endpoint.clone(),
            stop,
        },
    );
    Ok(endpoint)
}

fn lookup_forward(route_key: &str) -> Option<LocalEndpoint> {
    RUNTIME
        .lock()
        .ok()?
        .forwards
        .get(route_key)
        .map(|forward| forward.endpoint.clone())
}

async fn ensure_session(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    route: &SshRoute,
) -> Result<Session, String> {
    if let Some(session) = RUNTIME
        .lock()
        .expect("ssh tunnel runtime poisoned")
        .sessions
        .get(&route.session_key)
        .map(|state| state.session())
    {
        return Ok(session);
    }

    let mut bastions = Vec::with_capacity(route.bastion_ids.len());
    for bastion_id in &route.bastion_ids {
        bastions.push(load_bastion(pool, mode, bastion_id).await?);
    }
    let route_for_connect = route.clone();
    let (mut route_session, accepted_fingerprints) =
        tokio::task::spawn_blocking(move || connect_route_session(&bastions, &route_for_connect))
            .await
            .map_err(|error| error.to_string())??;

    for (bastion_id, fingerprint) in accepted_fingerprints {
        storage::bastions::update_bastion_host_key_fingerprint(
            pool,
            &bastion_id,
            Some(&fingerprint),
        )
        .await?;
    }

    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    if let Some(existing) = runtime.sessions.get(&route.session_key) {
        route_session.shutdown("dbunk duplicate session closed");
        return Ok(existing.session());
    }
    let session = route_session.session();
    runtime
        .sessions
        .insert(route.session_key.clone(), route_session);
    Ok(session)
}

async fn load_bastion(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    bastion_id: &str,
) -> Result<ResolvedBastion, String> {
    let server = storage::bastions::read_bastion_server_by_id(pool, bastion_id)
        .await?
        .ok_or_else(|| format!("Bastion Server '{bastion_id}' not found"))?;
    let password = credentials::read_bastion_secret(pool, mode, bastion_id, "password").await?;
    let private_key_content =
        credentials::read_bastion_secret(pool, mode, bastion_id, "privateKeyContent").await?;
    let passphrase = credentials::read_bastion_secret(pool, mode, bastion_id, "passphrase").await?;
    Ok(ResolvedBastion {
        server,
        password,
        private_key_content,
        passphrase,
    })
}

fn drop_forward_locked(runtime: &mut Runtime, route_key: &str) {
    if let Some(forward) = runtime.forwards.remove(route_key) {
        forward.stop.store(true, Ordering::SeqCst);
    }
}

fn drop_unused_sessions_locked(runtime: &mut Runtime) {
    let active_session_keys = runtime
        .forwards
        .values()
        .map(|forward| forward.session_key.clone())
        .collect::<std::collections::HashSet<_>>();
    runtime.sessions.retain(|session_key, state| {
        let keep = active_session_keys.contains(session_key);
        if !keep {
            state.shutdown("dbunk ssh session closed");
        }
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    fn ephemeral_route_drop_removes_forward_and_stops_listener() {
        let route = EphemeralRoute::new("diag");
        assert!(route.key().starts_with("diag-"));
        let route_key = route.key().to_string();
        let ssh_route = SshRoute::from_config(&crate::SshTunnelConfig {
            enabled: true,
            bastion_server_id: Some("bastion".into()),
            ..Default::default()
        })
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        RUNTIME.lock().unwrap().forwards.insert(
            route.key().to_string(),
            ForwardState {
                bastion_ids: ssh_route.bastion_ids,
                session_key: ssh_route.session_key,
                endpoint: LocalEndpoint {
                    host: "127.0.0.1".into(),
                    port: 1,
                },
                stop: Arc::clone(&stop),
            },
        );

        drop(route);

        assert!(stop.load(Ordering::SeqCst));
        assert!(!RUNTIME.lock().unwrap().forwards.contains_key(&route_key));
    }
}
