//! Bounded draft storage for the isolated native host. No connection/session is
//! resolved while loading; deleted connection bindings remain recoverable.

use super::Backend;
use crate::backend::Layout;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use std::{collections::HashSet, fmt, io::Write};

#[path = "workspace_copy.rs"]
mod copy_state;
#[path = "workspace_seed.rs"]
mod seed_state;
pub use copy_state::{WorkspaceTableCopy, WorkspaceTableCopyState, WORKSPACE_COPY_MAX_JOBS};
pub use seed_state::{WorkspaceTableSeed, WorkspaceTableSeedState, WORKSPACE_SEED_MAX_JOBS};

#[path = "workspace_maintenance.rs"]
mod maintenance_state;
pub use maintenance_state::{
    WorkspaceMaintenance, WorkspaceMaintenanceAction, WorkspaceMaintenanceKind,
    WorkspaceMaintenanceState,
};
#[path = "workspace_admin.rs"]
mod admin_state;
pub use admin_state::{WorkspaceAdminAction, WorkspaceAdminControl};
#[path = "workspace_table_ddl.rs"]
mod table_ddl_state;
pub use table_ddl_state::{WorkspaceTableDdl, WORKSPACE_TABLE_DDL_MAX_BYTES};
#[path = "workspace_schema.rs"]
mod schema_state;
#[path = "workspace_table.rs"]
mod table_state;
pub use schema_state::WorkspaceSchemaChanges;
pub use table_state::{
    WorkspaceApplyState, WorkspaceMutationDraft, WorkspaceQueryChanges, WorkspaceStagedChange,
    WorkspaceTableState, WORKSPACE_MUTATION_MAX_BYTES, WORKSPACE_MUTATION_MAX_CHANGES,
};

const KEY: &str = "ui.v1.native.workspace";
pub const NATIVE_WORKSPACE_MAX_BYTES: usize = 448 * 1024;
pub const NATIVE_WORKSPACE_MAX_DOCUMENTS: usize = 16;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceDensity {
    Compact,
    #[default]
    Comfortable,
}

/// UTF-8 byte offsets, with the active caret at `head`. Restoration clamps each
/// endpoint backwards to a character boundary without modifying SQL text.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSelection {
    pub anchor: usize,
    pub head: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceTool {
    BackupRestore,
    CsvTransfer,
    TableCopy,
    TableSeed,
    SchemaCompare,
    SchemaMap,
    Administration,
    Objects,
    History,
    SavedQueries,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceDocument {
    pub id: String,
    pub name: String,
    pub connection_id: Option<String>,
    pub sql: String,
    pub pinned: bool,
    pub selection: WorkspaceSelection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<WorkspaceTableState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_changes: Option<WorkspaceQueryChanges>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_changes: Option<WorkspaceSchemaChanges>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_ddl: Option<WorkspaceTableDdl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_control: Option<WorkspaceAdminControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<WorkspaceMaintenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<WorkspaceTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_query_id: Option<String>,
}

impl fmt::Debug for WorkspaceDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceDocument")
            .field("id", &self.id)
            .field("sql_bytes", &self.sql.len())
            .finish_non_exhaustive()
    }
}

/// Document order is the tab order. This payload has no runtime or credential
/// fields. Draft SQL is plaintext, independently of credential encryption.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSnapshot {
    pub documents: Vec<WorkspaceDocument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy_jobs: Vec<WorkspaceTableCopy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seed_jobs: Vec<WorkspaceTableSeed>,
    pub active_document_id: Option<String>,
    pub layout: Layout,
    pub density: WorkspaceDensity,
    pub navigator_width: f32,
}

impl Default for WorkspaceSnapshot {
    fn default() -> Self {
        Self {
            documents: Vec::new(),
            copy_jobs: Vec::new(),
            seed_jobs: Vec::new(),
            active_document_id: None,
            layout: Layout::Stacked,
            density: WorkspaceDensity::default(),
            navigator_width: 240.0,
        }
    }
}

/// Opaque commit identity. A reset creates a new identity, so an older writer
/// cannot revive discarded documents. None is valid only for a missing record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRevision(String);

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceLoad {
    pub revision: Option<WorkspaceRevision>,
    pub snapshot: Option<WorkspaceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    NotDevelopmentProfile,
    Closing,
    Storage,
    Corrupt,
    UnsupportedVersion(u64),
    TooLarge,
    InvalidSnapshot,
    StaleRevision,
    ConfirmationRequired,
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotDevelopmentProfile => "Workspace requires a stage04 development profile",
            Self::Closing => "Native backend is closing; drafts were not saved",
            Self::Storage => "Workspace storage failed; retry before closing",
            Self::Corrupt => "Saved workspace is unreadable; export or explicitly reset it",
            Self::UnsupportedVersion(_) => {
                "Saved workspace version is unsupported; export or explicitly reset it"
            }
            Self::TooLarge => "Workspace exceeds its byte budget; drafts were not saved",
            Self::InvalidSnapshot => "Workspace contains invalid documents; drafts were not saved",
            Self::StaleRevision => "Workspace changed since it was loaded; drafts were not saved",
            Self::ConfirmationRequired => {
                "Reset requires confirmation that saved drafts will be discarded"
            }
        })
    }
}

impl std::error::Error for WorkspaceError {}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredWorkspace {
    version: u64,
    snapshot: WorkspaceSnapshot,
}

impl Backend {
    pub async fn load_development_workspace(&self) -> Result<WorkspaceLoad, WorkspaceError> {
        self.workspace_authority()?;
        self.call(|state| async move { Ok(load(&state.pool).await) })
            .await
            .map_err(|_| WorkspaceError::Closing)?
    }

    /// Success is returned only after the SQLite commit. The caller owns one
    /// serialized, debounced writer and advances its revision only on success.
    pub async fn save_development_workspace(
        &self,
        expected: Option<WorkspaceRevision>,
        snapshot: WorkspaceSnapshot,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        self.workspace_authority()?;
        self.call(move |state| async move { Ok(save(&state.pool, expected, snapshot).await) })
            .await
            .map_err(|_| WorkspaceError::Closing)?
    }

    /// Returns original bytes as text, including corrupt or unsupported JSON.
    /// Values beyond the existing 512 KiB storage cap require offline recovery.
    pub async fn export_development_workspace(&self) -> Result<Option<String>, WorkspaceError> {
        self.workspace_authority()?;
        self.call(|state| async move {
            Ok(async {
                let mut conn = state
                    .pool
                    .acquire()
                    .await
                    .map_err(|_| WorkspaceError::Storage)?;
                read_record(&mut conn, crate::storage::UI_STATE_MAX_VALUE_BYTES)
                    .await
                    .map(|record| record.map(|(value, _)| value))
            }
            .await)
        })
        .await
        .map_err(|_| WorkspaceError::Closing)?
    }

    /// Explicit recovery replaces the record with an empty versioned workspace;
    /// it never deletes connection metadata or credentials.
    pub async fn reset_development_workspace(
        &self,
        confirmed_draft_loss: bool,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        self.workspace_authority()?;
        if !confirmed_draft_loss {
            return Err(WorkspaceError::ConfirmationRequired);
        }
        self.call(|state| async move { Ok(reset(&state.pool).await) })
            .await
            .map_err(|_| WorkspaceError::Closing)?
    }

    fn workspace_authority(&self) -> Result<(), WorkspaceError> {
        if self.0.development.is_none() {
            return Err(WorkspaceError::NotDevelopmentProfile);
        }
        Ok(())
    }
}

fn validate(mut snapshot: WorkspaceSnapshot) -> Result<WorkspaceSnapshot, WorkspaceError> {
    if snapshot.documents.len() > NATIVE_WORKSPACE_MAX_DOCUMENTS {
        return Err(WorkspaceError::InvalidSnapshot);
    }
    if snapshot.copy_jobs.len() > WORKSPACE_COPY_MAX_JOBS
        || snapshot.copy_jobs.capacity() > WORKSPACE_COPY_MAX_JOBS
    {
        return Err(WorkspaceError::InvalidSnapshot);
    }
    let mut attempts = HashSet::new();
    for job in &snapshot.copy_jobs {
        if !attempts.insert(job.attempt_id) {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        job.validate()?;
    }
    if snapshot.seed_jobs.len() > WORKSPACE_SEED_MAX_JOBS
        || snapshot.seed_jobs.capacity() > WORKSPACE_SEED_MAX_JOBS
    {
        return Err(WorkspaceError::InvalidSnapshot);
    }
    let mut attempts = HashSet::new();
    for job in &snapshot.seed_jobs {
        if !attempts.insert(job.attempt_id) {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        job.validate()?;
    }
    let mut ids = HashSet::new();
    for document in &mut snapshot.documents {
        if document.id.is_empty()
            || document.id.len() > 128
            || !ids.insert(document.id.clone())
            || document.name.is_empty()
            || document.name.len() > 512
            || document
                .connection_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 128)
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        if document
            .saved_query_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256)
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        if document.tool.is_some() && (document.table.is_some() || !document.sql.is_empty()) {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        // Only the tool tab identity/binding is durable. Job inputs, paths,
        // reviews and running state belong to the current app session.
        if matches!(
            document.tool,
            Some(
                WorkspaceTool::BackupRestore
                    | WorkspaceTool::CsvTransfer
                    | WorkspaceTool::TableCopy
                    | WorkspaceTool::TableSeed
                    | WorkspaceTool::SchemaCompare
                    | WorkspaceTool::SchemaMap
            )
        ) && document.saved_query_id.is_some()
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        if let Some(query) = &document.query_changes {
            if document.table.is_some() || document.tool.is_some() {
                return Err(WorkspaceError::InvalidSnapshot);
            }
            query.validate()?;
        }
        if let Some(ddl) = &document.table_ddl {
            if document.tool != Some(WorkspaceTool::Objects)
                || document.connection_id.is_none()
                || document.table.is_some()
                || document.schema_changes.is_some()
                || document.maintenance.is_some()
                || document.query_changes.is_some()
                || document.admin_control.is_some()
                || document.saved_query_id.is_some()
            {
                return Err(WorkspaceError::InvalidSnapshot);
            }
            ddl.validate()?;
        }
        if let Some(maintenance) = &document.maintenance {
            if document.tool != Some(WorkspaceTool::Objects)
                || document.connection_id.is_none()
                || document.schema_changes.is_some()
                || document.query_changes.is_some()
                || document.admin_control.is_some()
                || document.saved_query_id.is_some()
            {
                return Err(WorkspaceError::InvalidSnapshot);
            }
            maintenance.validate()?;
        }
        if let Some(control) = &document.admin_control {
            if document.tool != Some(WorkspaceTool::Administration)
                || document.connection_id.is_none()
                || document.schema_changes.is_some()
                || document.query_changes.is_some()
                || document.saved_query_id.is_some()
            {
                return Err(WorkspaceError::InvalidSnapshot);
            }
            control.validate()?;
        }
        if let Some(schema) = &document.schema_changes {
            if document.tool != Some(WorkspaceTool::Objects)
                || document.connection_id.is_none()
                || document.table.is_some()
                || document.query_changes.is_some()
                || document.saved_query_id.is_some()
            {
                return Err(WorkspaceError::InvalidSnapshot);
            }
            schema.validate()?;
        }
        if let Some(table) = &document.table {
            table.validate()?;
        }
        for offset in [&mut document.selection.anchor, &mut document.selection.head] {
            *offset = (*offset).min(document.sql.len());
            while !document.sql.is_char_boundary(*offset) {
                *offset -= 1;
            }
        }
    }
    match &snapshot.active_document_id {
        Some(id) if ids.contains(id) => {}
        None if ids.is_empty() => {}
        _ => return Err(WorkspaceError::InvalidSnapshot),
    }
    snapshot.navigator_width = if snapshot.navigator_width.is_finite() {
        snapshot.navigator_width.clamp(160.0, 480.0)
    } else {
        240.0
    };
    Ok(snapshot)
}

fn encode(snapshot: WorkspaceSnapshot) -> Result<String, WorkspaceError> {
    // Serialization itself is bounded, including JSON escape expansion.
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > NATIVE_WORKSPACE_MAX_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("workspace budget"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(
        &mut output,
        &StoredWorkspace {
            version: 13,
            snapshot: validate(snapshot)?,
        },
    )
    .map_err(|_| WorkspaceError::TooLarge)?;
    String::from_utf8(output.0).map_err(|_| WorkspaceError::InvalidSnapshot)
}

fn decode(encoded: &str) -> Result<WorkspaceSnapshot, WorkspaceError> {
    #[derive(Deserialize)]
    struct Version {
        version: u64,
    }
    let version: Version = serde_json::from_str(encoded).map_err(|_| WorkspaceError::Corrupt)?;
    if !matches!(version.version, 1..=13) {
        return Err(WorkspaceError::UnsupportedVersion(version.version));
    }
    let raw: serde_json::Value =
        serde_json::from_str(encoded).map_err(|_| WorkspaceError::Corrupt)?;
    // Reject a new field even when null in an older envelope. Loading never
    // upgrades or silently erases a recovery record.
    if version.version < 13
        && raw["snapshot"]["documents"]
            .as_array()
            .is_some_and(|documents| {
                documents.iter().any(|document| {
                    document
                        .as_object()
                        .is_some_and(|fields| fields.contains_key("tableDdl"))
                })
            })
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 10
        && raw["snapshot"]
            .as_object()
            .is_some_and(|snapshot| snapshot.contains_key("copyJobs"))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 11
        && raw["snapshot"]
            .as_object()
            .is_some_and(|snapshot| snapshot.contains_key("seedJobs"))
    {
        return Err(WorkspaceError::Corrupt);
    }
    let stored: StoredWorkspace =
        serde_json::from_str(encoded).map_err(|_| WorkspaceError::Corrupt)?;
    if version.version < 12
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.tool == Some(WorkspaceTool::SchemaMap))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 10
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.tool == Some(WorkspaceTool::TableCopy))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 11
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.tool == Some(WorkspaceTool::TableSeed))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version == 1
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.table.is_some() || document.tool.is_some())
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 3
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.query_changes.is_some())
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 4
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.schema_changes.is_some())
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 5
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.tool == Some(WorkspaceTool::BackupRestore))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 6
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.tool == Some(WorkspaceTool::CsvTransfer))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 7
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.tool == Some(WorkspaceTool::SchemaCompare))
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 8
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.admin_control.is_some())
    {
        return Err(WorkspaceError::Corrupt);
    }
    if version.version < 9
        && stored
            .snapshot
            .documents
            .iter()
            .any(|document| document.maintenance.is_some())
    {
        return Err(WorkspaceError::Corrupt);
    }
    // Shared filter/mutation DTOs predate strict durable decoding. Reject extra
    // nested fields too, so loading cannot silently discard future intent.
    let known = serde_json::to_value(&stored).map_err(|_| WorkspaceError::Corrupt)?;
    if !known_fields(&raw, &known) {
        return Err(WorkspaceError::Corrupt);
    }
    let mut snapshot = validate(stored.snapshot).map_err(|_| WorkspaceError::Corrupt)?;
    for job in &mut snapshot.copy_jobs {
        job.restore();
    }
    for job in &mut snapshot.seed_jobs {
        job.restore();
    }
    Ok(snapshot)
}

fn known_fields(raw: &serde_json::Value, known: &serde_json::Value) -> bool {
    match (raw, known) {
        (serde_json::Value::Object(raw), serde_json::Value::Object(known)) => {
            raw.iter().all(|(key, value)| {
                known
                    .get(key)
                    .is_some_and(|known| known_fields(value, known))
            })
        }
        (serde_json::Value::Array(raw), serde_json::Value::Array(known)) => {
            raw.len() == known.len()
                && raw
                    .iter()
                    .zip(known)
                    .all(|(raw, known)| known_fields(raw, known))
        }
        _ => true,
    }
}

async fn read_record(
    conn: &mut SqliteConnection,
    maximum: usize,
) -> Result<Option<(String, WorkspaceRevision)>, WorkspaceError> {
    // SQLite measures the entire BLOB, but never transfers an oversized value
    // into Rust. Read length and content in one statement to avoid a TOCTOU read.
    let row: Option<(i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT length(CAST(value AS BLOB)),
                CASE WHEN length(CAST(value AS BLOB)) <= ? THEN value END,
                CASE WHEN length(CAST(updated_at AS BLOB)) <= 128 THEN updated_at END
         FROM ui_state WHERE key = ?",
    )
    .bind(maximum as i64)
    .bind(KEY)
    .fetch_optional(conn)
    .await
    .map_err(|_| WorkspaceError::Storage)?;
    row.map(|(length, value, revision)| {
        if length > maximum as i64 {
            return Err(WorkspaceError::TooLarge);
        }
        Ok((
            value.ok_or(WorkspaceError::Corrupt)?,
            WorkspaceRevision(revision.ok_or(WorkspaceError::Corrupt)?),
        ))
    })
    .transpose()
}

async fn load(pool: &SqlitePool) -> Result<WorkspaceLoad, WorkspaceError> {
    let mut conn = pool.acquire().await.map_err(|_| WorkspaceError::Storage)?;
    let record = read_record(&mut conn, NATIVE_WORKSPACE_MAX_BYTES).await?;
    match record {
        None => Ok(WorkspaceLoad {
            revision: None,
            snapshot: None,
        }),
        Some((encoded, revision)) => Ok(WorkspaceLoad {
            revision: Some(revision),
            snapshot: Some(decode(&encoded)?),
        }),
    }
}

async fn save(
    pool: &SqlitePool,
    expected: Option<WorkspaceRevision>,
    snapshot: WorkspaceSnapshot,
) -> Result<WorkspaceRevision, WorkspaceError> {
    let encoded = encode(snapshot)?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| WorkspaceError::Storage)?;
    let current = read_record(&mut tx, NATIVE_WORKSPACE_MAX_BYTES).await?;
    if let Some((value, _)) = &current {
        decode(value)?;
    }
    if current.map(|(_, revision)| revision) != expected {
        return Err(WorkspaceError::StaleRevision);
    }
    let revision = write_record(&mut tx, encoded).await?;
    tx.commit().await.map_err(|_| WorkspaceError::Storage)?;
    Ok(revision)
}

async fn reset(pool: &SqlitePool) -> Result<WorkspaceRevision, WorkspaceError> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| WorkspaceError::Storage)?;
    let revision = write_record(&mut tx, encode(WorkspaceSnapshot::default())?).await?;
    tx.commit().await.map_err(|_| WorkspaceError::Storage)?;
    Ok(revision)
}

async fn write_record(
    conn: &mut SqliteConnection,
    encoded: String,
) -> Result<WorkspaceRevision, WorkspaceError> {
    // updated_at is an opaque commit identity for this private namespace. A UUID
    // avoids timestamp collisions even when two saves commit in one clock tick.
    let revision = WorkspaceRevision(uuid::Uuid::new_v4().to_string());
    sqlx::query(
        "INSERT INTO ui_state (key, value, updated_at) VALUES (?, ?, ?)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(KEY)
    .bind(encoded)
    .bind(&revision.0)
    .execute(conn)
    .await
    .map_err(|_| WorkspaceError::Storage)?;
    Ok(revision)
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workspace_query_tests.rs"]
mod query_tests;

#[cfg(test)]
#[path = "workspace_maintenance_tests.rs"]
mod maintenance_tests;

#[cfg(test)]
#[path = "workspace_table_ddl_tests.rs"]
mod table_ddl_tests;
