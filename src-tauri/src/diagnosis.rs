//! Staged connection diagnosis (ADR-0025, Plan 011).
//!
//! `test_connection` returns one string from a driver the query editor
//! does not use. This runs the connect **stepwise** — tunnel, DNS, TCP,
//! TLS, authentication, database — so every failure lands on the stage
//! that produced it, and reports the server's own view of encryption
//! (`pg_stat_ssl`) rather than what the selected mode implies.
//!
//! PostgreSQL only gets the full ladder. Other engines get the tunnel
//! stage and one `database` stage wrapping today's ping.

use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::net::TcpStream;
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::postgres::connect_error::{self, ConnectFailure};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::tls::{self, PoolHostnameVerification};
use crate::{
    dispatch, tunnel, CredentialStorageMode, DatabaseEngine, Environment, PgTlsMode,
    StoredConnection, TlsFailureKind,
};

const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnoseConnectionPayload {
    pub connection: StoredConnection,
    /// When set and the payload's password is empty, the stored credential
    /// for this connection id is hydrated backend-side (edit-mode test).
    #[serde(default)]
    pub hydrate_credential_from: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionDiagnosis {
    pub engine: DatabaseEngine,
    /// Fixed order: tunnel, dns, tcp, tls, authentication, database.
    pub stages: Vec<DiagnosisStage>,
    pub outcome: DiagnosisOutcome,
    pub warnings: Vec<DiagnosisWarning>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosisStage {
    pub stage: DiagnosisStageKind,
    pub result: StageResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosisStageKind {
    Tunnel,
    Dns,
    Tcp,
    Tls,
    Authentication,
    Database,
}

const STAGE_ORDER: [DiagnosisStageKind; 6] = [
    DiagnosisStageKind::Tunnel,
    DiagnosisStageKind::Dns,
    DiagnosisStageKind::Tcp,
    DiagnosisStageKind::Tls,
    DiagnosisStageKind::Authentication,
    DiagnosisStageKind::Database,
];

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum StageResult {
    Passed {
        elapsed_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<StageDetail>,
    },
    Failed {
        elapsed_ms: u64,
        kind: FailureKind,
        message: String,
    },
    Skipped {
        reason: SkipReason,
    },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum StageDetail {
    Tunnel {
        local_endpoint: String,
    },
    Dns {
        addresses: Vec<String>,
    },
    Tls {
        /// From `pg_stat_ssl` when the session got that far, else from the
        /// handshake outcome. The only honest source for `prefer`.
        encrypted: bool,
        protocol: Option<String>,
        cipher: Option<String>,
        certificate_verified: bool,
        hostname_verified: bool,
        client_certificate_presented: bool,
        pool_hostname_verification: PoolHostnameVerification,
    },
    Database {
        server_version: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FailureKind {
    TunnelFailed,
    DnsUnresolvable,
    ConnectionRefused,
    TimedOut,
    Unreachable,
    ServerRefusedTls,
    CertificateUntrusted,
    HostnameMismatch,
    ClientCertificateRejected,
    InvalidLocalMaterial,
    HandshakeFailed,
    AuthenticationFailed,
    DatabaseMissing,
    Other,
}

impl From<TlsFailureKind> for FailureKind {
    fn from(kind: TlsFailureKind) -> Self {
        match kind {
            TlsFailureKind::ServerRefusedTls => Self::ServerRefusedTls,
            TlsFailureKind::CertificateUntrusted => Self::CertificateUntrusted,
            TlsFailureKind::HostnameMismatch => Self::HostnameMismatch,
            TlsFailureKind::ClientCertificateRejected => Self::ClientCertificateRejected,
            TlsFailureKind::InvalidLocalMaterial => Self::InvalidLocalMaterial,
            TlsFailureKind::HandshakeFailed => Self::HandshakeFailed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SkipReason {
    NoTunnel,
    TlsDisabled,
    BlockedByEarlierFailure,
    NotApplicable,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum DiagnosisOutcome {
    Reachable { latency_ms: u64 },
    Failed { stage: DiagnosisStageKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosisWarning {
    NotEncrypted,
    PoolHostnameVerificationCaOnly,
    ProductionWithoutVerification,
}

// ---------------------------------------------------------------------------
// Report builder
// ---------------------------------------------------------------------------

struct Report {
    engine: DatabaseEngine,
    stages: Vec<DiagnosisStage>,
    warnings: Vec<DiagnosisWarning>,
    failed: Option<DiagnosisStageKind>,
    latency_ms: u64,
}

impl Report {
    fn new(engine: DatabaseEngine) -> Self {
        Self {
            engine,
            stages: Vec::with_capacity(STAGE_ORDER.len()),
            warnings: Vec::new(),
            failed: None,
            latency_ms: 0,
        }
    }

    fn push(&mut self, stage: DiagnosisStageKind, result: StageResult) {
        debug_assert_eq!(STAGE_ORDER[self.stages.len()], stage, "stage order");
        self.stages.push(DiagnosisStage { stage, result });
    }

    fn pass(&mut self, stage: DiagnosisStageKind, started: Instant, detail: Option<StageDetail>) {
        self.push(
            stage,
            StageResult::Passed {
                elapsed_ms: elapsed_ms(started),
                detail,
            },
        );
    }

    fn skip(&mut self, stage: DiagnosisStageKind, reason: SkipReason) {
        self.push(stage, StageResult::Skipped { reason });
    }

    /// Record a failure and mark every remaining stage as blocked.
    fn fail(
        &mut self,
        stage: DiagnosisStageKind,
        started: Instant,
        kind: FailureKind,
        message: String,
    ) {
        self.push(
            stage,
            StageResult::Failed {
                elapsed_ms: elapsed_ms(started),
                kind,
                message,
            },
        );
        self.failed = Some(stage);
        while self.stages.len() < STAGE_ORDER.len() {
            let next = STAGE_ORDER[self.stages.len()];
            self.push(
                next,
                StageResult::Skipped {
                    reason: SkipReason::BlockedByEarlierFailure,
                },
            );
        }
    }

    fn warn(&mut self, warning: DiagnosisWarning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn finish(self) -> ConnectionDiagnosis {
        debug_assert_eq!(self.stages.len(), STAGE_ORDER.len(), "every stage reported");
        let outcome = match self.failed {
            Some(stage) => DiagnosisOutcome::Failed { stage },
            None => DiagnosisOutcome::Reachable {
                latency_ms: self.latency_ms,
            },
        };
        ConnectionDiagnosis {
            engine: self.engine,
            stages: self.stages,
            outcome,
            warnings: self.warnings,
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Run the ladder. The outer `Err` is reserved for credential-store and
/// tunnel-validation failures; every probe failure lands in the report.
pub(crate) async fn run(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    connection: &StoredConnection,
) -> Result<ConnectionDiagnosis, String> {
    let mut report = Report::new(connection.engine());
    let route_key = format!("diag-{}", uuid::Uuid::new_v4());

    let tunnel_enabled = connection.ssh_tunnel().is_some_and(|config| config.enabled);
    let resolved = if tunnel_enabled {
        let started = Instant::now();
        match tunnel::resolve_connection(pool, mode, &route_key, connection).await {
            Ok(resolved) => {
                let local_endpoint = format!("{}:{}", resolved.host(), resolved.port());
                report.pass(
                    DiagnosisStageKind::Tunnel,
                    started,
                    Some(StageDetail::Tunnel { local_endpoint }),
                );
                resolved
            }
            Err(message) => {
                report.fail(
                    DiagnosisStageKind::Tunnel,
                    started,
                    FailureKind::TunnelFailed,
                    message,
                );
                tunnel::drop_connection(&route_key);
                return Ok(report.finish());
            }
        }
    } else {
        report.skip(DiagnosisStageKind::Tunnel, SkipReason::NoTunnel);
        connection.clone()
    };

    match &resolved {
        StoredConnection::PostgreSQL(pg) => {
            diagnose_postgres(pg, &mut report).await;
        }
        other => diagnose_generic(other, &mut report).await,
    }
    tunnel::drop_connection(&route_key);
    Ok(report.finish())
}

async fn diagnose_generic(connection: &StoredConnection, report: &mut Report) {
    for stage in [
        DiagnosisStageKind::Dns,
        DiagnosisStageKind::Tcp,
        DiagnosisStageKind::Tls,
        DiagnosisStageKind::Authentication,
    ] {
        report.skip(stage, SkipReason::NotApplicable);
    }
    let started = Instant::now();
    match dispatch::ping_connection(connection).await {
        Ok(result) => {
            report.latency_ms = result.latency_ms;
            report.pass(DiagnosisStageKind::Database, started, None);
        }
        Err(message) => report.fail(
            DiagnosisStageKind::Database,
            started,
            FailureKind::Other,
            message,
        ),
    }
}

async fn diagnose_postgres(pg: &crate::PgStoredConnection, report: &mut Report) {
    let spec = ResolvedPostgresConnectSpec::from_postgres(pg);
    let connect_timeout = spec.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);
    let mode = spec.tls.mode;
    if pg.environment == Environment::Production && !mode.verifies_chain() {
        report.warn(DiagnosisWarning::ProductionWithoutVerification);
    }

    // dns ------------------------------------------------------------------
    let started = Instant::now();
    let addresses: Vec<SocketAddr> = match tokio::time::timeout(
        DNS_TIMEOUT,
        tokio::net::lookup_host((spec.host.as_str(), spec.port)),
    )
    .await
    {
        Ok(Ok(addresses)) => addresses.collect(),
        Ok(Err(error)) => {
            report.fail(
                DiagnosisStageKind::Dns,
                started,
                FailureKind::DnsUnresolvable,
                format!("Could not resolve \"{}\": {error}", spec.host),
            );
            return;
        }
        Err(_) => {
            report.fail(
                DiagnosisStageKind::Dns,
                started,
                FailureKind::TimedOut,
                format!(
                    "Resolving \"{}\" timed out after {} s",
                    spec.host,
                    DNS_TIMEOUT.as_secs()
                ),
            );
            return;
        }
    };
    let Some(&first) = addresses.first() else {
        report.fail(
            DiagnosisStageKind::Dns,
            started,
            FailureKind::DnsUnresolvable,
            format!("\"{}\" resolved to no addresses", spec.host),
        );
        return;
    };
    report.pass(
        DiagnosisStageKind::Dns,
        started,
        Some(StageDetail::Dns {
            addresses: addresses.iter().map(|a| a.ip().to_string()).collect(),
        }),
    );

    // tcp ------------------------------------------------------------------
    let overall = Instant::now();
    let started = Instant::now();
    let stream = match tokio::time::timeout(connect_timeout, TcpStream::connect(first)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            let kind = match error.kind() {
                io::ErrorKind::ConnectionRefused => FailureKind::ConnectionRefused,
                io::ErrorKind::TimedOut => FailureKind::TimedOut,
                io::ErrorKind::HostUnreachable
                | io::ErrorKind::NetworkUnreachable
                | io::ErrorKind::NetworkDown => FailureKind::Unreachable,
                _ => FailureKind::Other,
            };
            report.fail(
                DiagnosisStageKind::Tcp,
                started,
                kind,
                format!("Could not connect to {first}: {error}"),
            );
            return;
        }
        Err(_) => {
            report.fail(
                DiagnosisStageKind::Tcp,
                started,
                FailureKind::TimedOut,
                format!(
                    "Connection to {first} timed out after {} ms",
                    connect_timeout.as_millis()
                ),
            );
            return;
        }
    };
    report.pass(DiagnosisStageKind::Tcp, started, None);

    // tls ------------------------------------------------------------------
    let started = Instant::now();
    let tls_config = match tls::client_config(&spec.tls) {
        Ok(config) => config,
        Err(error) => {
            report.fail(
                DiagnosisStageKind::Tls,
                started,
                FailureKind::InvalidLocalMaterial,
                error.to_string(),
            );
            return;
        }
    };
    let client_cert_configured = spec.tls.client_auth_configured();
    let pool_hostname_verification = tls::pool_hostname_verification(&spec.tls, &spec.host);
    let mut config = spec.tokio_config();
    // TLS is negotiated here, by hand, so the handshake is attributable;
    // tokio-postgres then speaks the plain protocol over whatever stream
    // it is handed.
    config.ssl_mode(tokio_postgres::config::SslMode::Disable);

    let mut encrypted = false;
    let startup = match tls_config {
        None => {
            report.skip(DiagnosisStageKind::Tls, SkipReason::TlsDisabled);
            connect_startup(&config, stream, connect_timeout).await
        }
        Some(tls_config) => {
            let mut stream = stream;
            let server_accepts =
                match tokio::time::timeout(connect_timeout, ssl_request(&mut stream)).await {
                    Ok(Ok(accepts)) => accepts,
                    Ok(Err(error)) => {
                        report.fail(
                            DiagnosisStageKind::Tls,
                            started,
                            FailureKind::HandshakeFailed,
                            format!("SSLRequest failed: {error}"),
                        );
                        return;
                    }
                    Err(_) => {
                        report.fail(
                            DiagnosisStageKind::Tls,
                            started,
                            FailureKind::TimedOut,
                            "SSLRequest timed out".into(),
                        );
                        return;
                    }
                };
            if !server_accepts {
                if mode == PgTlsMode::Prefer {
                    // libpq semantics: fall back to plaintext on the same socket.
                    report.pass(
                        DiagnosisStageKind::Tls,
                        started,
                        Some(tls_detail(
                            false,
                            None,
                            None,
                            false,
                            mode,
                            pool_hostname_verification,
                        )),
                    );
                    connect_startup(&config, stream, connect_timeout).await
                } else {
                    report.fail(
                        DiagnosisStageKind::Tls,
                        started,
                        FailureKind::ServerRefusedTls,
                        connect_error::tls_failure_message(TlsFailureKind::ServerRefusedTls, ""),
                    );
                    return;
                }
            } else {
                let mut make = MakeRustlsConnect::new(rustls::ClientConfig::clone(&tls_config));
                let connector =
                    match <MakeRustlsConnect as MakeTlsConnect<TcpStream>>::make_tls_connect(
                        &mut make,
                        &spec.tls.server_name,
                    ) {
                        Ok(connector) => connector,
                        Err(error) => {
                            report.fail(
                                DiagnosisStageKind::Tls,
                                started,
                                FailureKind::InvalidLocalMaterial,
                                format!(
                                    "Invalid TLS server name \"{}\": {error}",
                                    spec.tls.server_name
                                ),
                            );
                            return;
                        }
                    };
                match tokio::time::timeout(connect_timeout, connector.connect(stream)).await {
                    Ok(Ok(tls_stream)) => {
                        encrypted = true;
                        report.pass(
                            DiagnosisStageKind::Tls,
                            started,
                            Some(tls_detail(
                                true,
                                None,
                                None,
                                false,
                                mode,
                                pool_hostname_verification,
                            )),
                        );
                        connect_startup(&config, tls_stream, connect_timeout).await
                    }
                    Ok(Err(error)) => {
                        let view = connect_error::view_of_io(&error);
                        let kind = match connect_error::classify(&view, client_cert_configured) {
                            ConnectFailure::Tls(kind) => kind,
                            _ => TlsFailureKind::HandshakeFailed,
                        };
                        report.fail(
                            DiagnosisStageKind::Tls,
                            started,
                            kind.into(),
                            connect_error::tls_failure_message(kind, &view.message),
                        );
                        return;
                    }
                    Err(_) => {
                        report.fail(
                            DiagnosisStageKind::Tls,
                            started,
                            FailureKind::TimedOut,
                            "TLS handshake timed out".into(),
                        );
                        return;
                    }
                }
            }
        }
    };

    // authentication -------------------------------------------------------
    let started = Instant::now();
    let client = match startup {
        Ok(client) => client,
        Err(StartupError::Timeout) => {
            report.fail(
                DiagnosisStageKind::Authentication,
                started,
                FailureKind::TimedOut,
                "Server did not complete startup in time".into(),
            );
            return;
        }
        Err(StartupError::Postgres(error)) => {
            let view = connect_error::view_of(&error);
            match connect_error::classify(&view, client_cert_configured) {
                ConnectFailure::Authentication { message } => {
                    report.fail(
                        DiagnosisStageKind::Authentication,
                        started,
                        FailureKind::AuthenticationFailed,
                        message,
                    );
                }
                ConnectFailure::DatabaseMissing { message } => {
                    report.pass(DiagnosisStageKind::Authentication, started, None);
                    report.fail(
                        DiagnosisStageKind::Database,
                        started,
                        FailureKind::DatabaseMissing,
                        message,
                    );
                }
                ConnectFailure::Database { code, message, .. } => {
                    report.pass(DiagnosisStageKind::Authentication, started, None);
                    report.fail(
                        DiagnosisStageKind::Database,
                        started,
                        FailureKind::Other,
                        match code {
                            Some(code) => format!("{code}: {message}"),
                            None => message,
                        },
                    );
                }
                ConnectFailure::Tls(kind) => {
                    report.fail(
                        DiagnosisStageKind::Authentication,
                        started,
                        kind.into(),
                        connect_error::tls_failure_message(kind, &view.message),
                    );
                }
                ConnectFailure::Io(_, message) | ConnectFailure::Other(message) => {
                    report.fail(
                        DiagnosisStageKind::Authentication,
                        started,
                        FailureKind::Other,
                        format!("Connection dropped during startup: {message}"),
                    );
                }
            }
            return;
        }
    };
    report.pass(DiagnosisStageKind::Authentication, started, None);

    // database -------------------------------------------------------------
    let started = Instant::now();
    let version = client
        .query_one("SELECT current_setting('server_version')", &[])
        .await
        .map(|row| row.get::<_, String>(0));
    let server_version = match version {
        Ok(version) => version,
        Err(error) => {
            let view = connect_error::view_of(&error);
            report.fail(
                DiagnosisStageKind::Database,
                started,
                FailureKind::Other,
                view.db_message.unwrap_or(view.message),
            );
            return;
        }
    };
    // The server's own account of this session's transport. Missing view
    // (very old servers) leaves the handshake-derived values in place.
    if let Ok(row) = client
        .query_one(
            "SELECT ssl, version, cipher, client_dn IS NOT NULL FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
    {
        let ssl: bool = row.get(0);
        let protocol: Option<String> = row.get(1);
        let cipher: Option<String> = row.get(2);
        let client_cert: bool = row.get(3);
        encrypted = ssl;
        if let Some(DiagnosisStage {
            result: StageResult::Passed { detail, .. },
            ..
        }) = report
            .stages
            .iter_mut()
            .find(|s| s.stage == DiagnosisStageKind::Tls)
        {
            *detail = Some(tls_detail(ssl, protocol, cipher, client_cert, mode, pool_hostname_verification));
        }
    }
    report.latency_ms = elapsed_ms(overall);
    report.pass(
        DiagnosisStageKind::Database,
        started,
        Some(StageDetail::Database { server_version }),
    );
    if !encrypted {
        report.warn(DiagnosisWarning::NotEncrypted);
    }
    if pool_hostname_verification == PoolHostnameVerification::CaOnly {
        report.warn(DiagnosisWarning::PoolHostnameVerificationCaOnly);
    }
}

fn tls_detail(
    encrypted: bool,
    protocol: Option<String>,
    cipher: Option<String>,
    client_certificate_presented: bool,
    mode: PgTlsMode,
    pool_hostname_verification: PoolHostnameVerification,
) -> StageDetail {
    StageDetail::Tls {
        encrypted,
        protocol,
        cipher,
        certificate_verified: encrypted && mode.verifies_chain(),
        hostname_verified: encrypted && mode.verifies_hostname(),
        client_certificate_presented,
        pool_hostname_verification,
    }
}

/// Send the SSLRequest packet and read the one-byte answer.
async fn ssl_request(stream: &mut TcpStream) -> io::Result<bool> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // length (8) + SSLRequest code 80877103.
    stream
        .write_all(&[0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f])
        .await?;
    let mut answer = [0u8; 1];
    stream.read_exact(&mut answer).await?;
    match answer[0] {
        b'S' => Ok(true),
        b'N' => Ok(false),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected SSLRequest response {other:#x}"),
        )),
    }
}

enum StartupError {
    Timeout,
    Postgres(tokio_postgres::Error),
}

async fn connect_startup<S>(
    config: &tokio_postgres::Config,
    stream: S,
    timeout: Duration,
) -> Result<tokio_postgres::Client, StartupError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    match tokio::time::timeout(timeout, config.connect_raw(stream, NoTls)).await {
        Ok(Ok((client, connection))) => {
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Ok(client)
        }
        Ok(Err(error)) => Err(StartupError::Postgres(error)),
        Err(_) => Err(StartupError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_blocks_every_later_stage_and_names_the_stage() {
        let mut report = Report::new(DatabaseEngine::PostgreSQL);
        report.skip(DiagnosisStageKind::Tunnel, SkipReason::NoTunnel);
        report.fail(
            DiagnosisStageKind::Dns,
            Instant::now(),
            FailureKind::DnsUnresolvable,
            "nope".into(),
        );
        let diagnosis = report.finish();
        assert_eq!(diagnosis.stages.len(), 6);
        assert_eq!(
            diagnosis.outcome,
            DiagnosisOutcome::Failed {
                stage: DiagnosisStageKind::Dns
            }
        );
        for stage in &diagnosis.stages[2..] {
            assert_eq!(
                stage.result,
                StageResult::Skipped {
                    reason: SkipReason::BlockedByEarlierFailure
                }
            );
        }
        assert!(matches!(
            diagnosis.stages[1].result,
            StageResult::Failed {
                kind: FailureKind::DnsUnresolvable,
                ..
            }
        ));
    }

    #[test]
    fn warnings_dedupe_and_reachable_carries_latency() {
        let mut report = Report::new(DatabaseEngine::PostgreSQL);
        for stage in STAGE_ORDER {
            report.pass(stage, Instant::now(), None);
        }
        report.latency_ms = 42;
        report.warn(DiagnosisWarning::NotEncrypted);
        report.warn(DiagnosisWarning::NotEncrypted);
        let diagnosis = report.finish();
        assert_eq!(
            diagnosis.outcome,
            DiagnosisOutcome::Reachable { latency_ms: 42 }
        );
        assert_eq!(diagnosis.warnings, vec![DiagnosisWarning::NotEncrypted]);
    }

    #[test]
    fn tls_detail_derives_verification_flags_from_the_mode() {
        let StageDetail::Tls {
            certificate_verified,
            hostname_verified,
            ..
        } = tls_detail(
            true,
            None,
            None,
            false,
            PgTlsMode::VerifyCa,
            PoolHostnameVerification::NotApplicable,
        )
        else {
            panic!("tls detail");
        };
        assert!(certificate_verified && !hostname_verified);
        let StageDetail::Tls {
            certificate_verified,
            ..
        } = tls_detail(
            false,
            None,
            None,
            false,
            PgTlsMode::VerifyFull,
            PoolHostnameVerification::Full,
        )
        else {
            panic!("tls detail");
        };
        assert!(!certificate_verified, "plaintext verifies nothing");
    }

    #[test]
    fn wire_shape_uses_kebab_free_camel_case_tags() {
        let json = serde_json::to_string(&StageResult::Failed {
            elapsed_ms: 3,
            kind: FailureKind::ServerRefusedTls,
            message: "m".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"status":"failed","elapsedMs":3,"kind":"serverRefusedTls","message":"m"}"#
        );
        let json = serde_json::to_string(&DiagnosisOutcome::Failed {
            stage: DiagnosisStageKind::Authentication,
        })
        .unwrap();
        assert_eq!(json, r#"{"kind":"failed","stage":"authentication"}"#);
    }

    // Live tests -----------------------------------------------------------

    fn pg(port: u16, tls_options: Option<crate::PgTlsOptions>) -> crate::PgStoredConnection {
        crate::PgStoredConnection {
            organization: Default::default(),
            id: "diag".into(),
            name: "diag".into(),
            database: "dbunk_demo".into(),
            host: "127.0.0.1".into(),
            port,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: String::new(),
            environment: Default::default(),
            safe_mode: Default::default(),
            read_only: false,
            last_activity_at: None,
            ssl: true,
            tls_options,
            driver_options: Some(crate::PgDriverOptions {
                connect_timeout_ms: Some(5_000),
                ..Default::default()
            }),
            ssh_tunnel: Default::default(),
        }
    }

    fn fixture_cert(name: &str) -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../infrastructure/test-db/postgres-tls/certs")
            .join(name)
            .display()
            .to_string()
    }

    async fn diagnose(pg: crate::PgStoredConnection) -> ConnectionDiagnosis {
        let mut report = Report::new(DatabaseEngine::PostgreSQL);
        report.skip(DiagnosisStageKind::Tunnel, SkipReason::NoTunnel);
        diagnose_postgres(&pg, &mut report).await;
        report.finish()
    }

    fn result_of(d: &ConnectionDiagnosis, stage: DiagnosisStageKind) -> &StageResult {
        &d.stages.iter().find(|s| s.stage == stage).unwrap().result
    }

    fn tls_of(d: &ConnectionDiagnosis) -> (bool, bool, bool, bool) {
        match result_of(d, DiagnosisStageKind::Tls) {
            StageResult::Passed {
                detail:
                    Some(StageDetail::Tls {
                        encrypted,
                        certificate_verified,
                        hostname_verified,
                        client_certificate_presented,
                        ..
                    }),
                ..
            } => (
                *encrypted,
                *certificate_verified,
                *hostname_verified,
                *client_certificate_presented,
            ),
            other => panic!("tls stage: {other:?}"),
        }
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn diagnosis_live_wrong_password_lands_on_authentication() {
        let mut pg = pg(15432, None);
        pg.password = "wrong".into();
        let d = diagnose(pg).await;
        assert_eq!(
            d.outcome,
            DiagnosisOutcome::Failed {
                stage: DiagnosisStageKind::Authentication
            }
        );
        assert!(matches!(
            result_of(&d, DiagnosisStageKind::Dns),
            StageResult::Passed { .. }
        ));
        assert!(matches!(
            result_of(&d, DiagnosisStageKind::Tcp),
            StageResult::Passed { .. }
        ));
        assert!(matches!(
            result_of(&d, DiagnosisStageKind::Authentication),
            StageResult::Failed {
                kind: FailureKind::AuthenticationFailed,
                ..
            }
        ));
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn diagnosis_live_missing_database_lands_on_database() {
        let mut pg = pg(15432, None);
        pg.database = "no_such_db".into();
        let d = diagnose(pg).await;
        assert_eq!(
            d.outcome,
            DiagnosisOutcome::Failed {
                stage: DiagnosisStageKind::Database
            }
        );
        assert!(matches!(
            result_of(&d, DiagnosisStageKind::Database),
            StageResult::Failed {
                kind: FailureKind::DatabaseMissing,
                ..
            }
        ));
    }

    #[tokio::test]
    #[ignore = "requires nothing listening on 15499"]
    async fn diagnosis_live_closed_port_lands_on_tcp() {
        let d = diagnose(pg(15499, None)).await;
        assert_eq!(
            d.outcome,
            DiagnosisOutcome::Failed {
                stage: DiagnosisStageKind::Tcp
            }
        );
        assert!(matches!(
            result_of(&d, DiagnosisStageKind::Tcp),
            StageResult::Failed {
                kind: FailureKind::ConnectionRefused,
                ..
            }
        ));
    }

    #[tokio::test]
    #[ignore = "requires network DNS"]
    async fn diagnosis_live_bad_host_lands_on_dns() {
        let mut pg = pg(5432, None);
        pg.host = "nonexistent.invalid".into();
        let d = diagnose(pg).await;
        assert_eq!(
            d.outcome,
            DiagnosisOutcome::Failed {
                stage: DiagnosisStageKind::Dns
            }
        );
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn diagnosis_live_prefer_against_plaintext_reports_not_encrypted() {
        let d = diagnose(pg(15432, None)).await;
        assert!(matches!(d.outcome, DiagnosisOutcome::Reachable { .. }));
        assert!(!tls_of(&d).0);
        assert!(d.warnings.contains(&DiagnosisWarning::NotEncrypted));
        let mut disabled = pg(
            15432,
            Some(crate::PgTlsOptions {
                mode: PgTlsMode::Disable,
                ..Default::default()
            }),
        );
        disabled.environment = Environment::Production;
        let d = diagnose(disabled).await;
        assert_eq!(
            result_of(&d, DiagnosisStageKind::Tls),
            &StageResult::Skipped {
                reason: SkipReason::TlsDisabled
            }
        );
        assert!(d
            .warnings
            .contains(&DiagnosisWarning::ProductionWithoutVerification));
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn diagnosis_live_verify_full_with_ca_verifies_everything() {
        let d = diagnose(pg(
            15433,
            Some(crate::PgTlsOptions {
                mode: PgTlsMode::VerifyFull,
                root_cert_path: Some(fixture_cert("ca.crt")),
                ..Default::default()
            }),
        ))
        .await;
        assert!(
            matches!(d.outcome, DiagnosisOutcome::Reachable { .. }),
            "{d:?}"
        );
        assert_eq!(tls_of(&d), (true, true, true, false));
        assert!(d.warnings.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn diagnosis_live_verify_ca_without_ca_is_untrusted_and_wrong_name_mismatches() {
        let d = diagnose(pg(
            15433,
            Some(crate::PgTlsOptions {
                mode: PgTlsMode::VerifyCa,
                ..Default::default()
            }),
        ))
        .await;
        assert!(
            matches!(
                result_of(&d, DiagnosisStageKind::Tls),
                StageResult::Failed {
                    kind: FailureKind::CertificateUntrusted,
                    ..
                }
            ),
            "{d:?}"
        );
        let d = diagnose(pg(
            15433,
            Some(crate::PgTlsOptions {
                mode: PgTlsMode::VerifyFull,
                root_cert_path: Some(fixture_cert("ca.crt")),
                server_name: Some("wrong.example".into()),
                ..Default::default()
            }),
        ))
        .await;
        assert!(
            matches!(
                result_of(&d, DiagnosisStageKind::Tls),
                StageResult::Failed {
                    kind: FailureKind::HostnameMismatch,
                    ..
                }
            ),
            "{d:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn diagnosis_live_client_certificate_authenticates_and_encrypted_key_is_refused() {
        let mut pg_cert = pg(
            15433,
            Some(crate::PgTlsOptions {
                mode: PgTlsMode::VerifyFull,
                root_cert_path: Some(fixture_cert("ca.crt")),
                client_cert_path: Some(fixture_cert("client.crt")),
                client_key_path: Some(fixture_cert("client.key")),
                ..Default::default()
            }),
        );
        pg_cert.user = "dbunk_cert".into();
        pg_cert.password = String::new();
        let d = diagnose(pg_cert).await;
        assert!(
            matches!(d.outcome, DiagnosisOutcome::Reachable { .. }),
            "{d:?}"
        );
        assert!(tls_of(&d).3);

        let d = diagnose(pg(
            15433,
            Some(crate::PgTlsOptions {
                mode: PgTlsMode::Require,
                client_cert_path: Some(fixture_cert("client.crt")),
                client_key_path: Some(fixture_cert("client-encrypted.key")),
                ..Default::default()
            }),
        ))
        .await;
        match result_of(&d, DiagnosisStageKind::Tls) {
            StageResult::Failed {
                kind: FailureKind::InvalidLocalMaterial,
                message,
                ..
            } => {
                assert!(message.contains("client-encrypted.key"));
            }
            other => panic!("expected invalid material: {other:?}"),
        }
    }
}
