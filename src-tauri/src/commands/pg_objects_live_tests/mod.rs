//! Live PostgreSQL round trips for the object DDL commands (`pnpm db:postgres`).
//!
//! Every test owns disposable objects: it resets them before running, runs its
//! body under `catch_unwind`, resets again, and only then re-raises a failed
//! assertion, so a broken run never leaves triggers, policies, grants, or
//! replaced routines behind for the next one.

mod apply;
mod policy;
mod privilege;
mod routine;
mod table;
mod trigger;

use std::panic::AssertUnwindSafe;

use futures_util::FutureExt;
use sqlx::Connection;

use super::pg_objects::tests::connection;
use super::pg_objects::{apply_object_ddl_inner, preview_object_ddl_inner, ApplyObjectDdlPayload};
use crate::postgres::fetch_table_structure;
use crate::postgres::object_ddl::*;
use crate::postgres::objects::{describe_pg_object, PgObjectFacts, PgObjectKind, PgObjectRef};
use crate::{AppState, PolicyCommand, SafeMode, StoredConnection, TriggerEnabledState};

pub(super) async fn run_sql(connection: &StoredConnection, sql: &str) {
    let mut conn = crate::postgres::connect(connection).await.expect("connect");
    sqlx::raw_sql(sql)
        .execute(&mut *conn)
        .await
        .expect("run fixture SQL");
}

/// Saves a live connection into a fresh app state.
pub(super) async fn live_state(id: &str) -> (tempfile::TempDir, AppState, StoredConnection) {
    let (directory, state) = crate::test_app_state().await;
    let connection = connection(id, SafeMode::Disabled, false);
    crate::commands::connections::save_connection_inner(&state, connection.clone())
        .await
        .expect("save live connection");
    (directory, state, connection)
}

/// Runs `body` between two executions of `reset`. The second execution runs
/// even when the body panicked; the panic is re-raised afterwards.
pub(super) async fn with_reset<F>(connection: &StoredConnection, reset: &str, body: F)
where
    F: std::future::Future<Output = ()>,
{
    run_sql(connection, reset).await;
    let outcome = AssertUnwindSafe(body).catch_unwind().await;
    run_sql(connection, reset).await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

pub(super) fn lifecycle_ref(
    kind: PgObjectKind,
    name: &str,
    identity_args: Option<&str>,
) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: Some("lifecycle".into()),
        name: name.into(),
        identity_args: identity_args.map(String::from),
    }
}

pub(super) fn column(name: &str, data_type: &str, nullable: bool) -> NewColumnSpec {
    NewColumnSpec {
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        default: None,
        identity: None,
    }
}

pub(super) async fn apply_live(
    state: &AppState,
    connection_id: &str,
    ops: Vec<PgObjectOp>,
) -> Result<DdlApplyResult, PgObjectError> {
    apply_object_ddl_inner(
        state,
        ApplyObjectDdlPayload {
            connection_id: connection_id.into(),
            ops,
            confirmed: true,
        },
    )
    .await
}

pub(super) fn index_op(name: &str, expression: &str) -> PgObjectOp {
    PgObjectOp::CreateIndex(CreateIndexOp {
        schema: "object_apply".into(),
        table: "scratch".into(),
        name: Some(name.into()),
        unique: false,
        method: "btree".into(),
        columns: vec![PgIndexColumn {
            expression: expression.into(),
            descending: false,
        }],
        include: Vec::new(),
        where_predicate: None,
        concurrently: true,
    })
}
