use super::connections::*;
use super::*;

fn fixture() -> DevelopmentFixtures {
    DevelopmentFixtures::from_json(&serde_json::json!({
        "version": 1, "fixture": "dbunk-native-stage03", "instance": "2283820d-33ec-4c4c-ae03-7051092bd410",
        "host": "127.0.0.1", "port": 15432, "database": "dbunk_demo", "user": "dbunk"
    }).to_string()).unwrap()
}

fn form(name: &str) -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: name.into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: Default::default(),
        driver_options: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_workflow_preserves_secrets_metadata_and_atomic_failures() {
    const CASE: &str = "DBUNK_STAGE04_CONNECTION_TEST";
    if std::env::var_os(CASE).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backend::development::connection_tests::connection_workflow_preserves_secrets_metadata_and_atomic_failures", "--nocapture"])
            .env(CASE, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("profile");
    let backend = Backend::create_development(&path, fixture()).await.unwrap();
    assert!(backend
        .save_development_connection(None, form("before onboarding"), "secret".into())
        .await
        .is_err());
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    assert!(backend
        .save_development_connection_with_organization(
            None,
            form("Must not create"),
            "must-not-store".into(),
            DevelopmentConnectionOrganization {
                folder: "x".repeat(257),
                ..Default::default()
            },
        )
        .await
        .is_err());
    assert!(backend.development_connections().await.unwrap().is_empty());
    let first = backend
        .save_development_connection(None, form("Primary"), "secret-sentinel".into())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&first)
        .unwrap()
        .contains("secret-sentinel"));
    assert!(!format!("{first:?}").contains("secret-sentinel"));
    backend
        .save_development_connection(Some(first.id.clone()), form("Renamed"), String::new())
        .await
        .unwrap();
    let state = &backend.0.state;
    assert_eq!(
        credentials::read_all(&state.credentials, CredentialStorageMode::PlainSqlite)
            .await
            .unwrap()[&first.id],
        "secret-sentinel"
    );
    let copy = backend
        .duplicate_development_connection(first.id.clone())
        .await
        .unwrap();
    assert_ne!(copy.id, first.id);
    assert_eq!(copy.name, "Renamed copy");
    assert_eq!(
        credentials::read_all(&state.credentials, CredentialStorageMode::PlainSqlite)
            .await
            .unwrap()[&copy.id],
        "secret-sentinel"
    );
    backend
        .organize_development_connection(
            first.id.clone(),
            DevelopmentConnectionOrganization {
                folder: " Local ".into(),
                is_favorite: true,
                color: "green".into(),
            },
        )
        .await
        .unwrap();
    let duplicated = backend
        .duplicate_development_connection(first.id.clone())
        .await
        .unwrap();
    assert_eq!(duplicated.organization.folder, "Local");
    assert!(!duplicated.organization.is_favorite);

    // A native save and organization update share the same mutation lock.
    // Hold it explicitly to prove organization cannot slip past a credential
    // snapshot and subsequently be overwritten by that save.
    let guard = credentials::mutation_guard(&state.credentials).await;
    let pending_backend = backend.clone();
    let pending_id = first.id.clone();
    let mut organizing = tokio::spawn(async move {
        pending_backend
            .organize_development_connection(
                pending_id,
                DevelopmentConnectionOrganization {
                    folder: "Serialized".into(),
                    is_favorite: true,
                    color: "green".into(),
                },
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut organizing)
            .await
            .is_err()
    );
    drop(guard);
    organizing.await.unwrap().unwrap();
    let saved = backend
        .save_development_connection(Some(first.id.clone()), form("Renamed"), String::new())
        .await
        .unwrap();
    assert_eq!(saved.organization.folder, "Serialized");
    assert!(backend
        .save_development_connection_with_organization(
            Some(first.id.clone()),
            form("Must not rename"),
            "must-not-store".into(),
            DevelopmentConnectionOrganization {
                color: "x".repeat(65),
                ..Default::default()
            },
        )
        .await
        .is_err());
    let unchanged = storage::read_connection_by_id(&state.pool, &first.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.name(), "Renamed");
    assert_eq!(unchanged.folder(), "Serialized");
    let saved = backend
        .save_development_connection_with_organization(
            Some(first.id.clone()),
            form("Renamed"),
            String::new(),
            DevelopmentConnectionOrganization {
                folder: "Together".into(),
                is_favorite: true,
                color: "blue".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.organization.folder, "Together");
    assert!(saved.organization.is_favorite);
    assert_eq!(saved.organization.color, "blue");

    for mode in [
        DevelopmentStorageMode::PlainSqlite,
        DevelopmentStorageMode::EncryptedSqlite,
    ] {
        if mode == DevelopmentStorageMode::EncryptedSqlite {
            backend
                .change_development_credentials(mode, Some("test-password".into()), true)
                .await
                .unwrap();
        }
        sqlx::query("CREATE TRIGGER reject_native_credentials BEFORE INSERT ON credentials BEGIN SELECT RAISE(ABORT, 'injected credential write failure'); END").execute(&state.pool).await.unwrap();
        let failed = backend
            .save_development_connection_with_organization(
                Some(first.id.clone()),
                form("Must not commit"),
                "replacement-secret".into(),
                DevelopmentConnectionOrganization {
                    folder: "Must roll back".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(!failed.contains("replacement-secret"));
        assert_eq!(
            storage::read_connection_by_id(&state.pool, &first.id)
                .await
                .unwrap()
                .unwrap()
                .name(),
            "Renamed"
        );
        assert_eq!(
            storage::read_connection_by_id(&state.pool, &first.id)
                .await
                .unwrap()
                .unwrap()
                .folder(),
            "Together"
        );
        let core_mode = match mode {
            DevelopmentStorageMode::PlainSqlite => CredentialStorageMode::PlainSqlite,
            _ => CredentialStorageMode::EncryptedSqlite,
        };
        assert_eq!(
            credentials::read_all(&state.credentials, core_mode)
                .await
                .unwrap()[&first.id],
            "secret-sentinel"
        );
        sqlx::query("DROP TRIGGER reject_native_credentials")
            .execute(&state.pool)
            .await
            .unwrap();
    }
    let mut unsupported =
        crate::app::test_postgres_connection("foreign-metadata", crate::SafeMode::Strict, true);
    let StoredConnection::PostgreSQL(pg) = &mut unsupported else {
        unreachable!()
    };
    pg.host = "foreign.invalid".into();
    pg.ssh_tunnel.proxy_command = Some("preserve-me".into());
    storage::upsert_connection(&state.pool, &unsupported)
        .await
        .unwrap();
    let before = serde_json::to_string(
        &storage::read_connection_by_id(&state.pool, "foreign-metadata")
            .await
            .unwrap(),
    )
    .unwrap();
    let listed = backend.development_connections().await.unwrap();
    let foreign = listed
        .iter()
        .find(|row| row.id == "foreign-metadata")
        .unwrap();
    assert!(foreign.postgres.is_none());
    assert!(foreign.unsupported_reason.is_some());
    assert!(backend
        .save_development_connection(Some(foreign.id.clone()), form("rewrite"), String::new())
        .await
        .is_err());
    assert!(backend
        .duplicate_development_connection(foreign.id.clone())
        .await
        .is_err());
    assert_eq!(
        serde_json::to_string(
            &storage::read_connection_by_id(&state.pool, "foreign-metadata")
                .await
                .unwrap()
        )
        .unwrap(),
        before
    );
    let mut outside = form("foreign");
    outside.port = 15433;
    assert!(backend
        .save_development_connection(None, outside, String::new())
        .await
        .is_err());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut refused_probe = form("Must not dial");
    refused_probe.port = listener.local_addr().unwrap().port();
    assert!(backend
        .test_development_connection(None, refused_probe, "private".into())
        .await
        .is_err());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let mut tls = form("TLS");
    tls.tls.mode = DevelopmentTlsMode::Require;
    assert!(backend
        .save_development_connection(None, tls, String::new())
        .await
        .is_err());
    for column in ["tls_options", "driver_options"] {
        for raw in ["not-json", r#"{"futureOption":"preserve-raw"}"#] {
            sqlx::query(&format!("UPDATE connections SET {column} = ? WHERE id = ?"))
                .bind(raw)
                .bind(&copy.id)
                .execute(&state.pool)
                .await
                .unwrap();
            let listed = backend.development_connections().await.unwrap();
            let row = listed.iter().find(|row| row.id == copy.id).unwrap();
            assert!(row.postgres.is_none());
            assert!(row
                .unsupported_reason
                .as_deref()
                .unwrap()
                .contains("options"));
            assert!(backend
                .save_development_connection(
                    Some(copy.id.clone()),
                    form("rewrite invalid"),
                    String::new()
                )
                .await
                .is_err());
            assert!(backend
                .duplicate_development_connection(copy.id.clone())
                .await
                .is_err());
            assert!(backend
                .test_development_connection(
                    Some(copy.id.clone()),
                    form("probe invalid"),
                    String::new()
                )
                .await
                .is_err());
            backend
                .organize_development_connection(
                    copy.id.clone(),
                    DevelopmentConnectionOrganization::default(),
                )
                .await
                .unwrap();
            let preserved: String =
                sqlx::query_scalar(&format!("SELECT {column} FROM connections WHERE id = ?"))
                    .bind(&copy.id)
                    .fetch_one(&state.pool)
                    .await
                    .unwrap();
            assert_eq!(preserved, raw);
            sqlx::query(&format!(
                "UPDATE connections SET {column} = NULL WHERE id = ?"
            ))
            .bind(&copy.id)
            .execute(&state.pool)
            .await
            .unwrap();
        }
    }
    backend
        .delete_development_connection(first.id.clone())
        .await
        .unwrap();
    assert!(backend
        .save_development_connection(Some(first.id.clone()), form("resurrect"), String::new())
        .await
        .is_err());
    assert!(
        !credentials::read_all(&state.credentials, CredentialStorageMode::EncryptedSqlite)
            .await
            .unwrap()
            .contains_key(&first.id)
    );
    backend.shutdown().await.unwrap();
}

/// Synthetic PostgreSQL peers bind only owned ephemeral listeners. Private
/// manifest injection exercises the real public probe without relaxing launch
/// admission or contacting an existing fixture listener.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn probe_disconnect_and_post_connect_failure_join_owned_sockets() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    const CASE: &str = "DBUNK_STAGE04_PROBE_TEST";
    if std::env::var_os(CASE).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backend::development::connection_tests::probe_disconnect_and_post_connect_failure_join_owned_sockets", "--nocapture"])
            .env(CASE, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("profile");
    let mut backend = Backend::create_development(&path, fixture()).await.unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let authority = Arc::get_mut(
        Arc::get_mut(&mut backend.0)
            .unwrap()
            .development
            .as_mut()
            .unwrap(),
    )
    .unwrap();
    let EndpointCapability::OwnedFixtures(fixtures) = &mut authority.capability else {
        panic!("fixture authority expected")
    };
    fixtures.port = listener.local_addr().unwrap().port();
    let mut probe_form = form("Owned protocol peer");
    probe_form.port = listener.local_addr().unwrap().port();
    let row = backend
        .save_development_connection(None, probe_form.clone(), "private".into())
        .await
        .unwrap();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let length = socket.read_u32().await.unwrap();
        let mut startup = vec![0; length as usize - 4];
        socket.read_exact(&mut startup).await.unwrap();
        started.send(()).unwrap();
        released.await.unwrap();
        send_pg_error(&mut socket).await;
        let mut tail = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            socket.read_to_end(&mut tail),
        )
        .await
        .unwrap()
        .unwrap();
        listener
    });
    let caller = backend.clone();
    let id = row.id.clone();
    let input = probe_form.clone();
    let probe = tokio::spawn(async move {
        caller
            .test_development_connection(Some(id), input, String::new())
            .await
    });
    ready.await.unwrap();
    let disconnecting = backend.clone();
    let id = row.id.clone();
    let mut disconnect =
        tokio::spawn(async move { disconnecting.disconnect_development_connection(id).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut disconnect)
            .await
            .is_err(),
        "disconnect must wait for the admitted probe"
    );
    release.send(()).unwrap();
    assert!(matches!(
        probe.await.unwrap().unwrap(),
        DevelopmentConnectionTest::Failed { .. }
    ));
    disconnect.await.unwrap().unwrap();
    let listener = server.await.unwrap();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let length = socket.read_u32().await.unwrap();
        let mut startup = vec![0; length as usize - 4];
        socket.read_exact(&mut startup).await.unwrap();
        socket
            .write_all(b"R\0\0\0\x08\0\0\0\0K\0\0\0\x0c\0\0\0\x01\0\0\0\x02Z\0\0\0\x05I")
            .await
            .unwrap();
        assert_eq!(socket.read_u8().await.unwrap(), b'Q');
        let length = socket.read_u32().await.unwrap();
        let mut options = vec![0; length as usize - 4];
        socket.read_exact(&mut options).await.unwrap();
        send_pg_error(&mut socket).await;
        socket.write_all(b"Z\0\0\0\x05I").await.unwrap();
        let mut tail = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            socket.read_to_end(&mut tail),
        )
        .await
        .unwrap()
        .unwrap();
        listener
    });
    probe_form.driver_options.default_role = Some("missing-role".into());
    let result = backend
        .test_development_connection(Some(row.id.clone()), probe_form.clone(), String::new())
        .await
        .unwrap();
    assert!(matches!(result, DevelopmentConnectionTest::Failed { .. }));
    let listener = server.await.unwrap();

    // Forced native shutdown cancels a stalled post-connect options query and
    // retains the actual driver join even after the probe future is aborted.
    let caller = backend.clone();
    let probe = tokio::spawn(async move {
        caller
            .test_development_connection(Some(row.id), probe_form, String::new())
            .await
    });
    let (mut socket, _) = listener.accept().await.unwrap();
    let length = socket.read_u32().await.unwrap();
    let mut startup = vec![0; length as usize - 4];
    socket.read_exact(&mut startup).await.unwrap();
    socket
        .write_all(b"R\0\0\0\x08\0\0\0\0K\0\0\0\x0c\0\0\0\x01\0\0\0\x02Z\0\0\0\x05I")
        .await
        .unwrap();
    assert_eq!(socket.read_u8().await.unwrap(), b'Q');
    let length = socket.read_u32().await.unwrap();
    let mut options = vec![0; length as usize - 4];
    socket.read_exact(&mut options).await.unwrap();
    let now = tokio::time::Instant::now();
    backend
        .shutdown_with_deadlines(
            now + std::time::Duration::from_millis(30),
            now + std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert!(probe.await.unwrap().is_err());
    let mut tail = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        socket.read_to_end(&mut tail),
    )
    .await
    .unwrap()
    .unwrap();
}

async fn send_pg_error(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncWriteExt;
    let body = b"SERROR\0C42601\0Mowned synthetic failure\0\0";
    socket.write_u8(b'E').await.unwrap();
    socket.write_u32(body.len() as u32 + 4).await.unwrap();
    socket.write_all(body).await.unwrap();
}
