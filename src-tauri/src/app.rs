//! Application state and the connection lookup every service starts from.
//!
//! Host-neutral: a host opens the pool, builds `AppState`, starts the monitors
//! on its runtime and calls services with `&AppState`.

use sqlx::SqlitePool;

use crate::credentials;
use crate::postgres::backup::PgToolJobManager;
use crate::postgres::schema_compare::manager::CompareManager;
use crate::postgres::transfer::TransferManager;
use crate::query_session::QuerySessionManager;
use crate::result_mutation::ResultMutationManager;
use crate::safety::gate;
use crate::safety::policy::{AuditDisposition, WriteIntent};
use crate::storage::{self, Paths};
use crate::table_browse::TableBrowseManager;
use crate::{CredentialStorageMode, StoredConnection};

pub(crate) struct AppState {
    pub(crate) pool: SqlitePool,
    pub(crate) paths: Paths,
    pub(crate) query_sessions: QuerySessionManager,
    pub(crate) result_mutations: ResultMutationManager,
    pub(crate) table_browse: TableBrowseManager,
    pub(crate) pg_tool_jobs: PgToolJobManager,
    pub(crate) pg_transfers: TransferManager,
    pub(crate) pg_schema_compare: CompareManager,
}

impl AppState {
    /// Builds every manager over an opened pool. The comparison manager is
    /// passed in because a WebView host has to create it before setup: a
    /// configured window can commit its first document before setup runs.
    pub(crate) fn new(pool: SqlitePool, paths: Paths, pg_schema_compare: CompareManager) -> Self {
        Self {
            query_sessions: QuerySessionManager::new(pool.clone()),
            result_mutations: ResultMutationManager::new(),
            table_browse: TableBrowseManager::new(),
            pg_tool_jobs: PgToolJobManager::new(),
            pg_transfers: TransferManager::new(),
            pg_schema_compare,
            pool,
            paths,
        }
    }

    /// Starts every manager's monitor on the host's runtime. Hosts call this
    /// from setup on their main thread, with no ambient Tokio context.
    pub(crate) fn start_monitors(&self, runtime: &tokio::runtime::Handle) {
        self.query_sessions.start_monitor(runtime);
        self.table_browse.start_monitor(runtime);
        self.result_mutations.start_monitor(runtime);
        self.pg_tool_jobs.start_monitor(runtime);
        self.pg_transfers.start_monitor(runtime);
        self.pg_schema_compare.start_monitor(runtime);
    }
}

const EXIT_SOCKET_CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Closes every socket-owning manager before the process exits. Takes the
/// managers by value so a host can run it on a task that outlives its state
/// handle.
pub(crate) async fn close_socket_managers_for_exit(
    query_sessions: QuerySessionManager,
    table_browse: TableBrowseManager,
    result_mutations: ResultMutationManager,
    pg_tool_jobs: PgToolJobManager,
    pg_transfers: TransferManager,
    pg_schema_compare: CompareManager,
) {
    // Comparison teardown retains admission through real worker/driver joins.
    let comparison_close = pg_schema_compare.close_all();
    let existing_close = tokio::time::timeout(EXIT_SOCKET_CLOSE_TIMEOUT, async {
        tokio::join!(
            query_sessions.close_all(),
            table_browse.close_all(),
            result_mutations.close_all(),
            pg_tool_jobs.close_all(),
            pg_transfers.close_all()
        )
    });
    let _ = tokio::join!(comparison_close, existing_close);
}

// ---------------------------------------------------------------------------
// Connection lookup, shared by every service
// ---------------------------------------------------------------------------

/// Return all connections with passwords stripped (safe for a UI).
pub(crate) async fn public_connections(state: &AppState) -> Result<Vec<StoredConnection>, String> {
    let mut entries = storage::read_connections(&state.pool).await?;
    for entry in entries.iter_mut() {
        entry.set_password(String::new());
    }
    Ok(entries)
}

pub(crate) async fn current_credential_mode(
    state: &AppState,
) -> Result<CredentialStorageMode, String> {
    credentials::credential_mode(&state.pool)
        .await?
        .ok_or_else(|| "Credential storage is not configured".to_string())
}

pub(crate) async fn find_connection(
    state: &AppState,
    connection_id: &str,
) -> Result<StoredConnection, String> {
    let mode = current_credential_mode(state).await?;
    let mut connection = storage::read_connection_by_id(&state.pool, connection_id)
        .await?
        .ok_or_else(|| "Connection not found".to_string())?;
    credentials::hydrate(&state.pool, mode, &mut connection).await?;
    crate::tunnel::resolve_connection(&state.pool, mode, connection_id, &connection).await
}

/// Run `op` against a connection and bump its `lastActivityAt` on success.
///
/// Owns the contract from ADR-0004: every successful operation against a
/// connection counts as activity. By making the bump a property of the
/// helper rather than each command, new commands inherit the behaviour for
/// free and can't quietly drift out of policy.
///
/// The bump only fires when `op` returns `Ok` — failed queries do not count
/// as activity, so a connection that's unreachable doesn't appear "fresh".
pub(crate) async fn with_active_connection<T, Fut>(
    state: &AppState,
    connection_id: &str,
    op: impl FnOnce(StoredConnection) -> Fut,
) -> Result<T, String>
where
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let connection = find_connection(state, connection_id).await?;
    let result = op(connection).await?;
    touch_connection_activity(state, connection_id).await;
    Ok(result)
}

/// Policy-aware form of `with_active_connection` for legacy string-error
/// commands. The gate runs against the hydrated record before `op`, and a
/// required confirmed override is audited only after `op` succeeds.
pub(crate) async fn with_gated_active_connection<T, Intent, Op, Fut>(
    state: &AppState,
    connection_id: &str,
    command: &'static str,
    confirmed: bool,
    intent: Intent,
    op: Op,
) -> Result<T, String>
where
    Intent: FnOnce(&StoredConnection) -> WriteIntent,
    Op: FnOnce(StoredConnection) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let pool = state.pool.clone();
    with_active_connection(state, connection_id, |connection| async move {
        let intent = intent(&connection);
        let authorization = gate::assert_legacy_permitted(&connection, &intent, confirmed)?;
        let result = op(connection).await?;
        if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
            gate::record_override(&pool, connection_id, command, &intent).await;
        }
        Ok(result)
    })
    .await
}

/// Bump the `lastActivityAt` field on a connection record. Best-effort —
/// failures are logged but never bubble up because activity tracking should
/// not break the underlying operation.
pub(crate) async fn touch_connection_activity(state: &AppState, connection_id: &str) {
    if let Err(error) = storage::touch_connection_activity(&state.pool, connection_id).await {
        log::warn!("Failed to touch lastActivityAt: {error}");
    }
}

#[cfg(test)]
pub(crate) fn configure_test_keyring() {
    static MOCK_KEYRING: std::sync::Once = std::sync::Once::new();
    MOCK_KEYRING.call_once(|| {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    });
}

#[cfg(test)]
pub(crate) async fn test_app_state() -> (tempfile::TempDir, AppState) {
    configure_test_keyring();
    let directory = tempfile::tempdir().expect("app state temp dir");
    let paths = Paths::from_dir(directory.path().to_path_buf());
    let pool = storage::open_pool(&paths).await.expect("app state pool");
    credentials::configure(&pool, CredentialStorageMode::PlainSqlite, None)
        .await
        .expect("plain SQLite credential storage");
    (directory, AppState::new(pool, paths, CompareManager::new()))
}

/// A stored connection to the disposable PostgreSQL fixture (`pnpm
/// db:postgres`, or the port in `DBUNK_OBJECT_TEST_PORT`).
#[cfg(test)]
pub(crate) fn test_postgres_connection(
    id: &str,
    safe_mode: crate::SafeMode,
    read_only: bool,
) -> StoredConnection {
    let port = std::env::var("DBUNK_OBJECT_TEST_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(15432);
    StoredConnection::PostgreSQL(crate::PgStoredConnection {
        organization: crate::ConnectionOrganization::default(),
        id: id.into(),
        name: "Object DDL policy".into(),
        database: "dbunk_demo".into(),
        host: "127.0.0.1".into(),
        port,
        user: "dbunk".into(),
        password: "dbunk".into(),
        role: "read/write".into(),
        environment: crate::Environment::Development,
        safe_mode,
        read_only,
        last_activity_at: None,
        ssl: port == 15433,
        tls_options: None,
        driver_options: None,
        ssh_tunnel: crate::SshTunnelConfig::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgres::schema_compare::{
        manager::JobContext,
        protocol::{Endpoint, StartRequest},
    };

    #[tokio::test]
    async fn exit_cleanup_waits_for_active_comparison_termination() {
        let (_directory, state) = test_app_state().await;
        let manager = state.pg_schema_compare.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (stopped, done) = tokio::sync::oneshot::channel();
        let request = StartRequest {
            request_id: format!("{}:exit", chrono::Utc::now().timestamp_millis()),
            source: Endpoint {
                connection_id: "source".into(),
                schema: "public".into(),
            },
            target: Endpoint {
                connection_id: "target".into(),
                schema: "public".into(),
            },
        };
        manager
            .start(request, move |ctx: JobContext| async move {
                struct Stopped(Option<tokio::sync::oneshot::Sender<()>>);
                impl Drop for Stopped {
                    fn drop(&mut self) {
                        let _ = self.0.take().unwrap().send(());
                    }
                }
                let _stopped = Stopped(Some(stopped));
                started.send(()).unwrap();
                match ctx.control.wait(std::future::pending::<()>()).await {
                    Err(error) => Err(error),
                    Ok(()) => unreachable!("pending comparison completed"),
                }
            })
            .unwrap();
        ready.await.unwrap();

        close_socket_managers_for_exit(
            state.query_sessions,
            state.table_browse,
            state.result_mutations,
            state.pg_tool_jobs,
            state.pg_transfers,
            state.pg_schema_compare,
        )
        .await;

        done.await
            .expect("comparison worker terminated before exit cleanup returned");
        assert!(manager.list().is_empty());
    }
}
