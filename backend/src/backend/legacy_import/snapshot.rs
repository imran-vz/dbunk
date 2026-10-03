//! Consistent, verified capture of a possibly open WAL-mode legacy database.
//!
//! The source is never opened by SQLite: its main file and any `-wal` or
//! `-journal` are read with plain read-only handles, hashed while being copied
//! into a private work directory, then hashed again. Any difference means
//! another host wrote during the capture, and the snapshot is refused. SQLite
//! recovery runs only on the private copy, and `VACUUM INTO` produces one
//! consistent single-file database that is integrity checked, hashed and
//! made read-only.

use super::super::development::files;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqliteConnection};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const MANIFEST: &str = "legacy-snapshot.json";
pub(super) const DATABASE: &str = "legacy.sqlite";
const WORK: &str = "capture";
const KIND: &str = "dbunk-legacy-profile-snapshot";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// SQLite companions that hold committed content. `-shm` is a rebuildable index.
const COMPANIONS: [&str; 2] = ["-wal", "-journal"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotFile {
    /// File name only; directory paths are not recorded.
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Redacted capture record: file names, sizes and checksums only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacySnapshotManifest {
    pub version: u32,
    pub kind: String,
    pub snapshot_id: String,
    pub created_at: String,
    /// The source files as read, identical before and after the copy.
    pub source_files: Vec<SnapshotFile>,
    /// The consistent single-file database the importer reads.
    pub database: SnapshotFile,
    /// Highest recorded legacy migration; validation happens at import.
    pub schema_version: Option<i64>,
}

pub(super) struct VerifiedSnapshot {
    pub manifest: LegacySnapshotManifest,
    pub database: PathBuf,
}

/// Captures `source` (an explicit legacy `dbunk.sqlite`) into the NEW private
/// directory `snapshot`. The source and its directory are never modified.
pub async fn snapshot_legacy_profile(
    source: &Path,
    snapshot: &Path,
) -> Result<LegacySnapshotManifest, String> {
    snapshot_with(source, snapshot, || {}).await
}

/// `between` runs after the copy and before verification; tests use it to
/// simulate a concurrent writer.
pub(super) async fn snapshot_with(
    source: &Path,
    snapshot: &Path,
    between: impl FnOnce(),
) -> Result<LegacySnapshotManifest, String> {
    let source_dir = validate_source(source)?;
    if snapshot.starts_with(&source_dir) || source_dir.starts_with(snapshot) {
        return Err("Snapshot directory must be outside the legacy profile directory".into());
    }
    files::create_directory(snapshot)
        .map_err(|_| "Snapshot requires a new private directory with a canonical parent")?;
    let result = capture(source, snapshot, between).await;
    if result.is_err() {
        // The directory was created by this call and holds only its own files.
        let _ = std::fs::remove_dir_all(snapshot);
    }
    result
}

fn validate_source(source: &Path) -> Result<PathBuf, String> {
    let parent = source
        .parent()
        .filter(|_| source.is_absolute() && source.file_name().is_some())
        .ok_or("Legacy profile source must be an absolute database file path")?;
    let canonical = parent
        .canonicalize()
        .map_err(|_| "Legacy profile source is unavailable")?;
    if canonical != parent {
        return Err("Legacy profile source must have a canonical path without symlinks".into());
    }
    let metadata =
        std::fs::symlink_metadata(source).map_err(|_| "Legacy profile source is unavailable")?;
    if !metadata.is_file() {
        return Err("Legacy profile source must be a regular database file".into());
    }
    Ok(canonical)
}

async fn capture(
    source: &Path,
    snapshot: &Path,
    between: impl FnOnce(),
) -> Result<LegacySnapshotManifest, String> {
    let work = snapshot.join(WORK);
    files::create_directory(&work)?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Legacy profile source name must be UTF-8")?
        .to_owned();
    let mut members = vec![(source.to_owned(), name.clone())];
    for suffix in COMPANIONS {
        let path = PathBuf::from(format!("{}{suffix}", source.display()));
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() => members.push((path, format!("{name}{suffix}"))),
            Ok(_) => return Err("Legacy profile companion file is not a regular file".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Legacy profile companion file is unavailable".into()),
        }
    }
    let mut copied = Vec::new();
    for (path, member) in &members {
        copied.push(copy_hashed(path, &work.join(member), member)?);
    }
    between();
    for ((path, member), expected) in members.iter().zip(&copied) {
        if hash_file(path, member)? != *expected {
            return Err(
                "Legacy profile changed during capture; quit the previous app and retry".into(),
            );
        }
    }
    for suffix in COMPANIONS {
        let appeared = PathBuf::from(format!("{}{suffix}", source.display())).exists();
        if appeared
            != copied
                .iter()
                .any(|file| file.name == format!("{name}{suffix}"))
        {
            return Err(
                "Legacy profile changed during capture; quit the previous app and retry".into(),
            );
        }
    }

    let output = snapshot.join(DATABASE);
    let schema_version = consolidate(&work.join(&name), &output).await;
    std::fs::remove_dir_all(&work).map_err(|_| "Snapshot work files could not be removed")?;
    let schema_version = schema_version?;
    seal(&output)?;
    let database = hash_file(&output, DATABASE)?;
    let manifest = LegacySnapshotManifest {
        version: 1,
        kind: KIND.into(),
        snapshot_id: uuid::Uuid::new_v4().to_string(),
        created_at: crate::storage::now(),
        source_files: copied,
        database,
        schema_version,
    };
    let encoded = serde_json::to_vec_pretty(&manifest).map_err(|_| "Invalid snapshot manifest")?;
    files::write_new(&snapshot.join(MANIFEST), &encoded)?;
    seal(&snapshot.join(MANIFEST))?;
    files::sync_directory(snapshot)?;
    Ok(manifest)
}

/// Recovers the private copy and writes one consistent database file.
async fn consolidate(copy: &Path, output: &Path) -> Result<Option<i64>, String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(copy)
        .create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| "Legacy profile copy is not a readable SQLite database")?;
    let vacuum = sqlx::query("VACUUM INTO ?")
        .bind(output.to_str().ok_or("Snapshot path must be UTF-8")?)
        .execute(&mut connection)
        .await
        .map_err(|_| "Legacy profile copy could not be consolidated");
    let _ = connection.close().await;
    vacuum?;

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(output)
        .create_if_missing(false)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Full);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| "Snapshot database is unavailable")?;
    let checked = async {
        let integrity: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(&mut connection)
            .await
            .map_err(|_| "Snapshot integrity check failed")?;
        if integrity != ["ok"] {
            return Err("Snapshot integrity check failed".to_string());
        }
        let has_migrations: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
        )
        .fetch_optional(&mut connection)
        .await
        .map_err(|_| "Snapshot database is unreadable")?;
        if has_migrations.is_none() {
            return Ok(None);
        }
        sqlx::query_scalar::<_, Option<i64>>("SELECT max(version) FROM schema_migrations")
            .fetch_one(&mut connection)
            .await
            .map_err(|_| "Snapshot database is unreadable".to_string())
    }
    .await;
    connection
        .close()
        .await
        .map_err(|_| "Snapshot database could not close")?;
    checked
}

fn seal(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Snapshot file could not be committed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400))
            .map_err(|_| "Snapshot file could not be made read-only")?;
    }
    Ok(())
}

fn read_only(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|_| "Legacy profile file is unavailable".into())
}

fn copy_hashed(source: &Path, destination: &Path, name: &str) -> Result<SnapshotFile, String> {
    let mut input = read_only(source)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut output = options
        .open(destination)
        .map_err(|_| "Snapshot work file could not be created")?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| "Legacy profile file could not be read")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|_| "Snapshot work file could not be written")?;
        bytes += read as u64;
    }
    output
        .sync_all()
        .map_err(|_| "Snapshot work file could not be committed")?;
    Ok(SnapshotFile {
        name: name.into(),
        bytes,
        sha256: hex(&hasher.finalize()),
    })
}

pub(super) fn hash_file(path: &Path, name: &str) -> Result<SnapshotFile, String> {
    let mut input = read_only(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| "Snapshot file could not be read")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok(SnapshotFile {
        name: name.into(),
        bytes,
        sha256: hex(&hasher.finalize()),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Accepts only an untampered snapshot: exactly the manifest and database,
/// read-only, with the recorded size and checksum.
pub(super) fn verify(snapshot: &Path) -> Result<VerifiedSnapshot, String> {
    let invalid = || "Legacy snapshot is missing, modified or not a verified capture".to_string();
    if !snapshot.is_absolute() || snapshot.canonicalize().map_err(|_| invalid())? != snapshot {
        return Err("Legacy snapshot must be an absolute canonical directory".into());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(snapshot).map_err(|_| invalid())? {
        let entry = entry.map_err(|_| invalid())?;
        if !entry.file_type().map_err(|_| invalid())?.is_file() {
            return Err(invalid());
        }
        names.push(entry.file_name());
    }
    names.sort();
    if names != [MANIFEST, DATABASE] {
        return Err(invalid());
    }
    let mut encoded = Vec::new();
    read_only(&snapshot.join(MANIFEST))?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut encoded)
        .map_err(|_| invalid())?;
    if encoded.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid());
    }
    let manifest: LegacySnapshotManifest =
        serde_json::from_slice(&encoded).map_err(|_| invalid())?;
    if manifest.version != 1 || manifest.kind != KIND || manifest.database.name != DATABASE {
        return Err(invalid());
    }
    super::super::development::canonical_uuid(&manifest.snapshot_id).map_err(|_| invalid())?;
    let database = snapshot.join(DATABASE);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&database)
            .map_err(|_| invalid())?
            .permissions()
            .mode();
        if mode & 0o222 != 0 {
            return Err(invalid());
        }
    }
    if hash_file(&database, DATABASE)? != manifest.database {
        return Err(invalid());
    }
    Ok(VerifiedSnapshot { manifest, database })
}
