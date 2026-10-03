use std::fs::{File, OpenOptions};
use std::path::Path;

use super::FixtureSummary;
use crate::app::AppState;
use crate::postgres::schema_compare::manager::CompareManager;
use crate::{credentials, storage, CredentialStorageMode, StoredConnection};

pub(super) const CONNECTION_ID: &str = "native-stage03-fixture";
const MARKER: &str = ".dbunk-native-stage03";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    fixture: String,
    host: String,
    port: u16,
    database: String,
    profile_id: String,
}

pub(super) fn summary() -> FixtureSummary {
    FixtureSummary {
        id: CONNECTION_ID.into(),
        name: "Native PostgreSQL fixture".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
    }
}

/// Validate the directory before SQLite or credential code can access it.
fn validate(path: &Path) -> Result<(File, String), String> {
    if !path.is_absolute() || path.canonicalize().map_err(|e| e.to_string())? != path {
        return Err(
            "Fixture profile must be an absolute canonical directory without symlinks".into(),
        );
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() {
        return Err("Fixture profile is not a directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.permissions().mode() & 0o777 != 0o700
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err("Fixture profile must be owned by this user with mode 0700".into());
        }
    }
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file()
            || !matches!(
                entry.file_name().to_str(),
                Some(
                    MARKER
                        | ".dbunk-native-lock"
                        | "launch.json"
                        | "dbunk.sqlite"
                        | "dbunk.sqlite-wal"
                        | "dbunk.sqlite-shm"
                )
            )
        {
            return Err("Fixture profile contains a foreign file or symlink".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if entry.metadata().map_err(|e| e.to_string())?.nlink() != 1 {
                return Err("Fixture profile contains a hard-linked file".into());
            }
        }
    }
    if std::fs::metadata(path.join(MARKER))
        .map_err(|_| "Fixture ownership marker missing".to_string())?
        .len()
        > 4096
    {
        return Err("Fixture marker is too large".into());
    }
    let marker_bytes = std::fs::read(path.join(MARKER))
        .map_err(|_| "Fixture ownership marker missing".to_string())?;
    if marker_bytes.len() > 4096 {
        return Err("Fixture marker is too large".into());
    }
    let marker: Marker =
        serde_json::from_slice(&marker_bytes).map_err(|_| "Invalid fixture marker".to_string())?;
    if marker.version != 1
        || marker.fixture != "dbunk-native-stage03"
        || marker.host != "127.0.0.1"
        || marker.port != 15432
        || marker.database != "dbunk_demo"
        || uuid::Uuid::parse_str(&marker.profile_id).is_err()
    {
        return Err("Fixture marker does not identify the native loopback fixture".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options
        .open(path.join(".dbunk-native-lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Fixture profile is already in use".to_string())?;
    Ok((lock, marker.profile_id))
}

pub(super) async fn open(path: &Path) -> Result<(AppState, File), String> {
    let (lock, profile_id) = validate(path)?;
    let paths = storage::Paths::from_dir(path.to_path_buf());
    let fresh = !paths.db_file().exists();
    let pool = storage::open_pool(&paths).await?;
    let initialize = async {
        match credentials::credential_mode(&pool).await? {
            Some(CredentialStorageMode::PlainSqlite) => {}
            None if fresh => {
                // `configure` clears all credential backends, including the
                // keychain. This verified fresh profile has nothing to migrate.
                credentials::set_credential_mode(&pool, CredentialStorageMode::PlainSqlite).await?;
                credentials::mark_onboarding_completed(&pool).await?;
            }
            _ => {
                return Err("Fixture profile requires existing PlainSqlite credential mode".into())
            }
        }
        if fresh {
            storage::set_setting(&pool, "native.profileId", &profile_id).await?;
        } else if storage::get_setting(&pool, "native.profileId")
            .await?
            .as_deref()
            != Some(&profile_id)
        {
            return Err("Fixture profile database does not match its ownership marker".into());
        }
        let state = AppState::with_credentials(
            paths,
            CompareManager::new(),
            credentials::Context::fixture(pool.clone()),
        );
        let connections = storage::read_connections(&pool).await?;
        if connections.is_empty() && fresh {
            // This is immutable seed data in a validated fresh PlainSqlite
            // profile, before any sessions exist. Keep seed initialization
            // independent of ordinary-app onboarding/cleanup policy. The
            // runtime credential context also has no Keychain capability.
            let connection = fixture_connection();
            storage::upsert_connection(&pool, &connection).await?;
            storage::upsert_sqlite_credential(
                &pool,
                connection.id(),
                CredentialStorageMode::PlainSqlite,
                None,
                connection.password(),
            )
            .await?;
        } else {
            let [StoredConnection::PostgreSQL(connection)] = connections.as_slice() else {
                return Err("Fixture profile has foreign connection records".into());
            };
            if connection.id != CONNECTION_ID
                || connection.host != "127.0.0.1"
                || connection.port != 15432
                || connection.database != "dbunk_demo"
                || connection.user != "dbunk"
                || !connection.ssh_tunnel.is_default()
                || connection.ssl
                || connection.tls_options.is_some()
            {
                return Err("Fixture profile connection does not match its marker".into());
            }
        }
        Ok(state)
    }
    .await;
    if initialize.is_err() {
        pool.close().await;
    }
    initialize.map(|state| (state, lock))
}

fn fixture_connection() -> StoredConnection {
    StoredConnection::PostgreSQL(crate::PgStoredConnection {
        id: CONNECTION_ID.into(),
        name: "Native PostgreSQL fixture".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        password: "dbunk".into(),
        role: "read/write".into(),
        environment: crate::Environment::Development,
        safe_mode: crate::SafeMode::Protected,
        read_only: false,
        last_activity_at: None,
        ssl: false,
        tls_options: None,
        driver_options: Some(crate::PgDriverOptions {
            connect_timeout_ms: Some(5000),
            ..Default::default()
        }),
        ssh_tunnel: Default::default(),
        organization: Default::default(),
    })
}

#[cfg(test)]
pub(super) fn directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(
        directory.path().join(MARKER),
        serde_json::to_vec(&serde_json::json!({
            "version": 1, "fixture": "dbunk-native-stage03", "host": "127.0.0.1", "port": 15432,
            "database": "dbunk_demo", "profile_id": uuid::Uuid::new_v4().to_string(),
        }))
        .unwrap(),
    )
    .unwrap();
    directory
}
