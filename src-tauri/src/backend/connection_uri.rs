//! Bounded PostgreSQL URI prefill and secret-free export. These pure functions
//! never resolve credentials, files, endpoints, profile authority or sockets.

use super::{DevelopmentPostgresConnection, DevelopmentTlsMode};
use std::{fmt, net::Ipv6Addr};

pub const MAX_URI_BYTES: usize = 16 * 1024;
pub const MAX_EXPORT_URI_BYTES: usize = 4 * 1024;
const MAX_FIELD_BYTES: usize = 256;
const MAX_PASSWORD_BYTES: usize = 4 * 1024;
const MAX_QUERY_PAIRS: usize = 32;
const MAX_QUERY_KEY_BYTES: usize = 128;

pub struct ParsedPostgresUri {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    /// Absent and explicitly empty URI passwords leave a typed password alone.
    pub password: Option<String>,
    /// None preserves the form's current TLS mode.
    pub tls_mode: Option<DevelopmentTlsMode>,
    /// Ordered unique decoded names only. Ignored values are never returned.
    pub ignored_params: Vec<String>,
}

impl fmt::Debug for ParsedPostgresUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedPostgresUri")
            .field("host_bytes", &self.host.len())
            .field("user_bytes", &self.user.len())
            .field("database_bytes", &self.database.len())
            .field("has_password", &self.password.is_some())
            .field("ignored_parameter_count", &self.ignored_params.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UriOmissions {
    pub tls_files: bool,
    pub tls_server_name: bool,
    pub driver_options: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExportedPostgresUri {
    pub uri: String,
    pub omissions: UriOmissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UriError {
    Empty,
    TooLarge,
    UnsupportedScheme,
    Malformed,
    MissingHost,
    UnsupportedHost,
    InvalidPort,
    InvalidEncoding,
    InvalidCharacter,
    FieldTooLong,
    PasswordTooLong,
    TooManyQueryParameters,
    QueryKeyTooLong,
    ExtraPath,
    Fragment,
    ConflictingTlsModes,
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "Paste a PostgreSQL connection URI first",
            Self::TooLarge => "Connection URI exceeds its byte limit",
            Self::UnsupportedScheme => "Use postgres:// or postgresql://",
            Self::Malformed => "Malformed PostgreSQL connection URI",
            Self::MissingHost => "Add a host to apply the URI",
            Self::UnsupportedHost => {
                "Use one hostname or IPv6 address; socket and multiple-host URIs are unsupported"
            }
            Self::InvalidPort => "PostgreSQL port must be between 1 and 65535",
            Self::InvalidEncoding => "URI percent escapes must encode valid UTF-8",
            Self::InvalidCharacter => "Connection URI fields cannot contain control characters",
            Self::FieldTooLong => "URI host, user and database must each fit 256 bytes",
            Self::PasswordTooLong => "URI password exceeds 4096 bytes",
            Self::TooManyQueryParameters => "Connection URI has more than 32 query parameters",
            Self::QueryKeyTooLong => "URI query parameter name exceeds 128 bytes",
            Self::ExtraPath => "Use one database path segment; encode a database slash as %2F",
            Self::Fragment => "Connection URI fragments are unsupported",
            Self::ConflictingTlsModes => "Connection URI contains conflicting sslmode values",
        })
    }
}

impl std::error::Error for UriError {}

pub fn parse_postgres_uri(input: &str) -> Result<ParsedPostgresUri, UriError> {
    if input.len() > MAX_URI_BYTES {
        return Err(UriError::TooLarge);
    }
    reject_controls(input)?;
    let input = input.trim();
    if input.is_empty() {
        return Err(UriError::Empty);
    }
    let (scheme, tail) = input.split_once("://").ok_or(UriError::Malformed)?;
    if !scheme.eq_ignore_ascii_case("postgres") && !scheme.eq_ignore_ascii_case("postgresql") {
        return Err(UriError::UnsupportedScheme);
    }
    if input.contains('#') {
        return Err(UriError::Fragment);
    }
    // Validate even ignored query values. URL's query iterator otherwise uses
    // lossy UTF-8 decoding, which must not quietly rewrite pasted input.
    decode_component(input)?;
    let (location, query) = tail.split_once('?').unwrap_or((tail, ""));
    if query.split('&').filter(|pair| !pair.is_empty()).count() > MAX_QUERY_PAIRS {
        return Err(UriError::TooManyQueryParameters);
    }
    let (authority, path) = location.split_once('/').unwrap_or((location, ""));
    if path.contains('/') {
        return Err(UriError::ExtraPath);
    }
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = parse_authority(host_port)?;
    let url = reqwest::Url::parse(input).map_err(|_| UriError::Malformed)?;
    // Non-special URL hosts can be percent-encoded by the parser. Decode its
    // representation only to check equivalence; retain the selected host.
    if !host.contains(':')
        && decode_component(url.host_str().ok_or(UriError::MissingHost)?)? != host
    {
        return Err(UriError::UnsupportedHost);
    }
    let user = decode_component(url.username())?;
    let database = decode_component(path)?;
    check_field(&user)?;
    check_field(&database)?;
    let password = url
        .password()
        .filter(|value| !value.is_empty())
        .map(decode_component)
        .transpose()?;
    if password
        .as_ref()
        .is_some_and(|value| value.len() > MAX_PASSWORD_BYTES)
    {
        return Err(UriError::PasswordTooLong);
    }
    let mut tls_mode = None;
    let mut first_sslmode = None;
    let mut ignored_params = Vec::new();
    for (name, value) in url.query_pairs() {
        if name.len() > MAX_QUERY_KEY_BYTES {
            return Err(UriError::QueryKeyTooLong);
        }
        if name == "sslmode" {
            if first_sslmode
                .as_ref()
                .is_some_and(|previous| previous != &value)
            {
                return Err(UriError::ConflictingTlsModes);
            }
            tls_mode = parse_tls(&value);
            first_sslmode = Some(value);
            if tls_mode.is_some() {
                continue;
            }
        }
        if !ignored_params.iter().any(|previous| previous == &name) {
            ignored_params.push(name.into_owned());
        }
    }
    Ok(ParsedPostgresUri {
        host: host.into(),
        port,
        user,
        database,
        password,
        tls_mode,
        ignored_params,
    })
}

/// No password parameter exists. TLS files, alternate server name and driver
/// settings are disclosed as omissions, never silently encoded or hydrated.
pub fn build_postgres_uri(
    input: &DevelopmentPostgresConnection,
) -> Result<ExportedPostgresUri, UriError> {
    check_field(&input.user)?;
    check_field(&input.database)?;
    let raw_host = if input.host.is_empty() {
        "localhost"
    } else {
        &input.host
    };
    let host = if raw_host.starts_with('[') && raw_host.ends_with(']') {
        let host = &raw_host[1..raw_host.len() - 1];
        host.parse::<Ipv6Addr>()
            .map_err(|_| UriError::UnsupportedHost)?;
        host
    } else {
        raw_host
    };
    validate_host(host)?;
    let ipv6 = host.contains(':');
    let port = if input.port == 0 { 5432 } else { input.port };
    let port = port.to_string();
    let tls = tls_name(input.tls.mode);
    let query = input.tls.mode != DevelopmentTlsMode::Prefer;
    let size = "postgres://".len()
        + host.len()
        + usize::from(ipv6) * 2
        + 1
        + port.len()
        + if input.user.is_empty() {
            0
        } else {
            encoded_len(&input.user) + 1
        }
        + if input.database.is_empty() {
            0
        } else {
            encoded_len(&input.database) + 1
        }
        + if query {
            "?sslmode=".len() + tls.len()
        } else {
            0
        };
    if size > MAX_EXPORT_URI_BYTES {
        return Err(UriError::TooLarge);
    }
    let mut uri = String::with_capacity(size);
    uri.push_str("postgres://");
    if !input.user.is_empty() {
        encode_component(&input.user, &mut uri);
        uri.push('@');
    }
    if ipv6 {
        uri.push('[');
    }
    uri.push_str(host);
    if ipv6 {
        uri.push(']');
    }
    uri.push(':');
    uri.push_str(&port);
    if !input.database.is_empty() {
        uri.push('/');
        encode_component(&input.database, &mut uri);
    }
    if query {
        uri.push_str("?sslmode=");
        uri.push_str(tls);
    }
    let tls = &input.tls;
    let options = &input.driver_options;
    Ok(ExportedPostgresUri {
        uri,
        omissions: UriOmissions {
            tls_files: [
                &tls.root_cert_path,
                &tls.client_cert_path,
                &tls.client_key_path,
            ]
            .iter()
            .any(|path| path.as_ref().is_some_and(|path| !path.is_empty())),
            tls_server_name: tls
                .server_name
                .as_ref()
                .is_some_and(|name| !name.is_empty()),
            driver_options: options.statement_timeout_ms.is_some()
                || options.idle_in_transaction_timeout_ms.is_some()
                || options.connect_timeout_ms.is_some()
                || options.keepalive_seconds.is_some()
                || options
                    .default_role
                    .as_ref()
                    .is_some_and(|role| !role.is_empty())
                || options
                    .default_search_path
                    .as_ref()
                    .is_some_and(|path| !path.is_empty()),
        },
    })
}

fn parse_authority(value: &str) -> Result<(&str, u16), UriError> {
    if value.is_empty() {
        return Err(UriError::MissingHost);
    }
    let (host, port) = if let Some(bracketed) = value.strip_prefix('[') {
        let (host, suffix) = bracketed.split_once(']').ok_or(UriError::UnsupportedHost)?;
        host.parse::<Ipv6Addr>()
            .map_err(|_| UriError::UnsupportedHost)?;
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':').ok_or(UriError::UnsupportedHost)?)
        };
        (host, port)
    } else {
        match value.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (value, None),
        }
    };
    validate_host(host)?;
    let port = match port {
        Some(port) if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(UriError::InvalidPort)?,
        Some(_) => return Err(UriError::InvalidPort),
        None => 5432,
    };
    Ok((host, port))
}

fn validate_host(host: &str) -> Result<(), UriError> {
    if host.is_empty() {
        return Err(UriError::MissingHost);
    }
    check_field(host)?;
    if host.contains(':') {
        host.parse::<Ipv6Addr>()
            .map_err(|_| UriError::UnsupportedHost)?;
    } else if host
        .chars()
        .any(|ch| ch.is_whitespace() || "%/\\@?#[],<>^|".contains(ch))
    {
        return Err(UriError::UnsupportedHost);
    }
    Ok(())
}

fn check_field(value: &str) -> Result<(), UriError> {
    if value.len() > MAX_FIELD_BYTES {
        return Err(UriError::FieldTooLong);
    }
    reject_controls(value)
}

fn reject_controls(value: &str) -> Result<(), UriError> {
    if value.chars().any(char::is_control) {
        Err(UriError::InvalidCharacter)
    } else {
        Ok(())
    }
}

fn decode_component(input: &str) -> Result<String, UriError> {
    let mut decoded = Vec::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(hex)
                .ok_or(UriError::InvalidEncoding)?;
            let low = bytes
                .next()
                .and_then(hex)
                .ok_or(UriError::InvalidEncoding)?;
            decoded.push(high * 16 + low);
        } else {
            decoded.push(byte);
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| UriError::InvalidEncoding)?;
    reject_controls(&decoded)?;
    Ok(decoded)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn unescaped(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte)
}

fn encoded_len(value: &str) -> usize {
    value
        .bytes()
        .map(|byte| if unescaped(byte) { 1 } else { 3 })
        .sum()
}

fn encode_component(value: &str, output: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        if unescaped(byte) {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[usize::from(byte / 16)]));
            output.push(char::from(HEX[usize::from(byte % 16)]));
        }
    }
}

fn parse_tls(value: &str) -> Option<DevelopmentTlsMode> {
    Some(match value {
        "disable" => DevelopmentTlsMode::Disable,
        "prefer" => DevelopmentTlsMode::Prefer,
        "require" => DevelopmentTlsMode::Require,
        "verify-ca" => DevelopmentTlsMode::VerifyCa,
        "verify-full" => DevelopmentTlsMode::VerifyFull,
        _ => return None,
    })
}

fn tls_name(mode: DevelopmentTlsMode) -> &'static str {
    match mode {
        DevelopmentTlsMode::Disable => "disable",
        DevelopmentTlsMode::Prefer => "prefer",
        DevelopmentTlsMode::Require => "require",
        DevelopmentTlsMode::VerifyCa => "verify-ca",
        DevelopmentTlsMode::VerifyFull => "verify-full",
    }
}

#[cfg(test)]
mod tests;
