//! Launcher-verified fixture acceptance of native workspace services. Explicit
//! general operations use only a newly owned disposable general-capability
//! profile. Both modes query the fixed owned fixture, never a personal profile.
use dbunk_lib::backend::*;
use std::{io::Read, path::PathBuf, sync::Arc, time::Duration};

const SQL: &str = "SELECT 'draft 👩🏽‍💻' AS text;\n-- preserved without execution on reopen\n";

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Native workspace probe: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [operation, path, manifest] = args.as_slice() else {
        return Err(
            "Expected create|reopen, a new owned profile path, and verified manifest".into(),
        );
    };
    let mut json = String::new();
    std::fs::File::open(manifest)
        .map_err(|_| "Manifest unavailable")?
        .take(4097)
        .read_to_string(&mut json)
        .map_err(|_| "Manifest unreadable")?;
    let fixtures = DevelopmentFixtures::from_json(&json)?;
    let path = PathBuf::from(path);
    let (creating, general) = match operation.to_str() {
        Some("create") => (true, false),
        Some("reopen") => (false, false),
        Some("create-general") => (true, true),
        Some("reopen-general") => (false, true),
        _ => return Err("Expected create, reopen, create-general or reopen-general".into()),
    };
    let backend = match (creating, general) {
        (true, true) => Backend::create_native_profile(&path).await?,
        (false, true) => Backend::open_native_profile(&path).await?,
        (true, false) => Backend::create_development(&path, fixtures).await?,
        (false, false) => Backend::open_development(&path, &fixtures).await?,
    };
    let expected = if general {
        NativeProfileKind::GeneralPostgres
    } else {
        NativeProfileKind::OwnedFixtures
    };
    if backend.native_profile_kind() != Some(expected) {
        let _ = backend.shutdown().await;
        return Err("Probe selected the wrong profile capability".into());
    }
    let result = if creating {
        create(&backend).await
    } else {
        reopen(&backend).await
    };
    let cleanup = backend.shutdown().await;
    result?;
    cleanup?;
    println!(
        "PASS: {} and joined backend shutdown",
        if creating {
            "two saved connections, explicit probes, query sessions and durable drafts"
        } else {
            "encrypted profile unlock and exact disconnected draft restoration"
        }
    );
    Ok(())
}

async fn create(backend: &Backend) -> Result<(), String> {
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await?;
    let form = DevelopmentPostgresConnection {
        name: "Owned Postgres".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: Default::default(),
        driver_options: Default::default(),
        ssh_tunnel: None,
    };
    let first = backend
        .save_development_connection(None, form, "dbunk".into())
        .await?;
    let second = backend
        .duplicate_development_connection(first.id.clone())
        .await?;
    for connection in [&first, &second] {
        let result = backend
            .test_development_connection(
                Some(connection.id.clone()),
                connection.postgres.clone().ok_or("Missing form")?,
                String::new(),
            )
            .await?;
        if !matches!(result, DevelopmentConnectionTest::Reachable { .. }) {
            return Err("Owned connection probe failed".into());
        }
    }
    backend
        .register_owner(
            "workspace-probe",
            RegisterOwnerPayload {
                owner_id: "owner".into(),
            },
        )
        .await
        .map_err(|_| "Owner registration failed")?;
    // Both sessions share the window owner, but retain distinct connection/tab IDs.
    let (left, right) = tokio::join!(
        query(backend, &first.id, "left"),
        query(backend, &second.id, "right")
    );
    left?;
    right?;
    let documents = [&first, &second]
        .into_iter()
        .enumerate()
        .map(|(index, connection)| WorkspaceDocument {
            query_changes: None,
            schema_changes: None,
            table_ddl: None,
            schema_alter: None,
            object_ddl: None,
            admin_control: None,
            maintenance: None,
            tool: None,
            saved_query_id: None,
            table: None,
            id: format!("document-{index}"),
            name: format!("Query {}", index + 1),
            connection_id: Some(connection.id.clone()),
            sql: SQL.into(),
            pinned: index == 0,
            selection: WorkspaceSelection { anchor: 7, head: 7 },
        })
        .collect();
    backend
        .save_development_workspace(
            None,
            WorkspaceSnapshot {
                copy_jobs: Vec::new(),
                seed_jobs: Vec::new(),
                documents,
                active_document_id: Some("document-1".into()),
                layout: Layout::SideBySide,
                density: WorkspaceDensity::Compact,
                navigator_width: 260.0,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    backend
        .change_development_credentials(
            DevelopmentStorageMode::EncryptedSqlite,
            Some("disposable-probe-passphrase".into()),
            true,
        )
        .await?;
    Ok(())
}

async fn query(backend: &Backend, connection: &str, tab: &str) -> Result<(), String> {
    let (send, mut receive) = tokio::sync::mpsc::channel(64);
    backend
        .open(
            "workspace-probe",
            OpenSessionPayload {
                owner_id: "owner".into(),
                session_id: tab.into(),
                tab_id: tab.into(),
                connection_id: connection.into(),
            },
            Arc::new(move |event| send.try_send(event).map_err(|_| SinkClosed)),
        )
        .await
        .map_err(|_| "Session open failed")?;
    backend
        .execute(
            "workspace-probe",
            ExecutePayload {
                session_id: tab.into(),
                execution_id: tab.into(),
                sql: "SELECT 42 AS answer;".into(),
                confirmed: false,
                parameters: None,
                row_limit: None,
            },
        )
        .await
        .map_err(|_| "Query dispatch failed")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut saw_row = false;
        while let Some(event) = receive.recv().await {
            if event.connection_id != connection || event.tab_id != tab || event.session_id != tab {
                return Err("Cross-session event identity".to_string());
            }
            if let QueryEvent::RowBatch { rows, .. } = &event.event {
                saw_row |= rows == &[vec![Some("42".into())]];
            }
            if event.requires_ack {
                backend
                    .ack(
                        "workspace-probe",
                        AckPayload {
                            session_id: tab.into(),
                            execution_id: tab.into(),
                            ack_through_sequence: event.sequence,
                            retain_more_rows: true,
                        },
                    )
                    .await
                    .map_err(|_| "Query ACK failed")?;
            }
            if let QueryEvent::ExecutionCompleted { status, .. } = event.event {
                if status != "completed" || !saw_row {
                    return Err("Query result was not exact".into());
                }
                return Ok(());
            }
        }
        Err("Query stream closed early".into())
    })
    .await
    .map_err(|_| "Query timed out")??;
    backend
        .close(
            "workspace-probe",
            SessionPayload {
                session_id: tab.into(),
            },
        )
        .await
        .map_err(|_| "Session close failed".to_string())
}

async fn reopen(backend: &Backend) -> Result<(), String> {
    if backend.development_settings().await?.state != DevelopmentCredentialState::NeedsUnlock {
        return Err("Encrypted profile did not reopen locked".into());
    }
    if backend
        .unlock_development_credentials("wrong-probe-password".into())
        .await
        .is_ok()
    {
        return Err("Wrong password was accepted".into());
    }
    backend
        .unlock_development_credentials("disposable-probe-passphrase".into())
        .await?;
    let connections = backend.development_connections().await?;
    let loaded = backend
        .load_development_workspace()
        .await
        .map_err(|error| error.to_string())?;
    let snapshot = loaded.snapshot.ok_or("Workspace missing")?;
    if connections.len() != 2
        || snapshot.documents.len() != 2
        || loaded.revision.is_none()
        || snapshot.active_document_id.as_deref() != Some("document-1")
        || snapshot.layout != Layout::SideBySide
        || snapshot.density != WorkspaceDensity::Compact
        || snapshot.navigator_width != 260.0
        || snapshot
            .documents
            .iter()
            .enumerate()
            .any(|(index, document)| {
                document.sql != SQL
                    || document.pinned != (index == 0)
                    || document.selection != (WorkspaceSelection { anchor: 7, head: 7 })
                    || !connections
                        .iter()
                        .any(|connection| Some(&connection.id) == document.connection_id.as_ref())
            })
    {
        return Err("Restored workspace differs from acknowledged snapshot".into());
    }
    // No open/test/execute call here: restoration must remain disconnected.
    Ok(())
}
