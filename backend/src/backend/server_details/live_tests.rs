//! Opt-in owned stage03 read-only probe. No database objects or persistent settings changed.
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
            .expect("verify owned fixture");
        assert!(output.status.success(), "owned fixture verification failed");
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
async fn native_server_details_owned_read_cancel_retire_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
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
            .open_data_document("server-details-live", "details", &backend.fixture().id)
            .await
            .unwrap();
        let capture = backend.server_details(&document).await.unwrap();
        assert_eq!(capture.database, "dbunk_demo");
        assert!(capture.reader_pid > 0);
        assert!(capture.checked_heap_bytes().is_some());
        assert!(serde_json::to_vec(&capture).unwrap().len() <= MAX_SERVER_DETAILS_BYTES);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&capture.collected_start).unwrap()
                <= chrono::DateTime::parse_from_rfc3339(&capture.collected_end).unwrap()
        );
        let ServerSection::Loaded(facts) = &capture.facts else {
            panic!("owned facts should be readable")
        };
        assert!(matches!(&facts.server_version, ServerText::Value(value) if !value.is_empty()));
        assert!(matches!(&facts.encoding, ServerText::Value(value) if value == "UTF8"));
        assert!(matches!(&facts.locale, ServerText::Value(value) if !value.is_empty()));
        assert!(
            matches!(&capture.reader.current_user, ServerText::Value(value) if value == "dbunk")
        );
        let ServerSection::Loaded(settings) = &capture.settings else {
            panic!("owned settings should be readable")
        };
        assert!(settings.rows.len() <= MAX_SERVER_SETTINGS);
        for (name, expected) in [
            ("statement_timeout", capture.reader.statement_timeout_ms),
            ("lock_timeout", capture.reader.lock_timeout_ms),
        ] {
            let row = settings
                .rows
                .iter()
                .find(|row| row.name == name)
                .expect("inspection setting");
            assert!(row.inspection_override);
            assert!(
                matches!(&row.setting, ServerText::Value(value) if value == &expected.to_string())
            );
            assert!(matches!(&row.unit, ServerText::Value(value) if value == "ms"));
        }
        let ServerSection::Loaded(extensions) = &capture.extensions else {
            panic!("owned extensions should be readable")
        };
        assert!(extensions.rows.len() <= MAX_SERVER_EXTENSIONS);
        assert!(extensions
            .rows
            .iter()
            .any(|row| row.name == "plpgsql" && row.schema == "pg_catalog"));
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend.server_details(&document).await.unwrap().database,
            "dbunk_demo"
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend.server_details(&document).await,
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
    shutdown.expect("owned server-details tasks joined");
    restored.expect("fixture activity returned to baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("server-details probe deadline");
}
