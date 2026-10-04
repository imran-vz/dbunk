//! Plan 031 step 4: redacted native connection records for every engine.
//! PostgreSQL keeps its dedicated form; MySQL, SQLite, ClickHouse and Redis
//! map onto the existing `StoredConnection` variants, storage rows and
//! credential journal. Secrets are separate inputs and never appear here.
use super::{
    environment_of, safe_mode_of, stored_environment, stored_safe_mode, tunnel_config,
    validate_text, validate_tunnel_fields, DevelopmentEnvironment, DevelopmentPostgresConnection,
    DevelopmentSafeMode, DevelopmentSshTunnel,
};
use crate::StoredConnection;
use serde::{Deserialize, Serialize};

const NAME_LIMIT: usize = 256;
const PATH_LIMIT: usize = 4096;
const URL_PATH_LIMIT: usize = 1024;

/// One editable connection record per engine. Internally tagged on `engine`
/// with the `DatabaseEngine` spellings; each variant rejects unknown fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "engine")]
pub enum DevelopmentEngineConnection {
    PostgreSQL(DevelopmentPostgresConnection),
    MySQL(DevelopmentMySqlConnection),
    SQLite(DevelopmentSqliteConnection),
    ClickHouse(DevelopmentClickHouseConnection),
    Redis(DevelopmentRedisConnection),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentMySqlConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    /// Optional default schema; empty connects without one.
    pub database: String,
    pub user: String,
    pub environment: DevelopmentEnvironment,
    pub safe_mode: DevelopmentSafeMode,
    pub read_only: bool,
    /// Negotiate TLS when the server offers it (`ssl-mode=preferred`).
    pub ssl: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_tunnel: Option<DevelopmentSshTunnel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentSqliteConnection {
    pub name: String,
    /// Absolute path of an existing database file. Never created by dbunk.
    pub path: String,
    pub environment: DevelopmentEnvironment,
    pub safe_mode: DevelopmentSafeMode,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentClickHouseConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub environment: DevelopmentEnvironment,
    pub safe_mode: DevelopmentSafeMode,
    pub read_only: bool,
    /// HTTPS transport; the default port becomes 8443 instead of 8123.
    pub use_https: bool,
    /// Optional proxy path prefix such as `/clickhouse`. Empty = root.
    pub url_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_tunnel: Option<DevelopmentSshTunnel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentRedisConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    /// Logical database selected after connect.
    pub db_number: u8,
    /// ACL user; empty uses the default user.
    pub user: String,
    pub environment: DevelopmentEnvironment,
    pub safe_mode: DevelopmentSafeMode,
    pub read_only: bool,
    pub use_tls: bool,
    /// Only meaningful with `use_tls`.
    pub verify_tls_cert: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_tunnel: Option<DevelopmentSshTunnel>,
}

/// Display summary shared by every engine. No secret or TLS material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentEndpoint {
    /// Network host; empty for SQLite.
    pub host: String,
    /// Effective port (engine default applied); None for SQLite.
    pub port: Option<u16>,
    /// Database name, SQLite file path, or Redis logical database number.
    pub database: String,
    pub user: String,
}

impl DevelopmentEndpoint {
    /// Compact single-line label for lists: `host:port/database` or the path.
    pub fn label(&self) -> String {
        match self.port {
            None => self.database.clone(),
            Some(port) if self.database.is_empty() => format!("{}:{port}", self.host),
            Some(port) => format!("{}:{port}/{}", self.host, self.database),
        }
    }
}

impl DevelopmentEngineConnection {
    /// The `DatabaseEngine` spelling, matching `DevelopmentConnection::engine`.
    pub fn engine(&self) -> &'static str {
        match self {
            Self::PostgreSQL(_) => "PostgreSQL",
            Self::MySQL(_) => "MySQL",
            Self::SQLite(_) => "SQLite",
            Self::ClickHouse(_) => "ClickHouse",
            Self::Redis(_) => "Redis",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.name,
            Self::MySQL(c) => &c.name,
            Self::SQLite(c) => &c.name,
            Self::ClickHouse(c) => &c.name,
            Self::Redis(c) => &c.name,
        }
    }

    pub fn environment(&self) -> DevelopmentEnvironment {
        match self {
            Self::PostgreSQL(c) => c.environment,
            Self::MySQL(c) => c.environment,
            Self::SQLite(c) => c.environment,
            Self::ClickHouse(c) => c.environment,
            Self::Redis(c) => c.environment,
        }
    }

    pub fn endpoint(&self) -> DevelopmentEndpoint {
        let network = |host: &str, port: u16, database: String, user: &str| DevelopmentEndpoint {
            host: host.into(),
            port: Some(port),
            database,
            user: user.into(),
        };
        match self {
            Self::PostgreSQL(c) => network(&c.host, c.port, c.database.clone(), &c.user),
            Self::MySQL(c) => network(&c.host, c.port, c.database.clone(), &c.user),
            Self::ClickHouse(c) => network(&c.host, c.port, c.database.clone(), &c.user),
            Self::Redis(c) => network(&c.host, c.port, c.db_number.to_string(), &c.user),
            Self::SQLite(c) => DevelopmentEndpoint {
                host: String::new(),
                port: None,
                database: c.path.clone(),
                user: String::new(),
            },
        }
    }

    pub(super) fn from_stored(connection: &StoredConnection) -> Self {
        let port = connection.effective_port();
        match connection {
            StoredConnection::PostgreSQL(pg) => {
                Self::PostgreSQL(DevelopmentPostgresConnection::from_stored(pg))
            }
            StoredConnection::MySQL(c) => Self::MySQL(DevelopmentMySqlConnection {
                name: c.name.clone(),
                host: c.host.clone(),
                port,
                database: c.database.clone(),
                user: c.user.clone(),
                environment: environment_of(c.environment),
                safe_mode: safe_mode_of(c.safe_mode),
                read_only: c.read_only,
                ssl: c.ssl,
                ssh_tunnel: DevelopmentSshTunnel::from_config(&c.ssh_tunnel),
            }),
            StoredConnection::SQLite(c) => Self::SQLite(DevelopmentSqliteConnection {
                name: c.name.clone(),
                path: c.database.clone(),
                environment: environment_of(c.environment),
                safe_mode: safe_mode_of(c.safe_mode),
                read_only: c.read_only,
            }),
            StoredConnection::ClickHouse(c) => Self::ClickHouse(DevelopmentClickHouseConnection {
                name: c.name.clone(),
                host: c.host.clone(),
                port,
                database: c.database.clone(),
                user: c.user.clone(),
                environment: environment_of(c.environment),
                safe_mode: safe_mode_of(c.safe_mode),
                read_only: c.read_only,
                use_https: c.use_https,
                url_path: c.url_path.clone(),
                ssh_tunnel: DevelopmentSshTunnel::from_config(&c.ssh_tunnel),
            }),
            StoredConnection::Redis(c) => Self::Redis(DevelopmentRedisConnection {
                name: c.name.clone(),
                host: c.host.clone(),
                port,
                db_number: c.db_number,
                user: c.user.clone(),
                environment: environment_of(c.environment),
                safe_mode: safe_mode_of(c.safe_mode),
                read_only: c.read_only,
                use_tls: c.use_tls,
                verify_tls_cert: c.verify_tls_cert,
                ssh_tunnel: DevelopmentSshTunnel::from_config(&c.ssh_tunnel),
            }),
        }
    }

    /// Validates the form and builds the stored record. `previous` supplies
    /// columns the form does not edit (role, activity, organization, disabled
    /// route options and SQLite sentinel columns). A saved record keeps its
    /// engine; switching engines means creating a new connection.
    pub(super) fn into_stored(
        self,
        id: String,
        previous: Option<&StoredConnection>,
    ) -> Result<StoredConnection, String> {
        if let Some(previous) = previous {
            if previous.engine().as_str() != self.engine() {
                return Err(
                    "A saved connection cannot change engine; create a new connection instead"
                        .into(),
                );
            }
        }
        let role = previous
            .map_or("read/write", StoredConnection::role)
            .to_owned();
        let last_activity_at = previous
            .and_then(StoredConnection::last_activity_at)
            .map(str::to_owned);
        let organization = previous
            .map(|value| value.organization().clone())
            .unwrap_or_default();
        Ok(match self {
            Self::PostgreSQL(form) => form.into_stored(id, previous)?,
            Self::MySQL(form) => {
                validate_network(&form.name, &form.host, form.port)?;
                validate_required(&form.user, "MySQL user is required")?;
                validate_text(&form.database, NAME_LIMIT)?;
                let ssh_tunnel = tunnel_config(form.ssh_tunnel, previous)?;
                StoredConnection::MySQL(crate::MySqlStoredConnection {
                    id,
                    name: form.name.trim().into(),
                    database: form.database.trim().into(),
                    host: form.host.trim().into(),
                    port: form.port,
                    user: form.user,
                    password: String::new(),
                    role,
                    environment: stored_environment(form.environment),
                    safe_mode: stored_safe_mode(form.safe_mode),
                    read_only: form.read_only,
                    last_activity_at,
                    organization,
                    ssl: form.ssl,
                    ssh_tunnel,
                })
            }
            Self::SQLite(form) => {
                validate_required(&form.name, "Connection name is required")?;
                validate_text(&form.name, NAME_LIMIT)?;
                // A legacy location (relative path or `sqlite:` URI) may be
                // kept as stored; any new location must be a plain file path.
                let unchanged = matches!(
                    previous,
                    Some(StoredConnection::SQLite(stored)) if stored.database == form.path
                );
                if unchanged {
                    validate_text(&form.path, PATH_LIMIT)?;
                } else {
                    validate_sqlite_path(&form.path)?;
                }
                // SQLite keeps the flat row's unused network columns as stored.
                let (host, port, user) = match previous {
                    Some(StoredConnection::SQLite(stored)) => {
                        (stored.host.clone(), stored.port, stored.user.clone())
                    }
                    _ => (String::new(), 0, String::new()),
                };
                StoredConnection::SQLite(crate::SqliteStoredConnection {
                    id,
                    name: form.name.trim().into(),
                    database: form.path,
                    host,
                    port,
                    user,
                    password: String::new(),
                    role,
                    environment: stored_environment(form.environment),
                    safe_mode: stored_safe_mode(form.safe_mode),
                    read_only: form.read_only,
                    last_activity_at,
                    organization,
                })
            }
            Self::ClickHouse(form) => {
                validate_network(&form.name, &form.host, form.port)?;
                validate_text(&form.user, NAME_LIMIT)?;
                validate_text(&form.database, NAME_LIMIT)?;
                validate_url_path(&form.url_path)?;
                let ssh_tunnel = tunnel_config(form.ssh_tunnel, previous)?;
                StoredConnection::ClickHouse(crate::ClickHouseStoredConnection {
                    id,
                    name: form.name.trim().into(),
                    database: form.database.trim().into(),
                    host: form.host.trim().into(),
                    port: form.port,
                    user: form.user,
                    password: String::new(),
                    role,
                    environment: stored_environment(form.environment),
                    safe_mode: stored_safe_mode(form.safe_mode),
                    read_only: form.read_only,
                    last_activity_at,
                    organization,
                    use_https: form.use_https,
                    url_path: form.url_path.trim().into(),
                    ssh_tunnel,
                })
            }
            Self::Redis(form) => {
                validate_network(&form.name, &form.host, form.port)?;
                validate_text(&form.user, NAME_LIMIT)?;
                let ssh_tunnel = tunnel_config(form.ssh_tunnel, previous)?;
                // The flat row's unused `database` column is kept as stored.
                let database = match previous {
                    Some(StoredConnection::Redis(stored)) => stored.database.clone(),
                    _ => String::new(),
                };
                StoredConnection::Redis(crate::RedisStoredConnection {
                    id,
                    name: form.name.trim().into(),
                    database,
                    host: form.host.trim().into(),
                    port: form.port,
                    user: form.user,
                    password: String::new(),
                    role,
                    environment: stored_environment(form.environment),
                    safe_mode: stored_safe_mode(form.safe_mode),
                    last_activity_at,
                    organization,
                    db_number: form.db_number,
                    use_tls: form.use_tls,
                    verify_tls_cert: form.verify_tls_cert,
                    read_only: form.read_only,
                    ssh_tunnel,
                })
            }
        })
    }
}

fn validate_required(value: &str, message: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(message.into());
    }
    validate_text(value, NAME_LIMIT)
}

fn validate_network(name: &str, host: &str, port: u16) -> Result<(), String> {
    validate_required(name, "Connection name is required")?;
    validate_required(host, "Host is required")?;
    if port == 0 {
        return Err("Port must be between 1 and 65535".into());
    }
    Ok(())
}

fn validate_url_path(path: &str) -> Result<(), String> {
    validate_text(path, URL_PATH_LIMIT)?;
    if path
        .chars()
        .any(|c| c.is_whitespace() || c == '?' || c == '#')
    {
        return Err("ClickHouse URL path must not contain spaces, ? or #".into());
    }
    Ok(())
}

/// The SQLite driver parses its location as a URL: `%` is decoded and `?`
/// starts options (including `mode=rwc`, which creates files). Refuse those
/// characters so the probed file is exactly the stored path.
pub(super) fn validate_sqlite_path(path: &str) -> Result<(), String> {
    validate_text(path, PATH_LIMIT)?;
    if path.trim().is_empty() {
        return Err("SQLite database file path is required".into());
    }
    if !std::path::Path::new(path).is_absolute() {
        return Err("SQLite database file path must be absolute".into());
    }
    if path.contains(['?', '#', '%']) {
        return Err("SQLite database file paths containing ?, # or % are not supported".into());
    }
    Ok(())
}

/// Requires an existing, readable regular file. Never creates or writes it.
pub(in crate::backend) async fn check_sqlite_file(path: &str) -> Result<(), String> {
    validate_sqlite_path(path)?;
    const UNREADABLE: &str = "SQLite database file does not exist or is not readable";
    let metadata = tokio::fs::metadata(path).await.map_err(|_| UNREADABLE)?;
    if !metadata.is_file() {
        return Err("SQLite database path is not a regular file".into());
    }
    tokio::fs::File::open(path).await.map_err(|_| UNREADABLE)?;
    Ok(())
}

/// Loaded-record bounds for non-PostgreSQL engines. Loose enough for legacy
/// rows (port 0 = engine default, legacy SQLite URIs) yet bounded. No file,
/// DNS or credential access.
pub(super) fn supported_engine_fields(connection: &StoredConnection) -> bool {
    let bounded = |value: &str| validate_text(value, NAME_LIMIT).is_ok();
    let present = |value: &str| !value.trim().is_empty() && bounded(value);
    let tunnel = |config| validate_tunnel_fields(config).is_ok();
    match connection {
        StoredConnection::PostgreSQL(_) => false,
        StoredConnection::MySQL(c) => {
            present(&c.name)
                && present(&c.host)
                && present(&c.user)
                && bounded(&c.database)
                && tunnel(&c.ssh_tunnel)
        }
        StoredConnection::SQLite(c) => {
            present(&c.name)
                && !c.database.trim().is_empty()
                && validate_text(&c.database, PATH_LIMIT).is_ok()
                && bounded(&c.host)
                && bounded(&c.user)
        }
        StoredConnection::ClickHouse(c) => {
            present(&c.name)
                && present(&c.host)
                && bounded(&c.user)
                && bounded(&c.database)
                && validate_text(&c.url_path, URL_PATH_LIMIT).is_ok()
                && tunnel(&c.ssh_tunnel)
        }
        StoredConnection::Redis(c) => {
            present(&c.name)
                && present(&c.host)
                && bounded(&c.user)
                && bounded(&c.database)
                && tunnel(&c.ssh_tunnel)
        }
    }
}

/// Maps a dispatch ping error to a redacted failure class. The server text
/// itself is never returned to the caller.
pub(super) fn classify_engine_error(error: &str) -> super::DevelopmentConnectionFailure {
    use super::DevelopmentConnectionFailure as Failure;
    let lower = error.to_ascii_lowercase();
    let any = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if any(&["timed out", "timeout", "deadline"]) {
        Failure::Timeout
    } else if any(&[
        "authentication",
        "access denied",
        "noauth",
        "wrongpass",
        "wrong password",
        "password",
        "unauthorized",
        "401 ",
    ]) {
        Failure::Authentication
    } else if any(&["tls", "ssl", "certificate", "handshake"]) {
        Failure::Tls(crate::types::TlsFailureKind::HandshakeFailed)
    } else if any(&[
        "refused",
        "unreachable",
        "reset",
        "aborted",
        "resolve",
        "lookup",
        "network",
        "could not connect",
        "error trying to connect",
        "error sending request",
        "broken pipe",
        "connection closed",
    ]) {
        Failure::ConnectionLost
    } else {
        Failure::Database
    }
}

#[cfg(test)]
#[path = "engine_connection_tests.rs"]
mod tests;
