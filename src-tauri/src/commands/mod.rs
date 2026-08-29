//! Tauri command handlers, split by domain.
//!
//! Each sub-module groups `#[tauri::command]` functions by concern.
//! This module re-exports the shared helpers that multiple command
//! modules need (connection lookup, activity tracking) and the public
//! `handler_list!` invocations live in `lib.rs`.

pub(crate) mod bastions;
pub(crate) mod connections;
pub(crate) mod diagnosis;
pub(crate) mod keyvalue;
pub(crate) mod managed;
pub(crate) mod pg_objects;
pub(crate) mod query_session;
pub(crate) mod relational;
pub(crate) mod result_mutation;
pub(crate) mod safety;
pub(crate) mod settings;
pub(crate) mod table_browse;

use crate::credentials;
use crate::safety::policy::{AuditDisposition, WriteIntent};
use crate::storage;
use crate::{AppState, CredentialStorageMode, StoredConnection};

// ---------------------------------------------------------------------------
// Shared helpers — used by multiple command modules
// ---------------------------------------------------------------------------

/// Return all connections with passwords stripped (safe for frontend).
pub(super) async fn public_connections(state: &AppState) -> Result<Vec<StoredConnection>, String> {
    let mut entries = storage::read_connections(&state.pool).await?;
    for entry in entries.iter_mut() {
        entry.set_password(String::new());
    }
    Ok(entries)
}

pub(super) async fn current_credential_mode(
    state: &AppState,
) -> Result<CredentialStorageMode, String> {
    credentials::credential_mode(&state.pool)
        .await?
        .ok_or_else(|| "Credential storage is not configured".to_string())
}

pub(super) async fn find_connection(
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
pub(super) async fn with_active_connection<T, Fut>(
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
pub(super) async fn with_gated_active_connection<T, Intent, Op, Fut>(
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
        let authorization = safety::assert_legacy_permitted(&connection, &intent, confirmed)?;
        let result = op(connection).await?;
        if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
            safety::record_override(&pool, connection_id, command, &intent).await;
        }
        Ok(result)
    })
    .await
}

/// Bump the `lastActivityAt` field on a connection record. Best-effort —
/// failures are logged but never bubble up because activity tracking should
/// not break the underlying operation.
pub(super) async fn touch_connection_activity(state: &AppState, connection_id: &str) {
    if let Err(error) = storage::touch_connection_activity(&state.pool, connection_id).await {
        log::warn!("Failed to touch lastActivityAt: {error}");
    }
}
