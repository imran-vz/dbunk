//! Invoked only by the ownership-checking native TLS fixture launcher.
use dbunk_lib::backend::connection_diagnosis::*;
use dbunk_lib::backend::*;
use std::{io::Read, path::PathBuf, sync::Arc, time::Duration};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Native TLS probe: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [path, manifest, trusted, untrusted] = args.as_slice() else {
        return Err("Expected new profile, verified manifest, trusted CA, untrusted CA".into());
    };
    let mut encoded = String::new();
    std::fs::File::open(manifest)
        .map_err(|_| "Manifest unavailable")?
        .take(4097)
        .read_to_string(&mut encoded)
        .map_err(|_| "Manifest unreadable")?;
    let backend = Backend::create_development(
        &PathBuf::from(path),
        DevelopmentFixtures::from_json(&encoded)?,
    )
    .await?;
    let result = matrix(
        &backend,
        trusted.to_string_lossy().into_owned(),
        untrusted.to_string_lossy().into_owned(),
    )
    .await;
    let cleanup = backend.shutdown().await;
    result?;
    cleanup?;
    println!("PASS: native TLS matrix and joined backend shutdown");
    Ok(())
}
async fn matrix(backend: &Backend, trusted: String, untrusted: String) -> Result<(), String> {
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await?;
    let form = DevelopmentPostgresConnection {
        name: "Owned TLS PostgreSQL".into(),
        host: "127.0.0.1".into(),
        port: 15433,
        database: "dbunk_tls_demo".into(),
        user: "dbunk".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: DevelopmentTlsOptions {
            mode: DevelopmentTlsMode::VerifyFull,
            root_cert_path: Some(trusted.clone()),
            ..Default::default()
        },
        driver_options: Default::default(),
    };
    let saved = backend
        .save_development_connection(None, form.clone(), "dbunk".into())
        .await?;
    for (label, mode, ca, name, failure) in [
        (
            "trusted verify-full",
            DevelopmentTlsMode::VerifyFull,
            trusted.clone(),
            None,
            None,
        ),
        (
            "untrusted CA",
            DevelopmentTlsMode::VerifyFull,
            untrusted,
            None,
            Some(TlsFailureKind::CertificateUntrusted),
        ),
        (
            "hostname mismatch",
            DevelopmentTlsMode::VerifyFull,
            trusted.clone(),
            Some("wrong.example"),
            Some(TlsFailureKind::HostnameMismatch),
        ),
        (
            "verify-ca ignores hostname",
            DevelopmentTlsMode::VerifyCa,
            trusted.clone(),
            Some("wrong.example"),
            None,
        ),
        (
            "require TLS",
            DevelopmentTlsMode::Require,
            trusted.clone(),
            None,
            None,
        ),
        (
            "prefer TLS",
            DevelopmentTlsMode::Prefer,
            trusted,
            None,
            None,
        ),
    ] {
        let mut input = form.clone();
        input.tls.mode = mode;
        input.tls.root_cert_path = Some(ca);
        input.tls.server_name = name.map(str::to_owned);
        let result = backend
            // TLS edits require an explicit password for an unsaved probe.
            .test_development_connection(Some(saved.id.clone()), input.clone(), "dbunk".into())
            .await?;
        let accepted = match (result, failure) {
            (DevelopmentConnectionTest::Reachable { .. }, None) => true,
            (
                DevelopmentConnectionTest::Failed {
                    reason: DevelopmentConnectionFailure::Tls(actual),
                },
                Some(expected),
            ) => actual == expected,
            _ => false,
        };
        if !accepted {
            return Err(format!("Unexpected native TLS outcome for {label}"));
        }
        println!("PASS: {label}");
        let (_control, request) = backend.connection_diagnosis_control()?;
        let report = backend
            .diagnose_native_connection(request, Some(saved.id.clone()), input, "dbunk".into())
            .await?;
        if report.checked_heap_bytes().is_none() || report.stages.len() != 6 {
            return Err("Staged diagnosis exceeded its bounded report contract".into());
        }
        let accepted = match (&report.outcome, failure) {
            (NativeDiagnosisOutcome::Reachable { .. }, None) => true,
            (
                NativeDiagnosisOutcome::Failed {
                    stage: NativeStageKind::Tls,
                },
                Some(expected),
            ) => {
                matches!(&report.stages[3].result, NativeStageResult::Failed { kind, .. } if *kind == match expected {
                    TlsFailureKind::CertificateUntrusted => NativeFailureKind::CertificateUntrusted,
                    TlsFailureKind::HostnameMismatch => NativeFailureKind::HostnameMismatch,
                    _ => return Err("Unexpected fixture TLS failure case".into()),
                })
            }
            _ => false,
        };
        if !accepted {
            return Err(format!(
                "Unexpected staged TLS outcome for {label}: {report:?}"
            ));
        }
        if failure.is_none() {
            let NativeStageResult::Passed {
                detail:
                    Some(NativeStageDetail::Tls {
                        encrypted,
                        protocol,
                        cipher,
                        certificate_verified,
                        hostname_verified,
                        ..
                    }),
                ..
            } = &report.stages[3].result
            else {
                return Err("Staged TLS observation missing".into());
            };
            if !encrypted
                || protocol.is_none()
                || cipher.is_none()
                || *certificate_verified
                    != matches!(
                        mode,
                        DevelopmentTlsMode::VerifyCa | DevelopmentTlsMode::VerifyFull
                    )
                || *hostname_verified != matches!(mode, DevelopmentTlsMode::VerifyFull)
            {
                return Err(
                    "Staged TLS facts did not match the observed verified connection".into(),
                );
            }
        }
        println!("PASS: staged {label}");
    }
    backend
        .register_owner(
            "tls-probe",
            RegisterOwnerPayload {
                owner_id: "owner".into(),
            },
        )
        .await
        .map_err(|_| "Owner registration failed")?;
    let (send, mut receive) = tokio::sync::mpsc::channel(64);
    backend
        .open(
            "tls-probe",
            OpenSessionPayload {
                owner_id: "owner".into(),
                session_id: "tls".into(),
                tab_id: "tls".into(),
                connection_id: saved.id.clone(),
            },
            Arc::new(move |event| send.try_send(event).map_err(|_| SinkClosed)),
        )
        .await
        .map_err(|_| "TLS query session open failed")?;
    backend
        .execute(
            "tls-probe",
            ExecutePayload {
                session_id: "tls".into(),
                execution_id: "ssl".into(),
                sql: "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()".into(),
                confirmed: false,
                parameters: None,
                row_limit: None,
            },
        )
        .await
        .map_err(|_| "TLS query dispatch failed")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut encrypted = false;
        while let Some(event) = receive.recv().await {
            if event.connection_id != saved.id || event.session_id != "tls" || event.tab_id != "tls"
            {
                return Err("Foreign TLS event".to_string());
            }
            if let QueryEvent::RowBatch { rows, .. } = &event.event {
                encrypted |= rows == &[vec![Some("t".into())]];
            }
            if event.requires_ack {
                backend
                    .ack(
                        "tls-probe",
                        AckPayload {
                            session_id: "tls".into(),
                            execution_id: "ssl".into(),
                            ack_through_sequence: event.sequence,
                            retain_more_rows: true,
                        },
                    )
                    .await
                    .map_err(|_| "TLS ACK failed")?;
            }
            if let QueryEvent::ExecutionCompleted { status, .. } = event.event {
                return if status == "completed" && encrypted {
                    Ok(())
                } else {
                    Err("TLS query did not prove encryption".into())
                };
            }
        }
        Err("TLS stream closed early".into())
    })
    .await
    .map_err(|_| "TLS query timeout")??;
    backend
        .close_native_session("tls-probe", "tls")
        .await
        .map_err(|_| "TLS session close failed")?;
    println!("PASS: saved native connection opens an encrypted query session");
    let mut plain = form;
    plain.port = 15432;
    plain.database = "dbunk_demo".into();
    plain.tls = DevelopmentTlsOptions::default();
    let (_control, request) = backend.connection_diagnosis_control()?;
    let report = backend
        .diagnose_native_connection(request, None, plain, "dbunk".into())
        .await?;
    if !matches!(report.outcome, NativeDiagnosisOutcome::Reachable { .. })
        || !matches!(
            report.stages[3].result,
            NativeStageResult::Skipped {
                reason: NativeSkipReason::TlsDisabled
            }
        )
        || !report
            .warnings
            .contains(&NativeDiagnosisWarning::NotEncrypted)
    {
        return Err("Plain owned diagnosis did not disclose disabled encryption".into());
    }
    println!("PASS: staged plain connection and encryption warning");
    Ok(())
}
