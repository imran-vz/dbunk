use super::*;
use dbunk_lib::backend::maintenance::MaintenanceFailure;
use dbunk_lib::backend::{WorkspaceMaintenanceAction, WorkspaceMaintenanceKind};

fn journal() -> WorkspaceMaintenance {
    WorkspaceMaintenance {
        attempt_id: uuid::Uuid::new_v4().to_string(),
        action: WorkspaceMaintenanceAction::Vacuum,
        database_oid: 12,
        database: "owned".into(),
        namespace_oid: 34,
        schema: "資料".into(),
        relation_oid: 56,
        name: "rows".into(),
        kind: WorkspaceMaintenanceKind::Table,
        sql: "VACUUM \"資料\".\"rows\"".into(),
        potentially_partial: true,
        operation_timeout_ms: 300_000,
        statement_timeout_ms: None,
        state: WorkspaceMaintenanceState::OutcomeUnknown,
    }
}
#[test]
fn interrupted_effects_and_lost_acknowledgements_preserve_the_exact_attempt() {
    let original = journal();
    for outcome in [
        MaintenanceOutcome::OutcomeUnknown {
            reason: MaintenanceFailure::Connection,
        },
        MaintenanceOutcome::InterruptedEffectsPossible {
            reason: MaintenanceFailure::Cancelled,
        },
    ] {
        let mut recovery = Some(original.clone());
        settle_recovery(&mut recovery, &outcome);
        let state = if matches!(outcome, MaintenanceOutcome::OutcomeUnknown { .. }) {
            WorkspaceMaintenanceState::OutcomeUnknown
        } else {
            WorkspaceMaintenanceState::EffectsPossible
        };
        let mut expected = original.clone();
        expected.state = state;
        assert_eq!(recovery, Some(expected));
    }
    for outcome in [
        MaintenanceOutcome::Completed,
        MaintenanceOutcome::TargetChanged,
        MaintenanceOutcome::NotDispatched {
            reason: MaintenanceFailure::Cancelled,
        },
        MaintenanceOutcome::RolledBack {
            reason: MaintenanceFailure::Cancelled,
        },
    ] {
        let mut recovery = Some(original.clone());
        settle_recovery(&mut recovery, &outcome);
        assert!(recovery.is_none());
    }
}
#[test]
fn review_lease_refuses_without_charge_and_releases_for_another_tool() {
    let budget = Rc::new(Cell::new(128 * 1024 * 1024 - ALLOWANCE + 1));
    assert!(Lease::admit(budget.clone()).is_none());
    assert_eq!(budget.get(), 128 * 1024 * 1024 - ALLOWANCE + 1);
    budget.set(128 * 1024 * 1024 - ALLOWANCE);
    let lease = Lease::admit(budget.clone()).unwrap();
    assert_eq!(budget.get(), 128 * 1024 * 1024);
    assert!(Lease::admit(budget.clone()).is_none());
    drop(lease);
    assert!(Lease::admit(budget.clone()).is_some());
    assert_eq!(budget.get(), 128 * 1024 * 1024 - ALLOWANCE);
}
