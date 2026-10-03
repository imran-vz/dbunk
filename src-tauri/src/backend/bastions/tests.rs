//! Each profile test runs in its own child process: a native process may own
//! only one profile. No test contacts an SSH server; the only sockets are
//! loopback listeners owned by the test that never speak SSH.
use super::*;
use crate::backend::connection_diagnosis::{NativeFailureKind, NativeStageKind, NativeStageResult};
use crate::backend::{
    DevelopmentConnectionTest, DevelopmentEnvironment, DevelopmentPostgresConnection,
    DevelopmentSafeMode, DevelopmentSshTunnel, DevelopmentStorageMode,
};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

const CHILD: &str = "DBUNK_BASTION_PROFILE_TEST";
const SECRET: &str = "synthetic-bastion-secret-7f3a";

fn child(case: &str) -> bool {
    if std::env::var(CHILD).as_deref() == Ok(case) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("backend::bastions::tests::{case}"),
            "--nocapture",
        ])
        .env(CHILD, case)
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

async fn general(root: &Path) -> Backend {
    let backend = Backend::create_native_profile(&root.join("general"))
        .await
        .unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    backend
}

fn bastion_form(host: &str, port: u16) -> DevelopmentBastionForm {
    DevelopmentBastionForm {
        name: "Edge".into(),
        host: host.into(),
        port,
        user: "jump".into(),
        auth_method: DevelopmentBastionAuth::Password,
        private_key_path: None,
    }
}

fn password(value: &str) -> DevelopmentBastionSecrets {
    DevelopmentBastionSecrets {
        password: DevelopmentSecretInput::Set(value.into()),
        ..Default::default()
    }
}

fn connection(port: u16, bastion: Option<&str>) -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: "Routed".into(),
        host: "127.0.0.1".into(),
        port,
        database: "synthetic".into(),
        user: "synthetic".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: Default::default(),
        driver_options: Default::default(),
        ssh_tunnel: bastion.map(DevelopmentSshTunnel::new),
    }
}

async fn secrets(backend: &Backend) -> HashMap<String, String> {
    let state = &backend.0.state;
    let mode = crate::app::current_credential_mode(state).await.unwrap();
    credentials::read_all(&state.credentials, mode)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bastion_records_commit_with_their_secrets_and_never_expose_them() {
    const CASE: &str = "bastion_records_commit_with_their_secrets_and_never_expose_them";
    if !child(CASE) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let backend = general(&temp.path().canonicalize().unwrap()).await;

    let missing = backend
        .save_development_bastion(
            None,
            bastion_form("bastion.invalid", 22),
            Default::default(),
        )
        .await
        .unwrap_err();
    assert!(missing.contains("password is required"), "{missing}");
    assert!(backend.development_bastions().await.unwrap().is_empty());

    for (form, expected) in [
        (bastion_form("bastion.invalid", 0), "port"),
        (bastion_form("two words", 22), "whitespace"),
        (bastion_form(" ", 22), "host is required"),
        (
            DevelopmentBastionForm {
                auth_method: DevelopmentBastionAuth::PrivateKeyPath,
                ..bastion_form("bastion.invalid", 22)
            },
            "path is required",
        ),
    ] {
        let error = backend
            .save_development_bastion(None, form, password(SECRET))
            .await
            .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }

    let secrets_input = password(SECRET);
    assert!(!format!("{secrets_input:?}").contains(SECRET));
    let saved = backend
        .save_development_bastion(None, bastion_form("bastion.invalid", 22), secrets_input)
        .await
        .unwrap();
    assert!(saved.has_password && !saved.has_private_key_content);
    assert!(!format!("{saved:?}").contains(SECRET));
    assert!(!serde_json::to_string(&saved).unwrap().contains(SECRET));
    assert_eq!(
        secrets(&backend).await[&credentials::bastion_secret_id(&saved.id, "password")],
        SECRET
    );

    // Switching method prunes the inactive slot in the same commit.
    let key_form = DevelopmentBastionForm {
        auth_method: DevelopmentBastionAuth::PrivateKeyContent,
        ..bastion_form("bastion.invalid", 22)
    };
    let switched = backend
        .save_development_bastion(
            Some(saved.id.clone()),
            key_form.clone(),
            DevelopmentBastionSecrets {
                private_key_content: DevelopmentSecretInput::Set("synthetic key".into()),
                passphrase: DevelopmentSecretInput::Set("synthetic passphrase".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!switched.has_password);
    assert!(switched.has_private_key_content && switched.has_passphrase);
    assert!(!secrets(&backend)
        .await
        .contains_key(&credentials::bastion_secret_id(&saved.id, "password")));

    // A failed metadata write leaves both metadata and secrets unchanged.
    sqlx::query("CREATE TRIGGER reject_bastion BEFORE UPDATE ON bastion_servers BEGIN SELECT RAISE(ABORT, 'injected'); END")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    let before = secrets(&backend).await;
    assert!(backend
        .save_development_bastion(
            Some(saved.id.clone()),
            DevelopmentBastionForm {
                name: "Renamed".into(),
                ..key_form.clone()
            },
            DevelopmentBastionSecrets {
                passphrase: DevelopmentSecretInput::Clear,
                ..Default::default()
            },
        )
        .await
        .is_err());
    assert_eq!(secrets(&backend).await, before);
    let unchanged = &backend.development_bastions().await.unwrap()[0];
    assert_eq!(unchanged.form.name, "Edge");
    assert!(unchanged.has_passphrase);
    sqlx::query("DROP TRIGGER reject_bastion")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();

    // A blank replacement clears a slot rather than storing whitespace.
    let cleared = backend
        .save_development_bastion(
            Some(saved.id.clone()),
            key_form,
            DevelopmentBastionSecrets {
                passphrase: DevelopmentSecretInput::Set("  ".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!cleared.has_passphrase && cleared.has_private_key_content);

    assert!(backend
        .save_development_bastion(
            Some("no-such-bastion".into()),
            bastion_form("bastion.invalid", 22),
            password(SECRET),
        )
        .await
        .unwrap_err()
        .contains("no longer exists"));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_referenced_bastion_requires_exact_review() {
    const CASE: &str = "deleting_a_referenced_bastion_requires_exact_review";
    if !child(CASE) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let backend = general(&temp.path().canonicalize().unwrap()).await;
    let bastion = backend
        .save_development_bastion(None, bastion_form("bastion.invalid", 22), password(SECRET))
        .await
        .unwrap();
    assert!(backend
        .save_development_connection(None, connection(5432, Some("missing")), "pw".into())
        .await
        .unwrap_err()
        .contains("no longer exists"));
    let routed = backend
        .save_development_connection(None, connection(5432, Some(&bastion.id)), "pw".into())
        .await
        .unwrap();
    assert_eq!(
        routed.postgres.as_ref().unwrap().ssh_tunnel,
        Some(DevelopmentSshTunnel::new(bastion.id.clone()))
    );
    let expected = vec![DevelopmentBastionReference {
        connection_id: routed.id.clone(),
        connection_name: "Routed".into(),
    }];
    assert_eq!(
        backend.development_bastions().await.unwrap()[0].references,
        expected
    );

    for reviewed in [vec![], vec!["other".to_string()]] {
        assert_eq!(
            backend
                .delete_development_bastion(bastion.id.clone(), reviewed)
                .await
                .unwrap(),
            DevelopmentBastionDelete::ReviewRequired {
                references: expected.clone()
            }
        );
    }
    assert_eq!(backend.development_bastions().await.unwrap().len(), 1);
    assert!(secrets(&backend)
        .await
        .contains_key(&credentials::bastion_secret_id(&bastion.id, "password")));

    assert_eq!(
        backend
            .delete_development_bastion(bastion.id.clone(), vec![routed.id.clone()])
            .await
            .unwrap(),
        DevelopmentBastionDelete::Deleted
    );
    assert!(backend.development_bastions().await.unwrap().is_empty());
    let remaining = secrets(&backend).await;
    assert!(!remaining.keys().any(|key| key.contains(&bastion.id)));
    assert_eq!(remaining.get(&routed.id).map(String::as_str), Some("pw"));
    // The reviewed consequence: the route is kept and fails closed.
    let connections = backend.development_connections().await.unwrap();
    assert_eq!(
        connections[0].postgres.as_ref().unwrap().ssh_tunnel,
        Some(DevelopmentSshTunnel::new(bastion.id.clone()))
    );
    assert!(crate::app::find_connection(&backend.0.state, &routed.id)
        .await
        .unwrap_err()
        .contains("not found"));

    let unreferenced = backend
        .save_development_bastion(None, bastion_form("other.invalid", 22), password(SECRET))
        .await
        .unwrap();
    assert_eq!(
        backend
            .delete_development_bastion(unreferenced.id, Vec::new())
            .await
            .unwrap(),
        DevelopmentBastionDelete::Deleted
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_key_trust_changes_only_by_reviewed_compare_and_set() {
    const CASE: &str = "host_key_trust_changes_only_by_reviewed_compare_and_set";
    if !child(CASE) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let backend = general(&temp.path().canonicalize().unwrap()).await;
    let bastion = backend
        .save_development_bastion(None, bastion_form("bastion.invalid", 22), password(SECRET))
        .await
        .unwrap();
    assert_eq!(bastion.host_key_fingerprint, None);
    let first = "SHA256:AAAAfirst+key/1";
    let second = "SHA256:BBBBsecond+key/2";
    let trusted = backend
        .trust_development_bastion_host_key(bastion.id.clone(), None, first.into())
        .await
        .unwrap();
    assert_eq!(trusted.host_key_fingerprint.as_deref(), Some(first));

    // A stale review (made when nothing was trusted) cannot replace a key.
    assert!(backend
        .trust_development_bastion_host_key(bastion.id.clone(), None, second.into())
        .await
        .unwrap_err()
        .contains("changed since review"));
    for invalid in ["md5:aa", "SHA256:", "SHA256:has space", "SHA256:a\0"] {
        assert!(backend
            .trust_development_bastion_host_key(
                bastion.id.clone(),
                Some(first.into()),
                invalid.into()
            )
            .await
            .is_err());
    }
    let replaced = backend
        .trust_development_bastion_host_key(bastion.id.clone(), Some(first.into()), second.into())
        .await
        .unwrap();
    assert_eq!(replaced.host_key_fingerprint.as_deref(), Some(second));

    let reset = backend
        .reset_development_bastion_host_key(bastion.id.clone())
        .await
        .unwrap();
    assert_eq!(reset.host_key_fingerprint, None);

    backend
        .trust_development_bastion_host_key(bastion.id.clone(), None, first.into())
        .await
        .unwrap();
    let renamed = backend
        .save_development_bastion(
            Some(bastion.id.clone()),
            DevelopmentBastionForm {
                name: "Renamed".into(),
                ..bastion_form("bastion.invalid", 22)
            },
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(renamed.host_key_fingerprint.as_deref(), Some(first));
    let moved = backend
        .save_development_bastion(
            Some(bastion.id.clone()),
            bastion_form("moved.invalid", 22),
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        moved.host_key_fingerprint, None,
        "a new endpoint must be tested and trusted again"
    );
    backend.shutdown().await.unwrap();
}

/// A loopback peer that accepts and immediately closes, counting attempts.
async fn closing_listener() -> (u16, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (port, accepted, task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn routed_lanes_dial_the_bastion_and_never_the_database_directly() {
    const CASE: &str = "routed_lanes_dial_the_bastion_and_never_the_database_directly";
    if !child(CASE) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let backend = general(&temp.path().canonicalize().unwrap()).await;
    let (bastion_port, bastion_accepts, bastion_task) = closing_listener().await;
    let (database_port, database_accepts, database_task) = closing_listener().await;
    let bastion = backend
        .save_development_bastion(
            None,
            bastion_form("127.0.0.1", bastion_port),
            password(SECRET),
        )
        .await
        .unwrap();
    let form = connection(database_port, Some(&bastion.id));
    let routed = backend
        .save_development_connection(None, form.clone(), "pw".into())
        .await
        .unwrap();

    // Explicit Test.
    let result = backend
        .test_development_connection(Some(routed.id.clone()), form.clone(), String::new())
        .await
        .unwrap();
    assert!(matches!(
        result,
        DevelopmentConnectionTest::Failed {
            reason: DevelopmentConnectionFailure::SshTunnel
        }
    ));

    // Diagnosis reports the route as its own failed stage.
    let (_control, request) = backend.connection_diagnosis_control().unwrap();
    let report = backend
        .diagnose_native_connection(request, Some(routed.id.clone()), form, String::new())
        .await
        .unwrap();
    assert_eq!(report.stages[0].stage, NativeStageKind::Tunnel);
    assert!(matches!(
        report.stages[0].result,
        NativeStageResult::Failed {
            kind: NativeFailureKind::TunnelFailed,
            ..
        }
    ));

    // The shared lookup used by query sessions and data lanes.
    assert!(crate::app::find_connection(&backend.0.state, &routed.id)
        .await
        .is_err());

    // An explicit bastion Test never persists an unreviewed key.
    assert!(backend
        .test_development_bastion(bastion.id.clone())
        .await
        .is_err());
    assert_eq!(
        backend.development_bastions().await.unwrap()[0].host_key_fingerprint,
        None
    );

    assert!(bastion_accepts.load(Ordering::SeqCst) >= 4);
    assert_eq!(database_accepts.load(Ordering::SeqCst), 0);
    backend.shutdown().await.unwrap();
    bastion_task.abort();
    database_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owned_fixture_profiles_do_not_offer_bastions() {
    const CASE: &str = "owned_fixture_profiles_do_not_offer_bastions";
    if !child(CASE) {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let fixtures = crate::backend::DevelopmentFixtures::from_json(
        &serde_json::json!({
            "version":1,"fixture":"dbunk-native-stage03",
            "instance":"2283820d-33ec-4c4c-ae03-7051092bd410",
            "host":"127.0.0.1","port":15432,"database":"dbunk_demo","user":"dbunk"
        })
        .to_string(),
    )
    .unwrap();
    let backend =
        Backend::create_development(&temp.path().canonicalize().unwrap().join("p"), fixtures)
            .await
            .unwrap();
    assert!(backend
        .development_bastions()
        .await
        .unwrap_err()
        .contains("general PostgreSQL profile"));
    assert!(backend
        .test_development_bastion("any".into())
        .await
        .is_err());
    backend.shutdown().await.unwrap();
}

#[test]
fn route_errors_distinguish_host_key_refusals() {
    let later = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    for message in [
        format!("{} for h:22. Expected a, got b.", tunnel::HOST_KEY_CHANGED),
        format!("{} for h:22 (observed b).", tunnel::HOST_KEY_UNTRUSTED),
    ] {
        assert!(matches!(
            classify_route_error(&message, later),
            DevelopmentConnectionFailure::SshHostKey
        ));
    }
    assert!(matches!(
        classify_route_error("SSH handshake failed", later),
        DevelopmentConnectionFailure::SshTunnel
    ));
    assert!(matches!(
        classify_route_error("SSH handshake failed", tokio::time::Instant::now()),
        DevelopmentConnectionFailure::Timeout
    ));
}

/// Requires an owned, disposable local sshd fixture described by
/// `DBUNK_SSHD_FIXTURE=host:port:user:password`. Never point this at a real
/// host. Run with `--ignored`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs an owned local sshd fixture"]
async fn live_sshd_unknown_key_is_reviewed_then_trusted_and_a_changed_key_is_refused() {
    const CASE: &str =
        "live_sshd_unknown_key_is_reviewed_then_trusted_and_a_changed_key_is_refused";
    if std::env::var(CHILD).as_deref() != Ok(CASE) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("backend::bastions::tests::{CASE}"),
                "--nocapture",
                "--ignored",
            ])
            .env(CHILD, CASE)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = std::env::var("DBUNK_SSHD_FIXTURE").expect("DBUNK_SSHD_FIXTURE");
    let parts = fixture.splitn(4, ':').collect::<Vec<_>>();
    let [host, port, user, secret] = parts[..] else {
        panic!("DBUNK_SSHD_FIXTURE must be host:port:user:password");
    };
    let temp = tempfile::tempdir().unwrap();
    let backend = general(&temp.path().canonicalize().unwrap()).await;
    let bastion = backend
        .save_development_bastion(
            None,
            DevelopmentBastionForm {
                user: user.into(),
                ..bastion_form(host, port.parse().unwrap())
            },
            password(secret),
        )
        .await
        .unwrap();
    let unknown = backend
        .test_development_bastion(bastion.id.clone())
        .await
        .unwrap();
    assert_eq!(unknown.host_key, DevelopmentHostKeyStatus::Unknown);
    assert_eq!(
        unknown.authentication,
        DevelopmentBastionAuthentication::NotAttempted
    );
    backend
        .trust_development_bastion_host_key(
            bastion.id.clone(),
            None,
            unknown.observed_fingerprint.clone(),
        )
        .await
        .unwrap();
    let trusted = backend
        .test_development_bastion(bastion.id.clone())
        .await
        .unwrap();
    assert_eq!(trusted.host_key, DevelopmentHostKeyStatus::Trusted);
    assert_eq!(
        trusted.authentication,
        DevelopmentBastionAuthentication::Authenticated
    );
    // Simulate a rotated server key by trusting a different fingerprint.
    backend
        .trust_development_bastion_host_key(
            bastion.id.clone(),
            Some(unknown.observed_fingerprint.clone()),
            "SHA256:simulatedPreviousKey".into(),
        )
        .await
        .unwrap();
    let changed = backend
        .test_development_bastion(bastion.id.clone())
        .await
        .unwrap();
    assert!(matches!(
        changed.host_key,
        DevelopmentHostKeyStatus::Changed { .. }
    ));
    assert_eq!(
        changed.authentication,
        DevelopmentBastionAuthentication::NotAttempted
    );
    backend.shutdown().await.unwrap();
}
