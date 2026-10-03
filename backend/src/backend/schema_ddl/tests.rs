use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use tokio::sync::oneshot;

#[test]
fn intent_is_bounded_exact_validated_on_restore_and_redacted() {
    let name = "é".repeat(31);
    let input = CreateSchemaIntent::new(name.clone(), Some("private ' \\ comment".into())).unwrap();
    assert_eq!(input.name(), name);
    assert_eq!(input.comment(), Some("private ' \\ comment"));
    let encoded = serde_json::to_string(&input).unwrap();
    assert_eq!(
        serde_json::from_str::<CreateSchemaIntent>(&encoded).unwrap(),
        input
    );
    assert!(!format!("{input:?}").contains("private"));
    for name in [" ".into(), "é".repeat(32), "bad\0name".into()] {
        assert!(CreateSchemaIntent::new(name, None).is_err());
    }
    assert!(CreateSchemaIntent::new("exact".into(), Some("a".repeat(4097))).is_err());
    assert!(serde_json::from_str::<CreateSchemaIntent>(
        r#"{"name":"ok","comment":null,"sql":"DROP SCHEMA other"}"#
    )
    .is_err());
    let mut oversized_backing = String::with_capacity(1024 * 1024);
    oversized_backing.push_str("name");
    let compact = CreateSchemaIntent::new(oversized_backing, None).unwrap();
    assert!(compact.checked_heap_bytes().unwrap() < 256);
    assert_ne!(
        CreateSchemaIntent::new("x".into(), None).unwrap(),
        CreateSchemaIntent::new("x".into(), Some(String::new())).unwrap()
    );
}

#[test]
fn preview_quotes_exact_names_comments_and_admits_bounded_expansion() {
    let intent = CreateSchemaIntent::new("quoted\" schema".into(), Some("x'\\y".into())).unwrap();
    let rendered = preview(&intent).unwrap();
    assert_eq!(
        rendered.statements[0].sql,
        "CREATE SCHEMA \"quoted\"\" schema\";"
    );
    assert_eq!(
        rendered.statements[1].sql,
        "COMMENT ON SCHEMA \"quoted\"\" schema\" IS E'x''\\\\y';"
    );
    assert!(rendered.checked_heap_bytes().unwrap() < MAX_SCHEMA_PREVIEW_BYTES);
    let max = CreateSchemaIntent::new("x".repeat(63), Some("\\".repeat(4096))).unwrap();
    // SQL and JSON representations both fit or refuse, without truncation.
    assert!(matches!(
        preview(&max),
        Err(CreateSchemaError::InvalidPreview)
    ));
    let max_plain = CreateSchemaIntent::new("x".repeat(63), Some("a".repeat(4096))).unwrap();
    assert!(preview(&max_plain).is_ok());
    assert!(!format!("{rendered:?}").contains("quoted"));
}

#[test]
fn attempt_identity_requires_canonical_v4_and_receipt_preserves_exact_intent() {
    let id = CreateSchemaAttemptId::new();
    let restored: CreateSchemaAttemptId =
        serde_json::from_str(&serde_json::to_string(&id).unwrap()).unwrap();
    assert_eq!(id, restored);
    for value in [
        "00000000-0000-0000-0000-000000000000",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
        "not-an-id",
    ] {
        assert!(serde_json::from_value::<CreateSchemaAttemptId>(serde_json::json!(value)).is_err());
    }
    let receipt = CreateSchemaReceipt {
        attempt_id: id,
        connection_id: "connection".into(),
        intent: CreateSchemaIntent::new("exact".into(), Some("secret comment".into())).unwrap(),
        outcome: CreateSchemaOutcome::Applied {
            statements: 2,
            runtime_ms: 1,
        },
    };
    assert!(!format!("{receipt:?}").contains("secret comment"));
    assert_eq!(
        serde_json::to_value(&receipt).unwrap()["intent"]["comment"],
        "secret comment"
    );
}

async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    // Only a temporary SQLite fixture profile. No database endpoint is contacted.
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("schema-test", "one", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
async fn set_policy(backend: &Backend, read_only: bool) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        panic!("PostgreSQL fixture")
    };
    pg.read_only = read_only;
    pg.safe_mode = crate::SafeMode::Strict;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pure_review_policy_confirmation_and_current_readonly_refusal_need_no_socket() {
    let (_directory, backend, document) = fixture().await;
    set_policy(&backend, false).await;
    let review = backend
        .review_create_schema(
            &document,
            CreateSchemaIntent::new("exact".into(), None).unwrap(),
        )
        .await
        .unwrap();
    let attempt = review.attempt_id().clone();
    let confirmation = match backend.apply_create_schema(review).await.unwrap() {
        CreateSchemaSubmission::NeedsConfirmation(value) => value,
        _ => panic!("strict policy must refuse before connection"),
    };
    assert_eq!(confirmation.attempt_id(), &attempt);
    assert!(confirmation.belongs_to(&document));
    set_policy(&backend, true).await;
    assert!(matches!(
        backend.confirm_create_schema(*confirmation).await,
        Err(CreateSchemaError::PolicyBlocked)
    ));
    backend.close_data_document(&document).await.unwrap();
    assert!(matches!(
        backend
            .review_create_schema(
                &document,
                CreateSchemaIntent::new("exact".into(), None).unwrap()
            )
            .await,
        Err(CreateSchemaError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_waiter_retains_write_guard_until_owned_cleanup_finishes() {
    let (_directory, backend, document) = fixture().await;
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let owner = backend.clone();
    let inner = backend.0.clone();
    let target = document.clone();
    let waiter = tokio::spawn(async move {
        owner
            .data_call(&target, move |_, document, admission| async move {
                let _write = inner.documents.begin_write(&document).unwrap();
                drop(admission);
                let drivers = inner.tasks.child();
                drivers.track_task(tokio::spawn(async move {
                    let _ = released.await;
                }));
                started.send(()).unwrap();
                drivers.drain().await;
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    assert!(backend.0.documents.begin_write(&document).is_err());
    waiter.abort();
    let _ = waiter.await;
    backend.cancel_data(&document).await.unwrap();
    assert!(backend.0.documents.begin_write(&document).is_err());
    release.send(()).unwrap();
    assert_eq!(
        backend.close_data_document(&document).await.unwrap(),
        DataCloseOutcome::Closed
    );
    assert!(backend.0.documents.begin_write(&document).is_err());
    backend.shutdown().await.unwrap();
}
