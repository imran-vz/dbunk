use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use ssh2::Session;

const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const PUMP_IDLE_SLEEP: Duration = Duration::from_millis(5);
const MAX_BUFFERED_BYTES: usize = 256 * 1024;

pub(super) struct BridgeHandle {
    stop: Arc<AtomicBool>,
}

impl BridgeHandle {
    pub(super) fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for BridgeHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub(super) fn spawn_forward_accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    session: Session,
    remote_host: String,
    remote_port: u16,
) {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, local_addr)) => {
                    let session = session.clone();
                    let remote_host = remote_host.clone();
                    thread::spawn(move || {
                        if let Err(error) = handle_forward_stream(
                            stream,
                            session,
                            &remote_host,
                            remote_port,
                            local_addr,
                        ) {
                            log::warn!("SSH forward stream closed with error: {error}");
                        }
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(PUMP_IDLE_SLEEP);
                }
                Err(error) => {
                    log::warn!("SSH tunnel listener stopped: {error}");
                    break;
                }
            }
        }
    });
}

pub(super) fn spawn_channel_bridge(
    session: Session,
    remote_host: String,
    remote_port: u16,
) -> Result<(TcpStream, BridgeHandle), String> {
    let (client, bridge) = connected_tcp_pair()?;
    let local_addr = bridge.local_addr().map_err(|error| error.to_string())?;
    let mut channel = open_direct_channel(&session, &remote_host, remote_port, local_addr)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    thread::spawn(move || {
        let mut bridge = bridge;
        if let Err(error) = bridge.set_nonblocking(true) {
            log::warn!("SSH jump bridge failed to switch to nonblocking mode: {error}");
            return;
        }
        if let Err(error) = pump_nonblocking(&mut bridge, &mut channel, Some(&thread_stop)) {
            log::warn!("SSH jump bridge closed with error: {error}");
        }
    });
    Ok((client, BridgeHandle { stop }))
}

pub(super) fn connected_tcp_pair() -> Result<(TcpStream, TcpStream), String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("Failed to bind local SSH bridge: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("Failed to inspect local SSH bridge: {error}"))?;
    let client = TcpStream::connect(address)
        .map_err(|error| format!("Failed to connect local SSH bridge: {error}"))?;
    let (server, _) = listener
        .accept()
        .map_err(|error| format!("Failed to accept local SSH bridge: {error}"))?;
    Ok((client, server))
}

fn handle_forward_stream(
    mut stream: TcpStream,
    session: Session,
    remote_host: &str,
    remote_port: u16,
    local_addr: SocketAddr,
) -> Result<(), String> {
    stream
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let mut channel = open_direct_channel(&session, remote_host, remote_port, local_addr)?;
    pump_nonblocking(&mut stream, &mut channel, None)
}

fn open_direct_channel(
    session: &Session,
    remote_host: &str,
    remote_port: u16,
    local_addr: SocketAddr,
) -> Result<ssh2::Channel, String> {
    let started = Instant::now();
    loop {
        match session.channel_direct_tcpip(
            remote_host,
            remote_port,
            Some(("127.0.0.1", local_addr.port())),
        ) {
            Ok(channel) => return Ok(channel),
            Err(error) if ssh_would_block(&error) => {
                if started.elapsed() > CHANNEL_OPEN_TIMEOUT {
                    return Err(format!(
                        "Timed out opening SSH direct-tcpip channel to {remote_host}:{remote_port}"
                    ));
                }
                thread::sleep(PUMP_IDLE_SLEEP);
            }
            Err(error) => {
                return Err(format!(
                    "Failed to open SSH direct-tcpip channel to {remote_host}:{remote_port}: {error}"
                ));
            }
        }
    }
}

fn ssh_would_block(error: &ssh2::Error) -> bool {
    let io_error = std::io::Error::from(ssh2::Error::new(error.code(), ""));
    io_error.kind() == std::io::ErrorKind::WouldBlock
}

fn pump_nonblocking(
    stream: &mut TcpStream,
    channel: &mut ssh2::Channel,
    stop: Option<&AtomicBool>,
) -> Result<(), String> {
    let mut to_channel = Vec::<u8>::new();
    let mut to_stream = Vec::<u8>::new();
    let mut stream_eof = false;
    let mut channel_eof = false;
    let mut sent_channel_eof = false;
    let mut temp = [0_u8; 16 * 1024];

    loop {
        if stop.is_some_and(|stop| stop.load(Ordering::SeqCst)) {
            let _ = channel.close();
            return Ok(());
        }
        let mut progressed = false;

        if !stream_eof && to_channel.len() < MAX_BUFFERED_BYTES {
            match stream.read(&mut temp) {
                Ok(0) => {
                    stream_eof = true;
                    progressed = true;
                }
                Ok(n) => {
                    to_channel.extend_from_slice(&temp[..n]);
                    progressed = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
        }

        if !channel_eof && to_stream.len() < MAX_BUFFERED_BYTES {
            match channel.read(&mut temp) {
                Ok(0) => {
                    channel_eof = true;
                    progressed = true;
                }
                Ok(n) => {
                    to_stream.extend_from_slice(&temp[..n]);
                    progressed = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
        }

        if !to_channel.is_empty() {
            match channel.write(&to_channel) {
                Ok(0) => {}
                Ok(n) => {
                    to_channel.drain(..n);
                    progressed = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
        }

        if !to_stream.is_empty() {
            match stream.write(&to_stream) {
                Ok(0) => {}
                Ok(n) => {
                    to_stream.drain(..n);
                    progressed = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
        }

        if stream_eof && to_channel.is_empty() && !sent_channel_eof {
            match channel.send_eof() {
                Ok(()) => {
                    sent_channel_eof = true;
                    progressed = true;
                }
                Err(error) if ssh_would_block(&error) => {}
                Err(error) => return Err(error.to_string()),
            }
        }

        if channel_eof && to_stream.is_empty() {
            let _ = stream.shutdown(Shutdown::Write);
        }

        if stream_eof && channel_eof && to_channel.is_empty() && to_stream.is_empty() {
            let _ = channel.close();
            return Ok(());
        }

        if !progressed {
            thread::sleep(PUMP_IDLE_SLEEP);
        }
    }
}
