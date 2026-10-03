use super::*;
use crate::backend::profile;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn description() -> SchemaAlterDescription {
    SchemaAlterDescription {
        identity: SchemaIdentity {
            database_oid: 1,
            schema_oid: 2,
        },
        schema: "quoted\" schema".into(),
        namespace_xmin: "12".into(),
        namespace_ctid: "(0,1)".into(),
        comment: None,
    }
}
#[test]
fn schema_alter_exact_preview_quotes_and_distinguishes_null_from_empty_comment() {
    let target = description();
    let intent = SchemaAlterIntent::SetComment {
        comment: Some("x'\\y".into()),
    };
    let rendered = preview::render(&target, &intent, Some(123)).unwrap();
    assert_eq!(
        rendered.sql,
        "COMMENT ON SCHEMA \"quoted\"\" schema\" IS E'x''\\\\y';"
    );
    assert_eq!(
        rendered.summary,
        "Set comment on SCHEMA \"quoted\"\" schema\""
    );
    assert_eq!(rendered.statement_timeout_ms, Some(123));
    assert_eq!(
        rendered.operation_timeout_ms,
        SCHEMA_ALTER_OPERATION_TIMEOUT_MS
    );
    let rename = preview::render(
        &target,
        &SchemaAlterIntent::Rename {
            new_name: "next\"".into(),
        },
        None,
    )
    .unwrap();
    assert_eq!(
        rename.sql,
        "ALTER SCHEMA \"quoted\"\" schema\" RENAME TO \"next\"\"\";"
    );
    let clear = preview::render(
        &target,
        &SchemaAlterIntent::SetComment { comment: None },
        None,
    )
    .unwrap();
    assert_eq!(
        clear.sql,
        "COMMENT ON SCHEMA \"quoted\"\" schema\" IS NULL;"
    );
    let empty = preview::render(
        &target,
        &SchemaAlterIntent::SetComment {
            comment: Some(String::new()),
        },
        None,
    )
    .unwrap();
    assert_eq!(empty.sql, "COMMENT ON SCHEMA \"quoted\"\" schema\" IS E'';");
    assert_ne!(clear, empty);
    // Debug output never leaks names, comments or SQL.
    assert!(!format!("{target:?} {intent:?} {rendered:?}").contains("x'"));
    assert!(!format!("{target:?}").contains("quoted"));
}
#[test]
fn schema_alter_forms_match_legacy_renderer_and_accept_quoted_whitespace() {
    use crate::postgres::object_ddl::{
        self, PgCommentTarget, PgObjectOp, RenameObjectOp, SetCommentOp,
    };
    use crate::postgres::objects::{PgObjectKind, PgObjectRef};
    let mut target = description();
    target.schema = "s\";--".into();
    for intent in [
        SchemaAlterIntent::SetComment { comment: None },
        SchemaAlterIntent::SetComment {
            comment: Some("日本語\n'\\;--".into()),
        },
        SchemaAlterIntent::Rename {
            new_name: "n\";--".into(),
        },
    ] {
        let reference = PgObjectRef {
            kind: PgObjectKind::Schema,
            schema: None,
            name: target.schema.clone(),
            identity_args: None,
        };
        let op = match &intent {
            SchemaAlterIntent::SetComment { comment } => PgObjectOp::SetComment(SetCommentOp {
                target: PgCommentTarget::Object { reference },
                comment: comment.clone(),
            }),
            SchemaAlterIntent::Rename { new_name } => PgObjectOp::RenameObject(RenameObjectOp {
                reference,
                new_name: new_name.clone(),
            }),
        };
        assert_eq!(
            preview::render(&target, &intent, None).unwrap().sql,
            object_ddl::generate_object_ddl(&[op]).unwrap().statements[0].sql
        );
    }
    target.schema = " ".into();
    assert_eq!(
        preview::render(
            &target,
            &SchemaAlterIntent::Rename {
                new_name: "\t".into()
            },
            None
        )
        .unwrap()
        .sql,
        "ALTER SCHEMA \" \" RENAME TO \"\t\";"
    );
}
#[test]
fn schema_alter_bounds_refuse_invalid_identity_names_and_oversized_backings() {
    let target = description();
    let restored: SchemaAlterDescription =
        serde_json::from_str(&serde_json::to_string(&target).unwrap()).unwrap();
    assert_eq!(restored, target);
    assert!(restored.checked_heap_bytes().is_some());
    for defect in 0..6 {
        let mut bad = description();
        match defect {
            0 => bad.identity.schema_oid = 0,
            1 => bad.identity.database_oid = 0,
            2 => bad.schema = "x\0".into(),
            3 => bad.namespace_xmin = "x".into(),
            4 => bad.namespace_ctid = "nope!".into(),
            _ => bad.comment = Some("a".repeat(4097)),
        }
        assert!(bad.checked_heap_bytes().is_none(), "defect {defect}");
        assert!(
            preview::render(&bad, &SchemaAlterIntent::SetComment { comment: None }, None).is_err()
        );
    }
    for name in [String::new(), "é".repeat(32), "x\0".into()] {
        assert!(SchemaAlterIntent::Rename { new_name: name }
            .checked_heap_bytes()
            .is_none());
    }
    let mut huge = String::with_capacity(1024 * 1024);
    huge.push('x');
    assert!(SchemaAlterIntent::Rename { new_name: huge }
        .checked_heap_bytes()
        .is_none());
    assert!(SchemaAlterRequest {
        schema: "s".into(),
        expected: Some(SchemaIdentity {
            database_oid: 1,
            schema_oid: 0
        }),
    }
    .validate()
    .is_err());
    assert!(target.request().validate().is_ok());
    assert_eq!(target.request().expected, Some(target.identity));
}
#[test]
fn schema_alter_restored_preview_matches_only_its_typed_description() {
    let target = description();
    let intent = SchemaAlterIntent::Rename {
        new_name: "next".into(),
    };
    let rendered = preview::render(&target, &intent, Some(5)).unwrap();
    assert!(rendered.matches_typed_description(&target, &intent));
    let mut other = target.clone();
    other.schema = "other".into();
    assert!(!rendered.matches_typed_description(&other, &intent));
    let mut tampered = rendered.clone();
    tampered.sql.push_str(" SELECT 1;");
    assert!(!tampered.matches_typed_description(&target, &intent));
    let mut deadline = rendered;
    deadline.operation_timeout_ms += 1;
    assert!(!deadline.matches_typed_description(&target, &intent));
}
async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("schema-alter", "objects", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
fn target(document: &DataDocument) -> SchemaAlterTarget {
    SchemaAlterTarget {
        document: document.clone(),
        generation: *document.0.read_cancellation().borrow(),
        description: description(),
        statement_timeout_ms: None,
    }
}
pub(super) async fn policy(backend: &Backend, readonly: bool, timeout: Option<u32>) {
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!()
    };
    pg.read_only = readonly;
    pg.safe_mode = crate::SafeMode::Strict;
    pg.driver_options
        .get_or_insert_with(Default::default)
        .statement_timeout_ms = timeout;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
}
fn rename() -> SchemaAlterIntent {
    SchemaAlterIntent::Rename {
        new_name: "next".into(),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schema_alter_policy_confirmation_readonly_timeout_and_generation_refuse_before_credentials(
) {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let review = backend
        .review_schema_alter(target(&document), rename())
        .await
        .unwrap();
    let attempt = review.attempt_id().clone();
    let observed = calls.clone();
    let result = backend
        .submit_schema_alter(review, false, native_schema_alter::execute, move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { None })
        })
        .await
        .unwrap();
    let SchemaAlterSubmission::NeedsConfirmation(confirmation) = result else {
        panic!("strict confirmation")
    };
    assert_eq!(confirmation.review().attempt_id(), &attempt);
    assert_eq!(confirmation.review().intent(), &rename());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    policy(&backend, true, None).await;
    assert!(matches!(
        backend.confirm_schema_alter(*confirmation).await,
        Err(SchemaAlterError::PolicyBlocked)
    ));
    policy(&backend, false, None).await;
    let review = backend
        .review_schema_alter(target(&document), rename())
        .await
        .unwrap();
    // The stored statement timeout is part of the reviewed preview.
    policy(&backend, false, Some(20)).await;
    let observed = calls.clone();
    assert!(matches!(
        backend
            .submit_schema_alter(review, false, native_schema_alter::execute, move |_, _| {
                observed.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { None })
            })
            .await,
        Err(SchemaAlterError::InvalidTarget)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let stale = target(&document);
    backend.cancel_data(&document).await.unwrap();
    assert!(matches!(
        backend
            .review_schema_alter(stale, SchemaAlterIntent::SetComment { comment: None })
            .await,
        Err(SchemaAlterError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schema_alter_confirmed_missing_connection_is_not_dispatched_with_exact_receipt() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, None).await;
    let review = backend
        .review_schema_alter(target(&document), rename())
        .await
        .unwrap();
    let attempt = review.attempt_id().clone();
    let executed = Arc::new(AtomicUsize::new(0));
    let observed = executed.clone();
    let result = backend
        .submit_schema_alter(
            review,
            true,
            move |_spec, _drivers, _permit, _cancel, _target, _intent, _preview| async move {
                observed.fetch_add(1, Ordering::SeqCst);
                SchemaAlterOutcome::Applied { runtime_ms: 0 }
            },
            |_, _| Box::pin(async { None }),
        )
        .await
        .unwrap();
    let SchemaAlterSubmission::Finished(receipt) = result else {
        panic!("confirmed submission finishes")
    };
    assert_eq!(receipt.attempt_id, attempt);
    assert_eq!(receipt.target, description());
    assert_eq!(receipt.intent, rename());
    assert_eq!(receipt.connection_id, backend.fixture().id);
    assert_eq!(
        receipt.outcome,
        SchemaAlterOutcome::NotDispatched {
            reason: SchemaAlterFailure::Connection
        }
    );
    assert_eq!(executed.load(Ordering::SeqCst), 0);
    assert!(receipt.retained_bytes() <= MAX_SCHEMA_ALTER_RECEIPT_BYTES);
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schema_alter_dropped_waiter_keeps_write_permit_until_driver_join() {
    use tokio::sync::oneshot;
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, None).await;
    let review = backend
        .review_schema_alter(target(&document), rename())
        .await
        .unwrap();
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let owner = backend.clone();
    let waiter = tokio::spawn(async move {
        owner
            .submit_schema_alter(
                review,
                true,
                move |_spec, drivers, permit, _cancel, _target, _intent, _preview| async move {
                    let _permit = permit;
                    drivers.track_task(tokio::spawn(async move {
                        let _ = released.await;
                    }));
                    started.send(()).unwrap();
                    drivers.drain().await;
                    SchemaAlterOutcome::RolledBack {
                        reason: SchemaAlterFailure::Cancelled,
                    }
                },
                load,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    waiter.abort();
    let _ = waiter.await;
    assert!(backend.0.documents.begin_write(&document).is_err());
    backend.cancel_data(&document).await.unwrap();
    assert!(backend.0.documents.begin_write(&document).is_err());
    release.send(()).unwrap();
    backend.close_data_document(&document).await.unwrap();
    backend.shutdown().await.unwrap();
}
