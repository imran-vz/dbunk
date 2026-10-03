use super::*;
use crate::backend::{DevelopmentCredentialState as State, DevelopmentStorageMode as Mode};

fn fixtures() -> DevelopmentFixtures {
    DevelopmentFixtures::from_json(&serde_json::json!({
        "version": 1, "fixture": "dbunk-native-stage03", "instance": "2283820d-33ec-4c4c-ae03-7051092bd410",
        "host": "127.0.0.1", "port": 15432, "database": "dbunk_demo", "user": "dbunk"
    }).to_string()).unwrap()
}

/// All valid stage04 profile variants run in separate processes. This also
/// isolates keyring's builder and proves the real process profile-switch guard.
fn child(case: &str) -> bool {
    const ENV: &str = "DBUNK_STAGE04_PROFILE_TEST";
    if std::env::var(ENV).as_deref() == Ok(case) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("backend::development::tests::{case}"),
            "--nocapture",
        ])
        .env(ENV, case)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

fn record_keychain() -> Arc<Mutex<Vec<(String, String)>>> {
    #[derive(Debug)]
    struct Recording(Arc<Mutex<Vec<(String, String)>>>);
    impl keyring::credential::CredentialBuilderApi for Recording {
        fn build(
            &self,
            _: Option<&str>,
            service: &str,
            account: &str,
        ) -> keyring::Result<Box<keyring::credential::Credential>> {
            self.0
                .lock()
                .unwrap()
                .push((service.into(), account.into()));
            Err(keyring::Error::NoStorageAccess(Box::new(
                std::io::Error::other("injected denial"),
            )))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn persistence(&self) -> keyring::credential::CredentialPersistence {
            keyring::credential::CredentialPersistence::EntryOnly
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    keyring::set_default_credential_builder(Box::new(Recording(calls.clone())));
    calls
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_lifecycle_reopen_and_reset_never_construct_keychain_entries() {
    if !child("sqlite_lifecycle_reopen_and_reset_never_construct_keychain_entries") {
        return;
    }
    let calls = record_keychain();
    if let Some(path) = std::env::var_os("DBUNK_STAGE04_REOPEN_PATH") {
        let opened = Backend::open_development(Path::new(&path), &fixtures()).await;
        if std::env::var_os("DBUNK_STAGE04_EXPECT_LOCKED").is_some() {
            assert!(opened.err().unwrap().contains("already in use"));
        } else {
            let backend = opened.unwrap();
            assert_eq!(
                backend.development_settings().await.unwrap().state,
                State::NeedsUnlock
            );
            backend
                .unlock_development_credentials("profile-password".into())
                .await
                .unwrap();
            let secrets = credentials::read_all(
                &backend.0.state.credentials,
                CredentialStorageMode::EncryptedSqlite,
            )
            .await
            .unwrap();
            assert_eq!(secrets["stored"], "dbunk");
            backend.shutdown().await.unwrap();
        }
        assert!(calls.lock().unwrap().is_empty());
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("profile");
    let fixtures = fixtures();
    let backend = Backend::create_development(&path, fixtures.clone())
        .await
        .unwrap();
    let profile_id = backend.development_settings().await.unwrap().profile_id;
    assert_eq!(
        backend.development_settings().await.unwrap().state,
        State::NeedsOnboarding
    );
    let native = backend.0.state.clone();
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&native.pool)
        .await
        .unwrap();
    assert_eq!(synchronous, 2, "native commits use SQLite FULL durability");
    assert!(Backend::open_development(&path, &fixtures).await.is_err());
    assert!(backend
        .configure_development_credentials(Mode::EncryptedSqlite, None)
        .await
        .is_err());
    assert_eq!(
        backend.development_settings().await.unwrap().state,
        State::NeedsOnboarding
    );
    backend
        .configure_development_credentials(Mode::EncryptedSqlite, Some("profile-password".into()))
        .await
        .unwrap();
    let connection =
        crate::app::test_postgres_connection("stored", crate::SafeMode::Protected, false);
    crate::connections::save(&native, connection).await.unwrap();
    storage::upsert_ui_state(
        &native.pool,
        &[("ui.v1.native.workspace".into(), "draft".into())],
    )
    .await
    .unwrap();
    reopen_child(&path, true);
    backend.shutdown().await.unwrap();
    drop(backend);
    drop(native);
    reopen_child(&path, false);
    let backend = Backend::open_development(&path, &fixtures).await.unwrap();
    assert_eq!(
        backend.development_settings().await.unwrap().profile_id,
        profile_id
    );
    assert_eq!(
        backend.development_settings().await.unwrap().state,
        State::NeedsUnlock
    );
    assert!(backend
        .unlock_development_credentials("wrong".into())
        .await
        .is_err());
    assert_eq!(
        backend.development_settings().await.unwrap().state,
        State::NeedsUnlock
    );
    backend
        .unlock_development_credentials("profile-password".into())
        .await
        .unwrap();
    assert!(backend
        .change_development_credentials(Mode::PlainSqlite, None, false)
        .await
        .is_err());
    backend
        .change_development_credentials(Mode::PlainSqlite, None, true)
        .await
        .unwrap();
    assert!(backend.reset_development_credentials(false).await.is_err());
    assert_eq!(
        backend
            .reset_development_credentials(true)
            .await
            .unwrap()
            .state,
        State::NeedsOnboarding
    );
    assert_eq!(
        storage::read_connections(&backend.0.state.pool)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        storage::read_ui_state(&backend.0.state.pool).await.unwrap()[0].1,
        "draft"
    );
    assert!(calls.lock().unwrap().is_empty());
    let other = directory.path().canonicalize().unwrap().join("other");
    assert!(Backend::create_development(&other, fixtures.clone())
        .await
        .is_err());
    assert!(!other.exists());
    backend.shutdown().await.unwrap();
}

fn reopen_child(path: &Path, expect_locked: bool) {
    let case = "sqlite_lifecycle_reopen_and_reset_never_construct_keychain_entries";
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("backend::development::tests::{case}"),
            "--nocapture",
        ])
        .env("DBUNK_STAGE04_PROFILE_TEST", case)
        .env("DBUNK_STAGE04_REOPEN_PATH", path);
    if expect_locked {
        command.env("DBUNK_STAGE04_EXPECT_LOCKED", "1");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_paths_markers_databases_and_fixture_changes_are_preserved() {
    if !child("foreign_paths_markers_databases_and_fixture_changes_are_preserved") {
        return;
    }
    let calls = record_keychain();
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let path = root.join("profile");
    let fixture = fixtures();
    let backend = Backend::create_development(&path, fixture.clone())
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
    drop(backend);
    let marker = std::fs::read(path.join(MARKER)).unwrap();
    let database = std::fs::read(path.join("dbunk.sqlite")).unwrap();
    assert!(Backend::create_development(&path, fixture.clone())
        .await
        .is_err());
    let mut changed = fixture.clone();
    changed.instance = uuid::Uuid::new_v4().to_string();
    assert!(Backend::open_development(&path, &changed).await.is_err());
    let mut json: serde_json::Value = serde_json::from_slice(&marker).unwrap();
    json["credential_namespace"] = uuid::Uuid::new_v4().to_string().into();
    std::fs::write(path.join(MARKER), serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(Backend::open_development(&path, &fixture)
        .await
        .err()
        .unwrap()
        .contains("does not match"));
    assert_eq!(std::fs::read(path.join("dbunk.sqlite")).unwrap(), database);
    std::fs::write(path.join(MARKER), &marker).unwrap();
    let copy = root.join("copy");
    files::create_directory(&copy).unwrap();
    files::write_new(&copy.join(MARKER), &marker).unwrap();
    files::write_new(&copy.join("dbunk.sqlite"), &database).unwrap();
    assert!(Backend::open_development(&copy, &fixture).await.is_err());
    let foreign = path.join("foreign");
    std::fs::write(&foreign, "keep").unwrap();
    assert!(Backend::open_development(&path, &fixture).await.is_err());
    assert_eq!(std::fs::read(&foreign).unwrap(), b"keep");
    std::fs::remove_file(&foreign).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let link = root.join("linked");
        symlink(&path, &link).unwrap();
        assert!(Backend::open_development(&link, &fixture).await.is_err());
        std::fs::rename(path.join("dbunk.sqlite"), root.join("held.sqlite")).unwrap();
        symlink(root.join("held.sqlite"), path.join("dbunk.sqlite")).unwrap();
        assert!(Backend::open_development(&path, &fixture).await.is_err());
        std::fs::remove_file(path.join("dbunk.sqlite")).unwrap();
        std::fs::hard_link(root.join("held.sqlite"), path.join("dbunk.sqlite")).unwrap();
        assert!(Backend::open_development(&path, &fixture).await.is_err());
        std::fs::remove_file(path.join("dbunk.sqlite")).unwrap();
        std::fs::rename(root.join("held.sqlite"), path.join("dbunk.sqlite")).unwrap();
        std::fs::set_permissions(path.join(MARKER), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(Backend::open_development(&path, &fixture).await.is_err());
        std::fs::set_permissions(path.join(MARKER), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    assert_eq!(std::fs::read(path.join("dbunk.sqlite")).unwrap(), database);
    assert!(calls.lock().unwrap().is_empty());
    let backend = Backend::open_development(&path, &fixture).await.unwrap();
    backend.shutdown().await.unwrap();
}

#[test]
fn fixture_admission_rejects_endpoint_transport_and_engine_substitution() {
    let fixture = fixtures();
    let connection =
        crate::app::test_postgres_connection("owned", crate::SafeMode::Protected, false);
    assert!(fixture.permits(&connection));
    let StoredConnection::PostgreSQL(pg) = connection else {
        unreachable!()
    };
    for field in [
        "host",
        "port",
        "database",
        "user",
        "ssh",
        "tls",
        "server-name",
    ] {
        let mut connection = pg.clone();
        match field {
            "host" => connection.host = "localhost".into(),
            "port" => connection.port = 5432,
            "database" => connection.database = "foreign".into(),
            "user" => connection.user = "foreign".into(),
            "ssh" => connection.ssh_tunnel.enabled = true,
            "tls" => connection.ssl = true,
            "server-name" => {
                connection.tls_options = Some(crate::PgTlsOptions {
                    mode: crate::PgTlsMode::Disable,
                    server_name: Some("foreign.example".into()),
                    ..Default::default()
                })
            }
            _ => unreachable!(),
        }
        assert!(
            !fixture.permits(&StoredConnection::PostgreSQL(connection)),
            "{field}"
        );
    }
    let mut json = serde_json::to_value(&fixture).unwrap();
    json["port"] = 5432.into();
    assert!(DevelopmentFixtures::from_json(&json.to_string()).is_err());
    json["port"] = 15432.into();
    json["unknown"] = true.into();
    assert!(DevelopmentFixtures::from_json(&json.to_string()).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scoped_namespace_is_lazy_and_cannot_fall_back_to_production() {
    if !child("scoped_namespace_is_lazy_and_cannot_fall_back_to_production") {
        return;
    }
    let calls = record_keychain();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("profile");
    let backend = Backend::create_development(&path, fixtures())
        .await
        .unwrap();
    assert!(calls.lock().unwrap().is_empty());
    let marker: Marker =
        serde_json::from_slice(&std::fs::read(path.join(MARKER)).unwrap()).unwrap();
    // Direct private test of the injected OS adapter, never a native public API.
    for _ in 0..2 {
        assert!(credentials::read_all(
            &backend.0.state.credentials,
            CredentialStorageMode::Keychain
        )
        .await
        .unwrap_err()
        .contains("denied or locked"));
    }
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2, "denial is not cached as an empty store");
        for (service, account) in calls.iter() {
            assert_eq!(
                service,
                &format!("dbunk-native-stage04-{}", marker.credential_namespace)
            );
            assert_eq!(
                account,
                &format!("connection-credentials-{}", marker.credential_namespace)
            );
        }
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupt_settings_and_denied_keychain_never_look_empty() {
    if !child("corrupt_settings_and_denied_keychain_never_look_empty") {
        return;
    }
    let calls = record_keychain();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("profile");
    let backend = Backend::create_development(&path, fixtures())
        .await
        .unwrap();
    let state = &backend.0.state;
    storage::set_setting(&state.pool, "onboardingCompleted", "invalid")
        .await
        .unwrap();
    assert!(backend.development_settings().await.is_err());
    storage::set_setting(&state.pool, "onboardingCompleted", "true")
        .await
        .unwrap();
    assert!(backend.development_settings().await.is_err());
    storage::set_setting(&state.pool, "credentialStorageMode", "keychain")
        .await
        .unwrap();
    assert!(backend
        .development_settings()
        .await
        .unwrap_err()
        .contains("denied or locked"));
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "configured Keychain uses the scoped strict store"
    );
    storage::set_setting(&state.pool, "onboardingCompleted", "false")
        .await
        .unwrap();
    storage::set_setting(&state.pool, "credentialStorageMode", "plain-sqlite")
        .await
        .unwrap();
    storage::upsert_sqlite_credential(
        &state.pool,
        "preserved",
        CredentialStorageMode::PlainSqlite,
        None,
        "test-only",
    )
    .await
    .unwrap();
    assert!(backend.development_settings().await.is_err());
    assert!(backend
        .configure_development_credentials(Mode::PlainSqlite, None)
        .await
        .is_err());
    assert_eq!(
        storage::read_sqlite_credentials(&state.pool)
            .await
            .unwrap()
            .len(),
        1
    );
    backend.reset_development_credentials(true).await.unwrap();
    assert_eq!(
        backend.development_settings().await.unwrap().state,
        State::NeedsOnboarding
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_admission_rejects_foreign_endpoint_before_credentials_or_network() {
    use crate::backend::{
        OpenSessionPayload, QueryEventEnvelope, QuerySessionError, RegisterOwnerPayload,
    };
    if !child("session_admission_rejects_foreign_endpoint_before_credentials_or_network") {
        return;
    }
    let calls = record_keychain();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("profile");
    let backend = Backend::create_development(&path, fixtures())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut connection =
        crate::app::test_postgres_connection("foreign", crate::SafeMode::Protected, false);
    let StoredConnection::PostgreSQL(pg) = &mut connection else {
        unreachable!()
    };
    pg.port = listener.local_addr().unwrap().port();
    pg.ssl = false;
    storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
    // If admission accidentally reaches hydration, the injected builder will
    // record it. No real Keychain or unidentified network listener is involved.
    credentials::set_credential_mode(&backend.0.state.pool, CredentialStorageMode::Keychain)
        .await
        .unwrap();
    backend
        .register_owner(
            "native",
            RegisterOwnerPayload {
                owner_id: "owner".into(),
            },
        )
        .await
        .unwrap();
    let error = backend
        .open(
            "native",
            OpenSessionPayload {
                owner_id: "owner".into(),
                session_id: "session".into(),
                tab_id: "tab".into(),
                connection_id: "foreign".into(),
            },
            Arc::new(|_: QueryEventEnvelope| panic!("refused endpoint emitted an event")),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, QuerySessionError::ConnectionLost));
    assert!(calls.lock().unwrap().is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
    backend.shutdown().await.unwrap();
}

#[test]
fn tls_fixture_opt_in_pins_endpoint_without_bypassing_certificate_validation() {
    let mut manifest = serde_json::to_value(fixtures()).unwrap();
    assert!(manifest.get("tls").is_none());
    manifest["tls"] = serde_json::json!({
        "fixture":"dbunk-native-stage04-tls", "instance":"2c1b9ea7-31db-4413-9cd8-6d12436be6e9",
        "host":"127.0.0.1", "port":15433, "database":"dbunk_tls_demo", "user":"dbunk"
    });
    let opted_in = DevelopmentFixtures::from_json(&manifest.to_string()).unwrap();
    let StoredConnection::PostgreSQL(mut pg) =
        crate::app::test_postgres_connection("tls", crate::SafeMode::Protected, false)
    else {
        unreachable!()
    };
    pg.port = 15433;
    pg.database = "dbunk_tls_demo".into();
    pg.tls_options = Some(crate::PgTlsOptions {
        mode: crate::PgTlsMode::VerifyFull,
        server_name: Some("certificate-mismatch.invalid".into()),
        root_cert_path: Some("/private/tmp/test-untrusted-ca.pem".into()),
        ..Default::default()
    });
    assert!(!fixtures().permits(&StoredConnection::PostgreSQL(pg.clone())));
    assert!(opted_in.permits(&StoredConnection::PostgreSQL(pg.clone())));
    for field in ["host", "port", "database", "user", "ssh", "plaintext"] {
        let mut changed = pg.clone();
        match field {
            "host" => changed.host = "certificate-mismatch.invalid".into(),
            "port" => changed.port = 5432,
            "database" => changed.database = "foreign".into(),
            "user" => changed.user = "foreign".into(),
            "ssh" => changed.ssh_tunnel.enabled = true,
            "plaintext" => changed.tls_options.as_mut().unwrap().mode = crate::PgTlsMode::Disable,
            _ => unreachable!(),
        }
        assert!(
            !opted_in.permits(&StoredConnection::PostgreSQL(changed)),
            "{field}"
        );
    }
    for (field, value) in [
        ("host", serde_json::json!("localhost")),
        ("port", serde_json::json!(5432)),
        ("database", serde_json::json!("foreign")),
        ("instance", serde_json::json!("invalid")),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut changed = manifest.clone();
        changed["tls"][field] = value;
        assert!(
            DevelopmentFixtures::from_json(&changed.to_string()).is_err(),
            "{field}"
        );
    }
}
