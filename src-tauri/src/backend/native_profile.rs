//! Explicit general PostgreSQL profiles. A distinct marker and database identity
//! grant endpoint capability; fixture profiles are never adopted or converted.

use super::development::{self, files, Authority, EndpointCapability};
use super::Backend;
use crate::postgres::schema_compare::manager::CompareManager;
use crate::{app::AppState, credentials, storage};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, SqliteConnection};
use std::{path::Path, path::PathBuf, sync::Arc};

const MARKER: &str = ".dbunk-native-profile";
const IDENTITY_KEY: &str = "native.profile.identity.v1";
const MAX_MARKER_BYTES: usize = 8192;

/// Validated workspace authority. Stage03's immutable single-fixture host has
/// no workspace authority and returns None from `native_profile_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProfileKind {
    OwnedFixtures,
    GeneralPostgres,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum MarkerKind {
    GeneralPostgres,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    kind: MarkerKind,
    profile_id: String,
    credential_namespace: String,
    path: PathBuf,
}

impl Marker {
    fn validate(&self, path: &Path) -> Result<(), String> {
        if self.version != 1 || self.path != path {
            return Err("Native profile marker version or path does not match".into());
        }
        development::canonical_uuid(&self.profile_id)?;
        development::canonical_uuid(&self.credential_namespace)?;
        if self.profile_id == self.credential_namespace {
            return Err("Native credential namespace must be independently generated".into());
        }
        Ok(())
    }

    fn encoded(&self) -> Result<String, String> {
        let encoded = serde_json::to_string(self).map_err(|_| "Invalid native profile identity")?;
        if encoded.len() > MAX_MARKER_BYTES {
            return Err("Native profile marker is too large".into());
        }
        Ok(encoded)
    }
}

impl Backend {
    pub fn native_profile_kind(&self) -> Option<NativeProfileKind> {
        self.0
            .development
            .as_ref()
            .map(|authority| authority.kind())
    }

    /// Creates only a NEW private directory. No endpoint or OS credential store
    /// is opened. Failed initialization preserves the directory for inspection.
    pub async fn create_native_profile(path: &Path) -> Result<Self, String> {
        development::check_runtime()?;
        development::ensure_process_profile(path)?;
        let marker = Marker {
            version: 1,
            kind: MarkerKind::GeneralPostgres,
            profile_id: uuid::Uuid::new_v4().to_string(),
            credential_namespace: uuid::Uuid::new_v4().to_string(),
            path: path.to_owned(),
        };
        let encoded = marker.encoded()?;
        files::create_directory(path)?;
        let lock = files::lock(path)?;
        files::write_new(&path.join(MARKER), encoded.as_bytes())?;
        files::write_new(&path.join("dbunk.sqlite"), &[])?;
        let paths = storage::Paths::from_dir(path.to_owned());
        let pool = storage::open_native_profile_pool(&paths).await?;
        let initialized = storage::set_setting(&pool, IDENTITY_KEY, &encoded).await;
        pool.close().await;
        initialized?;
        files::sync_directory(path)?;
        drop(lock);
        Self::open_native_profile(path).await
    }

    /// Opens only a matching general-profile marker and SQLite identity, before
    /// migration or credential construction. No fallback or legacy import occurs.
    pub async fn open_native_profile(path: &Path) -> Result<Self, String> {
        development::check_runtime()?;
        let lock = files::validate_files(path, MARKER)?;
        let marker: Marker = serde_json::from_slice(&files::read_marker(path, MARKER)?)
            .map_err(|_| "Invalid native profile marker")?;
        marker.validate(path)?;
        verify_database(path, &marker.encoded()?).await?;
        development::claim_process(path)?;
        let paths = storage::Paths::from_dir(path.to_owned());
        let pool = storage::open_native_profile_pool(&paths).await?;
        let context = credentials::Context::development(
            pool,
            development::canonical_uuid(&marker.credential_namespace)?,
        );
        let state = AppState::with_credentials(paths, CompareManager::new(), context);
        Ok(Self::from_state(
            state,
            lock,
            Some(Arc::new(Authority {
                capability: EndpointCapability::GeneralPostgres,
                profile_id: marker.profile_id,
            })),
        ))
    }
}

async fn verify_database(path: &Path, encoded: &str) -> Result<(), String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path.join("dbunk.sqlite"))
        .read_only(true)
        .create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| "Native profile database is unavailable")?;
    let value = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = ? AND length(CAST(value AS BLOB)) <= ?",
    )
    .bind(IDENTITY_KEY)
    .bind(MAX_MARKER_BYTES as i64)
    .fetch_optional(&mut connection)
    .await;
    connection
        .close()
        .await
        .map_err(|_| "Native profile database could not close")?;
    if value.ok().flatten().as_deref() != Some(encoded) {
        return Err(
            "Native database does not match its ownership marker; profile preserved".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
