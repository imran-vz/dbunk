//! One service-issued confirmation per owned session; review text never grants
//! authority. Only the backend token retained here can authorize its bound SQL.
use super::{WINDOW, error_message};
use crate::mailbox::{Message, Sender};
use dbunk_lib::backend::{
    Backend, ExecutePayload, QueryConfirmation, QuerySubmission, QueryTransactionSnapshot,
    QueryTransactionStatus, TransactionControl,
};
use tokio::sync::watch;

#[derive(Default)]
pub(super) struct QueryControls {
    confirmation: Option<QueryConfirmation>,
}
impl QueryControls {
    pub(super) fn has_confirmation(&self, execution: &str) -> bool {
        self.confirmation
            .as_ref()
            .is_some_and(|value| value.execution_id() == execution)
    }
    pub(super) fn discard(&mut self, execution: &str) {
        if self
            .confirmation
            .as_ref()
            .is_some_and(|value| value.execution_id() == execution)
        {
            self.confirmation = None;
        }
    }

    /// True means the service accepted an execution and its terminal owns the
    /// execution permit. Every refusal/review returns false and releases it.
    pub(super) async fn submit(
        &mut self,
        backend: &Backend,
        payload: ExecutePayload,
        discarded: &watch::Receiver<Option<String>>,
        events: &Sender,
    ) -> Result<bool, String> {
        self.confirmation = None;
        let execution = payload.execution_id.clone();
        match backend.submit_query(WINDOW, payload).await {
            Ok(QuerySubmission::Accepted(_)) => Ok(true),
            Ok(QuerySubmission::NeedsConfirmation(confirmation)) => {
                if discarded.borrow().as_deref() == Some(execution.as_str()) {
                    return reject(
                        events,
                        execution,
                        "Query inputs changed; review discarded".into(),
                    );
                }
                let review = Message::Review {
                    execution,
                    sql: confirmation.sql().into(),
                    statements: confirmation.statements().to_vec(),
                    parameters: confirmation.parameters().map(|values| {
                        values
                            .iter()
                            .map(|value| crate::mailbox::ReviewParameter {
                                name: value.name.clone(),
                                value: value.value.clone(),
                            })
                            .collect()
                    }),
                    row_limit: confirmation.row_limit(),
                };
                self.confirmation = Some(*confirmation);
                events
                    .send(review)
                    .map_err(|_| "Query review delivery failed".to_string())?;
                Ok(false)
            }
            Err(error) => reject(events, execution, error_message(error)),
        }
    }

    pub(super) async fn confirm(
        &mut self,
        backend: &Backend,
        execution: String,
        discarded: &watch::Receiver<Option<String>>,
        events: &Sender,
    ) -> Result<bool, String> {
        if discarded.borrow().as_deref() == Some(execution.as_str()) {
            self.discard(&execution);
        }
        if !self
            .confirmation
            .as_ref()
            .is_some_and(|value| value.execution_id() == execution)
        {
            return reject(
                events,
                execution,
                "Review is no longer valid; run the query again".into(),
            );
        }
        let confirmation = self.confirmation.take().expect("matching confirmation");
        match backend.confirm_query(WINDOW, confirmation).await {
            Ok(_) => Ok(true),
            Err(error) => reject(events, execution, error_message(error)),
        }
    }
}

fn reject(events: &Sender, execution: String, message: String) -> Result<bool, String> {
    events
        .send(Message::Rejected { execution, message })
        .map_err(|_| "Query refusal delivery failed".to_string())?;
    Ok(false)
}

/// UI availability follows the last authoritative snapshot. Unknown state only
/// offers Recheck; a failed transaction must be rolled back, never committed.
pub fn transaction_allowed(
    snapshot: Option<&QueryTransactionSnapshot>,
    control: TransactionControl,
) -> bool {
    use QueryTransactionStatus as Status;
    match control {
        TransactionControl::Recheck => true,
        TransactionControl::Mode(_) | TransactionControl::Isolation(_) => {
            snapshot.is_some_and(|value| value.status == Status::Idle)
        }
        TransactionControl::Commit => snapshot.is_some_and(|value| value.status == Status::Active),
        TransactionControl::Rollback => {
            snapshot.is_some_and(|value| matches!(value.status, Status::Active | Status::Failed))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_or_unknown_state_never_enables_destructive_transaction_controls() {
        let controls = [
            TransactionControl::Commit,
            TransactionControl::Rollback,
            TransactionControl::Mode(dbunk_lib::backend::QueryTransactionMode::Manual),
            TransactionControl::Isolation(
                dbunk_lib::backend::QueryTransactionIsolation::Serializable,
            ),
        ];
        let unknown = QueryTransactionSnapshot {
            status: QueryTransactionStatus::Unknown,
            ..Default::default()
        };
        for snapshot in [None, Some(&unknown)] {
            assert!(transaction_allowed(snapshot, TransactionControl::Recheck));
            for control in controls {
                assert!(!transaction_allowed(snapshot, control));
            }
        }
        let failed = QueryTransactionSnapshot {
            status: QueryTransactionStatus::Failed,
            ..Default::default()
        };
        assert!(transaction_allowed(
            Some(&failed),
            TransactionControl::Rollback
        ));
        assert!(!transaction_allowed(
            Some(&failed),
            TransactionControl::Commit
        ));
    }
}
