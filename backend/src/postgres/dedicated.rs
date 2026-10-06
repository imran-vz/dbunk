//! Canonical dedicated tokio-postgres socket used by Query Session, Table
//! Browse, and Result Mutation. TLS comes from `super::tls` (ADR-0025).

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{
    future::{poll_fn, BoxFuture, Shared},
    FutureExt,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::tls::MakeTlsConnect;
use tokio_postgres::{AsyncMessage, Client, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::connect_error::{self, ConnectFailure};
use super::connect_spec::ResolvedPostgresConnectSpec;
use super::options::driver_option_sql;
use super::tls;
use crate::{ConnectionPooling, TlsFailureKind};

mod pooling;
pub(crate) use pooling::known as known_pooling;
pub(crate) mod reported;
pub(crate) mod wire;
use reported::ReportedParameters;
use wire::{Observed, ObservedTls, WireState};

#[derive(Debug)]
pub(crate) struct Notice {
    pub severity: String,
    pub message: String,
}

pub(crate) enum NoticeSink {
    Ignore,
    /// Byte- and count-bounded maintenance diagnostics. Truncate borrowed wire
    /// text before cloning; failed queue admission allocates no owned strings.
    #[cfg(feature = "isolated-profile")]
    Capped {
        tx: mpsc::Sender<Notice>,
        dropped: Arc<AtomicU32>,
        message_bytes: usize,
        severity_bytes: usize,
    },
    Bounded {
        tx: mpsc::Sender<Notice>,
        dropped: Arc<AtomicU32>,
    },
}

#[derive(Debug)]
pub(crate) enum DedicatedError {
    ConnectionLost,
    Timeout {
        operation: String,
    },
    /// TLS material or handshake failure at connect time. Distinguished
    /// from `ConnectionLost` so the actors can surface it as `tlsFailed`.
    Tls {
        kind: TlsFailureKind,
        message: String,
    },
    Database {
        code: Option<String>,
        message: String,
        severity: Option<String>,
        position: Option<u32>,
    },
}

/// The rustls config a live socket was opened with; cancel requests reuse
/// it so a verified session never cancels over an unverified one.
pub(crate) type TlsConfig = Option<Arc<rustls::ClientConfig>>;

pub(crate) struct DedicatedConnection {
    pub client: Arc<Client>,
    pub cancel: CancelHandle,
    pub tls: TlsConfig,
    /// GUC_REPORT values as last applied by this socket's driver.
    pub reported: Arc<ReportedParameters>,
    /// Transaction status and cancel identity as sent by the server.
    pub wire: Arc<WireState>,
    pub pooling: ConnectionPooling,
    _driver: DriverTask,
}

/// The socket is opened by dbunk, not tokio-postgres, so a cancel request
/// carries its own address: the same endpoint and TLS name as the session.
#[derive(Clone)]
pub(crate) struct CancelHandle {
    token: tokio_postgres::CancelToken,
    host: String,
    port: u16,
    server_name: String,
}

type DriverJoin = Shared<BoxFuture<'static, ()>>;

struct TrackedDriver {
    abort: tokio::task::AbortHandle,
    join: DriverJoin,
}

#[derive(Default)]
struct DriverJoinState {
    aborted: bool,
    drivers: Vec<TrackedDriver>,
    parent: Option<DriverJoins>,
}

/// A bounded operation can retain joins even when a connect future is dropped
/// during session setup. Existing socket owners may continue to use `connect`.
#[derive(Clone, Default)]
pub(crate) struct DriverJoins(Arc<std::sync::Mutex<DriverJoinState>>);

impl DriverJoins {
    /// A local cleanup barrier whose drivers also remain owned by the host.
    /// Cancelling the local operation cannot remove the host's join handles.
    #[cfg(feature = "isolated-profile")]
    pub(crate) fn child(&self) -> Self {
        Self(Arc::new(std::sync::Mutex::new(DriverJoinState {
            parent: Some(self.clone()),
            ..Default::default()
        })))
    }

    /// Aborts every tracked driver. The abort is latched so a driver registered
    /// concurrently after cleanup starts cannot escape the owning job's fence.
    pub(crate) fn abort_all(&self) {
        let mut state = self.0.lock().unwrap();
        state.aborted = true;
        for driver in &state.drivers {
            driver.abort.abort();
        }
    }

    pub(crate) async fn drain(&self) {
        loop {
            // Keep joins registered while awaiting: cancelling a graceful drain
            // must leave abort-and-join able to establish actual termination.
            let joins = self
                .0
                .lock()
                .unwrap()
                .drivers
                .iter()
                .map(|driver| driver.join.clone())
                .collect::<Vec<_>>();
            if joins.is_empty() {
                return;
            }
            for join in joins {
                join.await;
            }
            self.0
                .lock()
                .unwrap()
                .drivers
                .retain(|driver| driver.join.peek().is_none());
        }
    }

    pub(crate) fn track_task(&self, task: tokio::task::JoinHandle<()>) {
        let abort = task.abort_handle();
        let join = async move {
            let _ = task.await;
        }
        .boxed()
        .shared();
        self.track(abort, join);
    }

    fn track(&self, abort: tokio::task::AbortHandle, join: DriverJoin) {
        let mut state = self.0.lock().unwrap();
        if state.aborted {
            abort.abort();
        }
        if let Some(parent) = &state.parent {
            parent.track(abort.clone(), join.clone());
        }
        // A finished Tokio task still has an unobserved JoinHandle. Poll the
        // shared join before pruning it; is_finished alone is not a join.
        state
            .drivers
            .retain(|driver| driver.join.clone().now_or_never().is_none());
        state.drivers.push(TrackedDriver { abort, join });
    }
}

struct DriverTask {
    abort: tokio::task::AbortHandle,
    join: DriverJoin,
}

impl DriverTask {
    fn new(task: tokio::task::JoinHandle<()>, tracked: Option<&DriverJoins>) -> Self {
        let abort = task.abort_handle();
        let join = async move {
            let _ = task.await;
        }
        .boxed()
        .shared();
        if let Some(tracked) = tracked {
            tracked.track(abort.clone(), join.clone());
        }
        Self { abort, join }
    }
}

impl DriverTask {
    async fn join(self) {
        if tokio::time::timeout(Duration::from_secs(2), self.join.clone())
            .await
            .is_err()
        {
            self.abort.abort();
            self.join.clone().await;
        }
    }
}

impl Drop for DriverTask {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

impl DedicatedConnection {
    pub(crate) fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    /// Drops the last owned client before joining its socket driver. If the
    /// connection owner itself is aborted, `DriverTask` aborts the driver so a
    /// detached PostgreSQL socket cannot outlive the owning job.
    pub(crate) async fn close(self) {
        let Self {
            client,
            cancel,
            tls,
            reported,
            wire,
            pooling: _,
            _driver,
        } = self;
        drop(wire);
        drop(reported);
        drop(client);
        drop(cancel);
        drop(tls);
        _driver.join().await;
    }
}

#[cfg(test)]
pub(crate) async fn connect(
    spec: &ResolvedPostgresConnectSpec,
    notices: NoticeSink,
) -> Result<DedicatedConnection, DedicatedError> {
    connect_tracked(spec, notices, None).await
}

pub(crate) async fn connect_tracked(
    spec: &ResolvedPostgresConnectSpec,
    notices: NoticeSink,
    tracked: Option<&DriverJoins>,
) -> Result<DedicatedConnection, DedicatedError> {
    let mut connection = open(spec, notices, tracked).await?;
    connection.pooling = detect_pooling(spec, &connection.client, &connection.wire).await;
    // Behind a transaction pooler a session SET lands on whichever server
    // process ran it and stays there for other clients, so none is sent.
    let statements = if connection.pooling.pools_transactions() {
        Vec::new()
    } else {
        driver_option_sql(&spec.driver_options, spec.safety_policy.read_only)
    };
    if !statements.is_empty() {
        connection
            .client
            .batch_execute(&statements.join("; "))
            .await
            .map_err(database_error)?;
    }
    Ok(connection)
}

/// Classifies a started session whose protocol `wire` observes.
pub(crate) async fn detect_pooling(
    spec: &ResolvedPostgresConnectSpec,
    client: &Client,
    wire: &WireState,
) -> ConnectionPooling {
    pooling::classify(spec, client, wire).await
}

/// A started session with no settings applied. The socket is dbunk's own so
/// the protocol can be observed under any TLS mode.
async fn open(
    spec: &ResolvedPostgresConnectSpec,
    notices: NoticeSink,
    tracked: Option<&DriverJoins>,
) -> Result<DedicatedConnection, DedicatedError> {
    let tls_config = tls::client_config(&spec.tls).map_err(|error| DedicatedError::Tls {
        kind: TlsFailureKind::InvalidLocalMaterial,
        message: error.to_string(),
    })?;
    let mut config = spec.tokio_config();
    config.ssl_mode(tls::tokio_ssl_mode(spec.tls.mode));
    let client_cert = spec.tls.client_auth_configured();
    let reported = ReportedParameters::new();
    let wire = WireState::new();
    let (client, driver) = with_deadline(spec, async {
        // `host` is the endpoint (possibly a tunnel); the TLS server name is
        // the certificate name.
        let socket = open_socket(&spec.host, spec.port, spec.keepalive).await?;
        let socket = Observed::socket(socket, wire.clone());
        match &tls_config {
            Some(tls_config) => {
                let mut make = MakeRustlsConnect::new(rustls::ClientConfig::clone(tls_config));
                let Ok(connector) = MakeTlsConnect::<Observed<TcpStream>>::make_tls_connect(
                    &mut make,
                    &spec.tls.server_name,
                );
                let (client, connection) = config
                    .connect_raw(socket, ObservedTls::new(connector, wire.clone()))
                    .await
                    .map_err(|error| classify(&error, client_cert))?;
                Ok((client, spawn_driver(connection, notices, reported.clone())))
            }
            None => {
                let (client, connection) = config
                    .connect_raw(socket, NoTls)
                    .await
                    .map_err(|error| classify(&error, client_cert))?;
                Ok((client, spawn_driver(connection, notices, reported.clone())))
            }
        }
    })
    .await?;
    // From this point every early return aborts the socket driver.
    let driver = DriverTask::new(driver, tracked);
    let cancel = CancelHandle {
        token: client.cancel_token(),
        host: spec.host.clone(),
        port: spec.port,
        server_name: spec.tls.server_name.clone(),
    };
    Ok(DedicatedConnection {
        client: Arc::new(client),
        cancel,
        tls: tls_config,
        reported,
        wire,
        pooling: ConnectionPooling::Direct,
        _driver: driver,
    })
}

/// Tries each resolved address in order, as tokio-postgres does.
async fn open_socket(
    host: &str,
    port: u16,
    keepalive: Option<Duration>,
) -> Result<TcpStream, DedicatedError> {
    let addresses: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| DedicatedError::ConnectionLost)?
            .collect(),
    };
    for address in addresses {
        let Ok(socket) = TcpStream::connect(address).await else {
            continue;
        };
        socket
            .set_nodelay(true)
            .map_err(|_| DedicatedError::ConnectionLost)?;
        if let Some(idle) = keepalive {
            socket2::SockRef::from(&socket)
                .set_tcp_keepalive(&socket2::TcpKeepalive::new().with_time(idle))
                .map_err(|_| DedicatedError::ConnectionLost)?;
        }
        return Ok(socket);
    }
    Err(DedicatedError::ConnectionLost)
}

/// Map a tokio-postgres connect error onto the dedicated error space.
/// Authentication and database errors keep their SQLSTATE; TLS failures
/// become `Tls`; everything socket-shaped stays `ConnectionLost`.
fn classify(error: &tokio_postgres::Error, client_cert_configured: bool) -> DedicatedError {
    let view = connect_error::view_of(error);
    match connect_error::classify(&view, client_cert_configured) {
        ConnectFailure::Tls(kind) => DedicatedError::Tls {
            kind,
            message: connect_error::tls_failure_message(kind, &view.message),
        },
        ConnectFailure::Authentication { message }
        | ConnectFailure::DatabaseMissing { message } => DedicatedError::Database {
            code: view.sqlstate,
            message,
            severity: view.db_severity,
            position: None,
        },
        ConnectFailure::Database {
            code,
            message,
            severity,
        } => DedicatedError::Database {
            code,
            message,
            severity,
            position: None,
        },
        ConnectFailure::Io(..) | ConnectFailure::Other(_) => DedicatedError::ConnectionLost,
    }
}

fn spawn_driver<S, T>(
    mut connection: tokio_postgres::Connection<S, T>,
    notices: NoticeSink,
    reported: Arc<ReportedParameters>,
) -> tokio::task::JoinHandle<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio_postgres::tls::TlsStream + Unpin + Send + 'static,
{
    // Startup ParameterStatus values are published before any client request.
    reported.enter();
    reported.leave(|name| connection.parameter(name));
    tokio::spawn(async move {
        while let Some(Ok(message)) = poll_fn(|cx| {
            reported.enter();
            let polled = connection.poll_message(cx);
            reported.leave(|name| connection.parameter(name));
            polled
        })
        .await
        {
            if let AsyncMessage::Notice(notice) = message {
                match &notices {
                    NoticeSink::Ignore => {}
                    #[cfg(feature = "isolated-profile")]
                    NoticeSink::Capped {
                        tx,
                        dropped,
                        message_bytes,
                        severity_bytes,
                    } => {
                        capped_notice(
                            tx,
                            dropped,
                            notice.severity(),
                            notice.message(),
                            *severity_bytes,
                            *message_bytes,
                        );
                    }
                    NoticeSink::Bounded { tx, dropped } => {
                        if tx
                            .try_send(Notice {
                                severity: notice.severity().to_string(),
                                message: notice.message().to_string(),
                            })
                            .is_err()
                        {
                            dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    })
}

#[cfg(feature = "isolated-profile")]
fn capped_notice(
    tx: &mpsc::Sender<Notice>,
    dropped: &AtomicU32,
    severity: &str,
    message: &str,
    severity_bytes: usize,
    message_bytes: usize,
) {
    fn prefix(value: &str, cap: usize) -> &str {
        let mut end = value.len().min(cap);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        &value[..end]
    }
    let permit = match tx.try_reserve() {
        Ok(permit) => permit,
        Err(_) => {
            let _ = dropped.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                Some(n.saturating_add(1))
            });
            return;
        }
    };
    let short_severity = prefix(severity, severity_bytes);
    let short_message = prefix(message, message_bytes);
    if short_severity.len() != severity.len() || short_message.len() != message.len() {
        let _ = dropped.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            Some(n.saturating_add(1))
        });
    }
    permit.send(Notice {
        severity: short_severity.into(),
        message: short_message.into(),
    });
}

async fn with_deadline<T>(
    spec: &ResolvedPostgresConnectSpec,
    future: impl std::future::Future<Output = Result<T, DedicatedError>>,
) -> Result<T, DedicatedError> {
    match spec.connect_timeout {
        Some(limit) => {
            tokio::time::timeout(limit, future)
                .await
                .map_err(|_| DedicatedError::Timeout {
                    operation: "connect".into(),
                })?
        }
        None => future.await,
    }
}

pub(crate) async fn cancel(cancel: CancelHandle, tls: TlsConfig) -> bool {
    let future = async move {
        let socket = open_socket(&cancel.host, cancel.port, None).await.ok()?;
        match tls {
            Some(config) => {
                let mut make = MakeRustlsConnect::new(rustls::ClientConfig::clone(&config));
                let Ok(connector) =
                    MakeTlsConnect::<TcpStream>::make_tls_connect(&mut make, &cancel.server_name);
                cancel.token.cancel_query_raw(socket, connector).await.ok()
            }
            None => cancel.token.cancel_query_raw(socket, NoTls).await.ok(),
        }
    };
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .is_ok_and(|result| result.is_some())
}

pub(crate) fn database_error(error: tokio_postgres::Error) -> DedicatedError {
    if let Some(db) = error.as_db_error() {
        DedicatedError::Database {
            code: Some(db.code().code().into()),
            message: db.message().into(),
            severity: Some(db.severity().into()),
            position: match db.position() {
                Some(ErrorPosition::Original(pos)) => Some(*pos),
                _ => None,
            },
        }
    } else {
        DedicatedError::ConnectionLost
    }
}

#[cfg(test)]
mod live {
    //! Live TLS behaviour of the dedicated driver against the
    //! `postgres-tls` fixture (real CA, CA-signed server cert, client cert
    //! role). Run with `pnpm db:postgres-tls` up:
    //! `cargo test -- --ignored dedicated_live`.

    use std::time::Duration;

    use super::*;
    use crate::postgres::tls::ResolvedTls;
    use crate::{PgDriverOptions, PgTlsMode};

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../infrastructure/test-db/postgres-tls/certs")
            .join(name)
    }

    fn spec(port: u16, tls: ResolvedTls) -> ResolvedPostgresConnectSpec {
        ResolvedPostgresConnectSpec {
            connection_id: format!("dedicated-live-{port}"),
            host: "127.0.0.1".into(),
            port,
            database: "dbunk_demo".into(),
            user: "dbunk".into(),
            password: "dbunk".into(),
            tls,
            connect_timeout: Some(Duration::from_secs(5)),
            keepalive: Some(Duration::from_secs(30)),
            driver_options: PgDriverOptions::default(),
            safety_policy: Default::default(),
        }
    }

    async fn session_is_encrypted(connection: &DedicatedConnection) -> bool {
        connection
            .client
            .query_one(
                "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
                &[],
            )
            .await
            .expect("pg_stat_ssl")
            .get(0)
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn dedicated_live_verify_full_with_ca_connects_and_cancels_over_tls() {
        let mut tls = ResolvedTls::with_mode("127.0.0.1", PgTlsMode::VerifyFull);
        tls.root_cert_path = Some(fixture("ca.crt"));
        let connection = connect(&spec(15433, tls), NoticeSink::Ignore)
            .await
            .expect("verify-full with the fixture CA");
        assert!(session_is_encrypted(&connection).await);
        assert!(
            connection.tls.is_some(),
            "cancel reuses the verified config"
        );
        let token = connection.cancel.clone();
        let config = connection.tls.clone();
        let query = connection.client.simple_query("SELECT pg_sleep(30)");
        let cancel = async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel(token, config).await
        };
        let (result, requested) = tokio::join!(query, cancel);
        assert!(requested);
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn dedicated_live_verify_ca_without_ca_is_untrusted() {
        let tls = ResolvedTls::with_mode("127.0.0.1", PgTlsMode::VerifyCa);
        let error = connect(&spec(15433, tls), NoticeSink::Ignore)
            .await
            .err()
            .expect("fixture CA is not in the platform store");
        assert!(
            matches!(
                error,
                DedicatedError::Tls {
                    kind: TlsFailureKind::CertificateUntrusted,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn dedicated_live_verify_full_wrong_server_name_is_mismatch() {
        let mut tls = ResolvedTls::with_mode("wrong.example", PgTlsMode::VerifyFull);
        tls.root_cert_path = Some(fixture("ca.crt"));
        let error = connect(&spec(15433, tls), NoticeSink::Ignore)
            .await
            .err()
            .expect("certificate names localhost, not wrong.example");
        assert!(
            matches!(
                error,
                DedicatedError::Tls {
                    kind: TlsFailureKind::HostnameMismatch,
                    ..
                }
            ),
            "{error:?}"
        );
        // verify-ca tolerates the same mismatch.
        let mut tls = ResolvedTls::with_mode("wrong.example", PgTlsMode::VerifyCa);
        tls.root_cert_path = Some(fixture("ca.crt"));
        connect(&spec(15433, tls), NoticeSink::Ignore)
            .await
            .expect("verify-ca ignores the name");
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn dedicated_live_client_certificate_authenticates() {
        let mut tls = ResolvedTls::with_mode("127.0.0.1", PgTlsMode::VerifyFull);
        tls.root_cert_path = Some(fixture("ca.crt"));
        tls.client_cert_path = Some(fixture("client.crt"));
        tls.client_key_path = Some(fixture("client.key"));
        let mut spec = spec(15433, tls);
        spec.user = "dbunk_cert".into();
        spec.password = String::new();
        let connection = connect(&spec, NoticeSink::Ignore)
            .await
            .expect("cert auth for dbunk_cert");
        let presented: bool = connection
            .client
            .query_one(
                "SELECT client_dn IS NOT NULL FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
                &[],
            )
            .await
            .expect("pg_stat_ssl")
            .get(0);
        assert!(presented);

        // Without the certificate the role cannot log in at all.
        let mut tls = ResolvedTls::with_mode("127.0.0.1", PgTlsMode::VerifyFull);
        tls.root_cert_path = Some(fixture("ca.crt"));
        let mut spec = super::super::connect_spec::ResolvedPostgresConnectSpec { tls, ..spec };
        spec.user = "dbunk_cert".into();
        let error = connect(&spec, NoticeSink::Ignore)
            .await
            .err()
            .expect("hostssl cert rule rejects the role without a certificate");
        assert!(
            matches!(
                error,
                DedicatedError::Database { .. } | DedicatedError::Tls { .. }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn dedicated_live_reported_parameters_follow_parameter_status_without_queries() {
        let connection = connect(
            &spec(15432, ResolvedTls::prefer("127.0.0.1")),
            NoticeSink::Ignore,
        )
        .await
        .expect("plaintext fixture");
        let start = connection.reported.settled().await.expect("settled");
        assert_eq!(start.value("client_encoding"), Some("UTF8"));
        assert!(start.value("DateStyle").is_some());
        // set_config inside a SELECT is reported mid-statement.
        connection
            .client
            .simple_query("SELECT set_config('DateStyle', 'German, DMY', false)")
            .await
            .unwrap();
        let changed = connection.reported.settled().await.expect("settled");
        assert_ne!(changed.generation, start.generation);
        assert_eq!(changed.value("DateStyle"), Some("German, DMY"));
        connection
            .client
            .batch_execute("SET search_path TO \"$user\", public")
            .await
            .unwrap();
        let path = connection.reported.settled().await.expect("settled");
        // Servers that GUC_REPORT search_path report the exact text; older
        // servers leave it unknown, which keeps unqualified targets refused.
        assert!(matches!(
            path.value("search_path"),
            None | Some("\"$user\", public")
        ));
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn dedicated_live_prefer_against_plaintext_server_downgrades() {
        let connection = connect(
            &spec(15432, ResolvedTls::prefer("127.0.0.1")),
            NoticeSink::Ignore,
        )
        .await
        .expect("prefer falls back to plaintext");
        assert!(!session_is_encrypted(&connection).await);
        assert!(
            connection.tls.is_some(),
            "prefer keeps its config for cancel"
        );

        let error = connect(
            &spec(
                15432,
                ResolvedTls::with_mode("127.0.0.1", PgTlsMode::Require),
            ),
            NoticeSink::Ignore,
        )
        .await
        .err()
        .expect("require refuses a plaintext-only server");
        assert!(
            matches!(
                error,
                DedicatedError::Tls {
                    kind: TlsFailureKind::ServerRefusedTls,
                    ..
                }
            ),
            "{error:?}"
        );
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[cfg(feature = "isolated-profile")]
    #[tokio::test]
    async fn maintenance_notices_bound_utf8_before_cloning_and_count_overflow() {
        let (tx, mut rx) = mpsc::channel(1);
        let dropped = AtomicU32::new(0);
        capped_notice(&tx, &dropped, "WARNING", "字字字", 4, 7);
        capped_notice(&tx, &dropped, "WARNING", "extra", 4, 7);
        let notice = rx.recv().await.unwrap();
        assert_eq!(notice.severity, "WARN");
        assert_eq!(notice.message, "字字");
        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        dropped.store(u32::MAX, Ordering::Relaxed);
        drop(rx);
        capped_notice(&tx, &dropped, "WARNING", "extra", 4, 7);
        assert_eq!(dropped.load(Ordering::Relaxed), u32::MAX);
    }

    async fn finished_but_unobserved(tracked: &DriverJoins) -> Arc<std::sync::atomic::AtomicBool> {
        let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = tokio::spawn(async {});
        let abort = task.abort_handle();
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
        let signal = observed.clone();
        let join = async move {
            task.await.unwrap();
            signal.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        .boxed()
        .shared();
        tracked.track(abort, join);
        observed
    }

    #[tokio::test]
    async fn pruning_and_concurrent_registration_observe_every_join() {
        let tracked = DriverJoins::default();
        let observed = finished_but_unobserved(&tracked).await;
        tracked.track_task(tokio::spawn(async {}));
        assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
        tracked.drain().await;

        let (release, pending) = tokio::sync::oneshot::channel();
        tracked.track_task(tokio::spawn(async {
            pending.await.unwrap();
        }));
        let mut drain = Box::pin(tracked.drain());
        assert!(futures_util::poll!(&mut drain).is_pending());
        let late = finished_but_unobserved(&tracked).await;
        release.send(()).unwrap();
        drain.await;
        assert!(late.load(std::sync::atomic::Ordering::SeqCst));
        assert!(tracked.0.lock().unwrap().drivers.is_empty());
    }

    fn pending_driver(
        tracked: &DriverJoins,
    ) -> (
        DriverTask,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (started, ready) = tokio::sync::oneshot::channel();
        let (stopped, done) = tokio::sync::oneshot::channel();
        struct Stopped(Option<tokio::sync::oneshot::Sender<()>>);
        impl Drop for Stopped {
            fn drop(&mut self) {
                let _ = self.0.take().unwrap().send(());
            }
        }
        let stopped = Stopped(Some(stopped));
        let task = tokio::spawn(async move {
            let _stopped = stopped;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        (DriverTask::new(task, Some(tracked)), ready, done)
    }

    #[tokio::test]
    async fn tracked_driver_join_survives_cancellation_of_its_connection_owner() {
        let tracked = DriverJoins::default();
        let (driver, ready, done) = pending_driver(&tracked);
        ready.await.unwrap();
        drop(driver); // the connection future was dropped during setup
        tracked.drain().await;
        done.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_graceful_drain_preserves_abort_and_join_ownership() {
        let tracked = DriverJoins::default();
        let (driver, ready, done) = pending_driver(&tracked);
        ready.await.unwrap();
        let mut drain = Box::pin(tracked.drain());
        assert!(futures_util::poll!(&mut drain).is_pending());
        drop(drain);
        tracked.abort_all();
        tracked.drain().await;
        done.await.unwrap();
        drop(driver);
    }

    #[tokio::test]
    async fn abort_all_stops_existing_and_late_registered_drivers_before_join_returns() {
        let tracked = DriverJoins::default();
        let (existing, existing_ready, existing_done) = pending_driver(&tracked);
        existing_ready.await.unwrap();

        tracked.abort_all();
        let (late, _late_ready, late_done) = pending_driver(&tracked);
        tokio::time::timeout(Duration::from_secs(1), tracked.drain())
            .await
            .expect("all aborted driver joins completed");

        existing_done.await.unwrap();
        late_done.await.unwrap();
        drop(existing);
        drop(late);
    }
}
