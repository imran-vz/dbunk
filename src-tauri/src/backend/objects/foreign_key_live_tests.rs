//! Opt-in owned metadata corpus. Setup/cleanup belong to the guarded root probe.
use super::live_tests::fixture_count;
use super::*;
use crate::backend::{
    data::{
        BrowseCountPolicy, BrowseFilter, BrowsePageRequest, BrowseTableDataPayload,
        ComparisonOperator, DataCloseOutcome,
    },
    profile,
};
use futures_util::FutureExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 metadata corpus only; run serially"]
async fn native_foreign_keys_ordered_composite_navigation_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    const SCHEMA: &str = "native_metadata_20261003";
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
            .open_data_document("fk-live", "references", &backend.fixture().id)
            .await
            .unwrap();
        let keys = backend
            .load_foreign_keys(&document, SCHEMA.into(), "child".into())
            .await
            .unwrap();
        assert_eq!(keys.len(), 2);
        let key = keys.iter().find(|key| key.name == "composite_fk").unwrap();
        assert_eq!(key.columns, ["local second", "local first"]);
        assert_eq!(key.referenced_columns, ["second.part", "first part"]);
        assert_eq!(key.referenced_schema, SCHEMA);
        assert_eq!(key.referenced_table, "parent");
        assert_eq!(key.on_update.as_deref(), Some("CASCADE"));
        assert_eq!(key.on_delete.as_deref(), Some("SET NULL"));
        let single = keys.iter().find(|key| key.name == "single_fk").unwrap();
        assert_eq!(single.columns, ["local first"]);
        assert!(backend
            .load_foreign_keys(&document, SCHEMA.into(), "generated_table".into())
            .await
            .unwrap()
            .is_empty());
        assert!(backend
            .load_foreign_keys(&document, SCHEMA.into(), "foreign_metadata".into())
            .await
            .unwrap()
            .is_empty());
        assert!(matches!(
            backend
                .load_foreign_keys(&document, SCHEMA.into(), "missing".into())
                .await,
            Err(DataError::Catalog(CatalogError::ObjectNotFound))
        ));
        let payload = |request, table: &str, filters| BrowseTableDataPayload {
            connection_id: backend.fixture().id.clone(),
            tab_id: "references".into(),
            request_id: request,
            schema: SCHEMA.into(),
            table: table.into(),
            filters,
            sort: vec![],
            page_request: BrowsePageRequest::Offset { page: 1 },
            page_size: 10,
            count_policy: BrowseCountPolicy::None,
            refresh_structure: true,
        };
        let child = backend
            .browse_table(&document, payload(1, "child", vec![]))
            .await
            .unwrap();
        let row = child
            .rows
            .iter()
            .find(|row| row[0].as_deref() == Some("1"))
            .unwrap();
        let filters = key
            .columns
            .iter()
            .zip(&key.referenced_columns)
            .map(|(source, target)| {
                let column = child
                    .columns
                    .iter()
                    .position(|column| &column.name == source)
                    .unwrap();
                BrowseFilter::Comparison {
                    column: target.clone(),
                    operator: ComparisonOperator::Eq,
                    value: row[column].clone().unwrap(),
                }
            })
            .collect();
        let target = backend
            .browse_table(&document, payload(2, "parent", filters))
            .await
            .unwrap();
        assert_eq!(target.rows.len(), 1);
        assert_eq!(target.rows[0][0].as_deref(), Some("O'Reilly\\x雪"));
        assert_eq!(target.rows[0][1].as_deref(), Some("9223372036854775807"));
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend
                .load_foreign_keys(&document, SCHEMA.into(), "child".into())
                .await
                .unwrap(),
            keys
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend
                .load_foreign_keys(&document, SCHEMA.into(), "child".into())
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
    shutdown.expect("owned FK read/browse tasks joined");
    restored.expect("fixture activity returned to baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("FK acceptance deadline");
}
