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
    future::Future,
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
    publication_id: uuid::Uuid,
    bastion_ids: Vec<String>,
    session_key: SshSessionKey,
    endpoint: LocalEndpoint,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ForwardState {
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct CachedSession {
    publication_id: uuid::Uuid,
    route: RouteSession,
    pending_leases: usize,
}

struct Runtime {
    sessions: HashMap<SshSessionKey, CachedSession>,
    forwards: HashMap<String, ForwardState>,
}

struct SessionLease {
    session: Session,
    publication: Option<(SshSessionKey, uuid::Uuid)>,
}

impl SessionLease {
    fn new(session: Session, session_key: SshSessionKey, publication_id: uuid::Uuid) -> Self {
        Self {
            session,
            publication: Some((session_key, publication_id)),
        }
    }

    fn session(&self) -> Session {
        self.session.clone()
    }

    fn commit(&mut self) {
        self.release();
    }

    fn release(&mut self) {
        let Some((session_key, publication_id)) = self.publication.take() else {
            return;
        };
        release_session_lease(&session_key, publication_id);
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        self.release();
    }
}

struct ForwardPublication {
    route_key: String,
    publication_id: uuid::Uuid,
    committed: bool,
}

impl ForwardPublication {
    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for ForwardPublication {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        rollback_forward(&self.route_key, self.publication_id);
    }
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
    resolve_connection_checked(pool, mode, route_key, connection, Arc::new(|| Ok(()))).await
}

/// The caller retains this future until it stops. The check prevents a cancelled
/// setup from starting another SSH hop or publishing a newly resolved route.
pub(crate) async fn resolve_connection_checked(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    route_key: &str,
    connection: &StoredConnection,
    check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
) -> Result<StoredConnection, String> {
    check()?;
    let Some(config) = connection.ssh_tunnel() else {
        return Ok(connection.clone());
    };
    if !config.enabled {
        return Ok(connection.clone());
    }
    let endpoint = ensure_forward(pool, mode, route_key, connection, check).await?;
    rewrite_connection_endpoint(connection, &endpoint)
}

pub fn drop_connection(connection_id: &str) {
    let (forward, sessions) = {
        let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
        let forward = runtime.forwards.remove(connection_id);
        let sessions = take_unused_sessions_locked(&mut runtime);
        (forward, sessions)
    };
    shutdown_resources(forward, sessions);
}

pub fn drop_bastion(bastion_id: &str) {
    let (forwards, sessions) = {
        let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
        let route_keys = runtime
            .forwards
            .iter()
            .filter(|(_, forward)| forward.bastion_ids.iter().any(|id| id == bastion_id))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let forwards = route_keys
            .into_iter()
            .filter_map(|route_key| runtime.forwards.remove(&route_key))
            .collect::<Vec<_>>();
        let sessions = take_unused_sessions_locked(&mut runtime);
        (forwards, sessions)
    };
    shutdown_resources(forwards, sessions);
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
    check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
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
    let session = ensure_session(pool, mode, &route, check.clone()).await?;
    check()?;
    let worker = spawn_forward_accept_loop(
        listener,
        stop.clone(),
        session.session(),
        remote_host,
        remote_port,
    );
    publish_forward(route_key, route, endpoint, stop, worker, session, check)
}

fn publish_forward(
    route_key: &str,
    route: SshRoute,
    endpoint: LocalEndpoint,
    stop: Arc<AtomicBool>,
    worker: std::thread::JoinHandle<()>,
    mut session: SessionLease,
    check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
) -> Result<LocalEndpoint, String> {
    let publication_id = uuid::Uuid::new_v4();
    {
        let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
        if let Some(existing) = runtime.forwards.get(route_key) {
            stop.store(true, Ordering::SeqCst);
            let endpoint = existing.endpoint.clone();
            drop(runtime);
            let _ = worker.join();
            check()?;
            return Ok(endpoint);
        }
        runtime.forwards.insert(
            route_key.to_string(),
            ForwardState {
                publication_id,
                bastion_ids: route.bastion_ids,
                session_key: route.session_key,
                endpoint: endpoint.clone(),
                stop,
                worker: Some(worker),
            },
        );
    }
    let mut publication = ForwardPublication {
        route_key: route_key.to_string(),
        publication_id,
        committed: false,
    };
    // If cancellation wins after cache publication, the two guards remove the
    // exact forward and session created by this attempt. Pre-existing routes,
    // and a newly shared session already referenced by another forward, remain.
    check()?;
    publication.commit();
    session.commit();
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
    check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
) -> Result<SessionLease, String> {
    {
        let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
        if let Some(state) = runtime.sessions.get_mut(&route.session_key) {
            state.pending_leases += 1;
            return Ok(SessionLease::new(
                state.route.session(),
                route.session_key.clone(),
                state.publication_id,
            ));
        }
    }

    let mut bastions = Vec::with_capacity(route.bastion_ids.len());
    for bastion_id in &route.bastion_ids {
        bastions
            .push(await_setup_checked(load_bastion(pool, mode, bastion_id), check.as_ref()).await?);
    }
    check()?;
    let worker_check = check.clone();
    let route_for_connect = route.clone();
    let (route_session, accepted_fingerprints) = tokio::task::spawn_blocking(move || {
        session::connect_route_session_checked(&bastions, &route_for_connect, worker_check.as_ref())
    })
    .await
    .map_err(|error| error.to_string())??;

    finish_session_setup(
        pool,
        route,
        route_session,
        accepted_fingerprints,
        check.as_ref(),
    )
    .await
}

async fn finish_session_setup(
    pool: &SqlitePool,
    route: &SshRoute,
    route_session: RouteSession,
    accepted_fingerprints: Vec<(String, String)>,
    check: &(dyn Fn() -> Result<(), String> + Send + Sync),
) -> Result<SessionLease, String> {
    check()?;
    for (bastion_id, fingerprint) in accepted_fingerprints {
        let mut connection = await_setup_checked(
            async { pool.acquire().await.map_err(|error| error.to_string()) },
            check,
        )
        .await?;
        check()?;
        // SQLite's worker can execute a queued write even after its future is
        // dropped. Retain setup ownership until it finishes, then honor cancellation.
        storage::bastions::update_bastion_host_key_fingerprint(
            &mut *connection,
            &bastion_id,
            Some(&fingerprint),
        )
        .await?;
        check()?;
    }

    check()?;
    let (lease, duplicate) = publish_session(route, route_session);
    if let Some(mut duplicate) = duplicate {
        duplicate.shutdown("dbunk duplicate session closed");
    }
    Ok(lease)
}

/// Only for droppable async setup I/O. SSH workers must still be awaited through
/// their join, as must dispatched SQLite writes. An already connected RouteSession
/// stays owned by the caller.
async fn await_setup_checked<T>(
    operation: impl Future<Output = Result<T, String>>,
    check: &(dyn Fn() -> Result<(), String> + Send + Sync),
) -> Result<T, String> {
    tokio::pin!(operation);
    loop {
        check()?;
        tokio::select! {
            result = &mut operation => {
                check()?;
                return result;
            }
            () = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
}

fn publish_session(
    route: &SshRoute,
    route_session: RouteSession,
) -> (SessionLease, Option<RouteSession>) {
    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    if let Some(existing) = runtime.sessions.get_mut(&route.session_key) {
        existing.pending_leases += 1;
        let lease = SessionLease::new(
            existing.route.session(),
            route.session_key.clone(),
            existing.publication_id,
        );
        return (lease, Some(route_session));
    }
    let session = route_session.session();
    let publication_id = uuid::Uuid::new_v4();
    runtime.sessions.insert(
        route.session_key.clone(),
        CachedSession {
            publication_id,
            route: route_session,
            pending_leases: 1,
        },
    );
    (
        SessionLease::new(session, route.session_key.clone(), publication_id),
        None,
    )
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

fn take_unused_sessions_locked(runtime: &mut Runtime) -> Vec<CachedSession> {
    let active_session_keys = runtime
        .forwards
        .values()
        .map(|forward| forward.session_key.clone())
        .collect::<std::collections::HashSet<_>>();
    let unused = runtime
        .sessions
        .keys()
        .filter(|session_key| {
            !active_session_keys.contains(*session_key)
                && runtime.sessions[*session_key].pending_leases == 0
        })
        .cloned()
        .collect::<Vec<_>>();
    unused
        .into_iter()
        .filter_map(|session_key| runtime.sessions.remove(&session_key))
        .collect()
}

fn shutdown_resources(
    forwards: impl IntoIterator<Item = ForwardState>,
    sessions: Vec<CachedSession>,
) {
    for forward in forwards {
        forward.shutdown();
    }
    for mut session in sessions {
        session.route.shutdown("dbunk ssh session closed");
    }
}

fn rollback_forward(route_key: &str, publication_id: uuid::Uuid) {
    let (forward, sessions) = {
        let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
        let owns_forward = runtime
            .forwards
            .get(route_key)
            .is_some_and(|forward| forward.publication_id == publication_id);
        let forward = owns_forward
            .then(|| runtime.forwards.remove(route_key))
            .flatten();
        let sessions = take_unused_sessions_locked(&mut runtime);
        (forward, sessions)
    };
    shutdown_resources(forward, sessions);
}

fn release_session_lease(session_key: &SshSessionKey, publication_id: uuid::Uuid) {
    let session = {
        let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
        let Some(cached) = runtime
            .sessions
            .get_mut(session_key)
            .filter(|session| session.publication_id == publication_id)
        else {
            return;
        };
        cached.pending_leases = cached.pending_leases.saturating_sub(1);
        let remove = cached.pending_leases == 0
            && !runtime
                .forwards
                .values()
                .any(|forward| &forward.session_key == session_key);
        remove
            .then(|| runtime.sessions.remove(session_key))
            .flatten()
    };
    if let Some(mut session) = session {
        session.route.shutdown("dbunk cancelled ssh setup closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_route(id: &str) -> SshRoute {
        SshRoute::from_config(&crate::SshTunnelConfig {
            enabled: true,
            bastion_server_id: Some(id.into()),
            ..Default::default()
        })
        .unwrap()
    }

    fn test_session(route: &SshRoute, pending_leases: usize) -> SessionLease {
        let route_session = session::disconnected_route_session();
        let session = route_session.session();
        let publication_id = uuid::Uuid::new_v4();
        RUNTIME.lock().unwrap().sessions.insert(
            route.session_key.clone(),
            CachedSession {
                publication_id,
                route: route_session,
                pending_leases,
            },
        );
        SessionLease::new(session, route.session_key.clone(), publication_id)
    }

    fn stopped_worker(stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                std::thread::park_timeout(Duration::from_millis(1));
            }
        })
    }

    async fn exhausted_pool() -> (SqlitePool, Vec<sqlx::pool::PoolConnection<sqlx::Sqlite>>) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut held = Vec::new();
        for _ in 0..5 {
            held.push(pool.acquire().await.unwrap());
        }
        (pool, held)
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn cancellation_interrupts_bastion_pool_wait_before_ssh_starts() {
        let (pool, _held) = exhausted_pool().await;
        let route = ssh_route("cancelled-pool-wait");
        let cancelled = Arc::new(AtomicBool::new(false));
        let check_cancelled = cancelled.clone();
        let setup = ensure_session(
            &pool,
            CredentialStorageMode::PlainSqlite,
            &route,
            Arc::new(move || {
                if check_cancelled.load(Ordering::SeqCst) {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            }),
        );
        tokio::pin!(setup);
        assert!(futures_util::poll!(&mut setup).is_pending());
        cancelled.store(true, Ordering::SeqCst);
        let result = tokio::time::timeout(Duration::from_secs(1), setup)
            .await
            .expect("cancellation must not wait for the pool acquisition timeout");
        assert!(matches!(result, Err(error) if error == "cancelled"));
        assert!(!RUNTIME
            .lock()
            .unwrap()
            .sessions
            .contains_key(&route.session_key));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn original_deadline_interrupts_bastion_pool_wait() {
        let (pool, _held) = exhausted_pool().await;
        let route = ssh_route("expired-pool-wait");
        let deadline = Instant::now() + Duration::from_millis(100);
        let setup = ensure_session(
            &pool,
            CredentialStorageMode::PlainSqlite,
            &route,
            Arc::new(move || {
                if Instant::now() >= deadline {
                    Err("deadline expired".into())
                } else {
                    Ok(())
                }
            }),
        );
        tokio::pin!(setup);
        assert!(futures_util::poll!(&mut setup).is_pending());
        let result = tokio::time::timeout(Duration::from_secs(1), setup)
            .await
            .expect("the original deadline must interrupt pool acquisition");
        assert!(matches!(result, Err(error) if error == "deadline expired"));
        assert!(!RUNTIME
            .lock()
            .unwrap()
            .sessions
            .contains_key(&route.session_key));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn cancellation_interrupts_fingerprint_pool_wait_without_publishing_session() {
        let (pool, _held) = exhausted_pool().await;
        let route = ssh_route("cancelled-fingerprint-wait");
        let cancelled = AtomicBool::new(false);
        let check = || {
            if cancelled.load(Ordering::SeqCst) {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        };
        let setup = finish_session_setup(
            &pool,
            &route,
            session::disconnected_route_session(),
            vec![(route.bastion_ids[0].clone(), "test-fingerprint".into())],
            &check,
        );
        tokio::pin!(setup);
        assert!(futures_util::poll!(&mut setup).is_pending());
        cancelled.store(true, Ordering::SeqCst);
        let result = tokio::time::timeout(Duration::from_secs(1), setup)
            .await
            .expect("fingerprint persistence must observe cancellation");
        assert!(matches!(result, Err(error) if error == "cancelled"));
        assert!(!RUNTIME
            .lock()
            .unwrap()
            .sessions
            .contains_key(&route.session_key));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn cancellation_joins_dispatched_fingerprint_write_before_host_key_reset() {
        let directory = tempfile::Builder::new()
            .prefix("dbunk-fingerprint-write-ownership-")
            .tempdir()
            .unwrap();
        let connection_acquired = Arc::new(AtomicBool::new(false));
        let acquired = connection_acquired.clone();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .min_connections(2)
            .test_before_acquire(false)
            .before_acquire(move |_, _| {
                acquired.store(true, Ordering::SeqCst);
                Box::pin(async { Ok(true) })
            })
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("bastions.sqlite"))
                    .create_if_missing(true)
                    .busy_timeout(Duration::from_secs(5)),
            )
            .await
            .unwrap();
        let route = ssh_route("cancelled-dispatched-fingerprint-write");
        sqlx::query(
            "CREATE TABLE bastion_servers (
                id TEXT PRIMARY KEY, host_key_fingerprint TEXT, updated_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO bastion_servers (id) VALUES (?)")
            .bind(&route.bastion_ids[0])
            .execute(&pool)
            .await
            .unwrap();
        let mut blocker = pool.acquire().await.unwrap();
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *blocker)
            .await
            .unwrap();

        connection_acquired.store(false, Ordering::SeqCst);

        let cancelled = AtomicBool::new(false);
        let check = || {
            if cancelled.load(Ordering::SeqCst) {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        };
        let setup = finish_session_setup(
            &pool,
            &route,
            session::disconnected_route_session(),
            vec![(route.bastion_ids[0].clone(), "cancelled-setup-key".into())],
            &check,
        );
        tokio::pin!(setup);
        // With the spare connection already open and its pool check ready,
        // this poll acquires it and dispatches the UPDATE before yielding.
        tokio::time::timeout(Duration::from_secs(1), async {
            while !connection_acquired.load(Ordering::SeqCst) {
                tokio::select! {
                    _ = &mut setup => panic!("setup finished while the write was locked"),
                    () = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
        })
        .await
        .expect("setup must acquire the spare connection and dispatch its write");
        cancelled.store(true, Ordering::SeqCst);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut setup)
                .await
                .is_err(),
            "cancelled setup must retain ownership while its SQLite write is pending"
        );
        assert!(!RUNTIME
            .lock()
            .unwrap()
            .sessions
            .contains_key(&route.session_key));

        sqlx::query("COMMIT").execute(&mut *blocker).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), &mut setup)
            .await
            .expect("setup must finish after its owned write completes");
        assert!(matches!(result, Err(error) if error == "cancelled"));
        assert!(!RUNTIME
            .lock()
            .unwrap()
            .sessions
            .contains_key(&route.session_key));
        let fingerprint: Option<String> =
            sqlx::query_scalar("SELECT host_key_fingerprint FROM bastion_servers WHERE id = ?")
                .bind(&route.bastion_ids[0])
                .fetch_one(&mut *blocker)
                .await
                .unwrap();
        assert_eq!(fingerprint.as_deref(), Some("cancelled-setup-key"));

        storage::bastions::update_bastion_host_key_fingerprint(
            &mut *blocker,
            &route.bastion_ids[0],
            None,
        )
        .await
        .unwrap();
        drop(blocker);
        pool.close().await;
        // Reopening after draining both SQLite workers proves no abandoned
        // setup write can restore the fingerprint after the fenced reset.
        let reopened = SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(directory.path().join("bastions.sqlite")),
        )
        .await
        .unwrap();
        let fingerprint: Option<String> =
            sqlx::query_scalar("SELECT host_key_fingerprint FROM bastion_servers WHERE id = ?")
                .bind(&route.bastion_ids[0])
                .fetch_one(&reopened)
                .await
                .unwrap();
        assert_eq!(fingerprint, None);
        reopened.close().await;
    }

    #[test]
    #[serial_test::serial]
    fn ephemeral_route_drop_removes_forward_and_stops_listener() {
        let route = EphemeralRoute::new("diag");
        assert!(route.key().starts_with("diag-"));
        let route_key = route.key().to_string();
        let ssh_route = ssh_route("bastion");
        let stop = Arc::new(AtomicBool::new(false));
        RUNTIME.lock().unwrap().forwards.insert(
            route.key().to_string(),
            ForwardState {
                publication_id: uuid::Uuid::new_v4(),
                bastion_ids: ssh_route.bastion_ids,
                session_key: ssh_route.session_key,
                endpoint: LocalEndpoint {
                    host: "127.0.0.1".into(),
                    port: 1,
                },
                stop: Arc::clone(&stop),
                worker: None,
            },
        );

        drop(route);

        assert!(stop.load(Ordering::SeqCst));
        assert!(!RUNTIME.lock().unwrap().forwards.contains_key(&route_key));
    }

    #[test]
    #[serial_test::serial]
    fn cancelled_session_publication_waits_for_other_pending_setup() {
        let route = ssh_route("pending-session");
        let first = test_session(&route, 2);
        let session = first.session();
        let publication_id = first.publication.as_ref().unwrap().1;
        let second = SessionLease::new(session, route.session_key.clone(), publication_id);

        drop(first);
        assert_eq!(
            RUNTIME.lock().unwrap().sessions[&route.session_key].pending_leases,
            1
        );
        drop(second);
        assert!(!RUNTIME
            .lock()
            .unwrap()
            .sessions
            .contains_key(&route.session_key));
    }

    #[test]
    #[serial_test::serial]
    fn duplicate_session_cleanup_does_not_block_runtime_lookups() {
        let route = ssh_route("duplicate-session");
        let existing = test_session(&route, 1);
        let duplicate = session::disconnected_route_session();
        let (cleanup_started, cleanup_ready) = std::sync::mpsc::channel();
        let (release_cleanup, cleanup_released) = std::sync::mpsc::channel();
        let publisher = std::thread::spawn(move || {
            let (lease, duplicate) = publish_session(&route, duplicate);
            let mut duplicate = duplicate.expect("duplicate session returned for cleanup");
            cleanup_started.send(()).unwrap();
            cleanup_released.recv().unwrap();
            duplicate.shutdown("test duplicate cleanup complete");
            lease
        });
        cleanup_ready.recv().unwrap();

        let (lookup_finished, lookup_result) = std::sync::mpsc::channel();
        let lookup = std::thread::spawn(move || {
            lookup_finished
                .send(lookup_forward("lookup-during-cleanup"))
                .unwrap();
        });
        let result = lookup_result.recv_timeout(Duration::from_secs(1));
        release_cleanup.send(()).unwrap();
        lookup.join().unwrap();
        let duplicate_lease = publisher.join().unwrap();

        assert!(result.unwrap().is_none());
        drop(duplicate_lease);
        drop(existing);
    }

    #[test]
    #[serial_test::serial]
    fn cancellation_after_forward_publication_rolls_back_only_new_resources() {
        let route = ssh_route("shared-session");
        let mut new_session = test_session(&route, 1);
        let shared_session = new_session.session();
        let session_publication = new_session.publication.as_ref().unwrap().1;
        let existing_stop = Arc::new(AtomicBool::new(false));
        RUNTIME.lock().unwrap().forwards.insert(
            "existing".into(),
            ForwardState {
                publication_id: uuid::Uuid::new_v4(),
                bastion_ids: route.bastion_ids.clone(),
                session_key: route.session_key.clone(),
                endpoint: LocalEndpoint {
                    host: "127.0.0.1".into(),
                    port: 1,
                },
                stop: existing_stop.clone(),
                worker: None,
            },
        );
        new_session.commit();

        {
            let mut runtime = RUNTIME.lock().unwrap();
            runtime
                .sessions
                .get_mut(&route.session_key)
                .unwrap()
                .pending_leases += 1;
        }
        let cancelled_lease = SessionLease::new(
            shared_session,
            route.session_key.clone(),
            session_publication,
        );
        let cancelled_stop = Arc::new(AtomicBool::new(false));
        let error = match publish_forward(
            "cancelled",
            route.clone(),
            LocalEndpoint {
                host: "127.0.0.1".into(),
                port: 2,
            },
            cancelled_stop.clone(),
            stopped_worker(cancelled_stop.clone()),
            cancelled_lease,
            Arc::new(|| Err("cancelled at publication".into())),
        ) {
            Err(error) => error,
            Ok(_) => panic!("cancelled publication succeeded"),
        };
        assert_eq!(error, "cancelled at publication");

        let runtime = RUNTIME.lock().unwrap();
        assert!(runtime.forwards.contains_key("existing"));
        assert!(!runtime.forwards.contains_key("cancelled"));
        assert!(runtime.sessions.contains_key(&route.session_key));
        assert!(cancelled_stop.load(Ordering::SeqCst));
        assert!(!existing_stop.load(Ordering::SeqCst));
        drop(runtime);
        drop_connection("existing");
    }
}
