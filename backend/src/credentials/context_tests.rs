use super::*;
use crate::keychain::{testing, ReadPolicy};

async fn fixture() -> (tempfile::TempDir, Arc<Context>) {
    let directory = tempfile::tempdir().unwrap();
    let paths = storage::Paths::from_dir(directory.path().to_owned());
    let pool = storage::open_pool(&paths).await.unwrap();
    (directory, Context::fixture(pool))
}

fn entries(secret: &str) -> HashMap<String, String> {
    HashMap::from([("same-id".into(), secret.into())])
}

#[tokio::test]
async fn profiles_with_overlapping_ids_have_independent_keys_caches_and_locks() {
    let (_a, a) = fixture().await;
    let (_b, b) = fixture().await;
    let mode = CredentialStorageMode::EncryptedSqlite;
    configure(&a, mode, Some("password-a")).await.unwrap();
    configure(&b, mode, Some("password-b")).await.unwrap();
    write_all(&a, mode, &entries("secret-a")).await.unwrap();
    write_all(&b, mode, &entries("secret-b")).await.unwrap();
    assert_eq!(
        read_all_cached(&a, mode).await.unwrap(),
        entries("secret-a")
    );
    assert_eq!(
        read_all_cached(&b, mode).await.unwrap(),
        entries("secret-b")
    );

    let held = mutation_guard(&a).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), reset(&b))
        .await
        .unwrap()
        .unwrap();
    drop(held);
    assert!(is_unlocked(&a));
    assert!(!is_unlocked(&b));
    assert_eq!(
        read_all_cached(&a, mode).await.unwrap(),
        entries("secret-a")
    );
    let reopened = Context::fixture(a.pool.clone());
    assert!(read_all(&reopened, mode)
        .await
        .unwrap_err()
        .contains("locked"));
    assert!(unlock(&reopened, "password-b").await.is_err());
    assert!(!is_unlocked(&reopened));
    unlock(&reopened, "password-a").await.unwrap();
    assert_eq!(
        read_all(&reopened, mode).await.unwrap(),
        entries("secret-a")
    );
}

#[tokio::test]
async fn failed_onboarding_does_not_publish_verifier_key_or_completed_flag() {
    let (_directory, context) = fixture().await;
    sqlx::query("CREATE TRIGGER fail_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(ABORT, 'injected settings failure'); END")
        .execute(&context.pool).await.unwrap();
    assert!(configure(
        &context,
        CredentialStorageMode::EncryptedSqlite,
        Some("test-only")
    )
    .await
    .is_err());
    assert!(!is_unlocked(&context));
    assert!(!onboarding_completed(&context.pool).await.unwrap());
    assert_eq!(credential_mode(&context.pool).await.unwrap(), None);
    assert!(storage::read_verifier(&context.pool)
        .await
        .unwrap()
        .is_none());
    sqlx::query("DROP TRIGGER fail_settings")
        .execute(&context.pool)
        .await
        .unwrap();
    configure(
        &context,
        CredentialStorageMode::EncryptedSqlite,
        Some("test-only"),
    )
    .await
    .unwrap();
    assert!(is_unlocked(&context));
    assert!(onboarding_completed(&context.pool).await.unwrap());
}

#[tokio::test]
async fn failed_conversion_and_reset_preserve_durable_credentials_and_the_working_key() {
    let (_directory, context) = fixture().await;
    let encrypted = CredentialStorageMode::EncryptedSqlite;
    let plain = CredentialStorageMode::PlainSqlite;
    configure(&context, encrypted, Some("original-password"))
        .await
        .unwrap();
    write_all(&context, encrypted, &entries("preserved-secret"))
        .await
        .unwrap();
    let rows = storage::read_sqlite_credentials(&context.pool)
        .await
        .unwrap();
    let verifier = storage::read_verifier(&context.pool).await.unwrap();
    let connection =
        crate::app::test_postgres_connection("same-id", crate::SafeMode::default(), false);
    storage::upsert_connection(&context.pool, &connection)
        .await
        .unwrap();
    let drafts = vec![("ui.v1.native.workspace".into(), "SELECT 'draft';".into())];
    storage::upsert_ui_state(&context.pool, &drafts)
        .await
        .unwrap();

    sqlx::query("CREATE TRIGGER fail_settings BEFORE INSERT ON app_settings BEGIN SELECT RAISE(ABORT, 'injected settings failure'); END")
        .execute(&context.pool).await.unwrap();
    for operation in ["convert", "reset"] {
        let result = if operation == "convert" {
            change_mode(&context, encrypted, plain, None).await
        } else {
            reset(&context).await
        };
        assert!(result.is_err());
        assert_eq!(
            storage::read_sqlite_credentials(&context.pool)
                .await
                .unwrap(),
            rows
        );
        assert_eq!(
            storage::read_verifier(&context.pool).await.unwrap(),
            verifier
        );
        assert_eq!(
            credential_mode(&context.pool).await.unwrap(),
            Some(encrypted)
        );
        assert!(onboarding_completed(&context.pool).await.unwrap());
        assert!(is_unlocked(&context));
        assert_eq!(
            read_all_cached(&context, encrypted).await.unwrap(),
            entries("preserved-secret")
        );
    }
    let reopened = Context::fixture(context.pool.clone());
    unlock(&reopened, "original-password").await.unwrap();
    assert_eq!(
        read_all(&reopened, encrypted).await.unwrap(),
        entries("preserved-secret")
    );
    sqlx::query("DROP TRIGGER fail_settings")
        .execute(&context.pool)
        .await
        .unwrap();
    change_mode(&context, encrypted, plain, None).await.unwrap();
    assert!(!is_unlocked(&context));
    assert!(storage::read_verifier(&context.pool)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        read_all(&context, plain).await.unwrap(),
        entries("preserved-secret")
    );
    reset(&context).await.unwrap();
    assert!(!onboarding_completed(&context.pool).await.unwrap());
    assert!(read_all(&context, plain).await.unwrap().is_empty());
    assert!(storage::read_connection_by_id(&context.pool, "same-id")
        .await
        .unwrap()
        .is_some());
    assert_eq!(storage::read_ui_state(&context.pool).await.unwrap(), drafts);
}

#[tokio::test]
async fn failed_rewrite_rolls_back_deleted_rows_and_keeps_the_cache() {
    for mode in [
        CredentialStorageMode::PlainSqlite,
        CredentialStorageMode::EncryptedSqlite,
    ] {
        let (_directory, context) = fixture().await;
        configure(&context, mode, Some("test-only")).await.unwrap();
        write_all(&context, mode, &entries("original"))
            .await
            .unwrap();
        sqlx::query("CREATE TRIGGER fail_credentials BEFORE INSERT ON credentials BEGIN SELECT RAISE(ABORT, 'injected insert failure'); END")
            .execute(&context.pool).await.unwrap();
        assert!(write_all(&context, mode, &entries("replacement"))
            .await
            .is_err());
        assert_eq!(read_all(&context, mode).await.unwrap(), entries("original"));
        assert_eq!(
            read_all_cached(&context, mode).await.unwrap(),
            entries("original")
        );
        sqlx::query("DROP TRIGGER fail_credentials")
            .execute(&context.pool)
            .await
            .unwrap();
        write_all(&context, mode, &entries("replacement"))
            .await
            .unwrap();
        assert_eq!(
            read_all(&context, mode).await.unwrap(),
            entries("replacement")
        );
    }
}

#[tokio::test]
async fn scoped_store_is_carried_through_hydration_mutation_conversion_and_cleanup() {
    let (_directory, fixture) = fixture().await;
    let io = Arc::new(testing::RecordingStore::default());
    let namespace = uuid::Uuid::new_v4().to_string();
    let store = Arc::new(testing::store(&io, &namespace, ReadPolicy::Strict));
    // Exercise the complete legacy lifecycle with an injected strict identity.
    // Fixture lifecycle is separately transactional and cannot select Keychain.
    let context = Context::new(fixture.pool.clone(), store, LifecyclePolicy::Legacy);
    let mode = CredentialStorageMode::Keychain;
    configure(&context, mode, None).await.unwrap();
    let mut connection =
        crate::app::test_postgres_connection("same-id", crate::SafeMode::default(), false);
    connection.set_password("test-secret".into());
    upsert(&context, mode, &connection).await.unwrap();
    connection.set_password(String::new());
    hydrate(&context, mode, &mut connection).await.unwrap();
    assert_eq!(connection.password(), "test-secret");
    let mut secrets = read_all(&context, mode).await.unwrap();
    secrets.insert(
        bastion_secret_id("same-id", "password"),
        "ssh-secret".into(),
    );
    write_all(&context, mode, &secrets).await.unwrap();
    assert_eq!(
        read_bastion_secret(&context, mode, "same-id", "password")
            .await
            .unwrap()
            .as_deref(),
        Some("ssh-secret")
    );
    delete_bastion_secrets(&context, mode, "same-id")
        .await
        .unwrap();
    assert!(read_bastion_secret(&context, mode, "same-id", "password")
        .await
        .unwrap()
        .is_none());
    change_mode(
        &context,
        mode,
        CredentialStorageMode::EncryptedSqlite,
        Some("test-only"),
    )
    .await
    .unwrap();
    change_mode(&context, CredentialStorageMode::EncryptedSqlite, mode, None)
        .await
        .unwrap();
    delete(&context, mode, "same-id").await.unwrap();
    reset(&context).await.unwrap();
    let state = io.state.lock().unwrap();
    assert!(!state.calls.is_empty());
    for (_, service, account) in &state.calls {
        assert_eq!(service, &format!("dbunk-native-stage04-{namespace}"));
        assert_eq!(account, &format!("connection-credentials-{namespace}"));
    }
}

#[tokio::test]
async fn strict_profile_read_failure_is_retryable_without_caching_an_empty_store() {
    let (_directory, fixture) = fixture().await;
    let io = Arc::new(testing::RecordingStore::default());
    let store = Arc::new(testing::store(&io, "retry", ReadPolicy::Strict));
    let context = Context::new(fixture.pool.clone(), store, LifecyclePolicy::Legacy);
    let mode = CredentialStorageMode::Keychain;
    io.state.lock().unwrap().deny_read = true;
    assert!(read_all_cached(&context, mode)
        .await
        .unwrap_err()
        .contains("denied or locked"));
    io.state.lock().unwrap().deny_read = false;
    io.state.lock().unwrap().blobs.insert(
        (
            "dbunk-native-stage04-retry".into(),
            "connection-credentials-retry".into(),
        ),
        "invalid JSON containing a synthetic secret".into(),
    );
    let error = read_all_cached(&context, mode).await.unwrap_err();
    assert!(error.contains("unreadable"));
    assert!(!error.contains("synthetic secret"));
    io.state.lock().unwrap().blobs.insert(
        (
            "dbunk-native-stage04-retry".into(),
            "connection-credentials-retry".into(),
        ),
        serde_json::to_string(&entries("retry-secret")).unwrap(),
    );
    assert_eq!(
        read_all_cached(&context, mode).await.unwrap(),
        entries("retry-secret")
    );
}

#[tokio::test]
async fn same_mode_encrypted_change_rekeys_under_the_new_password() {
    let (_directory, context) = fixture().await;
    let mode = CredentialStorageMode::EncryptedSqlite;
    configure(&context, mode, Some("old-password"))
        .await
        .unwrap();
    write_all(&context, mode, &entries("rekeyed-secret"))
        .await
        .unwrap();
    change_mode(&context, mode, mode, Some("new-password"))
        .await
        .unwrap();
    assert!(is_unlocked(&context));
    assert_eq!(
        read_all_cached(&context, mode).await.unwrap(),
        entries("rekeyed-secret")
    );
    let reopened = Context::fixture(context.pool.clone());
    assert!(unlock(&reopened, "old-password")
        .await
        .unwrap_err()
        .contains("Incorrect"));
    unlock(&reopened, "new-password").await.unwrap();
    assert_eq!(
        read_all(&reopened, mode).await.unwrap(),
        entries("rekeyed-secret")
    );
    // A locked profile cannot be re-keyed: its secrets are unreadable.
    let locked = Context::fixture(context.pool.clone());
    assert!(change_mode(&locked, mode, mode, Some("third-password"))
        .await
        .unwrap_err()
        .contains("locked"));
}

#[tokio::test]
async fn a_load_that_raced_a_mutation_never_refills_the_cache() {
    let (_directory, context) = fixture().await;
    let mode = CredentialStorageMode::PlainSqlite;
    configure(&context, mode, None).await.unwrap();
    write_all(&context, mode, &entries("old")).await.unwrap();
    context.invalidate_cache();

    // A reader sees "not loaded" and loads the old secrets without the lock...
    let generation = context.cached().unwrap_err();
    let stale = read_all(&context, mode).await.unwrap();
    // ...while a mutation commits and publishes the new secrets.
    write_all(&context, mode, &entries("new")).await.unwrap();
    assert!(!context.fill_cache(generation, &stale));
    assert_eq!(read_all_cached(&context, mode).await.unwrap(), entries("new"));

    // Invalidation fences a racing load the same way.
    context.invalidate_cache();
    let generation = context.cached().unwrap_err();
    context.invalidate_cache();
    assert!(!context.fill_cache(generation, &stale));
    assert_eq!(read_all_cached(&context, mode).await.unwrap(), entries("new"));
}

#[tokio::test]
async fn an_empty_store_is_a_loaded_cache_state() {
    let (_directory, context) = fixture().await;
    let mode = CredentialStorageMode::PlainSqlite;
    configure(&context, mode, None).await.unwrap();
    assert!(read_all_cached(&context, mode).await.unwrap().is_empty());
    // Rows written behind the cache are not observed: empty means loaded,
    // not "reload on every read".
    sqlite::replace(&context.pool, &entries("out-of-band"), None)
        .await
        .unwrap();
    assert!(read_all_cached(&context, mode).await.unwrap().is_empty());
    context.invalidate_cache();
    assert_eq!(
        read_all_cached(&context, mode).await.unwrap(),
        entries("out-of-band")
    );
}

#[test]
fn malformed_nonce_is_an_error_instead_of_a_panic() {
    for length in [0, 1, 11, 13, 32] {
        assert_eq!(
            decrypt_bytes(&[0; 32], &B64.encode(vec![0; length]), "").unwrap_err(),
            "Invalid credential nonce"
        );
    }
}
