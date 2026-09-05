use super::*;
use crate::postgres::transfer::manager::JobContext;
use crate::SafeMode;

fn review(state: &AppState, id: &str, direction: Direction) -> String {
    let mut review = runner::test_review(id);
    review.payload.direction = direction;
    review.inspection.direction = direction;
    let admission = state.pg_transfers.admission(id).unwrap();
    state
        .pg_transfers
        .insert_review(&admission, review)
        .unwrap()
        .inspection_token
}

fn import() -> RunRequest {
    RunRequest::Import {
        mapping: vec![ColumnMapping {
            source_index: 0,
            target_column: "value".into(),
        }],
    }
}

async fn successful(
    ctx: JobContext,
    _: StoredConnection,
    _: runner::Review,
    _: RunRequest,
) -> Result<(), TransferError> {
    assert!(ctx.begin_finalizing());
    ctx.succeeded(Some(1));
    Ok(())
}

async fn terminal(state: &AppState, id: &str) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = state.pg_transfers.get(id).unwrap();
            if snapshot.phase.terminal() {
                return snapshot;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
#[serial_test::serial]
async fn import_policy_is_rechecked_and_only_acknowledged_success_is_audited() {
    let (_dir, state) = crate::test_app_state().await;
    let id = "csv-policy";
    crate::commands::connections::save_connection_inner(
        &state,
        crate::commands::pg_objects::tests::connection(id, SafeMode::Strict, false),
    )
    .await
    .unwrap();
    let token = review(&state, id, Direction::Import);
    assert!(
        matches!(start_with(&state, token.clone(), import(), false, successful).await,
        Err(TransferError::PolicyNeedsConfirmation { statements }) if !statements.is_empty())
    );
    assert!(state.pg_transfers.list(Some(id)).is_empty());
    assert!(state.pg_transfers.review(&token).is_ok());
    let snapshot = start_with(&state, token, import(), true, successful)
        .await
        .unwrap();
    assert_eq!(
        terminal(&state, &snapshot.job_id).await.phase,
        Phase::Completed
    );
    assert_eq!(
        storage::read_safety_overrides(&state.pool, id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(storage::read_connection_by_id(&state.pool, id)
        .await
        .unwrap()
        .unwrap()
        .last_activity_at()
        .is_some());

    for failure in [
        TransferError::OutcomeUnknown,
        TransferError::Cancelled,
        TransferError::SourceChanged,
    ] {
        let token = review(&state, id, Direction::Import);
        let snapshot = start_with(
            &state,
            token,
            import(),
            true,
            move |_, _, _, _| async move { Err(failure) },
        )
        .await
        .unwrap();
        assert_ne!(
            terminal(&state, &snapshot.job_id).await.phase,
            Phase::Completed
        );
    }
    assert_eq!(
        storage::read_safety_overrides(&state.pool, id)
            .await
            .unwrap()
            .len(),
        1
    );
    crate::commands::connections::save_connection_inner(
        &state,
        crate::commands::pg_objects::tests::connection(id, SafeMode::Disabled, true),
    )
    .await
    .unwrap();
    let token = review(&state, id, Direction::Import);
    assert!(matches!(
        start_with(&state, token, import(), true, successful).await,
        Err(TransferError::PolicyBlocked { .. })
    ));

    let token = review(&state, id, Direction::Export);
    let snapshot = start_with(
        &state,
        token,
        RunRequest::Export {
            destination_path: "/tmp/unused-csv-policy.csv".into(),
        },
        false,
        successful,
    )
    .await
    .unwrap();
    assert_eq!(
        terminal(&state, &snapshot.job_id).await.phase,
        Phase::Completed
    );
    assert_eq!(
        storage::read_safety_overrides(&state.pool, id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
#[serial_test::serial]
async fn saved_connection_edits_invalidate_review_and_confirmation_retry() {
    let (_dir, state) = crate::test_app_state().await;
    let id = "csv-edit";
    let connection = crate::commands::pg_objects::tests::connection(id, SafeMode::Strict, false);
    crate::commands::connections::save_connection_inner(&state, connection)
        .await
        .unwrap();
    let token = review(&state, id, Direction::Import);
    assert!(matches!(
        start_with(&state, token.clone(), import(), false, successful).await,
        Err(TransferError::PolicyNeedsConfirmation { .. })
    ));
    crate::commands::connections::save_connection_inner(
        &state,
        crate::commands::pg_objects::tests::connection(id, SafeMode::Strict, true),
    )
    .await
    .unwrap();
    assert!(matches!(
        start_with(&state, token, import(), true, successful).await,
        Err(TransferError::InspectionExpired)
    ));
    assert!(state.pg_transfers.list(Some(id)).is_empty());
}
