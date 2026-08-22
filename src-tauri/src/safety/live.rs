use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tauri::ipc::Channel;

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::dedicated::{self, NoticeSink};
use crate::query_session::postgres as session_postgres;
use crate::query_session::protocol::{
    ExecutePayload, OpenSessionPayload, QueryEventEnvelope, QuerySessionError,
};
use crate::{
    AppState, Environment, PgStoredConnection, SafeMode, SshTunnelConfig, StoredConnection,
};

fn connection(
    id: &str,
    environment: Environment,
    safe_mode: SafeMode,
    read_only: bool,
) -> StoredConnection {
    StoredConnection::PostgreSQL(PgStoredConnection {
        id: id.into(),
        name: "Safety live".into(),
        database: "dbunk_demo".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        user: "dbunk".into(),
        password: "dbunk".into(),
        role: "read/write".into(),
        environment,
        safe_mode,
        read_only,
        last_activity_at: None,
        ssl: true,
        driver_options: None,
        ssh_tunnel: SshTunnelConfig::default(),
    })
}

async fn save(state: &AppState, connection: &StoredConnection) {
    crate::commands::connections::save_connection_inner(state, connection.clone())
        .await
        .expect("save connection through command core");
}

async fn open_session(
    state: &AppState,
    connection: &StoredConnection,
    window_label: &str,
    session_id: &str,
    emitted: Arc<AtomicUsize>,
) {
    open_session_with_channel(
        state,
        connection,
        window_label,
        session_id,
        Channel::<QueryEventEnvelope>::new(move |_| {
            emitted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    )
    .await;
}

async fn open_session_with_channel(
    state: &AppState,
    connection: &StoredConnection,
    window_label: &str,
    session_id: &str,
    channel: Channel<QueryEventEnvelope>,
) {
    let owner_id = "safety-live-owner";
    state
        .query_sessions
        .register_owner(window_label, owner_id.into())
        .await;
    let spec = ResolvedPostgresConnectSpec::from_connection(connection).expect("Postgres spec");
    state
        .query_sessions
        .open(
            window_label,
            OpenSessionPayload {
                owner_id: owner_id.into(),
                session_id: session_id.into(),
                tab_id: session_id.into(),
                connection_id: connection.id().into(),
            },
            channel,
            spec,
        )
        .await
        .expect("open query session");
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn safety_live_query_audit_survives_frontend_delivery_failure() {
    let (_directory, state) = crate::test_app_state().await;
    let connection_id = format!("safety-delivery-{}", uuid::Uuid::new_v4().simple());
    let schema = format!("safety_delivery_{}", uuid::Uuid::new_v4().simple());
    let strict = connection(
        &connection_id,
        Environment::Production,
        SafeMode::Inherit,
        false,
    );
    let spec = ResolvedPostgresConnectSpec::from_connection(&strict).expect("Postgres spec");
    let admin = session_postgres::connect(&spec)
        .await
        .expect("admin session");
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; CREATE TABLE {schema}.rows(value integer); INSERT INTO {schema}.rows VALUES (0)"
        ))
        .await
        .expect("fixture");

    save(&state, &strict).await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let channel_attempts = attempts.clone();
    open_session_with_channel(
        &state,
        &strict,
        "safety-delivery-window",
        "delivery-failure",
        Channel::<QueryEventEnvelope>::new(move |_| {
            let attempt = channel_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                Ok(())
            } else {
                Err(std::io::Error::other("frontend receiver closed").into())
            }
        }),
    )
    .await;

    crate::commands::query_session::execute_query_session_inner(
        &state,
        "safety-delivery-window",
        execute_payload(
            "delivery-failure",
            "confirmed-write",
            format!("UPDATE {schema}.rows SET value = 1"),
            true,
        ),
    )
    .await
    .expect("confirmed strict execution admitted");

    wait_for_i32(
        &admin,
        &format!("SELECT value FROM {schema}.rows LIMIT 1"),
        1,
    )
    .await;
    wait_for_audit_count(&state, &connection_id, 1).await;
    wait_for_activity(&state, &connection_id).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 3);

    admin
        .client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .expect("cleanup");
}

fn execute_payload(
    session_id: &str,
    execution_id: &str,
    sql: String,
    confirmed: bool,
) -> ExecutePayload {
    ExecutePayload {
        session_id: session_id.into(),
        execution_id: execution_id.into(),
        sql,
        confirmed,
    }
}

async fn wait_for_i32(admin: &session_postgres::SessionConnection, sql: &str, expected: i32) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let value: i32 = admin
                .client
                .query_one(sql, &[])
                .await
                .expect("read query effect")
                .get(0);
            if value == expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("query effect observed");
}

async fn wait_for_event_count(emitted: &AtomicUsize, minimum: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while emitted.load(Ordering::SeqCst) < minimum {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("terminal query event observed");
}

async fn wait_for_audit_count(state: &AppState, connection_id: &str, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let audits = crate::storage::read_safety_overrides(&state.pool, connection_id)
                .await
                .expect("read query audits");
            if audits.len() == expected {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("query audit observed");
}

async fn wait_for_activity(state: &AppState, connection_id: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let active = crate::storage::read_connection_by_id(&state.pool, connection_id)
                .await
                .expect("read query connection")
                .is_some_and(|connection| connection.last_activity_at().is_some());
            if active {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("query activity observed");
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn safety_live_read_only_guc_covers_pooled_dedicated_and_policy_flip() {
    let (_directory, state) = crate::test_app_state().await;
    let connection_id = format!("safety-guc-{}", uuid::Uuid::new_v4().simple());
    let writable = connection(
        &connection_id,
        Environment::Development,
        SafeMode::Inherit,
        false,
    );
    save(&state, &writable).await;
    crate::postgres::connect(&writable)
        .await
        .expect("initial pooled connection");

    // The command core fences every actor and invalidates the cached pool.
    // Reusing the ID proves the next pooled socket picks up the saved policy.
    let read_only = connection(
        &connection_id,
        Environment::Development,
        SafeMode::Inherit,
        true,
    );
    save(&state, &read_only).await;
    let mut pooled = crate::postgres::connect(&read_only)
        .await
        .expect("read-only pooled connection");
    let pooled_error = sqlx::query("CREATE TEMP TABLE safety_live_pool_write(id integer)")
        .execute(&mut *pooled)
        .await
        .expect_err("pooled write must fail");
    assert_eq!(
        pooled_error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("25006")
    );

    let spec =
        ResolvedPostgresConnectSpec::from_connection(&read_only).expect("read-only Postgres spec");
    let dedicated = dedicated::connect(&spec, NoticeSink::Ignore)
        .await
        .expect("read-only dedicated connection");
    let dedicated_error = dedicated
        .client
        .batch_execute("CREATE TEMP TABLE safety_live_dedicated_write(id integer)")
        .await
        .expect_err("dedicated write must fail");
    assert_eq!(
        dedicated_error.code().map(|code| code.code()),
        Some("25006")
    );
    crate::postgres::drop_pool(&connection_id);
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn safety_live_query_command_core_enforces_policy_and_records_audit() {
    let (_directory, state) = crate::test_app_state().await;
    let connection_id = format!("safety-policy-{}", uuid::Uuid::new_v4().simple());
    let schema = format!("safety_live_{}", uuid::Uuid::new_v4().simple());
    let default_connection = connection(
        &connection_id,
        Environment::Development,
        SafeMode::Inherit,
        false,
    );
    let default_spec =
        ResolvedPostgresConnectSpec::from_connection(&default_connection).expect("Postgres spec");
    let admin = session_postgres::connect(&default_spec)
        .await
        .expect("admin session");
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; \
             CREATE TABLE {schema}.rows(id integer PRIMARY KEY, value integer); \
             INSERT INTO {schema}.rows VALUES (1, 0), (2, 0); \
             CREATE TABLE {schema}.drop_me(id integer)"
        ))
        .await
        .expect("fixture");

    let window = "safety-live-window";
    let strict = connection(
        &connection_id,
        Environment::Production,
        SafeMode::Inherit,
        false,
    );
    save(&state, &strict).await;
    let strict_events = Arc::new(AtomicUsize::new(0));
    open_session(&state, &strict, window, "strict", strict_events.clone()).await;
    let strict_sql = format!("UPDATE {schema}.rows SET value = 1 WHERE id = 1");
    let before_refusal = strict_events.load(Ordering::SeqCst);
    assert!(matches!(
        crate::commands::query_session::execute_query_session_inner(
            &state,
            window,
            execute_payload("strict", "strict-write", strict_sql.clone(), false),
        )
        .await,
        Err(QuerySessionError::PolicyNeedsConfirmation { .. })
    ));
    assert_eq!(strict_events.load(Ordering::SeqCst), before_refusal);
    wait_for_i32(
        &admin,
        &format!("SELECT value FROM {schema}.rows WHERE id = 1"),
        0,
    )
    .await;
    assert!(
        crate::storage::read_safety_overrides(&state.pool, &connection_id)
            .await
            .expect("audit rows after query refusal")
            .is_empty()
    );
    assert!(
        crate::storage::read_connection_by_id(&state.pool, &connection_id)
            .await
            .expect("read refused query connection")
            .expect("stored connection")
            .last_activity_at()
            .is_none()
    );

    crate::commands::query_session::execute_query_session_inner(
        &state,
        window,
        execute_payload("strict", "strict-write", strict_sql, true),
    )
    .await
    .expect("confirmed strict execution admitted");
    wait_for_i32(
        &admin,
        &format!("SELECT value FROM {schema}.rows WHERE id = 1"),
        1,
    )
    .await;
    wait_for_audit_count(&state, &connection_id, 1).await;
    let audit = crate::storage::read_safety_overrides(&state.pool, &connection_id)
        .await
        .expect("read query audit");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].command, "execute_query_session");
    assert_eq!(audit[0].classes, vec!["dml"]);

    let failed_events = Arc::new(AtomicUsize::new(0));
    open_session(
        &state,
        &strict,
        window,
        "strict-failure",
        failed_events.clone(),
    )
    .await;
    let failed_baseline = failed_events.load(Ordering::SeqCst);
    crate::commands::query_session::execute_query_session_inner(
        &state,
        window,
        execute_payload(
            "strict-failure",
            "failed-write",
            format!("UPDATE {schema}.missing_table SET value = 9"),
            true,
        ),
    )
    .await
    .expect("confirmed failing execution admitted");
    wait_for_event_count(&failed_events, failed_baseline + 2).await;
    assert_eq!(
        crate::storage::read_safety_overrides(&state.pool, &connection_id)
            .await
            .expect("audits after failed execution")
            .len(),
        1
    );

    let protected = connection(
        &connection_id,
        Environment::Staging,
        SafeMode::Inherit,
        false,
    );
    save(&state, &protected).await;

    let bounded_events = Arc::new(AtomicUsize::new(0));
    open_session(
        &state,
        &protected,
        window,
        "protected-bounded",
        bounded_events,
    )
    .await;
    crate::commands::query_session::execute_query_session_inner(
        &state,
        window,
        execute_payload(
            "protected-bounded",
            "bounded-write",
            format!("UPDATE {schema}.rows SET value = 2 WHERE id = 2"),
            false,
        ),
    )
    .await
    .expect("protected bounded write admitted");
    wait_for_i32(
        &admin,
        &format!("SELECT value FROM {schema}.rows WHERE id = 2"),
        2,
    )
    .await;

    let delete_events = Arc::new(AtomicUsize::new(0));
    open_session(
        &state,
        &protected,
        window,
        "protected-delete",
        delete_events.clone(),
    )
    .await;
    let delete_sql = format!("DELETE FROM {schema}.rows");
    let before_delete = delete_events.load(Ordering::SeqCst);
    assert!(matches!(
        crate::commands::query_session::execute_query_session_inner(
            &state,
            window,
            execute_payload("protected-delete", "delete", delete_sql.clone(), false),
        )
        .await,
        Err(QuerySessionError::PolicyNeedsConfirmation { .. })
    ));
    assert_eq!(delete_events.load(Ordering::SeqCst), before_delete);
    crate::commands::query_session::execute_query_session_inner(
        &state,
        window,
        execute_payload("protected-delete", "delete", delete_sql, true),
    )
    .await
    .expect("confirmed protected delete admitted");
    wait_for_i32(
        &admin,
        &format!("SELECT count(*)::integer FROM {schema}.rows"),
        0,
    )
    .await;

    let drop_events = Arc::new(AtomicUsize::new(0));
    open_session(
        &state,
        &protected,
        window,
        "protected-drop",
        drop_events.clone(),
    )
    .await;
    let drop_sql = format!("DROP TABLE {schema}.drop_me");
    let before_drop = drop_events.load(Ordering::SeqCst);
    assert!(matches!(
        crate::commands::query_session::execute_query_session_inner(
            &state,
            window,
            execute_payload("protected-drop", "drop", drop_sql.clone(), false),
        )
        .await,
        Err(QuerySessionError::PolicyNeedsConfirmation { .. })
    ));
    assert_eq!(drop_events.load(Ordering::SeqCst), before_drop);
    crate::commands::query_session::execute_query_session_inner(
        &state,
        window,
        execute_payload("protected-drop", "drop", drop_sql, true),
    )
    .await
    .expect("confirmed protected drop admitted");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let exists: bool = admin
                .client
                .query_one(
                    "SELECT to_regclass($1) IS NOT NULL",
                    &[&format!("{schema}.drop_me")],
                )
                .await
                .expect("inspect dropped table")
                .get(0);
            if !exists {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("drop effect observed");

    let read_only = connection(
        &connection_id,
        Environment::Development,
        SafeMode::Inherit,
        true,
    );
    save(&state, &read_only).await;
    let read_only_events = Arc::new(AtomicUsize::new(0));
    open_session(
        &state,
        &read_only,
        window,
        "read-only",
        read_only_events.clone(),
    )
    .await;
    let before_read_only = read_only_events.load(Ordering::SeqCst);
    assert!(matches!(
        crate::commands::query_session::execute_query_session_inner(
            &state,
            window,
            execute_payload(
                "read-only",
                "blocked-write",
                format!("UPDATE {schema}.rows SET value = 3 WHERE id = 1"),
                true,
            ),
        )
        .await,
        Err(QuerySessionError::PolicyBlocked { .. })
    ));
    assert_eq!(read_only_events.load(Ordering::SeqCst), before_read_only);

    state.query_sessions.close_window(window).await;
    admin
        .client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .expect("cleanup");
}
