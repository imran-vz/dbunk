//! One bounded, debounced draft writer. Acknowledgements identify the exact
//! submitted revision; a failed save never turns a newer draft into Saved.
use dbunk_lib::backend::{
    Backend, NATIVE_WORKSPACE_MAX_BYTES, WorkspaceError, WorkspaceRevision, WorkspaceSnapshot,
};
use serde::Serialize;
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, watch};

// Covers bounded UI materialization, pending/current worker snapshots and the
// encoded SQLite payload. The workspace admits this before retaining results.
pub const WORKSPACE_SAVE_ALLOWANCE: usize = 8 * 1024 * 1024;

/// Export may exceed the SQLite limit. Admit its immutable snapshot before
/// cloning, and keep the lease in the completion waiter until the owned job joins.
pub struct ExportSnapshotLease {
    budget: std::rc::Rc<std::cell::Cell<usize>>,
    bytes: usize,
}
impl ExportSnapshotLease {
    pub fn admit(
        payload_bytes: usize,
        budget: std::rc::Rc<std::cell::Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let bytes = payload_bytes
            .saturating_mul(4)
            .saturating_add(4 * 1024 * 1024);
        if bytes > (128usize * 1024 * 1024).saturating_sub(budget.get()) {
            return Err(
                "Workspace export needs more shared allowance; clear results or close another tool, then retry",
            );
        }
        budget.set(budget.get() + bytes);
        Ok(Self { budget, bytes })
    }
}
impl Drop for ExportSnapshotLease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

/// Hot-path edits (keystrokes, table cell drafts) are coalesced into one
/// snapshot per window instead of one per event; the writer then debounces
/// the SQLite commit. Structural changes and exact barriers save immediately.
pub const DRAFT_COALESCE: Duration = Duration::from_millis(250);

/// Pure coalescing state; the workspace owns the timer task. While an exact
/// apply/copy/seed barrier is in flight a save would supersede its revision
/// and fail it as stale, so a due save waits for the barrier to finish.
#[derive(Debug, Default)]
pub struct SaveCoalescer {
    dirty: bool,
    armed: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CoalescedSave {
    /// Build and submit one snapshot now.
    Save,
    /// Still dirty but a barrier is in flight; re-arm the timer.
    Wait,
    /// Nothing pending (an immediate save already captured it).
    Idle,
}
impl SaveCoalescer {
    /// Records a change. Returns true when the caller must arm a timer.
    pub fn mark(&mut self) -> bool {
        self.dirty = true;
        !std::mem::replace(&mut self.armed, true)
    }
    /// The timer elapsed. `busy` is true while an exact barrier is in flight.
    pub fn fire(&mut self, busy: bool) -> CoalescedSave {
        if !self.dirty {
            self.armed = false;
            return CoalescedSave::Idle;
        }
        if busy {
            return CoalescedSave::Wait;
        }
        self.dirty = false;
        self.armed = false;
        CoalescedSave::Save
    }
    /// An immediate save snapshotted everything marked so far.
    pub fn saved(&mut self) {
        self.dirty = false;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    Pending,
    Saved,
    Failed(WorkspaceError),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveState {
    pub revision: u64,
    pub state: SaveStatus,
}
#[derive(Clone)]
struct Pending {
    revision: u64,
    snapshot: Result<WorkspaceSnapshot, WorkspaceError>,
}

pub struct DraftWriter {
    latest: watch::Sender<Option<Pending>>,
    status: watch::Receiver<SaveState>,
    revision: AtomicU64,
    flush: Arc<Notify>,
    stop: watch::Sender<bool>,
    join: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    submission: Mutex<bool>,
}
impl DraftWriter {
    pub fn new(
        backend: Backend,
        runtime: tokio::runtime::Handle,
        revision: Option<WorkspaceRevision>,
    ) -> Arc<Self> {
        let (latest, input) = watch::channel(None);
        let (status, output) = watch::channel(SaveState {
            revision: 0,
            state: SaveStatus::Saved,
        });
        let (stop, stopping) = watch::channel(false);
        let flush = Arc::new(Notify::new());
        let wake = flush.clone();
        let join = runtime.spawn(run(backend, revision, input, status, wake, stopping));
        Arc::new(Self {
            latest,
            status: output,
            revision: AtomicU64::new(0),
            flush,
            stop,
            join: tokio::sync::Mutex::new(Some(join)),
            submission: Mutex::new(false),
        })
    }
    #[cfg(test)]
    pub fn submit(&self, snapshot: WorkspaceSnapshot) -> Result<u64, WorkspaceError> {
        self.submit_prepared(Ok(snapshot))
    }
    pub fn submit_prepared(
        &self,
        snapshot: Result<WorkspaceSnapshot, WorkspaceError>,
    ) -> Result<u64, WorkspaceError> {
        let stopped = self.submission.lock().unwrap();
        if *stopped {
            return Err(WorkspaceError::Closing);
        }
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let checked = snapshot.and_then(|snapshot| within_budget(&snapshot).map(|()| snapshot));
        let result = checked.as_ref().map(|_| revision).map_err(Clone::clone);
        self.latest.send_replace(Some(Pending {
            revision,
            snapshot: checked,
        }));
        result
    }
    pub fn status(&self) -> watch::Receiver<SaveState> {
        self.status.clone()
    }
    /// Flushes the latest revision at entry. UI freezes close-related changes
    /// before calling this; ordinary edits can continue during an explicit save.
    pub async fn flush(&self) -> Result<(), WorkspaceError> {
        let target = self.revision.load(Ordering::SeqCst);
        wait_for_revision(self.status.clone(), self.flush.clone(), target, false).await
    }
    /// Apply requires this exact snapshot's acknowledged SQLite commit. A
    /// coalesced newer revision safely refuses the barrier instead of assuming
    /// it still contains the same pending apply. Ordinary saves may coalesce.
    pub async fn flush_revision(&self, target: u64) -> Result<(), WorkspaceError> {
        if *self.submission.lock().unwrap() {
            return Err(WorkspaceError::Closing);
        }
        if target == 0 || target > self.revision.load(Ordering::SeqCst) {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        wait_for_revision(self.status.clone(), self.flush.clone(), target, true).await
    }
    pub fn retry(&self) {
        self.flush.notify_one();
    }
    /// Caller must resolve failures before shutdown. After stopping admission,
    /// this still flushes the final revision and joins the single owned task.
    pub async fn shutdown(&self) -> Result<(), WorkspaceError> {
        *self.submission.lock().unwrap() = true;
        let result = self.flush().await;
        self.stop.send_replace(true);
        if let Some(join) = self.join.lock().await.take() {
            join.await.map_err(|_| WorkspaceError::Closing)?;
        }
        result
    }
    /// Only after explicit discard. Stops future writes; an already committing
    /// snapshot is allowed to settle before its task is joined.
    pub async fn discard_and_shutdown(&self) -> Result<(), WorkspaceError> {
        *self.submission.lock().unwrap() = true;
        self.stop.send_replace(true);
        if let Some(join) = self.join.lock().await.take() {
            join.await.map_err(|_| WorkspaceError::Closing)?;
        }
        Ok(())
    }
}

async fn wait_for_revision(
    mut status: watch::Receiver<SaveState>,
    flush: Arc<Notify>,
    target: u64,
    exact: bool,
) -> Result<(), WorkspaceError> {
    let initial = status.borrow_and_update().clone();
    if exact && initial.revision > target {
        return Err(WorkspaceError::StaleRevision);
    }
    if initial.revision >= target && initial.state == SaveStatus::Saved {
        return Ok(());
    }
    let retrying = matches!(initial.state, SaveStatus::Failed(_));
    flush.notify_one();
    if retrying {
        status
            .changed()
            .await
            .map_err(|_| WorkspaceError::Closing)?;
    }
    loop {
        let current = status.borrow_and_update().clone();
        if exact && current.revision > target {
            return Err(WorkspaceError::StaleRevision);
        }
        if current.revision >= target {
            match current.state {
                SaveStatus::Saved => return Ok(()),
                SaveStatus::Failed(error) => return Err(error),
                SaveStatus::Pending => {}
            }
        }
        status
            .changed()
            .await
            .map_err(|_| WorkspaceError::Closing)?;
    }
}

fn within_budget(snapshot: &WorkspaceSnapshot) -> Result<(), WorkspaceError> {
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > NATIVE_WORKSPACE_MAX_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other("draft budget"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[derive(Serialize)]
    struct Envelope<'a> {
        version: u32,
        snapshot: &'a WorkspaceSnapshot,
    }
    serde_json::to_writer(
        Count(0),
        &Envelope {
            // Matches the backend envelope; only its encoded width matters.
            version: 16,
            snapshot,
        },
    )
    .map_err(|_| WorkspaceError::TooLarge)
}

async fn run(
    backend: Backend,
    mut committed: Option<WorkspaceRevision>,
    mut input: watch::Receiver<Option<Pending>>,
    status: watch::Sender<SaveState>,
    flush: Arc<Notify>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        let forced = tokio::select! {
            biased;
            _ = stop.changed() => return,
            _ = flush.notified() => true,
            changed = input.changed() => { if changed.is_err() { return; } false }
        };
        if *stop.borrow() {
            return;
        }
        if !forced {
            // Reset the debounce while edits arrive, retaining only the latest.
            loop {
                let value = input.borrow_and_update().clone();
                if let Some(value) = value {
                    status.send_replace(SaveState {
                        revision: value.revision,
                        state: SaveStatus::Pending,
                    });
                }
                tokio::select! {
                    biased;
                    _ = stop.changed() => return,
                    _ = flush.notified() => break,
                    changed = input.changed() => if changed.is_err() { return; },
                    _ = tokio::time::sleep(Duration::from_millis(500)) => break,
                }
            }
        }
        let Some(pending) = input.borrow_and_update().clone() else {
            continue;
        };
        status.send_replace(SaveState {
            revision: pending.revision,
            state: SaveStatus::Pending,
        });
        let result = match pending.snapshot {
            Ok(snapshot) => {
                backend
                    .save_development_workspace(committed.clone(), snapshot)
                    .await
            }
            Err(error) => Err(error),
        };
        let state = match result {
            Ok(revision) => {
                committed = Some(revision);
                SaveStatus::Saved
            }
            Err(error) => SaveStatus::Failed(error),
        };
        // A stale acknowledgement remains explicitly attached to its revision.
        status.send_replace(SaveState {
            revision: pending.revision,
            state,
        });
    }
}

/// Structured intent requires a complete workspace export, including when an
/// Objects tab is the only document and its SQL editor text is empty.
pub fn needs_workspace_export(snapshot: &WorkspaceSnapshot) -> bool {
    !snapshot.copy_jobs.is_empty()
        || !snapshot.seed_jobs.is_empty()
        || snapshot.documents.iter().any(|document| {
            document.table.is_some()
                || document.query_changes.is_some()
                || document.schema_changes.is_some()
                || document.table_ddl.is_some()
                || document.schema_alter.is_some()
                || document.object_ddl.is_some()
                || document.admin_control.is_some()
                || document.maintenance.is_some()
        })
}

/// Run on the blocking executor after a native save dialog. Export keeps all
/// current SQL, even when the workspace snapshot exceeds its storage budget.
pub fn export_sql(snapshot: &WorkspaceSnapshot, path: &std::path::Path) -> Result<(), String> {
    if needs_workspace_export(snapshot) {
        return Err(
            "This workspace contains structured changes; export workspace JSON to preserve them"
                .into(),
        );
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Choose a new writable SQL file; existing files are preserved".to_string())?;
    for (index, document) in snapshot.documents.iter().enumerate() {
        // Do not interpolate document names into SQL comments: names may contain
        // newlines or terminators. Numeric delimiters keep export unambiguous.
        writeln!(file, "-- Query document {}\n", index + 1)
            .map_err(|_| "SQL export failed".to_string())?;
        file.write_all(document.sql.as_bytes())
            .map_err(|_| "SQL export failed".to_string())?;
        file.write_all(b"\n\n")
            .map_err(|_| "SQL export failed".to_string())?;
    }
    file.sync_all()
        .map_err(|_| "SQL export could not be made durable".to_string())
}

/// Mixed workspaces need JSON to preserve table filters and pending changes.
/// Stream the in-memory snapshot, even if it exceeds the SQLite save budget.
pub fn export_workspace(
    snapshot: &WorkspaceSnapshot,
    path: &std::path::Path,
) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Choose a new writable JSON file; existing files are preserved".to_string())?;
    serde_json::to_writer(&mut file, snapshot)
        .map_err(|_| "Workspace export failed".to_string())?;
    file.sync_all()
        .map_err(|_| "Workspace export could not be made durable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::{DevelopmentFixtures, WorkspaceDocument, WorkspaceSelection};

    #[test]
    fn coalescer_builds_one_snapshot_per_window_and_defers_behind_barriers() {
        let mut saves = SaveCoalescer::default();
        assert!(saves.mark(), "first change arms the timer");
        for _ in 0..50 {
            assert!(!saves.mark(), "a burst shares the armed timer");
        }
        assert_eq!(saves.fire(false), CoalescedSave::Save);
        assert_eq!(saves.fire(false), CoalescedSave::Idle);

        // A change during an apply barrier must not supersede its revision.
        assert!(saves.mark());
        assert_eq!(saves.fire(true), CoalescedSave::Wait);
        assert!(!saves.mark(), "still armed while waiting");
        assert_eq!(saves.fire(true), CoalescedSave::Wait);
        assert_eq!(saves.fire(false), CoalescedSave::Save);

        // An immediate save (apply, close, quit) captures pending edits.
        assert!(saves.mark());
        saves.saved();
        assert_eq!(saves.fire(false), CoalescedSave::Idle);
        assert!(saves.mark(), "the idle timer ended, so a new change re-arms");
    }

    #[test]
    fn export_snapshot_refuses_before_copy_and_releases_only_its_own_allowance() {
        use std::{cell::Cell, rc::Rc};
        let budget = Rc::new(Cell::new(WORKSPACE_SAVE_ALLOWANCE));
        let lease = ExportSnapshotLease::admit(1024, budget.clone()).unwrap();
        let held = budget.get();
        assert!(ExportSnapshotLease::admit(usize::MAX, budget.clone()).is_err());
        assert_eq!(budget.get(), held);
        budget.set(budget.get() + 100);
        drop(lease);
        assert_eq!(budget.get(), WORKSPACE_SAVE_ALLOWANCE + 100);
    }

    #[tokio::test]
    async fn apply_barrier_waits_for_its_exact_commit_and_refuses_superseded_ack() {
        let (send, status) = watch::channel(SaveState {
            revision: 1,
            state: SaveStatus::Saved,
        });
        let flush = Arc::new(Notify::new());
        let mut waiting = tokio::spawn(wait_for_revision(status.clone(), flush.clone(), 2, true));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err(),
            "an older Saved revision must not authorize apply"
        );
        send.send_replace(SaveState {
            revision: 2,
            state: SaveStatus::Pending,
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err(),
            "queued is not durable"
        );
        send.send_replace(SaveState {
            revision: 2,
            state: SaveStatus::Saved,
        });
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        send.send_replace(SaveState {
            revision: 3,
            state: SaveStatus::Saved,
        });
        assert_eq!(
            wait_for_revision(status.clone(), flush.clone(), 2, true).await,
            Err(WorkspaceError::StaleRevision)
        );
        assert_eq!(
            wait_for_revision(status, flush, 2, false).await,
            Ok(()),
            "ordinary saves retain coalescing semantics"
        );
    }

    #[tokio::test]
    async fn apply_barrier_reports_storage_refusal_and_lost_writer() {
        let (send, status) = watch::channel(SaveState {
            revision: 1,
            state: SaveStatus::Pending,
        });
        let mut waiting = tokio::spawn(wait_for_revision(
            status.clone(),
            Arc::new(Notify::new()),
            1,
            true,
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        send.send_replace(SaveState {
            revision: 1,
            state: SaveStatus::Failed(WorkspaceError::Storage),
        });
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap(),
            Err(WorkspaceError::Storage)
        );
        drop(send);
        assert_eq!(
            wait_for_revision(status, Arc::new(Notify::new()), 2, true).await,
            Err(WorkspaceError::Closing)
        );
    }

    #[test]
    fn mixed_export_preserves_table_intent_and_oversize_sql_without_overwriting() {
        let mut draft = snapshot(&"-- exact 🙂\n".repeat(NATIVE_WORKSPACE_MAX_BYTES / 8));
        draft.documents.push(WorkspaceDocument {
            query_changes: None,
            schema_changes: None,
            table_ddl: None,
            schema_alter: None,
            object_ddl: None,
            admin_control: None,
            maintenance: None,
            tool: None,
            saved_query_id: None,
            id: "table".into(),
            name: "public.items".into(),
            connection_id: Some("removed-connection".into()),
            sql: String::new(),
            pinned: false,
            selection: WorkspaceSelection::default(),
            table: Some(dbunk_lib::backend::WorkspaceTableState {
                schema: "public".into(),
                table: "items".into(),
                filters: vec![dbunk_lib::backend::data::BrowseFilter::RawSql {
                    text: "name = '東京'".into(),
                }],
                sort: vec![],
                page_size: 25,
                draft: None,
            }),
        });
        let path = std::env::temp_dir().join(format!(
            "dbunk-workspace-export-{}.json",
            uuid::Uuid::new_v4()
        ));
        export_workspace(&draft, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > NATIVE_WORKSPACE_MAX_BYTES);
        let restored: WorkspaceSnapshot = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, draft);
        assert!(export_workspace(&WorkspaceSnapshot::default(), &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }

    fn snapshot(sql: &str) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            documents: vec![WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                tool: None,
                saved_query_id: None,
                table: None,
                id: "draft".into(),
                name: "Draft".into(),
                connection_id: None,
                sql: sql.into(),
                pinned: false,
                selection: WorkspaceSelection::default(),
            }],
            active_document_id: Some("draft".into()),
            ..WorkspaceSnapshot::default()
        }
    }

    #[test]
    fn table_change_recovery_requires_structured_export_and_round_trips_exactly() {
        use dbunk_lib::backend::table_ddl::*;
        let mut draft = snapshot("");
        let document = &mut draft.documents[0];
        document.tool = Some(dbunk_lib::backend::WorkspaceTool::Objects);
        document.connection_id = Some("owned-connection".into());
        document.table_ddl = Some(dbunk_lib::backend::WorkspaceTableDdl {
            attempt_id: TableDdlAttemptId::new(),
            target: TableDdlDescription {
                identity: TableIdentity {
                    database_oid: 1,
                    relation_oid: 2,
                },
                schema_oid: 3,
                schema: "a.b".into(),
                table: "rows".into(),
                namespace_xmin: "4".into(),
                namespace_ctid: "(0,1)".into(),
                column: Some(TableDdlColumn {
                    attnum: 7,
                    name: "value".into(),
                }),
                comment: Some("original comment".into()),
            },
            intent: TableDdlIntent::SetComment { comment: None },
            preview: TableDdlPreview {
                sql: "COMMENT ON COLUMN \"a.b\".\"rows\".\"value\" IS NULL;".into(),
                summary: "Clear comment on COLUMN \"a.b\".\"rows\".\"value\"".into(),
                statement_timeout_ms: Some(0),
                operation_timeout_ms: TABLE_DDL_OPERATION_TIMEOUT_MS,
            },
            apply_state: dbunk_lib::backend::WorkspaceApplyState::OutcomeUnknown,
        });
        document.table_ddl.as_ref().unwrap().validate().unwrap();
        assert!(needs_workspace_export(&draft));
        let path = std::env::temp_dir().join(format!(
            "dbunk-table-ddl-export-{}.json",
            uuid::Uuid::new_v4()
        ));
        assert!(export_sql(&draft, &path).is_err());
        assert!(!path.exists());
        export_workspace(&draft, &path).unwrap();
        let restored: WorkspaceSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, draft);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn schema_only_recovery_exports_exact_intent_and_refuses_lossy_sql() {
        use dbunk_lib::backend::schema_ddl::{CreateSchemaAttemptId, CreateSchemaIntent};
        let mut draft = snapshot("");
        let document = &mut draft.documents[0];
        document.tool = Some(dbunk_lib::backend::WorkspaceTool::Objects);
        document.connection_id = Some("owned-connection".into());
        document.schema_changes = Some(dbunk_lib::backend::WorkspaceSchemaChanges {
            attempt_id: CreateSchemaAttemptId::new(),
            intent: CreateSchemaIntent::new(
                "資料".into(),
                Some("exact ' comment\nnext line".into()),
            )
            .unwrap(),
            apply_state: dbunk_lib::backend::WorkspaceApplyState::OutcomeUnknown,
        });
        assert!(needs_workspace_export(&draft));
        let path =
            std::env::temp_dir().join(format!("dbunk-schema-export-{}.json", uuid::Uuid::new_v4()));
        assert!(export_sql(&draft, &path).is_err());
        assert!(!path.exists());
        export_workspace(&draft, &path).unwrap();
        let restored: WorkspaceSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, draft);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn copy_recovery_without_any_tab_requires_complete_export() {
        let endpoint =
            serde_json::json!({"connectionId":"removed", "schema":"資料", "table":"rows"});
        let connection = serde_json::json!({"connectionName":"owned", "host":"localhost", "port":15432,
            "database":"fixture", "user":"fixture", "environment":"Development", "safeMode":"Protected", "readOnly":false});
        let relation = serde_json::json!({"databaseOid":1,"relationOid":2,"kind":"r"});
        let record = serde_json::from_value(serde_json::json!({
            "attemptId":uuid::Uuid::new_v4().to_string(),
            "description": {"intent":{"source":endpoint,"destination":endpoint},
                "sourceConnection":connection,"destinationConnection":connection,
                "sourceRelation":relation,"destinationRelation":relation,"mappingSha256":"a".repeat(64),
                "copiedColumns":1,"defaultedColumns":0,"generatedColumns":0,"identityColumns":0},
            "state":"unknown"
        })).unwrap();
        let draft = WorkspaceSnapshot {
            copy_jobs: vec![record],
            ..Default::default()
        };
        let path =
            std::env::temp_dir().join(format!("dbunk-copy-export-{}.json", uuid::Uuid::new_v4()));
        assert!(needs_workspace_export(&draft));
        assert!(export_sql(&draft, &path).is_err());
        assert!(!path.exists());
        export_workspace(&draft, &path).unwrap();
        let restored: WorkspaceSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, draft);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn seed_recovery_without_a_tab_exports_exact_identity_instead_of_empty_sql() {
        let record = serde_json::from_value(serde_json::json!({
            "attemptId":uuid::Uuid::new_v4().to_string(),
            "description": {
                "endpoint":{"connectionId":"removed","schema":"資料","table":"rows"},
                "connection":{"connectionName":"owned","host":"localhost","port":15432,
                    "database":"fixture","user":"fixture","environment":"Development","safeMode":"Protected","readOnly":false},
                "databaseOid":1,"relationOid":2,"rowCount":100,"seedUsed":u64::MAX,
                "clockEpochSeconds":1700000000,"recipeSha256":"a".repeat(64),
                "recipeSummary":"value: constant 雪","recipeSummaryTruncated":false,
                "insertedColumns":1,"defaultedColumns":0
            },"state":"unknown"
        })).unwrap();
        let draft = WorkspaceSnapshot {
            seed_jobs: vec![record],
            ..Default::default()
        };
        let path =
            std::env::temp_dir().join(format!("dbunk-seed-export-{}.json", uuid::Uuid::new_v4()));
        assert!(needs_workspace_export(&draft));
        assert!(export_sql(&draft, &path).is_err());
        assert!(!path.exists());
        export_workspace(&draft, &path).unwrap();
        let restored: WorkspaceSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, draft);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn maintenance_recovery_export_preserves_partial_effects_without_sql_replay() {
        let mut draft = snapshot("");
        let document = &mut draft.documents[0];
        document.tool = Some(dbunk_lib::backend::WorkspaceTool::Objects);
        document.connection_id = Some("owned".into());
        document.maintenance = Some(dbunk_lib::backend::WorkspaceMaintenance {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            action: dbunk_lib::backend::WorkspaceMaintenanceAction::Vacuum,
            database_oid: 12,
            database: "owned".into(),
            namespace_oid: 34,
            schema: "資料".into(),
            relation_oid: 56,
            name: "rows".into(),
            kind: dbunk_lib::backend::WorkspaceMaintenanceKind::Table,
            sql: "VACUUM \"資料\".\"rows\"".into(),
            potentially_partial: true,
            operation_timeout_ms: 300_000,
            statement_timeout_ms: None,
            state: dbunk_lib::backend::WorkspaceMaintenanceState::EffectsPossible,
        });
        document.maintenance.as_ref().unwrap().validate().unwrap();
        let path = std::env::temp_dir().join(format!(
            "dbunk-maintenance-export-{}.json",
            uuid::Uuid::new_v4()
        ));
        assert!(needs_workspace_export(&draft));
        assert!(export_sql(&draft, &path).is_err());
        assert!(!path.exists());
        export_workspace(&draft, &path).unwrap();
        let restored: WorkspaceSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, draft);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn admin_only_recovery_refuses_lossy_sql_export_and_preserves_exact_identity() {
        let mut draft = snapshot("");
        let document = &mut draft.documents[0];
        document.tool = Some(dbunk_lib::backend::WorkspaceTool::Administration);
        document.connection_id = Some("owned".into());
        document.admin_control = Some(dbunk_lib::backend::WorkspaceAdminControl {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            action: dbunk_lib::backend::WorkspaceAdminAction::TerminateSession,
            pid: 12345,
            backend_start: "2026-10-03T01:02:03.123456Z".into(),
            query_start: None,
            database: Some("資料".into()),
            apply_state: dbunk_lib::backend::WorkspaceApplyState::OutcomeUnknown,
        });
        let path =
            std::env::temp_dir().join(format!("dbunk-admin-export-{}.json", uuid::Uuid::new_v4()));
        assert!(needs_workspace_export(&draft));
        assert!(export_sql(&draft, &path).is_err());
        assert!(!path.exists());
        export_workspace(&draft, &path).unwrap();
        let restored: WorkspaceSnapshot =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored, draft);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn writer_coalesces_flushes_recovers_and_refuses_stale_overwrite() {
        const CHILD: &str = "DBUNK_DRAFT_WRITER_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "persistence::tests::writer_coalesces_flushes_recovers_and_refuses_stale_overwrite", "--nocapture"])
                .env(CHILD, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        // Metadata only: no credentials, sockets, or OS Keychain are opened.
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("dbunk-draft-writer-{}", uuid::Uuid::new_v4()));
        let fixtures = DevelopmentFixtures::from_json(&serde_json::json!({
            "version":1,"fixture":"dbunk-native-stage03","instance":"2283820d-33ec-4c4c-ae03-7051092bd410",
            "host":"127.0.0.1","port":15432,"database":"dbunk_demo","user":"dbunk"
        }).to_string()).unwrap();
        let backend = Backend::create_development(&path, fixtures).await.unwrap();
        let writer = DraftWriter::new(backend.clone(), tokio::runtime::Handle::current(), None);
        for index in 0..30 {
            writer
                .submit(snapshot(&format!("SELECT {index};")))
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), writer.flush())
            .await
            .unwrap()
            .unwrap();
        let committed = backend.load_development_workspace().await.unwrap();
        assert_eq!(committed.snapshot, Some(snapshot("SELECT 29;")));
        assert_eq!(writer.status().borrow().revision, 30);
        assert_eq!(
            writer.submit_prepared(Err(WorkspaceError::TooLarge)),
            Err(WorkspaceError::TooLarge)
        );
        assert_eq!(
            writer.flush_revision(31).await,
            Err(WorkspaceError::TooLarge)
        );
        assert_eq!(
            backend.load_development_workspace().await.unwrap().snapshot,
            committed.snapshot
        );
        assert_eq!(
            writer.submit(snapshot(&"\0".repeat(NATIVE_WORKSPACE_MAX_BYTES))),
            Err(WorkspaceError::TooLarge)
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), writer.flush())
                .await
                .unwrap(),
            Err(WorkspaceError::TooLarge)
        );
        assert_eq!(
            backend.load_development_workspace().await.unwrap(),
            committed
        );
        let exact = snapshot("SELECT '東京🙂';\n-- trailing spaces  \n");
        writer.submit(exact.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), writer.flush())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            backend.load_development_workspace().await.unwrap().snapshot,
            Some(exact)
        );
        let mut applying = snapshot("pending apply, not SQL execution");
        applying.documents[0].table = Some(dbunk_lib::backend::WorkspaceTableState {
            schema: "public".into(),
            table: "items".into(),
            filters: vec![],
            sort: vec![],
            page_size: 100,
            draft: Some(dbunk_lib::backend::WorkspaceMutationDraft {
                apply_state: dbunk_lib::backend::WorkspaceApplyState::OutcomeUnknown,
                changes: vec![dbunk_lib::backend::WorkspaceStagedChange {
                    id: uuid::Uuid::new_v4().to_string(),
                    included: true,
                    identity_kind: None,
                    originals: vec![],
                    operation: dbunk_lib::backend::data::MutationOp::Insert {
                        table: dbunk_lib::backend::data::MutationTable {
                            schema: "public".into(),
                            table: "items".into(),
                        },
                        values: vec![],
                    },
                }],
            }),
        });
        let target = writer.submit(applying.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), writer.flush_revision(target))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(writer.status().borrow().revision, target);
        assert_eq!(
            backend.load_development_workspace().await.unwrap().snapshot,
            Some(applying)
        );
        let revision = backend.load_development_workspace().await.unwrap().revision;
        let external = snapshot("SELECT 'external writer';");
        backend
            .save_development_workspace(revision, external.clone())
            .await
            .unwrap();
        writer.submit(snapshot("SELECT 'stale writer';")).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), writer.flush())
                .await
                .unwrap(),
            Err(WorkspaceError::StaleRevision)
        );
        assert_eq!(
            backend.load_development_workspace().await.unwrap().snapshot,
            Some(external)
        );
        writer.discard_and_shutdown().await.unwrap();
        assert_eq!(
            writer.submit(snapshot("closed")),
            Err(WorkspaceError::Closing)
        );
        let current = backend.load_development_workspace().await.unwrap();
        let writer = DraftWriter::new(
            backend.clone(),
            tokio::runtime::Handle::current(),
            current.revision,
        );
        writer.submit(snapshot("SELECT 'final close';")).unwrap();
        tokio::time::timeout(Duration::from_secs(3), writer.shutdown())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            backend.load_development_workspace().await.unwrap().snapshot,
            Some(snapshot("SELECT 'final close';"))
        );
        backend.shutdown().await.unwrap();
        assert!(path.join(".dbunk-native-stage04").is_file());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn export_keeps_exact_sql_and_preserves_existing_files() {
        let path =
            std::env::temp_dir().join(format!("dbunk-sql-export-{}.sql", uuid::Uuid::new_v4()));
        let draft = snapshot("SELECT '東京🙂';\n-- trailing spaces  \n");
        export_sql(&draft, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            String::from_utf8(bytes.clone()).unwrap(),
            format!("-- Query document 1\n\n{}\n\n", draft.documents[0].sql)
        );
        assert!(export_sql(&snapshot("replacement"), &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }
}
