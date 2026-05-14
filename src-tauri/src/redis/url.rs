//! Redis connection-URL builder.
//!
//! Translates a `StoredConnection` (host / port / user / password /
//! `db_number` / `use_tls` / `verify_tls_cert`) into the `redis://` or
//! `rediss://` URL `redis-rs` expects. Auth shape:
//! - empty user + empty password → no auth
//! - empty user + non-empty password → `:password@` (Redis ≤5 compat,
//!   ACL `default` user with password)
//! - non-empty user + password → `user:password@` (Redis 6+ ACL)
//!
//! Verify-cert behaviour: `redis-rs`'s URL parser doesn't natively
//! recognise a `verify=false` query parameter — when `use_tls` is on
//! and `verify_tls_cert` is off, the caller is responsible for
//! constructing a [`redis::TlsCertificates`] / explicit
//! `ConnectionInfo` (Phase 1.1 trusts certs by default; the explicit
//! cert-skip path is wired alongside the connection-test command).

use crate::RedisStoredConnection;

const DEFAULT_PORT: u16 = 6379;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedisUrl {
    pub url: String,
    pub use_tls: bool,
    pub verify_tls_cert: bool,
    pub db_number: u8,
}

pub fn build(connection: &RedisStoredConnection) -> Result<RedisUrl, String> {
    if connection.host.trim().is_empty() {
        return Err("Redis host is required".to_string());
    }
    if connection.db_number > 15 {
        return Err(format!(
            "Redis DB number must be 0–15 (got {})",
            connection.db_number
        ));
    }

    let scheme = if connection.use_tls {
        "rediss"
    } else {
        "redis"
    };
    let port = if connection.port == 0 {
        DEFAULT_PORT
    } else {
        connection.port
    };
    let auth = match (connection.user.is_empty(), connection.password.is_empty()) {
        (true, true) => String::new(),
        (true, false) => format!(":{}@", encode(&connection.password)),
        (false, _) => format!(
            "{}:{}@",
            encode(&connection.user),
            encode(&connection.password)
        ),
    };

    let url = format!(
        "{scheme}://{auth}{host}:{port}/{db}",
        host = connection.host,
        db = connection.db_number,
    );

    // Redact credentials before logging the URL. Useful when a user
    // reports a connection error — we want to see what was actually
    // sent without dumping their password.
    let redacted_auth = match (connection.user.is_empty(), connection.password.is_empty()) {
        (true, true) => String::new(),
        (true, false) => ":***@".to_string(),
        (false, _) => format!("{}:***@", connection.user),
    };
    let redacted = format!(
        "{scheme}://{redacted_auth}{host}:{port}/{db}",
        host = connection.host,
        db = connection.db_number,
    );
    log::debug!("url::build: dsn={}", redacted);

    Ok(RedisUrl {
        url,
        use_tls: connection.use_tls,
        verify_tls_cert: connection.verify_tls_cert,
        db_number: connection.db_number,
    })
}

/// Percent-encode `@`, `:`, and `/` so passwords / usernames that
/// happen to contain URL-significant characters don't break parsing.
/// Intentionally narrow — `redis-rs`'s parser is more permissive than
/// full RFC 3986, so we only escape what it actually disambiguates.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'@' => out.push_str("%40"),
            b':' => out.push_str("%3A"),
            b'/' => out.push_str("%2F"),
            b'%' => out.push_str("%25"),
            _ => out.push(byte as char),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RedisStoredConnection {
        RedisStoredConnection {
            id: "id".into(),
            name: "n".into(),
            database: String::new(),
            host: "localhost".into(),
            port: 0,
            user: String::new(),
            password: String::new(),
            role: String::new(),
            last_activity_at: None,
            db_number: 0,
            use_tls: false,
            verify_tls_cert: true,
        }
    }

    #[test]
    fn defaults_to_port_6379_db_0_no_auth() {
        let url = build(&base()).unwrap();
        assert_eq!(url.url, "redis://localhost:6379/0");
        assert!(!url.use_tls);
    }

    #[test]
    fn password_only_skips_username() {
        let mut conn = base();
        conn.password = "secret".into();
        let url = build(&conn).unwrap();
        assert_eq!(url.url, "redis://:secret@localhost:6379/0");
    }

    #[test]
    fn user_and_password_renders_acl_form() {
        let mut conn = base();
        conn.user = "alice".into();
        conn.password = "secret".into();
        let url = build(&conn).unwrap();
        assert_eq!(url.url, "redis://alice:secret@localhost:6379/0");
    }

    #[test]
    fn tls_uses_rediss_scheme() {
        let mut conn = base();
        conn.use_tls = true;
        let url = build(&conn).unwrap();
        assert!(url.url.starts_with("rediss://"));
        assert!(url.use_tls);
    }

    #[test]
    fn db_number_is_path_component() {
        let mut conn = base();
        conn.db_number = 7;
        let url = build(&conn).unwrap();
        assert!(url.url.ends_with("/7"));
    }

    #[test]
    fn rejects_db_above_15() {
        let mut conn = base();
        conn.db_number = 16;
        assert!(build(&conn).is_err());
    }

    #[test]
    fn rejects_empty_host() {
        let mut conn = base();
        conn.host = String::new();
        assert!(build(&conn).is_err());
    }

    #[test]
    fn percent_encodes_url_significant_chars_in_password() {
        let mut conn = base();
        conn.password = "p@ss:w/rd%".into();
        let url = build(&conn).unwrap();
        assert_eq!(url.url, "redis://:p%40ss%3Aw%2Frd%25@localhost:6379/0");
    }

    #[test]
    fn custom_port_overrides_default() {
        let mut conn = base();
        conn.port = 6380;
        let url = build(&conn).unwrap();
        assert!(url.url.contains(":6380/"));
    }
}
