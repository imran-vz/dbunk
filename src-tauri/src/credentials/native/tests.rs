use super::*;
use crate::keychain::{testing, ReadPolicy};

struct Fixture {
    _directory: tempfile::TempDir,
    context: Arc<Context>,
    io: Arc<testing::RecordingStore>,
    namespace: String,
}

impl Fixture {
    async fn new(mode: CredentialStorageMode) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let pool = storage::open_pool(&storage::Paths::from_dir(directory.path().into()))
            .await
            .unwrap();
        let io = Arc::new(testing::RecordingStore::default());
        let namespace = uuid::Uuid::new_v4().to_string();
        let context = Context::new(
            pool,
            Arc::new(testing::store(&io, &namespace, ReadPolicy::Strict)),
            LifecyclePolicy::Native,
        );
        super::super::configure(&context, mode, Some("original-password"))
            .await
            .unwrap();
        write_all(&context, mode, &entries()).await.unwrap();
        Self {
            _directory: directory,
            context,
            io,
            namespace,
        }
    }

    fn reopen(&self) -> Arc<Context> {
        Context::new(
            self.context.pool.clone(),
            Arc::new(testing::store(
                &self.io,
                &self.namespace,
                ReadPolicy::Strict,
            )),
            LifecyclePolicy::Native,
        )
    }

    fn assert_scoped(&self) {
        for (_, service, account) in &self.io.state.lock().unwrap().calls {
            assert_eq!(service, &format!("dbunk-native-stage04-{}", self.namespace));
            assert_eq!(
                account,
                &format!("connection-credentials-{}", self.namespace)
            );
        }
    }
}

fn entries() -> HashMap<String, String> {
    HashMap::from([("same-id".into(), "private-fixture-secret".into())])
}

async fn fail_mode_commit(context: &Context) {
    sqlx::query("CREATE TRIGGER fail_mode BEFORE INSERT ON app_settings WHEN NEW.key = 'credentialStorageMode' BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END")
        .execute(&context.pool).await.unwrap();
}

async fn allow_mode_commit(context: &Context) {
    sqlx::query("DROP TRIGGER fail_mode")
        .execute(&context.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn every_mode_pair_survives_reopen_without_cross_profile_calls() {
    for from in ALL_MODES {
        for to in ALL_MODES {
            let fixture = Fixture::new(from).await;
            change_mode(&fixture.context, from, to, Some("new-password"))
                .await
                .unwrap();
            assert!(!recovery_required(&fixture.context).await.unwrap());
            let reopened = fixture.reopen();
            if to == CredentialStorageMode::EncryptedSqlite {
                assert!(unlock(&reopened, "wrong-password").await.is_err());
                unlock(&reopened, "new-password").await.unwrap();
            }
            assert_eq!(read_all(&reopened, to).await.unwrap(), entries());
            assert_eq!(credential_mode(&reopened.pool).await.unwrap(), Some(to));
            reset(&reopened).await.unwrap();
            assert!(!onboarding_completed(&reopened.pool).await.unwrap());
            assert_eq!(credential_mode(&reopened.pool).await.unwrap(), None);
            fixture.assert_scoped();
        }
    }
}

#[tokio::test]
async fn sqlite_only_lifecycle_never_contacts_keychain() {
    for mode in [
        CredentialStorageMode::PlainSqlite,
        CredentialStorageMode::EncryptedSqlite,
    ] {
        let fixture = Fixture::new(mode).await;
        change_mode(
            &fixture.context,
            mode,
            CredentialStorageMode::PlainSqlite,
            None,
        )
        .await
        .unwrap();
        reset(&fixture.context).await.unwrap();
        assert!(fixture.io.state.lock().unwrap().calls.is_empty());
    }
}

#[tokio::test]
async fn denied_staging_and_failed_commit_preserve_old_store_and_recover_on_reopen() {
    for mode in [
        CredentialStorageMode::PlainSqlite,
        CredentialStorageMode::EncryptedSqlite,
    ] {
        for deny_write in [false, true] {
            let fixture = Fixture::new(mode).await;
            let previous_rows = storage::read_sqlite_credentials(&fixture.context.pool)
                .await
                .unwrap();
            let previous_verifier = storage::read_verifier(&fixture.context.pool).await.unwrap();
            if deny_write {
                fixture.io.state.lock().unwrap().deny_write = true;
            } else {
                fail_mode_commit(&fixture.context).await;
            }
            assert!(change_mode(
                &fixture.context,
                mode,
                CredentialStorageMode::Keychain,
                None
            )
            .await
            .is_err());
            assert!(recovery_required(&fixture.context).await.unwrap());
            assert_eq!(
                storage::get_setting(&fixture.context.pool, JOURNAL_KEY)
                    .await
                    .unwrap()
                    .as_deref(),
                Some(ROLLBACK_KEYCHAIN)
            );
            assert_eq!(
                storage::read_sqlite_credentials(&fixture.context.pool)
                    .await
                    .unwrap(),
                previous_rows
            );
            assert_eq!(
                storage::read_verifier(&fixture.context.pool).await.unwrap(),
                previous_verifier
            );
            assert!(read_all_cached(&fixture.context, mode).await.is_err());
            let reopened = fixture.reopen();
            fixture.io.state.lock().unwrap().deny_write = false;
            if !deny_write {
                allow_mode_commit(&reopened).await;
            }
            recover(&reopened).await.unwrap();
            recover(&reopened).await.unwrap();
            if mode == CredentialStorageMode::EncryptedSqlite {
                unlock(&reopened, "original-password").await.unwrap();
            }
            assert_eq!(read_all(&reopened, mode).await.unwrap(), entries());
            assert!(fixture.io.state.lock().unwrap().blobs.is_empty());
            fixture.assert_scoped();
        }
    }
}

#[tokio::test]
async fn failed_keychain_to_sqlite_commit_preserves_source_and_cleanup_is_retryable() {
    for mode in [
        CredentialStorageMode::PlainSqlite,
        CredentialStorageMode::EncryptedSqlite,
    ] {
        let fixture = Fixture::new(CredentialStorageMode::Keychain).await;
        fail_mode_commit(&fixture.context).await;
        assert!(change_mode(
            &fixture.context,
            CredentialStorageMode::Keychain,
            mode,
            Some("new-password")
        )
        .await
        .is_err());
        assert!(!recovery_required(&fixture.context).await.unwrap());
        assert_eq!(
            read_all(&fixture.context, CredentialStorageMode::Keychain)
                .await
                .unwrap(),
            entries()
        );
        assert_eq!(
            credential_mode(&fixture.context.pool).await.unwrap(),
            Some(CredentialStorageMode::Keychain)
        );
        allow_mode_commit(&fixture.context).await;
        fixture.io.state.lock().unwrap().deny_write = true;
        assert!(change_mode(
            &fixture.context,
            CredentialStorageMode::Keychain,
            mode,
            Some("new-password")
        )
        .await
        .is_err());
        assert!(recovery_required(&fixture.context).await.unwrap());
        assert_eq!(
            credential_mode(&fixture.context.pool).await.unwrap(),
            Some(mode)
        );
        assert!(read_all(&fixture.context, mode).await.is_err());
        let reopened = fixture.reopen();
        assert!(recover(&reopened).await.is_err());
        fixture.io.state.lock().unwrap().deny_write = false;
        recover(&reopened).await.unwrap();
        if mode == CredentialStorageMode::EncryptedSqlite {
            unlock(&reopened, "new-password").await.unwrap();
        }
        assert_eq!(read_all(&reopened, mode).await.unwrap(), entries());
        assert!(fixture.io.state.lock().unwrap().blobs.is_empty());
        fixture.assert_scoped();
    }
}

#[tokio::test]
async fn reset_denial_and_interrupted_sqlite_commit_finish_only_after_explicit_recovery() {
    let fixture = Fixture::new(CredentialStorageMode::Keychain).await;
    let connection =
        crate::app::test_postgres_connection("same-id", crate::SafeMode::default(), false);
    storage::upsert_connection(&fixture.context.pool, &connection)
        .await
        .unwrap();
    let drafts = vec![("ui.v1.native.workspace".into(), "SELECT 'draft'".into())];
    storage::upsert_ui_state(&fixture.context.pool, &drafts)
        .await
        .unwrap();
    fixture.io.state.lock().unwrap().deny_write = true;
    assert!(reset(&fixture.context).await.is_err());
    assert!(recovery_required(&fixture.context).await.unwrap());
    assert!(!fixture.io.state.lock().unwrap().blobs.is_empty());
    fixture.io.state.lock().unwrap().deny_write = false;
    sqlx::query("CREATE TRIGGER fail_reset BEFORE INSERT ON app_settings WHEN NEW.key = 'onboardingCompleted' BEGIN SELECT RAISE(ABORT, 'injected reset failure'); END")
        .execute(&fixture.context.pool).await.unwrap();
    let reopened = fixture.reopen();
    assert!(recover(&reopened).await.is_err());
    assert!(fixture.io.state.lock().unwrap().blobs.is_empty());
    assert!(recovery_required(&reopened).await.unwrap());
    assert!(read_all(&reopened, CredentialStorageMode::Keychain)
        .await
        .is_err());
    sqlx::query("DROP TRIGGER fail_reset")
        .execute(&reopened.pool)
        .await
        .unwrap();
    recover(&reopened).await.unwrap();
    assert!(!onboarding_completed(&reopened.pool).await.unwrap());
    assert!(storage::read_connection_by_id(&reopened.pool, "same-id")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        storage::read_ui_state(&reopened.pool).await.unwrap(),
        drafts
    );
    fixture.assert_scoped();
}

#[tokio::test]
async fn strict_denied_corrupt_and_nonempty_reads_never_advance_onboarding() {
    for kind in ["denied", "corrupt", "nonempty"] {
        let fixture = Fixture::new(CredentialStorageMode::PlainSqlite).await;
        reset(&fixture.context).await.unwrap();
        {
            let mut io = fixture.io.state.lock().unwrap();
            if kind == "denied" {
                io.deny_read = true;
            } else {
                io.blobs.insert(
                    (
                        format!("dbunk-native-stage04-{}", fixture.namespace),
                        format!("connection-credentials-{}", fixture.namespace),
                    ),
                    if kind == "corrupt" {
                        "private-fixture-secret invalid JSON".into()
                    } else {
                        serde_json::to_string(&entries()).unwrap()
                    },
                );
            }
        }
        let reopened = fixture.reopen();
        let error = configure(&reopened, CredentialStorageMode::Keychain, None)
            .await
            .unwrap_err();
        assert!(!error.contains("private-fixture-secret"));
        assert!(!onboarding_completed(&reopened.pool).await.unwrap());
        assert!(!recovery_required(&reopened).await.unwrap());
        assert!(fixture
            .io
            .state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|(op, _, _)| *op == "read"));
        fixture.assert_scoped();
    }
}

#[tokio::test]
async fn unknown_recovery_record_never_deletes_any_store() {
    let fixture = Fixture::new(CredentialStorageMode::PlainSqlite).await;
    storage::set_setting(&fixture.context.pool, JOURNAL_KEY, "future-version")
        .await
        .unwrap();
    assert!(recover(&fixture.context).await.is_err());
    assert!(
        read_all_cached(&fixture.context, CredentialStorageMode::PlainSqlite)
            .await
            .is_err()
    );
    assert!(fixture.io.state.lock().unwrap().calls.is_empty());
    assert!(recovery_required(&fixture.context).await.unwrap());
}

#[tokio::test]
async fn inconsistent_recovery_record_preserves_active_keychain() {
    let fixture = Fixture::new(CredentialStorageMode::Keychain).await;
    storage::set_setting(&fixture.context.pool, JOURNAL_KEY, ROLLBACK_KEYCHAIN)
        .await
        .unwrap();
    let before = fixture.io.state.lock().unwrap().blobs.clone();
    assert!(recover(&fixture.context).await.is_err());
    assert_eq!(fixture.io.state.lock().unwrap().blobs, before);
    assert!(recovery_required(&fixture.context).await.unwrap());
}

#[tokio::test]
async fn cleanup_marker_survives_sqlite_failure_after_keychain_deletion() {
    let fixture = Fixture::new(CredentialStorageMode::Keychain).await;
    sqlx::query("CREATE TRIGGER fail_cleanup BEFORE DELETE ON app_settings WHEN OLD.key = 'native.credentials.transition.v1' BEGIN SELECT RAISE(ABORT, 'injected cleanup failure'); END")
        .execute(&fixture.context.pool).await.unwrap();
    assert!(change_mode(
        &fixture.context,
        CredentialStorageMode::Keychain,
        CredentialStorageMode::PlainSqlite,
        None
    )
    .await
    .is_err());
    assert!(fixture.io.state.lock().unwrap().blobs.is_empty());
    assert!(recovery_required(&fixture.context).await.unwrap());
    let reopened = fixture.reopen();
    assert!(recover(&reopened).await.is_err());
    sqlx::query("DROP TRIGGER fail_cleanup")
        .execute(&reopened.pool)
        .await
        .unwrap();
    recover(&reopened).await.unwrap();
    assert_eq!(
        read_all(&reopened, CredentialStorageMode::PlainSqlite)
            .await
            .unwrap(),
        entries()
    );
    fixture.assert_scoped();
}
