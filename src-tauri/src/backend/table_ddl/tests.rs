use super::*;
use crate::backend::profile;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn description() -> TableDdlDescription {
    TableDdlDescription {
        identity: TableIdentity {
            database_oid: 1,
            relation_oid: 2,
        },
        schema_oid: 3,
        schema: "quoted\" schema".into(),
        namespace_xmin: "12".into(),
        namespace_ctid: "(0,1)".into(),
        table: "table".into(),
        column: None,
        comment: None,
    }
}
#[test]
fn exact_preview_quotes_and_preserves_null_comment_and_column_identity() {
    let mut target = description();
    let intent = TableDdlIntent::SetComment {
        comment: Some("x'\\y".into()),
    };
    let rendered = preview::render(&target, &intent, Some(123)).unwrap();
    assert_eq!(
        rendered.sql,
        "COMMENT ON TABLE \"quoted\"\" schema\".\"table\" IS E'x''\\\\y';"
    );
    assert_eq!(rendered.statement_timeout_ms, Some(123));
    assert!(rendered.checked_heap_bytes().unwrap() < MAX_TABLE_DDL_PREVIEW_BYTES);
    target.column = Some(TableDdlColumn {
        attnum: 2,
        name: "odd\" column".into(),
    });
    let rename = preview::render(
        &target,
        &TableDdlIntent::Rename {
            new_name: "next".into(),
        },
        None,
    )
    .unwrap();
    assert_eq!(
        rename.sql,
        "ALTER TABLE \"quoted\"\" schema\".\"table\" RENAME COLUMN \"odd\"\" column\" TO \"next\";"
    );
    let clear =
        preview::render(&target, &TableDdlIntent::SetComment { comment: None }, None).unwrap();
    assert!(clear.sql.ends_with(" IS NULL;"));
    assert!(!format!("{target:?} {intent:?} {rendered:?}").contains("x'"));
}
#[test]
fn bounds_refuse_oversized_backings_and_preserve_serialized_readonly_description() {
    let target = description();
    let restored: TableDdlDescription =
        serde_json::from_str(&serde_json::to_string(&target).unwrap()).unwrap();
    assert_eq!(restored, target);
    assert!(restored.checked_heap_bytes().is_some());
    for name in [String::new(), "é".repeat(32), "x\0".into()] {
        assert!(TableDdlIntent::Rename { new_name: name }
            .checked_heap_bytes()
            .is_none());
    }
    assert!(TableDdlIntent::SetComment {
        comment: Some("a".repeat(4097))
    }
    .checked_heap_bytes()
    .is_none());
    let mut huge = String::with_capacity(1024 * 1024);
    huge.push('x');
    assert!(TableDdlIntent::Rename { new_name: huge }
        .checked_heap_bytes()
        .is_none());
    // Quoted whitespace names are legitimate identifiers, never trimmed.
    assert!(TableDdlIntent::Rename {
        new_name: " ".into()
    }
    .checked_heap_bytes()
    .is_some());
}
async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("ddl", "table", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
fn target(document: &DataDocument) -> TableDdlTarget {
    TableDdlTarget {
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_confirmation_stale_timeout_policy_and_generation_refuse_before_credentials() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, None).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let review = backend
        .review_table_ddl(
            target(&document),
            TableDdlIntent::Rename {
                new_name: "next".into(),
            },
        )
        .await
        .unwrap();
    let attempt = review.attempt_id().clone();
    let observed = calls.clone();
    let result = backend
        .submit(review, false, native_table_ddl::execute, move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { None })
        })
        .await
        .unwrap();
    let TableDdlSubmission::NeedsConfirmation(confirmation) = result else {
        panic!("strict confirmation")
    };
    assert_eq!(confirmation.review().attempt_id(), &attempt);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    policy(&backend, true, None).await;
    assert!(matches!(
        backend.confirm_table_ddl(*confirmation).await,
        Err(TableDdlError::PolicyBlocked)
    ));
    policy(&backend, false, None).await;
    let review = backend
        .review_table_ddl(
            target(&document),
            TableDdlIntent::Rename {
                new_name: "next".into(),
            },
        )
        .await
        .unwrap();
    policy(&backend, false, Some(20)).await;
    let observed = calls.clone();
    assert!(matches!(
        backend
            .submit(review, false, native_table_ddl::execute, move |_, _| {
                observed.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { None })
            })
            .await,
        Err(TableDdlError::InvalidTarget)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let stale = target(&document);
    backend.cancel_data(&document).await.unwrap();
    assert!(matches!(
        backend
            .review_table_ddl(stale, TableDdlIntent::SetComment { comment: None })
            .await,
        Err(TableDdlError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}

#[test]
fn four_fixed_forms_match_legacy_renderer_and_accept_quoted_whitespace() {
    use crate::postgres::object_ddl::{
        self, PgCommentTarget, PgObjectOp, RenameColumnOp, RenameObjectOp, SetCommentOp,
    };
    use crate::postgres::objects::{PgObjectKind, PgObjectRef};
    for column in [
        None,
        Some(TableDdlColumn {
            attnum: 1,
            name: "c\";--".into(),
        }),
    ] {
        let mut target = description();
        target.column = column;
        for intent in [
            TableDdlIntent::SetComment { comment: None },
            TableDdlIntent::SetComment {
                comment: Some("日本語\n'\\;--".into()),
            },
            TableDdlIntent::Rename {
                new_name: "n\";--".into(),
            },
        ] {
            let reference = PgObjectRef {
                kind: PgObjectKind::Table,
                schema: Some(target.schema.clone()),
                name: target.table.clone(),
                identity_args: None,
            };
            let op = match &intent {
                TableDdlIntent::SetComment { comment } => PgObjectOp::SetComment(SetCommentOp {
                    target: match &target.column {
                        Some(c) => PgCommentTarget::Column {
                            schema: target.schema.clone(),
                            table: target.table.clone(),
                            column: c.name.clone(),
                        },
                        None => PgCommentTarget::Object { reference },
                    },
                    comment: comment.clone(),
                }),
                TableDdlIntent::Rename { new_name } => match &target.column {
                    Some(c) => PgObjectOp::RenameColumn(RenameColumnOp {
                        schema: target.schema.clone(),
                        table: target.table.clone(),
                        name: c.name.clone(),
                        new_name: new_name.clone(),
                    }),
                    None => PgObjectOp::RenameObject(RenameObjectOp {
                        reference,
                        new_name: new_name.clone(),
                    }),
                },
            };
            assert_eq!(
                preview::render(&target, &intent, None).unwrap().sql,
                object_ddl::generate_object_ddl(&[op]).unwrap().statements[0].sql
            );
        }
    }
    let mut target = description();
    target.schema = " ".into();
    target.table = "\t".into();
    assert_eq!(
        preview::render(
            &target,
            &TableDdlIntent::Rename {
                new_name: "\n".into()
            },
            None
        )
        .unwrap()
        .sql,
        "ALTER TABLE \" \".\"\t\" RENAME TO \"\n\";"
    );
    assert!(preview::render(
        &target,
        &TableDdlIntent::SetComment {
            comment: Some("bad\0comment".into())
        },
        None
    )
    .is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_submission_waiter_keeps_write_permit_until_driver_join() {
    use tokio::sync::oneshot;
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, None).await;
    let review = backend
        .review_table_ddl(
            target(&document),
            TableDdlIntent::Rename {
                new_name: "next".into(),
            },
        )
        .await
        .unwrap();
    let (started, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let owner = backend.clone();
    let waiter = tokio::spawn(async move {
        owner
            .submit(
                review,
                true,
                move |_spec, drivers, permit, _cancel, _target, _intent, _preview| async move {
                    let _permit = permit;
                    drivers.track_task(tokio::spawn(async move {
                        let _ = released.await;
                    }));
                    started.send(()).unwrap();
                    drivers.drain().await;
                    TableDdlOutcome::RolledBack {
                        reason: TableDdlFailure::Cancelled,
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
