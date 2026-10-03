//! Durable schema intent is settled only by the exact attempt's terminal receipt.
//! An unavailable reply keeps the journal unknown until explicit reconciliation.
use dbunk_lib::backend::schema_ddl::{CreateSchemaOutcome, CreateSchemaReceipt};
use dbunk_lib::backend::{WorkspaceApplyState, WorkspaceSchemaChanges};

pub struct Recovery {
    connection: String,
    changes: Option<WorkspaceSchemaChanges>,
}
impl Recovery {
    pub fn new(connection: String, changes: Option<WorkspaceSchemaChanges>) -> Self {
        Self {
            connection,
            changes,
        }
    }
    pub fn changes(&self) -> Option<&WorkspaceSchemaChanges> {
        self.changes.as_ref()
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn unknown(&self) -> bool {
        self.changes
            .as_ref()
            .is_some_and(|c| c.apply_state == WorkspaceApplyState::OutcomeUnknown)
    }
    pub fn stage(&mut self, changes: WorkspaceSchemaChanges) -> bool {
        if self.unknown() || changes.apply_state != WorkspaceApplyState::Staged {
            return false;
        }
        self.changes = Some(changes);
        true
    }
    pub fn dispatching(&mut self) -> bool {
        let Some(changes) = &mut self.changes else {
            return false;
        };
        if changes.apply_state != WorkspaceApplyState::Staged {
            return false;
        }
        changes.apply_state = WorkspaceApplyState::OutcomeUnknown;
        true
    }
    /// Only before dispatch or after a definite non-execution refusal.
    pub fn not_sent(&mut self) {
        if let Some(changes) = &mut self.changes {
            changes.apply_state = WorkspaceApplyState::Staged;
        }
    }
    pub fn settle(&mut self, receipt: &CreateSchemaReceipt) -> bool {
        let Some(changes) = &self.changes else {
            return false;
        };
        if !self.unknown()
            || receipt.connection_id != self.connection
            || receipt.attempt_id != changes.attempt_id
            || receipt.intent != changes.intent
        {
            return false;
        }
        match receipt.outcome {
            CreateSchemaOutcome::Applied { .. } => self.changes = None,
            CreateSchemaOutcome::NotApplied { .. } => self.not_sent(),
            CreateSchemaOutcome::OutcomeUnknown { .. } => {}
        }
        true
    }
    /// Caller must obtain the explicit draft-discard/reconciliation action.
    pub fn discard(&mut self) {
        self.changes = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::schema_ddl::{
        CreateSchemaAttemptId, CreateSchemaFailure, CreateSchemaIntent,
    };
    fn changes() -> WorkspaceSchemaChanges {
        WorkspaceSchemaChanges {
            attempt_id: CreateSchemaAttemptId::new(),
            intent: CreateSchemaIntent::new("例 schema".into(), Some("exact ' comment".into()))
                .unwrap(),
            apply_state: WorkspaceApplyState::Staged,
        }
    }
    #[test]
    fn a_late_receipt_cannot_clear_another_attempt_connection_or_intent() {
        let current = changes();
        let receipt = CreateSchemaReceipt {
            attempt_id: current.attempt_id.clone(),
            connection_id: "connection".into(),
            intent: current.intent.clone(),
            outcome: CreateSchemaOutcome::Applied {
                statements: 2,
                runtime_ms: 1,
            },
        };
        let mut recovery = Recovery::new("connection".into(), Some(current.clone()));
        assert!(!recovery.settle(&receipt));
        assert!(recovery.dispatching());
        let mut wrong = receipt.clone();
        wrong.attempt_id = CreateSchemaAttemptId::new();
        assert!(!recovery.settle(&wrong));
        wrong = receipt.clone();
        wrong.connection_id = "other".into();
        assert!(!recovery.settle(&wrong));
        wrong = receipt.clone();
        wrong.intent = CreateSchemaIntent::new("another".into(), None).unwrap();
        assert!(!recovery.settle(&wrong));
        assert!(recovery.unknown());
        assert!(recovery.settle(&receipt));
        assert!(recovery.changes().is_none());
        assert!(!recovery.settle(&receipt));
    }
    #[test]
    fn restored_unknown_refuses_new_attempt_until_explicit_reconciliation() {
        let mut journal = changes();
        journal.apply_state = WorkspaceApplyState::OutcomeUnknown;
        let mut recovery = Recovery::new("connection".into(), Some(journal.clone()));
        assert!(!recovery.stage(changes()));
        assert!(!recovery.dispatching());
        let mut receipt = CreateSchemaReceipt {
            attempt_id: journal.attempt_id.clone(),
            connection_id: "connection".into(),
            intent: journal.intent.clone(),
            outcome: CreateSchemaOutcome::OutcomeUnknown {
                reason: CreateSchemaFailure::Connection,
            },
        };
        assert!(recovery.settle(&receipt));
        assert_eq!(recovery.changes(), Some(&journal));
        receipt.outcome = CreateSchemaOutcome::NotApplied {
            reason: CreateSchemaFailure::Cancelled,
        };
        assert!(recovery.settle(&receipt));
        assert!(!recovery.unknown());
        assert!(recovery.stage(changes()));
        assert!(recovery.dispatching());
        recovery.discard();
        assert!(recovery.changes().is_none());
    }
}
