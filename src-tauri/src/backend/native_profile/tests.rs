use super::*;
use crate::backend::{
    DevelopmentConnectionFailure, DevelopmentConnectionTest, DevelopmentCredentialState,
    DevelopmentEnvironment, DevelopmentFixtures, DevelopmentPostgresConnection,
    DevelopmentSafeMode, DevelopmentStorageMode, WorkspaceDocument, WorkspaceSelection,
    WorkspaceSnapshot,
};
use std::sync::Mutex;

const CHILD: &str = "DBUNK_GENERAL_PROFILE_TEST";

fn child(case: &str) -> bool {
    if std::env::var(CHILD).as_deref() == Ok(case) {
        return true;
    }
    run_child(case, None);
    false
}

fn run_child(case: &str, reopen: Option<(&Path, bool)>) {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("backend::native_profile::tests::{case}"),
            "--nocapture",
        ])
        .env(CHILD, case);
    if let Some((path, locked)) = reopen {
        command
            .env("DBUNK_GENERAL_REOPEN", path)
            .env("DBUNK_GENERAL_LOCKED", if locked { "1" } else { "0" });
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

fn fixtures() -> DevelopmentFixtures {
    DevelopmentFixtures::from_json(&serde_json::json!({
        "version":1,"fixture":"dbunk-native-stage03","instance":"2283820d-33ec-4c4c-ae03-7051092bd410",
        "host":"127.0.0.1","port":15432,"database":"dbunk_demo","user":"dbunk"
    }).to_string()).unwrap()
}

fn form(port: u16) -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: "User-selected PostgreSQL".into(),
        host: "127.0.0.1".into(),
        port,
        database: "synthetic_general_profile".into(),
        user: "synthetic_user".into(),
        environment: DevelopmentEnvironment::Test,
        safe_mode: DevelopmentSafeMode::Strict,
        read_only: true,
        tls: Default::default(),
        driver_options: Default::default(),
        ssh_tunnel: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn save_restore_is_disconnected_and_only_explicit_test_contacts_owned_peer() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    const CASE: &str = "save_restore_is_disconnected_and_only_explicit_test_contacts_owned_peer";
    if !child(CASE) {
        return;
    }
    let calls = record_keychain();
    if let Some(path) = std::env::var_os("DBUNK_GENERAL_REOPEN") {
        let opened = Backend::open_native_profile(Path::new(&path)).await;
        if std::env::var("DBUNK_GENERAL_LOCKED").as_deref() == Ok("1") {
            assert!(opened.err().unwrap().contains("already in use"));
        } else {
            let backend = opened.unwrap();
            assert_eq!(backend.development_connections().await.unwrap().len(), 1);
            let snapshot = backend
                .load_development_workspace()
                .await
                .unwrap()
                .snapshot
                .unwrap();
            assert_eq!(snapshot.documents[0].sql, "select 'retained 未実行';");
            assert_eq!(
                backend.native_profile_kind(),
                Some(NativeProfileKind::GeneralPostgres)
            );
            backend.shutdown().await.unwrap();
        }
        assert!(calls.lock().unwrap().is_empty());
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("general");
    let backend = Backend::create_native_profile(&path).await.unwrap();
    assert_eq!(
        backend.development_settings().await.unwrap().state,
        DevelopmentCredentialState::NeedsOnboarding
    );
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    // This test owns this loopback protocol peer; no fixture or DNS is involved.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let input = form(listener.local_addr().unwrap().port());
    let saved = backend
        .save_development_connection(None, input.clone(), "synthetic-secret".into())
        .await
        .unwrap();
    assert!(saved.unsupported_reason.is_none());
    let loaded = backend.development_connections().await.unwrap();
    let loaded_form = loaded[0].postgres.as_ref().unwrap();
    assert_eq!(loaded_form.port, input.port);
    assert!(loaded_form.read_only);
    assert_eq!(loaded_form.safe_mode, DevelopmentSafeMode::Strict);
    let document_id = uuid::Uuid::new_v4().to_string();
    let snapshot = WorkspaceSnapshot {
        documents: vec![WorkspaceDocument {
            id: document_id.clone(),
            name: "Retained".into(),
            connection_id: Some(saved.id.clone()),
            sql: "select 'retained 未実行';".into(),
            pinned: false,
            selection: WorkspaceSelection::default(),
            table: None,
            query_changes: None,
            schema_changes: None,
            table_ddl: None,
            schema_alter: None,
            object_ddl: None,
            admin_control: None,
            maintenance: None,
            tool: None,
            saved_query_id: None,
        }],
        active_document_id: Some(document_id),
        ..Default::default()
    };
    backend
        .save_development_workspace(None, snapshot.clone())
        .await
        .unwrap();
    assert_eq!(
        backend.load_development_workspace().await.unwrap().snapshot,
        Some(snapshot)
    );
    run_child(CASE, Some((&path, true)));
    backend.shutdown().await.unwrap();
    drop(backend);
    run_child(CASE, Some((&path, false)));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "save/list/journal restore must not contact the peer"
    );
    let backend = Backend::open_native_profile(&path).await.unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let length = socket.read_u32().await.unwrap();
        assert!((8..4096).contains(&length));
        let mut startup = vec![0; length as usize - 4];
        socket.read_exact(&mut startup).await.unwrap();
        let error = b"SERROR\0C28P01\0Mowned synthetic authentication failure\0\0";
        socket.write_u8(b'E').await.unwrap();
        socket.write_u32(error.len() as u32 + 4).await.unwrap();
        socket.write_all(error).await.unwrap();
        let mut tail = Vec::new();
        socket.read_to_end(&mut tail).await.unwrap();
    });
    let result = backend
        .test_development_connection(Some(saved.id), input, String::new())
        .await
        .unwrap();
    assert!(matches!(
        result,
        DevelopmentConnectionTest::Failed {
            reason: DevelopmentConnectionFailure::Authentication
        }
    ));
    tokio::time::timeout(std::time::Duration::from_secs(2), peer)
        .await
        .unwrap()
        .unwrap();
    assert!(Backend::create_native_profile(&root.join("other"))
        .await
        .is_err());
    assert!(!root.join("other").exists());
    assert!(
        Backend::create_development(&root.join("fixture"), fixtures())
            .await
            .is_err()
    );
    assert!(!root.join("fixture").exists());
    assert!(calls.lock().unwrap().is_empty());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mismatched_markers_and_databases_are_preserved_without_credentials() {
    if !child("mismatched_markers_and_databases_are_preserved_without_credentials") {
        return;
    }
    let calls = record_keychain();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("general");
    let backend = Backend::create_native_profile(&path).await.unwrap();
    backend.shutdown().await.unwrap();
    drop(backend);
    let marker = std::fs::read(path.join(MARKER)).unwrap();
    let database = std::fs::read(path.join("dbunk.sqlite")).unwrap();
    assert!(Backend::create_native_profile(&path).await.is_err());
    assert!(Backend::open_development(&path, &fixtures()).await.is_err());
    assert!(Backend::open_fixture(&path).await.is_err());
    for field in [
        "version",
        "kind",
        "path",
        "profile_id",
        "credential_namespace",
        "same",
        "unknown",
    ] {
        let mut changed: serde_json::Value = serde_json::from_slice(&marker).unwrap();
        match field {
            "version" => changed["version"] = 2.into(),
            "kind" => changed["kind"] = "owned-fixtures".into(),
            "path" => changed["path"] = root.join("elsewhere").to_str().unwrap().into(),
            "same" => changed["credential_namespace"] = changed["profile_id"].clone(),
            "unknown" => changed["unknown"] = true.into(),
            _ => changed[field] = uuid::Uuid::new_v4().to_string().into(),
        }
        let bytes = serde_json::to_vec(&changed).unwrap();
        std::fs::write(path.join(MARKER), &bytes).unwrap();
        assert!(
            Backend::open_native_profile(&path).await.is_err(),
            "{field}"
        );
        assert_eq!(std::fs::read(path.join(MARKER)).unwrap(), bytes);
        assert_eq!(std::fs::read(path.join("dbunk.sqlite")).unwrap(), database);
    }
    for corrupt in [b"{".to_vec(), vec![b' '; MAX_MARKER_BYTES + 1]] {
        std::fs::write(path.join(MARKER), &corrupt).unwrap();
        assert!(Backend::open_native_profile(&path).await.is_err());
        assert_eq!(std::fs::read(path.join(MARKER)).unwrap(), corrupt);
    }
    std::fs::write(path.join(MARKER), &marker).unwrap();
    files::write_new(&path.join(".dbunk-native-stage04"), b"{}").unwrap();
    assert!(Backend::open_native_profile(&path).await.is_err());
    std::fs::remove_file(path.join(".dbunk-native-stage04")).unwrap();
    std::fs::remove_file(path.join(MARKER)).unwrap();
    assert!(Backend::open_native_profile(&path).await.is_err());
    files::write_new(&path.join(MARKER), &marker).unwrap();
    let copy = root.join("copy");
    files::create_directory(&copy).unwrap();
    files::write_new(&copy.join(MARKER), &marker).unwrap();
    files::write_new(&copy.join("dbunk.sqlite"), &database).unwrap();
    assert!(Backend::open_native_profile(&copy).await.is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        symlink(&path, root.join("alias")).unwrap();
        assert!(Backend::open_native_profile(&root.join("alias"))
            .await
            .is_err());
        std::fs::hard_link(path.join(MARKER), root.join("hardlink")).unwrap();
        assert!(Backend::open_native_profile(&path).await.is_err());
        std::fs::remove_file(root.join("hardlink")).unwrap();
        std::fs::set_permissions(path.join(MARKER), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(Backend::open_native_profile(&path).await.is_err());
        std::fs::set_permissions(path.join(MARKER), std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(path.join("dbunk.sqlite")).unwrap(), database);
    let backend = Backend::open_native_profile(&path).await.unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fixture_profile_cannot_be_reopened_or_switched_to_general() {
    if !child("fixture_profile_cannot_be_reopened_or_switched_to_general") {
        return;
    }
    let calls = record_keychain();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("fixture");
    let backend = Backend::create_development(&path, fixtures())
        .await
        .unwrap();
    assert_eq!(
        backend.native_profile_kind(),
        Some(NativeProfileKind::OwnedFixtures)
    );
    backend.shutdown().await.unwrap();
    drop(backend);
    assert!(Backend::open_native_profile(&path).await.is_err());
    assert!(Backend::create_native_profile(&root.join("general"))
        .await
        .is_err());
    assert!(!root.join("general").exists());
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn general_namespace_is_lazy_strict_and_independent() {
    if !child("general_namespace_is_lazy_strict_and_independent") {
        return;
    }
    let calls = record_keychain();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("general");
    let backend = Backend::create_native_profile(&path).await.unwrap();
    let marker: Marker =
        serde_json::from_slice(&std::fs::read(path.join(MARKER)).unwrap()).unwrap();
    assert_ne!(marker.profile_id, marker.credential_namespace);
    assert!(calls.lock().unwrap().is_empty());
    for _ in 0..2 {
        assert!(credentials::read_all(
            &backend.0.state.credentials,
            crate::CredentialStorageMode::Keychain
        )
        .await
        .unwrap_err()
        .contains("denied or locked"));
    }
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
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
async fn loaded_invalid_options_refuse_before_hydration_and_preserve_metadata() {
    if !child("loaded_invalid_options_refuse_before_hydration_and_preserve_metadata") {
        return;
    }
    let calls = record_keychain();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("general");
    let backend = Backend::create_native_profile(&path).await.unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    let input = form(5432);
    let saved = backend
        .save_development_connection(None, input.clone(), "preserved".into())
        .await
        .unwrap();
    let mut invalid = input.clone();
    invalid.tls.server_name = Some("x".repeat(257));
    assert!(backend
        .save_development_connection(Some(saved.id.clone()), invalid, "replacement".into())
        .await
        .is_err());
    let secrets = credentials::read_all(
        &backend.0.state.credentials,
        crate::CredentialStorageMode::PlainSqlite,
    )
    .await
    .unwrap();
    assert_eq!(secrets[&saved.id], "preserved");
    // The injected adapter would record any accidental hydration after these
    // invalid stored records. No connection attempt is part of this test.
    credentials::set_credential_mode(
        &backend.0.state.pool,
        crate::CredentialStorageMode::Keychain,
    )
    .await
    .unwrap();
    for raw in [
        "not-json".to_owned(),
        r#"{"futureOption":"keep"}"#.to_owned(),
        serde_json::json!({"mode":"disable","serverName":"x".repeat(257)}).to_string(),
        serde_json::json!({"mode":"disable","rootCertPath":"a\0b"}).to_string(),
    ] {
        sqlx::query("UPDATE connections SET tls_options = ? WHERE id = ?")
            .bind(&raw)
            .bind(&saved.id)
            .execute(&backend.0.state.pool)
            .await
            .unwrap();
        let listed = backend.development_connections().await.unwrap();
        assert!(listed[0].postgres.is_none());
        assert!(listed[0].unsupported_reason.is_some());
        assert!(crate::backend::admit_connection(
            &backend.0.state,
            backend.0.development.as_deref(),
            &saved.id
        )
        .await
        .is_err());
        assert!(backend
            .test_development_connection(Some(saved.id.clone()), input.clone(), String::new())
            .await
            .is_err());
        let retained: String =
            sqlx::query_scalar("SELECT tls_options FROM connections WHERE id = ?")
                .bind(&saved.id)
                .fetch_one(&backend.0.state.pool)
                .await
                .unwrap();
        assert_eq!(retained, raw);
        assert!(calls.lock().unwrap().is_empty());
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maximum_name_duplicates_remain_supported_and_preserve_original_secret() {
    if !child("maximum_name_duplicates_remain_supported_and_preserve_original_secret") {
        return;
    }
    let calls = record_keychain();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("general");
    let backend = Backend::create_native_profile(&path).await.unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    for name in [
        "x".repeat(256),
        format!("{}a", "界".repeat(85)),
        "é".repeat(126),
    ] {
        let mut input = form(5432);
        input.name = name.clone();
        let original = backend
            .save_development_connection(None, input, "original-secret".into())
            .await
            .unwrap();
        let copy = backend
            .duplicate_development_connection(original.id.clone())
            .await
            .unwrap();
        assert_ne!(copy.id, original.id);
        assert!(copy.name.len() <= 256);
        assert!(copy.name.ends_with(" copy"));
        assert!(name.starts_with(copy.name.strip_suffix(" copy").unwrap()));
        assert!(copy.unsupported_reason.is_none());
        let mut edited = copy.postgres.unwrap();
        edited.name = "Edited duplicate".into();
        let edited = backend
            .save_development_connection(Some(copy.id.clone()), edited, String::new())
            .await
            .unwrap();
        assert!(edited.unsupported_reason.is_none());
        let secrets = credentials::read_all(
            &backend.0.state.credentials,
            crate::CredentialStorageMode::PlainSqlite,
        )
        .await
        .unwrap();
        assert_eq!(secrets[&original.id], "original-secret");
        assert_eq!(secrets[&copy.id], "original-secret");
        backend
            .delete_development_connection(copy.id.clone())
            .await
            .unwrap();
        let rows = backend.development_connections().await.unwrap();
        assert!(!rows.iter().any(|row| row.id == copy.id));
        let retained = rows.iter().find(|row| row.id == original.id).unwrap();
        assert_eq!(retained.name, name);
        assert!(retained.unsupported_reason.is_none());
        let secrets = credentials::read_all(
            &backend.0.state.credentials,
            crate::CredentialStorageMode::PlainSqlite,
        )
        .await
        .unwrap();
        assert_eq!(secrets[&original.id], "original-secret");
        assert!(!secrets.contains_key(&copy.id));
    }
    assert!(calls.lock().unwrap().is_empty());
    backend.shutdown().await.unwrap();
}
