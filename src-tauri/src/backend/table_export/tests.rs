use super::*;
use crate::backend::profile;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_export_foreign_closed_and_invalid_documents_refuse_before_credentials() {
    let directory = profile::directory();
    let other_directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let other = Backend::open_fixture(&other_directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let doc = backend
        .open_data_document("export-tests", "table", &backend.fixture().id)
        .await
        .unwrap();
    let request = TableExportRequest {
        schema: "public".into(),
        table: "t".into(),
        expected: None,
    };
    assert!(matches!(
        other.capture_table_export(&doc, request.clone()).await,
        Err(DataError::Document(_))
    ));
    let invalid = TableExportRequest {
        table: String::new(),
        ..request.clone()
    };
    assert!(matches!(
        backend.capture_table_export(&doc, invalid).await,
        Err(DataError::Catalog(
            crate::postgres::native_catalog::CatalogError::InvalidReference
        ))
    ));
    backend.close_data_document(&doc).await.unwrap();
    assert!(matches!(
        backend.capture_table_export(&doc, request).await,
        Err(DataError::Document(_))
    ));
    backend.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}
