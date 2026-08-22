//! Canonical dedicated tokio-postgres socket used by Query Session and Table Browse.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::poll_fn;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use tokio::sync::mpsc;
use tokio_postgres::error::ErrorPosition;
use tokio_postgres::{config::SslMode, AsyncMessage, Client, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::connect_spec::ResolvedPostgresConnectSpec;
use super::options::driver_option_sql;

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
    Database {
        code: Option<String>,
        message: String,
        severity: Option<String>,
        position: Option<u32>,
    },
}

pub(crate) struct DedicatedConnection {
    pub client: Arc<Client>,
    pub cancel: tokio_postgres::CancelToken,
    pub tls: bool,
    _driver: tokio::task::JoinHandle<()>,
}

impl DedicatedConnection {
    pub(crate) fn is_closed(&self) -> bool {
        self.client.is_closed()
    }
}

pub(crate) async fn connect(
    spec: &ResolvedPostgresConnectSpec,
    notices: NoticeSink,
) -> Result<DedicatedConnection, DedicatedError> {
    let mut config = spec.tokio_config();
    config.ssl_mode(if spec.tls_prefer {
        SslMode::Prefer
    } else {
        SslMode::Disable
    });
    let (client, driver) = if spec.tls_prefer {
        let tls = MakeRustlsConnect::new(permissive_tls_config());
        let connect = config.connect(tls);
        let (client, connection) = with_deadline(spec, connect).await?;
        let driver = spawn_driver(connection, notices);
        (client, driver)
    } else {
        let connect = config.connect(NoTls);
        let (client, connection) = with_deadline(spec, connect).await?;
        let driver = spawn_driver(connection, notices);
        (client, driver)
    };
    for statement in driver_option_sql(&spec.driver_options, spec.safety_policy.read_only) {
        client
            .batch_execute(&statement)
            .await
            .map_err(database_error)?;
    }
    let cancel = client.cancel_token();
    Ok(DedicatedConnection {
        client: Arc::new(client),
        cancel,
        tls: spec.tls_prefer,
        _driver: driver,
    })
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

async fn with_deadline<T, E>(
    spec: &ResolvedPostgresConnectSpec,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, DedicatedError> {
    match spec.connect_timeout {
        Some(limit) => tokio::time::timeout(limit, future)
            .await
            .map_err(|_| DedicatedError::Timeout {
                operation: "connect".into(),
            })?
            .map_err(|_| DedicatedError::ConnectionLost),
        None => future.await.map_err(|_| DedicatedError::ConnectionLost),
    }
}

pub(crate) async fn cancel(cancel: tokio_postgres::CancelToken, tls: bool) -> bool {
    let future = async move {
        if tls {
            cancel
                .cancel_query(MakeRustlsConnect::new(permissive_tls_config()))
                .await
        } else {
            cancel.cancel_query(NoTls).await
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

#[derive(Debug)]
struct AcceptAllVerifier;
impl ServerCertVerifier for AcceptAllVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ED25519,
        ]
    }
}

fn permissive_tls_config() -> rustls::ClientConfig {
    let mut config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("ring protocol versions")
    .with_root_certificates(rustls::RootCertStore::empty())
    .with_no_client_auth();
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(AcceptAllVerifier));
    config
}
