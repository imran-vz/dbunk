//! Headless admission races use a paused startup snapshot instead of a socket.
//! Each marked profile is owned by a separate process; no test contacts Postgres.
use super::*;
use crate::backend::{OpenSessionPayload, QueryEventEnvelope, QuerySessionError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::oneshot;

fn child(case: &str) -> bool {
    const ENV: &str = "DBUNK_STAGE04_ADMISSION_TEST";
    if std::env::var(ENV).as_deref() == Ok(case) {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("backend::development::admission_tests::{case}"),
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

async fn fixture() -> (tempfile::TempDir, Backend, String) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().canonicalize().unwrap().join("profile");
    let fixtures = DevelopmentFixtures::from_json(&serde_json::json!({
        "version": 1, "fixture": "dbunk-native-stage03", "instance": "2283820d-33ec-4c4c-ae03-7051092bd410",
        "host": "127.0.0.1", "port": 15432, "database": "dbunk_demo", "user": "dbunk"
    }).to_string()).unwrap();
    let backend = Backend::create_development(&path, fixtures).await.unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    let connection = backend
        .save_development_connection(None, form("original"), "original-secret".into())
        .await
        .unwrap();
    (directory, backend, connection.id)
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
async fn startup_snapshot_settles_before_queued_connection_mutations() {
    if !child("startup_snapshot_settles_before_queued_connection_mutations") {
        return;
    }
    let (_directory, backend, id) = fixture().await;
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let opening_backend = backend.clone();
    let opening_id = id.clone();
    let opening = tokio::spawn(async move {
        opening_backend
            .development_call(move |state| async move {
                let snapshot = crate::app::find_connection(&state, &opening_id)
                    .await
                    .unwrap();
                assert_eq!(snapshot.name(), "original");
                assert_eq!(snapshot.password(), "original-secret");
                started.send(()).unwrap();
                released.await.unwrap();
                // This is the point a delayed driver startup would finish. Neither
                // metadata nor its password may change before startup settles.
                let still_current = crate::app::find_connection(&state, &opening_id)
                    .await
                    .unwrap();
                assert_eq!(still_current.name(), snapshot.name());
                assert_eq!(still_current.password(), snapshot.password());
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    let first_backend = backend.clone();
    let first_id = id.clone();
    let mut first = tokio::spawn(async move {
        first_backend
            .save_development_connection(Some(first_id), form("first"), "first-secret".into())
            .await
    });
    let second_backend = backend.clone();
    let second_id = id.clone();
    let mut second = tokio::spawn(async move {
        second_backend
            .save_development_connection(Some(second_id), form("second"), "second-secret".into())
            .await
    });
    assert!(tokio::time::timeout(Duration::from_millis(40), &mut first)
        .await
        .is_err());
    assert!(tokio::time::timeout(Duration::from_millis(40), &mut second)
        .await
        .is_err());
    release.send(()).unwrap();
    opening.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    })
    .await
    .unwrap();
    let snapshot = backend
        .development_call(move |state| async move {
            Ok(crate::app::find_connection(&state, &id).await.unwrap())
        })
        .await
        .unwrap();
    assert!(matches!(snapshot.name(), "first" | "second"));
    assert_eq!(snapshot.password(), format!("{}-secret", snapshot.name()));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_open_waits_before_metadata_lookup_and_delete_cannot_cross_startup() {
    if !child("public_open_waits_before_metadata_lookup_and_delete_cannot_cross_startup") {
        return;
    }
    let (_directory, backend, id) = fixture().await;
    let (entered, entry) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let held_backend = backend.clone();
    let held = tokio::spawn(async move {
        held_backend
            .development_call(move |_| async move {
                entered.send(()).unwrap();
                released.await.unwrap();
                Ok(())
            })
            .await
    });
    entry.await.unwrap();
    let opening_backend = backend.clone();
    let mut opening = tokio::spawn(async move {
        // A permanently absent ID ensures no network I/O, even if this test
        // catches a regression that bypasses the native admission gate.
        opening_backend
            .open(
                "window",
                OpenSessionPayload {
                    owner_id: "owner".into(),
                    session_id: "opening".into(),
                    tab_id: "tab".into(),
                    connection_id: "absent-native-admission-test".into(),
                },
                Arc::new(|_: QueryEventEnvelope| Ok(())),
            )
            .await
    });
    let deleting_backend = backend.clone();
    let mut deleting =
        tokio::spawn(async move { deleting_backend.delete_development_connection(id).await });
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut opening)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), &mut deleting)
            .await
            .is_err()
    );
    release.send(()).unwrap();
    held.await.unwrap().unwrap();
    assert!(matches!(
        opening.await.unwrap(),
        Err(QuerySessionError::ConnectionLost)
    ));
    deleting.await.unwrap().unwrap();
    assert!(backend.development_connections().await.unwrap().is_empty());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_rejects_queued_native_work_after_gate_is_released() {
    if !child("shutdown_rejects_queued_native_work_after_gate_is_released") {
        return;
    }
    let (_directory, backend, _id) = fixture().await;
    let (entered, entry) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let held_backend = backend.clone();
    let held = tokio::spawn(async move {
        held_backend
            .development_call(move |_| async move {
                entered.send(()).unwrap();
                released.await.unwrap();
                Ok(())
            })
            .await
    });
    entry.await.unwrap();
    let ran = Arc::new(AtomicBool::new(false));
    let queued_ran = ran.clone();
    let queued_backend = backend.clone();
    let mut queued = tokio::spawn(async move {
        queued_backend
            .development_call(move |_| async move {
                queued_ran.store(true, Ordering::SeqCst);
                Ok(())
            })
            .await
    });
    assert!(tokio::time::timeout(Duration::from_millis(40), &mut queued)
        .await
        .is_err());
    let closing_backend = backend.clone();
    let closing = tokio::spawn(async move { closing_backend.shutdown().await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !backend.0.closing.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    held.await.unwrap().unwrap();
    assert!(matches!(
        queued.await.unwrap(),
        Err(QuerySessionError::ConnectionClosing)
    ));
    assert!(!ran.load(Ordering::SeqCst));
    closing.await.unwrap().unwrap();
}
