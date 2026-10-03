//! Opt-in owned stage03 read. Creates no fixture objects and reads no user rows.
use super::*;
use crate::backend::{data::DataCloseOutcome, objects::CatalogError, profile};
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
async fn native_completion_columns_exact_relation_cancel_retire_and_cleanup() {
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
            .open_data_document("completion-live", "columns", &backend.fixture().id)
            .await
            .unwrap();
        let table = backend
            .completion_columns(&document, "plan026".into(), "fixture_identity".into())
            .await
            .unwrap();
        assert_eq!(table.schema, "plan026");
        assert_eq!(table.relation, "fixture_identity");
        assert_ne!(table.relation_oid, 0);
        assert_eq!(table.kind, CompletionRelationKind::Table);
        assert_eq!(
            table.columns,
            [CompletionColumn {
                name: "instance".into(),
                data_type: "uuid".into(),
                ordinal_position: 1,
                is_primary_key: true
            }]
        );
        let view = backend
            .completion_columns(&document, "plan026".into(), "exact_values".into())
            .await
            .unwrap();
        assert_eq!(view.kind, CompletionRelationKind::View);
        assert_ne!(view.relation_oid, table.relation_oid);
        assert_eq!(
            view.columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            [
                "null_value",
                "empty_value",
                "large_integer",
                "precise_decimal",
                "unicode_value",
                "quoted_value"
            ]
        );
        assert_eq!(view.columns[2].data_type, "bigint");
        assert_eq!(view.columns[3].data_type, "numeric");
        assert!(view.columns.iter().all(|column| !column.is_primary_key));
        assert!(serde_json::to_vec(&view).unwrap().len() <= MAX_COMPLETION_BYTES);
        assert!(matches!(
            backend
                .completion_columns(
                    &document,
                    "plan026".into(),
                    "fixture_identity; SELECT 1".into()
                )
                .await,
            Err(DataError::Catalog(CatalogError::ObjectNotFound))
        ));
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend
                .completion_columns(&document, "plan026".into(), "fixture_identity".into())
                .await
                .unwrap(),
            table
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend
                .completion_columns(&document, "plan026".into(), "fixture_identity".into())
                .await,
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
    shutdown.expect("owned completion tasks joined");
    restored.expect("fixture activity returned to baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("completion acceptance deadline");
}
