//! A native confirmation owns the exact refused request. It cannot be forged,
//! cloned, edited or applied to a replacement session. Stored policy is checked
//! again on acceptance, so confirmation never bypasses read-only enforcement.
use super::*;
use std::sync::Weak;

#[derive(Debug)]
pub enum QuerySubmission {
    Accepted(AcceptedResult),
    NeedsConfirmation(Box<QueryConfirmation>),
}

pub struct QueryConfirmation {
    backend: Weak<Inner>,
    window: String,
    instance: uuid::Uuid,
    payload: ExecutePayload,
    statements: Vec<StatementClassSummary>,
}

impl std::fmt::Debug for QueryConfirmation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryConfirmation")
            .field("statements", &self.statements)
            .finish_non_exhaustive()
    }
}

impl QueryConfirmation {
    pub fn sql(&self) -> &str {
        &self.payload.sql
    }

    pub fn statements(&self) -> &[StatementClassSummary] {
        &self.statements
    }

    pub fn execution_id(&self) -> &str {
        &self.payload.execution_id
    }

    pub fn parameters(&self) -> Option<&[ParameterValue]> {
        self.payload.parameters.as_deref()
    }

    pub fn row_limit(&self) -> Option<i64> {
        self.payload.row_limit
    }
}

impl Backend {
    /// Attempt an unconfirmed execution. Only an actual service refusal issues
    /// a confirmation. The legacy execute entry point stays unconfirmed.
    pub async fn submit_query(
        &self,
        window: &str,
        mut payload: ExecutePayload,
    ) -> Result<QuerySubmission, QuerySessionError> {
        payload.confirmed = false;
        let window = window.to_owned();
        let backend = Arc::downgrade(&self.0);
        self.development_call(move |state| async move {
            let instance = state
                .query_sessions
                .instance(&payload.session_id, &window)
                .await?;
            match service::execute(&state, &window, payload.clone()).await {
                Ok(result) => Ok(QuerySubmission::Accepted(result)),
                Err(QuerySessionError::PolicyNeedsConfirmation { statements }) => Ok(
                    QuerySubmission::NeedsConfirmation(Box::new(QueryConfirmation {
                        backend,
                        window,
                        instance,
                        payload,
                        statements,
                    })),
                ),
                Err(error) => Err(error),
            }
        })
        .await
    }

    /// Consumes a single request-bound challenge. The UI must drop it whenever
    /// SQL/parameters/selection change or the dialog is cancelled. Reconnect and
    /// connection/credential changes invalidate it at the service boundary too.
    pub async fn confirm_query(
        &self,
        window: &str,
        confirmation: QueryConfirmation,
    ) -> Result<AcceptedResult, QuerySessionError> {
        if confirmation.window != window {
            return Err(QuerySessionError::OwnerMismatch);
        }
        if !confirmation
            .backend
            .upgrade()
            .is_some_and(|owner| Arc::ptr_eq(&owner, &self.0))
        {
            return Err(QuerySessionError::ConnectionLost);
        }
        self.development_call(move |state| async move {
            let instance = state
                .query_sessions
                .instance(&confirmation.payload.session_id, &confirmation.window)
                .await?;
            if instance != confirmation.instance {
                return Err(QuerySessionError::SessionNotFound);
            }
            let mut payload = confirmation.payload;
            payload.confirmed = true;
            service::execute(&state, &confirmation.window, payload).await
        })
        .await
    }
}
