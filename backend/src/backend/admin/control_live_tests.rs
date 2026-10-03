//! Explicit stage03 opt-in. Signals only the dedicated session created here;
//! fixture UUID, PID/start/user/database/application identity are checked first.
use super::*;
use crate::{
    backend::profile,
    postgres::{
        connect_spec::ResolvedPostgresConnectSpec,
        dedicated::{self, DriverJoins, NoticeSink},
    },
};
use futures_util::FutureExt;
use std::{path::Path, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";

async fn guard() -> u64 {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    tokio::task::spawn_blocking(|| {
        let output = std::process::Command::new("python3").arg("-c")
            .arg("import sys; sys.path.insert(0,sys.argv[1]); import fixture; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,\"SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()\"))")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE).output().expect("fixture verification helper");
        assert!(output.status.success(), "owned fixture verification failed");
        String::from_utf8(output.stdout).unwrap().trim().parse().unwrap()
    }).await.unwrap()
}
async fn policy(backend: &Backend, read_only: bool) {
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!("PostgreSQL")
    };
    pg.read_only = read_only;
    pg.safe_mode = crate::SafeMode::Strict;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
}
fn selected(capture: &AdminCapture, pid: i32, start: &str, application: &str) -> AdminRow {
    let index = capture
        .snapshot()
        .sessions
        .iter()
        .position(|row| row.pid == pid)
        .expect("owned target captured");
    let row = &capture.snapshot().sessions[index];
    assert_eq!(row.backend_start.as_deref(), Some(start));
    assert_eq!(row.database.as_deref(), Some("dbunk_demo"));
    assert_eq!(row.user.as_deref(), Some("dbunk"));
    assert_eq!(row.application_name.as_deref(), Some(application));
    AdminRow {
        section: AdminSection::Sessions,
        index,
    }
}
fn outcome(result: AdminControlSubmission) -> AdminControlOutcome {
    match result {
        AdminControlSubmission::Finished(receipt) => receipt.outcome,
        _ => panic!("unexpected confirmation"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "exact owned stage03 UUID; explicit DBUNK_NATIVE_FIXTURE_VERIFIED=1; serial exclusive fixture use"]
async fn native_admin_control_owned_identity_policy_signals_audit_and_joined_cleanup() {
    let baseline = guard().await;
    let directory = profile::directory();
    let application = format!("native_admin_control_{}", uuid::Uuid::new_v4().simple());
    println!("owned stage03 UUID={FIXTURE} target application={application} temporary profile={} baseline={baseline}", directory.path().display());
    let drivers = DriverJoins::default();
    let mut backend = None;
    let mut target = None;
    let mut query = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let document = backend
            .open_data_document("admin-control-live", "control", &backend.fixture().id)
            .await
            .unwrap();
        guard().await;
        let connection = crate::app::find_connection(&backend.0.state, &backend.fixture().id)
            .await
            .unwrap();
        let spec = ResolvedPostgresConnectSpec::from_connection(&connection).unwrap();
        assert_eq!(
            (
                spec.host.as_str(),
                spec.port,
                spec.database.as_str(),
                spec.user.as_str()
            ),
            ("127.0.0.1", 15432, "dbunk_demo", "dbunk")
        );
        target = Some(
            dedicated::connect_tracked(&spec, NoticeSink::Ignore, Some(&drivers))
                .await
                .unwrap(),
        );
        let client = target.as_ref().unwrap().client.clone();
        guard().await;
        client
            .query_one(
                "SELECT pg_catalog.set_config('application_name',$1,false)",
                &[&application],
            )
            .await
            .unwrap();
        guard().await;
        let identity = client.query_one("SELECT pid,pg_catalog.to_char(backend_start AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') FROM pg_catalog.pg_stat_activity WHERE pid=pg_catalog.pg_backend_pid() AND datname='dbunk_demo' AND usename='dbunk' AND application_name=$1", &[&application]).await.unwrap();
        let pid: i32 = identity.get(0);
        let start: String = identity.get(1);
        println!("owned target pid={pid} backend_start={start}");
        guard().await;
        query = Some(tokio::spawn(async move {
            client.batch_execute("SELECT pg_catalog.pg_sleep(30)").await
        }));
        let capture = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                guard().await;
                let capture = backend.admin_capture(&document).await.unwrap();
                let row = selected(&capture, pid, &start, &application);
                let session = &capture.snapshot().sessions[row.index];
                if session.state.as_deref() == Some("active")
                    && session
                        .query
                        .as_deref()
                        .is_some_and(|sql| sql.contains("pg_sleep(30)"))
                {
                    break capture;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let row = selected(&capture, pid, &start, &application);
        // Deliberately malformed internal authority is test-only. Production
        // callers cannot construct or edit these fields.
        let mut stale = capture
            .review(row, AdminControlAction::CancelQuery)
            .unwrap();
        stale.target.backend_start = "2000-01-01T00:00:00.000000Z".into();
        guard().await;
        assert_eq!(
            outcome(backend.apply_admin_control(stale).await.unwrap()),
            AdminControlOutcome::TargetChanged
        );
        let mut stale = capture
            .review(row, AdminControlAction::CancelQuery)
            .unwrap();
        stale.target.query_start = Some("2000-01-01T00:00:00.000000Z".into());
        guard().await;
        assert_eq!(
            outcome(backend.apply_admin_control(stale).await.unwrap()),
            AdminControlOutcome::TargetChanged
        );
        assert!(!query.as_ref().unwrap().is_finished());
        policy(backend, true).await;
        guard().await;
        assert_eq!(
            outcome(
                backend
                    .apply_admin_control(
                        capture
                            .review(row, AdminControlAction::CancelQuery)
                            .unwrap()
                    )
                    .await
                    .unwrap()
            ),
            AdminControlOutcome::SignalSent
        );
        let cancelled = tokio::time::timeout(Duration::from_secs(5), query.as_mut().unwrap())
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        query.take();
        assert_eq!(cancelled.code().map(|code| code.code()), Some("57014"));
        guard().await;
        let capture = backend.admin_capture(&document).await.unwrap();
        let row = selected(&capture, pid, &start, &application);
        guard().await;
        assert!(matches!(
            backend
                .apply_admin_control(
                    capture
                        .review(row, AdminControlAction::TerminateSession)
                        .unwrap()
                )
                .await,
            Err(AdminControlError::PolicyBlocked)
        ));
        policy(backend, false).await;
        guard().await;
        let confirmation = match backend
            .apply_admin_control(
                capture
                    .review(row, AdminControlAction::TerminateSession)
                    .unwrap(),
            )
            .await
            .unwrap()
        {
            AdminControlSubmission::NeedsConfirmation(value) => value,
            _ => panic!("Strict termination requires confirmation"),
        };
        assert_eq!(confirmation.target().pid(), pid);
        assert_eq!(confirmation.target().backend_start(), start);
        guard().await;
        assert_eq!(
            outcome(backend.confirm_admin_control(*confirmation).await.unwrap()),
            AdminControlOutcome::SignalSent
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while !target.as_ref().unwrap().is_closed() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let audit =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].command, "terminate_pg_backend");
        println!("observed stale-start refusal; stale-query refusal; read-only cancellation 57014; read-only termination refusal; exact Strict confirmation; terminate signal and target closure; one required override audit");
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    if let Some(query) = query {
        query.abort();
        let _ = query.await;
    }
    if let Some(target) = target {
        if tokio::time::timeout(Duration::from_secs(2), target.close())
            .await
            .is_err()
        {
            drivers.abort_all();
        }
    }
    drivers.drain().await;
    let restored = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = guard().await;
            if current == baseline {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    shutdown.expect("administration backend shutdown joined");
    println!(
        "joined target/backend cleanup baseline={baseline} final={}",
        restored.expect("activity baseline restored")
    );
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("administration control live deadline");
}
