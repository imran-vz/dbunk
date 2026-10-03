use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn pg(port: u16, mode: PgTlsMode, timeout: u32) -> crate::PgStoredConnection {
    crate::PgStoredConnection {
        organization: Default::default(),
        id: "owned-unit-probe".into(),
        name: "Owned unit probe".into(),
        database: "test".into(),
        host: "127.0.0.1".into(),
        port,
        user: "test".into(),
        password: "secret-must-not-appear".into(),
        role: String::new(),
        environment: Default::default(),
        safe_mode: Default::default(),
        read_only: false,
        last_activity_at: None,
        ssl: mode != PgTlsMode::Disable,
        tls_options: Some(crate::PgTlsOptions {
            mode,
            ..Default::default()
        }),
        driver_options: Some(crate::PgDriverOptions {
            connect_timeout_ms: Some(timeout),
            ..Default::default()
        }),
        ssh_tunnel: Default::default(),
    }
}
async fn startup_packet(stream: &mut TcpStream) {
    let length = stream.read_u32().await.unwrap() as usize;
    assert!((8..8192).contains(&length));
    let mut payload = vec![0; length - 4];
    stream.read_exact(&mut payload).await.unwrap();
}
async fn ready(stream: &mut TcpStream) {
    stream
        .write_all(&[b'R', 0, 0, 0, 8, 0, 0, 0, 0, b'Z', 0, 0, 0, 5, b'I'])
        .await
        .unwrap();
}
async fn closed(mut stream: TcpStream) {
    let mut bytes = [0; 4096];
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match stream.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    })
    .await
    .expect("owned probe socket must close before joined completion");
}
#[tokio::test]
async fn database_deadline_is_not_misattributed_and_driver_is_joined() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        startup_packet(&mut stream).await;
        ready(&mut stream).await;
        closed(stream).await;
    });
    let (_cancel, receiver) = watch::channel(false);
    let drivers = DriverJoins::default();
    let report = run(&pg(port, PgTlsMode::Disable, 150), &drivers, receiver)
        .await
        .unwrap();
    assert!(matches!(
        report.outcome,
        NativeDiagnosisOutcome::Failed {
            stage: NativeStageKind::Database
        }
    ));
    assert!(matches!(
        report.stages[4].result,
        NativeStageResult::Passed { .. }
    ));
    assert!(matches!(
        report.stages[5].result,
        NativeStageResult::Failed {
            kind: NativeFailureKind::TimedOut,
            ..
        }
    ));
    server.await.unwrap();
    assert!(report.checked_heap_bytes().is_some());
}
#[tokio::test]
async fn cancellation_after_startup_closes_and_joins_driver() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (entered, started) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        startup_packet(&mut stream).await;
        ready(&mut stream).await;
        let mut byte = [0];
        stream.read_exact(&mut byte).await.unwrap();
        let _ = entered.send(());
        closed(stream).await;
    });
    let (cancel, receiver) = watch::channel(false);
    let drivers = DriverJoins::default();
    let probe =
        tokio::spawn(
            async move { run(&pg(port, PgTlsMode::Disable, 2000), &drivers, receiver).await },
        );
    started.await.unwrap();
    cancel.send(true).unwrap();
    assert!(probe.await.unwrap().unwrap_err().contains("cancelled"));
    server.await.unwrap();
}
#[tokio::test]
async fn failed_authentication_does_not_echo_server_or_password() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        startup_packet(&mut stream).await;
        let body = b"SERROR\0C28P01\0Mserver-secret-echo\0\0";
        let mut packet = vec![b'E'];
        packet.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        packet.extend_from_slice(body);
        stream.write_all(&packet).await.unwrap();
    });
    let (_cancel, receiver) = watch::channel(false);
    let report = run(
        &pg(port, PgTlsMode::Disable, 2000),
        &DriverJoins::default(),
        receiver,
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert!(matches!(
        report.outcome,
        NativeDiagnosisOutcome::Failed {
            stage: NativeStageKind::Authentication
        }
    ));
    assert!(matches!(
        report.stages[5].result,
        NativeStageResult::Skipped {
            reason: NativeSkipReason::BlockedByEarlierFailure
        }
    ));
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("server-secret-echo"));
    assert!(!json.contains("secret-must-not-appear"));
}
#[tokio::test]
async fn prefer_fallback_remains_visible_after_authentication_failure() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 8];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47]);
        stream.write_all(b"N").await.unwrap();
        startup_packet(&mut stream).await;
        let body = b"SERROR\0C28P01\0Mauthentication refused\0\0";
        let mut packet = vec![b'E'];
        packet.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        packet.extend_from_slice(body);
        stream.write_all(&packet).await.unwrap();
        closed(stream).await;
    });
    let (_cancel, receiver) = watch::channel(false);
    let report = run(
        &pg(port, PgTlsMode::Prefer, 2000),
        &DriverJoins::default(),
        receiver,
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert!(matches!(
        report.outcome,
        NativeDiagnosisOutcome::Failed {
            stage: NativeStageKind::Authentication
        }
    ));
    assert!(matches!(
        report.stages[3].result,
        NativeStageResult::Passed {
            detail: Some(NativeStageDetail::Tls {
                encrypted: false,
                certificate_verified: false,
                hostname_verified: false,
                client_certificate_presented: false,
                ..
            }),
            ..
        }
    ));
    assert!(report
        .warnings
        .contains(&NativeDiagnosisWarning::NotEncrypted));
}

#[tokio::test]
async fn tls_deadline_and_dropped_control_are_explicit() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        closed(stream).await;
    });
    let (_cancel, receiver) = watch::channel(false);
    let report = run(
        &pg(port, PgTlsMode::Require, 150),
        &DriverJoins::default(),
        receiver,
    )
    .await
    .unwrap();
    assert!(matches!(
        report.outcome,
        NativeDiagnosisOutcome::Failed {
            stage: NativeStageKind::Tls
        }
    ));
    server.await.unwrap();
    let (sender, receiver) = watch::channel(false);
    drop(sender);
    assert!(run(
        &pg(port, PgTlsMode::Disable, 2000),
        &DriverJoins::default(),
        receiver
    )
    .await
    .unwrap_err()
    .contains("cancelled"));
}
#[test]
fn report_bound_accounts_actual_capacity() {
    let mut report = Report::new(DatabaseEngine::PostgreSQL);
    report.fail(
        DiagnosisStageKind::Tunnel,
        Instant::now(),
        FailureKind::Other,
        "bounded".into(),
    );
    let mut report = NativeDiagnosis::from_legacy(report.finish());
    assert!(report.checked_heap_bytes().is_some());
    if let NativeStageResult::Failed { message, .. } = &mut report.stages[0].result {
        message.reserve(MAX_DIAGNOSIS_BYTES);
    }
    assert!(report.checked_heap_bytes().is_none());
}

#[tokio::test]
async fn session_option_failure_is_database_failure_and_never_reachable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        startup_packet(&mut stream).await;
        ready(&mut stream).await;
        assert_eq!(stream.read_u8().await.unwrap(), b'Q');
        let size = stream.read_u32().await.unwrap();
        assert!(size < 4096);
        let mut sql = vec![0; size as usize - 4];
        stream.read_exact(&mut sql).await.unwrap();
        assert_eq!(sql, b"SET ROLE \"missing-role\"\0");
        let body = b"SERROR\0C42704\0Mprivate-server-detail\0\0";
        stream.write_u8(b'E').await.unwrap();
        stream.write_u32((body.len() + 4) as u32).await.unwrap();
        stream.write_all(body).await.unwrap();
        stream.write_all(b"Z\0\0\0\x05I").await.unwrap();
        closed(stream).await;
    });
    let mut connection = pg(port, PgTlsMode::Disable, 2000);
    connection.driver_options.as_mut().unwrap().default_role = Some("missing-role".into());
    let (_cancel, receiver) = watch::channel(false);
    let report = run(&connection, &DriverJoins::default(), receiver)
        .await
        .unwrap();
    server.await.unwrap();
    assert!(matches!(
        report.outcome,
        NativeDiagnosisOutcome::Failed {
            stage: NativeStageKind::Database
        }
    ));
    assert!(matches!(
        report.stages[4].result,
        NativeStageResult::Passed { .. }
    ));
    assert!(
        matches!(&report.stages[5].result, NativeStageResult::Failed { message, .. } if message == "Configured PostgreSQL session options could not be applied")
    );
}
