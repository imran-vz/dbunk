//! One TLS resolver for every PostgreSQL connect site (ADR-0025).
//!
//! Four places open PostgreSQL sockets — the dedicated tokio-postgres
//! driver, the SQLx metadata pool, the SQLx-Any DSN fallback, and the
//! libpq subprocess tools — and before this module each made its own
//! `prefer | disable` decision. [`ResolvedTls`] is computed once from the
//! stored record and rendered per backend here, so a mode means the same
//! thing everywhere and a new mode is a change to this file only.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Once, OnceLock};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use tokio_postgres::config::SslMode;

use crate::{PgStoredConnection, PgTlsMode};

/// SQLx 0.8 seeds every PostgreSQL options value from libpq-style process
/// variables, including credentials and certificate paths that have no public
/// "clear" setter. dbunk owns those values in `StoredConnection`, so discard
/// the inherited inputs once, before the application starts any worker threads.
pub(crate) const SQLX_INHERITED_OVERRIDES: &[&str] = &[
    "PGPASSWORD",
    "PGPASSFILE",
    "PGOPTIONS",
    "PGSSLCERT",
    "PGSSLKEY",
    "PGSSLROOTCERT",
];

/// Runs once, from `run()`, before any worker thread exists. `remove_var`
/// is not safe alongside C code reading the environment on another thread,
/// so connect sites must not call this lazily — they rely on `run()`.
pub(crate) fn prepare_sqlx_environment() {
    static PREPARE: Once = Once::new();

    PREPARE.call_once(|| {
        for key in SQLX_INHERITED_OVERRIDES {
            std::env::remove_var(key);
        }
    });
}

/// TLS decision for one connect, independent of which driver performs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTls {
    pub mode: PgTlsMode,
    /// Server name for SNI and certificate matching. Equals the
    /// connection host unless the user or the SSH tunnel supplied one.
    pub server_name: String,
    pub root_cert_path: Option<PathBuf>,
    pub client_cert_path: Option<PathBuf>,
    pub client_key_path: Option<PathBuf>,
}

impl ResolvedTls {
    pub(crate) fn from_postgres(pg: &PgStoredConnection) -> Self {
        let options = pg.tls_options.as_ref();
        let server_name = options
            .and_then(|o| o.server_name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(pg.host.as_str())
            .to_string();
        Self {
            mode: pg.resolved_tls_mode(),
            server_name,
            root_cert_path: options.and_then(|o| path_of(o.root_cert_path.as_deref())),
            client_cert_path: options.and_then(|o| path_of(o.client_cert_path.as_deref())),
            client_key_path: options.and_then(|o| path_of(o.client_key_path.as_deref())),
        }
    }

    /// Plaintext; the fixture default for tests that never negotiate TLS.
    #[cfg(test)]
    pub(crate) fn plain(host: &str) -> Self {
        Self::with_mode(host, PgTlsMode::Disable)
    }

    /// `prefer` with no material; the pre-ADR-0025 behaviour of `ssl: true`.
    #[cfg(test)]
    pub(crate) fn prefer(host: &str) -> Self {
        Self::with_mode(host, PgTlsMode::Prefer)
    }

    #[cfg(test)]
    pub(crate) fn with_mode(host: &str, mode: PgTlsMode) -> Self {
        Self {
            mode,
            server_name: host.to_string(),
            root_cert_path: None,
            client_cert_path: None,
            client_key_path: None,
        }
    }

    /// True when the certificate must be matched against a name other
    /// than the socket host — the SSH-tunnel and IP-literal cases.
    pub(crate) fn server_name_differs_from(&self, host: &str) -> bool {
        self.server_name != host
    }

    pub(crate) fn client_auth_configured(&self) -> bool {
        self.client_cert_path.is_some() || self.client_key_path.is_some()
    }
}

fn path_of(raw: Option<&str>) -> Option<PathBuf> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Certificate material problems detected before any socket is opened.
/// Every variant names the offending path so the message is actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TlsMaterialError {
    /// Exactly one of client certificate / client key was configured.
    ClientPairIncomplete {
        present: &'static str,
        path: String,
    },
    ClientKeyEncrypted {
        path: String,
    },
    Unreadable {
        path: String,
        detail: String,
    },
    Malformed {
        path: String,
        detail: String,
    },
}

impl std::fmt::Display for TlsMaterialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClientPairIncomplete { present, path } => write!(
                f,
                "Client certificate authentication needs both a certificate and a key; only the {present} was provided at {path}."
            ),
            Self::ClientKeyEncrypted { path } => write!(
                f,
                "Client key {path} is passphrase-protected; encrypted keys are not supported. Export an unencrypted PKCS#8 key."
            ),
            Self::Unreadable { path, detail } => write!(f, "Could not read {path}: {detail}"),
            Self::Malformed { path, detail } => write!(f, "{path} is not valid PEM: {detail}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Mode tables
// ---------------------------------------------------------------------------

/// tokio-postgres only knows whether to *request* TLS; verification is
/// the rustls config's job, so every verifying mode maps to `Require`.
pub(crate) fn tokio_ssl_mode(mode: PgTlsMode) -> SslMode {
    match mode {
        PgTlsMode::Disable => SslMode::Disable,
        PgTlsMode::Prefer => SslMode::Prefer,
        PgTlsMode::Require | PgTlsMode::VerifyCa | PgTlsMode::VerifyFull => SslMode::Require,
    }
}

/// SQLx has no `hostaddr`: it verifies against the socket host. When
/// `verify-full` must match a different name (SSH tunnel), the pool path
/// can only verify the chain — it downgrades to `VerifyCa` and the
/// caller discloses it (see `PoolHostnameVerification`).
pub(crate) fn sqlx_ssl_mode(mode: PgTlsMode, server_name_differs: bool) -> PgSslMode {
    match mode {
        PgTlsMode::Disable => PgSslMode::Disable,
        PgTlsMode::Prefer => PgSslMode::Prefer,
        PgTlsMode::Require => PgSslMode::Require,
        PgTlsMode::VerifyCa => PgSslMode::VerifyCa,
        PgTlsMode::VerifyFull if server_name_differs => PgSslMode::VerifyCa,
        PgTlsMode::VerifyFull => PgSslMode::VerifyFull,
    }
}

// ---------------------------------------------------------------------------
// Renderer A — rustls client config for the dedicated driver
// ---------------------------------------------------------------------------

/// Build the rustls config for `tls`. `Ok(None)` for `disable` (use
/// `NoTls`). Material errors surface here, before any socket opens.
pub(crate) fn client_config(
    tls: &ResolvedTls,
) -> Result<Option<Arc<rustls::ClientConfig>>, TlsMaterialError> {
    if tls.mode == PgTlsMode::Disable {
        return Ok(None);
    }
    let client_auth = load_client_auth(tls)?;
    let builder = rustls::ClientConfig::builder_with_provider(ring_provider())
        .with_safe_default_protocol_versions()
        .expect("ring protocol versions");

    let verifier: Arc<dyn ServerCertVerifier> = if tls.mode.verifies_chain() {
        let roots = root_store(tls.root_cert_path.as_deref())?;
        let webpki = WebPkiServerVerifier::builder_with_provider(roots, ring_provider())
            .build()
            .map_err(|error| TlsMaterialError::Malformed {
                path: tls
                    .root_cert_path
                    .as_deref()
                    .map(display_path)
                    .unwrap_or_else(|| "platform trust store".into()),
                detail: error.to_string(),
            })?;
        if tls.mode.verifies_hostname() {
            webpki
        } else {
            Arc::new(CaOnlyVerifier { inner: webpki })
        }
    } else {
        // `prefer` / `require`: encrypt without authenticating the peer —
        // libpq semantics, and the policy every pre-ADR-0025 `ssl: true`
        // connection already had.
        Arc::new(NoVerification)
    };
    let builder = builder
        .dangerous()
        .with_custom_certificate_verifier(verifier);
    let config = match client_auth {
        Some((chain, key)) => builder
            .with_client_auth_cert(chain, key)
            .map_err(|error| client_auth_error(tls, error))?,
        None => builder.with_no_client_auth(),
    };
    Ok(Some(Arc::new(config)))
}

fn ring_provider() -> Arc<rustls::crypto::CryptoProvider> {
    static PROVIDER: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    Arc::clone(PROVIDER.get_or_init(|| rustls::crypto::ring::default_provider().into()))
}

fn client_auth_error(tls: &ResolvedTls, error: RustlsError) -> TlsMaterialError {
    TlsMaterialError::Malformed {
        path: tls
            .client_key_path
            .as_deref()
            .map(display_path)
            .unwrap_or_default(),
        detail: error.to_string(),
    }
}

type ClientAuth = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

fn load_client_auth(tls: &ResolvedTls) -> Result<Option<ClientAuth>, TlsMaterialError> {
    match (&tls.client_cert_path, &tls.client_key_path) {
        (None, None) => Ok(None),
        (Some(cert_path), None) => Err(TlsMaterialError::ClientPairIncomplete {
            present: "certificate",
            path: display_path(cert_path),
        }),
        (None, Some(key_path)) => Err(TlsMaterialError::ClientPairIncomplete {
            present: "key",
            path: display_path(key_path),
        }),
        (Some(cert_path), Some(key_path)) => {
            let chain = load_certs(cert_path)?;
            let key = load_private_key(key_path)?;
            Ok(Some((chain, key)))
        }
    }
}

fn read_pem(path: &Path) -> Result<Vec<u8>, TlsMaterialError> {
    std::fs::read(path).map_err(|error| TlsMaterialError::Unreadable {
        path: display_path(path),
        detail: error.to_string(),
    })
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, TlsMaterialError> {
    let pem = read_pem(path)?;
    let certs = CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| TlsMaterialError::Malformed {
            path: display_path(path),
            detail: error.to_string(),
        })?;
    if certs.is_empty() {
        return Err(TlsMaterialError::Malformed {
            path: display_path(path),
            detail: "no CERTIFICATE section found".into(),
        });
    }
    Ok(certs)
}

fn load_private_key(path: &Path) -> Result<PrivateKeyDer<'static>, TlsMaterialError> {
    let pem = read_pem(path)?;
    if pem_contains(&pem, b"ENCRYPTED PRIVATE KEY") || pem_contains(&pem, b"Proc-Type: 4,ENCRYPTED")
    {
        return Err(TlsMaterialError::ClientKeyEncrypted {
            path: display_path(path),
        });
    }
    PrivateKeyDer::from_pem_slice(&pem).map_err(|error| TlsMaterialError::Malformed {
        path: display_path(path),
        detail: error.to_string(),
    })
}

fn pem_contains(pem: &[u8], needle: &[u8]) -> bool {
    pem.windows(needle.len()).any(|window| window == needle)
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

/// Platform trust store, loaded once per process. Failures log and
/// yield an empty set — verification then rests on the user CA file,
/// which is exactly SQLx's behaviour on the pool path.
struct NativeRoots {
    store: Arc<RootCertStore>,
    /// The accepted roots, PEM-framed once for the libpq bundle.
    pem: String,
}

fn native_roots() -> &'static NativeRoots {
    static ROOTS: OnceLock<NativeRoots> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let mut store = RootCertStore::empty();
        let mut pem = String::new();
        match rustls_native_certs::load_native_certs() {
            Ok(certs) => {
                for cert in certs {
                    let framed_at = pem.len();
                    push_cert_pem(&mut pem, cert.as_ref());
                    if let Err(error) = store.add(cert) {
                        pem.truncate(framed_at);
                        log::warn!("skipping unparsable native root certificate: {error}");
                    }
                }
            }
            Err(error) => log::warn!("could not load platform root certificates: {error}"),
        }
        NativeRoots {
            store: Arc::new(store),
            pem,
        }
    })
}

/// Native roots ∪ the user's CA file (union, not replacement — SQLx
/// cannot replace, and both paths must trust the same set).
fn root_store(root_cert_path: Option<&Path>) -> Result<Arc<RootCertStore>, TlsMaterialError> {
    let Some(path) = root_cert_path else {
        return Ok(Arc::clone(&native_roots().store));
    };
    let mut store = RootCertStore::clone(&native_roots().store);
    for cert in load_certs(path)? {
        store
            .add(cert)
            .map_err(|error| TlsMaterialError::Malformed {
                path: display_path(path),
                detail: error.to_string(),
            })?;
    }
    Ok(Arc::new(store))
}

/// `prefer` / `require`: accept any certificate. Encryption without
/// authentication — never used for the verifying modes.
#[derive(Debug)]
struct NoVerification;

impl ServerCertVerifier for NoVerification {
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
        ring_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// `verify-ca`: full chain verification, hostname mismatch tolerated.
#[derive(Debug)]
struct CaOnlyVerifier {
    inner: Arc<WebPkiServerVerifier>,
}

impl ServerCertVerifier for CaOnlyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(RustlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// ---------------------------------------------------------------------------
// Renderer B — SQLx pool options
// ---------------------------------------------------------------------------

/// Whether the SQLx pool path authenticates the host name for this
/// connection. Disclosed by the diagnosis report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PoolHostnameVerification {
    Full,
    CaOnly,
    NotApplicable,
}

pub(crate) fn pool_hostname_verification(
    tls: &ResolvedTls,
    host: &str,
) -> PoolHostnameVerification {
    if !tls.mode.verifies_hostname() {
        PoolHostnameVerification::NotApplicable
    } else if tls.server_name_differs_from(host) {
        PoolHostnameVerification::CaOnly
    } else {
        PoolHostnameVerification::Full
    }
}

/// SQLx has no `hostaddr`, so a tunneled `verify-full` can only verify the
/// CA chain on the SQLx paths (ADR-0025 §5). Each renderer that downgrades
/// discloses it here, with the same wording.
pub(crate) fn log_sqlx_verify_full_downgrade(
    tls: &ResolvedTls,
    host: &str,
    connection_id: &str,
    path: &str,
) {
    if tls.mode == PgTlsMode::VerifyFull && tls.server_name_differs_from(host) {
        log::warn!(
            "connection {connection_id}: verify-full over a tunnel verifies the CA chain only on the {path} (SQLx has no hostaddr); query sessions verify the host name"
        );
    }
}

pub(crate) fn apply_to_pg_options(
    tls: &ResolvedTls,
    host: &str,
    connection_id: &str,
    mut options: PgConnectOptions,
) -> PgConnectOptions {
    let differs = tls.server_name_differs_from(host);
    log_sqlx_verify_full_downgrade(tls, host, connection_id, "metadata pool");
    options = options.ssl_mode(sqlx_ssl_mode(tls.mode, differs));
    if let Some(path) = &tls.root_cert_path {
        options = options.ssl_root_cert(path);
    }
    if let Some(path) = &tls.client_cert_path {
        options = options.ssl_client_cert(path);
    }
    if let Some(path) = &tls.client_key_path {
        options = options.ssl_client_key(path);
    }
    options
}

// ---------------------------------------------------------------------------
// Renderer C — DSN query string (SQLx-Any fallback)
// ---------------------------------------------------------------------------

/// `sslmode=…` plus the certificate parameters SQLx's URL parser accepts.
/// Percent-encoding is the caller's helper so this module stays free of
/// the dispatch layer.
pub(crate) fn dsn_query(tls: &ResolvedTls, host: &str, encode: impl Fn(&str) -> String) -> String {
    let mode = match tls.mode {
        PgTlsMode::VerifyFull if tls.server_name_differs_from(host) => PgTlsMode::VerifyCa,
        mode => mode,
    };
    let mut query = format!("sslmode={}", mode.as_str());
    for (key, path) in [
        ("sslrootcert", &tls.root_cert_path),
        ("sslcert", &tls.client_cert_path),
        ("sslkey", &tls.client_key_path),
    ] {
        if let Some(path) = path {
            query.push('&');
            query.push_str(key);
            query.push('=');
            query.push_str(&encode(&display_path(path)));
        }
    }
    query
}

// ---------------------------------------------------------------------------
// Renderer D — libpq subprocess environment
// ---------------------------------------------------------------------------

/// Sets `--host` and the `PGSSL*` environment for `pg_dump` /
/// `pg_restore`. Over a tunnel `--host` is the certificate name and
/// `PGHOSTADDR` the loopback address, so libpq verifies the real host
/// name while connecting to the forwarded port.
const LIBPQ_INHERITED_OVERRIDES: &[&str] = &[
    "PGSERVICE",
    "PGSERVICEFILE",
    "PGHOST",
    "PGHOSTADDR",
    "PGPORT",
    "PGDATABASE",
    "PGUSER",
    "PGPASSWORD",
    "PGPASSFILE",
    "PGOPTIONS",
    "PGCONNECT_TIMEOUT",
    "PGSSLMODE",
    "PGREQUIRESSL",
    "PGSSLCOMPRESSION",
    "PGSSLCERT",
    "PGSSLCERTMODE",
    "PGSSLKEY",
    "PGSSLROOTCERT",
    "PGSSLCRL",
    "PGSSLCRLDIR",
    "PGSSLNEGOTIATION",
    "PGSSLMINPROTOCOLVERSION",
    "PGSSLMAXPROTOCOLVERSION",
    "PGSSLSNI",
    "PGCHANNELBINDING",
    "PGREQUIREAUTH",
    "PGTARGETSESSIONATTRS",
    "PGLOADBALANCE",
    "PGGSSENCMODE",
];

/// Keeps generated libpq TLS material alive until its child exits.
pub(crate) struct LibpqTlsMaterial {
    _root_bundle: Option<tempfile::NamedTempFile>,
    _isolated_home: tempfile::TempDir,
}

pub(crate) fn apply_to_command(
    tls: &ResolvedTls,
    host: &str,
    command: &mut Command,
) -> Result<LibpqTlsMaterial, TlsMaterialError> {
    // Validate the same client certificate/key pair the in-process driver
    // consumes. In particular, reject incomplete or encrypted keys before
    // libpq can open a terminal prompt and wait indefinitely. Disabled TLS
    // ignores stale certificate settings, matching the in-process driver.
    if tls.mode != PgTlsMode::Disable {
        let _ = load_client_auth(tls)?;
    }

    // A GUI app may inherit libpq variables when launched from a shell.
    // Make the resolved connection authoritative: in particular, a stale
    // PGHOSTADDR must never redirect a subprocess carrying PGPASSWORD.
    for key in LIBPQ_INHERITED_OVERRIDES {
        command.env_remove(key);
    }
    // GSS encryption is a separate transport from TLS. Leave it disabled so
    // libpq cannot prefer GSS and bypass the resolved sslmode policy.
    command.env("PGGSSENCMODE", "disable");
    // Keep an empty, private directory alive for explicit "not configured"
    // paths. On Unix libpq may locate ~/.postgresql through the passwd
    // database rather than HOME, so changing HOME alone cannot suppress its
    // ambient certificate defaults.
    let isolated_home = tempfile::tempdir().map_err(|error| TlsMaterialError::Unreadable {
        path: "temporary libpq home".into(),
        detail: error.to_string(),
    })?;
    for key in ["HOME", "USERPROFILE", "APPDATA"] {
        command.env(key, isolated_home.path());
    }
    // `--no-password` only suppresses the prompt: an empty PGPASSWORD still
    // makes libpq consult the password file, and it can find ~/.pgpass (or
    // %APPDATA%\postgresql\pgpass.conf) without HOME. Point it at a file
    // that does not exist, which libpq silently ignores.
    command.env(
        "PGPASSFILE",
        isolated_home.path().join("pgpass-not-configured"),
    );
    command.arg("--host").arg(&tls.server_name);
    if tls.server_name_differs_from(host) {
        command.env("PGHOSTADDR", host);
    }
    command.env("PGSSLMODE", tls.mode.as_str());
    let missing_client_cert = isolated_home.path().join("client-cert-not-configured.pem");
    let missing_client_key = isolated_home.path().join("client-key-not-configured.pem");
    let missing_root_cert = isolated_home.path().join("root-cert-not-configured.pem");
    let missing_crl = isolated_home.path().join("crl-not-configured.pem");
    let root_bundle = if tls.mode.verifies_chain() {
        let bundle = libpq_root_bundle(tls.root_cert_path.as_deref())?;
        command.env("PGSSLROOTCERT", bundle.path());
        // libpq can discover ~/.postgresql/root.crl through the passwd
        // database even after HOME is isolated. Explicitly disable both CRL
        // sources so only the resolved trust inputs affect verification.
        command.env("PGSSLCRL", &missing_crl);
        command.env("PGSSLCRLDIR", isolated_home.path());
        Some(bundle)
    } else {
        if tls.mode != PgTlsMode::Disable {
            // `require` silently becomes verify-ca when libpq discovers its
            // default root.crt. An explicit missing path preserves dbunk's
            // non-verifying semantics; libpq treats it as "no root cert".
            command.env("PGSSLROOTCERT", &missing_root_cert);
        }
        None
    };
    if tls.mode != PgTlsMode::Disable {
        for (key, configured, missing) in [
            (
                "PGSSLCERT",
                tls.client_cert_path.as_deref(),
                missing_client_cert.as_path(),
            ),
            (
                "PGSSLKEY",
                tls.client_key_path.as_deref(),
                missing_client_key.as_path(),
            ),
        ] {
            // Explicitly point at a guaranteed-missing file instead of
            // allowing libpq to consult ~/.postgresql/postgresql.{crt,key}.
            command.env(key, configured.unwrap_or(missing));
        }
    }
    Ok(LibpqTlsMaterial {
        _root_bundle: root_bundle,
        _isolated_home: isolated_home,
    })
}

/// libpq accepts one root file, while the in-process drivers use native
/// roots plus the optional user CA. Materialize that same union as a
/// short-lived PEM bundle rather than replacing either trust source.
fn libpq_root_bundle(
    root_cert_path: Option<&Path>,
) -> Result<tempfile::NamedTempFile, TlsMaterialError> {
    use std::io::Write as _;

    let mut user_roots = String::new();
    if let Some(path) = root_cert_path {
        for cert in load_certs(path)? {
            push_cert_pem(&mut user_roots, cert.as_ref());
        }
    }
    let write = || -> std::io::Result<tempfile::NamedTempFile> {
        let mut bundle = tempfile::NamedTempFile::new()?;
        bundle.write_all(native_roots().pem.as_bytes())?;
        bundle.write_all(user_roots.as_bytes())?;
        bundle.flush()?;
        Ok(bundle)
    };
    write().map_err(|error| TlsMaterialError::Unreadable {
        path: "temporary libpq CA bundle".into(),
        detail: error.to_string(),
    })
}

/// RFC 7468 framing of one DER certificate, as libpq/OpenSSL read it.
fn push_cert_pem(pem: &mut String, certificate: &[u8]) {
    let encoded = BASE64.encode(certificate);
    pem.push_str("-----BEGIN CERTIFICATE-----\n");
    let mut rest = encoded.as_str();
    while !rest.is_empty() {
        // base64 output is ASCII, so any byte offset is a char boundary.
        let (line, tail) = rest.split_at(rest.len().min(64));
        pem.push_str(line);
        pem.push('\n');
        rest = tail;
    }
    pem.push_str("-----END CERTIFICATE-----\n");
}

#[cfg(test)]
pub(crate) fn testdata(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/postgres/testdata")
        .join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PgTlsOptions;

    fn pg(ssl: bool, tls_options: Option<PgTlsOptions>) -> PgStoredConnection {
        PgStoredConnection {
            organization: Default::default(),
            id: "c".into(),
            name: "c".into(),
            database: "db".into(),
            host: "db.internal".into(),
            port: 5432,
            user: "u".into(),
            password: String::new(),
            role: String::new(),
            environment: Default::default(),
            safe_mode: Default::default(),
            read_only: false,
            last_activity_at: None,
            ssl,
            tls_options,
            driver_options: None,
            ssh_tunnel: Default::default(),
        }
    }

    #[test]
    fn legacy_rows_resolve_through_ssl() {
        assert_eq!(
            ResolvedTls::from_postgres(&pg(true, None)).mode,
            PgTlsMode::Prefer
        );
        assert_eq!(
            ResolvedTls::from_postgres(&pg(false, None)).mode,
            PgTlsMode::Disable
        );
    }

    #[test]
    fn tls_options_win_over_ssl_and_blank_paths_are_none() {
        let resolved = ResolvedTls::from_postgres(&pg(
            false,
            Some(PgTlsOptions {
                mode: PgTlsMode::VerifyFull,
                root_cert_path: Some("  ".into()),
                client_cert_path: Some("/c.pem".into()),
                client_key_path: None,
                server_name: Some(" db.example ".into()),
            }),
        ));
        assert_eq!(resolved.mode, PgTlsMode::VerifyFull);
        assert_eq!(resolved.server_name, "db.example");
        assert_eq!(resolved.root_cert_path, None);
        assert_eq!(resolved.client_cert_path, Some(PathBuf::from("/c.pem")));
        assert!(resolved.server_name_differs_from("db.internal"));
        assert!(
            !ResolvedTls::from_postgres(&pg(true, None)).server_name_differs_from("db.internal")
        );
    }

    #[test]
    fn mode_tables() {
        assert!(matches!(
            tokio_ssl_mode(PgTlsMode::Disable),
            SslMode::Disable
        ));
        assert!(matches!(tokio_ssl_mode(PgTlsMode::Prefer), SslMode::Prefer));
        assert!(matches!(
            tokio_ssl_mode(PgTlsMode::Require),
            SslMode::Require
        ));
        assert!(matches!(
            tokio_ssl_mode(PgTlsMode::VerifyCa),
            SslMode::Require
        ));
        assert!(matches!(
            tokio_ssl_mode(PgTlsMode::VerifyFull),
            SslMode::Require
        ));
        assert!(matches!(
            sqlx_ssl_mode(PgTlsMode::Disable, false),
            PgSslMode::Disable
        ));
        assert!(matches!(
            sqlx_ssl_mode(PgTlsMode::Prefer, true),
            PgSslMode::Prefer
        ));
        assert!(matches!(
            sqlx_ssl_mode(PgTlsMode::Require, true),
            PgSslMode::Require
        ));
        assert!(matches!(
            sqlx_ssl_mode(PgTlsMode::VerifyCa, true),
            PgSslMode::VerifyCa
        ));
        assert!(matches!(
            sqlx_ssl_mode(PgTlsMode::VerifyFull, false),
            PgSslMode::VerifyFull
        ));
        assert_eq!(PgTlsMode::VerifyCa.as_str(), "verify-ca");
        assert_eq!(PgTlsMode::VerifyFull.as_str(), "verify-full");
    }

    #[test]
    fn sqlx_downgrades_verify_full_only_when_the_server_name_differs() {
        assert!(matches!(
            sqlx_ssl_mode(PgTlsMode::VerifyFull, true),
            PgSslMode::VerifyCa
        ));
        let full = ResolvedTls::with_mode("db.internal", PgTlsMode::VerifyFull);
        assert_eq!(
            pool_hostname_verification(&full, "db.internal"),
            PoolHostnameVerification::Full
        );
        assert_eq!(
            pool_hostname_verification(&full, "127.0.0.1"),
            PoolHostnameVerification::CaOnly
        );
        let ca = ResolvedTls::with_mode("db.internal", PgTlsMode::VerifyCa);
        assert_eq!(
            pool_hostname_verification(&ca, "127.0.0.1"),
            PoolHostnameVerification::NotApplicable
        );
    }

    #[test]
    fn dsn_query_renders_mode_and_encoded_paths() {
        let mut tls = ResolvedTls::with_mode("h", PgTlsMode::VerifyCa);
        assert_eq!(dsn_query(&tls, "h", |s| s.to_string()), "sslmode=verify-ca");
        tls.root_cert_path = Some("/a b/ca.pem".into());
        tls.client_cert_path = Some("/c.pem".into());
        tls.client_key_path = Some("/k.pem".into());
        assert_eq!(
            dsn_query(&tls, "h", |s| s.replace(' ', "%20")),
            "sslmode=verify-ca&sslrootcert=/a%20b/ca.pem&sslcert=/c.pem&sslkey=/k.pem"
        );
    }

    #[test]
    fn dsn_downgrades_tunneled_verify_full_like_pg_options() {
        let tls = ResolvedTls::with_mode("db.internal", PgTlsMode::VerifyFull);
        assert_eq!(
            dsn_query(&tls, "127.0.0.1", |s| s.to_string()),
            "sslmode=verify-ca"
        );
        assert_eq!(
            dsn_query(&tls, "db.internal", |s| s.to_string()),
            "sslmode=verify-full"
        );
    }

    #[test]
    fn command_gets_host_hostaddr_and_pgssl_env() {
        let mut tls = ResolvedTls::with_mode("db.internal", PgTlsMode::VerifyFull);
        tls.root_cert_path = Some(testdata("ca.pem"));
        let mut command = Command::new("pg_dump");
        let material = apply_to_command(&tls, "127.0.0.1", &mut command).unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--host", "db.internal"]);
        let env: Vec<(String, Option<String>)> = command
            .get_envs()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(env.contains(&("PGHOSTADDR".into(), Some("127.0.0.1".into()))));
        assert!(env.contains(&("PGSSLMODE".into(), Some("verify-full".into()))));
        let root_bundle = PathBuf::from(
            env.iter()
                .find_map(|(key, value)| (key == "PGSSLROOTCERT").then_some(value.as_ref()))
                .flatten()
                .expect("generated root bundle"),
        );
        let certificates = CertificateDer::pem_file_iter(&root_bundle)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(certificates.len(), native_roots().store.len() + 1);
        for key in ["PGSSLCERT", "PGSSLKEY", "PGSSLCRL", "PGPASSFILE"] {
            let path = env
                .iter()
                .find_map(|(candidate, value)| (candidate == key).then_some(value.as_ref()))
                .flatten()
                .map(PathBuf::from)
                .unwrap_or_else(|| panic!("{key} must be explicit"));
            assert!(!path.exists(), "{key} must point to a missing file");
        }
        let crl_dir = env
            .iter()
            .find_map(|(key, value)| (key == "PGSSLCRLDIR").then_some(value.as_ref()))
            .flatten()
            .map(PathBuf::from)
            .expect("isolated CRL directory");
        assert!(crl_dir.is_dir());
        assert!(crl_dir.read_dir().unwrap().next().is_none());
        assert!(root_bundle.exists());
        drop(material);
        assert!(!root_bundle.exists());
        assert!(!crl_dir.exists());

        let mut plain = Command::new("pg_dump");
        let _material = apply_to_command(&ResolvedTls::plain("h"), "h", &mut plain).unwrap();
        assert!(!plain
            .get_envs()
            .any(|(k, value)| k == "PGHOSTADDR" && value.is_some()));
    }

    #[test]
    fn command_clears_inherited_connection_and_certificate_overrides() {
        let mut command = Command::new("pg_dump");
        for key in LIBPQ_INHERITED_OVERRIDES {
            command.env(key, "/ambient/value");
        }

        let _material = apply_to_command(
            &ResolvedTls::prefer("db.internal"),
            "db.internal",
            &mut command,
        )
        .unwrap();

        for key in LIBPQ_INHERITED_OVERRIDES {
            assert!(command
                .get_envs()
                .filter(|(candidate, _)| *candidate == *key)
                .all(|(_, value)| value.is_none_or(|value| value != "/ambient/value")));
        }
        for key in [
            "PGSERVICE",
            "PGSERVICEFILE",
            "PGSSLCERTMODE",
            "PGSSLCRL",
            "PGSSLCRLDIR",
            "PGSSLNEGOTIATION",
            "PGSSLMINPROTOCOLVERSION",
            "PGSSLMAXPROTOCOLVERSION",
        ] {
            assert!(command
                .get_envs()
                .any(|(candidate, value)| candidate == key && value.is_none()));
        }
        assert!(command
            .get_envs()
            .any(|(candidate, value)| candidate == "PGGSSENCMODE"
                && value == Some("disable".as_ref())));
    }

    #[test]
    fn command_isolates_libpq_default_files_and_keeps_home_alive() {
        let ambient_home = tempfile::tempdir().unwrap();
        let ambient_postgresql = ambient_home.path().join(".postgresql");
        std::fs::create_dir(&ambient_postgresql).unwrap();
        for name in ["postgresql.crt", "postgresql.key", "root.crt", "root.crl"] {
            std::fs::write(ambient_postgresql.join(name), "ambient material").unwrap();
        }
        let mut command = Command::new("pg_dump");
        command.env("HOME", ambient_home.path());

        let material = apply_to_command(
            &ResolvedTls::with_mode("db.internal", PgTlsMode::Require),
            "db.internal",
            &mut command,
        )
        .unwrap();

        let isolated_home = command
            .get_envs()
            .find_map(|(key, value)| (key == "HOME").then(|| value.unwrap()))
            .map(PathBuf::from)
            .expect("isolated libpq home");
        assert_ne!(isolated_home, ambient_home.path());
        assert!(isolated_home.is_dir());
        assert!(!isolated_home.join(".postgresql/postgresql.crt").exists());
        assert!(!isolated_home.join(".postgresql/postgresql.key").exists());
        assert!(!isolated_home.join(".postgresql/root.crt").exists());
        for key in ["PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT", "PGPASSFILE"] {
            let path = command
                .get_envs()
                .find_map(|(candidate, value)| (candidate == key).then(|| value.unwrap()))
                .map(PathBuf::from)
                .unwrap_or_else(|| panic!("{key} must be explicit"));
            assert!(path.starts_with(&isolated_home));
            assert!(!path.exists());
        }

        drop(material);
        assert!(
            !isolated_home.exists(),
            "temporary home must live exactly as long as the command material"
        );
    }

    #[test]
    fn command_blocks_libpq_default_tls_material_with_explicit_missing_paths() {
        for mode in [PgTlsMode::Prefer, PgTlsMode::Require] {
            let mut tls = ResolvedTls::with_mode("h", mode);
            tls.root_cert_path = Some("/ambient/root.crt".into());
            let mut command = Command::new("pg_dump");

            let _material = apply_to_command(&tls, "h", &mut command).unwrap();

            for key in ["PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT"] {
                let path = command
                    .get_envs()
                    .find_map(|(candidate, value)| (candidate == key).then(|| value.unwrap()))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| panic!("{mode:?} must set {key}"));
                assert!(path.is_absolute(), "{key} must be an absolute path");
                assert!(!path.exists(), "{key} must point to a missing file");
            }
        }
    }

    #[test]
    fn command_only_enables_explicit_crl_isolation_for_verifying_modes() {
        for mode in [PgTlsMode::Disable, PgTlsMode::Prefer, PgTlsMode::Require] {
            let mut command = Command::new("pg_dump");
            command.env("PGSSLCRL", "/ambient/root.crl");
            command.env("PGSSLCRLDIR", "/ambient/crls");

            let _material =
                apply_to_command(&ResolvedTls::with_mode("h", mode), "h", &mut command).unwrap();

            for key in ["PGSSLCRL", "PGSSLCRLDIR"] {
                assert!(command
                    .get_envs()
                    .any(|(candidate, value)| candidate == key && value.is_none()));
            }
        }

        for mode in [PgTlsMode::VerifyCa, PgTlsMode::VerifyFull] {
            let mut command = Command::new("pg_dump");
            let material =
                apply_to_command(&ResolvedTls::with_mode("h", mode), "h", &mut command).unwrap();
            let crl = command
                .get_envs()
                .find_map(|(key, value)| (key == "PGSSLCRL").then(|| value.unwrap()))
                .map(PathBuf::from)
                .expect("explicit missing CRL path");
            let crl_dir = command
                .get_envs()
                .find_map(|(key, value)| (key == "PGSSLCRLDIR").then(|| value.unwrap()))
                .map(PathBuf::from)
                .expect("explicit empty CRL directory");

            assert!(crl.starts_with(&crl_dir));
            assert!(!crl.exists());
            assert!(crl_dir.is_dir());
            assert!(crl_dir.read_dir().unwrap().next().is_none());

            drop(material);
            assert!(!crl_dir.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn command_keeps_missing_tls_material_directory_alive_through_child() {
        let mut command = Command::new("sh");
        command.arg("-c").arg(
            r#"test -d "${PGSSLCERT%/*}" &&
               test ! -e "$PGSSLCERT" &&
               test ! -e "$PGSSLKEY" &&
               test -s "$PGSSLROOTCERT" &&
               test -n "$PGSSLCRL" &&
               test ! -e "$PGSSLCRL" &&
               test -d "$PGSSLCRLDIR" &&
               test "$PGSSLCRLDIR" = "${PGSSLCRL%/*}""#,
        );
        let material = apply_to_command(
            &ResolvedTls::with_mode("h", PgTlsMode::VerifyCa),
            "h",
            &mut command,
        )
        .unwrap();
        let material_dir = command
            .get_envs()
            .find_map(|(key, value)| (key == "PGSSLCERT").then(|| value.unwrap()))
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(PathBuf::from))
            .expect("material directory");

        let output = command.output().unwrap();

        assert!(
            output.status.success(),
            "child could not read live TLS material paths: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(material_dir.is_dir());
        drop(material);
        assert!(!material_dir.exists());
    }

    #[test]
    fn command_preserves_configured_client_material() {
        let mut tls = ResolvedTls::prefer("h");
        tls.client_cert_path = Some(testdata("client.pem"));
        tls.client_key_path = Some(testdata("client-key.pem"));
        let mut command = Command::new("pg_dump");

        let _material = apply_to_command(&tls, "h", &mut command).unwrap();

        for (key, expected) in [
            ("PGSSLCERT", tls.client_cert_path.as_ref().unwrap()),
            ("PGSSLKEY", tls.client_key_path.as_ref().unwrap()),
        ] {
            let actual = command
                .get_envs()
                .find_map(|(candidate, value)| (candidate == key).then(|| value.unwrap()));
            assert_eq!(actual, Some(expected.as_os_str()));
        }
    }

    #[test]
    fn command_disable_does_not_enable_tls_material() {
        let mut command = Command::new("pg_dump");
        for key in ["PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT"] {
            command.env(key, "/ambient/material");
        }

        let _material = apply_to_command(&ResolvedTls::plain("h"), "h", &mut command).unwrap();

        assert!(command
            .get_envs()
            .any(|(key, value)| key == "PGSSLMODE" && value == Some("disable".as_ref())));
        for key in ["PGSSLCERT", "PGSSLKEY", "PGSSLROOTCERT"] {
            assert!(command
                .get_envs()
                .any(|(candidate, value)| candidate == key && value.is_none()));
        }
    }

    #[test]
    fn disable_yields_no_config_and_verifying_modes_build_with_fixture_ca() {
        assert!(client_config(&ResolvedTls::plain("h")).unwrap().is_none());
        assert!(client_config(&ResolvedTls::prefer("h")).unwrap().is_some());
        let mut tls = ResolvedTls::with_mode("localhost", PgTlsMode::VerifyFull);
        tls.root_cert_path = Some(testdata("ca.pem"));
        assert!(client_config(&tls).unwrap().is_some());
        tls.mode = PgTlsMode::VerifyCa;
        tls.client_cert_path = Some(testdata("client.pem"));
        tls.client_key_path = Some(testdata("client-key.pem"));
        assert!(client_config(&tls).unwrap().is_some());
    }

    #[test]
    fn client_pair_must_be_complete() {
        let mut tls = ResolvedTls::prefer("h");
        tls.client_cert_path = Some(testdata("client.pem"));
        let error = client_config(&tls).unwrap_err();
        assert!(matches!(
            error,
            TlsMaterialError::ClientPairIncomplete {
                present: "certificate",
                ref path,
            }
            if path.ends_with("client.pem")
        ));
        assert!(error.to_string().contains("client.pem"));
        tls.client_cert_path = None;
        tls.client_key_path = Some(testdata("client-key.pem"));
        let error = client_config(&tls).unwrap_err();
        assert!(matches!(
            error,
            TlsMaterialError::ClientPairIncomplete {
                present: "key",
                ref path,
            }
            if path.ends_with("client-key.pem")
        ));
        assert!(error.to_string().contains("client-key.pem"));
    }

    #[test]
    fn encrypted_keys_are_refused_by_path() {
        let mut tls = ResolvedTls::prefer("h");
        tls.client_cert_path = Some(testdata("client.pem"));
        tls.client_key_path = Some(testdata("client-key-encrypted.pem"));
        let error = client_config(&tls).unwrap_err();
        assert!(
            matches!(error, TlsMaterialError::ClientKeyEncrypted { ref path } if path.ends_with("client-key-encrypted.pem"))
        );
        assert!(error.to_string().contains("passphrase"));
    }

    #[test]
    fn command_validates_client_material_before_spawn() {
        let mut incomplete = ResolvedTls::prefer("h");
        incomplete.client_cert_path = Some(testdata("client.pem"));
        let error = apply_to_command(&incomplete, "h", &mut Command::new("pg_dump"))
            .err()
            .expect("incomplete pair must fail");
        assert!(matches!(
            error,
            TlsMaterialError::ClientPairIncomplete {
                present: "certificate",
                ..
            }
        ));

        let mut encrypted = ResolvedTls::prefer("h");
        encrypted.client_cert_path = Some(testdata("client.pem"));
        encrypted.client_key_path = Some(testdata("client-key-encrypted.pem"));
        let error = apply_to_command(&encrypted, "h", &mut Command::new("pg_dump"))
            .err()
            .expect("encrypted key must fail");
        assert!(matches!(error, TlsMaterialError::ClientKeyEncrypted { .. }));
    }

    #[test]
    fn command_disabled_tls_ignores_stale_client_material() {
        let mut incomplete = ResolvedTls::plain("h");
        incomplete.client_cert_path = Some(testdata("does-not-exist.pem"));

        let mut encrypted = ResolvedTls::plain("h");
        encrypted.client_cert_path = Some(testdata("client.pem"));
        encrypted.client_key_path = Some(testdata("client-key-encrypted.pem"));

        for tls in [incomplete, encrypted] {
            let mut command = Command::new("pg_dump");
            command.env("PGSSLCERT", "/ambient/client.pem");
            command.env("PGSSLKEY", "/ambient/client-key.pem");

            let _material = apply_to_command(&tls, "h", &mut command).unwrap();

            for key in ["PGSSLCERT", "PGSSLKEY"] {
                assert!(command
                    .get_envs()
                    .any(|(candidate, value)| candidate == key && value.is_none()));
            }
        }
    }

    #[test]
    fn missing_and_malformed_files_name_the_path() {
        let mut tls = ResolvedTls::with_mode("h", PgTlsMode::VerifyCa);
        tls.root_cert_path = Some(testdata("does-not-exist.pem"));
        assert!(matches!(
            client_config(&tls).unwrap_err(),
            TlsMaterialError::Unreadable { ref path, .. } if path.ends_with("does-not-exist.pem")
        ));
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.pem");
        std::fs::write(&junk, "not a certificate").unwrap();
        tls.root_cert_path = Some(junk.clone());
        assert!(matches!(
            client_config(&tls).unwrap_err(),
            TlsMaterialError::Malformed { ref path, .. } if path == &junk.display().to_string()
        ));
    }
}
