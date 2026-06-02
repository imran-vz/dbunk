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
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD_NO_PAD as B64_NO_PAD, Engine as _};
use once_cell::sync::Lazy;
use sqlx::SqlitePool;
use ssh2::{HashType, Session};

use crate::{
    credentials, storage, BastionAuthMethod, BastionServer, CredentialStorageMode,
    StoredConnection, TestBastionResult,
};

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

use endpoint::{remote_endpoint, rewrite_connection_endpoint};
use forwarding::spawn_forward_accept_loop;

struct ForwardState {
    bastion_id: String,
    endpoint: LocalEndpoint,
    stop: Arc<AtomicBool>,
}

struct Runtime {
    sessions: HashMap<String, Session>,
    forwards: HashMap<String, ForwardState>,
}

static RUNTIME: Lazy<Mutex<Runtime>> = Lazy::new(|| {
    Mutex::new(Runtime {
        sessions: HashMap::new(),
        forwards: HashMap::new(),
    })
});

struct ResolvedBastion {
    server: BastionServer,
    password: Option<String>,
    private_key_content: Option<String>,
    passphrase: Option<String>,
}

struct PasswordPrompter {
    password: String,
}

impl ssh2::KeyboardInteractivePrompt for PasswordPrompter {
    fn prompt<'a>(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[ssh2::Prompt<'a>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.password.clone()).collect()
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
            if forward.bastion_id == bastion_id {
                Some(key.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for route_key in route_keys {
        drop_forward_locked(&mut runtime, &route_key);
    }
    runtime.sessions.remove(bastion_id);
}

pub async fn test_bastion(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    bastion_id: &str,
) -> Result<TestBastionResult, String> {
    let started = Instant::now();
    let bastion = load_bastion(pool, mode, bastion_id).await?;
    let (session, accepted_fingerprint) =
        tokio::task::spawn_blocking(move || connect_bastion_session(&bastion))
            .await
            .map_err(|error| error.to_string())??;
    if let Some(fingerprint) = accepted_fingerprint {
        storage::bastions::update_bastion_host_key_fingerprint(
            pool,
            bastion_id,
            Some(&fingerprint),
        )
        .await?;
    }
    let _ = session.disconnect(None, "dbunk test complete", None);
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
    let bastion_id = config
        .bastion_server_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "SSH tunnel is enabled but no Bastion Server is selected".to_string())?
        .to_string();
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
    let session = ensure_session(pool, mode, &bastion_id).await?;
    spawn_forward_accept_loop(listener, stop.clone(), session, remote_host, remote_port);

    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    if let Some(existing) = runtime.forwards.get(route_key) {
        stop.store(true, Ordering::SeqCst);
        return Ok(existing.endpoint.clone());
    }
    runtime.forwards.insert(
        route_key.to_string(),
        ForwardState {
            bastion_id,
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
    bastion_id: &str,
) -> Result<Session, String> {
    if let Some(session) = RUNTIME
        .lock()
        .expect("ssh tunnel runtime poisoned")
        .sessions
        .get(bastion_id)
        .cloned()
    {
        return Ok(session);
    }

    let bastion = load_bastion(pool, mode, bastion_id).await?;
    let (session, accepted_fingerprint) =
        tokio::task::spawn_blocking(move || connect_bastion_session(&bastion))
            .await
            .map_err(|error| error.to_string())??;

    if let Some(fingerprint) = accepted_fingerprint {
        storage::bastions::update_bastion_host_key_fingerprint(
            pool,
            bastion_id,
            Some(&fingerprint),
        )
        .await?;
    }

    let mut runtime = RUNTIME.lock().expect("ssh tunnel runtime poisoned");
    if let Some(existing) = runtime.sessions.get(bastion_id) {
        let _ = session.disconnect(None, "dbunk duplicate session closed", None);
        return Ok(existing.clone());
    }
    runtime
        .sessions
        .insert(bastion_id.to_string(), session.clone());
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

fn connect_bastion_session(bastion: &ResolvedBastion) -> Result<(Session, Option<String>), String> {
    let port = if bastion.server.port == 0 {
        DEFAULT_SSH_PORT
    } else {
        bastion.server.port
    };
    let tcp = connect_tcp_with_timeout(&bastion.server.host, port)?;
    let mut session = Session::new().map_err(|error| error.to_string())?;
    session.set_timeout(SSH_CONNECT_TIMEOUT.as_millis() as u32);
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|error| format!("SSH handshake failed: {error}"))?;

    let fingerprint = host_key_fingerprint(&session)?;
    let accepted = match bastion.server.host_key_fingerprint.as_deref() {
        Some(expected) if expected == fingerprint => None,
        Some(expected) => {
            return Err(format!(
                "SSH host key mismatch for {}:{}. Expected {expected}, got {fingerprint}. Reset host-key trust before reconnecting if this change is expected.",
                bastion.server.host, port
            ));
        }
        None => Some(fingerprint),
    };

    authenticate(&session, bastion)?;
    session.set_blocking(false);
    Ok((session, accepted))
}

fn connect_tcp_with_timeout(host: &str, port: u16) -> Result<TcpStream, String> {
    let target = format!("{host}:{port}");
    let addresses = target
        .to_socket_addrs()
        .map_err(|error| format!("Could not resolve SSH host {target}: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(format!("Could not resolve SSH host {target}"));
    }
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, SSH_CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "Could not connect to SSH host {target}: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown network error".to_string())
    ))
}

fn host_key_fingerprint(session: &Session) -> Result<String, String> {
    let hash = session
        .host_key_hash(HashType::Sha256)
        .ok_or_else(|| "SSH server did not provide a host key".to_string())?;
    Ok(format!("SHA256:{}", B64_NO_PAD.encode(hash)))
}

fn authenticate(session: &Session, bastion: &ResolvedBastion) -> Result<(), String> {
    let user = bastion.server.user.as_str();
    match bastion.server.auth_method {
        BastionAuthMethod::Password => {
            let password = bastion
                .password
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Bastion password is missing".to_string())?;
            if session.userauth_password(user, password).is_err() {
                let mut prompter = PasswordPrompter {
                    password: password.to_string(),
                };
                session
                    .userauth_keyboard_interactive(user, &mut prompter)
                    .map_err(|error| format!("SSH password authentication failed: {error}"))?;
            }
        }
        BastionAuthMethod::PrivateKeyPath => {
            let path = bastion
                .server
                .private_key_path
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Bastion private key path is missing".to_string())?;
            session
                .userauth_pubkey_file(user, None, Path::new(path), bastion.passphrase.as_deref())
                .map_err(|error| format!("SSH private key authentication failed: {error}"))?;
        }
        BastionAuthMethod::PrivateKeyContent => {
            let key = bastion
                .private_key_content
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "Bastion private key content is missing".to_string())?;
            session
                .userauth_pubkey_memory(user, None, key, bastion.passphrase.as_deref())
                .map_err(|error| format!("SSH private key authentication failed: {error}"))?;
        }
    }
    if !session.authenticated() {
        return Err("SSH authentication did not complete".to_string());
    }
    Ok(())
}

fn drop_forward_locked(runtime: &mut Runtime, route_key: &str) {
    if let Some(forward) = runtime.forwards.remove(route_key) {
        forward.stop.store(true, Ordering::SeqCst);
    }
}

fn drop_unused_sessions_locked(runtime: &mut Runtime) {
    let active_bastions = runtime
        .forwards
        .values()
        .map(|forward| forward.bastion_id.clone())
        .collect::<std::collections::HashSet<_>>();
    runtime
        .sessions
        .retain(|bastion_id, _| active_bastions.contains(bastion_id));
}
