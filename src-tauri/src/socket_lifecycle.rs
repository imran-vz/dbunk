//! Canonical fence for dedicated PostgreSQL sockets.
//!
//! Begin/close all managers concurrently, run invalidation while admission is
//! blocked, then release the fence even if the caller panics.

use crate::postgres::backup::PgToolJobManager;
use crate::postgres::schema_compare::manager::CompareManager;
use crate::postgres::transfer::TransferManager;
use crate::query_session::QuerySessionManager;
use crate::result_mutation::ResultMutationManager;
use crate::table_browse::TableBrowseManager;
use crate::tunnel;
use crate::{postgres, redis, AppState, DatabaseEngine};

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheInvalidation {
    Connection(String),
    Bastion(String),
}

#[cfg(test)]
struct CacheInvalidationSubscription {
    targets: Vec<CacheInvalidation>,
    sender: tokio::sync::mpsc::UnboundedSender<CacheInvalidation>,
}

#[cfg(test)]
static CACHE_INVALIDATION_OBSERVER: std::sync::OnceLock<
    std::sync::Mutex<Option<CacheInvalidationSubscription>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
pub(crate) struct CacheInvalidationObserver;

#[cfg(test)]
pub(crate) fn observe_cache_invalidations(
    targets: impl IntoIterator<Item = CacheInvalidation>,
) -> (
    CacheInvalidationObserver,
    tokio::sync::mpsc::UnboundedReceiver<CacheInvalidation>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let subscription = CacheInvalidationSubscription {
        targets: targets.into_iter().collect(),
        sender,
    };
    let mut observer = CACHE_INVALIDATION_OBSERVER
        .get_or_init(Default::default)
        .lock()
        .expect("cache invalidation observer poisoned");
    assert!(
        observer.replace(subscription).is_none(),
        "observer already installed"
    );
    (CacheInvalidationObserver, receiver)
}

#[cfg(test)]
impl Drop for CacheInvalidationObserver {
    fn drop(&mut self) {
        *CACHE_INVALIDATION_OBSERVER
            .get_or_init(Default::default)
            .lock()
            .expect("cache invalidation observer poisoned") = None;
    }
}

#[cfg(test)]
fn observe_cache_invalidation(event: CacheInvalidation) {
    if let Some(subscription) = CACHE_INVALIDATION_OBSERVER
        .get_or_init(Default::default)
        .lock()
        .expect("cache invalidation observer poisoned")
        .as_ref()
        .filter(|subscription| subscription.targets.contains(&event))
    {
        let _ = subscription.sender.send(event);
    }
}

pub(crate) async fn with_connection_fence<T>(
    state: &AppState,
    connection_id: &str,
    work: impl std::future::Future<Output = T>,
) -> T {
    with_connection_ids_fence(state, &[connection_id.to_string()], work).await
}

pub(crate) async fn with_connection_ids_fence<T>(
    state: &AppState,
    connection_ids: &[String],
    work: impl std::future::Future<Output = T>,
) -> T {
    futures_util::future::join_all(connection_ids.iter().map(|connection_id| {
        let query_sessions = state.query_sessions.clone();
        let table_browse = state.table_browse.clone();
        let result_mutations = state.result_mutations.clone();
        let pg_tool_jobs = state.pg_tool_jobs.clone();
        let pg_transfers = state.pg_transfers.clone();
        let pg_schema_compare = state.pg_schema_compare.clone();
        async move {
            tokio::join!(
                query_sessions.begin_connection_teardown(connection_id),
                table_browse.begin_connection_teardown(connection_id),
                result_mutations.begin_connection_teardown(connection_id),
                pg_tool_jobs.begin_connection_teardown(connection_id),
                pg_transfers.begin_connection_teardown(connection_id),
                pg_schema_compare.begin_connection_teardown(connection_id),
            )
        }
    }))
    .await;
    let mut guard = FenceGuard::connections(
        state.query_sessions.clone(),
        state.table_browse.clone(),
        state.result_mutations.clone(),
        state.pg_tool_jobs.clone(),
        state.pg_transfers.clone(),
        state.pg_schema_compare.clone(),
        connection_ids.to_vec(),
    );
    let result = work.await;
    guard.release().await;
    result
}

pub(crate) async fn with_global_fence<T>(
    state: &AppState,
    work: impl std::future::Future<Output = T>,
) -> T {
    tokio::join!(
        state.query_sessions.begin_global_teardown(),
        state.table_browse.begin_global_teardown(),
        state.result_mutations.begin_global_teardown(),
        state.pg_tool_jobs.begin_global_teardown(),
        state.pg_transfers.begin_global_teardown(),
        state.pg_schema_compare.begin_global_teardown(),
    );
    let mut guard = FenceGuard::global(
        state.query_sessions.clone(),
        state.table_browse.clone(),
        state.result_mutations.clone(),
        state.pg_tool_jobs.clone(),
        state.pg_transfers.clone(),
        state.pg_schema_compare.clone(),
    );
    let result = work.await;
    guard.release().await;
    result
}

pub(crate) fn invalidate_connection_caches(connection_id: &str, engine: Option<DatabaseEngine>) {
    #[cfg(test)]
    observe_cache_invalidation(CacheInvalidation::Connection(connection_id.to_string()));
    tunnel::drop_connection(connection_id);
    match engine {
        Some(DatabaseEngine::PostgreSQL) => postgres::drop_pool(connection_id),
        Some(DatabaseEngine::Redis) => redis::connection::drop_cached(connection_id),
        None => {
            postgres::drop_pool(connection_id);
            redis::connection::drop_cached(connection_id);
        }
        _ => {}
    }
}

pub(crate) fn invalidate_bastion_caches(bastion_id: &str, connection_ids: &[String]) {
    #[cfg(test)]
    observe_cache_invalidation(CacheInvalidation::Bastion(bastion_id.to_string()));
    tunnel::drop_bastion(bastion_id);
    for connection_id in connection_ids {
        invalidate_connection_caches(connection_id, None);
    }
}

struct FenceGuard {
    query_sessions: QuerySessionManager,
    table_browse: TableBrowseManager,
    result_mutations: ResultMutationManager,
    pg_tool_jobs: PgToolJobManager,
    pg_transfers: TransferManager,
    pg_schema_compare: CompareManager,
    kind: Option<FenceKind>,
}

enum FenceKind {
    Connections(Vec<String>),
    Global,
}

impl FenceGuard {
    fn connections(
        query_sessions: QuerySessionManager,
        table_browse: TableBrowseManager,
        result_mutations: ResultMutationManager,
        pg_tool_jobs: PgToolJobManager,
        pg_transfers: TransferManager,
        pg_schema_compare: CompareManager,
        connection_ids: Vec<String>,
    ) -> Self {
        Self {
            query_sessions,
            table_browse,
            result_mutations,
            pg_tool_jobs,
            pg_transfers,
            pg_schema_compare,
            kind: Some(FenceKind::Connections(connection_ids)),
        }
    }

    fn global(
        query_sessions: QuerySessionManager,
        table_browse: TableBrowseManager,
        result_mutations: ResultMutationManager,
        pg_tool_jobs: PgToolJobManager,
        pg_transfers: TransferManager,
        pg_schema_compare: CompareManager,
    ) -> Self {
        Self {
            query_sessions,
            table_browse,
            result_mutations,
            pg_tool_jobs,
            pg_transfers,
            pg_schema_compare,
            kind: Some(FenceKind::Global),
        }
    }

    async fn release(&mut self) {
        let Some(kind) = self.kind.take() else {
            return;
        };
        release_fence(
            &self.query_sessions,
            &self.table_browse,
            &self.result_mutations,
            &self.pg_tool_jobs,
            &self.pg_transfers,
            &self.pg_schema_compare,
            kind,
        )
        .await;
    }
}

impl Drop for FenceGuard {
    fn drop(&mut self) {
        let Some(kind) = self.kind.take() else {
            return;
        };
        let query_sessions = self.query_sessions.clone();
        let table_browse = self.table_browse.clone();
        let result_mutations = self.result_mutations.clone();
        let pg_tool_jobs = self.pg_tool_jobs.clone();
        let pg_transfers = self.pg_transfers.clone();
        let pg_schema_compare = self.pg_schema_compare.clone();
        tokio::spawn(async move {
            release_fence(
                &query_sessions,
                &table_browse,
                &result_mutations,
                &pg_tool_jobs,
                &pg_transfers,
                &pg_schema_compare,
                kind,
            )
            .await;
        });
    }
}

async fn release_fence(
    query_sessions: &QuerySessionManager,
    table_browse: &TableBrowseManager,
    result_mutations: &ResultMutationManager,
    pg_tool_jobs: &PgToolJobManager,
    pg_transfers: &TransferManager,
    pg_schema_compare: &CompareManager,
    kind: FenceKind,
) {
    match kind {
        FenceKind::Connections(connection_ids) => {
            for connection_id in connection_ids {
                tokio::join!(
                    query_sessions.end_connection_teardown(&connection_id),
                    table_browse.end_connection_teardown(&connection_id),
                    result_mutations.end_connection_teardown(&connection_id),
                    pg_tool_jobs.end_connection_teardown(&connection_id),
                    pg_transfers.end_connection_teardown(&connection_id),
                    pg_schema_compare.end_connection_teardown(&connection_id),
                );
            }
        }
        FenceKind::Global => {
            tokio::join!(
                query_sessions.end_global_teardown(),
                table_browse.end_global_teardown(),
                result_mutations.end_global_teardown(),
                pg_tool_jobs.end_global_teardown(),
                pg_transfers.end_global_teardown(),
                pg_schema_compare.end_global_teardown(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgres::backup::{
        protocol::*,
        runner::{Ready, Request},
    };
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[tokio::test]
    #[serial_test::serial]
    async fn pg_tools_are_stopped_before_connection_bastion_and_global_invalidation() {
        let (_dir, state) = crate::test_app_state().await;
        for scope in 0..3 {
            let mut stopped = Vec::new();
            let mut ids = Vec::new();
            for i in 0..if scope == 0 { 1 } else { 3 } {
                let id = format!("fence-{scope}-{i}");
                let mut p =
                    crate::postgres::backup::tests::backup(std::path::Path::new("/unused/archive"));
                p.connection_id = id.clone();
                let flag = Arc::new(AtomicBool::new(false));
                let child_stopped = flag.clone();
                state
                    .pg_tool_jobs
                    .start(
                        state.pg_tool_jobs.admission(&id).unwrap(),
                        Request::Backup(p).snapshot(),
                        move |ctx| async move {
                            ctx.cancelled().await;
                            // Inject termination of a child using a resolved tunnel.
                            child_stopped.store(true, Ordering::SeqCst);
                            Err::<Ready, _>(PgToolJobError::Cancelled)
                        },
                        Box::pin(async {}),
                    )
                    .unwrap();
                stopped.push(flag);
                ids.push(id);
            }
            let invalidation = async {
                assert!(
                    stopped.iter().all(|f| f.load(Ordering::SeqCst)),
                    "tunnel invalidated before child termination"
                );
                for id in &ids {
                    assert!(matches!(
                        state.pg_tool_jobs.admission(id),
                        Err(PgToolJobError::ConnectionClosing)
                    ));
                }
                // Exercise all canonical fence entry points. Bastion and credential
                // mutations use the global fence; caller tests verify their ordering.
            };
            match scope {
                0 => with_connection_fence(&state, &ids[0], invalidation).await,
                1 => with_connection_ids_fence(&state, &ids, invalidation).await,
                _ => with_global_fence(&state, invalidation).await,
            }
            for id in &ids {
                assert!(state.pg_tool_jobs.admission(id).is_ok());
            }
        }
    }
    #[tokio::test]
    async fn csv_transfers_join_the_canonical_connection_and_global_fences() {
        let (_dir, state) = crate::test_app_state().await;
        for global in [false, true] {
            let stopped = Arc::new(AtomicBool::new(false));
            crate::postgres::transfer::manager::tests::start_waiting(
                &state.pg_transfers,
                "csv-fence",
                stopped.clone(),
            );
            let invalidation = async {
                assert!(stopped.load(Ordering::SeqCst));
                assert!(matches!(
                    state.pg_transfers.admission("csv-fence"),
                    Err(crate::postgres::transfer::protocol::TransferError::ConnectionClosing)
                ));
            };
            if global {
                with_global_fence(&state, invalidation).await;
            } else {
                with_connection_fence(&state, "csv-fence", invalidation).await;
            }
            assert!(state.pg_transfers.admission("csv-fence").is_ok());
        }
    }
}
