//! Read-only recovery record. Deserializing it cannot recreate a signal token.
use super::{WorkspaceApplyState, WorkspaceError};
use crate::backend::admin::{AdminControlAction, AdminControlReview, AdminControlTarget};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceAdminAction {
    CancelQuery,
    TerminateSession,
}
impl From<AdminControlAction> for WorkspaceAdminAction {
    fn from(action: AdminControlAction) -> Self {
        match action {
            AdminControlAction::CancelQuery => Self::CancelQuery,
            AdminControlAction::TerminateSession => Self::TerminateSession,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceAdminControl {
    pub attempt_id: String,
    pub action: WorkspaceAdminAction,
    pub pid: i32,
    pub backend_start: String,
    pub query_start: Option<String>,
    pub database: Option<String>,
    pub apply_state: WorkspaceApplyState,
}
impl WorkspaceAdminControl {
    pub fn from_review(review: &AdminControlReview) -> Self {
        let target = review.target();
        Self {
            attempt_id: review.attempt_id().into(),
            action: review.action().into(),
            pid: target.pid(),
            backend_start: target.backend_start().into(),
            query_start: target.query_start().map(str::to_owned),
            database: target.database().map(str::to_owned),
            apply_state: WorkspaceApplyState::Staged,
        }
    }
    pub fn matches(
        &self,
        attempt: &str,
        action: AdminControlAction,
        target: &AdminControlTarget,
    ) -> bool {
        self.attempt_id == attempt
            && self.action == action.into()
            && self.pid == target.pid()
            && self.backend_start == target.backend_start()
            && self.query_start.as_deref() == target.query_start()
            && self.database.as_deref() == target.database()
    }
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        let uuid = uuid::Uuid::parse_str(&self.attempt_id).ok();
        let timestamp = |value: &str| {
            value.len() == 27
                && value.ends_with('Z')
                && chrono::DateTime::parse_from_rfc3339(value).is_ok()
        };
        if !uuid.is_some_and(|id| id.get_version_num() == 4 && id.to_string() == self.attempt_id)
            || self.pid <= 0
            || !timestamp(&self.backend_start)
            || self
                .query_start
                .as_deref()
                .is_some_and(|value| !timestamp(value))
            || self
                .database
                .as_ref()
                .is_some_and(|value| value.len() > 1024 || value.contains('\0'))
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        Ok(())
    }
}
