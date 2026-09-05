use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::commands::{bastions, connections, settings};
use crate::postgres::backup::{
    manager::JobContext,
    protocol::{PgBackupFormat, PgBackupScope, PgToolJobError, StartPgBackupPayload},
    runner::{Ready, Request},
};
use crate::socket_lifecycle::{self, CacheInvalidation};
use crate::{
    AppState, BastionAuthMethod, ConnectionOrganization, CredentialStorageMode, Environment,
    PgStoredConnection, SafeMode, SaveBastionServerPayload, SecretChange, SshTunnelConfig,
    StoredConnection,
};

struct HeldTermination {
    cancellation: oneshot::Receiver<()>,
    allow_termination: Option<oneshot::Sender<()>>,
    terminated: oneshot::Receiver<()>,
}

impl HeldTermination {
    async fn wait_for_cancellation(&mut self) {
        tokio::time::timeout(Duration::from_secs(1), &mut self.cancellation)
            .await
            .expect("lifecycle requested cancellation")
            .expect("worker reported cancellation");
    }

    async fn terminate(mut self) {
        self.allow_termination
            .take()
            .expect("termination permit retained")
            .send(())
            .expect("worker retained termination permit");
        tokio::time::timeout(Duration::from_secs(1), self.terminated)
            .await
            .expect("worker terminated")
            .expect("worker reported termination");
    }
}

fn start_held_job(state: &AppState, connection_id: &str) -> HeldTermination {
    let payload = StartPgBackupPayload {
        connection_id: connection_id.to_string(),
        destination_path: "/unused/archive.sql".to_string(),
        format: PgBackupFormat::Plain,
        scope: PgBackupScope::Database,
        clean: false,
    };
    let (cancelled_tx, cancellation) = oneshot::channel();
    let (allow_termination, terminate_rx) = oneshot::channel();
    let (terminated_tx, terminated) = oneshot::channel();
    state
        .pg_tool_jobs
        .start(
            state.pg_tool_jobs.admission(connection_id).unwrap(),
            Request::Backup(payload).snapshot(),
            move |context: JobContext| async move {
                context.cancelled().await;
                let _ = cancelled_tx.send(());
                let _ = terminate_rx.await;
                let _ = terminated_tx.send(());
                Err::<Ready, _>(PgToolJobError::Cancelled)
            },
            Box::pin(async {}),
        )
        .expect("admit held job");
    HeldTermination {
        cancellation,
        allow_termination: Some(allow_termination),
        terminated,
    }
}

fn pg_connection(connection_id: &str, bastion_id: Option<&str>) -> StoredConnection {
    StoredConnection::PostgreSQL(PgStoredConnection {
        id: connection_id.to_string(),
        name: format!("Connection {connection_id}"),
        database: "postgres".to_string(),
        host: "database.internal".to_string(),
        port: 5432,
        user: "postgres".to_string(),
        password: "secret".to_string(),
        role: "read/write".to_string(),
        environment: Environment::Development,
        safe_mode: SafeMode::Inherit,
        read_only: false,
        last_activity_at: None,
        organization: ConnectionOrganization::default(),
        ssl: false,
        tls_options: None,
        driver_options: None,
        ssh_tunnel: match bastion_id {
            Some(bastion_id) => SshTunnelConfig {
                enabled: true,
                bastion_server_id: Some(bastion_id.to_string()),
                ..SshTunnelConfig::default()
            },
            None => SshTunnelConfig::default(),
        },
    })
}

fn bastion_payload(id: &str, name: &str, password: SecretChange) -> SaveBastionServerPayload {
    SaveBastionServerPayload {
        id: id.to_string(),
        name: name.to_string(),
        host: "bastion.internal".to_string(),
        port: 22,
        user: "operator".to_string(),
        auth_method: BastionAuthMethod::Password,
        private_key_path: None,
        password,
        private_key_content: SecretChange::Keep,
        passphrase: SecretChange::Keep,
    }
}

async fn assert_connection_present(state: &AppState, connection_id: &str, expected: bool) {
    let present = crate::storage::read_connections(&state.pool)
        .await
        .expect("read connections")
        .iter()
        .any(|connection| connection.id() == connection_id);
    assert_eq!(present, expected);
}

fn assert_admission_blocked(state: &AppState, connection_id: &str) {
    assert!(matches!(
        state.pg_tool_jobs.admission(connection_id),
        Err(PgToolJobError::ConnectionClosing)
    ));
}

fn assert_admission_reopened(state: &AppState, connection_id: &str) {
    assert!(state.pg_tool_jobs.admission(connection_id).is_ok());
}

fn assert_invalidation_not_observed(
    invalidations: &mut tokio::sync::mpsc::UnboundedReceiver<CacheInvalidation>,
    expected: &CacheInvalidation,
) {
    while let Ok(actual) = invalidations.try_recv() {
        assert_ne!(&actual, expected, "cache invalidated before termination");
    }
}

async fn receive_invalidation(
    invalidations: &mut tokio::sync::mpsc::UnboundedReceiver<CacheInvalidation>,
    expected: CacheInvalidation,
) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if invalidations.recv().await.as_ref() == Some(&expected) {
                return;
            }
        }
    })
    .await
    .expect("expected cache invalidation");
}

#[tokio::test]
#[serial_test::serial]
async fn disconnect_waits_for_job_termination_before_cache_invalidation() {
    let (_directory, state) = crate::test_app_state().await;
    let state = Arc::new(state);
    let mut job = start_held_job(&state, "disconnect");
    let expected = CacheInvalidation::Connection("disconnect".to_string());
    let (_observer, mut invalidations) =
        socket_lifecycle::observe_cache_invalidations([expected.clone()]);

    let command_state = state.clone();
    let command = tokio::spawn(async move {
        connections::disconnect_connection_inner(&command_state, "disconnect").await
    });

    job.wait_for_cancellation().await;
    assert!(!command.is_finished());
    assert_admission_blocked(&state, "disconnect");
    assert_invalidation_not_observed(&mut invalidations, &expected);

    job.terminate().await;
    command.await.expect("disconnect task").expect("disconnect");
    receive_invalidation(&mut invalidations, expected).await;
    assert_admission_reopened(&state, "disconnect");
}

#[tokio::test]
#[serial_test::serial]
async fn delete_waits_for_job_termination_before_record_and_cache_removal() {
    let (_directory, state) = crate::test_app_state().await;
    connections::save_connection_inner(&state, pg_connection("delete", None))
        .await
        .expect("save connection");
    let state = Arc::new(state);
    let mut job = start_held_job(&state, "delete");
    let expected = CacheInvalidation::Connection("delete".to_string());
    let (_observer, mut invalidations) =
        socket_lifecycle::observe_cache_invalidations([expected.clone()]);

    let command_state = state.clone();
    let command = tokio::spawn(async move {
        connections::delete_connection_inner(&command_state, "delete").await
    });

    job.wait_for_cancellation().await;
    assert!(!command.is_finished());
    assert_admission_blocked(&state, "delete");
    assert_connection_present(&state, "delete", true).await;
    assert!(
        crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read credentials")
            .contains_key("delete")
    );
    assert_invalidation_not_observed(&mut invalidations, &expected);

    job.terminate().await;
    command.await.expect("delete task").expect("delete");
    assert_connection_present(&state, "delete", false).await;
    assert!(
        !crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read credentials")
            .contains_key("delete")
    );
    receive_invalidation(&mut invalidations, expected).await;
    assert_admission_reopened(&state, "delete");
}

#[tokio::test]
#[serial_test::serial]
async fn credential_reset_waits_for_all_jobs_then_reopens_global_admission() {
    let (_directory, state) = crate::test_app_state().await;
    for connection_id in ["reset-a", "reset-b"] {
        connections::save_connection_inner(&state, pg_connection(connection_id, None))
            .await
            .expect("save connection");
    }
    let state = Arc::new(state);
    let mut first = start_held_job(&state, "reset-a");
    let mut second = start_held_job(&state, "reset-b");

    let command_state = state.clone();
    let command =
        tokio::spawn(async move { settings::reset_credential_storage_inner(&command_state).await });

    first.wait_for_cancellation().await;
    second.wait_for_cancellation().await;
    assert!(!command.is_finished());
    assert_admission_blocked(&state, "reset-a");
    assert_admission_blocked(&state, "unrelated");
    assert!(crate::credentials::onboarding_completed(&state.pool)
        .await
        .expect("read onboarding state"));
    assert_eq!(
        crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read credentials")
            .len(),
        2
    );

    first.terminate().await;
    second.terminate().await;
    command
        .await
        .expect("reset task")
        .expect("reset credentials");
    assert!(!crate::credentials::onboarding_completed(&state.pool)
        .await
        .expect("read onboarding state"));
    assert!(
        crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read credentials")
            .is_empty()
    );
    assert_admission_reopened(&state, "reset-a");
    assert_admission_reopened(&state, "unrelated");
}

#[tokio::test]
#[serial_test::serial]
async fn bastion_save_and_host_key_reset_wait_for_referencing_job_termination() {
    let (_directory, state) = crate::test_app_state().await;
    bastions::save_bastion_server_inner(
        &state,
        bastion_payload(
            "bastion",
            "Original bastion",
            SecretChange::Set {
                value: "bastion-secret".to_string(),
            },
        ),
    )
    .await
    .expect("save initial bastion");
    crate::storage::bastions::update_bastion_host_key_fingerprint(
        &state.pool,
        "bastion",
        Some("SHA256:known"),
    )
    .await
    .expect("set host key");
    connections::save_connection_inner(&state, pg_connection("through-bastion", Some("bastion")))
        .await
        .expect("save tunneled connection");
    let state = Arc::new(state);
    let expected_bastion = CacheInvalidation::Bastion("bastion".to_string());
    let expected_connection = CacheInvalidation::Connection("through-bastion".to_string());
    let (_observer, mut invalidations) = socket_lifecycle::observe_cache_invalidations([
        expected_bastion.clone(),
        expected_connection.clone(),
    ]);

    let mut save_job = start_held_job(&state, "through-bastion");
    let command_state = state.clone();
    let save = tokio::spawn(async move {
        bastions::save_bastion_server_inner(
            &command_state,
            bastion_payload("bastion", "Updated bastion", SecretChange::Keep),
        )
        .await
    });
    save_job.wait_for_cancellation().await;
    assert!(!save.is_finished());
    assert_admission_blocked(&state, "through-bastion");
    // A connection added during the wait was absent from the old reference set.
    // It must still be unable to start work before the Bastion changes.
    connections::save_connection_inner(&state, pg_connection("new-during-save", Some("bastion")))
        .await
        .expect("add concurrent Bastion reference");
    assert_admission_blocked(&state, "new-during-save");
    let stored = crate::storage::bastions::read_bastion_server_by_id(&state.pool, "bastion")
        .await
        .expect("read bastion")
        .expect("stored bastion");
    assert_eq!(stored.name, "Original bastion");
    assert_invalidation_not_observed(&mut invalidations, &expected_bastion);
    save_job.terminate().await;
    save.await.expect("save task").expect("update bastion");
    let stored = crate::storage::bastions::read_bastion_server_by_id(&state.pool, "bastion")
        .await
        .expect("read bastion")
        .expect("stored bastion");
    assert_eq!(stored.name, "Updated bastion");
    receive_invalidation(&mut invalidations, expected_bastion.clone()).await;
    receive_invalidation(&mut invalidations, expected_connection.clone()).await;
    assert_admission_reopened(&state, "through-bastion");

    let mut reset_job = start_held_job(&state, "through-bastion");
    let command_state = state.clone();
    let reset = tokio::spawn(async move {
        bastions::reset_bastion_host_key_inner(&command_state, "bastion").await
    });
    reset_job.wait_for_cancellation().await;
    assert!(!reset.is_finished());
    assert_admission_blocked(&state, "through-bastion");
    connections::save_connection_inner(&state, pg_connection("new-during-reset", Some("bastion")))
        .await
        .expect("add concurrent host-key reset reference");
    assert_admission_blocked(&state, "new-during-reset");
    let stored = crate::storage::bastions::read_bastion_server_by_id(&state.pool, "bastion")
        .await
        .expect("read bastion")
        .expect("stored bastion");
    assert_eq!(stored.host_key_fingerprint.as_deref(), Some("SHA256:known"));
    assert_invalidation_not_observed(&mut invalidations, &expected_bastion);
    reset_job.terminate().await;
    reset.await.expect("reset task").expect("reset host key");
    let stored = crate::storage::bastions::read_bastion_server_by_id(&state.pool, "bastion")
        .await
        .expect("read bastion")
        .expect("stored bastion");
    assert!(stored.host_key_fingerprint.is_none());
    receive_invalidation(&mut invalidations, expected_bastion).await;
    receive_invalidation(&mut invalidations, expected_connection).await;
    assert_admission_reopened(&state, "through-bastion");
}

#[tokio::test]
#[serial_test::serial]
async fn exit_close_join_waits_for_job_termination_within_the_existing_budget() {
    let (_directory, state) = crate::test_app_state().await;
    let mut first = start_held_job(&state, "exit-a");
    let mut second = start_held_job(&state, "exit-b");
    let pg_tool_jobs = state.pg_tool_jobs.clone();
    let started = Instant::now();
    let close = tokio::spawn(crate::close_socket_managers_for_exit(
        state.query_sessions.clone(),
        state.table_browse.clone(),
        state.result_mutations.clone(),
        pg_tool_jobs.clone(),
    ));

    first.wait_for_cancellation().await;
    second.wait_for_cancellation().await;
    assert!(!close.is_finished());
    first.terminate().await;
    second.terminate().await;
    tokio::time::timeout(Duration::from_secs(3), close)
        .await
        .expect("exit close stayed within its budget")
        .expect("exit close task");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        pg_tool_jobs.list(None).len(),
        2,
        "both exit-time jobs remain observable as terminal records"
    );
    assert!(pg_tool_jobs
        .list(None)
        .iter()
        .all(|job| job.phase.terminal()));
}

#[tokio::test]
#[serial_test::serial]
async fn credential_configuration_and_migration_wait_for_jobs_before_changing_storage() {
    for migrate in [false, true] {
        let (_directory, state) = crate::test_app_state().await;
        connections::save_connection_inner(&state, pg_connection("credential-change", None))
            .await
            .unwrap();
        let state = Arc::new(state);
        let mut job = start_held_job(&state, "credential-change");
        let command_state = state.clone();
        let command = tokio::spawn(async move {
            if migrate {
                settings::change_credential_storage_inner(
                    &command_state,
                    crate::ChangeCredentialStoragePayload {
                        mode: CredentialStorageMode::Keychain,
                        password: None,
                        confirm: true,
                    },
                )
                .await
            } else {
                settings::configure_credential_storage_inner(
                    &command_state,
                    crate::ConfigureCredentialStoragePayload {
                        mode: CredentialStorageMode::PlainSqlite,
                        password: None,
                    },
                )
                .await
            }
        });
        job.wait_for_cancellation().await;
        assert!(!command.is_finished());
        assert_admission_blocked(&state, "new-during-credential-change");
        assert!(
            crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
                .await
                .unwrap()
                .contains_key("credential-change")
        );
        job.terminate().await;
        command.await.unwrap().unwrap();
        assert_eq!(
            crate::commands::current_credential_mode(&state)
                .await
                .unwrap(),
            if migrate {
                CredentialStorageMode::Keychain
            } else {
                CredentialStorageMode::PlainSqlite
            }
        );
        assert_admission_reopened(&state, "new-during-credential-change");
    }
}

#[tokio::test]
#[serial_test::serial]
async fn bastion_delete_closes_admission_before_checking_new_references() {
    let (_directory, state) = crate::test_app_state().await;
    bastions::save_bastion_server_inner(
        &state,
        bastion_payload(
            "delete-bastion",
            "Bastion",
            SecretChange::Set {
                value: "secret".into(),
            },
        ),
    )
    .await
    .unwrap();
    let state = Arc::new(state);
    let mut job = start_held_job(&state, "unrelated-before-delete");
    let command_state = state.clone();
    let command = tokio::spawn(async move {
        bastions::delete_bastion_server_inner(&command_state, "delete-bastion").await
    });
    job.wait_for_cancellation().await;
    connections::save_connection_inner(
        &state,
        pg_connection("new-before-delete", Some("delete-bastion")),
    )
    .await
    .unwrap();
    assert_admission_blocked(&state, "new-before-delete");
    job.terminate().await;
    assert!(command.await.unwrap().unwrap_err().contains("reference it"));
    assert!(
        crate::storage::bastions::read_bastion_server_by_id(&state.pool, "delete-bastion")
            .await
            .unwrap()
            .is_some()
    );
    assert_admission_reopened(&state, "new-before-delete");
}
