use super::*;
use crate::postgres::backup::{manager::JobContext, runner::Ready, tests::terminal};
use crate::{SafeMode, StoredConnection};

async fn successful(
    ctx: JobContext,
    _: StoredConnection,
    _: Request,
) -> Result<Ready, PgToolJobError> {
    ctx.phase(PgToolJobPhase::Preflight)?;
    ctx.phase(PgToolJobPhase::Running)?;
    ctx.phase(PgToolJobPhase::Finalizing)?;
    Ok(Ready::Restore)
}
fn restore(id: &str, path: &std::path::Path, confirmed: bool) -> Request {
    Request::Restore(StartPgRestorePayload {
        connection_id: id.into(),
        source_path: path.to_string_lossy().into_owned(),
        format: PgBackupFormat::Plain,
        clean: false,
        confirmed,
    })
}

#[tokio::test]
#[serial_test::serial]
async fn typed_restore_policy_admission_and_success_only_effects() {
    let (_directory, state) = crate::test_app_state().await;
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(source.path(), b"SELECT 1;").unwrap();
    for mode in [SafeMode::Strict, SafeMode::Protected] {
        let id = format!("policy-{mode:?}");
        let connection = crate::commands::pg_objects::tests::connection(&id, mode, false);
        crate::commands::connections::save_connection_inner(&state, connection)
            .await
            .unwrap();
        assert!(matches!(
            start_with(&state, restore(&id, source.path(), false), successful).await,
            Err(PgToolJobError::PolicyNeedsConfirmation { .. })
        ));
        assert!(state.pg_tool_jobs.list(Some(&id)).is_empty());
        assert!(storage::read_safety_overrides(&state.pool, &id)
            .await
            .unwrap()
            .is_empty());
        let s = start_with(&state, restore(&id, source.path(), true), successful)
            .await
            .unwrap();
        assert_eq!(
            terminal(&state.pg_tool_jobs, &s.job_id).await.phase,
            PgToolJobPhase::Completed
        );
        assert_eq!(
            storage::read_safety_overrides(&state.pool, &id)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(storage::read_connection_by_id(&state.pool, &id)
            .await
            .unwrap()
            .unwrap()
            .last_activity_at()
            .is_some());
        let s = start_with(&state, restore(&id, source.path(), true), |_, _, _| async {
            Err(PgToolJobError::ToolUnavailable {
                tool: "psql".into(),
            })
        })
        .await
        .unwrap();
        terminal(&state.pg_tool_jobs, &s.job_id).await;
        assert_eq!(
            storage::read_safety_overrides(&state.pool, &id)
                .await
                .unwrap()
                .len(),
            1
        );
        let s = start_with(
            &state,
            restore(&id, source.path(), true),
            |ctx, _, _| async move {
                ctx.cancelled().await;
                Err(PgToolJobError::Cancelled)
            },
        )
        .await
        .unwrap();
        state.pg_tool_jobs.cancel(&s.job_id).unwrap();
        terminal(&state.pg_tool_jobs, &s.job_id).await;
        assert_eq!(
            storage::read_safety_overrides(&state.pool, &id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    let id = "readonly";
    crate::commands::connections::save_connection_inner(
        &state,
        crate::commands::pg_objects::tests::connection(id, SafeMode::Disabled, true),
    )
    .await
    .unwrap();
    assert!(matches!(
        start_with(&state, restore(id, source.path(), true), successful).await,
        Err(PgToolJobError::PolicyBlocked { .. })
    ));
    assert!(state.pg_tool_jobs.list(Some(id)).is_empty());
    let mut p = crate::postgres::backup::tests::backup(&source.path().with_extension("backup"));
    p.connection_id = id.into();
    let s = start_with(&state, Request::Backup(p), successful)
        .await
        .unwrap();
    terminal(&state.pg_tool_jobs, &s.job_id).await;
    assert!(storage::read_safety_overrides(&state.pool, id)
        .await
        .unwrap()
        .is_empty());
    assert!(storage::read_connection_by_id(&state.pool, id)
        .await
        .unwrap()
        .unwrap()
        .last_activity_at()
        .is_some());
}

#[tokio::test]
#[serial_test::serial]
async fn missing_and_unsupported_connections_never_admit() {
    let (_dir, state) = crate::test_app_state().await;
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(source.path(), b"SELECT 1").unwrap();
    assert!(matches!(
        start_with(&state, restore("missing", source.path(), true), successful).await,
        Err(PgToolJobError::InvalidRequest { .. })
    ));
    let pg =
        crate::commands::pg_objects::tests::connection("unsupported", SafeMode::Disabled, false);
    let mut value = serde_json::to_value(pg).unwrap();
    value["engine"] = "MySQL".into();
    let connection: StoredConnection = serde_json::from_value(value).unwrap();
    crate::commands::connections::save_connection_inner(&state, connection)
        .await
        .unwrap();
    assert_eq!(
        start_with(
            &state,
            restore("unsupported", source.path(), true),
            successful
        )
        .await
        .unwrap_err(),
        PgToolJobError::UnsupportedEngine
    );
    assert!(state.pg_tool_jobs.list(None).is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn cosmetic_connection_save_cancels_job_before_edit_and_reopens_admission() {
    let (_dir, state) = crate::test_app_state().await;
    let mut connection =
        crate::commands::pg_objects::tests::connection("rename", SafeMode::Disabled, false);
    crate::commands::connections::save_connection_inner(&state, connection.clone())
        .await
        .unwrap();
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(source.path(), b"SELECT 1").unwrap();
    let job = start_with(
        &state,
        restore("rename", source.path(), false),
        |ctx, _, _| async move {
            ctx.cancelled().await;
            Err(PgToolJobError::Cancelled)
        },
    )
    .await
    .unwrap();
    if let StoredConnection::PostgreSQL(pg) = &mut connection {
        pg.name = "Renamed".into();
    }
    crate::commands::connections::save_connection_inner(&state, connection)
        .await
        .unwrap();
    assert_eq!(
        state.pg_tool_jobs.get(&job.job_id).unwrap().phase,
        PgToolJobPhase::Cancelled
    );
    assert!(state.pg_tool_jobs.admission("rename").is_ok());
    assert!(storage::read_safety_overrides(&state.pool, "rename")
        .await
        .unwrap()
        .is_empty());
}

mod live;
