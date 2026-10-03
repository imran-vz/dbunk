use dbunk_lib::backend::{WorkspaceTableSeedState as State, table_seed::*};

/// A receipt describes transaction knowledge independently from cleanup. Once
/// the user reconciles an attempt, later polling cannot revive its authority.
pub(super) fn observed_state(
    current: State,
    phase: TableSeedPhase,
    outcome: TableSeedOutcome,
) -> Option<State> {
    if current == State::Reconciled {
        return None;
    }
    match outcome {
        TableSeedOutcome::Completed { rows } => Some(State::Completed { rows }),
        TableSeedOutcome::RolledBack => Some(State::RolledBack),
        TableSeedOutcome::OutcomeUnknown => Some(State::Unknown),
        TableSeedOutcome::NotStarted if phase.terminal() => Some(State::NotStarted),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_commit_survives_failed_cleanup_and_reconciliation_survives_late_receipts() {
        assert_eq!(
            observed_state(
                State::Applying,
                TableSeedPhase::Failed,
                TableSeedOutcome::Completed { rows: 100 }
            ),
            Some(State::Completed { rows: 100 })
        );
        for outcome in [
            TableSeedOutcome::Completed { rows: 100 },
            TableSeedOutcome::RolledBack,
            TableSeedOutcome::OutcomeUnknown,
        ] {
            assert_eq!(
                observed_state(State::Reconciled, TableSeedPhase::Failed, outcome),
                None
            );
        }
    }

    #[test]
    fn pending_or_unknown_work_cannot_become_a_retryable_staged_recipe() {
        assert_eq!(
            observed_state(
                State::Applying,
                TableSeedPhase::Running,
                TableSeedOutcome::Pending
            ),
            None
        );
        assert_eq!(
            observed_state(
                State::Applying,
                TableSeedPhase::Failed,
                TableSeedOutcome::OutcomeUnknown
            ),
            Some(State::Unknown)
        );
        assert_eq!(
            observed_state(
                State::Applying,
                TableSeedPhase::Cancelled,
                TableSeedOutcome::NotStarted
            ),
            Some(State::NotStarted)
        );
    }
}
