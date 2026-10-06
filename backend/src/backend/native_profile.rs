//! Explicit general PostgreSQL profiles. A distinct marker and database identity
//! grant endpoint capability; fixture profiles are never adopted or converted.

use super::development::{self, files, Authority, EndpointCapability};
use super::Backend;
use crate::postgres::schema_compare::manager::CompareManager;
use crate::{app::AppState, credentials, storage};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, SqliteConnection};
use std::{path::Path, path::PathBuf, sync::Arc};

pub(super) const MARKER: &str = ".dbunk-native-profile";
pub(super) const IDENTITY_KEY: &str = "native.profile.identity.v1";
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

/// A fresh general-profile identity whose marker names `path`. The legacy
/// importer writes it into private staging before publishing at `path`.
pub(super) fn new_identity(path: &Path) -> Result<(String, String), String> {
    let marker = Marker {
        version: 1,
        kind: MarkerKind::GeneralPostgres,
        profile_id: uuid::Uuid::new_v4().to_string(),
        credential_namespace: uuid::Uuid::new_v4().to_string(),
        path: path.to_owned(),
    };
    Ok((marker.encoded()?, marker.profile_id))
}

/// Reads and validates the marker stored in `directory` for a profile whose
/// final location is `path`. Returns the canonical encoding and profile ID.
pub(super) fn read_identity(directory: &Path, path: &Path) -> Result<(String, String), String> {
    let marker: Marker = serde_json::from_slice(&files::read_marker(directory, MARKER)?)
        .map_err(|_| "Invalid native profile marker")?;
    marker.validate(path)?;
    Ok((marker.encoded()?, marker.profile_id))
}

impl Backend {
    pub fn native_profile_kind(&self) -> Option<NativeProfileKind> {
        self.0
            .development
            .as_ref()
            .map(|authority| authority.kind())
    }

    /// Creates only a NEW private directory. No endpoint or OS credential store
    /// is opened. The profile is built in a private sibling staging directory
    /// and appears at `path` only through an atomic no-replace rename, so an
    /// interrupted first launch never leaves a partial profile at `path`.
    pub async fn create_native_profile(path: &Path) -> Result<Self, String> {
        development::check_runtime()?;
        development::ensure_process_profile(path)?;
        let lock = stage_and_publish(path, |_| Ok(())).await?;
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

const STAGING_TAG: &str = ".dbunk-create-";
const STAGING_SUFFIX: &str = ".tmp";

/// Returns the canonical parent and UTF-8 name of a new profile path.
fn profile_parts(path: &Path) -> Result<(&Path, &str), String> {
    let invalid = || "Native profile needs a new absolute path with a canonical parent".to_string();
    let parent = path.parent().ok_or_else(invalid)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    if !path.is_absolute()
        || parent.canonicalize().map_err(|_| invalid())? != parent
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid());
    }
    Ok((parent, name))
}

/// Builds the marker and identity-bearing database in a fresh staging
/// directory, commits both to disk, then publishes the directory at `path`
/// without ever replacing an existing entry. `before_publish` is a test seam
/// for failure injection. Any failure removes this attempt's staging
/// directory, so `path` either holds a complete profile or does not exist.
/// Returns the lock taken on the staging directory; it moves with the rename.
async fn stage_and_publish(
    path: &Path,
    before_publish: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<std::fs::File, String> {
    let (parent, name) = profile_parts(path)?;
    if std::fs::symlink_metadata(path).is_ok() {
        return Err("Development profile creation requires a new directory".into());
    }
    discard_abandoned_staging(parent, name);
    let staging = parent.join(format!(
        ".{name}{STAGING_TAG}{}{STAGING_SUFFIX}",
        uuid::Uuid::new_v4()
    ));
    files::create_directory(&staging)?;
    let staged = async {
        let lock = files::lock(&staging)?;
        let (encoded, _) = new_identity(path)?;
        files::write_new(&staging.join(MARKER), encoded.as_bytes())?;
        files::write_new(&staging.join("dbunk.sqlite"), &[])?;
        let paths = storage::Paths::from_dir(staging.clone());
        let pool = storage::open_native_profile_pool(&paths).await?;
        let initialized = storage::set_setting(&pool, IDENTITY_KEY, &encoded).await;
        pool.close().await;
        initialized?;
        files::sync_directory(&staging)?;
        before_publish(&staging)?;
        rename_no_replace(&staging, path)?;
        Ok::<_, String>(lock)
    }
    .await;
    match staged {
        Ok(lock) => {
            files::sync_directory(parent)?;
            Ok(lock)
        }
        Err(error) => {
            // The failed attempt is discarded, never published. A crash before
            // this point leaves only staging, which the next creation removes.
            let _ = std::fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

/// Removes staging directories left by an interrupted creation of `name`.
/// A directory whose lock is held belongs to a live creator and is skipped;
/// anything that is not a plain directory is left untouched. Best effort:
/// leftovers never block creating a new profile.
fn discard_abandoned_staging(parent: &Path, name: &str) {
    let prefix = format!(".{name}{STAGING_TAG}");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(&prefix) || !file_name.ends_with(STAGING_SUFFIX) {
            continue;
        }
        let candidate = entry.path();
        if !std::fs::symlink_metadata(&candidate).is_ok_and(|metadata| metadata.is_dir()) {
            continue;
        }
        if let Ok(lock) = files::lock(&candidate) {
            let _ = std::fs::remove_dir_all(&candidate);
            drop(lock);
        }
    }
}

/// Publishes without ever replacing an existing path.
fn rename_no_replace(from: &Path, to: &Path) -> Result<(), String> {
    let failed = || "Native profile could not be published without replacing a path".to_string();
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let from = std::ffi::CString::new(from.as_os_str().as_bytes()).map_err(|_| failed())?;
        let to = std::ffi::CString::new(to.as_os_str().as_bytes()).map_err(|_| failed())?;
        // SAFETY: both arguments are valid NUL-terminated paths for the call.
        if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
            return Err(failed());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        if std::fs::symlink_metadata(to).is_ok() {
            return Err(failed());
        }
        std::fs::rename(from, to).map_err(|_| failed())
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
