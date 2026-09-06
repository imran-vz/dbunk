use std::{
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD_NO_PAD as B64_NO_PAD, Engine as _};
use ssh2::{HashType, Session};

use super::{
    forwarding::{connected_tcp_pair, spawn_channel_bridge, BridgeHandle},
    route::SshRoute,
    DEFAULT_SSH_PORT, SSH_CONNECT_TIMEOUT,
};
use crate::{BastionAuthMethod, BastionServer};

mod proxy_io;

pub(super) struct ResolvedBastion {
    pub(super) server: BastionServer,
    pub(super) password: Option<String>,
    pub(super) private_key_content: Option<String>,
    pub(super) passphrase: Option<String>,
}

struct RouteHop {
    session: Session,
    bridge_to_next: BridgeHandle,
}

pub(super) struct RouteSession {
    final_session: Session,
    keepalive_stop: Arc<AtomicBool>,
    intermediate_hops: Vec<RouteHop>,
    proxy_command: Option<ProxyCommandHandle>,
    keepalive_workers: Vec<thread::JoinHandle<()>>,
    closed: bool,
}

impl RouteSession {
    pub(super) fn session(&self) -> Session {
        self.final_session.clone()
    }

    pub(super) fn shutdown(&mut self, reason: &str) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.keepalive_stop.store(true, Ordering::SeqCst);
        for worker in &self.keepalive_workers {
            worker.thread().unpark();
        }
        let _ = self.final_session.disconnect(None, reason, None);
        for hop in self.intermediate_hops.iter_mut().rev() {
            hop.bridge_to_next.shutdown();
            let _ = hop.session.disconnect(None, reason, None);
        }
        if let Some(proxy_command) = &mut self.proxy_command {
            proxy_command.shutdown();
        }
        for worker in self.keepalive_workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for RouteSession {
    fn drop(&mut self) {
        self.shutdown("dbunk ssh route session dropped");
    }
}

#[cfg(test)]
pub(super) fn disconnected_route_session() -> RouteSession {
    RouteSession {
        final_session: Session::new().expect("test SSH session"),
        keepalive_stop: Arc::new(AtomicBool::new(false)),
        intermediate_hops: Vec::new(),
        proxy_command: None,
        keepalive_workers: Vec::new(),
        closed: false,
    }
}

pub(super) fn connect_route_session(
    bastions: &[ResolvedBastion],
    route: &SshRoute,
) -> Result<(RouteSession, Vec<(String, String)>), String> {
    connect_route_session_checked(bastions, route, &|| Ok(()))
}

pub(super) fn connect_route_session_checked(
    bastions: &[ResolvedBastion],
    route: &SshRoute,
    check: &(dyn Fn() -> Result<(), String> + Send + Sync),
) -> Result<(RouteSession, Vec<(String, String)>), String> {
    check()?;
    let Some(first) = bastions.first() else {
        return Err("SSH tunnel route has no Bastion Servers".to_string());
    };
    let mut accepted_fingerprints = Vec::new();
    let mut intermediate_hops = Vec::new();
    let mut proxy_command = None;

    let first_port = defaulted_ssh_port(first.server.port);
    let first_stream = if let Some(command) = route.proxy_command.as_deref() {
        let transport = connect_proxy_command(command, &first.server.host, first_port)?;
        proxy_command = Some(transport.handle);
        transport.stream
    } else {
        connect_tcp_with_timeout(&first.server.host, first_port)?
    };
    check()?;
    let mut session =
        connect_bastion_session(first, first_stream, route, &mut accepted_fingerprints)?;

    for bastion in bastions.iter().skip(1) {
        check()?;
        let port = defaulted_ssh_port(bastion.server.port);
        let (stream, handle) =
            spawn_channel_bridge(session.clone(), bastion.server.host.clone(), port, check)?;
        intermediate_hops.push(RouteHop {
            session,
            bridge_to_next: handle,
        });
        session = connect_bastion_session(bastion, stream, route, &mut accepted_fingerprints)?;
    }

    check()?;
    let route_session = route_session_for_route(session, route, intermediate_hops, proxy_command);
    Ok((route_session, accepted_fingerprints))
}

fn connect_bastion_session(
    bastion: &ResolvedBastion,
    tcp: TcpStream,
    route: &SshRoute,
    accepted_fingerprints: &mut Vec<(String, String)>,
) -> Result<Session, String> {
    let port = defaulted_ssh_port(bastion.server.port);
    let mut session = Session::new().map_err(|error| error.to_string())?;
    session.set_compress(route.compression);
    session.set_timeout(SSH_CONNECT_TIMEOUT.as_millis() as u32);
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|error| format!("SSH handshake failed: {error}"))?;

    let fingerprint = host_key_fingerprint(&session)?;
    match bastion.server.host_key_fingerprint.as_deref() {
        Some(expected) if expected == fingerprint => {}
        Some(expected) => {
            return Err(format!(
                "SSH host key mismatch for {}:{}. Expected {expected}, got {fingerprint}. Reset host-key trust before reconnecting if this change is expected.",
                bastion.server.host, port
            ));
        }
        None => {
            accepted_fingerprints.push((bastion.server.id.clone(), fingerprint));
        }
    };

    authenticate(&session, bastion)?;
    if let Some(interval) = route.keepalive_interval_seconds {
        session.set_keepalive(route.keepalive_want_reply, interval);
    }
    session.set_blocking(false);
    Ok(session)
}

fn defaulted_ssh_port(port: u16) -> u16 {
    if port == 0 {
        DEFAULT_SSH_PORT
    } else {
        port
    }
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

struct ProxyCommandTransport {
    stream: TcpStream,
    handle: ProxyCommandHandle,
}

struct ProxyCommandHandle {
    child: Arc<Mutex<Child>>,
    stop: Arc<AtomicBool>,
    bridge: TcpStream,
    workers: Vec<thread::JoinHandle<()>>,
}

impl ProxyCommandHandle {
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.bridge.shutdown(std::net::Shutdown::Both);
        for worker in &self.workers {
            worker.thread().unpark();
        }
        if let Ok(mut child) = self.child.lock() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                Err(error) => log::warn!("SSH proxy command status check failed: {error}"),
            }
        }
        proxy_io::join_workers(&mut self.workers);
    }
}

impl Drop for ProxyCommandHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn connect_proxy_command(
    command: &str,
    host: &str,
    port: u16,
) -> Result<ProxyCommandTransport, String> {
    let expanded_command = expand_proxy_command(command, host, port);
    let (client, bridge) = connected_tcp_pair()?;
    // Clone before spawning the child so setup failures cannot leak a process.
    let mut to_child = bridge
        .try_clone()
        .map_err(|error| format!("Failed to clone SSH proxy bridge: {error}"))?;
    let mut from_child = bridge
        .try_clone()
        .map_err(|error| format!("Failed to clone SSH proxy bridge: {error}"))?;
    let mut child_command = proxy_shell_command(&expanded_command);
    let child = child_command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Failed to start SSH proxy command: {error}"))?;

    // Own the child immediately, including every fallible setup path below.
    let mut handle = ProxyCommandHandle {
        child: Arc::new(Mutex::new(child)),
        stop: Arc::new(AtomicBool::new(false)),
        bridge,
        workers: Vec::new(),
    };
    let mut child = handle
        .child
        .lock()
        .map_err(|error| format!("Failed to access SSH proxy command: {error}"))?;
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "SSH proxy command stdin is unavailable".to_string())?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "SSH proxy command stdout is unavailable".to_string())?;
    drop(child);
    #[cfg(unix)]
    {
        proxy_io::set_nonblocking(&child_stdin)
            .and_then(|()| proxy_io::set_nonblocking(&child_stdout))
            .and_then(|()| to_child.set_nonblocking(true))
            .map_err(|error| format!("Failed to configure SSH proxy pipes: {error}"))?;
    }

    let stop = handle.stop.clone();
    handle.workers.push(thread::spawn(move || {
        let _ = proxy_io::copy(&mut to_child, &mut child_stdin, &stop);
    }));
    let stop = handle.stop.clone();
    handle.workers.push(thread::spawn(move || {
        let _ = proxy_io::copy(&mut child_stdout, &mut from_child, &stop);
    }));
    handle.workers.push(spawn_proxy_reaper(
        handle.child.clone(),
        handle.stop.clone(),
    ));

    Ok(ProxyCommandTransport {
        stream: client,
        handle,
    })
}

pub(super) fn expand_proxy_command(command: &str, host: &str, port: u16) -> String {
    let mut expanded = String::with_capacity(command.len() + host.len() + 8);
    let mut chars = command.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            expanded.push(ch);
            continue;
        }
        match chars.next() {
            Some('h') => expanded.push_str(host),
            Some('p') => expanded.push_str(&port.to_string()),
            Some('%') => expanded.push('%'),
            Some(other) => {
                expanded.push('%');
                expanded.push(other);
            }
            None => expanded.push('%'),
        }
    }
    expanded
}

fn proxy_shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut proxy = Command::new("cmd");
        proxy.arg("/C").arg(command);
        proxy
    }
    #[cfg(not(windows))]
    {
        let mut proxy = Command::new("sh");
        proxy.arg("-lc").arg(command);
        proxy
    }
}

fn spawn_proxy_reaper(child: Arc<Mutex<Child>>, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let status = {
                let Ok(mut child) = child.lock() else {
                    return;
                };
                child.try_wait()
            };
            match status {
                Ok(Some(status)) => {
                    if !status.success() {
                        log::warn!("SSH proxy command exited with status {status}");
                    }
                    return;
                }
                Ok(None) => thread::park_timeout(Duration::from_millis(100)),
                Err(error) => {
                    log::warn!("SSH proxy command wait failed: {error}");
                    return;
                }
            }
        }
    })
}

fn route_session_for_route(
    final_session: Session,
    route: &SshRoute,
    intermediate_hops: Vec<RouteHop>,
    proxy_command: Option<ProxyCommandHandle>,
) -> RouteSession {
    let keepalive_stop = Arc::new(AtomicBool::new(false));
    let mut keepalive_workers = Vec::new();
    if let Some(interval) = route.keepalive_interval_seconds {
        keepalive_workers.push(spawn_keepalive_loop(
            final_session.clone(),
            interval,
            keepalive_stop.clone(),
        ));
        for hop in &intermediate_hops {
            keepalive_workers.push(spawn_keepalive_loop(
                hop.session.clone(),
                interval,
                keepalive_stop.clone(),
            ));
        }
    }
    RouteSession {
        final_session,
        keepalive_stop,
        intermediate_hops,
        proxy_command,
        keepalive_workers,
        closed: false,
    }
}

fn spawn_keepalive_loop(
    session: Session,
    interval_seconds: u32,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut sleep_for = Duration::from_secs(u64::from(interval_seconds));
        while !stop.load(Ordering::SeqCst) {
            thread::park_timeout(sleep_for);
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match session.keepalive_send() {
                Ok(next_seconds) => {
                    sleep_for = Duration::from_secs(u64::from(next_seconds.max(1)));
                }
                Err(error) => {
                    log::warn!("SSH keepalive failed: {error}");
                    break;
                }
            }
        }
    })
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

#[cfg(test)]
mod tests {
    use super::expand_proxy_command;

    #[cfg(unix)]
    use {
        super::{connect_proxy_command, ProxyCommandTransport},
        std::{
            io::{Read, Write},
            time::{Duration, Instant},
        },
    };

    #[cfg(unix)]
    fn proxy_with_pipe_holding_descendant() -> ProxyCommandTransport {
        // FD 3 preserves stdin because shells otherwise give background jobs
        // /dev/null. The finite sleep also bounds failures of this regression.
        let mut transport = connect_proxy_command(
            "exec 3<&0; sleep 3 <&3 & printf 'ready\\n'; wait",
            "unused",
            22,
        )
        .expect("start local proxy command");
        transport
            .stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut ready = [0; 6];
        transport.stream.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready\n");
        transport
    }

    #[cfg(unix)]
    fn assert_proxy_shutdown_completes(transport: &mut ProxyCommandTransport) {
        let start = Instant::now();
        transport.handle.shutdown();
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "proxy cleanup waited for a descendant holding the pipes: {:?}",
            start.elapsed()
        );
        assert!(transport.handle.workers.is_empty());
        assert!(transport
            .handle
            .child
            .lock()
            .unwrap()
            .try_wait()
            .unwrap()
            .is_some());
    }

    #[cfg(unix)]
    #[test]
    fn proxy_shutdown_cancels_read_when_descendant_keeps_stdout_open() {
        let mut transport = proxy_with_pipe_holding_descendant();
        assert_proxy_shutdown_completes(&mut transport);
    }

    #[cfg(unix)]
    #[test]
    fn proxy_shutdown_cancels_backpressured_write_to_descendant_stdin() {
        let mut transport = proxy_with_pipe_holding_descendant();
        transport.stream.set_nonblocking(true).unwrap();
        let buffer = [1; 65536];
        let mut sent = 0;
        loop {
            match transport.stream.write(&buffer) {
                Ok(count) => sent += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("fill proxy input: {error}"),
            }
            assert!(sent < 64 * 1024 * 1024, "proxy input did not backpressure");
        }
        assert!(sent > 0);
        // Allow the worker to reach the full child pipe before cancellation.
        std::thread::sleep(Duration::from_millis(100));
        assert_proxy_shutdown_completes(&mut transport);
    }

    #[test]
    fn proxy_command_expands_host_port_and_percent_escape() {
        assert_eq!(
            expand_proxy_command("ssh -W %h:%p edge && echo %%", "bastion.local", 2222),
            "ssh -W bastion.local:2222 edge && echo %"
        );
    }
}
