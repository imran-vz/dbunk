//! C09.a-c native Managed Servers: Docker-provisioned PostgreSQL owned by one
//! general profile. Containers and volumes carry the profile and record
//! identity as labels; every lifecycle call resolves and re-verifies those
//! labels on the exact container ID and never acts on a name alone.
//!
//! Provisioning and destruction are two-step: a reviewed plan is resubmitted
//! and recomputed, so the operation does exactly what the user saw. Each
//! Docker call is bounded and nothing is retried automatically.
use super::development::Authority;
use super::{Backend, NativeProfileKind};
use crate::app::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod runner;
mod service;
#[cfg(test)]
mod tests;

pub use runner::DockerAvailability;

/// Image major versions offered, newest first. Also the tag allowlist.
pub const MANAGED_POSTGRES_VERSIONS: &[&str] = &["18", "17", "16", "15", "14"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedProvisionRequest {
    pub name: String,
    pub version: String,
    /// `None` picks a free loopback port from 5433.
    pub port: Option<u16>,
}

/// Everything provisioning creates, shown for review before it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedProvisionPlan {
    pub record_id: String,
    pub name: String,
    pub image: String,
    pub version: String,
    /// Always loopback-only.
    pub host_binding: String,
    pub port: u16,
    pub container_name: String,
    pub volume_name: String,
    pub database: String,
    pub user: String,
    pub connection_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedProvisioned {
    pub record_id: String,
    pub connection_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ManagedContainerStatus {
    Running,
    Stopped {
        state: String,
    },
    /// No container carries this record's labels.
    Missing,
    /// More than one container carries this record's labels.
    Conflict,
    /// Docker could not be observed.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedConnectionRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedServerSummary {
    pub id: String,
    pub name: String,
    pub image: String,
    pub version: String,
    pub port: u16,
    pub container_name: String,
    pub volume_name: String,
    pub database: String,
    pub user: String,
    pub connection: Option<ManagedConnectionRef>,
    pub status: ManagedContainerStatus,
    /// Full ID of the single owned container, when one exists.
    pub container_id: Option<String>,
    /// `None` when Docker could not be observed.
    pub volume_present: Option<bool>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedServerList {
    pub docker: DockerAvailability,
    /// Set when Docker is available but its listing failed.
    pub observe_error: Option<String>,
    pub servers: Vec<ManagedServerSummary>,
}

/// Exactly what Destroy deletes. The record itself is always forgotten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedDestroyPlan {
    pub record_id: String,
    pub name: String,
    pub container_id: Option<String>,
    pub volume: Option<String>,
    pub connection: Option<ManagedConnectionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ManagedDestroy {
    Destroyed,
    /// Nothing changed; review this current plan and resubmit it.
    ReviewRequired {
        plan: ManagedDestroyPlan,
    },
}

const CLOSING: &str = "Native backend is closing";

impl Backend {
    /// Managed Servers run arbitrary local containers, so owned-fixture
    /// profiles (pinned to their manifest endpoints) never offer them.
    fn managed_profile(&self) -> Result<Arc<Authority>, String> {
        let authority = self.development()?;
        require_general(authority.kind())?;
        Ok(authority)
    }

    pub async fn managed_docker_status(&self) -> Result<DockerAvailability, String> {
        self.managed_profile()?;
        self.call(move |_| async move { Ok(service::availability(&runner::CliHost).await) })
            .await
            .map_err(|_| CLOSING.to_string())
    }

    /// Records saved in this profile, with status observed live from Docker
    /// containers labelled for this profile only.
    pub async fn managed_servers(&self) -> Result<ManagedServerList, String> {
        let authority = self.managed_profile()?;
        self.call(move |state| async move {
            Ok(service::list(&scope(&state, &authority), &runner::CliHost).await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Validates the request and picks names, credentials identity and port.
    /// Nothing is created.
    pub async fn plan_managed_server(
        &self,
        request: ManagedProvisionRequest,
    ) -> Result<ManagedProvisionPlan, String> {
        let authority = self.managed_profile()?;
        self.call(move |state| async move {
            Ok(service::plan(&scope(&state, &authority), &runner::CliHost, &request).await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Creates a NEW labelled volume and container for exactly `plan`, waits
    /// for PostgreSQL, then saves the record and a connection whose generated
    /// password goes through the native credential journal. Any failure
    /// removes what this attempt created.
    pub async fn provision_managed_server(
        &self,
        plan: ManagedProvisionPlan,
    ) -> Result<ManagedProvisioned, String> {
        let authority = self.managed_profile()?;
        let inner = self.0.clone();
        // Image pulls may take minutes; only persistence takes the lifecycle gate.
        self.call(move |state| async move {
            let gate = async {
                let guard = inner.development_gate.lock().await;
                if inner.closing.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(CLOSING.to_string());
                }
                Ok(guard)
            };
            Ok(service::provision(&scope(&state, &authority), &runner::CliHost, &plan, gate).await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Starts the exact owned container and waits for PostgreSQL; cached
    /// routes for the linked connection are dropped afterwards.
    pub async fn start_managed_server(&self, id: String) -> Result<(), String> {
        let authority = self.managed_profile()?;
        self.call(move |state| async move {
            Ok(async {
                let scope = scope(&state, &authority);
                let connection = service::linked_connection_id(&scope, &id).await?;
                let result = service::start(&scope, &runner::CliHost, &id).await;
                if let Some(connection) = connection {
                    crate::socket_lifecycle::invalidate_connection_caches(
                        &connection,
                        Some(crate::DatabaseEngine::PostgreSQL),
                    )
                    .await;
                }
                result
            }
            .await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Connect intent: starts the stopped managed container linked to
    /// `connection_id` and waits for PostgreSQL. `Ok(false)` means nothing
    /// was started (no managed record, already running, or a profile kind
    /// that never offers managed servers). Query-session opens run the same
    /// check before connecting, so hosts need not call this first.
    pub async fn ensure_managed_running(&self, connection_id: String) -> Result<bool, String> {
        let authority = self.development()?;
        self.call(move |state| async move {
            Ok(ensure_running_for_connection(&state, &authority, &connection_id).await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Settles the linked connection (open work retired, sessions closed),
    /// then stops the exact owned container.
    pub async fn stop_managed_server(&self, id: String) -> Result<(), String> {
        let authority = self.managed_profile()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let scope = scope(&state, &authority);
                let connection = service::linked_connection_id(&scope, &id).await?;
                settled(&inner, &state, connection, async {
                    service::stop(&scope, &runner::CliHost, &id).await
                })
                .await
            }
            .await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Current destructive plan for review. Nothing changes.
    pub async fn review_managed_server_destroy(
        &self,
        id: String,
    ) -> Result<ManagedDestroyPlan, String> {
        let authority = self.managed_profile()?;
        self.call(move |state| async move {
            Ok(service::destroy_plan(&scope(&state, &authority), &runner::CliHost, &id).await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Removes the reviewed container, volume and connection, then forgets
    /// the record. A changed plan returns `ReviewRequired` with no change.
    pub async fn destroy_managed_server(
        &self,
        id: String,
        reviewed: ManagedDestroyPlan,
    ) -> Result<ManagedDestroy, String> {
        let authority = self.managed_profile()?;
        let inner = self.0.clone();
        self.development_call(move |state| async move {
            Ok(async {
                let scope = scope(&state, &authority);
                let connection = reviewed.connection.as_ref().map(|c| c.id.clone());
                settled(&inner, &state, connection, async {
                    service::destroy(&scope, &runner::CliHost, &id, &reviewed).await
                })
                .await
            }
            .await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }

    /// Recreates a missing container for the same record, port, connection
    /// and password. A failed attempt removes only what it created.
    pub async fn recreate_managed_server(&self, id: String) -> Result<(), String> {
        let authority = self.managed_profile()?;
        self.call(move |state| async move {
            Ok(async {
                let scope = scope(&state, &authority);
                let connection = service::linked_connection_id(&scope, &id).await?;
                let result = service::recreate(&scope, &runner::CliHost, &id).await;
                if let Some(connection) = connection {
                    crate::socket_lifecycle::invalidate_connection_caches(
                        &connection,
                        Some(crate::DatabaseEngine::PostgreSQL),
                    )
                    .await;
                }
                result
            }
            .await)
        })
        .await
        .map_err(|_| CLOSING.to_string())?
    }
}

fn require_general(kind: NativeProfileKind) -> Result<(), String> {
    if kind != NativeProfileKind::GeneralPostgres {
        return Err("Managed servers require a general PostgreSQL profile".into());
    }
    Ok(())
}

/// Shared by [`Backend::ensure_managed_running`] and the PostgreSQL open
/// path. Runs outside the lifecycle gate: Docker and readiness waits are
/// bounded by the service's own deadlines and must not stall other opens.
pub(super) async fn ensure_running_for_connection(
    state: &AppState,
    authority: &Authority,
    connection_id: &str,
) -> Result<bool, String> {
    if authority.kind() != NativeProfileKind::GeneralPostgres {
        return Ok(false);
    }
    service::ensure_running_for_connection(
        &scope(state, authority),
        &runner::CliHost,
        connection_id,
    )
    .await
}

fn scope<'a>(state: &'a AppState, authority: &'a Authority) -> service::Scope<'a> {
    service::Scope {
        profile_id: &authority.profile_id,
        pool: &state.pool,
        credentials: &state.credentials,
        authority,
    }
}

/// Retire work on `connection` and fence it while `work` runs, so a stopped
/// or destroyed server never leaves a live session behind.
async fn settled<T>(
    inner: &Arc<super::Inner>,
    state: &AppState,
    connection: Option<String>,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let Some(connection) = connection else {
        return work.await;
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    crate::backend::pg_tools::require_connection_settled(inner, Some(&connection))?;
    let _copy =
        crate::backend::table_copy::retire_connection(inner, Some(&connection), deadline).await?;
    let _seed =
        crate::backend::table_seed::retire_connection(inner, Some(&connection), deadline).await?;
    let _csv = crate::backend::csv_transfers::retire_connection(inner, Some(&connection), deadline)
        .await?;
    crate::backend::data::retire_data(inner, state, Some(&connection)).await?;
    crate::socket_lifecycle::with_connection_fence(state, &connection, async {
        let result = work.await;
        crate::socket_lifecycle::invalidate_connection_caches(&connection, None).await;
        result
    })
    .await
}
