use crate::postgres::tls::ResolvedTls;
use crate::{PgDriverOptions, PgStoredConnection, StoredConnection};
use std::time::Duration;

/// Deadline applied when a connection sets no `connect_timeout_ms`. Every
/// one-shot connect (diagnosis, Test Connection) is bounded by this so an
/// unreachable host cannot hold the caller until the OS TCP timeout.
pub(crate) const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct ResolvedPostgresConnectSpec {
    pub connection_id: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    /// One TLS decision shared by every driver (ADR-0025).
    pub tls: ResolvedTls,
    pub connect_timeout: Option<Duration>,
    /// TCP keepalive idle time (ADR-0013 `keepalive_seconds`). Applied on
    /// the dedicated driver; the SQLx pool cannot set it.
    pub keepalive: Option<Duration>,
    pub driver_options: PgDriverOptions,
    pub safety_policy: crate::safety::policy::ResolvedSafetyPolicy,
}

impl ResolvedPostgresConnectSpec {
    pub(crate) fn from_connection(connection: &StoredConnection) -> Result<Self, ()> {
        let StoredConnection::PostgreSQL(pg) = connection else {
            return Err(());
        };
        Ok(Self::from_postgres(pg))
    }
    pub(crate) fn from_postgres(pg: &PgStoredConnection) -> Self {
        let driver_options = pg.driver_options.clone().unwrap_or_default();
        Self {
            connection_id: pg.id.clone(),
            host: pg.host.clone(),
            port: pg.effective_port(),
            database: pg.database.clone(),
            user: pg.user.clone(),
            password: pg.password.clone(),
            tls: ResolvedTls::from_postgres(pg),
            connect_timeout: driver_options
                .connect_timeout_ms
                .map(|ms| Duration::from_millis(ms.into())),
            keepalive: driver_options
                .keepalive_seconds
                .filter(|seconds| *seconds > 0)
                .map(|seconds| Duration::from_secs(seconds.into())),
            driver_options,
            safety_policy: crate::safety::policy::resolve_policy(crate::ConnectionPolicy {
                environment: pg.environment,
                safe_mode: pg.safe_mode,
                read_only: pg.read_only,
            }),
        }
    }
    /// tokio-postgres config. `host` is the TLS server name; when it
    /// differs from the socket host (SSH tunnel, IP literal override)
    /// `dedicated::connect` adds a `hostaddr` so the socket goes to the
    /// real endpoint while the certificate is matched against the name.
    pub(crate) fn tokio_config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&self.tls.server_name)
            .port(self.port)
            .dbname(&self.database)
            .user(&self.user);
        if !self.password.is_empty() {
            config.password(&self.password);
        }
        if let Some(idle) = self.keepalive {
            config.keepalives(true).keepalives_idle(idle);
        }
        config
    }
}

impl std::fmt::Debug for ResolvedPostgresConnectSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedPostgresConnectSpec")
            .field("connection_id", &self.connection_id)
            .field("port", &self.port)
            .field("tls_mode", &self.tls.mode)
            .field("tls_server_name", &self.tls.server_name)
            .field("connect_timeout", &self.connect_timeout)
            .field("keepalive", &self.keepalive)
            .field("read_only", &self.safety_policy.read_only)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PgTlsMode, PgTlsOptions, SshTunnelConfig};

    fn connection(driver_options: Option<PgDriverOptions>) -> PgStoredConnection {
        PgStoredConnection {
            organization: Default::default(),
            id: "connection".into(),
            name: "Connection".into(),
            database: "postgres".into(),
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            password: String::new(),
            role: String::new(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            ssl: false,
            tls_options: None,
            driver_options,
            ssh_tunnel: SshTunnelConfig::default(),
        }
    }

    #[test]
    fn keepalive_seconds_is_applied_to_the_dedicated_driver() {
        let spec = ResolvedPostgresConnectSpec::from_postgres(&connection(Some(PgDriverOptions {
            keepalive_seconds: Some(15),
            ..PgDriverOptions::default()
        })));
        assert_eq!(spec.keepalive, Some(Duration::from_secs(15)));
        let config = spec.tokio_config();
        assert!(config.get_keepalives());
        assert_eq!(config.get_keepalives_idle(), Duration::from_secs(15));
    }

    #[test]
    fn absent_or_zero_keepalive_leaves_the_driver_default() {
        for options in [
            None,
            Some(PgDriverOptions {
                keepalive_seconds: Some(0),
                ..PgDriverOptions::default()
            }),
        ] {
            let spec = ResolvedPostgresConnectSpec::from_postgres(&connection(options));
            assert_eq!(spec.keepalive, None);
            assert_eq!(
                spec.tokio_config().get_keepalives_idle(),
                Duration::from_secs(2 * 60 * 60)
            );
        }
    }

    #[test]
    fn tokio_config_uses_the_tls_server_name_as_host() {
        let mut pg = connection(None);
        pg.tls_options = Some(PgTlsOptions {
            mode: PgTlsMode::VerifyFull,
            server_name: Some("db.example".into()),
            ..Default::default()
        });
        let spec = ResolvedPostgresConnectSpec::from_postgres(&pg);
        assert_eq!(spec.tls.mode, PgTlsMode::VerifyFull);
        assert!(spec.tls.server_name_differs_from(&spec.host));
        let hosts = spec.tokio_config().get_hosts().to_vec();
        assert!(matches!(&hosts[..], [tokio_postgres::config::Host::Tcp(h)] if h == "db.example"));
    }

    #[test]
    fn zero_port_defaults_to_5432() {
        let mut pg = connection(None);
        pg.port = 0;
        assert_eq!(ResolvedPostgresConnectSpec::from_postgres(&pg).port, 5432);
    }
}
