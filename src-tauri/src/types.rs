//! Pure data shapes used across the Tauri command surface.
//!
//! Everything in this module is a serde DTO — payloads the frontend sends,
//! result types it receives, and the persisted shapes for connections,
//! query history, and saved queries. There is no behaviour here; the
//! domain glossary in `CONTEXT.md` is the load-bearing reading.
//!
//! All items are `pub(crate)` so command dispatchers, the persistence layer
//! (`storage.rs`), the credential layer (`keychain.rs`), and the engine
//! implementations (`postgres.rs`) can use them via `crate::Foo` after the
//! re-export in `lib.rs`.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Engine + persisted entities
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DatabaseEngine {
    #[serde(rename = "PostgreSQL")]
    PostgreSQL,
    #[serde(rename = "MySQL")]
    MySQL,
    #[serde(rename = "ClickHouse")]
    ClickHouse,
    #[serde(rename = "SQLite")]
    SQLite,
    #[serde(rename = "Redis")]
    Redis,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Environment {
    #[default]
    Development,
    Test,
    Staging,
    Production,
}

impl Environment {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }
}

impl FromStr for Environment {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "development" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "staging" => Ok(Self::Staging),
            "production" => Ok(Self::Production),
            _ => Err(format!("Unsupported connection environment: {value}")),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SafeMode {
    #[default]
    Inherit,
    Disabled,
    Protected,
    Strict,
}

impl SafeMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Disabled => "disabled",
            Self::Protected => "protected",
            Self::Strict => "strict",
        }
    }
}

impl FromStr for SafeMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "inherit" => Ok(Self::Inherit),
            "disabled" => Ok(Self::Disabled),
            "protected" => Ok(Self::Protected),
            "strict" => Ok(Self::Strict),
            _ => Err(format!("Unsupported connection Safe Mode: {value}")),
        }
    }
}

/// The stored inputs that resolve into an enforced safety policy. Connection
/// variants remain wire-flat, while backend policy consumers pass one value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionPolicy {
    pub(crate) environment: Environment,
    pub(crate) safe_mode: SafeMode,
    pub(crate) read_only: bool,
}

/// Top-level engine class — `Relational` engines share schemas/tables/
/// rows/SQL; `KeyValue` engines share a keyspace of typed keys. The
/// class is derived from `DatabaseEngine::storage_class()`; never
/// stored. See ADR-0008.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StorageClass {
    Relational,
    KeyValue,
}

impl DatabaseEngine {
    /// Class lookup — the single source of truth for "is this engine a
    /// relational or keyvalue thing?" Exhaustive over variants so a new
    /// engine forces the question at compile time.
    pub fn storage_class(&self) -> StorageClass {
        match self {
            Self::PostgreSQL | Self::MySQL | Self::SQLite | Self::ClickHouse => {
                StorageClass::Relational
            }
            Self::Redis => StorageClass::KeyValue,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CredentialStorageMode {
    Keychain,
    EncryptedSqlite,
    PlainSqlite,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppSettingsSnapshot {
    pub onboarding_completed: bool,
    pub credential_storage_mode: Option<CredentialStorageMode>,
    pub credential_state: CredentialState,
    pub config_dir: String,
    /// User-chosen theme mode: `"system"`, `"light"`, or `"dark"`.
    /// `None` resolves to `"system"` in the frontend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// User-chosen preset: `"default" | "dracula" | "github" | "gruvbox"`.
    /// `None` resolves to `"default"` in the frontend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_preset: Option<String>,
}

/// Both fields are optional so callers can update mode and preset
/// independently. Validated by the command handler.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveAppSettingsPayload {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub theme_preset: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CredentialState {
    NeedsOnboarding,
    NeedsUnlock,
    Ready,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfigureCredentialStoragePayload {
    pub mode: CredentialStorageMode,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnlockCredentialsPayload {
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangeCredentialStoragePayload {
    pub mode: CredentialStorageMode,
    pub password: Option<String>,
    pub confirm: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SshTunnelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bastion_server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_bind_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_port: Option<u16>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub compression: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_interval_seconds: Option<u32>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub keepalive_want_reply: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jump_chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_command: Option<String>,
}

impl Default for SshTunnelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bastion_server_id: None,
            local_bind_host: None,
            local_port: None,
            compression: false,
            keepalive_interval_seconds: None,
            keepalive_want_reply: true,
            jump_chain: Vec::new(),
            proxy_command: None,
        }
    }
}

impl SshTunnelConfig {
    pub fn is_default(&self) -> bool {
        !self.enabled
            && self.bastion_server_id.is_none()
            && self.local_bind_host.is_none()
            && self.local_port.is_none()
            && !self.compression
            && self.keepalive_interval_seconds.is_none()
            && self.keepalive_want_reply
            && self.jump_chain.is_empty()
            && self.proxy_command.is_none()
    }

    pub fn normalized(&self) -> Self {
        if !self.enabled {
            return Self::default();
        }
        let bastion_server_id = normalize_optional_text(self.bastion_server_id.as_deref());
        let local_bind_host = normalize_optional_text(self.local_bind_host.as_deref());
        let proxy_command = normalize_optional_text(self.proxy_command.as_deref());
        let jump_chain = self
            .jump_chain
            .iter()
            .filter_map(|bastion_id| normalize_optional_text(Some(bastion_id)))
            .collect::<Vec<_>>();

        Self {
            enabled: true,
            bastion_server_id,
            local_bind_host,
            local_port: self.local_port,
            compression: self.compression,
            keepalive_interval_seconds: self.keepalive_interval_seconds,
            keepalive_want_reply: self.keepalive_want_reply,
            jump_chain,
            proxy_command,
        }
    }

    pub fn referenced_bastion_ids(&self) -> Vec<String> {
        let normalized = self.normalized();
        if !normalized.enabled {
            return Vec::new();
        }
        let mut ids = Vec::new();
        for bastion_id in normalized.jump_chain {
            if !ids.iter().any(|id| id == &bastion_id) {
                ids.push(bastion_id);
            }
        }
        if let Some(bastion_id) = normalized.bastion_server_id {
            if !ids.iter().any(|id| id == &bastion_id) {
                ids.push(bastion_id);
            }
        }
        ids
    }

    pub fn references_bastion(&self, bastion_id: &str) -> bool {
        self.referenced_bastion_ids()
            .iter()
            .any(|id| id == bastion_id)
    }
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BastionAuthMethod {
    Password,
    PrivateKeyPath,
    PrivateKeyContent,
}

impl BastionAuthMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKeyPath => "privateKeyPath",
            Self::PrivateKeyContent => "privateKeyContent",
        }
    }
}

impl FromStr for BastionAuthMethod {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "password" => Ok(Self::Password),
            "privateKeyPath" => Ok(Self::PrivateKeyPath),
            "privateKeyContent" => Ok(Self::PrivateKeyContent),
            _ => Err(format!("unknown bastion auth method '{value}'")),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", tag = "action")]
pub(crate) enum SecretChange {
    #[default]
    Keep,
    Set {
        value: String,
    },
    Clear,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BastionServer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_method: BastionAuthMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key_fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicBastionServer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_method: BastionAuthMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key_fingerprint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub has_password: bool,
    pub has_private_key_content: bool,
    pub has_passphrase: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveBastionServerPayload {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_method: BastionAuthMethod,
    #[serde(default)]
    pub private_key_path: Option<String>,
    #[serde(default)]
    pub password: SecretChange,
    #[serde(default)]
    pub private_key_content: SecretChange,
    #[serde(default)]
    pub passphrase: SecretChange,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BastionServerPayload {
    pub bastion_server_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestBastionResult {
    pub latency_ms: u64,
}

/// `StoredConnection` is a per-engine tagged union — the backend's
/// counterpart to the frontend's `Connection` discriminated union
/// (see ADR-0010). Each variant carries exactly the fields its engine
/// uses: `ssl` on PG/MySQL, `useHttps`/`urlPath` on ClickHouse,
/// `dbNumber`/`useTls`/`verifyTlsCert` on Redis, nothing extra on
/// SQLite.
///
/// Serialized internally-tagged on `engine`, so the wire JSON is flat:
/// `{ "engine": "PostgreSQL", "id": "...", "host": "...", "ssl": true, ... }`.
/// This matches the previous flat-struct shape byte-for-byte except
/// that engine-irrelevant fields are now absent from the wire when
/// they don't apply (the frontend's optional `useHttps?` / `dbNumber?`
/// types tolerate the absence).
///
/// The enum mirrors the **Credential Backend** pattern: closed
/// variant set, exhaustive dispatch, per-variant logic concentrated.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "engine")]
pub(crate) enum StoredConnection {
    PostgreSQL(PgStoredConnection),
    MySQL(MySqlStoredConnection),
    SQLite(SqliteStoredConnection),
    ClickHouse(ClickHouseStoredConnection),
    Redis(RedisStoredConnection),
}

/// Organization metadata shared by every stored-connection variant
/// (Plan 009): a single-level `folder` (empty = ungrouped), the
/// favorite flag, and an opaque presentation `color` token that the
/// frontend validates. Flattened into each variant so the wire shape
/// stays `folder` / `isFavorite` / `color` at the top level.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionOrganization {
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub color: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgStoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub safe_mode: SafeMode,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Folder / favorite / color; see `ConnectionOrganization`.
    #[serde(default, flatten)]
    pub organization: ConnectionOrganization,
    /// Legacy TLS on/off mirror. `tls_options.mode` is authoritative when
    /// present (ADR-0025); storage normalizes this flag to
    /// `mode != disable` on every save so the two can never disagree on
    /// disk. Read TLS decisions through [`PgStoredConnection::resolved_tls_mode`],
    /// never through this field. Distinct concept from ClickHouse's
    /// `useHttps` and Redis's `useTls` — see CONTEXT.md `Connection`.
    #[serde(default = "default_true")]
    pub ssl: bool,
    /// TLS verification mode and certificate material (paths). `None`
    /// on legacy rows, which resolve through `ssl`. See ADR-0025.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_options: Option<PgTlsOptions>,
    /// Optional driver/session knobs applied after every connect.
    /// See ADR-0013. Missing or empty fields fall back to PG defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_options: Option<PgDriverOptions>,
    #[serde(default, skip_serializing_if = "SshTunnelConfig::is_default")]
    pub ssh_tunnel: SshTunnelConfig,
}

impl PgStoredConnection {
    /// The single source of truth for "which TLS mode does this
    /// connection use". `tls_options` wins when present; legacy rows
    /// without it map `ssl` to `prefer` / `disable`, which is exactly
    /// the behaviour those rows had before ADR-0025.
    pub(crate) fn resolved_tls_mode(&self) -> PgTlsMode {
        match &self.tls_options {
            Some(options) => options.mode,
            None if self.ssl => PgTlsMode::Prefer,
            None => PgTlsMode::Disable,
        }
    }

    pub(crate) fn effective_port(&self) -> u16 {
        if self.port == 0 {
            5432
        } else {
            self.port
        }
    }

    /// Legacy `ssl` mirror of `tls_options.mode`. Legacy rows (no blob)
    /// keep the stored flag as given.
    pub(crate) fn ssl_mirror(&self) -> bool {
        match &self.tls_options {
            Some(options) => options.mode != PgTlsMode::Disable,
            None => self.ssl,
        }
    }

    /// Bring `ssl` in line with `tls_options.mode` (no-op on legacy rows).
    pub(crate) fn normalize_tls(&mut self) {
        if self.tls_options.is_some() {
            self.ssl = self.ssl_mirror();
        }
    }
}

/// libpq's `sslmode` vocabulary, persisted and sent over the wire in the
/// same spelling so URIs, `PGSSLMODE`, and the form share one set of
/// strings. See ADR-0025.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTlsMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl PgTlsMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }

    /// Whether the handshake authenticates the server's certificate chain.
    pub(crate) fn verifies_chain(self) -> bool {
        matches!(self, Self::VerifyCa | Self::VerifyFull)
    }

    /// Whether the certificate must also match the expected host name.
    pub(crate) fn verifies_hostname(self) -> bool {
        self == Self::VerifyFull
    }
}

/// Why a TLS-protected connect failed, as a typed wire value shared by
/// the actor error unions and the diagnosis report (ADR-0025).
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TlsFailureKind {
    /// The server answered the SSLRequest with `N` under a mode that
    /// requires encryption.
    ServerRefusedTls,
    /// Chain validation failed: unknown issuer, expired, wrong purpose.
    CertificateUntrusted,
    /// The chain is trusted but names a different host.
    HostnameMismatch,
    /// The server rejected our client certificate during the handshake.
    ClientCertificateRejected,
    /// Local CA / client material is missing, unreadable, malformed, or
    /// encrypted — detected before any socket opened.
    InvalidLocalMaterial,
    /// Any other handshake failure.
    HandshakeFailed,
}

/// TLS material for a PostgreSQL connection, persisted as one JSON blob
/// (migration 18, `tls_options`). Paths, never contents: the client key
/// stays on disk under the user's control and never enters the SQLite
/// store or the credential blob. Every path is optional; `None` means
/// "use the platform trust store" (roots) or "no client auth".
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgTlsOptions {
    #[serde(default)]
    pub mode: PgTlsMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_cert_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key_path: Option<String>,
    /// Host name the server certificate must match when it differs from
    /// `host` (IP-literal hosts, SSH tunnels). The tunnel fills this on
    /// the resolved copy at connect time and never persists it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
}

/// Per-connection driver/session knobs persisted on the Postgres
/// connection record. Reads as a single JSON blob in SQLite so adding
/// a new knob is a struct field, not a schema migration. See
/// ADR-0013.
///
/// Every field is optional — `None` means "use the server default".
/// Empty `default_search_path` is treated the same as `None`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDriverOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statement_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_in_transaction_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_search_path: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_role: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MySqlStoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub safe_mode: SafeMode,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Folder / favorite / color; see `ConnectionOrganization`.
    #[serde(default, flatten)]
    pub organization: ConnectionOrganization,
    #[serde(default = "default_true")]
    pub ssl: bool,
    #[serde(default, skip_serializing_if = "SshTunnelConfig::is_default")]
    pub ssh_tunnel: SshTunnelConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SqliteStoredConnection {
    pub id: String,
    pub name: String,
    /// File path to the SQLite database (or `:memory:`).
    pub database: String,
    /// Sentinel fields preserved for wire-compatibility with the
    /// frontend's flat `Connection` shape (host/port/user/password
    /// are `required` on the TS side). Always empty for SQLite.
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    pub role: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub safe_mode: SafeMode,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Folder / favorite / color; see `ConnectionOrganization`.
    #[serde(default, flatten)]
    pub organization: ConnectionOrganization,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClickHouseStoredConnection {
    pub id: String,
    pub name: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub safe_mode: SafeMode,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Folder / favorite / color; see `ConnectionOrganization`.
    #[serde(default, flatten)]
    pub organization: ConnectionOrganization,
    /// When true the URL builder uses `https://` and defaults the
    /// port to 8443 instead of 8123.
    #[serde(default)]
    pub use_https: bool,
    /// Optional URL path prefix for deployments that proxy CH behind
    /// a path (e.g. `/clickhouse`). Empty = root.
    #[serde(default)]
    pub url_path: String,
    #[serde(default, skip_serializing_if = "SshTunnelConfig::is_default")]
    pub ssh_tunnel: SshTunnelConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisStoredConnection {
    pub id: String,
    pub name: String,
    /// Frontend sends this as `""` for Redis; Redis uses `db_number`
    /// for keyspace selection. Kept on the wire for shape stability.
    #[serde(default)]
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub role: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub safe_mode: SafeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// Folder / favorite / color; see `ConnectionOrganization`.
    #[serde(default, flatten)]
    pub organization: ConnectionOrganization,
    /// Which numbered DB (0–15 on standalone).
    #[serde(default)]
    pub db_number: u8,
    /// Connect over TLS (`rediss://`).
    #[serde(default)]
    pub use_tls: bool,
    /// Verify the TLS certificate. Only meaningful when `use_tls` is
    /// true. Default true; users disable for self-signed dev servers.
    #[serde(default = "default_true")]
    pub verify_tls_cert: bool,
    /// Belt-and-braces safety toggle: when `true`, the backend
    /// rejects every write (`assert_writable`) for this connection,
    /// independent of the replica-role check. Defaults to `false`.
    /// See ADR-0009.
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "SshTunnelConfig::is_default")]
    pub ssh_tunnel: SshTunnelConfig,
}

fn default_true() -> bool {
    true
}

impl StoredConnection {
    pub fn engine(&self) -> DatabaseEngine {
        match self {
            Self::PostgreSQL(_) => DatabaseEngine::PostgreSQL,
            Self::MySQL(_) => DatabaseEngine::MySQL,
            Self::SQLite(_) => DatabaseEngine::SQLite,
            Self::ClickHouse(_) => DatabaseEngine::ClickHouse,
            Self::Redis(_) => DatabaseEngine::Redis,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.id,
            Self::MySQL(c) => &c.id,
            Self::SQLite(c) => &c.id,
            Self::ClickHouse(c) => &c.id,
            Self::Redis(c) => &c.id,
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

    pub fn database(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.database,
            Self::MySQL(c) => &c.database,
            Self::SQLite(c) => &c.database,
            Self::ClickHouse(c) => &c.database,
            Self::Redis(c) => &c.database,
        }
    }

    pub fn host(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.host,
            Self::MySQL(c) => &c.host,
            Self::SQLite(c) => &c.host,
            Self::ClickHouse(c) => &c.host,
            Self::Redis(c) => &c.host,
        }
    }

    pub fn port(&self) -> u16 {
        match self {
            Self::PostgreSQL(c) => c.port,
            Self::MySQL(c) => c.port,
            Self::SQLite(c) => c.port,
            Self::ClickHouse(c) => c.port,
            Self::Redis(c) => c.port,
        }
    }

    pub(crate) fn effective_port(&self) -> u16 {
        match self {
            Self::PostgreSQL(c) => c.effective_port(),
            Self::MySQL(c) => {
                if c.port == 0 {
                    3306
                } else {
                    c.port
                }
            }
            Self::ClickHouse(c) => {
                if c.port == 0 {
                    if c.use_https {
                        8443
                    } else {
                        8123
                    }
                } else {
                    c.port
                }
            }
            Self::Redis(c) => {
                if c.port == 0 {
                    6379
                } else {
                    c.port
                }
            }
            Self::SQLite(c) => c.port,
        }
    }

    pub fn user(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.user,
            Self::MySQL(c) => &c.user,
            Self::SQLite(c) => &c.user,
            Self::ClickHouse(c) => &c.user,
            Self::Redis(c) => &c.user,
        }
    }

    pub fn password(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.password,
            Self::MySQL(c) => &c.password,
            Self::SQLite(c) => &c.password,
            Self::ClickHouse(c) => &c.password,
            Self::Redis(c) => &c.password,
        }
    }

    pub fn role(&self) -> &str {
        match self {
            Self::PostgreSQL(c) => &c.role,
            Self::MySQL(c) => &c.role,
            Self::SQLite(c) => &c.role,
            Self::ClickHouse(c) => &c.role,
            Self::Redis(c) => &c.role,
        }
    }

    /// Clone this connection as a new record: fresh id and name, no
    /// favorite flag, no activity, and no in-memory password (the
    /// credential copy travels through the credential backend, never
    /// the wire). Plan 009.
    pub(crate) fn duplicated_as(&self, id: String, name: String) -> Self {
        let mut copy = self.clone();
        macro_rules! reset {
            ($c:expr) => {{
                $c.id = id;
                $c.name = name;
                $c.last_activity_at = None;
                $c.password = String::new();
            }};
        }
        match &mut copy {
            Self::PostgreSQL(c) => reset!(c),
            Self::MySQL(c) => reset!(c),
            Self::SQLite(c) => reset!(c),
            Self::ClickHouse(c) => reset!(c),
            Self::Redis(c) => reset!(c),
        }
        copy.organization_mut().is_favorite = false;
        copy
    }

    pub fn organization(&self) -> &ConnectionOrganization {
        match self {
            Self::PostgreSQL(c) => &c.organization,
            Self::MySQL(c) => &c.organization,
            Self::SQLite(c) => &c.organization,
            Self::ClickHouse(c) => &c.organization,
            Self::Redis(c) => &c.organization,
        }
    }

    pub fn organization_mut(&mut self) -> &mut ConnectionOrganization {
        match self {
            Self::PostgreSQL(c) => &mut c.organization,
            Self::MySQL(c) => &mut c.organization,
            Self::SQLite(c) => &mut c.organization,
            Self::ClickHouse(c) => &mut c.organization,
            Self::Redis(c) => &mut c.organization,
        }
    }

    pub fn folder(&self) -> &str {
        &self.organization().folder
    }

    pub fn is_favorite(&self) -> bool {
        self.organization().is_favorite
    }

    pub fn color(&self) -> &str {
        &self.organization().color
    }

    pub(crate) fn policy(&self) -> ConnectionPolicy {
        match self {
            Self::PostgreSQL(c) => ConnectionPolicy {
                environment: c.environment,
                safe_mode: c.safe_mode,
                read_only: c.read_only,
            },
            Self::MySQL(c) => ConnectionPolicy {
                environment: c.environment,
                safe_mode: c.safe_mode,
                read_only: c.read_only,
            },
            Self::SQLite(c) => ConnectionPolicy {
                environment: c.environment,
                safe_mode: c.safe_mode,
                read_only: c.read_only,
            },
            Self::ClickHouse(c) => ConnectionPolicy {
                environment: c.environment,
                safe_mode: c.safe_mode,
                read_only: c.read_only,
            },
            Self::Redis(c) => ConnectionPolicy {
                environment: c.environment,
                safe_mode: c.safe_mode,
                read_only: c.read_only,
            },
        }
    }

    pub fn last_activity_at(&self) -> Option<&str> {
        match self {
            Self::PostgreSQL(c) => c.last_activity_at.as_deref(),
            Self::MySQL(c) => c.last_activity_at.as_deref(),
            Self::SQLite(c) => c.last_activity_at.as_deref(),
            Self::ClickHouse(c) => c.last_activity_at.as_deref(),
            Self::Redis(c) => c.last_activity_at.as_deref(),
        }
    }

    /// Replace the password on whichever variant is active. Used by
    /// `credentials::hydrate` to inject the resolved password before
    /// passing the connection to the engine dispatchers.
    pub fn set_password(&mut self, password: String) {
        match self {
            Self::PostgreSQL(c) => c.password = password,
            Self::MySQL(c) => c.password = password,
            Self::SQLite(c) => c.password = password,
            Self::ClickHouse(c) => c.password = password,
            Self::Redis(c) => c.password = password,
        }
    }

    pub fn set_network_endpoint(&mut self, host: String, port: u16) -> Result<(), String> {
        match self {
            Self::PostgreSQL(c) => {
                c.host = host;
                c.port = port;
            }
            Self::MySQL(c) => {
                c.host = host;
                c.port = port;
            }
            Self::ClickHouse(c) => {
                c.host = host;
                c.port = port;
            }
            Self::Redis(c) => {
                c.host = host;
                c.port = port;
            }
            Self::SQLite(_) => {
                return Err("SQLite connections do not support SSH tunnels".to_string());
            }
        }
        Ok(())
    }

    pub fn ssh_tunnel(&self) -> Option<&SshTunnelConfig> {
        match self {
            Self::PostgreSQL(c) => Some(&c.ssh_tunnel),
            Self::MySQL(c) => Some(&c.ssh_tunnel),
            Self::SQLite(_) => None,
            Self::ClickHouse(c) => Some(&c.ssh_tunnel),
            Self::Redis(c) => Some(&c.ssh_tunnel),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafetyOverrideRecord {
    pub command: String,
    pub classes: Vec<String>,
    pub occurred_at: String,
}

/// Connect-time pipeline result for Redis. Surfaced in the connection-
/// test "modules-detected" banner. Every field is `Option<>` because
/// managed Redis (Upstash hobby tier, ElastiCache locked-down ACLs)
/// often restricts `INFO` sections or `MODULE LIST` — degrading
/// per-field is better than failing the whole connect-test.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisCapabilities {
    /// `redis_version` from `INFO server` — e.g. `"7.2.4"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// `role` from `INFO replication` — `"master"` or `"replica"`.
    /// Drives auto-read-only (ADR-0009).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// `connected_slaves` from `INFO replication`. Drives the soft
    /// "this may be a production master" notice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_slaves: Option<u32>,
    /// `MODULE LIST` results. `None` when the command was rejected;
    /// `Some(vec![])` when it succeeded with zero modules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<Vec<RedisModuleInfo>>,
    /// `DBSIZE` for the active DB. `None` if rejected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_size: Option<u64>,
    /// `maxmemory-policy` from `CONFIG GET`. Drives whether
    /// `OBJECT FREQ` is meaningful on keys (LFU policies only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maxmemory_policy: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisModuleInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedisCliHistoryEntry {
    pub id: String,
    pub connection_id: String,
    pub command: String,
    pub submitted_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedRedisCommand {
    pub id: String,
    pub name: String,
    pub body: String,
    /// `None` = engine-portable, not pinned to a specific connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub is_favorite: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryHistoryEntry {
    pub id: String,
    pub sql: String,
    pub connection_id: String,
    pub connection_name: String,
    pub database: String,
    pub engine: DatabaseEngine,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub runtime_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u64>,
    pub started_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedQuery {
    pub id: String,
    pub name: String,
    pub body: String,
    /// `None` = saved query is not pinned to a specific connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub is_favorite: bool,
    /// Reserved for future cloud-sync. Local-only writes leave this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ---------------------------------------------------------------------------
// Generic command results
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub runtime_ms: u64,
    pub row_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectResult {
    pub latency_ms: u64,
    /// Populated when the connection is to a Redis server — the
    /// post-test-connection banner reads from here. `None` for
    /// relational engines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redis_capabilities: Option<RedisCapabilities>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub(crate) enum HealthCheckResult {
    #[serde(rename = "healthy")]
    Healthy { latency_ms: u64 },
    #[serde(rename = "error")]
    Error { error: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableDataResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub page: u32,
    pub page_size: u32,
    pub total_rows: Option<u64>,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteDdlResult {
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitCellEditsResult {
    pub rows_affected: u64,
    pub runtime_ms: u64,
    /// "committed" — the change is applied (PostgreSQL).
    /// "queued"    — the change is accepted but applied asynchronously
    ///   (ClickHouse `ALTER … UPDATE`). The frontend polls
    ///   `poll_mutation_status` until all `mutation_ids` report
    ///   `is_done = true`.
    pub state: String,
    /// Database the mutations apply to. Needed alongside `mutation_ids`
    /// because CH's `system.mutations` is keyed by `(database, table,
    /// mutation_id)`. Empty for synchronous engines.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub database: String,
    /// Table the mutations apply to. Empty for synchronous engines.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub table: String,
    /// One ID per `ALTER TABLE … UPDATE` we issued. Empty for
    /// synchronous engines (PostgreSQL).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mutation_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsertRowResult {
    pub runtime_ms: u64,
    pub rows_affected: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRowsResult {
    pub runtime_ms: u64,
    pub rows_affected: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportDdlResult {
    pub sql: String,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDumpResult {
    pub data_base64: String,
    pub extension: String,
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgRestoreResult {
    pub runtime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyTableResult {
    pub runtime_ms: u64,
    pub rows_copied: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgSessionInfo {
    pub pid: i32,
    pub user: String,
    pub database: Option<String>,
    pub application_name: String,
    pub client_addr: Option<String>,
    pub state: Option<String>,
    pub wait_event_type: Option<String>,
    pub wait_event: Option<String>,
    pub query_age_seconds: Option<i64>,
    pub transaction_age_seconds: Option<i64>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgLockInfo {
    pub pid: i32,
    pub lock_type: String,
    pub relation: Option<String>,
    pub mode: String,
    pub granted: bool,
    pub blocked_by: Vec<i32>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgPendingTransactionInfo {
    pub pid: i32,
    pub user: String,
    pub state: Option<String>,
    pub transaction_age_seconds: Option<i64>,
    pub query: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgAdminStats {
    pub database_size_bytes: i64,
    pub cache_hit_ratio: Option<f64>,
    pub active_sessions: i64,
    pub idle_in_transaction: i64,
    pub blocked_locks: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgAdminSnapshot {
    pub sessions: Vec<PgSessionInfo>,
    pub locks: Vec<PgLockInfo>,
    pub pending_transactions: Vec<PgPendingTransactionInfo>,
    pub stats: PgAdminStats,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgBackendActionResult {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteRowsResult {
    pub runtime_ms: u64,
    pub rows_affected: u64,
    /// "committed" or "queued" — same semantics as `CommitCellEditsResult.state`.
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub database: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub table: String,
    /// One per `ALTER TABLE … DELETE` (CH). Empty for PG.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mutation_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MutationStatus {
    pub mutation_id: String,
    pub is_done: bool,
    /// Populated when CH reports `latest_fail_reason` on
    /// `system.mutations`.
    pub latest_fail_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PollMutationStatusPayload {
    pub connection_id: String,
    pub database: String,
    pub table: String,
    pub mutation_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// Command payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunQueryPayload {
    pub connection_id: String,
    pub query: String,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadTableDataPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadTableStructurePayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

// ---------------------------------------------------------------------------
// Managed Servers (ADR-0019)
// ---------------------------------------------------------------------------

/// A local database server dbunk provisioned and owns: a Docker
/// container plus a named data volume with independent lifetimes.
/// Separate entity from `StoredConnection`; the link is one-way via
/// `connection_id` (the auto-created Connection that points at it).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedServer {
    pub id: String,
    pub name: String,
    pub engine: DatabaseEngine,
    /// Major-version image tag, e.g. `"17"` for `postgres:17`.
    pub version: String,
    pub port: u16,
    pub container_name: String,
    pub volume_name: String,
    pub database: String,
    pub user: String,
    pub connection_id: Option<String>,
    pub created_at: String,
}

/// `ManagedServer` plus its live status, always derived from Docker at
/// observation time — never trusted from the stored record.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedServerWithStatus {
    #[serde(flatten)]
    pub server: ManagedServer,
    /// `"running"` | `"stopped"` | `"orphaned"`.
    pub status: String,
    /// For `orphaned`: whether the named volume survived, i.e. whether
    /// Recreate can restore the server with its data intact.
    pub volume_exists: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DockerStatus {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvisionManagedServerPayload {
    pub name: String,
    pub engine: DatabaseEngine,
    pub version: String,
    /// Omitted: dbunk scans for a free port from the engine's
    /// non-default base (5433+/3307+).
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvisionManagedServerResult {
    pub server: ManagedServer,
    pub connection_id: String,
    /// Shown once after creation so the user can paste it into their
    /// project's env; the password is also persisted via the
    /// credential backend like any other connection credential.
    pub connection_string: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedServerPayload {
    pub managed_server_id: String,
}

// ---------------------------------------------------------------------------
// Table Seeding (ADR-0020)
// ---------------------------------------------------------------------------

/// Per-column entry of a Seed Spec. Source precedence:
/// `skip` > `constant` > `values` > FK sampling > `generator`/auto.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedColumnSpec {
    pub column: String,
    #[serde(default)]
    pub skip: bool,
    #[serde(default)]
    pub constant: Option<String>,
    #[serde(default)]
    pub values: Option<Vec<String>>,
    /// Generator id override (e.g. `"email"`, `"price"`).
    #[serde(default)]
    pub generator: Option<String>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    /// Probability `[0, 1]` of NULL for nullable columns.
    #[serde(default)]
    pub null_rate: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedTablePayload {
    pub operation_id: String,
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub row_count: u32,
    /// Omitted seed: backend picks one and reports it in the result so
    /// every run is reproducible.
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub columns: Vec<SeedColumnSpec>,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedTableProgress {
    pub operation_id: String,
    pub rows_completed: u64,
    pub total_rows: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SeedTableResult {
    pub rows_inserted: u64,
    pub seed_used: u64,
    pub runtime_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadSchemaRelationshipsPayload {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadTableSchemaRelationshipsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaMapScopePayload {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveSchemaMapPositionPayload {
    pub connection_id: String,
    pub schema: String,
    pub table_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaMapPrefsPatch {
    #[serde(default)]
    pub routing: Option<String>,
    #[serde(default)]
    pub attr_mode: Option<String>,
    #[serde(default)]
    pub show_types: Option<bool>,
    #[serde(default)]
    pub show_nulls: Option<bool>,
    #[serde(default)]
    pub show_comments: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveSchemaMapPrefsPayload {
    pub connection_id: String,
    pub schema: String,
    pub patch: SchemaMapPrefsPatch,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadDatabaseOverviewStatsPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadRelationStatsPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadServerDetailsPayload {
    pub connection_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecuteDdlPayload {
    pub connection_id: String,
    pub sql: String,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellEditKeyValue {
    pub column: String,
    // None == NULL when binding the parameter. Serde turns missing or null
    // JSON values into None, anything else becomes the string verbatim. We
    // intentionally keep types as strings here: the UI only ever has the
    // string form in hand and Postgres is happy to accept text params for
    // most types via `&str` binding (it coerces server-side).
    pub value: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CellEdit {
    // Echoed back through to the caller for UI mapping; ignored by SQL.
    #[serde(default)]
    #[allow(dead_code)]
    pub row_index: u32,
    pub identity: Vec<CellEditKeyValue>,
    pub set: Vec<CellEditKeyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitCellEditsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub edits: Vec<CellEdit>,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InsertRowPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    // column -> Option<String> (None = NULL). Columns omitted from this list
    // get the database default — that lets users insert rows that rely on
    // SERIAL/identity columns or DEFAULT NOW() etc.
    pub values: Vec<CellEditKeyValue>,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRowsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub use_copy: bool,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportDdlPayload {
    pub connection_id: String,
    pub scope: String,
    pub schema: Option<String>,
    pub table: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgDumpPayload {
    pub connection_id: String,
    pub scope: String,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub format: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgRestorePayload {
    pub connection_id: String,
    pub data_base64: String,
    pub format: String,
    #[serde(default)]
    pub clean: bool,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyTablePayload {
    pub source_connection_id: String,
    pub source_schema: String,
    pub source_table: String,
    pub destination_connection_id: String,
    pub destination_schema: String,
    pub destination_table: String,
    pub page_size: Option<u32>,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefreshMaterializedViewPayload {
    pub connection_id: String,
    pub schema: String,
    pub view: String,
    #[serde(default)]
    pub concurrently: bool,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgBackendActionPayload {
    pub connection_id: String,
    pub pid: i32,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgMaintenancePayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    pub action: String,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteRowsPayload {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
    // Each Vec<CellEditKeyValue> is the identity for one row. We delete each
    // identified row inside a single transaction; if any identified row
    // misses (rows_affected == 0) we ROLLBACK the whole batch.
    pub rows: Vec<Vec<CellEditKeyValue>>,
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteSavedQueryPayload {
    pub id: String,
}

// ---------------------------------------------------------------------------
// Schema introspection — returned by load_table_structure
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub ordinal_position: i32,
    /// Engine-specific derivation marker. ClickHouse populates this
    /// with `"MATERIALIZED"` / `"ALIAS"` / `"EPHEMERAL"` for derived
    /// columns; PostgreSQL leaves it `None`. Frontend uses it to
    /// render a "derived" icon in the column header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derivation_kind: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<String>,
    pub on_delete: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
    pub method: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConstraintInfo {
    pub name: String,
    pub kind: String,
    pub definition: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StructureCapabilities {
    pub columns: bool,
    pub primary_key: bool,
    pub foreign_keys: bool,
    pub indexes: bool,
    pub constraints: bool,
    /// Per-table mutation capabilities. The frontend reads these to
    /// decide whether to show edit/insert/delete UI rather than gating
    /// on engine name — this matters for ClickHouse where MergeTree
    /// tables accept mutations and Distributed/View tables don't.
    pub can_insert_rows: bool,
    pub can_update_rows: bool,
    pub can_delete_rows: bool,
    pub can_alter_schema: bool,
    /// "exact" — identity columns guarantee at most one matching row
    ///   (PostgreSQL with a real PK).
    /// "best-effort" — identity may match multiple rows (ClickHouse,
    ///   where sorting key is not a uniqueness constraint).
    pub uniqueness_guarantee: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableStructure {
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Option<Vec<String>>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
    pub indexes: Vec<IndexInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub capabilities: StructureCapabilities,
    /// Engine-specific extension fields. Currently populated only for
    /// ClickHouse — the storage engine name (`MergeTree`,
    /// `ReplicatedMergeTree`, `Distributed`, `View`, …) and the
    /// MergeTree partition / sampling expressions. Left `None` for
    /// engines that don't have these concepts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_by: Option<String>,
}

// ---------------------------------------------------------------------------
// Schema relationships — returned by load_schema_relationships
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaTableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub ordinal_position: i32,
    pub comment: Option<String>,
}

/// Compact trigger metadata for Table Cards and Column Rows in the
/// Schema Map. Full trigger function bodies / DDL stay out of this
/// payload by design.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaTableTrigger {
    pub name: String,
    pub table: String,
    /// Columns the trigger explicitly targets (PostgreSQL
    /// `UPDATE OF column`); empty when it fires for the whole table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// `BEFORE` | `AFTER` | `INSTEAD OF`.
    pub timing: String,
    /// Any of `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`.
    pub events: Vec<String>,
    /// `ROW` | `STATEMENT`.
    pub orientation: String,
    pub enabled: bool,
    pub function_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaTableNode {
    pub schema: String,
    pub name: String,
    pub column_count: u32,
    pub columns: Vec<SchemaTableColumn>,
    /// Junction Table Card marker — `Some(true)` when the table is
    /// detected as a many-to-many association. `None` on engines that
    /// don't compute junction detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_junction_table: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<SchemaTableTrigger>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaForeignKey {
    pub constraint_name: String,
    pub from_schema: String,
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_schema: String,
    pub to_table: String,
    pub to_columns: Vec<String>,
    /// `"foreign key"` for FK-backed Relationship Edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship_type: Option<String>,
    /// Relationship Cardinality: `one-to-one` | `one-to-many` |
    /// `unknown`. Engines without enough metadata return `unknown` or
    /// omit the field entirely rather than guessing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cardinality_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_delete: Option<String>,
    /// `true` when any referencing (FK) column is nullable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fk_columns_nullable: Option<bool>,
    /// `true` when the referencing columns are covered by a unique
    /// constraint on the referencing table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fk_columns_unique: Option<bool>,
    /// `Some(true)` when the edge participates in a detected
    /// junction-table (many-to-many) path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_junction_participant: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaRelationships {
    pub tables: Vec<SchemaTableNode>,
    pub foreign_keys: Vec<SchemaForeignKey>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PositionRow {
    pub table_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaMapPrefs {
    pub routing: String,
    pub attr_mode: String,
    pub show_types: bool,
    pub show_nulls: bool,
    pub show_comments: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SchemaExplorer {
    pub name: String,
    pub tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materialized_views: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequences: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign_tables: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aggregate_functions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_triggers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tablespaces: Vec<String>,
}

// ---------------------------------------------------------------------------
// Database overview stats
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatabaseOverviewStats {
    pub database_size_bytes: i64,
    pub table_size_bytes: i64,
    pub index_size_bytes: i64,
    pub table_count: i64,
    pub schema_count: i64,
    pub row_count_estimate: i64,
    pub index_count: i64,
    pub connection_count: i64,
}

// ---------------------------------------------------------------------------
// Per-relation stats (Tables + Schemas sub-tabs)
// ---------------------------------------------------------------------------

/// One row per user-visible relation (table, view, materialised view)
/// in the connection's catalogue, used to populate the Tables sub-tab
/// and to derive the Schemas sub-tab's per-schema aggregates in the
/// frontend. Today only Postgres is implemented; other relational
/// engines return an empty list — the Schemas sub-tab is gated to PG
/// in the UI and the Tables sub-tab degrades to schema/name/kind
/// columns on non-PG.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelationInfo {
    pub schema: String,
    pub name: String,
    /// One of "table", "view", "materialized view". Matches the
    /// kind values rendered by the Tables sub-tab badge.
    pub kind: String,
    /// `pg_class.reltuples` cast to bigint — a planner estimate, not
    /// an exact COUNT. Zero when the estimate is missing or the row
    /// is a view (where it has no useful meaning).
    pub row_count_estimate: i64,
    /// `pg_total_relation_size` in bytes — table plus its TOAST and
    /// every index. Zero for views.
    pub total_size_bytes: i64,
}

// ---------------------------------------------------------------------------
// Server details (Details sub-tab)
// ---------------------------------------------------------------------------

/// One row from `pg_settings` — a single GUC parameter. The category
/// + short_desc + source fields drive the Details sub-tab's grouping,
///   tooltip, and "modified from default" highlight.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgSetting {
    pub name: String,
    pub setting: String,
    pub unit: Option<String>,
    pub category: String,
    pub short_desc: Option<String>,
    /// `default`, `configuration file`, `command line`, `session`,
    /// `client`, `database`, `user`, `override`, etc. The frontend
    /// highlights any non-"default" value to surface operator
    /// overrides.
    pub source: String,
    pub boot_val: Option<String>,
    pub reset_val: Option<String>,
}

/// One row per installed Postgres extension, surfaced read-only on
/// the Details sub-tab. Install/drop UI is explicitly Phase 6, not
/// Phase 1.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgExtension {
    pub name: String,
    pub version: String,
    pub schema: String,
    pub description: Option<String>,
}

/// Aggregate snapshot returned by `load_server_details` — the Details
/// sub-tab's data source. Postgres-only; non-PG connections never
/// invoke the command (the UI renders a Postgres-only panel).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerDetails {
    pub server_version: String,
    pub encoding: String,
    pub locale: String,
    pub timezone: String,
    pub settings: Vec<PgSetting>,
    pub extensions: Vec<PgExtension>,
}
