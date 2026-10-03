//! Query Session operations as a host calls them.
//!
//! This is the family's whole surface for a host: every function takes the
//! label of the window that owns the session. `open`, `execute` and
//! `refresh_transaction_state` resolve the stored connection themselves, so
//! the safety policy and the audit always come from the hydrated record and
//! never from the caller (ADR-0024).

use super::protocol::*;
use super::{ExecutionRequest, ExecutionSafety};
use crate::app::{find_connection, touch_connection_activity, AppState};
use crate::host::SharedSink;
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::sql_params::{scan_parameters, ParameterRejectionReason};
use crate::safety::gate;
use crate::safety::policy::AuditDisposition;
use crate::{DatabaseEngine, StoredConnection};

/// The name a confirmed override is audited under. It is stored, so it stays
/// the Tauri command's name whichever host runs the execution.
const EXECUTE_AUDIT_COMMAND: &str = "execute_query_session";

async fn hydrated_connection(
    state: &AppState,
    connection_id: &str,
) -> Result<StoredConnection, QuerySessionError> {
    find_connection(state, connection_id)
        .await
        .map_err(|_| QuerySessionError::ConnectionLost)
}

fn connect_spec(
    connection: &StoredConnection,
) -> Result<ResolvedPostgresConnectSpec, QuerySessionError> {
    ResolvedPostgresConnectSpec::from_connection(connection)
        .map_err(|_| QuerySessionError::UnsupportedEngine)
}

pub(crate) async fn register_owner(
    state: &AppState,
    window: &str,
    payload: RegisterOwnerPayload,
) -> RegisterOwnerResult {
    state
        .query_sessions
        .register_owner(window, payload.owner_id)
        .await
}

pub(crate) async fn open(
    state: &AppState,
    window: &str,
    payload: OpenSessionPayload,
    sink: SharedSink<QueryEventEnvelope>,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    let connection = hydrated_connection(state, &payload.connection_id).await?;
    if connection.engine() != DatabaseEngine::PostgreSQL {
        return Err(QuerySessionError::UnsupportedEngine);
    }
    let spec = connect_spec(&connection)?;
    let connection_id = payload.connection_id.clone();
    let result = state
        .query_sessions
        .open(window, payload, sink, spec)
        .await?;
    touch_connection_activity(state, &connection_id).await;
    Ok(result)
}

pub(crate) async fn execute(
    state: &AppState,
    window: &str,
    payload: ExecutePayload,
) -> Result<AcceptedResult, QuerySessionError> {
    let connection_id = state
        .query_sessions
        .connection_id(&payload.session_id, window)
        .await?;
    let connection = hydrated_connection(state, &connection_id).await?;
    let policy = gate::resolved_policy(&connection);
    let pool = state.pool.clone();
    state
        .query_sessions
        .execute(
            &payload.session_id,
            ExecutionRequest {
                execution_id: payload.execution_id,
                sql: payload.sql,
                parameters: payload.parameters,
                row_limit: payload.row_limit,
            },
            window,
            ExecutionSafety {
                policy: &policy,
                confirmed: payload.confirmed,
                on_success: Some(Box::new(move |intent, authorization| {
                    Box::pin(async move {
                        if matches!(
                            authorization.audit_disposition(),
                            AuditDisposition::RequiredAfterSuccess
                        ) {
                            gate::record_override(
                                &pool,
                                &connection_id,
                                EXECUTE_AUDIT_COMMAND,
                                &intent,
                            )
                            .await;
                        }
                    })
                })),
            },
        )
        .await
}

/// Pure: the same scan execution binds with, and no database access.
pub(crate) fn describe_parameters(
    sql: &str,
) -> Result<DescribeParametersResult, QuerySessionError> {
    let scan = scan_parameters(sql).map_err(|()| QuerySessionError::ParametersRejected {
        reason: ParameterRejectionReason::Unlexable,
        names: Vec::new(),
    })?;
    Ok(DescribeParametersResult {
        names: scan.names().to_vec(),
    })
}

pub(crate) async fn ack(
    state: &AppState,
    window: &str,
    payload: AckPayload,
) -> Result<(), QuerySessionError> {
    state.query_sessions.ack(payload, window).await
}

pub(crate) async fn heartbeat(
    state: &AppState,
    window: &str,
    payload: HeartbeatPayload,
) -> Result<HeartbeatResult, QuerySessionError> {
    state.query_sessions.heartbeat(window, payload).await
}

pub(crate) async fn cancel(
    state: &AppState,
    window: &str,
    payload: ExecutionPayload,
) -> Result<CancelResult, QuerySessionError> {
    state.query_sessions.cancel(payload, window).await
}

pub(crate) async fn refresh_transaction_state(
    state: &AppState,
    window: &str,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    let connection_id = state
        .query_sessions
        .connection_id(&payload.session_id, window)
        .await?;
    let connection = hydrated_connection(state, &connection_id).await?;
    let spec = connect_spec(&connection)?;
    state
        .query_sessions
        .refresh(&payload.session_id, window, spec)
        .await
}

pub(crate) async fn set_mode(
    state: &AppState,
    window: &str,
    payload: SetModePayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .set_mode(&payload.session_id, window, payload.mode)
        .await
}

pub(crate) async fn set_isolation(
    state: &AppState,
    window: &str,
    payload: SetIsolationPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .set_isolation(&payload.session_id, window, payload.manual_isolation)
        .await
}

pub(crate) async fn commit(
    state: &AppState,
    window: &str,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .transaction_action(&payload.session_id, window, true)
        .await
}

pub(crate) async fn rollback(
    state: &AppState,
    window: &str,
    payload: SessionPayload,
) -> Result<QueryTransactionSnapshot, QuerySessionError> {
    state
        .query_sessions
        .transaction_action(&payload.session_id, window, false)
        .await
}

pub(crate) async fn close(
    state: &AppState,
    window: &str,
    payload: SessionPayload,
) -> Result<(), QuerySessionError> {
    state
        .query_sessions
        .close(&payload.session_id, window)
        .await
}
