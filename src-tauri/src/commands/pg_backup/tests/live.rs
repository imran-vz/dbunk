//! Opt-in round trip on infrastructure/test-db's disposable PostgreSQL only.
use super::*;
use futures_util::FutureExt;
use sqlx::{Connection, Executor};
use std::time::Duration;

fn fixture(database: &str) -> crate::PgStoredConnection {
    crate::PgStoredConnection {
        organization: Default::default(),
        id: database.into(),
        name: "Disposable backup round trip".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: database.into(),
        user: "dbunk".into(),
        password: "dbunk".into(),
        role: "read/write".into(),
        environment: crate::Environment::Test,
        safe_mode: SafeMode::Strict,
        read_only: false,
        last_activity_at: None,
        ssl: false,
        tls_options: None,
        driver_options: None,
        ssh_tunnel: Default::default(),
    }
}
async fn connect(database: &str) -> sqlx::PgConnection {
    let options = sqlx::postgres::PgConnectOptions::new()
        .host("127.0.0.1")
        .port(15432)
        .username("dbunk")
        .password("dbunk")
        .database(database)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable);
    sqlx::PgConnection::connect_with(&options)
        .await
        .expect("disposable PostgreSQL fixture on 15432")
}
async fn complete(state: &AppState, request: Request) -> PgToolJobSnapshot {
    let job = start(state, request).await.unwrap();
    let ended = terminal(&state.pg_tool_jobs, &job.job_id).await;
    assert_eq!(ended.phase, PgToolJobPhase::Completed, "{ended:?}");
    assert!(ended
        .tool_version
        .as_ref()
        .unwrap()
        .contains("(PostgreSQL)"));
    ended
}
async fn assert_rows(connection: &mut sqlx::PgConnection) {
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT id, value FROM backup_probe ORDER BY id")
        .fetch_all(connection)
        .await
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (1, "plan018-distinctive-value".into()),
            (2, "second-row".into())
        ]
    );
}

async fn assert_restricted_restore_rolls_back(
    state: &AppState,
    database: &str,
    connection: &mut sqlx::PgConnection,
    directory: &std::path::Path,
) {
    let marker = directory.join("must-not-execute");
    let inputs = [
        format!("CREATE TABLE restore_rollback_probe (id integer);\n\\! touch '{}'\n", marker.display()),
        "--\n-- PostgreSQL database dump\n--\n\n\\restrict SourceKey\n\nCREATE TABLE restore_rollback_probe (id integer);\n\\unrestrict WrongKey\n\n".into(),
    ];
    let audits = storage::read_safety_overrides(&state.pool, database)
        .await
        .unwrap()
        .len();
    for (index, input) in inputs.into_iter().enumerate() {
        let path = directory.join(format!("restricted-{index}.sql"));
        std::fs::write(&path, input).unwrap();
        let job = start(state, restore(database, &path, true)).await.unwrap();
        let ended = terminal(&state.pg_tool_jobs, &job.job_id).await;
        assert_eq!(ended.phase, PgToolJobPhase::Failed, "{ended:?}");
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT to_regclass('public.restore_rollback_probe') IS NULL"
            )
            .fetch_one(&mut *connection)
            .await
            .unwrap(),
            "valid SQL before a rejected meta-command must roll back"
        );
        assert!(!marker.exists(), "psql must not execute a local command");
        assert_eq!(
            storage::read_safety_overrides(&state.pool, database)
                .await
                .unwrap()
                .len(),
            audits
        );
        std::fs::remove_file(path).unwrap();
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable PostgreSQL fixture on port 15432 and matching native client tools"]
async fn disposable_postgres_plain_custom_clean_and_cancellation_round_trip() {
    let (_directory, state) = crate::test_app_state().await;
    let archives = tempfile::tempdir().unwrap();
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let source_db = format!("dbunk_backup_src_{unique}");
    let target_db = format!("dbunk_backup_dst_{unique}");
    let mut admin = connect("dbunk_demo").await;
    for db in [&source_db, &target_db] {
        admin
            .execute(format!("CREATE DATABASE {}", crate::quote_double(db)).as_str())
            .await
            .unwrap();
    }
    let result = std::panic::AssertUnwindSafe(async {
        for db in [&source_db, &target_db] {
            crate::commands::connections::save_connection_inner(&state, StoredConnection::PostgreSQL(fixture(db))).await.unwrap();
        }
        let mut source = connect(&source_db).await;
        let mut target = connect(&target_db).await;
        source.execute("CREATE TABLE backup_probe (id integer PRIMARY KEY, value text NOT NULL); INSERT INTO backup_probe VALUES (1, 'plan018-distinctive-value'), (2, 'second-row')").await.unwrap();
        let mut paths = Vec::new();
        for (name, format, clean) in [("plain.sql", PgBackupFormat::Plain, false), ("clean.sql", PgBackupFormat::Plain, true), ("custom.dump", PgBackupFormat::Custom, false)] {
            let path = archives.path().join(name);
            let job = complete(&state, Request::Backup(StartPgBackupPayload { connection_id: source_db.clone(), destination_path: path.to_string_lossy().into_owned(), format, scope: PgBackupScope::Database, clean })).await;
            assert!(job.bytes_processed.unwrap() > 0); assert!(std::fs::metadata(&path).unwrap().len() > 0);
            paths.push(path);
        }
        let plain = complete(&state, restore(&target_db, &paths[0], true)).await;
        assert_eq!(plain.bytes_processed, None); assert!(plain.total_bytes.unwrap() > 0);
        assert_rows(&mut target).await;
        source.execute("UPDATE backup_probe SET value = 'changed'").await.unwrap();
        complete(&state, restore(&source_db, &paths[1], true)).await;
        assert_rows(&mut source).await;
        target.execute("UPDATE backup_probe SET value = 'changed'").await.unwrap();
        complete(&state, Request::Restore(StartPgRestorePayload { connection_id: target_db.clone(), source_path: paths[2].to_string_lossy().into_owned(), format: PgBackupFormat::Custom, clean: true, confirmed: true })).await;
        assert_rows(&mut target).await;
        assert_eq!(std::fs::read_dir(archives.path()).unwrap().count(), 3, "no sibling partials after success");
        // A lock gives a deterministic long-running dump without manufacturing a huge database.
        source.execute("BEGIN; LOCK TABLE backup_probe IN ACCESS EXCLUSIVE MODE").await.unwrap();
        let cancelled_path = archives.path().join("cancelled.dump");
        let job = start(&state, Request::Backup(StartPgBackupPayload { connection_id: source_db.clone(), destination_path: cancelled_path.to_string_lossy().into_owned(), format: PgBackupFormat::Custom, scope: PgBackupScope::Database, clean: false })).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=$1 AND application_name='dbunk-pg-tool' AND wait_event_type='Lock'").bind(&source_db).fetch_one(&mut admin).await.unwrap();
                if n > 0 { break; }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.unwrap();
        assert_eq!(state.pg_tool_jobs.cancel(&job.job_id).unwrap().phase, PgToolJobPhase::Cancelling);
        assert_eq!(terminal(&state.pg_tool_jobs, &job.job_id).await.phase, PgToolJobPhase::Cancelled);
        assert!(!cancelled_path.exists()); assert_eq!(std::fs::read_dir(archives.path()).unwrap().count(), 3);
        source.execute("ROLLBACK").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let n: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=$1 AND application_name='dbunk-pg-tool'").bind(&source_db).fetch_one(&mut admin).await.unwrap();
                if n == 0 { break; } tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }).await.unwrap();
        assert_restricted_restore_rolls_back(&state, &target_db, &mut target, archives.path()).await;
        source.close().await.unwrap(); target.close().await.unwrap();
    }).catch_unwind().await;
    state.pg_tool_jobs.close_all().await;
    for db in [&source_db, &target_db] {
        crate::socket_lifecycle::invalidate_connection_caches(db, None);
        admin
            .execute(format!("DROP DATABASE {} WITH (FORCE)", crate::quote_double(db)).as_str())
            .await
            .unwrap();
    }
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
