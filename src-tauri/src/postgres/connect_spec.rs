use crate::{PgDriverOptions, PgStoredConnection, StoredConnection};
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct ResolvedPostgresConnectSpec {
    pub connection_id: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub tls_prefer: bool,
    pub connect_timeout: Option<Duration>,
    pub driver_options: PgDriverOptions,
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
            port: if pg.port == 0 { 5432 } else { pg.port },
            database: pg.database.clone(),
            user: pg.user.clone(),
            password: pg.password.clone(),
            tls_prefer: pg.ssl,
            connect_timeout: driver_options
                .connect_timeout_ms
                .map(|ms| Duration::from_millis(ms.into())),
            driver_options,
        }
    }
    pub(crate) fn tokio_config(&self) -> tokio_postgres::Config {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .dbname(&self.database)
            .user(&self.user);
        if !self.password.is_empty() {
            config.password(&self.password);
        }
        config
    }
}

impl std::fmt::Debug for ResolvedPostgresConnectSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedPostgresConnectSpec")
            .field("connection_id", &self.connection_id)
            .field("port", &self.port)
            .field("tls_prefer", &self.tls_prefer)
            .field("connect_timeout", &self.connect_timeout)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SshTunnelConfig;

    #[test]
    fn query_sessions_leave_keepalive_at_the_driver_default() {
        let connection = PgStoredConnection {
            id: "connection".into(),
            name: "Connection".into(),
            database: "postgres".into(),
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            password: String::new(),
            role: String::new(),
            last_activity_at: None,
            ssl: false,
            driver_options: Some(PgDriverOptions {
                keepalive_seconds: Some(15),
                ..PgDriverOptions::default()
            }),
            ssh_tunnel: SshTunnelConfig::default(),
        };

        let config = ResolvedPostgresConnectSpec::from_postgres(&connection).tokio_config();

        assert_eq!(
            config.get_keepalives_idle(),
            Duration::from_secs(2 * 60 * 60)
        );
    }
}
