use super::connections::*;
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Only ephemeral loopback peers and an encrypted temporary profile are used.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unsaved_probe_edits_require_explicit_credentials_before_hydration() {
    const CASE: &str = "DBUNK_NATIVE_PROBE_CREDENTIAL_BOUNDARY_TEST";
    if std::env::var_os(CASE).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backend::development::probe_credentials_tests::unsaved_probe_edits_require_explicit_credentials_before_hydration", "--nocapture"])
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
    let backend =
        Backend::create_native_profile(&directory.path().canonicalize().unwrap().join("profile"))
            .await
            .unwrap();
    backend
        .configure_development_credentials(
            DevelopmentStorageMode::EncryptedSqlite,
            Some("owned-profile-password".into()),
        )
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alternate = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let form = DevelopmentPostgresConnection {
        name: "Owned probe peer".into(),
        host: "127.0.0.1".into(),
        port: listener.local_addr().unwrap().port(),
        database: "owned".into(),
        user: "owned".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: Default::default(),
        driver_options: Default::default(),
    };
    let saved = backend
        .save_development_connection(None, form.clone(), "stored-sentinel".into())
        .await
        .unwrap();
    let before = serde_json::to_value(
        storage::read_connection_by_id(&backend.0.state.pool, &saved.id)
            .await
            .unwrap(),
    )
    .unwrap();
    // A locked store makes any attempt to hydrate observable. Boundary refusal
    // must precede that failure as well as any socket or certificate access.
    credentials::lock_for_tests(&backend.0.state.credentials);
    for field in [
        "host", "port", "user", "mode", "root", "cert", "key", "name",
    ] {
        let mut edited = form.clone();
        match field {
            "host" => edited.host = "127.0.0.2".into(),
            "port" => edited.port = alternate.local_addr().unwrap().port(),
            "user" => edited.user = "changed-user".into(),
            "mode" => edited.tls.mode = DevelopmentTlsMode::Require,
            "root" => edited.tls.root_cert_path = Some("not-an-existing-root".into()),
            "cert" => edited.tls.client_cert_path = Some("not-an-existing-cert".into()),
            "key" => edited.tls.client_key_path = Some("not-an-existing-key".into()),
            "name" => edited.tls.server_name = Some("changed.invalid".into()),
            _ => unreachable!(),
        }
        let error = backend
            .test_development_connection(Some(saved.id.clone()), edited.clone(), String::new())
            .await
            .unwrap_err();
        assert!(
            error.starts_with("Enter the password again"),
            "{field}: {error}"
        );
        assert!(!error.contains("stored-sentinel"));
        let (_control, request) = backend.connection_diagnosis_control().unwrap();
        let error = backend
            .diagnose_native_connection(request, Some(saved.id.clone()), edited, String::new())
            .await
            .unwrap_err();
        assert!(
            error.starts_with("Enter the password again"),
            "staged {field}: {error}"
        );
    }
    let error = backend
        .test_development_connection(Some(saved.id.clone()), form.clone(), String::new())
        .await
        .unwrap_err();
    assert!(error.contains("locked"), "{error}");
    let (control, request) = backend.connection_diagnosis_control().unwrap();
    drop(control);
    let error = backend
        .diagnose_native_connection(request, Some(saved.id.clone()), form.clone(), String::new())
        .await
        .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    for peer in [&listener, &alternate] {
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), peer.accept())
                .await
                .is_err()
        );
    }
    backend
        .unlock_development_credentials("owned-profile-password".into())
        .await
        .unwrap();
    // Cosmetic/database edits retain the original boundary. An explicitly
    // supplied password permits a user change but must not replace storage.
    for explicit in [false, true] {
        let mut edited = form.clone();
        edited.name = "Unsaved name".into();
        edited.database = "other_database".into();
        edited.read_only = true;
        if explicit {
            edited.user = "changed-user".into();
        }
        let input = if explicit { "replacement-sentinel" } else { "" };
        let expected = if explicit {
            "replacement-sentinel"
        } else {
            "stored-sentinel"
        };
        let peer = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let length = socket.read_u32().await.unwrap();
            assert!(length < 4096);
            let mut startup = vec![0; length as usize - 4];
            socket.read_exact(&mut startup).await.unwrap();
            socket.write_all(b"R\0\0\0\x08\0\0\0\x03").await.unwrap();
            assert_eq!(socket.read_u8().await.unwrap(), b'p');
            let length = socket.read_u32().await.unwrap();
            assert!(length < 4096);
            let mut password = vec![0; length as usize - 4];
            socket.read_exact(&mut password).await.unwrap();
            assert_eq!(password, format!("{expected}\0").as_bytes());
            // Close after observing authentication; this is deliberately not
            // a successful PostgreSQL server or a live fixture pass.
        };
        let probe =
            backend.test_development_connection(Some(saved.id.clone()), edited, input.into());
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::join!(probe, peer)
        })
        .await
        .unwrap();
        assert!(matches!(
            result.unwrap(),
            DevelopmentConnectionTest::Failed { .. }
        ));
    }
    assert_eq!(
        serde_json::to_value(
            storage::read_connection_by_id(&backend.0.state.pool, &saved.id)
                .await
                .unwrap()
        )
        .unwrap(),
        before
    );
    assert_eq!(
        credentials::read_all(
            &backend.0.state.credentials,
            CredentialStorageMode::EncryptedSqlite
        )
        .await
        .unwrap()[&saved.id],
        "stored-sentinel"
    );
    backend.shutdown().await.unwrap();
}
