use super::*;
use crate::postgres::schema_compare::{capture::test_support, diff};
use std::sync::atomic::{AtomicUsize, Ordering};
use tauri::webview::PageLoadEvent;

static REQUEST_TIME: std::sync::OnceLock<i64> = std::sync::OnceLock::new();

fn request(id: &str, source: &str, target: &str) -> StartRequest {
    StartRequest {
        request_id: format!(
            "{}:{id}",
            *REQUEST_TIME.get_or_init(|| chrono::Utc::now().timestamp_millis())
        ),
        source: Endpoint {
            connection_id: source.into(),
            schema: "public".into(),
        },
        target: Endpoint {
            connection_id: target.into(),
            schema: "public".into(),
        },
    }
}
async fn empty(ctx: JobContext) -> Result<Comparison, CompareError> {
    let source = test_support::fixture(
        &ctx.budget,
        &ctx.request.source.connection_id,
        &ctx.request.source.schema,
        160015,
        vec![],
    );
    let mut target = test_support::fixture(
        &ctx.budget,
        &ctx.request.target.connection_id,
        &ctx.request.target.schema,
        160015,
        vec![],
    );
    if ctx.request.source.connection_id == ctx.request.target.connection_id {
        test_support::share_snapshot(&source, &mut target);
    }
    diff::compare(ctx.identity, source, target, &ctx.control, Instant::now())
}
async fn finished(manager: &CompareManager, id: &str) -> Status {
    let mut done = manager
        .inner
        .lock()
        .unwrap()
        .jobs
        .iter()
        .find(|e| e.status.job_id == id)
        .unwrap()
        .done
        .clone();
    tokio::time::timeout(Duration::from_secs(5), done.wait_for(|v| *v))
        .await
        .unwrap()
        .unwrap();
    manager.get(id).unwrap()
}
async fn waiting(ctx: JobContext) -> Result<Comparison, CompareError> {
    ctx.control.wait(std::future::pending::<()>()).await?;
    unreachable!()
}

#[test]
fn start_monitor_is_callable_outside_a_tokio_runtime() {
    // setup() invokes this synchronously on the main thread. A bare
    // tokio::spawn here panics and prevents the app from starting.
    CompareManager::new().start_monitor();
}

#[tokio::test]
async fn fence_ledger_is_bounded_and_overflow_fails_closed_globally() {
    let manager = CompareManager::new();
    let ids = (0..MAX_FENCE_SCOPES)
        .map(|index| format!("fence-{index}"))
        .collect::<Vec<_>>();
    for id in &ids {
        manager.begin_connection_teardown(id).await;
    }
    let overflow = "x".repeat(MAX_CONNECTION_ID_BYTES + 1);
    manager.begin_connection_teardown(&overflow).await;

    {
        let state = manager.inner.lock().unwrap();
        assert_eq!(state.connections.len(), MAX_FENCE_SCOPES);
        assert_eq!(state.global.closing, 1);
    }
    assert_eq!(
        manager.start(request("globally-fenced", "free-a", "free-b"), empty),
        Err(CompareError::Unavailable)
    );

    manager.end_connection_teardown(&overflow).await;
    for id in &ids {
        manager.end_connection_teardown(id).await;
    }
    let state = manager.inner.lock().unwrap();
    assert!(state.connections.is_empty());
    assert_eq!(state.global.closing, 0);
}

#[test]
fn maximum_control_ledger_fits_reserved_headroom() {
    use std::mem::size_of;

    const MAX_REQUEST_TEXT: usize = 128 + 2 * (128 + 63);
    const MAX_JOB_TEXT: usize = 36 + MAX_REQUEST_TEXT;
    const MAX_RESPONSE_TEXT: usize = 2 * (128 + 128);
    // Covers the manager Arc/Mutex, budget counters, four watch-channel
    // allocations, and allocator headers not represented by container capacity.
    const RUNTIME_ALLOWANCE: usize = 8 * 1024;

    let mut maximum = request("compact", "source", "target");
    maximum.request_id = "r".repeat(128);
    maximum.source.connection_id = "s".repeat(128);
    maximum.source.schema = "s".repeat(63);
    maximum.target.connection_id = "t".repeat(128);
    maximum.target.schema = "t".repeat(63);
    compact_request(&mut maximum);
    for value in [
        &maximum.request_id,
        &maximum.source.connection_id,
        &maximum.source.schema,
        &maximum.target.connection_id,
        &maximum.target.schema,
    ] {
        assert_eq!(value.capacity(), value.len());
    }

    let bytes = size_of::<State>()
        + MAX_REQUESTS * (size_of::<RequestRecord>() + MAX_REQUEST_TEXT + 36)
        + (MAX_ACTIVE + MAX_TERMINAL) * (size_of::<Entry>() + MAX_JOB_TEXT)
        + MAX_FENCE_SCOPES * (size_of::<(String, Scope)>() + MAX_CONNECTION_ID_BYTES)
        + MAX_RESPONSE_TEXT
        + MAX_TRANSPORTS * (size_of::<DocumentTransport>() + 128 + 36)
        + RUNTIME_ALLOWANCE;
    assert!(bytes <= super::super::budget::CONTROL_BYTES, "{bytes}");
}

#[tokio::test]
async fn admission_reserves_both_endpoints_and_reconciles_without_running_twice() {
    let manager = CompareManager::new();
    let start = request("one", "a", "b");
    let first = manager.start(start.clone(), waiting).unwrap();
    assert_eq!(
        manager
            .start(start, |_| async { panic!("duplicate worker") })
            .unwrap(),
        first
    );
    for pair in [("a", "c"), ("c", "a"), ("b", "b")] {
        assert_eq!(
            manager
                .start(request("collision", pair.0, pair.1), waiting)
                .unwrap_err(),
            CompareError::Busy
        );
    }
    let second = manager.start(request("two", "c", "c"), waiting).unwrap();
    assert_eq!(
        manager
            .start(request("three", "d", "e"), waiting)
            .unwrap_err(),
        CompareError::Busy
    );
    assert_eq!(
        manager
            .start(request("one", "b", "a"), waiting)
            .unwrap_err(),
        CompareError::InvalidRequest
    );
    manager.cancel(&first.job_id).unwrap();
    manager.cancel(&second.job_id).unwrap();
    assert_eq!(
        finished(&manager, &first.job_id).await.state,
        StatusState::Cancelled
    );
    finished(&manager, &second.job_id).await;
    assert_eq!(manager.budget.used(), 0);
}

#[tokio::test]
async fn cancellation_wins_against_success_and_holds_capacity_until_real_join() {
    let manager = CompareManager::new();
    let (begun, began) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let first = manager
        .start(request("one", "a", "b"), move |ctx| async move {
            let result = empty(ctx).await;
            begun.send(()).unwrap();
            released.await.unwrap(); // deliberately ignores cancellation during cleanup
            result
        })
        .unwrap();
    began.await.unwrap();
    assert_eq!(
        manager.cancel(&first.job_id).unwrap().state,
        StatusState::Cancelling
    );
    assert_eq!(manager.release(&first.job_id), Err(CompareError::Busy));
    assert_eq!(
        manager.start(request("next", "b", "c"), empty).unwrap_err(),
        CompareError::Busy
    );
    release.send(()).unwrap();
    assert_eq!(
        finished(&manager, &first.job_id).await.state,
        StatusState::Cancelled
    );
    assert_eq!(manager.budget.used(), 0);
    let next = manager.start(request("next", "b", "c"), empty).unwrap();
    assert!(matches!(
        finished(&manager, &next.job_id).await.state,
        StatusState::Completed { .. }
    ));
}

#[tokio::test]
async fn either_endpoint_and_global_fences_wait_before_resource_invalidation() {
    let (_dir, state) = crate::test_app_state().await;
    for scope in 0..3 {
        let status = state
            .pg_schema_compare
            .start(request(&format!("fence-{scope}"), "a", "b"), waiting)
            .unwrap();
        let invalidation = async {
            assert_eq!(
                state.pg_schema_compare.get(&status.job_id),
                Err(CompareError::Unavailable)
            );
            assert_eq!(state.pg_schema_compare.budget.used(), 0);
            assert_eq!(
                state
                    .pg_schema_compare
                    .start(request("blocked", "a", "b"), empty)
                    .unwrap_err(),
                CompareError::Unavailable
            );
        };
        match scope {
            0 => crate::socket_lifecycle::with_connection_fence(&state, "a", invalidation).await,
            1 => crate::socket_lifecycle::with_connection_fence(&state, "b", invalidation).await,
            _ => crate::socket_lifecycle::with_global_fence(&state, invalidation).await,
        }
    }
    let next = state
        .pg_schema_compare
        .start(request("after", "a", "b"), empty)
        .unwrap();
    finished(&state.pg_schema_compare, &next.job_id).await;
}

#[tokio::test]
async fn terminal_retention_release_and_ttl_never_turn_missing_results_into_empty() {
    let manager = CompareManager::new();
    let mut ids = Vec::new();
    for n in 0..3 {
        let status = manager
            .start(request(&format!("r{n}"), "a", "b"), empty)
            .unwrap();
        finished(&manager, &status.job_id).await;
        ids.push(status.job_id);
    }
    assert_eq!(manager.list().len(), 2);
    assert_eq!(manager.get(&ids[0]), Err(CompareError::Unavailable));
    assert_eq!(
        manager.start(request("r0", "a", "b"), empty).unwrap_err(),
        CompareError::Unavailable
    );
    manager.release(&ids[1]).unwrap();
    assert_eq!(
        manager.start(request("r1", "a", "b"), empty).unwrap_err(),
        CompareError::Unavailable
    );
    prune(
        &mut manager.inner.lock().unwrap(),
        Instant::now() + RESULT_TTL,
    );
    assert!(manager.list().is_empty());
    assert_eq!(manager.budget.used(), 0);
}

#[tokio::test]
async fn pages_bind_endpoints_and_keep_two_leases_until_the_receiving_transport_acknowledges() {
    let manager = CompareManager::new();
    let status = manager.start(request("r", "a", "b"), empty).unwrap();
    let status = finished(&manager, &status.job_id).await;
    let StatusState::Completed { result_id } = status.state else {
        panic!()
    };
    let read = ResultRequest {
        identity: ResultIdentity {
            job_id: status.job_id.clone(),
            result_id,
        },
        source: status.source,
        target: status.target,
    };
    manager.transport_page_load("webview", PageLoadEvent::Started);
    let transport = manager.transport("webview").unwrap();
    let sent = AtomicUsize::new(0);
    for id in ["first", "second"] {
        manager
            .read(
                "webview",
                &transport,
                id,
                &read,
                ReadRequest::Objects { offset: 0 },
                |json| {
                    let page: serde_json::Value = serde_json::from_str(&json).unwrap();
                    assert_eq!(page["responseId"], id);
                    sent.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
    }
    assert_eq!(sent.load(Ordering::SeqCst), 2);
    assert_eq!(
        manager.read(
            "webview",
            &transport,
            "third",
            &read,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Busy)
    );
    assert_eq!(
        manager.acknowledge("other", &transport, "first"),
        Err(CompareError::Unavailable)
    );
    manager.acknowledge("webview", &transport, "first").unwrap();
    let mut wrong = read.clone();
    wrong.target.schema = "wrong".into();
    assert_eq!(
        manager.read(
            "webview",
            &transport,
            "third",
            &wrong,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Unavailable)
    );
    manager
        .read(
            "webview",
            &transport,
            "third",
            &read,
            ReadRequest::Metadata,
            |_| {},
        )
        .unwrap();
    manager.release(&status.job_id).unwrap();
    assert_eq!(
        manager.budget.used(),
        2 * super::super::budget::SERIALIZER_SCRATCH
    );
    manager.transport_destroyed("webview");
    assert_eq!(manager.budget.used(), 0);
    assert_eq!(
        manager.read(
            "webview",
            &transport,
            "fourth",
            &read,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Unavailable)
    );
}

#[tokio::test]
async fn reload_reclaims_abandoned_replies_and_rejects_late_document_work() {
    let manager = CompareManager::new();
    let status = manager.start(request("reload", "a", "b"), empty).unwrap();
    let status = finished(&manager, &status.job_id).await;
    let StatusState::Completed { result_id } = status.state else {
        panic!()
    };
    let request = ResultRequest {
        identity: ResultIdentity {
            job_id: status.job_id,
            result_id,
        },
        source: status.source,
        target: status.target,
    };
    manager.transport_page_load("webview", PageLoadEvent::Started);
    let old = manager.transport("webview").unwrap();
    let baseline = manager.budget.used();
    for id in ["one", "two"] {
        manager
            .read("webview", &old, id, &request, ReadRequest::Metadata, |_| {})
            .unwrap();
    }
    assert_eq!(
        manager.budget.used(),
        baseline + 2 * super::super::budget::SERIALIZER_SCRATCH
    );

    // A failed/cancelled provisional load can finish without a committed
    // replacement. Unmatched or duplicate completions cannot free live replies.
    for _ in 0..2 {
        manager.transport_page_load("webview", PageLoadEvent::Finished);
        assert_eq!(manager.transport("webview").unwrap(), old);
        assert_eq!(
            manager.budget.used(),
            baseline + 2 * super::super::budget::SERIALIZER_SCRATCH
        );
        assert_eq!(
            manager.read(
                "webview",
                &old,
                "third",
                &request,
                ReadRequest::Metadata,
                |_| panic!()
            ),
            Err(CompareError::Busy)
        );
    }
    manager.acknowledge("webview", &old, "one").unwrap();
    assert_eq!(
        manager.budget.used(),
        baseline + super::super::budget::SERIALIZER_SCRATCH
    );
    manager
        .read(
            "webview",
            &old,
            "one",
            &request,
            ReadRequest::Metadata,
            |_| {},
        )
        .unwrap();

    // Started is the desktop document-commit callback, not a provisional start.
    manager.transport_page_load("webview", PageLoadEvent::Started);
    let new = manager.transport("webview").unwrap();
    assert_ne!(new, old);
    assert_eq!(manager.budget.used(), baseline);
    assert_eq!(
        manager.read(
            "webview",
            &old,
            "late",
            &request,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Unavailable)
    );
    // Reusing a response ID cannot let an old document acknowledge a new lease.
    manager
        .read(
            "webview",
            &new,
            "one",
            &request,
            ReadRequest::Metadata,
            |_| {},
        )
        .unwrap();
    // A delayed completion from either load must also preserve the new reply.
    manager.transport_page_load("webview", PageLoadEvent::Finished);
    assert_eq!(manager.transport("webview").unwrap(), new);
    assert_eq!(
        manager.budget.used(),
        baseline + super::super::budget::SERIALIZER_SCRATCH
    );
    assert_eq!(
        manager.acknowledge("webview", &old, "one"),
        Err(CompareError::Unavailable)
    );
    manager.acknowledge("webview", &new, "one").unwrap();
    assert_eq!(manager.budget.used(), baseline);

    manager.transport_destroyed("webview");
    assert_eq!(manager.transport("webview"), Err(CompareError::Unavailable));
    assert_eq!(
        manager.read(
            "webview",
            &new,
            "late",
            &request,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Unavailable)
    );
    manager.transport_page_load("webview", PageLoadEvent::Finished);
    assert_eq!(manager.transport("webview"), Err(CompareError::Unavailable));
    manager.transport_page_load("webview", PageLoadEvent::Started);
    assert_ne!(manager.transport("webview").unwrap(), new);
}

#[test]
fn document_transport_registry_is_bounded_and_reclaims_destroyed_windows() {
    let manager = CompareManager::new();
    for index in 0..MAX_TRANSPORTS {
        manager.transport_page_load(&format!("window-{index}"), PageLoadEvent::Started);
    }
    manager.transport_page_load("extra", PageLoadEvent::Started);
    assert_eq!(manager.transport("extra"), Err(CompareError::Unavailable));
    manager.transport_destroyed("window-0");
    manager.transport_page_load("extra", PageLoadEvent::Started);
    assert!(manager.transport("extra").is_ok());
    assert_eq!(
        manager.inner.lock().unwrap().transports.len(),
        MAX_TRANSPORTS
    );
}

#[tokio::test]
async fn deadline_includes_resolution_and_grace_expiry_does_not_release_a_running_worker() {
    let manager = CompareManager::new();
    let (begun, began) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let status = manager
        .start_with_timing(
            request("deadline", "a", "b"),
            move |_| async move {
                begun.send(()).unwrap();
                released.await.unwrap(); // an uninterruptible OS resolver, with a real join
                Err(CompareError::Unavailable)
            },
            Duration::from_millis(30),
            Duration::from_millis(5),
        )
        .unwrap();
    began.await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(
        manager.get(&status.job_id).unwrap().state,
        StatusState::Cancelling
    );
    assert_eq!(
        manager.start(request("busy", "a", "c"), empty).unwrap_err(),
        CompareError::Busy
    );
    assert!(manager.budget.used() > 0);
    release.send(()).unwrap();
    assert_eq!(
        finished(&manager, &status.job_id).await.state,
        StatusState::Failed {
            failure: CompareError::DeadlineExceeded
        }
    );
    assert_eq!(manager.budget.used(), 0);
}

#[tokio::test]
async fn expired_start_ids_cannot_recreate_evicted_jobs_and_request_history_is_bounded() {
    let manager = CompareManager::new();
    let mut old = request("old", "a", "b");
    old.request_id = format!("{}:old", chrono::Utc::now().timestamp_millis() - 600_000);
    assert_eq!(
        manager.start(old, empty).unwrap_err(),
        CompareError::Unavailable
    );
    for n in 0..MAX_REQUESTS {
        let status = manager
            .start(request(&format!("history-{n}"), "a", "b"), |_| async {
                Err(CompareError::Unavailable)
            })
            .unwrap();
        finished(&manager, &status.job_id).await;
        manager.release(&status.job_id).unwrap();
    }
    assert_eq!(
        manager.start(request("full", "a", "b"), empty).unwrap_err(),
        CompareError::Busy
    );
    assert!(manager.list().is_empty());
}

#[tokio::test]
async fn worker_panic_is_failed_and_releases_only_after_join() {
    let manager = CompareManager::new();
    let status = manager
        .start(request("panic", "a", "b"), |_| async {
            panic!("injected worker failure")
        })
        .unwrap();
    assert_eq!(
        finished(&manager, &status.job_id).await.state,
        StatusState::Failed {
            failure: CompareError::Unavailable
        }
    );
    assert_eq!(manager.budget.used(), 0);
}

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixture only"]
async fn native_manager_capture_pages_and_connection_edit_invalidation() {
    use crate::postgres::{connect_spec::ResolvedPostgresConnectSpec, dedicated};
    let (_dir, state) = crate::test_app_state().await;
    let crate::StoredConnection::PostgreSQL(mut pg) =
        crate::commands::pg_objects::tests::connection(
            "manager-native",
            crate::SafeMode::Strict,
            false,
        )
    else {
        panic!()
    };
    pg.port = std::env::var("DBUNK_SCHEMA_COMPARE_TEST_PORT")
        .expect("use native.py")
        .parse()
        .unwrap();
    pg.user = "postgres".into();
    pg.password.clear();
    pg.database = "schema_compare_native".into();
    pg.ssl = false;
    let admin = dedicated::connect(
        &ResolvedPostgresConnectSpec::from_postgres(&pg),
        dedicated::NoticeSink::Ignore,
    )
    .await
    .unwrap();
    admin.client.batch_execute("CREATE SCHEMA manager_source; CREATE SCHEMA manager_target; CREATE TABLE manager_source.t(id integer DEFAULT 1); CREATE TABLE manager_target.t(id integer DEFAULT 2)").await.unwrap();
    crate::storage::upsert_connection(
        &state.pool,
        &crate::StoredConnection::PostgreSQL(pg.clone()),
    )
    .await
    .unwrap();
    let mut payload = request("native", &pg.id, &pg.id);
    payload.source.schema = "manager_source".into();
    payload.target.schema = "manager_target".into();
    let status = state
        .pg_schema_compare
        .start_native(payload, state.pool.clone())
        .unwrap();
    let status = finished(&state.pg_schema_compare, &status.job_id).await;
    assert_eq!(status.source_objects, 1);
    assert_eq!(status.target_objects, 1);
    let StatusState::Completed { result_id } = status.state else {
        panic!("{:?}", status.state)
    };
    let request = ResultRequest {
        identity: ResultIdentity {
            job_id: status.job_id.clone(),
            result_id,
        },
        source: status.source,
        target: status.target,
    };
    state
        .pg_schema_compare
        .transport_page_load("native", PageLoadEvent::Started);
    let transport = state.pg_schema_compare.transport("native").unwrap();
    state
        .pg_schema_compare
        .read(
            "native",
            &transport,
            "metadata",
            &request,
            ReadRequest::Metadata,
            |json| {
                let page: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(page["detail"]["kind"], "changed");
                assert_eq!(
                    page["detail"]["metadata"]["consistency"],
                    "sharedTransaction"
                );
            },
        )
        .unwrap();
    state
        .pg_schema_compare
        .acknowledge("native", &transport, "metadata")
        .unwrap();
    let mut fields = None;
    state
        .pg_schema_compare
        .read(
            "native",
            &transport,
            "fields",
            &request,
            ReadRequest::Fields {
                object: RelationIdentity {
                    kind: RelationKind::Table,
                    name: "t".into(),
                },
                offset: 0,
            },
            |json| fields = Some(json),
        )
        .unwrap();
    let fields: serde_json::Value = serde_json::from_str(&fields.unwrap()).unwrap();
    assert!(fields["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"]["field"] == "default" && f["kind"] == "changed"));
    // Real connection edit uses the canonical fence before changing local settings.
    pg.name = "edited fixture".into();
    crate::commands::connections::save_connection_inner(
        &state,
        crate::StoredConnection::PostgreSQL(pg),
    )
    .await
    .unwrap();
    assert_eq!(
        state.pg_schema_compare.get(&status.job_id),
        Err(CompareError::Unavailable)
    );
    assert_eq!(
        state.pg_schema_compare.read(
            "native",
            &transport,
            "stale",
            &request,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Unavailable)
    );
    // Invalidating the result cannot discard a response already in transport.
    assert_eq!(
        state.pg_schema_compare.budget.used(),
        super::super::budget::SERIALIZER_SCRATCH
    );
    state
        .pg_schema_compare
        .acknowledge("native", &transport, "fields")
        .unwrap();
    assert_eq!(state.pg_schema_compare.budget.used(), 0);
    admin.close().await;
}
