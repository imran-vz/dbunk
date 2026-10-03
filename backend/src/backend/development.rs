//! Stage04 development profiles. Filesystem and SQLite identity checks precede
//! construction of any credential context. The stage03 guard remains separate.

#[cfg(test)]
mod admission_tests;
#[cfg(test)]
mod connection_tests;
pub(super) mod connections;
pub(super) mod files;
#[cfg(test)]
mod probe_credentials_tests;
mod settings;
mod workspace;
pub use connections::{
    DevelopmentClickHouseConnection, DevelopmentConnection, DevelopmentConnectionFailure,
    DevelopmentConnectionOrganization, DevelopmentConnectionTest, DevelopmentDriverOptions,
    DevelopmentEndpoint, DevelopmentEngineConnection, DevelopmentEnvironment,
    DevelopmentMySqlConnection, DevelopmentPostgresConnection, DevelopmentRedisConnection,
    DevelopmentSafeMode, DevelopmentSqliteConnection, DevelopmentSshTunnel, DevelopmentTlsMode,
    DevelopmentTlsOptions,
};
pub use settings::{DevelopmentCredentialState, DevelopmentSettings, DevelopmentStorageMode};
pub(super) use workspace::encode_record as encode_workspace_record;
pub use workspace::{
    WorkspaceAdminAction, WorkspaceAdminControl, WorkspaceApplyState, WorkspaceDensity,
    WorkspaceDocument, WorkspaceError, WorkspaceLoad, WorkspaceMaintenance,
    WorkspaceMaintenanceAction, WorkspaceMaintenanceKind, WorkspaceMaintenanceState,
    WorkspaceMutationDraft, WorkspaceQueryChanges, WorkspaceRevision, WorkspaceSchemaAlter,
    WorkspaceSchemaChanges, WorkspaceSelection, WorkspaceSnapshot, WorkspaceStagedChange,
    WorkspaceTableCopy, WorkspaceTableCopyState, WorkspaceTableDdl, WorkspaceTableSeed,
    WorkspaceTableSeedState, WorkspaceTableState, WorkspaceTool, NATIVE_WORKSPACE_MAX_BYTES,
    NATIVE_WORKSPACE_MAX_DOCUMENTS, WORKSPACE_COPY_MAX_JOBS, WORKSPACE_MUTATION_MAX_BYTES,
    WORKSPACE_MUTATION_MAX_CHANGES, WORKSPACE_SCHEMA_ALTER_MAX_BYTES, WORKSPACE_SEED_MAX_JOBS,
    WORKSPACE_TABLE_DDL_MAX_BYTES,
};
pub use workspace::{WorkspaceObjectDdl, WORKSPACE_OBJECT_DDL_MAX_BYTES};
#[cfg(test)]
mod tests;

use super::Backend;
use crate::postgres::schema_compare::manager::CompareManager;
#[cfg(test)]
use crate::CredentialStorageMode;
use crate::{app::AppState, credentials, storage, StoredConnection};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, SqliteConnection};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MARKER: &str = ".dbunk-native-stage04";
const IDENTITY_KEY: &str = "native.stage04.identity";
static PROCESS_PROFILE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Ownership must be verified by the launcher immediately before opening a
/// profile. TLS testing is a separate explicit fixture; reopening cannot widen
/// the endpoints recorded in a profile's ownership marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DevelopmentFixtures {
    version: u32,
    fixture: String,
    instance: String,
    host: String,
    port: u16,
    database: String,
    user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls: Option<DevelopmentTlsFixture>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DevelopmentTlsFixture {
    fixture: String,
    instance: String,
    host: String,
    port: u16,
    database: String,
    user: String,
}

impl DevelopmentFixtures {
    pub fn from_json(json: &str) -> Result<Self, String> {
        if json.len() > 4096 {
            return Err("Fixture manifest is too large".into());
        }
        let value: Self = serde_json::from_str(json).map_err(|_| "Invalid fixture manifest")?;
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.fixture != "dbunk-native-stage03"
            || self.host != "127.0.0.1"
            || self.port != 15432
            || self.database != "dbunk_demo"
            || self.user != "dbunk"
        {
            return Err("Manifest does not identify an allowed native fixture".into());
        }
        canonical_uuid(&self.instance)?;
        if let Some(tls) = &self.tls {
            if tls.fixture != "dbunk-native-stage04-tls"
                || tls.host != "127.0.0.1"
                || tls.port != 15433
                || tls.database != "dbunk_tls_demo"
                || tls.user != "dbunk"
            {
                return Err("Manifest does not identify an allowed native TLS fixture".into());
            }
            canonical_uuid(&tls.instance)?;
        }
        Ok(())
    }

    pub(super) fn permits(&self, connection: &StoredConnection) -> bool {
        let StoredConnection::PostgreSQL(pg) = connection else {
            return false;
        };
        if !pg.ssh_tunnel.is_default() {
            return false;
        }
        if self.tls.as_ref().is_some_and(|tls| {
            pg.host == tls.host
                && pg.effective_port() == tls.port
                && pg.database == tls.database
                && pg.user == tls.user
        }) {
            // Certificate and server-name inputs are intentionally validated by
            // the TLS driver. The TCP address remains the allowlisted host.
            return pg.resolved_tls_mode() != crate::PgTlsMode::Disable;
        }
        pg.host == self.host
            && pg.effective_port() == self.port
            && pg.database == self.database
            && pg.user == self.user
            && pg.resolved_tls_mode() == crate::PgTlsMode::Disable
            && pg.tls_options.as_ref().is_none_or(|tls| {
                tls.server_name.as_deref().is_none_or(str::is_empty)
                    && tls.root_cert_path.as_deref().is_none_or(str::is_empty)
                    && tls.client_cert_path.as_deref().is_none_or(str::is_empty)
                    && tls.client_key_path.as_deref().is_none_or(str::is_empty)
            })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    profile_id: String,
    credential_namespace: String,
    path: PathBuf,
    fixtures: DevelopmentFixtures,
}

pub(super) enum EndpointCapability {
    OwnedFixtures(Box<DevelopmentFixtures>),
    /// General profiles (marker kind `general-postgres`, kept for on-disk
    /// compatibility) admit every engine within supported field limits.
    GeneralPostgres,
}

pub(super) struct Authority {
    pub(super) capability: EndpointCapability,
    pub(super) profile_id: String,
}

impl Authority {
    pub(super) fn kind(&self) -> super::NativeProfileKind {
        match self.capability {
            EndpointCapability::OwnedFixtures(_) => super::NativeProfileKind::OwnedFixtures,
            EndpointCapability::GeneralPostgres => super::NativeProfileKind::GeneralPostgres,
        }
    }

    pub(super) fn permits(&self, connection: &StoredConnection) -> bool {
        if !connections::supported_fields(connection) {
            return false;
        }
        match &self.capability {
            EndpointCapability::OwnedFixtures(fixtures) => fixtures.permits(connection),
            EndpointCapability::GeneralPostgres => true,
        }
    }
}

impl Backend {
    /// Create a NEW private profile with app-generated identities. Failures leave
    /// the new directory for inspection; no existing directory is adopted.
    pub async fn create_development(
        path: &Path,
        fixtures: DevelopmentFixtures,
    ) -> Result<Self, String> {
        check_runtime()?;
        fixtures.validate()?;
        ensure_process_profile(path)?;
        files::create_directory(path)?;
        let marker = Marker {
            version: 1,
            profile_id: uuid::Uuid::new_v4().to_string(),
            credential_namespace: uuid::Uuid::new_v4().to_string(),
            path: path.to_owned(),
            fixtures,
        };
        let encoded =
            serde_json::to_vec(&marker).map_err(|_| "Invalid development profile path")?;
        let lock = files::lock(path)?;
        files::write_new(&path.join(MARKER), &encoded)?;
        files::write_new(&path.join("dbunk.sqlite"), &[])?;
        let paths = storage::Paths::from_dir(path.to_owned());
        let pool = storage::open_native_profile_pool(&paths).await?;
        let initialized =
            storage::set_setting(&pool, IDENTITY_KEY, std::str::from_utf8(&encoded).unwrap()).await;
        pool.close().await;
        initialized?;
        files::sync_directory(path)?;
        drop(lock);
        Self::open_development(path, &marker.fixtures).await
    }

    /// The manifest is supplied again on every launch after the fixture owner
    /// checks its live instance. A stale or changed fixture identity is refused.
    pub async fn open_development(
        path: &Path,
        fixtures: &DevelopmentFixtures,
    ) -> Result<Self, String> {
        check_runtime()?;
        fixtures.validate()?;
        let (marker, lock) = files::validate(path)?;
        if &marker.fixtures != fixtures {
            return Err("Development profile fixture identity changed".into());
        }
        verify_database(path, &marker).await?;
        claim_process(path)?;
        let paths = storage::Paths::from_dir(path.to_owned());
        let pool = storage::open_native_profile_pool(&paths).await?;
        let result = async {
            let context = credentials::Context::development(
                pool.clone(),
                canonical_uuid(&marker.credential_namespace)?,
            );
            let state = AppState::with_credentials(paths, CompareManager::new(), context);
            Ok(Backend::from_state(
                state,
                lock,
                Some(Arc::new(Authority {
                    capability: EndpointCapability::OwnedFixtures(Box::new(marker.fixtures)),
                    profile_id: marker.profile_id,
                })),
            ))
        }
        .await;
        if result.is_err() {
            pool.close().await;
        }
        result
    }
}

pub(super) fn check_runtime() -> Result<(), String> {
    if tokio::runtime::Handle::current().runtime_flavor()
        != tokio::runtime::RuntimeFlavor::MultiThread
    {
        return Err("Native backend requires a multi-thread Tokio runtime".into());
    }
    Ok(())
}

pub(super) fn ensure_process_profile(path: &Path) -> Result<(), String> {
    if PROCESS_PROFILE
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|selected| selected != path)
    {
        return Err("A native process cannot switch native profiles; restart the process".into());
    }
    Ok(())
}

pub(super) fn claim_process(path: &Path) -> Result<(), String> {
    let mut selected = PROCESS_PROFILE.lock().unwrap();
    if selected.as_deref().is_some_and(|selected| selected != path) {
        return Err("A native process cannot switch native profiles; restart the process".into());
    }
    *selected = Some(path.to_owned());
    Ok(())
}

pub(super) fn canonical_uuid(value: &str) -> Result<uuid::Uuid, String> {
    let uuid = uuid::Uuid::parse_str(value).map_err(|_| "Invalid development profile identity")?;
    if uuid.get_version_num() != 4 || uuid.to_string() != value {
        return Err("Invalid development profile identity".into());
    }
    Ok(uuid)
}

async fn verify_database(path: &Path, marker: &Marker) -> Result<(), String> {
    // Never migrate an existing database before proving it belongs to this
    // marker, path, namespace and fixture. Read-only inspection has no secrets.
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path.join("dbunk.sqlite"))
        .read_only(true)
        .create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| "Development profile database is unavailable")?;
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM app_settings WHERE key = ?")
        .bind(IDENTITY_KEY)
        .fetch_optional(&mut connection)
        .await;
    connection
        .close()
        .await
        .map_err(|_| "Development profile database could not close")?;
    let encoded =
        serde_json::to_string(marker).map_err(|_| "Invalid development profile identity")?;
    if value.ok().flatten().as_deref() != Some(&encoded) {
        return Err(
            "Development database does not match its ownership marker; profile preserved".into(),
        );
    }
    Ok(())
}
