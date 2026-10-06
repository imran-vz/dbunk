//! Explicit-profile native host boundary. Services own credentials, policy and auditing.
//!
//! Raw application state and storage remain inaccessible to downstream hosts:
//! ```compile_fail
//! use dbunk_lib::app::AppState;
//! ```
//! ```compile_fail
//! use dbunk_lib::postgres::connect_spec::ResolvedPostgresConnectSpec;
//! ```

pub mod admin;
pub mod bastions;
pub mod clickhouse;
pub mod completion;
pub mod connection_diagnosis;
pub mod connection_uri;
pub mod csv_transfers;
pub mod data;
mod data_documents;
pub mod ddl_export;
mod development;
pub mod explain;
pub mod export_configurations;
pub mod legacy_import;
pub mod maintenance;
pub mod managed_servers;
pub mod mysql_sessions;
mod native_profile;
pub mod object_ddl;
pub mod objects;
pub mod overview;
pub mod pg_tools;
mod profile;
mod query_confirmation;
#[cfg(test)]
mod query_control_tests;
pub mod query_library;
pub mod query_mutation;
pub mod result_files;
pub mod safety_audit;
pub mod schema_alter;
pub mod schema_comparisons;
pub mod schema_ddl;
pub mod schema_map;
mod selection;
pub mod sequences;
pub mod server_details;
pub mod sqlite_session;
pub mod table_copy;
pub mod table_ddl;
pub mod table_export;
pub mod table_seed;
pub mod table_structure;
mod transactions;

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::app::AppState;
use crate::host::SharedSink;
use crate::postgres::connect_spec::{ResolvedPostgresConnectSpec, DEFAULT_CONNECT_TIMEOUT};
use crate::postgres::dedicated::DriverJoins;
use crate::query_session::service;
use crate::{CredentialStorageMode, DatabaseEngine, StoredConnection};
use tokio::sync::{Mutex, Semaphore, TryAcquireError};

pub use crate::host::{EventSink, SinkClosed};
pub use crate::postgres::sql_class::{StatementClassKind, StatementClassSummary};
pub use crate::postgres::sql_params::{ParameterRejectionReason, ParameterValue};
pub use crate::query_session::protocol::*;
pub use crate::types::TlsFailureKind;
pub use bastions::{
    DevelopmentBastion, DevelopmentBastionAuth, DevelopmentBastionAuthentication,
    DevelopmentBastionDelete, DevelopmentBastionForm, DevelopmentBastionReference,
    DevelopmentBastionSecrets, DevelopmentBastionTest, DevelopmentHostKeyStatus,
    DevelopmentSecretInput,
};
pub use development::{
    DevelopmentClickHouseConnection, DevelopmentEndpoint, DevelopmentEngineConnection,
    DevelopmentMySqlConnection, DevelopmentRedisConnection, DevelopmentSqliteConnection,
};
pub use development::{
    DevelopmentConnection, DevelopmentConnectionFailure, DevelopmentConnectionOrganization,
    DevelopmentConnectionTest, DevelopmentCredentialState, DevelopmentDriverOptions,
    DevelopmentEnvironment, DevelopmentFixtures, DevelopmentPostgresConnection,
    DevelopmentSafeMode, DevelopmentSettings, DevelopmentSshTunnel, DevelopmentStorageMode,
    DevelopmentTlsMode, DevelopmentTlsOptions, WorkspaceAdminAction, WorkspaceAdminControl,
    WorkspaceApplyState, WorkspaceDensity, WorkspaceDocument, WorkspaceError, WorkspaceLoad,
    WorkspaceMaintenance, WorkspaceMaintenanceAction, WorkspaceMaintenanceKind,
    WorkspaceMaintenanceState, WorkspaceMutationDraft, WorkspaceQueryChanges, WorkspaceRevision,
    WorkspaceSchemaAlter, WorkspaceSchemaChanges, WorkspaceSelection, WorkspaceSnapshot,
    WorkspaceStagedChange, WorkspaceTableCopy, WorkspaceTableCopyState, WorkspaceTableDdl,
    WorkspaceTableSeed, WorkspaceTableSeedState, WorkspaceTableState, WorkspaceTool,
    NATIVE_WORKSPACE_MAX_BYTES, NATIVE_WORKSPACE_MAX_DOCUMENTS, WORKSPACE_COPY_MAX_JOBS,
    WORKSPACE_MUTATION_MAX_BYTES, WORKSPACE_MUTATION_MAX_CHANGES, WORKSPACE_SCHEMA_ALTER_MAX_BYTES,
    WORKSPACE_SEED_MAX_JOBS, WORKSPACE_TABLE_DDL_MAX_BYTES,
};
pub use development::{
    RedisConsoleOutcome, RedisDatabase, RedisKey, RedisKeyInspection, RedisKeyValue, RedisOverview,
    RedisPolicy, RedisScanPage, RedisSession, RedisSessionError, RedisValue, REDIS_INSPECT_ITEMS,
    REDIS_MAX_DATABASES, REDIS_PAGE_KEYS, REDIS_SCAN_COUNT,
};
pub use development::{WorkspaceObjectDdl, WORKSPACE_OBJECT_DDL_MAX_BYTES};
pub use legacy_import::{
    import_legacy_profile, snapshot_legacy_profile, LegacyImportManifest, LegacySnapshotManifest,
};
pub use native_profile::NativeProfileKind;
pub use query_confirmation::{QueryConfirmation, QuerySubmission};
pub use query_mutation::{QueryMutationSource, QueryMutationSourceError};
pub use selection::{select_sql, select_sql_range, SelectionError};
pub use transactions::TransactionControl;

/// Geometry preference only; changes never replace an editor or session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    #[default]
    Stacked,
    SideBySide,
    ResultsFirst,
}

/// Last normal-quit window frame: display-local top-left origin and content
/// size in points, plus the display UUID. Native validates it against
/// connected displays before use; storage never decides placement.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FixtureSummary {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
}

struct Inner {
    state: Arc<AppState>,
    tasks: DriverJoins,
    tool_jobs: pg_tools::Registry,
    csv_transfers: csv_transfers::Registry,
    table_copy: table_copy::Registry,
    table_seed: table_seed::Registry,
    schema_comparisons: schema_comparisons::Registry,
    closing: AtomicBool,
    submission: std::sync::Mutex<()>,
    admission: Arc<Semaphore>,
    data_admission: Arc<Semaphore>,
    documents: data_documents::Documents,
    monitors: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    shutdown: Mutex<Option<Result<(), String>>>,
    _profile_lock: std::fs::File,
    development: Option<Arc<development::Authority>>,
    development_gate: Arc<Mutex<()>>,
    /// In-flight PostgreSQL opens, so a close can cancel one without waiting
    /// on the gate. Boxed for the same `size_of::<Inner>()` budget as below.
    opens: Box<OpenTickets>,
    /// Plan 031 step 4: live native MySQL sessions. Boxed: safety-audit
    /// cursors count `size_of::<Inner>()` against a fixed budget.
    mysql: Box<mysql_sessions::Registry>,
}

/// An opaque, cloneable service handle for an explicitly validated native profile.
/// Futures must run on the host's multi-thread Tokio runtime.
#[derive(Clone)]
pub struct Backend(Arc<Inner>);

impl Backend {
    /// Opens only an explicitly marked, private fixture directory. No platform
    /// profile resolution, credential migration, or keychain call occurs here.
    pub async fn open_fixture(path: &Path) -> Result<Self, String> {
        if tokio::runtime::Handle::current().runtime_flavor()
            != tokio::runtime::RuntimeFlavor::MultiThread
        {
            return Err("Native backend requires a multi-thread Tokio runtime".into());
        }
        let (state, profile_lock) = profile::open(path).await?;
        Ok(Self::from_state(state, profile_lock, None))
    }

    fn from_state(
        mut state: AppState,
        profile_lock: std::fs::File,
        development: Option<Arc<development::Authority>>,
    ) -> Self {
        let tasks = DriverJoins::default();
        let tool_jobs = pg_tools::Registry::default();
        let csv_transfers = csv_transfers::Registry::default();
        let table_copy = table_copy::Registry::default();
        let table_seed = table_seed::Registry::default();
        let schema_comparisons = schema_comparisons::Registry::default();
        state.pg_schema_compare = state
            .pg_schema_compare
            .with_native_ownership(schema_comparisons.owner.clone(), tasks.clone());
        state.pg_transfers = state
            .pg_transfers
            .with_native_ownership(csv_transfers.owner.clone(), tasks.clone());
        state.pg_tool_jobs = state
            .pg_tool_jobs
            .with_native_ownership(tool_jobs.ownership());
        state.query_sessions = state.query_sessions.with_native_tasks(tasks.clone());
        state.table_browse = state.table_browse.with_native_tasks(tasks.clone());
        state.result_mutations = state.result_mutations.with_native_tasks(tasks.clone());
        let runtime = tokio::runtime::Handle::current();
        let monitors = vec![
            state.query_sessions.spawn_monitor(&runtime),
            state.table_browse.spawn_monitor(&runtime),
            state.result_mutations.spawn_monitor(&runtime),
            state.pg_tool_jobs.spawn_monitor(&runtime),
            state.pg_transfers.spawn_monitor(&runtime),
            state.pg_schema_compare.spawn_monitor(&runtime),
        ];
        Self(Arc::new(Inner {
            state: Arc::new(state),
            tasks,
            tool_jobs,
            csv_transfers,
            table_copy,
            table_seed,
            schema_comparisons,
            closing: AtomicBool::new(false),
            submission: std::sync::Mutex::new(()),
            admission: Arc::new(Semaphore::new(16)),
            data_admission: Arc::new(Semaphore::new(8)),
            documents: Default::default(),
            monitors: Mutex::new(monitors),
            shutdown: Mutex::new(None),
            _profile_lock: profile_lock,
            development,
            development_gate: Arc::new(Mutex::new(())),
            opens: Default::default(),
            mysql: Default::default(),
        }))
    }

    pub fn fixture(&self) -> FixtureSummary {
        profile::summary()
    }

    /// Every service call is owned independently of its caller's future. Closing
    /// a view cannot abandon an opening reservation or detach a socket worker.
    async fn call<T, F, Fut>(&self, operation: F) -> Result<T, QuerySessionError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AppState>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, QuerySessionError>> + Send + 'static,
    {
        self.admitted_call(self.0.admission.clone(), Some(ADMISSION_WAIT), operation)
            .await
    }

    // Long data requests have a separate bounded budget. They cannot consume
    // the slots used by ACK, cancellation, lifecycle and persistence calls.
    // Data refusal stays immediate: its UI retries on the next request.
    async fn call_with_admission<T, F, Fut>(
        &self,
        admission: Arc<Semaphore>,
        operation: F,
    ) -> Result<T, QuerySessionError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AppState>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, QuerySessionError>> + Send + 'static,
    {
        self.admitted_call(admission, None, operation).await
    }

    /// A closed semaphore is shutdown and refuses at once. Exhaustion is
    /// transient load: with `wait`, the caller waits that long for a slot and
    /// is then refused with `Timeout`, never with `ConnectionClosing`.
    async fn admitted_call<T, F, Fut>(
        &self,
        admission: Arc<Semaphore>,
        wait: Option<Duration>,
        operation: F,
    ) -> Result<T, QuerySessionError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AppState>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, QuerySessionError>> + Send + 'static,
    {
        let permit = {
            let _submission = self.0.submission.lock().unwrap();
            if self.0.closing.load(Ordering::SeqCst) {
                return Err(QuerySessionError::ConnectionClosing);
            }
            admission.clone().try_acquire_owned()
        };
        let permit = match permit {
            Ok(permit) => permit,
            // Waits outside the submission lock; shutdown closes the
            // semaphore, which ends this wait with `Closed`.
            Err(TryAcquireError::NoPermits) => match wait {
                Some(wait) => match tokio::time::timeout(wait, admission.acquire_owned()).await {
                    Ok(Ok(permit)) => permit,
                    Ok(Err(_)) => return Err(QuerySessionError::ConnectionClosing),
                    Err(_) => {
                        return Err(QuerySessionError::Timeout {
                            operation: "nativeAdmission".into(),
                        })
                    }
                },
                None => return Err(QuerySessionError::ConnectionClosing),
            },
            Err(TryAcquireError::Closed) => return Err(QuerySessionError::ConnectionClosing),
        };
        let receive = {
            // Synchronize registration with shutdown's admission fence. Merely
            // checking an atomic flag before spawn leaves a late-worker race.
            let _submission = self.0.submission.lock().unwrap();
            if self.0.closing.load(Ordering::SeqCst) {
                return Err(QuerySessionError::ConnectionClosing);
            }
            let state = self.0.state.clone();
            let (send, receive) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                let _permit = permit;
                let _ = send.send(operation(state).await);
            });
            self.0.tasks.track_task(task);
            receive
        };
        receive
            .await
            .map_err(|_| QuerySessionError::ConnectionClosing)?
    }

    pub async fn register_owner(
        &self,
        window: &str,
        payload: RegisterOwnerPayload,
    ) -> Result<RegisterOwnerResult, QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move {
            Ok(service::register_owner(&state, &window, payload).await)
        })
        .await
    }

    /// Native lifecycle operations share this gate: connection, credential,
    /// bastion and managed-server mutations, and startup admission. A
    /// PostgreSQL open holds it only to admit and snapshot the record and,
    /// after connecting, to revalidate it; never across network I/O (see
    /// [`open_native_session`]). Existing Tauri fences are kept at the service
    /// boundary; native callers cannot overlap those fences.
    async fn development_call<T, F, Fut>(&self, operation: F) -> Result<T, QuerySessionError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<AppState>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, QuerySessionError>> + Send + 'static,
    {
        let inner = self.0.clone();
        self.call(move |state| async move {
            let _admission = inner.development_gate.lock().await;
            if inner.closing.load(Ordering::SeqCst) {
                return Err(QuerySessionError::ConnectionClosing);
            }
            operation(state).await
        })
        .await
    }

    pub async fn open(
        &self,
        window: &str,
        payload: OpenSessionPayload,
        sink: Arc<dyn EventSink<QueryEventEnvelope>>,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        if self.0.development.is_none() && payload.connection_id != profile::CONNECTION_ID {
            return Err(QuerySessionError::ConnectionLost);
        }
        // Registered before admission so a close issued after this call
        // cancels the open whatever step it has reached.
        let registration = OpenTickets::register(&self.0, window, &payload.session_id)?;
        let window = window.to_owned();
        let development = self.0.development.clone();
        let inner = self.0.clone();
        self.call(move |state| async move {
            let ticket = registration.ticket.clone();
            let result = open_native_session(
                &inner,
                &state,
                development.as_deref(),
                &ticket,
                &window,
                payload,
                sink,
            )
            .await;
            drop(registration);
            result
        })
        .await
    }

    pub async fn execute(
        &self,
        window: &str,
        mut payload: ExecutePayload,
    ) -> Result<AcceptedResult, QuerySessionError> {
        // The native slice has no confirmation UI. The service still performs
        // the complete stored-policy check and retains its existing audit path.
        payload.confirmed = false;
        let window = window.to_owned();
        self.call(move |state| async move { service::execute(&state, &window, payload).await })
            .await
    }

    pub async fn ack(&self, window: &str, payload: AckPayload) -> Result<(), QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move { service::ack(&state, &window, payload).await })
            .await
    }

    pub async fn cancel(
        &self,
        window: &str,
        payload: ExecutionPayload,
    ) -> Result<CancelResult, QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move { service::cancel(&state, &window, payload).await })
            .await
    }

    pub async fn heartbeat(
        &self,
        window: &str,
        payload: HeartbeatPayload,
    ) -> Result<HeartbeatResult, QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move { service::heartbeat(&state, &window, payload).await })
            .await
    }

    pub async fn close(
        &self,
        window: &str,
        payload: SessionPayload,
    ) -> Result<(), QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move { service::close(&state, &window, payload).await })
            .await
    }

    pub async fn set_focus(&self, window: &str, focused: bool) -> Result<(), QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move {
            state.query_sessions.set_focused(&window, focused).await;
            Ok(())
        })
        .await
    }

    /// Closes only this document's session. An in-flight open of it is
    /// cancelled first rather than waited for: the open runs ungated while it
    /// connects, observes the cancellation at its next step, and closes any
    /// session it still publishes. Its socket is owned by a tracked task that
    /// shutdown joins, so a cancelled open never leaves a socket behind.
    pub async fn close_native_session(
        &self,
        window: &str,
        session_id: &str,
    ) -> Result<(), QuerySessionError> {
        if self.0.development.is_none() {
            return Err(QuerySessionError::ConnectionLost);
        }
        self.0.opens.cancel(window, session_id);
        let window = window.to_owned();
        let session_id = session_id.to_owned();
        self.development_call(move |state| async move {
            state
                .query_sessions
                .close_native(&session_id, &window)
                .await
        })
        .await
    }

    /// Observes the owned socket's known closure; this is not a network probe.
    pub async fn session_alive(
        &self,
        window: &str,
        session_id: &str,
    ) -> Result<bool, QuerySessionError> {
        let window = window.to_owned();
        let id = session_id.to_owned();
        self.call(
            move |state| async move { state.query_sessions.session_alive(&window, &id).await },
        )
        .await
    }

    /// Retire a query owner after admitted startup, then join its sessions.
    /// Unrelated browse/mutation actors do not belong to this query barrier.
    /// The caller must stop issuing old-owner commands.
    pub async fn retire_window(&self, window: &str) -> Result<(), QuerySessionError> {
        let window = window.to_owned();
        tokio::time::timeout(
            Duration::from_secs(3),
            self.development_call(move |state| async move {
                state.query_sessions.retire_window(&window).await;
                Ok(())
            }),
        )
        .await
        .map_err(|_| QuerySessionError::Timeout {
            operation: "nativeRetire".into(),
        })?
    }

    pub async fn layout(&self) -> Result<Layout, String> {
        let value = self
            .call(move |state| async move {
                Ok(crate::storage::get_setting(&state.pool, "native.layout").await)
            })
            .await
            .map_err(|error| format!("{error:?}"))??;
        Ok(value
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default())
    }

    pub async fn set_layout(&self, layout: Layout) -> Result<(), String> {
        let encoded = serde_json::to_string(&layout).map_err(|error| error.to_string())?;
        self.call(move |state| async move {
            Ok(crate::storage::set_setting(&state.pool, "native.layout", &encoded).await)
        })
        .await
        .map_err(|error| format!("{error:?}"))?
    }

    /// Unreadable or unknown records read as absent and are left unchanged
    /// until the next normal quit replaces them.
    pub async fn window_geometry(&self) -> Result<Option<WindowGeometry>, String> {
        let value = self
            .call(move |state| async move {
                Ok(crate::storage::get_setting(&state.pool, "native.window").await)
            })
            .await
            .map_err(|error| format!("{error:?}"))??;
        Ok(value.and_then(|value| serde_json::from_str(&value).ok()))
    }

    pub async fn set_window_geometry(&self, geometry: WindowGeometry) -> Result<(), String> {
        let encoded = serde_json::to_string(&geometry).map_err(|error| error.to_string())?;
        self.call(move |state| async move {
            Ok(crate::storage::set_setting(&state.pool, "native.window", &encoded).await)
        })
        .await
        .map_err(|error| format!("{error:?}"))?
    }

    /// Idempotent teardown barrier: graceful close, then abort-and-join. A
    /// timeout that cannot establish termination is returned as a failed gate.
    pub async fn shutdown(&self) -> Result<(), String> {
        let started = tokio::time::Instant::now();
        self.shutdown_with_deadlines(
            started + Duration::from_secs(3),
            started + Duration::from_secs(5),
        )
        .await
    }

    /// Shares the host's overall shutdown budget, including its own workers.
    /// Expired grace still fences admission before abort-and-join is attempted.
    pub async fn shutdown_with_deadlines(
        &self,
        graceful_deadline: tokio::time::Instant,
        final_deadline: tokio::time::Instant,
    ) -> Result<(), String> {
        {
            let _submission = self.0.submission.lock().unwrap();
            self.0.closing.store(true, Ordering::SeqCst);
            self.0.admission.close();
            self.0.data_admission.close();
            self.0.documents.retire_matching(None);
            self.0.tool_jobs.close(&self.0.state.pg_tool_jobs);
            self.0.csv_transfers.close(&self.0.state.pg_transfers);
            self.0.table_copy.close();
            self.0.table_seed.close();
            self.0.mysql.close();
            self.0.schema_comparisons.close();
        }
        let mut shutdown = self.0.shutdown.lock().await;
        if let Some(result) = &*shutdown {
            return result.clone();
        }
        let monitors = std::mem::take(&mut *self.0.monitors.lock().await);
        for monitor in &monitors {
            monitor.abort();
        }
        for monitor in monitors {
            let _ = monitor.await;
        }
        let graceful = async {
            tokio::join!(
                self.0.state.query_sessions.begin_global_teardown(),
                self.0.state.table_browse.begin_global_teardown(),
                self.0.state.result_mutations.begin_global_teardown(),
                self.0.state.pg_tool_jobs.begin_global_teardown(),
                self.0.state.pg_transfers.begin_global_teardown(),
                self.0.state.pg_schema_compare.begin_global_teardown(),
            );
            self.0.tasks.drain().await;
            let _ = self.0.tool_jobs.drain_until(graceful_deadline).await;
            let _ = self.0.csv_transfers.drain_until(graceful_deadline).await;
            let _ = self.0.table_copy.drain_until(graceful_deadline).await;
            let _ = self.0.table_seed.drain_until(graceful_deadline).await;
            let _ = self
                .0
                .schema_comparisons
                .drain_until(graceful_deadline)
                .await;
            self.0
                .state
                .query_sessions
                .clear_native_reservations()
                .await;
        };
        let forced = tokio::time::timeout_at(graceful_deadline, graceful)
            .await
            .is_err();
        let result = if forced {
            self.0.tasks.abort_all();
            tokio::time::timeout_at(final_deadline, async {
                tokio::join!(
                    self.0.state.table_browse.force_native_teardown(None),
                    self.0.state.result_mutations.force_native_teardown(None),
                );
                self.0.tasks.drain().await;
                self.0.state.query_sessions.clear_native_reservations().await;

            }).await.map_err(|_| "Native cleanup could not join all owned tasks and close storage within five seconds".to_string())
        } else {
            Ok(())
        };
        let owners = async {
            self.0.tool_jobs.drain_until(final_deadline).await?;
            self.0.csv_transfers.drain_until(final_deadline).await?;
            self.0.table_copy.drain_until(final_deadline).await?;
            self.0.table_seed.drain_until(final_deadline).await?;
            self.0.schema_comparisons.drain_until(final_deadline).await
        };
        let owners = async {
            let result = owners.await;
            // Every lane has released its route; stop forwards and SSH
            // sessions and join their workers before storage closes.
            crate::tunnel::drop_all_async().await;
            result
        };
        let result = match owners.await {
            Ok(()) => {
                match tokio::time::timeout_at(final_deadline, self.0.state.pool.close()).await {
                    Ok(()) => result,
                    Err(_) => {
                        Err("Native storage close exceeded the shared shutdown deadline".into())
                    }
                }
            }
            Err(error) => Err(error),
        };
        *shutdown = Some(result.clone());
        result
    }
}

/// Bounded wait for a lifecycle admission slot. Exhaustion is transient load
/// (a burst of heartbeats, focus changes and ACKs), not shutdown.
const ADMISSION_WAIT: Duration = Duration::from_secs(2);

/// In-flight native PostgreSQL opens by session ID.
#[derive(Default)]
struct OpenTickets(std::sync::Mutex<HashMap<String, Arc<OpenTicket>>>);

struct OpenTicket {
    window: String,
    cancelled: AtomicBool,
}

impl OpenTicket {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Owned by the open's task; removes exactly its own ticket when that task
/// finishes or is aborted.
struct OpenRegistration {
    inner: Arc<Inner>,
    session: String,
    ticket: Arc<OpenTicket>,
}

impl OpenTickets {
    fn register(
        inner: &Arc<Inner>,
        window: &str,
        session: &str,
    ) -> Result<OpenRegistration, QuerySessionError> {
        let ticket = Arc::new(OpenTicket {
            window: window.into(),
            cancelled: AtomicBool::new(false),
        });
        {
            let mut tickets = inner.opens.0.lock().unwrap();
            if tickets.contains_key(session) {
                return Err(QuerySessionError::InvalidSequence);
            }
            tickets.insert(session.into(), ticket.clone());
        }
        Ok(OpenRegistration {
            inner: inner.clone(),
            session: session.into(),
            ticket,
        })
    }

    /// Only the owning window may cancel; an unknown session is a no-op.
    fn cancel(&self, window: &str, session: &str) {
        if let Some(ticket) = self.0.lock().unwrap().get(session) {
            if ticket.window == window {
                ticket.cancelled.store(true, Ordering::SeqCst);
            }
        }
    }

    #[cfg(test)]
    fn contains(&self, session: &str) -> bool {
        self.0.lock().unwrap().contains_key(session)
    }
}

impl Drop for OpenRegistration {
    fn drop(&mut self) {
        let mut tickets = self
            .inner
            .opens
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if tickets
            .get(&self.session)
            .is_some_and(|ticket| Arc::ptr_eq(ticket, &self.ticket))
        {
            tickets.remove(&self.session);
        }
    }
}

/// Native PostgreSQL open in three steps. What the lifecycle gate protects:
///
/// 1. Gated: admission (profile authority, credential recovery) and the
///    hydrated record are read together, so the open starts from either the
///    pre- or the post-mutation record of any queued edit, never a mix.
/// 2. Ungated: managed-server start, SSH route, observer and session sockets.
///    Each step has its own deadline (Docker lifecycle and readiness, the
///    tunnel's, and the connect timeout, defaulted here when the record sets
///    none). A slow host delays only this open, and a close cancels it via
///    its ticket instead of queueing behind it.
/// 3. Gated again: a mutation that completed while connecting is detected by
///    re-admitting and comparing the hydrated record (ignoring activity time,
///    name and organization). A fence that overlapped finalization already
///    refused or closed the session; a dead or cancelled session is closed.
///
/// Residual: a fence that changes no stored field (an explicit Disconnect or
/// a credential-mode change that keeps the password), and that starts and
/// ends entirely while this open connects, does not close the new session.
/// The record it connected with is still current in that case.
async fn open_native_session(
    inner: &Inner,
    state: &AppState,
    development: Option<&development::Authority>,
    ticket: &OpenTicket,
    window: &str,
    payload: OpenSessionPayload,
    sink: SharedSink<QueryEventEnvelope>,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    let connection_id = payload.connection_id.clone();
    let session_id = payload.session_id.clone();
    let (connection, mode, fingerprint) = {
        let _admission = inner.development_gate.lock().await;
        if inner.closing.load(Ordering::SeqCst) || ticket.cancelled() {
            return Err(QuerySessionError::ConnectionClosing);
        }
        admit_connection(state, development, &connection_id).await?;
        admission_snapshot(state, &connection_id).await?
    };
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(QuerySessionError::UnsupportedEngine);
    }
    if let Some(authority) = development {
        if ticket.cancelled() {
            return Err(QuerySessionError::ConnectionClosing);
        }
        managed_servers::ensure_running_for_connection(state, authority, &connection_id)
            .await
            .map_err(|message| QuerySessionError::Database {
                code: None,
                message,
                severity: None,
                position: None,
            })?;
    }
    if ticket.cancelled() {
        return Err(QuerySessionError::ConnectionClosing);
    }
    let resolved = crate::tunnel::resolve_connection(
        &state.credentials,
        &state.pool,
        mode,
        &connection_id,
        &connection,
    )
    .await
    .map_err(|_| QuerySessionError::ConnectionLost)?;
    let spec = native_connect_spec(&resolved)?;
    if ticket.cancelled() {
        return Err(QuerySessionError::ConnectionClosing);
    }
    let opened = state.query_sessions.open(window, payload, sink, spec).await;

    let _admission = inner.development_gate.lock().await;
    if ticket.cancelled() {
        // The close may have run before this open reserved or finalized;
        // repeat it so the reservation and any published session are joined.
        let _ = state.query_sessions.close_native(&session_id, window).await;
        return Err(QuerySessionError::ConnectionClosing);
    }
    let snapshot = opened?;
    if inner.closing.load(Ordering::SeqCst) {
        // Shutdown's global teardown owns this session now.
        return Err(QuerySessionError::ConnectionClosing);
    }
    let current = async {
        admit_connection(state, development, &connection_id).await?;
        Ok::<_, QuerySessionError>(admission_snapshot(state, &connection_id).await?.2)
    }
    .await;
    let unchanged = matches!(&current, Ok(current) if *current == fingerprint);
    let alive = unchanged
        && state
            .query_sessions
            .session_alive(window, &session_id)
            .await
            .unwrap_or(false);
    if !alive {
        let _ = state.query_sessions.close_native(&session_id, window).await;
        return Err(QuerySessionError::ConnectionLost);
    }
    crate::app::touch_connection_activity(state, &connection_id).await;
    Ok(snapshot)
}

/// The spec a native session connects with. A record without a connect
/// timeout gets the default deadline, so no open waits on the OS TCP timeout.
fn native_connect_spec(
    connection: &StoredConnection,
) -> Result<ResolvedPostgresConnectSpec, QuerySessionError> {
    let mut spec = ResolvedPostgresConnectSpec::from_connection(connection)
        .map_err(|_| QuerySessionError::UnsupportedEngine)?;
    spec.connect_timeout.get_or_insert(DEFAULT_CONNECT_TIMEOUT);
    Ok(spec)
}

/// Hydrated record, without its SSH route, plus a fingerprint of every field
/// a session's connection depends on.
async fn admission_snapshot(
    state: &AppState,
    connection_id: &str,
) -> Result<(StoredConnection, CredentialStorageMode, Vec<u8>), QuerySessionError> {
    let mode = crate::app::current_credential_mode(state)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)?;
    let mut connection = crate::storage::read_connection_by_id(&state.pool, connection_id)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)?
        .ok_or(QuerySessionError::ConnectionLost)?;
    crate::credentials::hydrate(&state.credentials, mode, &mut connection)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)?;
    let fingerprint = connection_fingerprint(&connection)?;
    Ok((connection, mode, fingerprint))
}

/// Activity time, name and organization never affect a live session, and
/// concurrent opens touch activity, so they are excluded.
fn connection_fingerprint(connection: &StoredConnection) -> Result<Vec<u8>, QuerySessionError> {
    let mut connection = connection.clone();
    if let StoredConnection::PostgreSQL(pg) = &mut connection {
        pg.last_activity_at = None;
        pg.name = String::new();
        pg.organization = Default::default();
    }
    serde_json::to_vec(&connection).map_err(|_| QuerySessionError::ConnectionLost)
}

/// Call while native startup admission is held, before any secret hydration.
/// Recheck can replace an observer socket, so it uses the same boundary as open.
async fn admit_connection(
    state: &AppState,
    development: Option<&development::Authority>,
    connection_id: &str,
) -> Result<(), QuerySessionError> {
    let Some(development) = development else {
        return if connection_id == profile::CONNECTION_ID {
            Ok(())
        } else {
            Err(QuerySessionError::ConnectionLost)
        };
    };
    let (connection, options_supported) =
        crate::storage::read_native_connection_by_id(&state.pool, connection_id)
            .await
            .map_err(|_| QuerySessionError::ConnectionLost)?
            .ok_or(QuerySessionError::ConnectionLost)?;
    if !options_supported || !development.permits(&connection) {
        return Err(QuerySessionError::ConnectionLost);
    }
    if crate::credentials::native_recovery_required(&state.credentials)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)?
    {
        return Err(QuerySessionError::ConnectionLost);
    }
    Ok(())
}

/// Admission for PostgreSQL-only services. General profiles now admit every
/// engine, so services built on PostgreSQL catalogs refuse others up front.
async fn admit_postgres_connection(
    state: &AppState,
    development: Option<&development::Authority>,
    connection_id: &str,
) -> Result<(), QuerySessionError> {
    admit_connection(state, development, connection_id).await?;
    match crate::storage::read_connection_by_id(&state.pool, connection_id).await {
        Ok(Some(crate::StoredConnection::PostgreSQL(_))) => Ok(()),
        _ => Err(QuerySessionError::ConnectionLost),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A separate process keeps keyring's process-global builder isolated from
    /// unrelated credential tests. This builder never reaches the OS keychain.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fixture_startup_never_opens_a_keychain_entry() {
        const CHILD: &str = "DBUNK_FIXTURE_KEYCHAIN_GUARD_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "backend::tests::fixture_startup_never_opens_a_keychain_entry",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        #[derive(Debug)]
        struct RecordingBuilder(Arc<std::sync::Mutex<Vec<(String, String)>>>);
        impl keyring::credential::CredentialBuilderApi for RecordingBuilder {
            fn build(
                &self,
                target: Option<&str>,
                service: &str,
                account: &str,
            ) -> keyring::Result<Box<keyring::credential::Credential>> {
                self.0
                    .lock()
                    .unwrap()
                    .push((service.into(), account.into()));
                keyring::mock::default_credential_builder().build(target, service, account)
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn persistence(&self) -> keyring::credential::CredentialPersistence {
                keyring::credential::CredentialPersistence::EntryOnly
            }
        }
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        keyring::set_default_credential_builder(Box::new(RecordingBuilder(calls.clone())));
        let directory = profile::directory();
        let path = directory.path().canonicalize().unwrap();
        for _ in 0..2 {
            let backend = Backend::open_fixture(&path).await.unwrap();
            let connection = crate::app::find_connection(&backend.0.state, &backend.fixture().id)
                .await
                .unwrap();
            assert_eq!(connection.password(), "dbunk");
            backend.shutdown().await.unwrap();
        }
        let calls = calls.lock().unwrap();
        assert!(
            calls.is_empty(),
            "Fixture startup reached Keychain identities: {calls:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn isolated_profile_rejects_foreign_files_and_remembers_layout() {
        let directory = profile::directory();
        let path = directory.path().canonicalize().unwrap();
        std::fs::write(path.join("foreign"), "do not alter").unwrap();
        assert!(Backend::open_fixture(&path).await.is_err());
        assert!(!path.join("dbunk.sqlite").exists());
        std::fs::remove_file(path.join("foreign")).unwrap();
        let backend = Backend::open_fixture(&path).await.unwrap();
        assert_eq!(backend.layout().await.unwrap(), Layout::Stacked);
        assert!(
            Backend::open_fixture(&path).await.is_err(),
            "exclusive profile lock"
        );
        backend.set_layout(Layout::SideBySide).await.unwrap();
        assert_eq!(backend.layout().await.unwrap(), Layout::SideBySide);
        assert_eq!(backend.window_geometry().await.unwrap(), None);
        let geometry = WindowGeometry {
            x: -1200.5,
            y: 40.0,
            width: 1100.0,
            height: 700.0,
            maximized: true,
            display: Some("37D8832A-2D66-02CA-B9F7-8F30A301B230".into()),
        };
        backend.set_window_geometry(geometry.clone()).await.unwrap();
        let connection = crate::app::find_connection(&backend.0.state, &backend.fixture().id)
            .await
            .unwrap();
        assert_eq!(connection.password(), "dbunk");
        backend.shutdown().await.unwrap();
        backend.shutdown().await.unwrap();
        assert!(matches!(
            backend
                .register_owner(
                    "native",
                    RegisterOwnerPayload {
                        owner_id: "late".into()
                    }
                )
                .await,
            Err(QuerySessionError::ConnectionClosing)
        ));
        drop(backend);
        let backend = Backend::open_fixture(&path).await.unwrap();
        assert_eq!(backend.layout().await.unwrap(), Layout::SideBySide);
        assert_eq!(backend.window_geometry().await.unwrap(), Some(geometry));
        crate::storage::set_setting(&backend.0.state.pool, "native.window", "{\"x\":1}")
            .await
            .unwrap();
        assert_eq!(backend.window_geometry().await.unwrap(), None);
        backend.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn existing_non_plain_credential_mode_is_never_hydrated() {
        let directory = profile::directory();
        let path = directory.path().canonicalize().unwrap();
        let backend = Backend::open_fixture(&path).await.unwrap();
        crate::credentials::set_credential_mode(
            &backend.0.state.pool,
            crate::CredentialStorageMode::Keychain,
        )
        .await
        .unwrap();
        backend.shutdown().await.unwrap();
        drop(backend);
        assert!(Backend::open_fixture(&path)
            .await
            .err()
            .unwrap()
            .contains("PlainSqlite"));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn profile_symlinks_are_rejected_before_database_access() {
        let directory = profile::directory();
        let path = directory.path().canonicalize().unwrap();
        let target = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(target.path(), path.join("dbunk.sqlite")).unwrap();
        assert!(Backend::open_fixture(&path).await.is_err());
        assert_eq!(std::fs::metadata(target.path()).unwrap().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_joins_admitted_work_even_when_the_caller_is_cancelled() {
        let directory = profile::directory();
        let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
            .await
            .unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let (finished, done) = tokio::sync::oneshot::channel();
        let caller = backend.clone();
        let task = tokio::spawn(async move {
            caller
                .call(move |_| async move {
                    started.send(()).unwrap();
                    released.await.unwrap();
                    finished.send(()).unwrap();
                    Ok(())
                })
                .await
        });
        ready.await.unwrap();
        task.abort();
        let _ = task.await;
        release.send(()).unwrap();
        backend.shutdown().await.unwrap();
        done.await.unwrap();
    }
    /// Loopback listener that accepts and never answers, wired into the
    /// fixture record. Private test setup only.
    async fn stalled_fixture(backend: &Backend) -> tokio::net::TcpListener {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut connection = crate::storage::read_connections(&backend.0.state.pool)
            .await
            .unwrap()
            .remove(0);
        let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
            unreachable!()
        };
        pg.port = listener.local_addr().unwrap().port();
        crate::storage::upsert_connection(&backend.0.state.pool, &connection)
            .await
            .unwrap();
        backend
            .register_owner(
                "native",
                RegisterOwnerPayload {
                    owner_id: "owner".into(),
                },
            )
            .await
            .unwrap();
        listener
    }

    fn spawn_open(
        backend: &Backend,
        session: &str,
    ) -> tokio::task::JoinHandle<Result<QueryTransactionSnapshot, QuerySessionError>> {
        let caller = backend.clone();
        let session = session.to_owned();
        tokio::spawn(async move {
            caller
                .open(
                    "native",
                    OpenSessionPayload {
                        owner_id: "owner".into(),
                        session_id: session,
                        tab_id: "tab".into(),
                        connection_id: caller.fixture().id,
                    },
                    Arc::new(|_: QueryEventEnvelope| Ok(())),
                )
                .await
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stalled_connect_does_not_hold_the_lifecycle_gate() {
        let directory = profile::directory();
        let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
            .await
            .unwrap();
        let listener = stalled_fixture(&backend).await;
        let opening = spawn_open(&backend, "stalled");
        let (_socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        // The open is inside its network step. Lifecycle work, including a
        // second document's admission, is not queued behind it.
        tokio::time::timeout(
            Duration::from_secs(1),
            backend.development_call(|_| async { Ok(()) }),
        )
        .await
        .expect("gate is free while a connect stalls")
        .unwrap();
        assert!(backend.0.opens.contains("stalled"));
        // A duplicate in-flight session ID is refused before admission.
        assert!(matches!(
            spawn_open(&backend, "stalled").await.unwrap(),
            Err(QuerySessionError::InvalidSequence)
        ));
        backend.shutdown().await.unwrap();
        assert!(opening.await.unwrap().is_err());
        assert!(!backend.0.opens.contains("stalled"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_cancelled_open_never_reaches_the_network() {
        let directory = profile::directory();
        let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
            .await
            .unwrap();
        let listener = stalled_fixture(&backend).await;
        let (entered, entry) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let held_backend = backend.clone();
        let held = tokio::spawn(async move {
            held_backend
                .development_call(move |_| async move {
                    entered.send(()).unwrap();
                    released.await.unwrap();
                    Ok(())
                })
                .await
        });
        entry.await.unwrap();
        let opening = spawn_open(&backend, "cancelled");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !backend.0.opens.contains("cancelled") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Another window cannot cancel this window's open.
        backend.0.opens.cancel("elsewhere", "cancelled");
        backend.0.opens.cancel("native", "cancelled");
        release.send(()).unwrap();
        held.await.unwrap().unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), opening)
                .await
                .unwrap()
                .unwrap(),
            Err(QuerySessionError::ConnectionClosing)
        ));
        assert!(!backend.0.opens.contains("cancelled"));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err(),
            "a cancelled open must not connect"
        );
        backend.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn admission_exhaustion_waits_briefly_and_is_not_reported_as_closing() {
        let directory = profile::directory();
        let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
            .await
            .unwrap();
        let (release, _) = tokio::sync::broadcast::channel::<()>(1);
        let (started, mut ready) = tokio::sync::mpsc::channel(16);
        let mut holders = Vec::new();
        for _ in 0..16 {
            let caller = backend.clone();
            let mut released = release.subscribe();
            let started = started.clone();
            holders.push(tokio::spawn(async move {
                caller
                    .call(move |_| async move {
                        started.send(()).await.unwrap();
                        let _ = released.recv().await;
                        Ok(())
                    })
                    .await
            }));
        }
        for _ in 0..16 {
            ready.recv().await.unwrap();
        }
        assert!(matches!(
            backend.call(|_| async { Ok(()) }).await,
            Err(QuerySessionError::Timeout { .. })
        ));
        // A slot freed during the wait admits the waiting call.
        let waiting = {
            let caller = backend.clone();
            tokio::spawn(async move { caller.call(|_| async { Ok(()) }).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        for holder in holders {
            holder.await.unwrap().unwrap();
        }
        backend.shutdown().await.unwrap();
        assert!(matches!(
            backend.call(|_| async { Ok(()) }).await,
            Err(QuerySessionError::ConnectionClosing)
        ));
    }

    #[test]
    fn fingerprint_ignores_activity_name_and_organization_only() {
        let base = crate::StoredConnection::PostgreSQL(crate::PgStoredConnection {
            organization: Default::default(),
            id: "c".into(),
            name: "Name".into(),
            database: "db".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            user: "u".into(),
            password: "secret".into(),
            role: String::new(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            ssl: false,
            tls_options: None,
            driver_options: None,
            ssh_tunnel: crate::SshTunnelConfig::default(),
        });
        let fingerprint = connection_fingerprint(&base).unwrap();
        let mut cosmetic = base.clone();
        if let crate::StoredConnection::PostgreSQL(pg) = &mut cosmetic {
            pg.name = "Renamed".into();
            pg.last_activity_at = Some("2026-10-06T00:00:00Z".into());
            pg.organization.folder = "Folder".into();
        }
        assert_eq!(connection_fingerprint(&cosmetic).unwrap(), fingerprint);
        let changes: [fn(&mut crate::PgStoredConnection); 3] = [
            |pg| pg.password = "rotated".into(),
            |pg| pg.port = 5433,
            |pg| pg.read_only = true,
        ];
        for change in changes {
            let mut changed = base.clone();
            if let crate::StoredConnection::PostgreSQL(pg) = &mut changed {
                change(pg);
            }
            assert_ne!(connection_fingerprint(&changed).unwrap(), fingerprint);
        }
    }

    #[test]
    fn native_spec_bounds_a_record_without_a_connect_timeout() {
        let connection = crate::StoredConnection::PostgreSQL(crate::PgStoredConnection {
            organization: Default::default(),
            id: "c".into(),
            name: "Name".into(),
            database: "db".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            user: "u".into(),
            password: String::new(),
            role: String::new(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            ssl: false,
            tls_options: None,
            driver_options: None,
            ssh_tunnel: crate::SshTunnelConfig::default(),
        });
        assert_eq!(
            native_connect_spec(&connection).unwrap().connect_timeout,
            Some(DEFAULT_CONNECT_TIMEOUT)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_during_stalled_connect_aborts_and_joins_the_owned_socket() {
        use tokio::io::AsyncReadExt;
        let directory = profile::directory();
        let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
            .await
            .unwrap();
        // Private test setup only: the public API never accepts an endpoint.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut connection = crate::storage::read_connections(&backend.0.state.pool)
            .await
            .unwrap()
            .remove(0);
        let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
            unreachable!()
        };
        pg.port = listener.local_addr().unwrap().port();
        crate::storage::upsert_connection(&backend.0.state.pool, &connection)
            .await
            .unwrap();
        backend
            .register_owner(
                "native",
                RegisterOwnerPayload {
                    owner_id: "owner".into(),
                },
            )
            .await
            .unwrap();
        let caller = backend.clone();
        let opening = tokio::spawn(async move {
            caller
                .open(
                    "native",
                    OpenSessionPayload {
                        owner_id: "owner".into(),
                        session_id: "pending".into(),
                        tab_id: "tab".into(),
                        connection_id: caller.fixture().id,
                    },
                    Arc::new(|_: QueryEventEnvelope| panic!("late session event")),
                )
                .await
        });
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut first_byte = [0];
        tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut first_byte))
            .await
            .unwrap()
            .unwrap();
        backend.shutdown().await.unwrap();
        assert!(matches!(
            opening.await.unwrap(),
            Err(QuerySessionError::ConnectionClosing)
        ));
        let mut handshake = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut handshake))
            .await
            .unwrap()
            .unwrap();
        assert!(
            !handshake.is_empty(),
            "connect reached the owned loopback socket"
        );
    }
}
