//! Typed transaction controls reuse the Query Session service and its owner,
//! transition and observer rules. This module adds no SQL execution path.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionControl {
    Mode(QueryTransactionMode),
    Isolation(QueryTransactionIsolation),
    Commit,
    Rollback,
    Recheck,
}

impl Backend {
    /// Uses the same pure parameter scanner as execution. Values are supplied
    /// separately through ExecutePayload; this never interpolates or connects.
    pub fn describe_query_parameters(
        sql: &str,
    ) -> Result<DescribeParametersResult, QuerySessionError> {
        service::describe_parameters(sql)
    }

    /// Controls exactly the session owned by this window. The caller uses the
    /// returned snapshot, including failed/unknown state, rather than guessing
    /// a transaction outcome from the requested action.
    pub async fn control_transaction(
        &self,
        window: &str,
        session_id: &str,
        control: TransactionControl,
    ) -> Result<QueryTransactionSnapshot, QuerySessionError> {
        let window = window.to_owned();
        let session_id = session_id.to_owned();
        let development = self.0.development.clone();
        self.development_call(move |state| async move {
            match control {
                TransactionControl::Mode(mode) => {
                    service::set_mode(&state, &window, SetModePayload { session_id, mode }).await
                }
                TransactionControl::Isolation(manual_isolation) => {
                    service::set_isolation(
                        &state,
                        &window,
                        SetIsolationPayload {
                            session_id,
                            manual_isolation,
                        },
                    )
                    .await
                }
                TransactionControl::Commit => {
                    service::commit(&state, &window, SessionPayload { session_id }).await
                }
                TransactionControl::Rollback => {
                    service::rollback(&state, &window, SessionPayload { session_id }).await
                }
                TransactionControl::Recheck => {
                    let connection_id = state
                        .query_sessions
                        .connection_id(&session_id, &window)
                        .await?;
                    admit_connection(&state, development.as_deref(), &connection_id).await?;
                    service::refresh_transaction_state(
                        &state,
                        &window,
                        SessionPayload { session_id },
                    )
                    .await
                }
            }
        })
        .await
    }
}
