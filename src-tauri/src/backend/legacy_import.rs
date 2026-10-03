//! Explicit copy-based import of a baseline Tauri profile (`102568b`, storage
//! schema 18) into a NEW general native profile.
//!
//! Only a verified [`snapshot`](snapshot_legacy_profile) is read, through an
//! immutable read-only attachment; the legacy database is never passed to the
//! ordinary native opener. The destination is built in a private staging
//! directory that already carries its own native marker and SQLite identity,
//! with a journal advanced in the same transaction as each stage's writes. The
//! final path appears only through an atomic no-replace rename after the
//! journal is verified, so an ordinary open can never observe a partial import.
//! Re-running after any interruption resumes from the committed journal stage;
//! re-running after completion returns the stored manifest without writing.
//!
//! Original IDs and React `ui.v1.*` keys are copied verbatim. Native state is
//! added only under its own new keys. Records the native host cannot use
//! (non-PostgreSQL engines, Redis history, unknown JSON fields) are preserved
//! unchanged and stay inactive. Keychain secrets are never read: a Keychain
//! profile imports metadata only and requires credential setup again.

mod baseline;
#[cfg(test)]
mod corpus;
mod session;
mod snapshot;
#[cfg(test)]
mod tests;

use super::development::files;
use super::native_profile;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
};

pub use session::{WorkspaceMapping, WorkspaceMappingStatus};
pub use snapshot::{snapshot_legacy_profile, LegacySnapshotManifest, SnapshotFile};

const JOURNAL_KEY: &str = "native.import.legacy.v1";
const MAX_JOURNAL_BYTES: i64 = 1024 * 1024;
const SETTING_MODE: &str = "credentialStorageMode";
const SESSION_KEY: &str = "ui.v1.session";
/// Copied in this order so foreign keys always find their connection.
const RECORD_TABLES: [&str; 13] = [
    "connections",
    "bastion_servers",
    "managed_servers",
    "query_history",
    "saved_queries",
    "schema_map_positions",
    "schema_map_prefs",
    "table_grid_prefs",
    "virtual_keys",
    "safety_overrides",
    "redis_cli_history",
    "saved_redis_commands",
    "ui_state",
];
const CREDENTIAL_TABLES: [&str; 2] = ["credentials", "credential_verifier"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Stage {
    Created,
    RecordsCommitted,
    CredentialsCommitted,
    NativeStateMapped,
    Verified,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    version: u32,
    import_id: String,
    snapshot_id: String,
    snapshot_sha256: String,
    stage: Stage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceMapping>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest: Option<LegacyImportManifest>,
}

/// Redacted result: counts, IDs and checksums only. No secrets, SQL text,
/// hosts or paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyImportManifest {
    pub version: u32,
    pub import_id: String,
    pub profile_id: String,
    pub snapshot_id: String,
    pub snapshot_sha256: String,
    pub source_schema_version: i64,
    /// Legacy credential mode; Keychain secrets are never imported.
    pub credential_mode: Option<String>,
    pub credential_rows_imported: u64,
    /// Rows outside the active legacy mode, which the baseline treats as stale.
    pub credential_rows_not_imported: u64,
    pub keychain_secrets_require_setup: bool,
    /// Verified source and destination row counts for each copied table.
    pub tables: BTreeMap<String, u64>,
    pub connection_ids: Vec<String>,
    /// Preserved unchanged and inactive in the native host.
    pub unsupported_engine_connection_ids: Vec<String>,
    /// PostgreSQL rows whose stored options the native host does not accept.
    pub unsupported_option_connection_ids: Vec<String>,
    pub workspace: WorkspaceMapping,
}

/// Injected interruptions; production callers always pass `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failpoint {
    AfterCreate,
    BeforeRecordsCommit,
    AfterRecordsCommit,
    BeforeCredentialsCommit,
    AfterCredentialsCommit,
    BeforeNativeStateCommit,
    AfterNativeStateCommit,
    BeforeVerifiedCommit,
    AfterVerifiedCommit,
    BeforePublish,
    AfterPublish,
}

fn check(fail: Option<Failpoint>, point: Failpoint) -> Result<(), String> {
    if fail == Some(point) {
        return Err(format!("Injected interruption at {point:?}"));
    }
    Ok(())
}

/// Imports a verified legacy snapshot into the NEW native profile `destination`.
/// Existing destinations are accepted only when they are this exact import's
/// completed result; anything else is refused without modification.
pub async fn import_legacy_profile(
    snapshot: &Path,
    destination: &Path,
) -> Result<LegacyImportManifest, String> {
    import_with(snapshot, destination, None).await
}

async fn import_with(
    snapshot: &Path,
    destination: &Path,
    fail: Option<Failpoint>,
) -> Result<LegacyImportManifest, String> {
    let snapshot_dir = snapshot;
    let snapshot = snapshot::verify(snapshot_dir)?;
    let (parent, name) = destination_parts(destination)?;
    if destination.starts_with(snapshot_dir) || snapshot_dir.starts_with(destination) {
        return Err("Destination and snapshot must be separate directories".into());
    }
    if std::fs::symlink_metadata(destination).is_ok() {
        return completed(destination, &snapshot.manifest).await;
    }
    let legacy = inspect(&snapshot.database).await?;
    let staging = parent.join(format!(".{name}.dbunk-legacy-import"));
    let (lock, profile_id) = if std::fs::symlink_metadata(&staging).is_ok() {
        open_staging(&staging, destination, &snapshot.manifest).await?
    } else {
        create_staging(&parent, &name, &staging, destination, &snapshot.manifest).await?
    };
    check(fail, Failpoint::AfterCreate)?;

    let mut connection = connect(&staging.join("dbunk.sqlite")).await?;
    let result = run_stages(&mut connection, &snapshot, &legacy, &profile_id, fail).await;
    if result.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut connection).await;
    }
    let closed = async {
        sqlx::query("DETACH DATABASE legacy")
            .execute(&mut connection)
            .await
            .map_err(|_| "Legacy snapshot could not be detached")?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&mut connection)
            .await
            .map_err(|_| "Imported profile could not be checkpointed")?;
        Ok::<_, String>(())
    }
    .await;
    let _ = connection.close().await;
    let manifest = result?;
    closed?;
    files::sync_directory(&staging)?;
    check(fail, Failpoint::BeforePublish)?;
    rename_no_replace(&staging, destination)?;
    files::sync_directory(&parent)?;
    drop(lock);
    check(fail, Failpoint::AfterPublish)?;
    Ok(manifest)
}

fn destination_parts(destination: &Path) -> Result<(PathBuf, String), String> {
    let invalid = || "Destination must be a new absolute path with a canonical parent".to_string();
    let parent = destination.parent().ok_or_else(invalid)?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(invalid)?;
    if !destination.is_absolute()
        || parent.canonicalize().map_err(|_| invalid())? != parent
        || destination.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid());
    }
    Ok((parent.to_owned(), name.to_owned()))
}

/// Facts read from the validated snapshot before any destination write.
struct Legacy {
    columns: BTreeMap<String, Vec<String>>,
    credential_mode: Option<String>,
    unsupported_engines: Vec<String>,
}

/// Validates the snapshot read-only against the frozen baseline schema.
async fn inspect(database: &Path) -> Result<Legacy, String> {
    let expected = baseline_columns().await?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .read_only(true)
        .immutable(true)
        .create_if_missing(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|_| "Legacy snapshot is unavailable")?;
    let inspected = async {
        let mut connection = pool
            .acquire()
            .await
            .map_err(|_| "Legacy snapshot is unavailable")?;
        let legacy = inspect_connection(&mut connection, expected).await?;
        drop(connection);
        // Every row must decode with the baseline reader, so the native reader
        // cannot later reject the whole imported profile.
        crate::storage::read_connections(&pool)
            .await
            .map_err(|_| "Legacy connection metadata is unreadable; nothing was imported")?;
        Ok(legacy)
    }
    .await;
    pool.close().await;
    inspected
}

async fn inspect_connection(
    connection: &mut SqliteConnection,
    expected: BTreeMap<String, Vec<String>>,
) -> Result<Legacy, String> {
    let unreadable = |_| "Legacy snapshot is unreadable".to_string();
    let has_migrations: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(unreadable)?;
    if has_migrations.is_none() {
        return Err("Snapshot is not a dbunk legacy profile; nothing was imported".into());
    }
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(&mut *connection)
            .await
            .map_err(unreadable)?;
    if let Some(newest) = versions
        .last()
        .filter(|newest| **newest > baseline::BASELINE_SCHEMA_VERSION)
    {
        return Err(format!(
            "Legacy profile schema version {newest} is newer than the supported baseline ({}); nothing was imported",
            baseline::BASELINE_SCHEMA_VERSION
        ));
    }
    if versions != (1..=baseline::BASELINE_SCHEMA_VERSION).collect::<Vec<_>>() {
        return Err(
            "Legacy profile is not at the baseline schema version; open it once with the previous app or keep it unchanged"
                .into(),
        );
    }
    let actual = table_columns(connection, "main").await?;
    if actual != expected {
        return Err(
            "Legacy profile schema does not match the baseline; nothing was imported".into(),
        );
    }
    let foreign: Vec<String> = sqlx::query_scalar("SELECT \"table\" FROM pragma_foreign_key_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(unreadable)?;
    if !foreign.is_empty() {
        return Err("Legacy profile has orphaned records; nothing was imported".into());
    }
    let native: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM app_settings WHERE key LIKE 'native.%')
              + (SELECT count(*) FROM ui_state WHERE key LIKE 'ui.v1.native.%' OR key NOT LIKE 'ui.v1.%')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(unreadable)?;
    if native != 0 {
        return Err(
            "Snapshot contains native-owned or unnamespaced state; it is not a baseline profile"
                .into(),
        );
    }
    let credential_mode: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(SETTING_MODE)
            .fetch_optional(&mut *connection)
            .await
            .map_err(unreadable)?;
    if credential_mode
        .as_deref()
        .is_some_and(|mode| !["keychain", "plain-sqlite", "encrypted-sqlite"].contains(&mode))
    {
        return Err("Legacy credential mode is unknown; nothing was imported".into());
    }
    // Every row must decode with the baseline reader, including all five
    // baseline engines, so the native reader cannot later reject the profile.
    let rows =
        sqlx::query("SELECT id, engine, environment, safe_mode FROM connections ORDER BY id")
            .fetch_all(&mut *connection)
            .await
            .map_err(unreadable)?;
    let mut unsupported_engines = Vec::new();
    for row in rows {
        let engine: String = row.get("engine");
        let environment: String = row.get("environment");
        let safe_mode: String = row.get("safe_mode");
        if engine.parse::<crate::DatabaseEngine>().is_err()
            || environment.parse::<crate::Environment>().is_err()
            || safe_mode.parse::<crate::SafeMode>().is_err()
        {
            return Err("Legacy connection metadata is unreadable; nothing was imported".into());
        }
        if engine != "PostgreSQL" {
            unsupported_engines.push(row.get("id"));
        }
    }
    Ok(Legacy {
        columns: actual,
        credential_mode,
        unsupported_engines,
    })
}

/// Column names per table of the frozen baseline schema, built in memory.
async fn baseline_columns() -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut connection = SqliteConnection::connect("sqlite::memory:")
        .await
        .map_err(|_| "Baseline schema could not be prepared")?;
    apply_baseline(&mut connection).await?;
    let columns = table_columns(&mut connection, "main").await;
    let _ = connection.close().await;
    columns
}

/// Applies the frozen baseline migrations, as the baseline host did.
async fn apply_baseline(connection: &mut SqliteConnection) -> Result<(), String> {
    let failed = |_| "Baseline schema could not be prepared".to_string();
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
    )
    .execute(&mut *connection)
    .await
    .map_err(failed)?;
    for (version, sql) in baseline::BASELINE_MIGRATIONS {
        let mut transaction = connection.begin().await.map_err(failed)?;
        for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .map_err(failed)?;
        }
        sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (?, ?)")
            .bind(version)
            .bind("2026-01-01T00:00:00+00:00")
            .execute(&mut *transaction)
            .await
            .map_err(failed)?;
        transaction.commit().await.map_err(failed)?;
    }
    Ok(())
}

async fn table_columns(
    connection: &mut SqliteConnection,
    schema: &str,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let unreadable = |_| "Profile schema is unreadable".to_string();
    let tables: Vec<String> = sqlx::query_scalar(&format!(
        "SELECT name FROM {schema}.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
    ))
    .fetch_all(&mut *connection)
    .await
    .map_err(unreadable)?;
    let mut columns = BTreeMap::new();
    for table in tables {
        let names: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?, ?) ORDER BY cid")
                .bind(&table)
                .bind(schema)
                .fetch_all(&mut *connection)
                .await
                .map_err(unreadable)?;
        columns.insert(table, names);
    }
    Ok(columns)
}

async fn connect(database: &Path) -> Result<SqliteConnection, String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(false)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Full)
        .foreign_keys(true);
    SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| "Import staging database is unavailable".into())
}

/// Creates the destination identity in a unique private directory, commits the
/// identity and journal together, then publishes it as the staging directory.
async fn create_staging(
    parent: &Path,
    name: &str,
    staging: &Path,
    destination: &Path,
    snapshot: &LegacySnapshotManifest,
) -> Result<(std::fs::File, String), String> {
    let temporary = parent.join(format!(
        ".{name}.dbunk-legacy-import-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    files::create_directory(&temporary)?;
    let lock = files::lock(&temporary)?;
    let (encoded, profile_id) = native_profile::new_identity(destination)?;
    files::write_new(&temporary.join(native_profile::MARKER), encoded.as_bytes())?;
    files::write_new(&temporary.join("dbunk.sqlite"), &[])?;
    let paths = crate::storage::Paths::from_dir(temporary.clone());
    let pool = crate::storage::open_native_profile_pool(&paths).await?;
    let journal = Journal {
        version: 1,
        import_id: uuid::Uuid::new_v4().to_string(),
        snapshot_id: snapshot.snapshot_id.clone(),
        snapshot_sha256: snapshot.database.sha256.clone(),
        stage: Stage::Created,
        workspace: None,
        manifest: None,
    };
    let initialized = async {
        let mut transaction = pool
            .begin()
            .await
            .map_err(|_| "Import identity could not be written")?;
        for (key, value) in [
            (native_profile::IDENTITY_KEY, encoded.clone()),
            (JOURNAL_KEY, encode_journal(&journal)?),
        ] {
            sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)")
                .bind(key)
                .bind(value)
                .bind(crate::storage::now())
                .execute(&mut *transaction)
                .await
                .map_err(|_| "Import identity could not be written")?;
        }
        transaction
            .commit()
            .await
            .map_err(|_| "Import identity could not be written".to_string())
    }
    .await;
    pool.close().await;
    initialized?;
    files::sync_directory(&temporary)?;
    rename_no_replace(&temporary, staging)?;
    files::sync_directory(parent)?;
    Ok((lock, profile_id))
}

/// Resumes only staging created for this destination and this snapshot.
async fn open_staging(
    staging: &Path,
    destination: &Path,
    snapshot: &LegacySnapshotManifest,
) -> Result<(std::fs::File, String), String> {
    let lock = files::validate_files(staging, native_profile::MARKER)
        .map_err(|error| format!("Unfinished import cannot be resumed: {error}"))?;
    let (encoded, profile_id) = native_profile::read_identity(staging, destination)?;
    let journal = read_journal(&staging.join("dbunk.sqlite"), &encoded).await?;
    if journal.snapshot_sha256 != snapshot.database.sha256
        || journal.snapshot_id != snapshot.snapshot_id
    {
        return Err(
            "An unfinished import of a different snapshot exists for this destination; nothing was changed"
                .into(),
        );
    }
    Ok((lock, profile_id))
}

/// An existing destination is accepted only as this import's completed result.
async fn completed(
    destination: &Path,
    snapshot: &LegacySnapshotManifest,
) -> Result<LegacyImportManifest, String> {
    let other =
        || "Destination already exists with another identity; nothing was changed".to_string();
    if std::fs::symlink_metadata(destination.join(native_profile::MARKER)).is_err() {
        return Err(other());
    }
    let lock = files::validate_files(destination, native_profile::MARKER).map_err(|error| {
        format!("Destination cannot be inspected ({error}); nothing was changed")
    })?;
    let (encoded, _) =
        native_profile::read_identity(destination, destination).map_err(|_| other())?;
    let journal = read_journal(&destination.join("dbunk.sqlite"), &encoded)
        .await
        .map_err(|_| other())?;
    drop(lock);
    match journal {
        Journal {
            stage: Stage::Verified,
            manifest: Some(manifest),
            snapshot_sha256,
            snapshot_id,
            ..
        } if snapshot_sha256 == snapshot.database.sha256 && snapshot_id == snapshot.snapshot_id => {
            Ok(manifest)
        }
        _ => Err(other()),
    }
}

/// Reads the journal after proving the database holds the marker's identity.
async fn read_journal(database: &Path, identity: &str) -> Result<Journal, String> {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .read_only(true)
        .create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| "Import database is unavailable")?;
    let read = async {
        let value = |key: &'static str| {
            sqlx::query_scalar::<_, String>(
                "SELECT value FROM app_settings WHERE key = ? AND length(CAST(value AS BLOB)) <= ?",
            )
            .bind(key)
            .bind(MAX_JOURNAL_BYTES)
        };
        let stored = value(native_profile::IDENTITY_KEY)
            .fetch_optional(&mut connection)
            .await
            .ok()
            .flatten();
        if stored.as_deref() != Some(identity) {
            return Err("Import database does not match its ownership marker".to_string());
        }
        let journal = value(JOURNAL_KEY)
            .fetch_optional(&mut connection)
            .await
            .map_err(|_| "Import journal is unreadable")?
            .ok_or("Native profile was not created by a legacy import")?;
        serde_json::from_str::<Journal>(&journal)
            .ok()
            .filter(|journal| journal.version == 1)
            .ok_or_else(|| "Import journal is unreadable".to_string())
    }
    .await;
    let _ = connection.close().await;
    read
}

fn encode_journal(journal: &Journal) -> Result<String, String> {
    serde_json::to_string(journal).map_err(|_| "Import journal could not be encoded".into())
}

async fn run_stages(
    connection: &mut SqliteConnection,
    snapshot: &snapshot::VerifiedSnapshot,
    legacy: &Legacy,
    profile_id: &str,
    fail: Option<Failpoint>,
) -> Result<LegacyImportManifest, String> {
    sqlx::query("ATTACH DATABASE ? AS legacy")
        .bind(attach_uri(&snapshot.database)?)
        .execute(&mut *connection)
        .await
        .map_err(|_| "Legacy snapshot could not be attached read-only")?;
    let mut journal = current_journal(connection).await?;
    if journal.snapshot_sha256 != snapshot.manifest.database.sha256 {
        return Err("Import journal names a different snapshot; nothing was changed".into());
    }
    let destination = table_columns(connection, "main").await?;
    for (table, columns) in &legacy.columns {
        let present = destination
            .get(table)
            .ok_or("Native schema lacks a baseline table")?;
        if columns.iter().any(|column| !present.contains(column)) {
            return Err("Native schema lacks a baseline column".into());
        }
    }

    if journal.stage < Stage::RecordsCommitted {
        begin(connection).await?;
        for table in RECORD_TABLES {
            copy_table(connection, table, &legacy.columns[table], None).await?;
        }
        copy_table(
            connection,
            "app_settings",
            &legacy.columns["app_settings"],
            Some("key NOT IN ('onboardingCompleted', 'credentialStorageMode')"),
        )
        .await?;
        journal.stage = Stage::RecordsCommitted;
        commit_stage(connection, &journal, fail, Failpoint::BeforeRecordsCommit).await?;
        check(fail, Failpoint::AfterRecordsCommit)?;
    }

    if journal.stage < Stage::CredentialsCommitted {
        begin(connection).await?;
        if let Some(mode @ ("plain-sqlite" | "encrypted-sqlite")) =
            legacy.credential_mode.as_deref()
        {
            sqlx::query(
                "INSERT INTO main.credentials (credential_id, storage_mode, nonce, password_value, updated_at)
                 SELECT credential_id, storage_mode, nonce, password_value, updated_at
                 FROM legacy.credentials WHERE storage_mode = ? ORDER BY rowid",
            )
            .bind(mode)
            .execute(&mut *connection)
            .await
            .map_err(|_| "Legacy credentials could not be imported")?;
            if mode == "encrypted-sqlite" {
                copy_table(
                    connection,
                    "credential_verifier",
                    &legacy.columns["credential_verifier"],
                    None,
                )
                .await?;
            }
            copy_table(
                connection,
                "app_settings",
                &legacy.columns["app_settings"],
                Some("key IN ('onboardingCompleted', 'credentialStorageMode')"),
            )
            .await?;
        }
        journal.stage = Stage::CredentialsCommitted;
        commit_stage(
            connection,
            &journal,
            fail,
            Failpoint::BeforeCredentialsCommit,
        )
        .await?;
        check(fail, Failpoint::AfterCredentialsCommit)?;
    }

    if journal.stage < Stage::NativeStateMapped {
        begin(connection).await?;
        let session = sqlx::query("SELECT value, updated_at FROM legacy.ui_state WHERE key = ?")
            .bind(SESSION_KEY)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|_| "Legacy session could not be read")?;
        let unsupported: HashSet<String> = legacy.unsupported_engines.iter().cloned().collect();
        let raw = session.as_ref().map(|row| row.get::<String, _>("value"));
        let mapped = session::map_session(raw.as_deref(), &unsupported);
        if let (Some((key, value)), Some(row)) = (mapped.record, session) {
            // Plain INSERT: an existing native record is never overwritten.
            sqlx::query("INSERT INTO main.ui_state (key, value, updated_at) VALUES (?, ?, ?)")
                .bind(key)
                .bind(value)
                .bind(row.get::<String, _>("updated_at"))
                .execute(&mut *connection)
                .await
                .map_err(|_| "Native workspace could not be written")?;
        }
        journal.workspace = Some(mapped.mapping);
        journal.stage = Stage::NativeStateMapped;
        commit_stage(
            connection,
            &journal,
            fail,
            Failpoint::BeforeNativeStateCommit,
        )
        .await?;
        check(fail, Failpoint::AfterNativeStateCommit)?;
    }

    if journal.stage < Stage::Verified {
        let manifest = verify(connection, snapshot, legacy, &journal, profile_id).await?;
        begin(connection).await?;
        journal.manifest = Some(manifest);
        journal.stage = Stage::Verified;
        commit_stage(connection, &journal, fail, Failpoint::BeforeVerifiedCommit).await?;
        check(fail, Failpoint::AfterVerifiedCommit)?;
    }
    journal
        .manifest
        .ok_or_else(|| "Import journal is incomplete".to_string())
}

/// Compares the staged destination with the snapshot before it can be published.
async fn verify(
    connection: &mut SqliteConnection,
    snapshot: &snapshot::VerifiedSnapshot,
    legacy: &Legacy,
    journal: &Journal,
    profile_id: &str,
) -> Result<LegacyImportManifest, String> {
    let mismatch =
        || "Imported profile does not match the snapshot; it was not published".to_string();
    let integrity: Vec<String> = sqlx::query_scalar("PRAGMA main.integrity_check")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| mismatch())?;
    if integrity != ["ok"] {
        return Err(mismatch());
    }
    let mut tables = BTreeMap::new();
    for table in RECORD_TABLES {
        let columns = legacy.columns[table]
            .iter()
            .map(|column| quote(column))
            .collect::<Vec<_>>()
            .join(", ");
        // Every legacy row exists unchanged in the destination.
        let missing: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM (SELECT {columns} FROM legacy.{t} EXCEPT SELECT {columns} FROM main.{t})",
            t = quote(table)
        ))
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| mismatch())?;
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM legacy.{}", quote(table)))
                .fetch_one(&mut *connection)
                .await
                .map_err(|_| mismatch())?;
        if missing != 0 {
            return Err(mismatch());
        }
        tables.insert(table.to_string(), count as u64);
    }
    let missing_settings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM legacy.app_settings l
         WHERE l.key NOT IN ('onboardingCompleted', 'credentialStorageMode')
           AND NOT EXISTS (SELECT 1 FROM main.app_settings m WHERE m.key = l.key AND m.value IS l.value)",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| mismatch())?;
    if missing_settings != 0 {
        return Err(mismatch());
    }
    let settings: i64 = sqlx::query_scalar("SELECT count(*) FROM legacy.app_settings")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| mismatch())?;
    tables.insert("app_settings".into(), settings as u64);
    let destination_ids: BTreeSet<String> = sqlx::query_scalar("SELECT id FROM main.connections")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| mismatch())?
        .into_iter()
        .collect();
    let connection_ids: BTreeSet<String> = sqlx::query_scalar("SELECT id FROM legacy.connections")
        .fetch_all(&mut *connection)
        .await
        .map_err(|_| mismatch())?
        .into_iter()
        .collect();
    if destination_ids != connection_ids {
        return Err(mismatch());
    }
    let total_credentials: i64 = sqlx::query_scalar("SELECT count(*) FROM legacy.credentials")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| mismatch())?;
    let imported_credentials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM main.credentials m JOIN legacy.credentials l
         ON l.credential_id = m.credential_id AND l.storage_mode = m.storage_mode
         AND l.nonce IS m.nonce AND l.password_value = m.password_value",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| mismatch())?;
    let stored_credentials: i64 = sqlx::query_scalar("SELECT count(*) FROM main.credentials")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| mismatch())?;
    if imported_credentials != stored_credentials {
        return Err(mismatch());
    }
    for table in CREDENTIAL_TABLES {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM main.{}", quote(table)))
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| mismatch())?;
        tables.insert(format!("{table}.imported"), count as u64);
    }
    // The native reader must accept every row, flagging unsupported options.
    let options: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, engine, tls_options, driver_options FROM main.connections ORDER BY id",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| mismatch())?;
    let unsupported_option_connection_ids = options
        .into_iter()
        .filter(|(_, engine, tls, driver)| {
            engine == "PostgreSQL"
                && !(native_options(tls, TLS_KEYS) && native_options(driver, DRIVER_KEYS))
        })
        .map(|(id, ..)| id)
        .collect();
    Ok(LegacyImportManifest {
        version: 1,
        import_id: journal.import_id.clone(),
        profile_id: profile_id.into(),
        snapshot_id: snapshot.manifest.snapshot_id.clone(),
        snapshot_sha256: snapshot.manifest.database.sha256.clone(),
        source_schema_version: baseline::BASELINE_SCHEMA_VERSION,
        credential_mode: legacy.credential_mode.clone(),
        credential_rows_imported: imported_credentials as u64,
        credential_rows_not_imported: (total_credentials - imported_credentials) as u64,
        keychain_secrets_require_setup: legacy.credential_mode.as_deref() == Some("keychain"),
        tables,
        connection_ids: connection_ids.into_iter().collect(),
        unsupported_engine_connection_ids: legacy.unsupported_engines.clone(),
        unsupported_option_connection_ids,
        workspace: journal
            .workspace
            .clone()
            .ok_or_else(|| "Import journal is incomplete".to_string())?,
    })
}

const TLS_KEYS: &[&str] = &[
    "mode",
    "rootCertPath",
    "clientCertPath",
    "clientKeyPath",
    "serverName",
];
const DRIVER_KEYS: &[&str] = &[
    "statementTimeoutMs",
    "idleInTransactionTimeoutMs",
    "connectTimeoutMs",
    "keepaliveSeconds",
    "defaultSearchPath",
    "defaultRole",
];

/// Mirrors the native reader's key allowlist for reporting; the stored JSON
/// itself is preserved byte-for-byte either way.
fn native_options(raw: &Option<String>, keys: &[&str]) -> bool {
    let Some(raw) = raw else {
        return true;
    };
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .map(|object| object.keys().all(|key| keys.contains(&key.as_str())))
        })
        .unwrap_or(false)
}

async fn current_journal(connection: &mut SqliteConnection) -> Result<Journal, String> {
    let encoded: String = sqlx::query_scalar("SELECT value FROM main.app_settings WHERE key = ?")
        .bind(JOURNAL_KEY)
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| "Import journal is missing")?;
    serde_json::from_str(&encoded).map_err(|_| "Import journal is unreadable".into())
}

async fn begin(connection: &mut SqliteConnection) -> Result<(), String> {
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|_| "Import transaction could not start".into())
}

/// Advances the journal inside the open transaction, then commits both.
async fn commit_stage(
    connection: &mut SqliteConnection,
    journal: &Journal,
    fail: Option<Failpoint>,
    before: Failpoint,
) -> Result<(), String> {
    let updated =
        sqlx::query("UPDATE main.app_settings SET value = ?, updated_at = ? WHERE key = ?")
            .bind(encode_journal(journal)?)
            .bind(crate::storage::now())
            .bind(JOURNAL_KEY)
            .execute(&mut *connection)
            .await
            .map_err(|_| "Import journal could not be advanced")?;
    if updated.rows_affected() != 1 {
        return Err("Import journal is missing".into());
    }
    check(fail, before)?;
    sqlx::query("COMMIT")
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|_| "Import stage could not be committed".into())
}

async fn copy_table(
    connection: &mut SqliteConnection,
    table: &str,
    columns: &[String],
    filter: Option<&str>,
) -> Result<(), String> {
    let columns = columns
        .iter()
        .map(|column| quote(column))
        .collect::<Vec<_>>()
        .join(", ");
    let filter = filter.map_or_else(String::new, |filter| format!(" WHERE {filter}"));
    sqlx::query(&format!(
        "INSERT INTO main.{t} ({columns}) SELECT {columns} FROM legacy.{t}{filter} ORDER BY rowid",
        t = quote(table)
    ))
    .execute(&mut *connection)
    .await
    .map(|_| ())
    .map_err(|_| format!("Legacy {table} records could not be imported"))
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Read-only, immutable URI for ATTACH; the snapshot file is also mode 0400.
fn attach_uri(database: &Path) -> Result<String, String> {
    let path = database.to_str().ok_or("Snapshot path must be UTF-8")?;
    let mut uri = String::from("file:");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Ok(uri)
}

/// Publishes without ever replacing an existing directory.
fn rename_no_replace(from: &Path, to: &Path) -> Result<(), String> {
    let failed = || "Import directory could not be published without replacing a path".to_string();
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

#[cfg(test)]
const ALL_FAILPOINTS: [Failpoint; 11] = [
    Failpoint::AfterCreate,
    Failpoint::BeforeRecordsCommit,
    Failpoint::AfterRecordsCommit,
    Failpoint::BeforeCredentialsCommit,
    Failpoint::AfterCredentialsCommit,
    Failpoint::BeforeNativeStateCommit,
    Failpoint::AfterNativeStateCommit,
    Failpoint::BeforeVerifiedCommit,
    Failpoint::AfterVerifiedCommit,
    Failpoint::BeforePublish,
    Failpoint::AfterPublish,
];
