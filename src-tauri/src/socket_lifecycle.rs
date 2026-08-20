//! Canonical fence for dedicated PostgreSQL sockets.
//!
//! Begin/close all managers concurrently, run invalidation while admission is
//! blocked, then release the fence even if the caller panics.

use crate::query_session::QuerySessionManager;
use crate::result_mutation::ResultMutationManager;
use crate::table_browse::TableBrowseManager;
use crate::tunnel;
use crate::{postgres, redis, AppState, DatabaseEngine};

pub(crate) async fn with_connection_fence<T>(
    state: &AppState,
    connection_id: &str,
    work: impl std::future::Future<Output = T>,
) -> T {
    tokio::join!(
        state
            .query_sessions
            .begin_connection_teardown(connection_id),
        state.table_browse.begin_connection_teardown(connection_id),
        state
            .result_mutations
            .begin_connection_teardown(connection_id),
    );
    let mut guard = FenceGuard::connection(
        state.query_sessions.clone(),
        state.table_browse.clone(),
        state.result_mutations.clone(),
        connection_id.to_string(),
    );
    let result = work.await;
    guard.release().await;
    result
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
        async move {
            tokio::join!(
                query_sessions.begin_connection_teardown(connection_id),
                table_browse.begin_connection_teardown(connection_id),
                result_mutations.begin_connection_teardown(connection_id),
            )
        }
    }))
    .await;
    let mut guard = FenceGuard::connections(
        state.query_sessions.clone(),
        state.table_browse.clone(),
        state.result_mutations.clone(),
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
    );
    let mut guard = FenceGuard::global(
        state.query_sessions.clone(),
        state.table_browse.clone(),
        state.result_mutations.clone(),
    );
    let result = work.await;
    guard.release().await;
    result
}

pub(crate) fn invalidate_connection_caches(connection_id: &str, engine: Option<DatabaseEngine>) {
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
    tunnel::drop_bastion(bastion_id);
    for connection_id in connection_ids {
        invalidate_connection_caches(connection_id, None);
    }
}

struct FenceGuard {
    query_sessions: QuerySessionManager,
    table_browse: TableBrowseManager,
    result_mutations: ResultMutationManager,
    kind: Option<FenceKind>,
}

enum FenceKind {
    Connection(String),
    Connections(Vec<String>),
    Global,
}

impl FenceGuard {
    fn connection(
        query_sessions: QuerySessionManager,
        table_browse: TableBrowseManager,
        result_mutations: ResultMutationManager,
        connection_id: String,
    ) -> Self {
        Self {
            query_sessions,
            table_browse,
            result_mutations,
            kind: Some(FenceKind::Connection(connection_id)),
        }
    }

    fn connections(
        query_sessions: QuerySessionManager,
        table_browse: TableBrowseManager,
        result_mutations: ResultMutationManager,
        connection_ids: Vec<String>,
    ) -> Self {
        Self {
            query_sessions,
            table_browse,
            result_mutations,
            kind: Some(FenceKind::Connections(connection_ids)),
        }
    }

    fn global(
        query_sessions: QuerySessionManager,
        table_browse: TableBrowseManager,
        result_mutations: ResultMutationManager,
    ) -> Self {
        Self {
            query_sessions,
            table_browse,
            result_mutations,
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
        tokio::spawn(async move {
            release_fence(&query_sessions, &table_browse, &result_mutations, kind).await;
        });
    }
}

async fn release_fence(
    query_sessions: &QuerySessionManager,
    table_browse: &TableBrowseManager,
    result_mutations: &ResultMutationManager,
    kind: FenceKind,
) {
    match kind {
        FenceKind::Connection(connection_id) => {
            tokio::join!(
                query_sessions.end_connection_teardown(&connection_id),
                table_browse.end_connection_teardown(&connection_id),
                result_mutations.end_connection_teardown(&connection_id),
            );
        }
        FenceKind::Connections(connection_ids) => {
            for connection_id in connection_ids {
                tokio::join!(
                    query_sessions.end_connection_teardown(&connection_id),
                    table_browse.end_connection_teardown(&connection_id),
                    result_mutations.end_connection_teardown(&connection_id),
                );
            }
        }
        FenceKind::Global => {
            tokio::join!(
                query_sessions.end_global_teardown(),
                table_browse.end_global_teardown(),
                result_mutations.end_global_teardown(),
            );
        }
    }
}
