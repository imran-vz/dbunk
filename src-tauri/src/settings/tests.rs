use super::*;
use crate::postgres::schema_compare::manager::CompareManager;

#[tokio::test]
#[serial_test::serial]
async fn credential_mutations_join_affected_jobs_before_changing_storage() {
    use crate::postgres::backup::{
        manager::JobContext,
        protocol::{PgBackupFormat, PgBackupScope, PgToolJobError, StartPgBackupPayload},
        runner::{Ready, Request},
    };
    use std::{sync::Arc, time::Duration};

    for operation in ["configure", "change", "reset"] {
        let (_directory, state) = crate::test_app_state().await;
        let state = Arc::new(state);
        let (cancelled, cancellation) = tokio::sync::oneshot::channel();
        let (release, termination) = tokio::sync::oneshot::channel();
        state
            .pg_tool_jobs
            .start(
                state.pg_tool_jobs.admission("held-fixture").unwrap(),
                Request::Backup(StartPgBackupPayload {
                    connection_id: "held-fixture".into(),
                    destination_path: "/unused".into(),
                    format: PgBackupFormat::Plain,
                    scope: PgBackupScope::Database,
                    clean: false,
                })
                .snapshot(),
                move |context: JobContext| async move {
                    context.cancelled().await;
                    cancelled.send(()).unwrap();
                    termination.await.unwrap();
                    Err::<Ready, _>(PgToolJobError::Cancelled)
                },
                Box::pin(async {}),
            )
            .unwrap();
        let work_state = state.clone();
        let work = tokio::spawn(async move {
            match operation {
                "configure" => {
                    configure_credential_storage(
                        &work_state,
                        ConfigureCredentialStoragePayload {
                            mode: CredentialStorageMode::EncryptedSqlite,
                            password: Some("test-only".into()),
                        },
                    )
                    .await
                }
                "change" => {
                    change_credential_storage(
                        &work_state,
                        ChangeCredentialStoragePayload {
                            mode: CredentialStorageMode::EncryptedSqlite,
                            password: Some("test-only".into()),
                            confirm: true,
                        },
                    )
                    .await
                }
                "reset" => reset_credential_storage(&work_state).await,
                _ => unreachable!(),
            }
        });
        tokio::time::timeout(Duration::from_secs(2), cancellation)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !work.is_finished(),
            "{operation} must wait for job termination"
        );
        assert!(state.pg_tool_jobs.admission("another-fixture").is_err());
        assert_eq!(
            credentials::credential_mode(&state.pool).await.unwrap(),
            Some(CredentialStorageMode::PlainSqlite)
        );
        assert!(credentials::onboarding_completed(&state.pool)
            .await
            .unwrap());
        release.send(()).unwrap();
        let snapshot = tokio::time::timeout(Duration::from_secs(2), work)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(state.pg_tool_jobs.admission("another-fixture").is_ok());
        if operation == "reset" {
            assert!(matches!(
                snapshot.credential_state,
                CredentialState::NeedsOnboarding
            ));
        } else {
            assert_eq!(
                snapshot.credential_storage_mode,
                Some(CredentialStorageMode::EncryptedSqlite)
            );
        }
        credentials::lock_for_tests(&state.credentials);
    }
}

async fn fresh_state() -> (tempfile::TempDir, AppState) {
    crate::configure_test_keyring();
    let directory = tempfile::tempdir().unwrap();
    let paths = storage::Paths::from_dir(directory.path().to_owned());
    let pool = storage::open_pool(&paths).await.unwrap();
    (directory, AppState::new(pool, paths, CompareManager::new()))
}

#[tokio::test]
#[serial_test::serial]
async fn onboarding_unlock_and_theme_snapshots_preserve_the_wire_contract() {
    let (_directory, state) = fresh_state().await;
    let snapshot = load_app_settings(&state).await.unwrap();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        serde_json::json!({
            "onboardingCompleted": false,
            "credentialStorageMode": null,
            "credentialState": "needs-onboarding",
            "configDir": state.paths.config_dir().display().to_string(),
        })
    );
    let snapshot = configure_credential_storage(
        &state,
        ConfigureCredentialStoragePayload {
            mode: CredentialStorageMode::EncryptedSqlite,
            password: Some("test-only passphrase".into()),
        },
    )
    .await
    .unwrap();
    assert!(matches!(snapshot.credential_state, CredentialState::Ready));
    credentials::lock_for_tests(&state.credentials);
    assert!(matches!(
        load_app_settings(&state).await.unwrap().credential_state,
        CredentialState::NeedsUnlock
    ));
    assert!(unlock_credentials(
        &state,
        UnlockCredentialsPayload {
            password: "wrong".into()
        }
    )
    .await
    .is_err());
    assert!(matches!(
        load_app_settings(&state).await.unwrap().credential_state,
        CredentialState::NeedsUnlock
    ));
    unlock_credentials(
        &state,
        UnlockCredentialsPayload {
            password: "test-only passphrase".into(),
        },
    )
    .await
    .unwrap();
    let snapshot = save_app_settings(
        &state,
        SaveAppSettingsPayload {
            theme: Some("dark".into()),
            theme_preset: Some("github".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap(),
        serde_json::json!({
            "onboardingCompleted": true,
            "credentialStorageMode": "encrypted-sqlite",
            "credentialState": "ready",
            "configDir": state.paths.config_dir().display().to_string(),
            "theme": "dark", "themePreset": "github",
        })
    );
    credentials::lock_for_tests(&state.credentials);
}

#[tokio::test]
#[serial_test::serial]
async fn refused_configuration_and_unavailable_storage_do_not_report_ready() {
    let (_directory, state) = fresh_state().await;
    assert!(configure_credential_storage(
        &state,
        ConfigureCredentialStoragePayload {
            mode: CredentialStorageMode::EncryptedSqlite,
            password: None,
        }
    )
    .await
    .is_err());
    assert!(matches!(
        load_app_settings(&state).await.unwrap().credential_state,
        CredentialState::NeedsOnboarding
    ));
    state.pool.close().await;
    assert!(load_app_settings(&state).await.is_err());
    assert!(configure_credential_storage(
        &state,
        ConfigureCredentialStoragePayload {
            mode: CredentialStorageMode::PlainSqlite,
            password: None,
        }
    )
    .await
    .is_err());
}

#[tokio::test]
#[serial_test::serial]
async fn mode_change_requires_confirmation_and_reset_preserves_metadata_and_drafts() {
    let (_directory, state) = crate::test_app_state().await;
    let connection =
        crate::app::test_postgres_connection("settings-fixture", crate::SafeMode::Protected, false);
    crate::connections::save(&state, connection).await.unwrap();
    save_ui_state(
        &state,
        SaveUiStatePayload {
            entries: vec![UiStateEntry {
                key: "ui.v1.native.workspace".into(),
                value: "SELECT 'draft';".into(),
            }],
        },
    )
    .await
    .unwrap();
    assert!(change_credential_storage(
        &state,
        ChangeCredentialStoragePayload {
            mode: CredentialStorageMode::EncryptedSqlite,
            password: Some("test-only".into()),
            confirm: false,
        }
    )
    .await
    .unwrap_err()
    .contains("confirmed"));
    assert_eq!(
        credentials::credential_mode(&state.pool).await.unwrap(),
        Some(CredentialStorageMode::PlainSqlite)
    );
    let snapshot = reset_credential_storage(&state).await.unwrap();
    assert!(matches!(
        snapshot.credential_state,
        CredentialState::NeedsOnboarding
    ));
    assert_eq!(crate::connections::list(&state).await.unwrap().len(), 1);
    assert_eq!(
        load_ui_state(&state).await.unwrap()[0].value,
        "SELECT 'draft';"
    );
    assert!(
        credentials::read_all(&state.credentials, CredentialStorageMode::PlainSqlite)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn ui_service_keeps_namespace_validation_and_atomic_batch_writes() {
    let (_directory, state) = fresh_state().await;
    let saved = SaveUiStatePayload {
        entries: vec![UiStateEntry {
            key: "ui.v1.native.workspace".into(),
            value: "old draft".into(),
        }],
    };
    save_ui_state(&state, saved).await.unwrap();
    let refused = SaveUiStatePayload {
        entries: vec![
            UiStateEntry {
                key: "ui.v1.native.workspace".into(),
                value: "new draft".into(),
            },
            UiStateEntry {
                key: "credentialStorageMode".into(),
                value: "keychain".into(),
            },
        ],
    };
    assert!(save_ui_state(&state, refused).await.is_err());
    assert_eq!(load_ui_state(&state).await.unwrap()[0].value, "old draft");
    let payload: DeleteUiStatePayload = serde_json::from_value(serde_json::json!({
        "prefixes": ["ui.v1.native."]
    }))
    .unwrap();
    delete_ui_state(&state, payload).await.unwrap();
    assert!(load_ui_state(&state).await.unwrap().is_empty());
}
