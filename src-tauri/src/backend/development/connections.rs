//! Redacted connection metadata for the native shell. Secrets are separate input
//! arguments and never appear in a form model, serialized response or Debug.
use super::*;
use crate::credentials::native_connections::{self, Change};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DevelopmentEnvironment {
    #[default]
    Development,
    Test,
    Staging,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DevelopmentSafeMode {
    #[default]
    Inherit,
    Disabled,
    Protected,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DevelopmentTlsMode {
    #[default]
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentTlsOptions {
    pub mode: DevelopmentTlsMode,
    pub root_cert_path: Option<String>,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentDriverOptions {
    pub statement_timeout_ms: Option<u32>,
    pub idle_in_transaction_timeout_ms: Option<u32>,
    pub connect_timeout_ms: Option<u32>,
    pub keepalive_seconds: Option<u32>,
    pub default_search_path: Option<Vec<String>>,
    pub default_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentPostgresConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub environment: DevelopmentEnvironment,
    pub safe_mode: DevelopmentSafeMode,
    pub read_only: bool,
    pub tls: DevelopmentTlsOptions,
    pub driver_options: DevelopmentDriverOptions,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentConnectionOrganization {
    pub folder: String,
    pub is_favorite: bool,
    pub color: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentConnection {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub organization: DevelopmentConnectionOrganization,
    /// Unsupported connections remain visible but cannot be rewritten through
    /// the PostgreSQL form. Organization changes preserve their other columns.
    pub unsupported_reason: Option<String>,
    pub postgres: Option<DevelopmentPostgresConnection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DevelopmentConnectionTest {
    Reachable {
        latency_ms: u64,
    },
    Failed {
        reason: DevelopmentConnectionFailure,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DevelopmentConnectionFailure {
    ConnectionLost,
    Timeout,
    Tls(crate::types::TlsFailureKind),
    Authentication,
    Database,
}

impl Backend {
    pub async fn development_connections(&self) -> Result<Vec<DevelopmentConnection>, String> {
        let authority = self.development()?;
        self.development_call(move |state| async move { Ok(list(&state, &authority).await) })
            .await
            .map_err(|_| "Native backend is closing".to_string())?
    }

    /// None creates an app-generated ID. Some edits an existing supported row.
    /// A blank password preserves its stored credential, including after unlock.
    pub async fn save_development_connection(
        &self,
        id: Option<String>,
        form: DevelopmentPostgresConnection,
        password: String,
    ) -> Result<DevelopmentConnection, String> {
        self.save_native_connection(id, form, password, None).await
    }

    /// Form submission commits organization, metadata and credentials together.
    pub async fn save_development_connection_with_organization(
        &self,
        id: Option<String>,
        form: DevelopmentPostgresConnection,
        password: String,
        organization: DevelopmentConnectionOrganization,
    ) -> Result<DevelopmentConnection, String> {
        validate_text(&organization.folder, 256)?;
        validate_text(&organization.color, 64)?;
        self.save_native_connection(id, form, password, Some(organization))
            .await
    }

    async fn save_native_connection(
        &self,
        id: Option<String>,
        form: DevelopmentPostgresConnection,
        password: String,
        organization: Option<DevelopmentConnectionOrganization>,
    ) -> Result<DevelopmentConnection, String> {
        let authority = self.development()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let connection_id = id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                crate::backend::pg_tools::require_connection_settled(&inner, Some(&connection_id))?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    Some(&connection_id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    Some(&connection_id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    Some(&connection_id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::backend::data::retire_data(&inner, &state, Some(&connection_id)).await?;
                let result =
                    crate::socket_lifecycle::with_connection_fence(&state, &connection_id, async {
                        let updated = native_connections::mutate(&state.credentials, |all| {
                            let previous = id
                                .as_ref()
                                .map(|id| supported(all, id, &authority))
                                .transpose()?;
                            let mut connection =
                                form.into_stored(connection_id.clone(), previous)?;
                            if let (Some(organization), StoredConnection::PostgreSQL(pg)) =
                                (organization, &mut connection)
                            {
                                pg.organization = crate::ConnectionOrganization {
                                    folder: organization.folder.trim().into(),
                                    is_favorite: organization.is_favorite,
                                    color: organization.color.trim().into(),
                                };
                            }
                            require_supported(&connection, &authority)?;
                            Ok(Change::Save {
                                connection,
                                password,
                                copy_from: None,
                            })
                        })
                        .await?;
                        crate::socket_lifecycle::invalidate_connection_caches(
                            &connection_id,
                            Some(crate::DatabaseEngine::PostgreSQL),
                        )
                        .await;
                        Ok::<_, String>(updated.expect("save returns metadata"))
                    })
                    .await?;
                Ok(summary(&result, &authority))
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn duplicate_development_connection(
        &self,
        id: String,
    ) -> Result<DevelopmentConnection, String> {
        let authority = self.development()?;
        self.development_call(move |state| async move {
            Ok(async {
                let result = native_connections::mutate(&state.credentials, |all| {
                    let source = supported(all, &id, &authority)?;
                    let connection = source.duplicated_as(
                        uuid::Uuid::new_v4().to_string(),
                        bounded_copy_name(source.name()),
                    );
                    require_supported(&connection, &authority)?;
                    Ok(Change::Save {
                        connection,
                        password: String::new(),
                        copy_from: Some(id),
                    })
                })
                .await?;
                Ok(summary(
                    &result.expect("duplicate returns metadata"),
                    &authority,
                ))
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn delete_development_connection(&self, id: String) -> Result<(), String> {
        let authority = self.development()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                crate::backend::pg_tools::require_connection_settled(&inner, Some(&id))?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    Some(&id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    Some(&id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    Some(&id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::backend::data::retire_data(&inner, &state, Some(&id)).await?;
                crate::socket_lifecycle::with_connection_fence(&state, &id, async {
                    native_connections::mutate(&state.credentials, |all| {
                        supported(all, &id, &authority)?;
                        Ok(Change::Delete { id: id.clone() })
                    })
                    .await?;
                    crate::socket_lifecycle::invalidate_connection_caches(&id, None).await;
                    Ok(())
                })
                .await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn organize_development_connection(
        &self,
        id: String,
        organization: DevelopmentConnectionOrganization,
    ) -> Result<Vec<DevelopmentConnection>, String> {
        let authority = self.development()?;
        self.development_call(move |state| async move {
            Ok(async {
                let _guard = credentials::mutation_guard(&state.credentials).await;
                native_connections::ensure_settled(&state.credentials).await?;
                validate_text(&organization.folder, 256)?;
                validate_text(&organization.color, 64)?;
                // Reuse the column-only storage operation without the legacy
                // service's permissive metadata reload/logging afterwards.
                let updated = storage::update_connection_organization(
                    &state.pool,
                    &id,
                    &crate::ConnectionOrganization {
                        folder: organization.folder.trim().into(),
                        is_favorite: organization.is_favorite,
                        color: organization.color.trim().into(),
                    },
                )
                .await
                .map_err(|_| "Connection organization could not be saved".to_string())?;
                if !updated {
                    return Err("Connection no longer exists; reload and retry".into());
                }
                list(&state, &authority).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    /// Explicit one-shot probe using the same owned driver as query sessions.
    /// Driver errors are classified without returning server text or secrets.
    pub async fn test_development_connection(
        &self,
        id: Option<String>,
        form: DevelopmentPostgresConnection,
        password: String,
    ) -> Result<DevelopmentConnectionTest, String> {
        let authority = self.development()?;
        let tasks = self.0.tasks.child();
        self.development_call(move |state| async move {
            Ok(async {
                // Serialize credential snapshots with edits/reset throughout this
                // bounded probe. It cannot outlive a later credential change.
                let _guard = credentials::mutation_guard(&state.credentials).await;
                let connection = prepare_probe(&state, &authority, id, form, password).await?;
                let spec =
                    crate::postgres::connect_spec::ResolvedPostgresConnectSpec::from_connection(
                        &connection,
                    )
                    .map_err(|_| "PostgreSQL connection required")?;
                let started = std::time::Instant::now();
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
                let result = tokio::time::timeout_at(
                    deadline,
                    crate::postgres::dedicated::connect_tracked(
                        &spec,
                        crate::postgres::dedicated::NoticeSink::Ignore,
                        Some(&tasks),
                    ),
                )
                .await;
                let reason = match result {
                    Ok(Ok(connected)) => {
                        if !settle_probe(&tasks, connected.close(), deadline).await {
                            return Ok(DevelopmentConnectionTest::Failed {
                                reason: DevelopmentConnectionFailure::Timeout,
                            });
                        }
                        return Ok(DevelopmentConnectionTest::Reachable {
                            latency_ms: started.elapsed().as_millis().min(u128::from(u64::MAX))
                                as u64,
                        });
                    }
                    Err(_) => DevelopmentConnectionFailure::Timeout,
                    Ok(Err(error)) => match error {
                        crate::postgres::dedicated::DedicatedError::ConnectionLost => {
                            DevelopmentConnectionFailure::ConnectionLost
                        }
                        crate::postgres::dedicated::DedicatedError::Timeout { .. } => {
                            DevelopmentConnectionFailure::Timeout
                        }
                        crate::postgres::dedicated::DedicatedError::Tls { kind, .. } => {
                            DevelopmentConnectionFailure::Tls(kind)
                        }
                        crate::postgres::dedicated::DedicatedError::Database { code, .. }
                            if code.as_deref().is_some_and(|code| code.starts_with("28")) =>
                        {
                            DevelopmentConnectionFailure::Authentication
                        }
                        crate::postgres::dedicated::DedicatedError::Database { .. } => {
                            DevelopmentConnectionFailure::Database
                        }
                    },
                };
                tasks.abort_all();
                tasks.drain().await;
                Ok(DevelopmentConnectionTest::Failed { reason })
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    pub async fn disconnect_development_connection(&self, id: String) -> Result<(), String> {
        self.development()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            // FIFO with bounded probes, including their local driver joins.
            let _guard = credentials::mutation_guard(&state.credentials).await;
            Ok(async {
                crate::backend::pg_tools::require_connection_settled(&inner, Some(&id))?;
                let _copy_retirement = crate::backend::table_copy::retire_connection(
                    &inner,
                    Some(&id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _seed_retirement = crate::backend::table_seed::retire_connection(
                    &inner,
                    Some(&id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                let _csv_retirement = crate::backend::csv_transfers::retire_connection(
                    &inner,
                    Some(&id),
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                )
                .await?;
                crate::backend::data::retire_data(&inner, &state, Some(&id)).await?;
                crate::connections::disconnect(&state, &id).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }
}

async fn list(
    state: &AppState,
    authority: &Authority,
) -> Result<Vec<DevelopmentConnection>, String> {
    let connections = storage::read_native_connections(&state.pool).await?;
    Ok(connections
        .iter()
        .map(|(connection, valid)| {
            let mut result = summary(connection, authority);
            if !valid {
                result.postgres = None;
                result.unsupported_reason =
                    Some("Stored connection options are unsupported or unreadable".into());
            }
            result
        })
        .collect())
}

fn require_supported(connection: &StoredConnection, authority: &Authority) -> Result<(), String> {
    if !authority.permits(connection) {
        return Err(format!(
            "{}; metadata preserved",
            unsupported_reason(authority)
        ));
    }
    Ok(())
}

fn unsupported_reason(authority: &Authority) -> &'static str {
    match authority.capability {
        EndpointCapability::OwnedFixtures(_) => {
            "Outside the owned PostgreSQL fixture manifest or supported field limits"
        }
        EndpointCapability::GeneralPostgres => {
            "Only supported direct PostgreSQL connections within field limits are available"
        }
    }
}

/// Caller holds the credential mutation guard and native lifecycle admission.
/// Unsaved test inputs never change the stored metadata or password.
pub(in crate::backend) async fn prepare_probe(
    state: &crate::app::AppState,
    authority: &Authority,
    id: Option<String>,
    form: DevelopmentPostgresConnection,
    password: String,
) -> Result<StoredConnection, String> {
    let rows = storage::read_native_connections(&state.pool).await?;
    if rows
        .iter()
        .any(|(connection, valid)| !valid && Some(connection.id()) == id.as_deref())
    {
        return Err(
            "Stored connection options are unsupported or unreadable; metadata preserved".into(),
        );
    }
    let all = rows
        .into_iter()
        .map(|(connection, _)| connection)
        .collect::<Vec<_>>();
    let previous = id
        .as_ref()
        .map(|id| supported(&all, id, authority))
        .transpose()?;
    let mut connection = form.into_stored(
        id.clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        previous,
    )?;
    require_supported(&connection, authority)?;
    // An unsaved edit must not send the saved password to a new
    // authentication boundary. Match the Tauri diagnosis contract
    // before reading credentials or opening a socket.
    if password.is_empty()
        && previous.is_some_and(|stored| !credentials::destination_matches(&connection, stored))
    {
        return Err("Enter the password again after changing the connection endpoint or transport security settings.".into());
    }
    if !credentials::onboarding_completed(&state.pool).await? {
        return Err("Configure credential storage before testing connections".into());
    }
    let mode = crate::app::current_credential_mode(state).await?;
    // read_all also checks interrupted native credential journals.
    let secrets = credentials::read_all(&state.credentials, mode).await?;
    connection.set_password(if password.is_empty() {
        id.as_ref()
            .and_then(|id| secrets.get(id))
            .cloned()
            .unwrap_or_default()
    } else {
        password
    });
    Ok(connection)
}

fn supported<'a>(
    all: &'a [StoredConnection],
    id: &str,
    authority: &Authority,
) -> Result<&'a StoredConnection, String> {
    let connection = all
        .iter()
        .find(|connection| connection.id() == id)
        .ok_or("Connection no longer exists; reload and retry")?;
    require_supported(connection, authority)?;
    Ok(connection)
}

fn summary(connection: &StoredConnection, authority: &Authority) -> DevelopmentConnection {
    let supported = authority.permits(connection);
    DevelopmentConnection {
        id: connection.id().into(),
        name: connection.name().into(),
        engine: connection.engine().as_str().into(),
        organization: DevelopmentConnectionOrganization {
            folder: connection.folder().into(),
            is_favorite: connection.is_favorite(),
            color: connection.color().into(),
        },
        unsupported_reason: (!supported).then(|| unsupported_reason(authority).into()),
        postgres: match connection {
            StoredConnection::PostgreSQL(pg) if supported => {
                Some(DevelopmentPostgresConnection::from_stored(pg))
            }
            _ => None,
        },
    }
}

/// Display names need not be unique; connection identity is the generated UUID.
/// Keep the suffix inside the same field limit enforced by save and admission.
fn bounded_copy_name(source: &str) -> String {
    const SUFFIX: &str = " copy";
    let mut end = source.len().min(256 - SUFFIX.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let mut name = String::with_capacity(end + SUFFIX.len());
    name.push_str(&source[..end]);
    name.push_str(SUFFIX);
    name
}

fn validate_text(value: &str, limit: usize) -> Result<(), String> {
    if value.len() > limit || value.contains('\0') {
        return Err("Connection field is too long or contains a NUL character".into());
    }
    Ok(())
}

impl DevelopmentPostgresConnection {
    fn from_stored(pg: &crate::PgStoredConnection) -> Self {
        let tls = pg
            .tls_options
            .clone()
            .unwrap_or_else(|| crate::PgTlsOptions {
                mode: pg.resolved_tls_mode(),
                ..Default::default()
            });
        let options = pg.driver_options.clone().unwrap_or_default();
        Self {
            name: pg.name.clone(),
            host: pg.host.clone(),
            port: pg.effective_port(),
            database: pg.database.clone(),
            user: pg.user.clone(),
            environment: match pg.environment {
                crate::Environment::Development => DevelopmentEnvironment::Development,
                crate::Environment::Test => DevelopmentEnvironment::Test,
                crate::Environment::Staging => DevelopmentEnvironment::Staging,
                crate::Environment::Production => DevelopmentEnvironment::Production,
            },
            safe_mode: match pg.safe_mode {
                crate::SafeMode::Inherit => DevelopmentSafeMode::Inherit,
                crate::SafeMode::Disabled => DevelopmentSafeMode::Disabled,
                crate::SafeMode::Protected => DevelopmentSafeMode::Protected,
                crate::SafeMode::Strict => DevelopmentSafeMode::Strict,
            },
            read_only: pg.read_only,
            tls: DevelopmentTlsOptions {
                mode: match tls.mode {
                    crate::PgTlsMode::Disable => DevelopmentTlsMode::Disable,
                    crate::PgTlsMode::Prefer => DevelopmentTlsMode::Prefer,
                    crate::PgTlsMode::Require => DevelopmentTlsMode::Require,
                    crate::PgTlsMode::VerifyCa => DevelopmentTlsMode::VerifyCa,
                    crate::PgTlsMode::VerifyFull => DevelopmentTlsMode::VerifyFull,
                },
                root_cert_path: tls.root_cert_path,
                client_cert_path: tls.client_cert_path,
                client_key_path: tls.client_key_path,
                server_name: tls.server_name,
            },
            driver_options: DevelopmentDriverOptions {
                statement_timeout_ms: options.statement_timeout_ms,
                idle_in_transaction_timeout_ms: options.idle_in_transaction_timeout_ms,
                connect_timeout_ms: options.connect_timeout_ms,
                keepalive_seconds: options.keepalive_seconds,
                default_search_path: options.default_search_path,
                default_role: options.default_role,
            },
        }
    }

    fn into_stored(
        self,
        id: String,
        previous: Option<&StoredConnection>,
    ) -> Result<StoredConnection, String> {
        for value in [&self.name, &self.host, &self.database, &self.user] {
            validate_text(value, 256)?;
            if value.trim().is_empty() {
                return Err("Connection name, host, database and user are required".into());
            }
        }
        if self.port == 0 {
            return Err("PostgreSQL port must be between 1 and 65535".into());
        }
        validate_driver_fields(
            self.driver_options.default_role.as_deref(),
            self.driver_options.default_search_path.as_deref(),
        )?;
        validate_tls_fields(
            [
                self.tls.root_cert_path.as_deref(),
                self.tls.client_cert_path.as_deref(),
                self.tls.client_key_path.as_deref(),
            ],
            self.tls.server_name.as_deref(),
        )?;
        let organization = previous
            .map(|value| crate::ConnectionOrganization {
                folder: value.folder().into(),
                is_favorite: value.is_favorite(),
                color: value.color().into(),
            })
            .unwrap_or_default();
        Ok(StoredConnection::PostgreSQL(crate::PgStoredConnection {
            id,
            name: self.name.trim().into(),
            host: self.host,
            port: self.port,
            database: self.database,
            user: self.user,
            password: String::new(),
            role: previous.map_or("read/write", StoredConnection::role).into(),
            last_activity_at: previous
                .and_then(StoredConnection::last_activity_at)
                .map(str::to_owned),
            organization,
            environment: match self.environment {
                DevelopmentEnvironment::Development => crate::Environment::Development,
                DevelopmentEnvironment::Test => crate::Environment::Test,
                DevelopmentEnvironment::Staging => crate::Environment::Staging,
                DevelopmentEnvironment::Production => crate::Environment::Production,
            },
            safe_mode: match self.safe_mode {
                DevelopmentSafeMode::Inherit => crate::SafeMode::Inherit,
                DevelopmentSafeMode::Disabled => crate::SafeMode::Disabled,
                DevelopmentSafeMode::Protected => crate::SafeMode::Protected,
                DevelopmentSafeMode::Strict => crate::SafeMode::Strict,
            },
            read_only: self.read_only,
            ssl: self.tls.mode != DevelopmentTlsMode::Disable,
            tls_options: Some(crate::PgTlsOptions {
                mode: match self.tls.mode {
                    DevelopmentTlsMode::Disable => crate::PgTlsMode::Disable,
                    DevelopmentTlsMode::Prefer => crate::PgTlsMode::Prefer,
                    DevelopmentTlsMode::Require => crate::PgTlsMode::Require,
                    DevelopmentTlsMode::VerifyCa => crate::PgTlsMode::VerifyCa,
                    DevelopmentTlsMode::VerifyFull => crate::PgTlsMode::VerifyFull,
                },
                root_cert_path: self.tls.root_cert_path,
                client_cert_path: self.tls.client_cert_path,
                client_key_path: self.tls.client_key_path,
                server_name: self.tls.server_name,
            }),
            driver_options: Some(crate::PgDriverOptions {
                statement_timeout_ms: self.driver_options.statement_timeout_ms,
                idle_in_transaction_timeout_ms: self.driver_options.idle_in_transaction_timeout_ms,
                connect_timeout_ms: self.driver_options.connect_timeout_ms,
                keepalive_seconds: self.driver_options.keepalive_seconds,
                default_search_path: self.driver_options.default_search_path,
                default_role: self.driver_options.default_role,
            }),
            ssh_tunnel: Default::default(),
        }))
    }
}

// Save and loaded admission share the same borrowed validation. No certificate
// files, DNS or credentials are touched while deciding metadata support.
pub(super) fn supported_fields(connection: &StoredConnection) -> bool {
    let StoredConnection::PostgreSQL(pg) = connection else {
        return false;
    };
    if !pg.ssh_tunnel.is_default() {
        return false;
    }
    if [&pg.name, &pg.host, &pg.database, &pg.user]
        .iter()
        .any(|value| value.trim().is_empty() || validate_text(value, 256).is_err())
    {
        return false;
    }
    if let Some(tls) = &pg.tls_options {
        if validate_tls_fields(
            [
                tls.root_cert_path.as_deref(),
                tls.client_cert_path.as_deref(),
                tls.client_key_path.as_deref(),
            ],
            tls.server_name.as_deref(),
        )
        .is_err()
        {
            return false;
        }
    }
    pg.driver_options.as_ref().is_none_or(|options| {
        validate_driver_fields(
            options.default_role.as_deref(),
            options.default_search_path.as_deref(),
        )
        .is_ok()
    })
}

fn validate_tls_fields(paths: [Option<&str>; 3], server_name: Option<&str>) -> Result<(), String> {
    for path in paths.into_iter().flatten() {
        validate_text(path, 4096)?;
    }
    if let Some(name) = server_name {
        validate_text(name, 256)?;
    }
    Ok(())
}

fn validate_driver_fields(
    role: Option<&str>,
    search_path: Option<&[String]>,
) -> Result<(), String> {
    if let Some(role) = role {
        validate_text(role, 256)?;
    }
    if let Some(path) = search_path {
        if path.len() > 64 {
            return Err("Search path has too many entries".into());
        }
        for part in path {
            validate_text(part, 256)?;
        }
    }
    Ok(())
}

/// Connect and graceful cleanup consume one absolute deadline. Expiry aborts
/// and joins the owned drivers before returning a truthful timeout result.
async fn settle_probe(
    tasks: &crate::postgres::dedicated::DriverJoins,
    close: impl std::future::Future<Output = ()>,
    deadline: tokio::time::Instant,
) -> bool {
    if tokio::time::timeout_at(deadline, async {
        close.await;
        tasks.drain().await;
    })
    .await
    .is_ok()
    {
        return true;
    }
    tasks.abort_all();
    tasks.drain().await;
    false
}

#[cfg(test)]
#[path = "connection_bounds_tests.rs"]
mod bounds_tests;
