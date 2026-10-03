//! C08.b/C08.c native Bastion Server facade. Metadata and profile-owned SSH
//! secrets commit together through the native credential journal. Secrets are
//! write-only inputs: responses carry presence flags, never values, and every
//! secret-bearing type redacts its `Debug` output.
//!
//! Host-key trust is explicit. A native route never trusts a first-seen key
//! (see `credentials::Context::requires_trusted_host_keys`); an explicit Test
//! reports the observed fingerprint, and only a reviewed trust or reset
//! changes the stored key. A changed key is refused until reviewed.
use super::development::DevelopmentConnectionFailure;
use super::{Backend, Inner, NativeProfileKind};
use crate::{
    app::AppState, credentials, credentials::native_connections, socket_lifecycle, storage, tunnel,
    BastionAuthMethod, BastionServer, SecretChange, StoredConnection,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

#[cfg(test)]
mod tests;

const MAX_TEXT: usize = 256;
const MAX_PATH: usize = 4096;
const MAX_SECRET: usize = 64 * 1024;
const MAX_FINGERPRINT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DevelopmentBastionAuth {
    Password,
    PrivateKeyPath,
    PrivateKeyContent,
}

impl DevelopmentBastionAuth {
    fn core(self) -> BastionAuthMethod {
        match self {
            Self::Password => BastionAuthMethod::Password,
            Self::PrivateKeyPath => BastionAuthMethod::PrivateKeyPath,
            Self::PrivateKeyContent => BastionAuthMethod::PrivateKeyContent,
        }
    }

    fn from_core(method: BastionAuthMethod) -> Self {
        match method {
            BastionAuthMethod::Password => Self::Password,
            BastionAuthMethod::PrivateKeyPath => Self::PrivateKeyPath,
            BastionAuthMethod::PrivateKeyContent => Self::PrivateKeyContent,
        }
    }
}

/// Non-secret Bastion Server fields edited by the native form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DevelopmentBastionForm {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_method: DevelopmentBastionAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
}

/// One write-only secret slot. A blank `Set` clears the slot.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum DevelopmentSecretInput {
    #[default]
    Keep,
    Set(String),
    Clear,
}

impl std::fmt::Debug for DevelopmentSecretInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Keep => "Keep",
            Self::Set(_) => "Set(<redacted>)",
            Self::Clear => "Clear",
        })
    }
}

impl DevelopmentSecretInput {
    fn core(&self) -> Result<SecretChange, String> {
        Ok(match self {
            Self::Keep => SecretChange::Keep,
            Self::Clear => SecretChange::Clear,
            Self::Set(value) if value.trim().is_empty() => SecretChange::Clear,
            Self::Set(value) => {
                if value.len() > MAX_SECRET || value.contains('\0') {
                    return Err("Bastion secret is too long or contains a NUL character".into());
                }
                SecretChange::Set {
                    value: value.clone(),
                }
            }
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DevelopmentBastionSecrets {
    pub password: DevelopmentSecretInput,
    pub private_key_content: DevelopmentSecretInput,
    pub passphrase: DevelopmentSecretInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentBastionReference {
    pub connection_id: String,
    pub connection_name: String,
}

/// Redacted Bastion Server summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentBastion {
    pub id: String,
    pub form: DevelopmentBastionForm,
    pub host_key_fingerprint: Option<String>,
    pub has_password: bool,
    pub has_private_key_content: bool,
    pub has_passphrase: bool,
    /// Connections whose route uses this bastion as a final or jump hop.
    pub references: Vec<DevelopmentBastionReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DevelopmentBastionDelete {
    Deleted,
    /// Nothing changed. Deleting leaves these connections with a route that
    /// fails closed until edited; resubmit with exactly these IDs to confirm.
    ReviewRequired {
        references: Vec<DevelopmentBastionReference>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DevelopmentHostKeyStatus {
    /// Observed key equals the trusted key.
    Trusted,
    /// No key is trusted yet; review `observed` before trusting it.
    Unknown,
    /// The host now presents a different key than the trusted one.
    Changed { trusted: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum DevelopmentBastionAuthentication {
    /// Credentials are only offered to a host with a trusted key.
    NotAttempted,
    Authenticated,
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentBastionTest {
    pub observed_fingerprint: String,
    pub host_key: DevelopmentHostKeyStatus,
    pub authentication: DevelopmentBastionAuthentication,
    pub latency_ms: u64,
}

impl Backend {
    /// Bastions reach arbitrary SSH hosts, so owned-fixture profiles (whose
    /// endpoints are pinned by manifest) do not offer them.
    fn bastion_profile(&self) -> Result<(), String> {
        let authority = self.development()?;
        if authority.kind() != NativeProfileKind::GeneralPostgres {
            return Err("Bastion Servers require a general PostgreSQL profile".into());
        }
        Ok(())
    }

    pub async fn development_bastions(&self) -> Result<Vec<DevelopmentBastion>, String> {
        self.bastion_profile()?;
        self.development_call(move |state| async move { Ok(list(&state).await) })
            .await
            .map_err(|_| "Native backend is closing".to_string())?
    }

    /// `None` creates an app-generated ID. Changing the endpoint drops its
    /// trusted host key, so the new endpoint must be tested and reviewed.
    pub async fn save_development_bastion(
        &self,
        id: Option<String>,
        form: DevelopmentBastionForm,
        secrets: DevelopmentBastionSecrets,
    ) -> Result<DevelopmentBastion, String> {
        self.bastion_profile()?;
        validate_form(&form)?;
        let patch_inputs = [
            secrets.password.core()?,
            secrets.private_key_content.core()?,
            secrets.passphrase.core()?,
        ];
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let bastion_id = id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let existing =
                    storage::bastions::read_bastion_server_by_id(&state.pool, &bastion_id).await?;
                if id.is_some() && existing.is_none() {
                    return Err("Bastion Server no longer exists; reload and retry".into());
                }
                let affected = referencing_ids(&state, &bastion_id).await?;
                let [password, private_key_content, passphrase] = patch_inputs;
                let now = storage::now();
                let host = form.host.trim().to_owned();
                let port = form.port;
                let bastion = BastionServer {
                    id: bastion_id.clone(),
                    name: form.name.trim().into(),
                    host: host.clone(),
                    port,
                    user: form.user.trim().into(),
                    auth_method: form.auth_method.core(),
                    private_key_path: form
                        .private_key_path
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                    host_key_fingerprint: existing
                        .as_ref()
                        .filter(|server| server.host == host && server.port == port)
                        .and_then(|server| server.host_key_fingerprint.clone()),
                    created_at: existing
                        .as_ref()
                        .map_or_else(|| now.clone(), |server| server.created_at.clone()),
                    updated_at: now,
                };
                let patch = credentials::BastionSecretPatch {
                    bastion_id: bastion_id.clone(),
                    auth_method: bastion.auth_method,
                    password,
                    private_key_content,
                    passphrase,
                };
                fenced(&inner, &state, &bastion_id, affected, async {
                    native_connections::mutate(&state.credentials, |_| {
                        Ok(native_connections::Change::SaveBastion { bastion, patch })
                    })
                    .await
                    .map(|_| ())
                })
                .await?;
                summary(&state, &bastion_id).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    /// Refuses while any connection references the bastion, unless `reviewed`
    /// names exactly the current referencing connection IDs.
    pub async fn delete_development_bastion(
        &self,
        id: String,
        reviewed: Vec<String>,
    ) -> Result<DevelopmentBastionDelete, String> {
        self.bastion_profile()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let affected = referencing_ids(&state, &id).await?;
                let mut review = None;
                let result = fenced(&inner, &state, &id, affected, async {
                    native_connections::mutate(&state.credentials, |all| {
                        // Authoritative check under the credential mutation
                        // guard that every connection save also holds.
                        let references = references_in(all, &id);
                        if !reviewed_matches(&references, &reviewed) {
                            review = Some(references);
                            return Err("Bastion Server deletion requires review".into());
                        }
                        Ok(native_connections::Change::DeleteBastion { id: id.clone() })
                    })
                    .await
                    .map(|_| ())
                })
                .await;
                match (result, review) {
                    (Err(_), Some(references)) => {
                        Ok(DevelopmentBastionDelete::ReviewRequired { references })
                    }
                    (Err(error), None) => Err(error),
                    (Ok(()), _) => Ok(DevelopmentBastionDelete::Deleted),
                }
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    /// Explicit Test: handshake directly with this bastion and compare its
    /// key. Authentication is attempted only for a trusted key. Nothing is
    /// persisted; the SSH worker is joined before this returns.
    pub async fn test_development_bastion(
        &self,
        id: String,
    ) -> Result<DevelopmentBastionTest, String> {
        self.bastion_profile()?;
        // Bounded network I/O runs under ordinary admission, not the native
        // lifecycle gate, so a slow SSH host cannot stall connection opens.
        self.call(move |state| async move {
            Ok(async {
                let mode = crate::app::current_credential_mode(&state).await?;
                let probe =
                    tunnel::probe_bastion(&state.credentials, &state.pool, mode, &id).await?;
                Ok(test_result(probe))
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    /// Trust a reviewed fingerprint. `expected` is the trusted key the user
    /// reviewed against (`None` when none was trusted); a concurrent change
    /// refuses the request. Replacing a changed key requires this explicit call.
    pub async fn trust_development_bastion_host_key(
        &self,
        id: String,
        expected: Option<String>,
        fingerprint: String,
    ) -> Result<DevelopmentBastion, String> {
        self.bastion_profile()?;
        validate_fingerprint(&fingerprint)?;
        self.set_host_key(id, expected, Some(fingerprint)).await
    }

    /// Forget the trusted key; routes through this bastion fail closed until a
    /// new key is tested and trusted.
    pub async fn reset_development_bastion_host_key(
        &self,
        id: String,
    ) -> Result<DevelopmentBastion, String> {
        self.bastion_profile()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let affected = referencing_ids(&state, &id).await?;
                fenced(&inner, &state, &id, affected, async {
                    let _guard = credentials::mutation_guard(&state.credentials).await;
                    storage::bastions::read_bastion_server_by_id(&state.pool, &id)
                        .await?
                        .ok_or("Bastion Server no longer exists; reload and retry")?;
                    storage::bastions::update_bastion_host_key_fingerprint(&state.pool, &id, None)
                        .await
                        .map_err(|_| "Host-key trust could not be reset".to_string())
                })
                .await?;
                summary(&state, &id).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }

    async fn set_host_key(
        &self,
        id: String,
        expected: Option<String>,
        fingerprint: Option<String>,
    ) -> Result<DevelopmentBastion, String> {
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let affected = referencing_ids(&state, &id).await?;
                fenced(&inner, &state, &id, affected, async {
                    let _guard = credentials::mutation_guard(&state.credentials).await;
                    let replaced = storage::bastions::replace_bastion_host_key_fingerprint(
                        &state.pool,
                        &id,
                        expected.as_deref(),
                        fingerprint.as_deref(),
                    )
                    .await
                    .map_err(|_| "Host-key trust could not be saved".to_string())?;
                    if !replaced {
                        return Err(
                            "Bastion host-key trust changed since review; test again".into()
                        );
                    }
                    Ok(())
                })
                .await?;
                summary(&state, &id).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }
}

/// Shared connect-path routing for one-shot native probes (Test, health).
/// Holding the returned guard keeps the loopback forward alive; dropping it
/// stops the listener and joins its worker. Direct connections pass through.
pub(in crate::backend) struct ProbeRoute(#[allow(dead_code)] Option<tunnel::EphemeralRoute>);

pub(in crate::backend) async fn route_probe(
    state: &AppState,
    connection: StoredConnection,
    deadline: tokio::time::Instant,
) -> Result<(ProbeRoute, StoredConnection), DevelopmentConnectionFailure> {
    if !connection.ssh_tunnel().is_some_and(|tunnel| tunnel.enabled) {
        return Ok((ProbeRoute(None), connection));
    }
    let mode = crate::app::current_credential_mode(state)
        .await
        .map_err(|_| DevelopmentConnectionFailure::SshTunnel)?;
    let route = tunnel::EphemeralRoute::new("native-probe");
    let resolved = tunnel::resolve_connection_checked(
        &state.credentials,
        &state.pool,
        mode,
        route.key(),
        &connection,
        Arc::new(move || {
            if tokio::time::Instant::now() >= deadline {
                Err("SSH route deadline expired".into())
            } else {
                Ok(())
            }
        }),
    )
    .await
    .map_err(|error| classify_route_error(&error, deadline))?;
    Ok((ProbeRoute(Some(route)), resolved))
}

pub(in crate::backend) fn classify_route_error(
    error: &str,
    deadline: tokio::time::Instant,
) -> DevelopmentConnectionFailure {
    if tunnel::is_host_key_error(error) {
        DevelopmentConnectionFailure::SshHostKey
    } else if tokio::time::Instant::now() >= deadline {
        DevelopmentConnectionFailure::Timeout
    } else {
        DevelopmentConnectionFailure::SshTunnel
    }
}

/// Retire and fence the connections routed through `bastion_id`, run `work`,
/// then drop their cached routes and the bastion's SSH sessions.
async fn fenced<T>(
    inner: &Arc<Inner>,
    state: &AppState,
    bastion_id: &str,
    affected: Vec<String>,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    if affected.is_empty() {
        let result = work.await;
        if result.is_ok() {
            tunnel::drop_bastion_async(bastion_id).await;
        }
        return result;
    }
    crate::backend::pg_tools::require_connection_settled(inner, None)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let _copy = crate::backend::table_copy::retire_connection(inner, None, deadline).await?;
    let _seed = crate::backend::table_seed::retire_connection(inner, None, deadline).await?;
    let _csv = crate::backend::csv_transfers::retire_connection(inner, None, deadline).await?;
    for connection in &affected {
        crate::backend::data::retire_data(inner, state, Some(connection)).await?;
    }
    socket_lifecycle::with_connection_ids_fence(state, &affected, async {
        let result = work.await;
        if result.is_ok() {
            socket_lifecycle::invalidate_bastion_caches(bastion_id, &affected).await;
        }
        result
    })
    .await
}

async fn referencing_ids(state: &AppState, bastion_id: &str) -> Result<Vec<String>, String> {
    storage::bastions::connection_ids_referencing_bastion(&state.pool, bastion_id)
        .await
        .map_err(|_| "Connection routes could not be loaded".to_string())
}

fn references_in(all: &[StoredConnection], bastion_id: &str) -> Vec<DevelopmentBastionReference> {
    let mut references = all
        .iter()
        .filter(|connection| {
            connection
                .ssh_tunnel()
                .is_some_and(|tunnel| tunnel.references_bastion(bastion_id))
        })
        .map(|connection| DevelopmentBastionReference {
            connection_id: connection.id().into(),
            connection_name: connection.name().into(),
        })
        .collect::<Vec<_>>();
    references.sort_by(|a, b| a.connection_id.cmp(&b.connection_id));
    references
}

fn reviewed_matches(references: &[DevelopmentBastionReference], reviewed: &[String]) -> bool {
    let mut reviewed = reviewed.to_vec();
    reviewed.sort();
    reviewed.dedup();
    references.len() == reviewed.len()
        && references
            .iter()
            .zip(&reviewed)
            .all(|(reference, id)| &reference.connection_id == id)
}

async fn list(state: &AppState) -> Result<Vec<DevelopmentBastion>, String> {
    let servers = storage::bastions::read_bastion_servers(&state.pool)
        .await
        .map_err(|_| "Bastion Servers could not be loaded".to_string())?;
    let connections = storage::read_native_connections(&state.pool)
        .await?
        .into_iter()
        .map(|(connection, _)| connection)
        .collect::<Vec<_>>();
    // Presence flags only; locked or unconfigured storage reports none.
    let secrets = match crate::app::current_credential_mode(state).await {
        Ok(mode) => credentials::read_all(&state.credentials, mode)
            .await
            .unwrap_or_default(),
        Err(_) => HashMap::new(),
    };
    Ok(servers
        .into_iter()
        .map(|server| public(server, &secrets, &connections))
        .collect())
}

async fn summary(state: &AppState, id: &str) -> Result<DevelopmentBastion, String> {
    list(state)
        .await?
        .into_iter()
        .find(|bastion| bastion.id == id)
        .ok_or_else(|| "Bastion Server no longer exists; reload and retry".into())
}

fn public(
    server: BastionServer,
    secrets: &HashMap<String, String>,
    connections: &[StoredConnection],
) -> DevelopmentBastion {
    let has = |slot| credentials::bastion_secret_present(secrets, &server.id, slot);
    DevelopmentBastion {
        has_password: has("password"),
        has_private_key_content: has("privateKeyContent"),
        has_passphrase: has("passphrase"),
        references: references_in(connections, &server.id),
        host_key_fingerprint: server.host_key_fingerprint,
        form: DevelopmentBastionForm {
            name: server.name,
            host: server.host,
            port: server.port,
            user: server.user,
            auth_method: DevelopmentBastionAuth::from_core(server.auth_method),
            private_key_path: server.private_key_path,
        },
        id: server.id,
    }
}

fn test_result(probe: tunnel::BastionProbe) -> DevelopmentBastionTest {
    let host_key = match &probe.trusted {
        Some(trusted) if *trusted == probe.observed => DevelopmentHostKeyStatus::Trusted,
        Some(trusted) => DevelopmentHostKeyStatus::Changed {
            trusted: trusted.clone(),
        },
        None => DevelopmentHostKeyStatus::Unknown,
    };
    let authentication = match probe.authentication {
        None => DevelopmentBastionAuthentication::NotAttempted,
        Some(Ok(())) => DevelopmentBastionAuthentication::Authenticated,
        Some(Err(message)) => DevelopmentBastionAuthentication::Failed { message },
    };
    DevelopmentBastionTest {
        observed_fingerprint: probe.observed,
        host_key,
        authentication,
        latency_ms: probe.latency_ms,
    }
}

fn text(value: &str, limit: usize, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("Bastion {label} is required"));
    }
    if value.len() > limit || value.contains('\0') {
        return Err(format!(
            "Bastion {label} is too long or contains a NUL character"
        ));
    }
    Ok(())
}

fn validate_form(form: &DevelopmentBastionForm) -> Result<(), String> {
    text(&form.name, MAX_TEXT, "name")?;
    text(&form.host, MAX_TEXT, "host")?;
    text(&form.user, MAX_TEXT, "user")?;
    if form.host.trim().chars().any(char::is_whitespace) {
        return Err("Bastion host cannot contain whitespace".into());
    }
    if form.port == 0 {
        return Err("Bastion port must be between 1 and 65535".into());
    }
    match (&form.private_key_path, form.auth_method) {
        (Some(path), _) if !path.trim().is_empty() => text(path, MAX_PATH, "private key path"),
        (_, DevelopmentBastionAuth::PrivateKeyPath) => Err("Private key path is required".into()),
        _ => Ok(()),
    }
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), String> {
    let Some(body) = fingerprint.strip_prefix("SHA256:") else {
        return Err("Host-key fingerprint must be a SHA256 fingerprint".into());
    };
    if body.is_empty()
        || fingerprint.len() > MAX_FINGERPRINT
        || !body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
    {
        return Err("Host-key fingerprint must be a SHA256 fingerprint".into());
    }
    Ok(())
}
