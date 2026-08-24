use crate::StoredConnection;

use super::LocalEndpoint;

pub(super) fn remote_endpoint(connection: &StoredConnection) -> Result<(String, u16), String> {
    let port = match connection {
        StoredConnection::PostgreSQL(c) => defaulted_port(c.port, 5432),
        StoredConnection::MySQL(c) => defaulted_port(c.port, 3306),
        StoredConnection::ClickHouse(c) => {
            if c.host.starts_with("http://") || c.host.starts_with("https://") {
                let url = reqwest::Url::parse(&c.host).map_err(|error| error.to_string())?;
                let host = url
                    .host_str()
                    .ok_or_else(|| "ClickHouse URL host is required".to_string())?
                    .to_string();
                let port = url.port_or_known_default().ok_or_else(|| {
                    "ClickHouse URL must include a port or use http/https".to_string()
                })?;
                return Ok((host, port));
            }
            if c.use_https {
                defaulted_port(c.port, 8443)
            } else {
                defaulted_port(c.port, 8123)
            }
        }
        StoredConnection::Redis(c) => defaulted_port(c.port, 6379),
        StoredConnection::SQLite(_) => {
            return Err("SQLite connections do not support SSH tunnels".to_string());
        }
    };
    if connection.host().trim().is_empty() {
        return Err(format!(
            "{} host is required before an SSH tunnel can be established",
            crate::dispatch::relational::engine_name(&connection.engine())
        ));
    }
    Ok((connection.host().to_string(), port))
}

pub(super) fn rewrite_connection_endpoint(
    connection: &StoredConnection,
    endpoint: &LocalEndpoint,
) -> Result<StoredConnection, String> {
    let mut routed = connection.clone();
    if let StoredConnection::ClickHouse(ch) = &mut routed {
        if ch.host.starts_with("http://") || ch.host.starts_with("https://") {
            let scheme = if ch.host.starts_with("https://") {
                "https"
            } else {
                "http"
            };
            ch.host = format!("{scheme}://{}:{}", endpoint.host, endpoint.port);
            ch.port = endpoint.port;
            return Ok(routed);
        }
    }
    // ADR-0025: the tunnel replaces `host` with the loopback endpoint, so
    // carry the real host name forward as the TLS server name on this
    // resolved copy (never persisted). A user-supplied name wins, and a
    // legacy row keeps its `ssl`-derived mode.
    if let StoredConnection::PostgreSQL(pg) = &mut routed {
        let original = pg.host.trim().to_string();
        let mode = pg.resolved_tls_mode();
        let options = pg.tls_options.get_or_insert_with(|| crate::PgTlsOptions {
            mode,
            ..Default::default()
        });
        let user_supplied = options
            .server_name
            .as_deref()
            .map(str::trim)
            .is_some_and(|name| !name.is_empty());
        if !user_supplied && !original.is_empty() {
            options.server_name = Some(original);
        }
    }
    routed.set_network_endpoint(endpoint.host.clone(), endpoint.port)?;
    Ok(routed)
}

fn defaulted_port(port: u16, default_port: u16) -> u16 {
    if port == 0 {
        default_port
    } else {
        port
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgStoredConnection, RedisStoredConnection, SshTunnelConfig};

    fn tunnel() -> SshTunnelConfig {
        SshTunnelConfig {
            enabled: true,
            bastion_server_id: Some("bastion-1".to_string()),
            local_bind_host: None,
            local_port: None,
            ..SshTunnelConfig::default()
        }
    }

    #[test]
    fn remote_endpoint_uses_database_endpoint_seen_from_bastion() {
        let connection = StoredConnection::PostgreSQL(PgStoredConnection {
            organization: Default::default(),
            id: "conn-1".into(),
            name: "pg".into(),
            database: "postgres".into(),
            host: "db.internal".into(),
            port: 0,
            user: "postgres".into(),
            password: String::new(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            ssl: true,
            tls_options: None,
            driver_options: None,
            ssh_tunnel: tunnel(),
        });

        assert_eq!(
            remote_endpoint(&connection).expect("endpoint"),
            ("db.internal".to_string(), 5432)
        );
    }

    fn pg_with_tls(tls_options: Option<crate::PgTlsOptions>) -> StoredConnection {
        StoredConnection::PostgreSQL(PgStoredConnection {
            organization: Default::default(),
            id: "conn-1".into(),
            name: "pg".into(),
            database: "postgres".into(),
            host: " db.internal ".into(),
            port: 5432,
            user: "postgres".into(),
            password: String::new(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            ssl: false,
            tls_options,
            driver_options: None,
            ssh_tunnel: tunnel(),
        })
    }

    fn loopback() -> LocalEndpoint {
        LocalEndpoint {
            host: "127.0.0.1".into(),
            port: 49152,
        }
    }

    #[test]
    fn rewrite_carries_the_original_host_as_tls_server_name() {
        let StoredConnection::PostgreSQL(routed) =
            rewrite_connection_endpoint(&pg_with_tls(None), &loopback()).expect("rewrite")
        else {
            panic!("expected postgres");
        };
        assert_eq!(routed.host, "127.0.0.1");
        assert_eq!(routed.port, 49152);
        // A legacy `ssl: false` row keeps its disabled mode.
        assert_eq!(routed.resolved_tls_mode(), crate::PgTlsMode::Disable);
        let options = routed.tls_options.expect("server name recorded");
        assert_eq!(options.server_name.as_deref(), Some("db.internal"));
        assert_eq!(options.mode, crate::PgTlsMode::Disable);
    }

    #[test]
    fn rewrite_preserves_a_user_supplied_server_name() {
        let StoredConnection::PostgreSQL(routed) = rewrite_connection_endpoint(
            &pg_with_tls(Some(crate::PgTlsOptions {
                mode: crate::PgTlsMode::VerifyFull,
                server_name: Some("cert.example".into()),
                ..Default::default()
            })),
            &loopback(),
        )
        .expect("rewrite") else {
            panic!("expected postgres");
        };
        let options = routed.tls_options.expect("options kept");
        assert_eq!(options.server_name.as_deref(), Some("cert.example"));
        assert_eq!(options.mode, crate::PgTlsMode::VerifyFull);
    }

    #[test]
    fn rewrite_connection_endpoint_keeps_redis_tunnel_metadata() {
        let connection = StoredConnection::Redis(RedisStoredConnection {
            organization: Default::default(),
            id: "redis-1".into(),
            name: "redis".into(),
            database: String::new(),
            host: "redis.internal".into(),
            port: 0,
            user: "default".into(),
            password: String::new(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            last_activity_at: None,
            db_number: 2,
            use_tls: false,
            verify_tls_cert: true,
            read_only: false,
            ssh_tunnel: tunnel(),
        });

        let routed = rewrite_connection_endpoint(
            &connection,
            &LocalEndpoint {
                host: "127.0.0.1".into(),
                port: 49152,
            },
        )
        .expect("rewrite");

        let StoredConnection::Redis(redis) = routed else {
            panic!("expected redis");
        };
        assert_eq!(redis.host, "127.0.0.1");
        assert_eq!(redis.port, 49152);
        assert_eq!(redis.db_number, 2);
        assert!(redis.ssh_tunnel.enabled);
    }

    #[test]
    fn sqlite_endpoint_rewrite_is_rejected() {
        let connection = StoredConnection::SQLite(crate::SqliteStoredConnection {
            organization: Default::default(),
            id: "sqlite".into(),
            name: "sqlite".into(),
            database: "/tmp/db.sqlite".into(),
            host: String::new(),
            port: 0,
            user: String::new(),
            password: String::new(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
        });

        let error = rewrite_connection_endpoint(
            &connection,
            &LocalEndpoint {
                host: "127.0.0.1".into(),
                port: 49152,
            },
        )
        .expect_err("sqlite should reject tunnel endpoint");
        assert!(error.contains("SQLite"));
    }
}
