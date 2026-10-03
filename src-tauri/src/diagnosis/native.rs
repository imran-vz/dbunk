//! Direct native diagnosis. Socket tasks remain owned through cancellation and
//! joined cleanup; the legacy Tauri diagnosis keeps its existing behavior.
//!
//! The deadline bounds asynchronous stages, not a hard wall-clock return time:
//! Tokio's platform DNS resolver and synchronous TLS-file/platform-root reads
//! can involve uninterruptible platform I/O. Cancelling DNS does not prove its
//! internal blocking resolver has stopped. Protocol socket drivers are owned
//! separately and always joined before this runner returns.
use super::*;
use crate::postgres::dedicated::DriverJoins;
use std::future::Future;
use tokio::sync::watch;
mod types;
pub use types::*;
#[cfg(test)]
mod tests;

const MAX_ADDRESSES: usize = 32;
const MAX_DURATION: Duration = Duration::from_secs(10);
pub const CHANNEL_BINDING_LIMITATION: &str = "This staged probe does not use SCRAM channel binding; a server requiring it can reject this probe even when the regular query driver connects.";

struct Probe<'a> {
    deadline: tokio::time::Instant,
    cancellation: watch::Receiver<bool>,
    drivers: &'a DriverJoins,
}
#[derive(Clone, Copy)]
enum Stop {
    Cancelled,
    Deadline,
}
impl Probe<'_> {
    async fn wait<T>(&mut self, future: impl Future<Output = T>) -> Result<T, Stop> {
        if *self.cancellation.borrow() || self.cancellation.has_changed().is_err() {
            return Err(Stop::Cancelled);
        }
        tokio::select! {
            biased;
            _ = async {
                loop {
                    if *self.cancellation.borrow() || self.cancellation.changed().await.is_err() { break; }
                }
            } => Err(Stop::Cancelled),
            _ = tokio::time::sleep_until(self.deadline) => Err(Stop::Deadline),
            result = future => Ok(result),
        }
    }
}
impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.drivers.abort_all();
    }
}

/// `drivers` must be a probe-local child of the backend shutdown owner. A
/// cancelled waiter may drop this future; the parent still owns every join.
pub(crate) async fn run(
    pg: &crate::PgStoredConnection,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<bool>,
) -> Result<NativeDiagnosis, String> {
    if !pg.ssh_tunnel.is_default() {
        return Err("Native diagnosis supports direct PostgreSQL connections only".into());
    }
    let spec = ResolvedPostgresConnectSpec::from_postgres(pg);
    let duration = spec
        .connect_timeout
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT)
        .min(MAX_DURATION);
    let mut probe = Probe {
        deadline: tokio::time::Instant::now() + duration,
        cancellation,
        drivers,
    };
    let mut report = Report::new(DatabaseEngine::PostgreSQL);
    report.skip(DiagnosisStageKind::Tunnel, SkipReason::NoTunnel);
    let result = diagnose(pg, &spec, &mut probe, &mut report).await;
    drivers.abort_all();
    drivers.drain().await;
    result?;
    if *probe.cancellation.borrow() || probe.cancellation.has_changed().is_err() {
        return Err("Connection diagnosis cancelled".into());
    }
    let report = NativeDiagnosis::from_legacy(report.finish());
    report
        .checked_heap_bytes()
        .ok_or_else(|| "Diagnosis report exceeds its bounded allowance".to_string())?;
    Ok(report)
}
fn stopped(
    stop: Stop,
    report: &mut Report,
    stage: DiagnosisStageKind,
    started: Instant,
) -> Result<(), String> {
    match stop {
        Stop::Cancelled => Err("Connection diagnosis cancelled".into()),
        Stop::Deadline => {
            report.fail(
                stage,
                started,
                FailureKind::TimedOut,
                "The diagnosis deadline expired during this stage".into(),
            );
            Ok(())
        }
    }
}
fn fail(
    report: &mut Report,
    stage: DiagnosisStageKind,
    started: Instant,
    kind: FailureKind,
    message: &'static str,
) {
    report.fail(stage, started, kind, message.into());
}
async fn diagnose(
    pg: &crate::PgStoredConnection,
    spec: &ResolvedPostgresConnectSpec,
    probe: &mut Probe<'_>,
    report: &mut Report,
) -> Result<(), String> {
    let overall = Instant::now();
    let mode = spec.tls.mode;
    let verification = tls::pool_hostname_verification(&spec.tls, &spec.host);
    record_known_transport_warnings(report, mode, verification);
    if pg.environment == Environment::Production && !mode.verifies_chain() {
        report.warn(DiagnosisWarning::ProductionWithoutVerification);
    }
    let started = Instant::now();
    let addresses = match probe
        .wait(tokio::net::lookup_host((spec.host.as_str(), spec.port)))
        .await
    {
        Err(stop) => return stopped(stop, report, DiagnosisStageKind::Dns, started),
        Ok(Err(_)) => {
            fail(
                report,
                DiagnosisStageKind::Dns,
                started,
                FailureKind::DnsUnresolvable,
                "The host name could not be resolved",
            );
            return Ok(());
        }
        Ok(Ok(addresses)) => addresses.take(MAX_ADDRESSES + 1).collect::<Vec<_>>(),
    };
    if addresses.is_empty() || addresses.len() > MAX_ADDRESSES {
        fail(
            report,
            DiagnosisStageKind::Dns,
            started,
            FailureKind::DnsUnresolvable,
            if addresses.is_empty() {
                "The host resolved to no addresses"
            } else {
                "The host resolved to more than 32 addresses; diagnosis refused the oversized result"
            },
        );
        return Ok(());
    }
    report.pass(
        DiagnosisStageKind::Dns,
        started,
        Some(StageDetail::Dns {
            addresses: addresses
                .iter()
                .map(|address| address.ip().to_string())
                .collect(),
        }),
    );
    let started = Instant::now();
    let mut stream = None;
    let mut last_kind = FailureKind::Other;
    for address in addresses {
        match probe.wait(TcpStream::connect(address)).await {
            Err(stop) => return stopped(stop, report, DiagnosisStageKind::Tcp, started),
            Ok(Ok(socket)) => {
                stream = Some(socket);
                break;
            }
            Ok(Err(error)) => {
                last_kind = match error.kind() {
                    io::ErrorKind::ConnectionRefused => FailureKind::ConnectionRefused,
                    io::ErrorKind::TimedOut => FailureKind::TimedOut,
                    io::ErrorKind::HostUnreachable
                    | io::ErrorKind::NetworkUnreachable
                    | io::ErrorKind::NetworkDown => FailureKind::Unreachable,
                    _ => FailureKind::Other,
                }
            }
        }
    }
    let mut stream = match stream {
        Some(stream) => stream,
        None => {
            fail(
                report,
                DiagnosisStageKind::Tcp,
                started,
                last_kind,
                "None of the resolved addresses accepted the connection",
            );
            return Ok(());
        }
    };
    report.pass(DiagnosisStageKind::Tcp, started, None);
    let tls_started = Instant::now();
    let tls_config = match tls::native::client_config(&spec.tls) {
        Ok(config) => config,
        Err(_) => {
            fail(
                report,
                DiagnosisStageKind::Tls,
                tls_started,
                FailureKind::InvalidLocalMaterial,
                "Local certificate or private-key material could not be used",
            );
            return Ok(());
        }
    };
    let mut config = spec.tokio_config();
    config.ssl_mode(tokio_postgres::config::SslMode::Disable);
    let client_cert = spec.tls.client_auth_configured();
    let mut handshake = None;
    let auth_started;
    let startup = if let Some(tls_config) = tls_config {
        let accepts = match probe.wait(ssl_request(&mut stream)).await {
            Err(stop) => return stopped(stop, report, DiagnosisStageKind::Tls, tls_started),
            Ok(Err(_)) => {
                fail(
                    report,
                    DiagnosisStageKind::Tls,
                    tls_started,
                    FailureKind::HandshakeFailed,
                    "The server did not complete the PostgreSQL TLS request",
                );
                return Ok(());
            }
            Ok(Ok(accepts)) => accepts,
        };
        if !accepts {
            if mode != PgTlsMode::Prefer {
                fail(
                    report,
                    DiagnosisStageKind::Tls,
                    tls_started,
                    FailureKind::ServerRefusedTls,
                    "The server does not support TLS on this port",
                );
                return Ok(());
            }
            report.warn(DiagnosisWarning::NotEncrypted);
            report.pass(
                DiagnosisStageKind::Tls,
                tls_started,
                Some(tls_detail(false, None, None, false, mode, verification)),
            );
            auth_started = Instant::now();
            startup(&config, stream, probe).await
        } else {
            let mut make = MakeRustlsConnect::new(rustls::ClientConfig::clone(&tls_config));
            let connector = match <MakeRustlsConnect as MakeTlsConnect<TcpStream>>::make_tls_connect(
                &mut make,
                &spec.tls.server_name,
            ) {
                Ok(connector) => connector,
                Err(_) => {
                    fail(
                        report,
                        DiagnosisStageKind::Tls,
                        tls_started,
                        FailureKind::InvalidLocalMaterial,
                        "The TLS server name is invalid",
                    );
                    return Ok(());
                }
            };
            let stream = match probe.wait(connector.connect(stream)).await {
                Err(stop) => return stopped(stop, report, DiagnosisStageKind::Tls, tls_started),
                Ok(Err(error)) => {
                    let view = connect_error::view_of_io(&error);
                    let kind = match connect_error::classify(&view, client_cert) {
                        ConnectFailure::Tls(kind) => kind,
                        _ => TlsFailureKind::HandshakeFailed,
                    };
                    fail(
                        report,
                        DiagnosisStageKind::Tls,
                        tls_started,
                        kind.into(),
                        "TLS verification or handshake failed; no server error text is retained",
                    );
                    return Ok(());
                }
                Ok(Ok(stream)) => stream,
            };
            handshake = Some((
                elapsed_ms(tls_started),
                tls_detail(true, None, None, false, mode, verification),
            ));
            auth_started = Instant::now();
            startup(&config, stream, probe).await
        }
    } else {
        report.skip(DiagnosisStageKind::Tls, SkipReason::TlsDisabled);
        auth_started = Instant::now();
        startup(&config, stream, probe).await
    };
    let client = match startup {
        Err(stop) => {
            confirm_handshake(report, handshake.take());
            return stopped(
                stop,
                report,
                DiagnosisStageKind::Authentication,
                auth_started,
            );
        }
        Ok(Err(error)) => {
            let failure = connect_error::classify(&connect_error::view_of(&error), client_cert);
            if let ConnectFailure::Tls(kind) = failure {
                if let Some((elapsed_ms, _)) = handshake.take() {
                    report.record(
                        DiagnosisStageKind::Tls,
                        StageResult::Failed {
                            elapsed_ms,
                            kind: kind.into(),
                            message: "The server rejected the TLS startup exchange".into(),
                        },
                    );
                    return Ok(());
                }
            }
            confirm_handshake(report, handshake.take());
            match failure {
                ConnectFailure::Authentication { .. } => fail(
                    report,
                    DiagnosisStageKind::Authentication,
                    auth_started,
                    FailureKind::AuthenticationFailed,
                    "The server rejected authentication",
                ),
                ConnectFailure::DatabaseMissing { .. } => {
                    report.pass(DiagnosisStageKind::Authentication, auth_started, None);
                    fail(
                        report,
                        DiagnosisStageKind::Database,
                        Instant::now(),
                        FailureKind::DatabaseMissing,
                        "The requested database does not exist",
                    );
                }
                ConnectFailure::Database { .. } => {
                    report.pass(DiagnosisStageKind::Authentication, auth_started, None);
                    fail(
                        report,
                        DiagnosisStageKind::Database,
                        Instant::now(),
                        FailureKind::Other,
                        "The server refused database startup",
                    );
                }
                _ => fail(
                    report,
                    DiagnosisStageKind::Authentication,
                    auth_started,
                    FailureKind::Other,
                    "The connection ended during startup",
                ),
            }
            return Ok(());
        }
        Ok(Ok(client)) => {
            confirm_handshake(report, handshake.take());
            client
        }
    };
    report.pass(DiagnosisStageKind::Authentication, auth_started, None);
    database(&client, spec, probe, report, overall).await
}
async fn startup<S>(
    config: &tokio_postgres::Config,
    stream: S,
    probe: &mut Probe<'_>,
) -> Result<Result<tokio_postgres::Client, tokio_postgres::Error>, Stop>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let drivers = probe.drivers;
    probe
        .wait(async move {
            let (client, connection) = config.connect_raw(stream, NoTls).await?;
            drivers.track_task(tokio::spawn(async move {
                let _ = connection.await;
            }));
            Ok(client)
        })
        .await
}
async fn database(
    client: &tokio_postgres::Client,
    spec: &ResolvedPostgresConnectSpec,
    probe: &mut Probe<'_>,
    report: &mut Report,
    overall: Instant,
) -> Result<(), String> {
    let started = Instant::now();
    // Preserve the earlier native Test action's session-option validation.
    // These statements affect only the disposable probe connection.
    for sql in crate::postgres::options::driver_option_sql(
        &spec.driver_options,
        spec.safety_policy.read_only,
    ) {
        match probe.wait(client.batch_execute(&sql)).await {
            Err(stop) => return stopped(stop, report, DiagnosisStageKind::Database, started),
            Ok(Err(_)) => {
                fail(
                    report,
                    DiagnosisStageKind::Database,
                    started,
                    FailureKind::Other,
                    "Configured PostgreSQL session options could not be applied",
                );
                return Ok(());
            }
            Ok(Ok(())) => {}
        }
    }
    let mode = spec.tls.mode;
    let verification = tls::pool_hostname_verification(&spec.tls, &spec.host);
    let result = probe
        .wait(client.query_one(
            "SELECT CASE WHEN octet_length(current_setting('server_version')) <= 1024 \
             THEN current_setting('server_version') END",
            &[],
        ))
        .await;
    let version = match result {
        Err(stop) => return stopped(stop, report, DiagnosisStageKind::Database, started),
        Ok(Err(_)) => {
            fail(
                report,
                DiagnosisStageKind::Database,
                started,
                FailureKind::Other,
                "The server version could not be read",
            );
            return Ok(());
        }
        Ok(Ok(row)) => match row.try_get::<_, Option<String>>(0) {
            Ok(Some(value)) if value.len() <= 1024 => value,
            _ => {
                fail(
                    report,
                    DiagnosisStageKind::Database,
                    started,
                    FailureKind::Other,
                    "The server version was missing or exceeded its limit",
                );
                return Ok(());
            }
        },
    };
    let ssl = probe
        .wait(client.query_one(
            "SELECT ssl, CASE WHEN octet_length(version)<=128 THEN version END, \
             CASE WHEN octet_length(cipher)<=128 THEN cipher END, client_dn IS NOT NULL \
             FROM pg_stat_ssl WHERE pid=pg_backend_pid()",
            &[],
        ))
        .await;
    match ssl {
        Err(stop) => return stopped(stop, report, DiagnosisStageKind::Database, started),
        Ok(Ok(row)) => {
            if let (Ok(encrypted), Ok(protocol), Ok(cipher), Ok(client_cert)) = (
                row.try_get::<_, bool>(0),
                row.try_get::<_, Option<String>>(1),
                row.try_get::<_, Option<String>>(2),
                row.try_get::<_, bool>(3),
            ) {
                if protocol.as_ref().is_none_or(|value| value.len() <= 128)
                    && cipher.as_ref().is_none_or(|value| value.len() <= 128)
                {
                    if let Some(stage) = report
                        .stages
                        .iter_mut()
                        .find(|stage| stage.stage == DiagnosisStageKind::Tls)
                    {
                        if matches!(stage.result, StageResult::Passed { .. }) {
                            stage.result = StageResult::Passed {
                                elapsed_ms: match stage.result {
                                    StageResult::Passed { elapsed_ms, .. } => elapsed_ms,
                                    _ => 0,
                                },
                                detail: Some(tls_detail(
                                    encrypted,
                                    protocol,
                                    cipher,
                                    client_cert,
                                    mode,
                                    verification,
                                )),
                            };
                        }
                    }
                }
            }
        }
        Ok(Err(_)) => {} // Optional server observation; retain successful handshake facts.
    }
    report.latency_ms = elapsed_ms(overall);
    report.pass(
        DiagnosisStageKind::Database,
        started,
        Some(StageDetail::Database {
            server_version: version,
        }),
    );
    Ok(())
}
