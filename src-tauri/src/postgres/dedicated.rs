//! Canonical dedicated tokio-postgres socket used by Query Session, Table
//! Browse, and Result Mutation. TLS comes from `super::tls` (ADR-0025).

use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{
    future::{poll_fn, BoxFuture, Shared},
    FutureExt,
};
use tokio::sync::mpsc;
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::{AsyncMessage, Client, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::connect_error::{self, ConnectFailure};
use super::connect_spec::ResolvedPostgresConnectSpec;
use super::options::driver_option_sql;
use super::tls;
use crate::TlsFailureKind;

#[derive(Debug)]
pub(crate) struct Notice {
    pub severity: String,
    pub message: String,
}

pub(crate) enum NoticeSink {
    Ignore,
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
    pub cancel: tokio_postgres::CancelToken,
    pub tls: TlsConfig,
    _driver: DriverTask,
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
}

/// A bounded operation can retain joins even when a connect future is dropped
/// during session setup. Existing socket owners may continue to use `connect`.
#[derive(Clone, Default)]
pub(crate) struct DriverJoins(Arc<std::sync::Mutex<DriverJoinState>>);

impl DriverJoins {
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
        let joins = std::mem::take(&mut self.0.lock().unwrap().drivers);
        for join in joins {
            join.join.await;
        }
    }

    fn track(&self, abort: tokio::task::AbortHandle, join: DriverJoin) {
        let mut state = self.0.lock().unwrap();
        if state.aborted {
            abort.abort();
        }
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
            _driver,
        } = self;
        drop(client);
        drop(cancel);
        drop(tls);
        _driver.join().await;
    }
}

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
    let tls_config = tls::client_config(&spec.tls).map_err(|error| DedicatedError::Tls {
        kind: TlsFailureKind::InvalidLocalMaterial,
        message: error.to_string(),
    })?;
    let mut config = spec.tokio_config();
    config.ssl_mode(tls::tokio_ssl_mode(spec.tls.mode));
    if spec.tls.server_name_differs_from(&spec.host) {
        // `host` is the certificate name; the socket must still reach the
        // real (tunnel) endpoint.
        let addr = with_deadline(spec, resolve_host(&spec.host, spec.port)).await?;
        config.hostaddr(addr);
    }
    let client_cert = spec.tls.client_auth_configured();
    let (client, driver) = match &tls_config {
        Some(tls_config) => {
            let tls = MakeRustlsConnect::new(rustls::ClientConfig::clone(tls_config));
            let connect = config.connect(tls);
            let (client, connection) = with_deadline(spec, async {
                connect.await.map_err(|error| classify(&error, client_cert))
            })
            .await?;
            let driver = spawn_driver(connection, notices);
            (client, driver)
        }
        None => {
            let connect = config.connect(NoTls);
            let (client, connection) = with_deadline(spec, async {
                connect.await.map_err(|error| classify(&error, client_cert))
            })
            .await?;
            let driver = spawn_driver(connection, notices);
            (client, driver)
        }
    };
    // From this point every early return aborts the socket driver instead of
    // detaching it while post-connect session options are applied.
    let driver = DriverTask::new(driver, tracked);
    let statements = driver_option_sql(&spec.driver_options, spec.safety_policy.read_only);
    if !statements.is_empty() {
        client
            .batch_execute(&statements.join("; "))
            .await
            .map_err(database_error)?;
    }
    let cancel = client.cancel_token();
    Ok(DedicatedConnection {
        client: Arc::new(client),
        cancel,
        tls: tls_config,
        _driver: driver,
    })
}

async fn resolve_host(host: &str, port: u16) -> Result<IpAddr, DedicatedError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| DedicatedError::ConnectionLost)?
        .next()
        .map(|addr| addr.ip())
        .ok_or(DedicatedError::ConnectionLost)
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
) -> tokio::task::JoinHandle<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio_postgres::tls::TlsStream + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(Ok(message)) = poll_fn(|cx| connection.poll_message(cx)).await {
            if let AsyncMessage::Notice(notice) = message {
                match &notices {
                    NoticeSink::Ignore => {}
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

pub(crate) async fn cancel(cancel: tokio_postgres::CancelToken, tls: TlsConfig) -> bool {
    let future = async move {
        match tls {
            Some(config) => {
                cancel
                    .cancel_query(MakeRustlsConnect::new(rustls::ClientConfig::clone(&config)))
                    .await
            }
            None => cancel.cancel_query(NoTls).await,
        }
    };
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .is_ok_and(|result| result.is_ok())
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
