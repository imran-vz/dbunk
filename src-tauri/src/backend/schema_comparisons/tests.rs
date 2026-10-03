use super::*;
use crate::{
    backend::profile,
    postgres::schema_compare::{
        capture::{
            test_support::{self, TestRelation},
            CapturedValue,
        },
        diff,
        protocol::InventoryEntry,
    },
};
use futures_util::FutureExt;
use std::time::Duration;
fn endpoint(schema: &str) -> Endpoint {
    Endpoint {
        connection_id: profile::CONNECTION_ID.into(),
        schema: schema.into(),
    }
}
async fn backend() -> (tempfile::TempDir, Backend) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    *backend.0.schema_comparisons.test_capture.lock().unwrap() = Some(Arc::new(|ctx| {
        async move {
            fn rows() -> Vec<TestRelation> {
                (0..101)
                    .map(|index| TestRelation {
                        oid: index + 1,
                        entry: InventoryEntry {
                            identity: RelationIdentity {
                                kind: RelationKind::Table,
                                name: format!("table_{index:03}"),
                            },
                            eligibility: Eligibility::Eligible,
                        },
                        fields: vec![(
                            FieldPath::Table {
                                field: TableField::Comment,
                            },
                            CapturedValue::Text(if index == 0 {
                                "日".repeat(30_000)
                            } else {
                                "exact comment".into()
                            }),
                        )],
                    })
                    .collect()
            }
            let source = test_support::fixture(
                &ctx.budget,
                &ctx.request.source.connection_id,
                &ctx.request.source.schema,
                160015,
                rows(),
            );
            let mut target = test_support::fixture(
                &ctx.budget,
                &ctx.request.target.connection_id,
                &ctx.request.target.schema,
                160015,
                rows(),
            );
            test_support::share_snapshot(&source, &mut target);
            diff::compare(
                ctx.identity,
                source,
                target,
                &ctx.control,
                std::time::Instant::now(),
            )
        }
        .boxed()
    }));
    (directory, backend)
}
async fn terminal(backend: &Backend, id: &str) -> Status {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = backend.get_schema_comparison(id).unwrap();
            if matches!(
                row.state,
                StatusState::Completed { .. } | StatusState::Cancelled | StatusState::Failed { .. }
            ) {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn result(backend: &Backend) -> ResultRequest {
    let start = SchemaComparisonStart::new(endpoint("source"), endpoint("target")).unwrap();
    let row = backend.begin_schema_comparison(start.clone()).unwrap();
    assert_eq!(
        backend.begin_schema_comparison(start).unwrap().job_id,
        row.job_id
    );
    let row = terminal(backend, &row.job_id).await;
    let StatusState::Completed { result_id } = row.state else {
        panic!("{row:?}")
    };
    ResultRequest {
        identity: ResultIdentity {
            job_id: row.job_id,
            result_id,
        },
        source: row.source,
        target: row.target,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_pages_and_utf8_chunks_preserve_server_continuations_and_serializer_ownership() {
    let (_dir, backend) = backend().await;
    let request = result(&backend).await;
    let reader = backend
        .open_schema_comparison_reader("window", "tab", request)
        .await
        .unwrap();
    let first = backend
        .read_schema_comparison(&reader, ReadRequest::Objects { offset: 0 })
        .await
        .unwrap();
    let second = backend
        .read_schema_comparison(&reader, ReadRequest::Metadata)
        .await
        .unwrap();
    assert!(matches!(
        backend
            .read_schema_comparison(&reader, ReadRequest::Objects { offset: 100 })
            .await,
        Err(CompareError::Busy)
    ));
    let page = first.into_page().unwrap();
    let CompareReply::Objects {
        next_offset, items, ..
    } = page.reply
    else {
        panic!("objects")
    };
    assert_eq!(items.len(), 100);
    assert_eq!(next_offset, Some(100));
    let last = backend
        .read_schema_comparison(
            &reader,
            ReadRequest::Objects {
                offset: next_offset.unwrap(),
            },
        )
        .await
        .unwrap()
        .into_page()
        .unwrap();
    let CompareReply::Objects {
        items, next_offset, ..
    } = last.reply
    else {
        panic!("objects")
    };
    assert_eq!(items.len(), 1);
    assert_eq!(next_offset, None);
    let metadata = second.into_page().unwrap();
    let CompareReply::Metadata { metadata, .. } = metadata.reply else {
        panic!("metadata")
    };
    assert_eq!(metadata.consistency, SnapshotConsistency::SharedTransaction);
    let fields = backend
        .read_schema_comparison(
            &reader,
            ReadRequest::Fields {
                object: RelationIdentity {
                    kind: RelationKind::Table,
                    name: "table_000".into(),
                },
                offset: 0,
            },
        )
        .await
        .unwrap()
        .into_page()
        .unwrap();
    let CompareReply::Fields { items, .. } = fields.reply else {
        panic!("fields")
    };
    let SummaryDifference::Equal { source, .. } = items[0].difference else {
        panic!("equal")
    };
    let chunk = backend
        .read_schema_comparison(
            &reader,
            ReadRequest::Value {
                value: source,
                offset: 0,
            },
        )
        .await
        .unwrap()
        .into_page()
        .unwrap();
    let CompareReply::Value {
        text,
        next_offset,
        complete,
        ..
    } = chunk.reply
    else {
        panic!("value")
    };
    assert_eq!(text.len(), 65_535);
    assert_eq!(next_offset, 65_535);
    assert!(!complete);
    assert!(text.chars().all(|c| c == '日'));
    let next = backend
        .read_schema_comparison(
            &reader,
            ReadRequest::Value {
                value: source,
                offset: next_offset,
            },
        )
        .await
        .unwrap()
        .into_page()
        .unwrap();
    let CompareReply::Value {
        next_offset,
        complete,
        ..
    } = next.reply
    else {
        panic!("value")
    };
    assert_eq!(next_offset, 90_000);
    assert!(complete);
    backend
        .close_schema_comparison_reader(&reader)
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reader_close_waits_for_queued_reply_and_reused_tab_never_accepts_old_generation() {
    let (_dir, backend) = backend().await;
    let request = result(&backend).await;
    let reader = backend
        .open_schema_comparison_reader("window", "tab", request.clone())
        .await
        .unwrap();
    let queued = backend
        .read_schema_comparison(&reader, ReadRequest::Metadata)
        .await
        .unwrap();
    let close = backend.close_schema_comparison_reader(&reader);
    tokio::pin!(close);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut close)
        .await
        .is_err());
    assert!(backend
        .open_schema_comparison_reader("window", "tab", request.clone())
        .await
        .is_err());
    assert!(matches!(queued.into_page(), Err(CompareError::Unavailable)));
    close.await.unwrap();
    let fresh = backend
        .open_schema_comparison_reader("window", "tab", request)
        .await
        .unwrap();
    assert!(backend
        .read_schema_comparison(&reader, ReadRequest::Metadata)
        .await
        .is_err());
    backend
        .read_schema_comparison(&fresh, ReadRequest::Metadata)
        .await
        .unwrap()
        .into_page()
        .unwrap();
    drop(fresh);
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn both_profile_authorities_precede_capture_and_unrelated_data_retirement_preserves_history()
{
    let (_dir, backend) = backend().await;
    let mut foreign = endpoint("target");
    foreign.connection_id = "outside-profile".into();
    let start = backend
        .begin_schema_comparison(SchemaComparisonStart::new(endpoint("source"), foreign).unwrap())
        .unwrap();
    assert!(matches!(
        terminal(&backend, &start.job_id).await.state,
        StatusState::Failed {
            failure: CompareError::Unavailable
        }
    ));
    let request = result(&backend).await;
    let reader = backend
        .open_schema_comparison_reader("window", "tab", request)
        .await
        .unwrap();
    {
        let _gate = backend.0.development_gate.lock().await;
        crate::backend::data::retire_data(
            &backend.0,
            &backend.0.state,
            Some(profile::CONNECTION_ID),
        )
        .await
        .unwrap();
    }
    backend
        .read_schema_comparison(&reader, ReadRequest::Metadata)
        .await
        .unwrap()
        .into_page()
        .unwrap();
    let queued = backend
        .read_schema_comparison(&reader, ReadRequest::Metadata)
        .await
        .unwrap();
    backend
        .0
        .state
        .pg_schema_compare
        .begin_connection_teardown(profile::CONNECTION_ID)
        .await;
    assert!(queued.into_page().is_err());
    backend
        .0
        .state
        .pg_schema_compare
        .end_connection_teardown(profile::CONNECTION_ID)
        .await;
    backend
        .close_schema_comparison_reader(&reader)
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_reports_unjoined_worker_and_holds_admission_until_real_completion() {
    let (_dir, backend) = backend().await;
    let (release, held) = std::sync::mpsc::channel();
    let held = Arc::new(std::sync::Mutex::new(Some(held)));
    let started = Arc::new(tokio::sync::Notify::new());
    let notice = started.clone();
    *backend.0.schema_comparisons.test_capture.lock().unwrap() = Some(Arc::new(move |_| {
        let held = held.lock().unwrap().take().unwrap();
        let notice = notice.clone();
        async move {
            notice.notify_one();
            held.recv().unwrap();
            Err(CompareError::Cancelled)
        }
        .boxed()
    }));
    let row = backend
        .begin_schema_comparison(
            SchemaComparisonStart::new(endpoint("source"), endpoint("target")).unwrap(),
        )
        .unwrap();
    started.notified().await;
    let now = tokio::time::Instant::now();
    assert!(backend
        .shutdown_with_deadlines(
            now + Duration::from_millis(10),
            now + Duration::from_millis(20)
        )
        .await
        .is_err());
    assert!(!backend.0.schema_comparisons.owner.settled());
    assert_eq!(
        backend.release_schema_comparison(&row.job_id),
        Err(CompareError::Busy)
    );
    release.send(()).unwrap();
    backend
        .0
        .schema_comparisons
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    backend.0.state.pool.close().await;
}
#[test]
fn public_page_bounds_reject_wrong_identity_lengths_and_unearned_continuations() {
    let request = ResultRequest {
        identity: ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        },
        source: endpoint("source"),
        target: endpoint("target"),
    };
    let value = ValueRef {
        side: Side::Source,
        value_id: 0,
        raw_bytes: 3,
        value_kind: ValueKind::Text,
    };
    let mut p = SchemaComparisonPage {
        response_id: "response".into(),
        request,
        read: ReadRequest::Value { value, offset: 0 },
        reply: CompareReply::Value {
            value,
            offset: 0,
            text: "日".into(),
            next_offset: 3,
            complete: true,
        },
    };
    assert!(p.checked_heap_bytes().is_some());
    let CompareReply::Value { next_offset, .. } = &mut p.reply else {
        unreachable!()
    };
    *next_offset = 2;
    assert!(p.checked_heap_bytes().is_none());
    let CompareReply::Value {
        text, next_offset, ..
    } = &mut p.reply
    else {
        unreachable!()
    };
    *next_offset = 3;
    text.reserve(MAX_COMPARISON_PAGE_BYTES);
    assert!(p.checked_heap_bytes().is_none());
    let mut target = endpoint("target");
    target.schema.reserve(MAX_COMPARISON_PAGE_BYTES);
    assert!(SchemaComparisonStart::new(endpoint("source"), target).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn released_result_cannot_free_queued_response_slots_or_revalidate_its_old_reader() {
    let (_dir, backend) = backend().await;
    let request = result(&backend).await;
    let reader = backend
        .open_schema_comparison_reader("window", "old", request.clone())
        .await
        .unwrap();
    let first = backend
        .read_schema_comparison(&reader, ReadRequest::Metadata)
        .await
        .unwrap();
    let second = backend
        .read_schema_comparison(&reader, ReadRequest::Objects { offset: 0 })
        .await
        .unwrap();
    backend
        .release_schema_comparison(&request.identity.job_id)
        .unwrap();
    let next = result(&backend).await;
    let fresh = backend
        .open_schema_comparison_reader("window", "new", next)
        .await
        .unwrap();
    assert!(matches!(
        backend
            .read_schema_comparison(&fresh, ReadRequest::Metadata)
            .await,
        Err(CompareError::Busy)
    ));
    assert!(first.into_page().is_err());
    backend
        .read_schema_comparison(&fresh, ReadRequest::Metadata)
        .await
        .unwrap()
        .into_page()
        .unwrap();
    drop(second);
    backend
        .close_schema_comparison_reader(&reader)
        .await
        .unwrap();
    backend
        .close_schema_comparison_reader(&fresh)
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_mutation_fence_does_not_pass_unjoined_native_comparison_worker() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_dir, backend) = backend().await;
    let (release, held) = std::sync::mpsc::channel();
    let held = Arc::new(std::sync::Mutex::new(Some(held)));
    let started = Arc::new(tokio::sync::Notify::new());
    let notice = started.clone();
    *backend.0.schema_comparisons.test_capture.lock().unwrap() = Some(Arc::new(move |_| {
        let held = held.lock().unwrap().take().unwrap();
        let notice = notice.clone();
        async move {
            notice.notify_one();
            held.recv().unwrap();
            Err(CompareError::Cancelled)
        }
        .boxed()
    }));
    let row = backend
        .begin_schema_comparison(
            SchemaComparisonStart::new(endpoint("source"), endpoint("target")).unwrap(),
        )
        .unwrap();
    started.notified().await;
    let gate = backend.0.development_gate.lock().await;
    let mutated = AtomicBool::new(false);
    let fence = crate::socket_lifecycle::with_connection_fence(
        &backend.0.state,
        profile::CONNECTION_ID,
        async {
            mutated.store(true, Ordering::SeqCst);
        },
    );
    tokio::pin!(fence);
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut fence)
        .await
        .is_err());
    assert!(!mutated.load(Ordering::SeqCst));
    assert_eq!(
        backend.release_schema_comparison(&row.job_id),
        Err(CompareError::Busy)
    );
    release.send(()).unwrap();
    fence.await;
    assert!(mutated.load(Ordering::SeqCst));
    assert!(backend.get_schema_comparison(&row.job_id).is_err());
    drop(gate);
    backend.shutdown().await.unwrap();
}
