//! Attribute a tokio-postgres connect failure to a stage (ADR-0025).
//!
//! `tokio_postgres::Error` has no public constructors, so the
//! classification runs over a small extracted [`ConnectErrorView`] that
//! tests can build by hand. [`view_of`] walks the error's source chain
//! for the rustls / io details; [`classify`] is the pure decision.

use std::error::Error as StdError;
use std::io;

use crate::TlsFailureKind;

/// What the connect attempt actually reported, flattened.
#[derive(Debug, Clone, Default)]
pub(crate) struct ConnectErrorView {
    pub sqlstate: Option<String>,
    pub db_message: Option<String>,
    pub db_severity: Option<String>,
    pub rustls: Option<rustls::Error>,
    pub io_kind: Option<io::ErrorKind>,
    /// Full `Display` text, used only for the few string-shaped cases
    /// tokio-postgres exposes ("server does not support TLS").
    pub message: String,
}

/// Stage-attributed failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectFailure {
    Tls(TlsFailureKind),
    /// SQLSTATE 28000 / 28P01.
    Authentication {
        message: String,
    },
    /// SQLSTATE 3D000.
    DatabaseMissing {
        message: String,
    },
    /// Any other error the server sent during startup.
    Database {
        code: Option<String>,
        message: String,
        severity: Option<String>,
    },
    /// Socket-level failure before or instead of a handshake.
    Io(io::ErrorKind, String),
    Other(String),
}

/// `Display` of the error followed by every cause in its source chain —
/// tokio-postgres prints only its own kind ("error performing TLS
/// handshake"), so the detail lives in the sources.
fn chained_message(error: &(dyn StdError + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(current) = source {
        let text = current.to_string();
        if !text.is_empty() && !message.ends_with(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = current.source();
    }
    message
}

pub(crate) fn view_of(error: &tokio_postgres::Error) -> ConnectErrorView {
    let mut view = ConnectErrorView {
        message: chained_message(error),
        ..Default::default()
    };
    if let Some(db) = error.as_db_error() {
        view.sqlstate = Some(db.code().code().to_string());
        view.db_message = Some(db.message().to_string());
        view.db_severity = Some(db.severity().to_string());
        return view;
    }
    let mut source: Option<&(dyn StdError + 'static)> = error.source();
    while let Some(current) = source {
        if let Some(rustls_error) = current.downcast_ref::<rustls::Error>() {
            view.rustls = Some(rustls_error.clone());
            break;
        }
        if let Some(io_error) = current.downcast_ref::<io::Error>() {
            if view.io_kind.is_none() {
                view.io_kind = Some(io_error.kind());
            }
            // tokio-rustls wraps rustls errors in `io::Error(InvalidData)`;
            // `get_ref` exposes the inner error without consuming it.
            if let Some(inner) = io_error.get_ref() {
                if let Some(rustls_error) = inner.downcast_ref::<rustls::Error>() {
                    view.rustls = Some(rustls_error.clone());
                    break;
                }
            }
        }
        source = current.source();
    }
    view
}

/// View of a bare `io::Error`, as returned by the TLS connector when the
/// diagnosis performs the handshake itself. tokio-rustls wraps rustls
/// errors in `io::Error(InvalidData)`.
pub(crate) fn view_of_io(error: &io::Error) -> ConnectErrorView {
    let mut view = ConnectErrorView {
        io_kind: Some(error.kind()),
        message: chained_message(error),
        ..Default::default()
    };
    let mut source: Option<&(dyn StdError + 'static)> = error.get_ref().map(|e| e as _);
    while let Some(current) = source {
        if let Some(rustls_error) = current.downcast_ref::<rustls::Error>() {
            view.rustls = Some(rustls_error.clone());
            break;
        }
        source = current.source();
    }
    view
}

/// `client_cert_configured` decides whether a handshake alert is read as
/// the server rejecting our certificate or as a generic failure.
pub(crate) fn classify(view: &ConnectErrorView, client_cert_configured: bool) -> ConnectFailure {
    if let Some(code) = &view.sqlstate {
        let message = view
            .db_message
            .clone()
            .unwrap_or_else(|| view.message.clone());
        return match code.as_str() {
            "28P01" | "28000" => ConnectFailure::Authentication { message },
            "3D000" => ConnectFailure::DatabaseMissing { message },
            _ => ConnectFailure::Database {
                code: Some(code.clone()),
                message,
                severity: view.db_severity.clone(),
            },
        };
    }
    if let Some(rustls_error) = &view.rustls {
        use rustls::CertificateError as CE;
        return ConnectFailure::Tls(match rustls_error {
            rustls::Error::InvalidCertificate(CE::NotValidForName)
            | rustls::Error::InvalidCertificate(CE::NotValidForNameContext { .. }) => {
                TlsFailureKind::HostnameMismatch
            }
            rustls::Error::InvalidCertificate(_) => TlsFailureKind::CertificateUntrusted,
            rustls::Error::AlertReceived(_) if client_cert_configured => {
                TlsFailureKind::ClientCertificateRejected
            }
            _ => TlsFailureKind::HandshakeFailed,
        });
    }
    let lower = view.message.to_ascii_lowercase();
    if lower.contains("server does not support tls") {
        return ConnectFailure::Tls(TlsFailureKind::ServerRefusedTls);
    }
    if lower.contains("error performing tls handshake") {
        return ConnectFailure::Tls(TlsFailureKind::HandshakeFailed);
    }
    if let Some(kind) = view.io_kind {
        return ConnectFailure::Io(kind, view.message.clone());
    }
    ConnectFailure::Other(view.message.clone())
}

pub(crate) fn tls_failure_message(kind: TlsFailureKind, detail: &str) -> String {
    let headline = match kind {
        TlsFailureKind::ServerRefusedTls => "The server does not support TLS on this port",
        TlsFailureKind::CertificateUntrusted => "The server certificate is not trusted",
        TlsFailureKind::HostnameMismatch => {
            "The server certificate does not match the expected host name"
        }
        TlsFailureKind::ClientCertificateRejected => "The server rejected the client certificate",
        TlsFailureKind::InvalidLocalMaterial => "Local certificate material is invalid",
        TlsFailureKind::HandshakeFailed => "The TLS handshake failed",
    };
    if detail.is_empty() {
        headline.to_string()
    } else {
        format!("{headline}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{AlertDescription, CertificateError};

    fn view(message: &str) -> ConnectErrorView {
        ConnectErrorView {
            message: message.into(),
            ..Default::default()
        }
    }

    #[test]
    fn sqlstate_wins_over_everything_else() {
        let mut v = view("db error");
        v.sqlstate = Some("28P01".into());
        v.db_message = Some("password authentication failed".into());
        v.rustls = Some(rustls::Error::AlertReceived(
            AlertDescription::BadCertificate,
        ));
        assert_eq!(
            classify(&v, true),
            ConnectFailure::Authentication {
                message: "password authentication failed".into()
            }
        );
        v.sqlstate = Some("28000".into());
        assert!(matches!(
            classify(&v, false),
            ConnectFailure::Authentication { .. }
        ));
        v.sqlstate = Some("3D000".into());
        assert!(matches!(
            classify(&v, false),
            ConnectFailure::DatabaseMissing { .. }
        ));
        v.sqlstate = Some("53300".into());
        v.db_severity = Some("FATAL".into());
        assert_eq!(
            classify(&v, false),
            ConnectFailure::Database {
                code: Some("53300".into()),
                message: "password authentication failed".into(),
                severity: Some("FATAL".into()),
            }
        );
    }

    #[test]
    fn rustls_errors_map_to_tls_kinds() {
        let mut v = view("error performing TLS handshake");
        v.rustls = Some(rustls::Error::InvalidCertificate(
            CertificateError::NotValidForName,
        ));
        assert_eq!(
            classify(&v, false),
            ConnectFailure::Tls(TlsFailureKind::HostnameMismatch)
        );
        v.rustls = Some(rustls::Error::InvalidCertificate(
            CertificateError::UnknownIssuer,
        ));
        assert_eq!(
            classify(&v, false),
            ConnectFailure::Tls(TlsFailureKind::CertificateUntrusted)
        );
        v.rustls = Some(rustls::Error::InvalidCertificate(CertificateError::Expired));
        assert_eq!(
            classify(&v, false),
            ConnectFailure::Tls(TlsFailureKind::CertificateUntrusted)
        );
        v.rustls = Some(rustls::Error::AlertReceived(
            AlertDescription::BadCertificate,
        ));
        assert_eq!(
            classify(&v, true),
            ConnectFailure::Tls(TlsFailureKind::ClientCertificateRejected)
        );
        assert_eq!(
            classify(&v, false),
            ConnectFailure::Tls(TlsFailureKind::HandshakeFailed)
        );
        v.rustls = Some(rustls::Error::NoCertificatesPresented);
        assert_eq!(
            classify(&v, true),
            ConnectFailure::Tls(TlsFailureKind::HandshakeFailed)
        );
    }

    #[test]
    fn string_shaped_and_io_cases() {
        assert_eq!(
            classify(
                &view("error performing TLS handshake: server does not support TLS"),
                false
            ),
            ConnectFailure::Tls(TlsFailureKind::ServerRefusedTls)
        );
        assert_eq!(
            classify(&view("error performing TLS handshake: boom"), false),
            ConnectFailure::Tls(TlsFailureKind::HandshakeFailed)
        );
        let mut v = view("error connecting to server: Connection refused (os error 61)");
        v.io_kind = Some(io::ErrorKind::ConnectionRefused);
        assert!(matches!(
            classify(&v, false),
            ConnectFailure::Io(io::ErrorKind::ConnectionRefused, _)
        ));
        assert!(matches!(
            classify(&view("weird"), false),
            ConnectFailure::Other(_)
        ));
    }

    #[test]
    fn messages_carry_the_headline_and_detail() {
        assert_eq!(
            tls_failure_message(TlsFailureKind::HostnameMismatch, ""),
            "The server certificate does not match the expected host name"
        );
        assert!(
            tls_failure_message(TlsFailureKind::CertificateUntrusted, "UnknownIssuer")
                .ends_with(": UnknownIssuer")
        );
    }
}
