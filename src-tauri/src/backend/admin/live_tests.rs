//! Explicit owned stage03 opt-in. Read-only; creates no database objects or roles.
use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use futures_util::FutureExt;
use std::{path::Path, time::Duration};

async fn fixture_count() -> u64 {
    tokio::task::spawn_blocking(|| {
        let output = std::process::Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native/fixture.py"))
            .arg("count")
            .output()
            .expect("run owned fixture verification");
        assert!(
            output.status.success(),
            "owned fixture verification failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture only; run serially"]
async fn native_admin_snapshot_owned_read_cancel_retire_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    // Verifies container/process and SQL instance sentinel before opening sockets.
    let baseline = fixture_count().await;
    let directory = profile::directory();
    let mut backend = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let document = backend
            .open_data_document("admin-live", "snapshot", &backend.fixture().id)
            .await
            .unwrap();
        let snapshot = backend.admin_snapshot(&document).await.unwrap();
        assert_eq!(snapshot.database, "dbunk_demo");
        assert!(snapshot.reader_pid > 0);
        assert!(snapshot.sessions.len() <= MAX_ADMIN_ROWS);
        assert!(snapshot.locks.len() <= MAX_ADMIN_ROWS);
        assert!(snapshot.pending_transactions.len() <= MAX_ADMIN_ROWS);
        assert!(serde_json::to_vec(&snapshot).unwrap().len() <= MAX_ADMIN_BYTES);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&snapshot.collected_start).unwrap()
                <= chrono::DateTime::parse_from_rfc3339(&snapshot.collected_end).unwrap()
        );
        assert!(snapshot.scope_note.contains("cluster-wide"));
        if !snapshot.sessions_truncated {
            let reader = snapshot
                .sessions
                .iter()
                .find(|row| row.pid == snapshot.reader_pid)
                .expect("snapshot reader session");
            assert_eq!(reader.database.as_deref(), Some("dbunk_demo"));
            assert_eq!(reader.state.as_deref(), Some("active"));
            assert!(!reader.details_restricted);
            assert!(reader.backend_start.is_some() && reader.query_start.is_some());
            // Activity can cache the earlier identity SELECT in this transaction.
            assert!(reader.query.as_ref().unwrap().chars().count() <= MAX_ADMIN_QUERY_CHARS);
        }
        for row in snapshot
            .sessions
            .iter()
            .chain(&snapshot.pending_transactions)
        {
            if row.query_clipped {
                assert_eq!(
                    row.query.as_ref().unwrap().chars().count(),
                    MAX_ADMIN_QUERY_CHARS
                );
            }
            if row.details_restricted {
                assert!(row.query.is_none());
            }
        }
        for row in &snapshot.locks {
            assert!(row.blocked_by.len() <= MAX_ADMIN_BLOCKERS);
        }
        assert!(snapshot
            .pending_transactions
            .iter()
            .all(|row| row.transaction_age_seconds.is_some_and(|age| age > 0)));
        assert!(
            matches!(snapshot.stats.database_size_bytes, AdminMetric::Value(value) if value > 0)
        );
        match snapshot.stats.cache_hit_ratio {
            AdminMetric::Value(value) => assert!(value.is_finite() && (0.0..=1.0).contains(&value)),
            AdminMetric::Null => {}
            other => panic!("fixture cache-hit metric unexpectedly unavailable: {other:?}"),
        }
        // Idle cancellation is reusable; shared owner tests cover in-flight joins.
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend.admin_snapshot(&document).await.unwrap().database,
            "dbunk_demo"
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend.admin_snapshot(&document).await,
            Err(DataError::Document(_))
        ));
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let restored = tokio::time::timeout(Duration::from_secs(10), async {
        while fixture_count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    shutdown.expect("owned administration tasks joined");
    restored.expect("fixture activity returned to baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("administration acceptance deadline");
}
