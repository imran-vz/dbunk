//! Fixture-only native host boundary. Services own credentials, policy and auditing.
//!
//! Raw application state and storage remain inaccessible to downstream hosts:
//! ```compile_fail
//! use dbunk_lib::app::AppState;
//! ```
//! ```compile_fail
//! use dbunk_lib::postgres::connect_spec::ResolvedPostgresConnectSpec;
//! ```

mod profile;
mod selection;

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::app::AppState;
use crate::postgres::dedicated::DriverJoins;
use crate::query_session::service;
use tokio::sync::{Mutex, Semaphore};

pub use crate::host::{EventSink, SinkClosed};
pub use crate::postgres::sql_class::{StatementClassKind, StatementClassSummary};
pub use crate::postgres::sql_params::{ParameterRejectionReason, ParameterValue};
pub use crate::query_session::protocol::*;
pub use crate::types::TlsFailureKind;
pub use selection::{select_sql, select_sql_range, SelectionError};

/// Geometry preference only; changes never replace an editor or session.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    #[default]
    Stacked,
    SideBySide,
    ResultsFirst,
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
    closing: AtomicBool,
    submission: std::sync::Mutex<()>,
    admission: Arc<Semaphore>,
    monitor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown: Mutex<Option<Result<(), String>>>,
    _profile_lock: std::fs::File,
}

/// An opaque, cloneable service handle for the disposable native fixture.
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
        let (mut state, profile_lock) = profile::open(path).await?;
        let tasks = DriverJoins::default();
        state.query_sessions = state.query_sessions.with_native_tasks(tasks.clone());
        let monitor = state
            .query_sessions
            .spawn_monitor(&tokio::runtime::Handle::current());
        Ok(Self(Arc::new(Inner {
            state: Arc::new(state),
            tasks,
            closing: AtomicBool::new(false),
            submission: std::sync::Mutex::new(()),
            admission: Arc::new(Semaphore::new(16)),
            monitor: Mutex::new(Some(monitor)),
            shutdown: Mutex::new(None),
            _profile_lock: profile_lock,
        })))
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
        let receive = {
            // Synchronize registration with shutdown's admission fence. Merely
            // checking an atomic flag before spawn leaves a late-worker race.
            let _submission = self.0.submission.lock().unwrap();
            if self.0.closing.load(Ordering::SeqCst) {
                return Err(QuerySessionError::ConnectionClosing);
            }
            let permit = self
                .0
                .admission
                .clone()
                .try_acquire_owned()
                .map_err(|_| QuerySessionError::ConnectionClosing)?;
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

    pub async fn open(
        &self,
        window: &str,
        payload: OpenSessionPayload,
        sink: Arc<dyn EventSink<QueryEventEnvelope>>,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        if payload.connection_id != profile::CONNECTION_ID {
            return Err(QuerySessionError::ConnectionLost);
        }
        let window = window.to_owned();
        self.call(move |state| async move { service::open(&state, &window, payload, sink).await })
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

    /// Fence the owner before closing sessions; pending opens then fail their
    /// second admission check. The caller must stop issuing old-owner commands.
    pub async fn retire_window(&self, window: &str) -> Result<(), QuerySessionError> {
        let window = window.to_owned();
        self.call(move |state| async move {
            state.query_sessions.retire_window(&window).await;
            Ok(())
        })
        .await?;
        tokio::time::timeout(Duration::from_secs(3), self.0.tasks.drain())
            .await
            .map_err(|_| QuerySessionError::Timeout {
                operation: "nativeRetire".into(),
            })?;
        Ok(())
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
        }
        let mut shutdown = self.0.shutdown.lock().await;
        if let Some(result) = &*shutdown {
            return result.clone();
        }
        if let Some(monitor) = self.0.monitor.lock().await.take() {
            monitor.abort();
            let _ = monitor.await;
        }
        let graceful = async {
            self.0.state.query_sessions.begin_global_teardown().await;
            self.0.tasks.drain().await;
            self.0
                .state
                .query_sessions
                .clear_native_reservations()
                .await;
            self.0.state.pool.close().await;
        };
        let forced = tokio::time::timeout_at(graceful_deadline, graceful)
            .await
            .is_err();
        let result = if forced {
            self.0.tasks.abort_all();
            tokio::time::timeout_at(final_deadline, async {
                self.0.tasks.drain().await;
                self.0.state.query_sessions.clear_native_reservations().await;
                self.0.state.pool.close().await;
            }).await.map_err(|_| "Native cleanup could not join all owned tasks and close storage within five seconds".to_string())
        } else {
            Ok(())
        };
        *shutdown = Some(result.clone());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
