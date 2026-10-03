//! Headless facade refusals; none of these requests may open a database socket.
use super::*;
use crate::backend::profile;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn map_invalid_foreign_and_retired_documents_refuse_before_read() {
    let directory = profile::directory();
    let other_directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let other = Backend::open_fixture(&other_directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("window", "schema-map", &backend.fixture().id)
        .await
        .unwrap();
    let invalid = SchemaMapRequest {
        scope: SchemaMapScope::Relation {
            schema: "public".into(),
            table: "".into(),
            expected: None,
        },
        expected_database_oid: None,
    };
    assert!(matches!(
        backend.schema_map(&document, invalid).await,
        Err(DataError::Catalog(
            crate::postgres::native_catalog::CatalogError::InvalidReference
        ))
    ));
    assert!(matches!(
        other
            .schema_map(&document, SchemaMapRequest::default())
            .await,
        Err(DataError::Document(_))
    ));
    backend.close_data_document(&document).await.unwrap();
    assert!(matches!(
        backend
            .schema_map(&document, SchemaMapRequest::default())
            .await,
        Err(DataError::Document(_))
    ));
    other.shutdown().await.unwrap();
    backend.shutdown().await.unwrap();
}
