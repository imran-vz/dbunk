//! Plan 021 Step 6: failure and boundedness validation through the real
//! manager, resolver and native reader. Pure tests use local sockets only.
//! `native_*` tests require the owned disposable containers created by
//! `infrastructure/test-db/schema-compare/native.py`; they never accept a DSN.
use super::{
    tests::{empty, request},
    *,
};
use crate::host::DocumentLoad;
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec,
    dedicated,
    schema_compare::budget::{
        CONTROL_BYTES, GLOBAL_BYTES, MAX_VALUES, RESULT_BYTES, SERIALIZER_SCRATCH, TABLE_ENTRIES,
    },
    tls::ResolvedTls,
};
use crate::{AppState, StoredConnection};
use crate::{BastionAuthMethod, BastionServer, PgStoredConnection, PgTlsMode, PgTlsOptions};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::AsyncReadExt;

const PRIMARY_PORT: &str = "DBUNK_SCHEMA_COMPARE_TEST_PORT";
const MINOR_PORT: &str = "DBUNK_SCHEMA_COMPARE_MINOR_PORT";
const OTHER_MAJOR_PORT: &str = "DBUNK_SCHEMA_COMPARE_OTHER_PORT";
const TLS_PORT: &str = "DBUNK_SCHEMA_COMPARE_TLS_PORT";
const TLS_CA: &str = "DBUNK_SCHEMA_COMPARE_TLS_CA";
const FIXTURE_DATABASE: &str = "schema_compare_native";
const WINDOW: &str = "validation";

fn env_port(name: &str) -> u16 {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name}: use infrastructure/test-db/schema-compare/native.py"))
        .parse()
        .expect("fixture port")
}

fn stored(id: &str, port: u16, database: &str) -> PgStoredConnection {
    let StoredConnection::PostgreSQL(mut pg) =
        crate::app::test_postgres_connection(id, crate::SafeMode::Strict, false)
    else {
        panic!("fixture connection is PostgreSQL")
    };
    pg.port = port;
    pg.user = "postgres".into();
    pg.password.clear();
    pg.database = database.into();
    pg.ssl = false;
    pg.tls_options = None;
    pg
}

async fn store(state: &AppState, pg: &PgStoredConnection) {
    crate::storage::upsert_connection(&state.pool, &StoredConnection::PostgreSQL(pg.clone()))
        .await
        .unwrap();
}

async fn admin(pg: &PgStoredConnection) -> dedicated::DedicatedConnection {
    dedicated::connect(
        &ResolvedPostgresConnectSpec::from_postgres(pg),
        dedicated::NoticeSink::Ignore,
    )
    .await
    .unwrap()
}

fn fresh(
    source: &PgStoredConnection,
    from: &str,
    target: &PgStoredConnection,
    to: &str,
) -> StartRequest {
    StartRequest {
        request_id: format!(
            "{}:{}",
            chrono::Utc::now().timestamp_millis(),
            uuid::Uuid::new_v4()
        ),
        source: Endpoint {
            connection_id: source.id.clone(),
            schema: from.into(),
        },
        target: Endpoint {
            connection_id: target.id.clone(),
            schema: to.into(),
        },
    }
}

fn done_signal(manager: &CompareManager, id: &str) -> watch::Receiver<bool> {
    manager
        .inner
        .lock()
        .unwrap()
        .jobs
        .iter()
        .find(|e| e.status.job_id == id)
        .expect("job is retained")
        .done
        .clone()
}

async fn finished_within(manager: &CompareManager, id: &str, limit: Duration) -> Status {
    let mut done = done_signal(manager, id);
    tokio::time::timeout(limit, done.wait_for(|v| *v))
        .await
        .expect("job terminated within its bound")
        .unwrap();
    manager.get(id).unwrap()
}

async fn run(state: &AppState, request: StartRequest, limit: Duration) -> Status {
    let status = state
        .pg_schema_compare
        .start_native(request, state.pool.clone())
        .unwrap();
    finished_within(&state.pg_schema_compare, &status.job_id, limit).await
}

/// Completed results retain their reservation until release or expiry.
fn release_all(manager: &CompareManager) {
    for status in manager.list() {
        manager.release(&status.job_id).unwrap();
    }
    assert_eq!(manager.budget.used(), 0);
}

fn result_request(status: &Status) -> ResultRequest {
    let StatusState::Completed { result_id } = &status.state else {
        panic!("not completed: {:?}", status.state)
    };
    ResultRequest {
        identity: ResultIdentity {
            job_id: status.job_id.clone(),
            result_id: result_id.clone(),
        },
        source: status.source.clone(),
        target: status.target.clone(),
    }
}

/// Reads through the same window-token, serializer-lease and acknowledgement
/// path the native commands use.
struct Reader<'a> {
    manager: &'a CompareManager,
    transport: String,
}

impl<'a> Reader<'a> {
    fn new(manager: &'a CompareManager) -> Self {
        manager.transport_document_load(WINDOW, DocumentLoad::Started);
        Self {
            manager,
            transport: manager.transport(WINDOW).unwrap(),
        }
    }

    fn read(&self, request: &ResultRequest, read: ReadRequest) -> Value {
        let response_id = uuid::Uuid::new_v4().to_string();
        let mut body = None;
        self.manager
            .read(
                WINDOW,
                &self.transport,
                &response_id,
                request,
                read,
                |json| body = Some(json),
            )
            .unwrap();
        let page: Value = serde_json::from_str(&body.expect("page body")).unwrap();
        assert_eq!(page["responseId"], response_id.as_str());
        self.manager
            .acknowledge(WINDOW, &self.transport, &response_id)
            .unwrap();
        page
    }

    fn metadata(&self, request: &ResultRequest) -> Value {
        self.read(request, ReadRequest::Metadata)["detail"].clone()
    }

    fn objects(&self, request: &ResultRequest) -> Vec<Value> {
        let mut items = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.read(request, ReadRequest::Objects { offset });
            items.extend(page["items"].as_array().unwrap().iter().cloned());
            match page["nextOffset"].as_u64() {
                Some(next) => offset = next as u32,
                None => return items,
            }
        }
    }

    fn fields(&self, request: &ResultRequest, table: &str) -> Vec<Value> {
        let mut items = Vec::new();
        let mut offset = 0;
        loop {
            let page = self.read(
                request,
                ReadRequest::Fields {
                    object: RelationIdentity {
                        kind: RelationKind::Table,
                        name: table.into(),
                    },
                    offset,
                },
            );
            items.extend(page["items"].as_array().unwrap().iter().cloned());
            match page["nextOffset"].as_u64() {
                Some(next) => offset = next as u32,
                None => return items,
            }
        }
    }

    fn text(&self, request: &ResultRequest, value: &Value) -> String {
        let value: super::super::values::ValueRef = serde_json::from_value(value.clone()).unwrap();
        let mut text = String::new();
        let mut offset = 0;
        loop {
            let chunk = self.read(request, ReadRequest::Value { value, offset });
            text.push_str(chunk["text"].as_str().unwrap());
            if chunk["complete"] == true {
                return text;
            }
            offset = chunk["nextOffset"].as_u64().unwrap() as u32;
        }
    }
}

fn field(items: &[Value], predicate: impl Fn(&Value) -> bool) -> &Value {
    items
        .iter()
        .find(|item| predicate(&item["path"]))
        .unwrap_or_else(|| panic!("missing field among {}", items.len()))
}

fn column<'v>(items: &'v [Value], name: &str, field_name: &str) -> &'v Value {
    field(items, |p| {
        p["kind"] == "column" && p["name"] == name && p["field"] == field_name
    })
}

async fn wait_for_lock(client: &tokio_postgres::Client, table: &str) -> i32 {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = client
                .query_opt(
                    "SELECT pid FROM pg_locks WHERE relation = $1::text::regclass AND NOT granted AND mode = 'AccessShareLock'",
                    &[&table],
                )
                .await
                .unwrap();
            if let Some(row) = row {
                return row.get::<_, i32>(0);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("reader reached its pre-snapshot lock wait")
}

fn max_rss_bytes() -> usize {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    let raw = usage.ru_maxrss as usize;
    if cfg!(target_os = "macos") {
        raw
    } else {
        raw * 1024
    }
}

// ---------------------------------------------------------------------------
// Pure tests: resolution-phase cancellation through the real runner
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cancellation_interrupts_storage_resolution_before_any_connection_attempt() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_dir, state) = crate::test_app_state().await;
    let pg = stored("pool-wait", port, FIXTURE_DATABASE);
    store(&state, &pg).await;
    // Every local storage connection is held, so credential/connection reads
    // block in pool acquisition rather than proceeding to the endpoint.
    let mut held = Vec::new();
    for _ in 0..5 {
        held.push(state.pool.acquire().await.unwrap());
    }
    let status = state
        .pg_schema_compare
        .start_native(fresh(&pg, "a", &pg, "b"), state.pool.clone())
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        state.pg_schema_compare.get(&status.job_id).unwrap().state,
        StatusState::Resolving
    );
    state.pg_schema_compare.cancel(&status.job_id).unwrap();
    let status = finished_within(
        &state.pg_schema_compare,
        &status.job_id,
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(status.state, StatusState::Cancelled);
    assert_eq!(state.pg_schema_compare.budget.used(), 0);
    drop(held);
    // No socket was ever opened towards the endpoint.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_and_deadline_interrupt_a_silent_handshake_without_a_connect_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_dir, state) = crate::test_app_state().await;
    let pg = stored("silent-server", port, FIXTURE_DATABASE);
    assert!(pg.driver_options.is_none(), "no configured connect timeout");
    store(&state, &pg).await;

    let manager = state.pg_schema_compare.clone();
    let status = manager
        .start_native(fresh(&pg, "a", &pg, "b"), state.pool.clone())
        .unwrap();
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("handshake reached the endpoint")
        .unwrap();
    // The startup message has been sent; the server stays silent.
    let mut buffer = [0u8; 256];
    assert!(socket.read(&mut buffer).await.unwrap() > 0);
    manager.cancel(&status.job_id).unwrap();
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(2)).await;
    assert_eq!(status.state, StatusState::Cancelled);
    assert_eq!(manager.budget.used(), 0);
    // Cancellation closed the socket instead of leaking a pending handshake.
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if socket.read(&mut buffer).await.unwrap() == 0 {
                break;
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "driver socket must be closed after cancellation"
    );

    // An absent configured connect timeout never disables the job deadline,
    // which starts at admission and covers resolution and the handshake.
    let pool = state.pool.clone();
    let status = manager
        .start_with_timing(
            fresh(&pg, "a", &pg, "b"),
            move |ctx| runner::run(ctx, pool),
            Duration::from_millis(400),
            Duration::from_millis(100),
        )
        .unwrap();
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("second handshake reached the endpoint")
        .unwrap();
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(3)).await;
    assert_eq!(
        status.state,
        StatusState::Failed {
            failure: CompareError::DeadlineExceeded
        }
    );
    assert_eq!(manager.budget.used(), 0);
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if socket.read(&mut buffer).await.unwrap() == 0 {
                break;
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "driver socket must be closed after deadline expiry"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn cancellation_during_ssh_setup_holds_admission_until_the_blocked_worker_joins() {
    let bastion = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = bastion.local_addr().unwrap().port();
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = bastion.accept() {
            let _ = accepted_tx.send(stream);
        }
    });
    let (_dir, state) = crate::test_app_state().await;
    let now = chrono::Utc::now().to_rfc3339();
    crate::storage::bastions::upsert_bastion_server(
        &state.pool,
        &BastionServer {
            id: "silent-bastion".into(),
            name: "silent bastion".into(),
            host: "127.0.0.1".into(),
            port,
            user: "dbunk".into(),
            auth_method: BastionAuthMethod::Password,
            private_key_path: None,
            host_key_fingerprint: None,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .unwrap();
    let mut pg = stored("ssh-endpoint", 5432, FIXTURE_DATABASE);
    pg.ssh_tunnel.enabled = true;
    pg.ssh_tunnel.bastion_server_id = Some("silent-bastion".into());
    store(&state, &pg).await;

    let manager = state.pg_schema_compare.clone();
    let status = manager
        .start_native(fresh(&pg, "a", &pg, "b"), state.pool.clone())
        .unwrap();
    // The SSH handshake is a blocking OS call on the setup worker. It has
    // reached the bastion socket and now waits for a banner that never comes.
    let stream = tokio::time::timeout(Duration::from_secs(5), accepted_rx)
        .await
        .expect("SSH setup reached the bastion")
        .unwrap();
    assert_eq!(
        manager.cancel(&status.job_id).unwrap().state,
        StatusState::Cancelling
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Grace has not expired, and even after it does the blocked worker keeps
    // its admission: another job on this connection stays busy.
    assert_eq!(
        manager.get(&status.job_id).unwrap().state,
        StatusState::Cancelling
    );
    assert_eq!(
        manager
            .start(request("collide", "ssh-endpoint", "other"), empty)
            .unwrap_err(),
        CompareError::Busy
    );
    assert!(manager.budget.used() > 0);
    // Closing the bastion side lets the blocked handshake fail and join.
    drop(stream);
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(5)).await;
    assert_eq!(status.state, StatusState::Cancelled);
    assert_eq!(manager.budget.used(), 0);
    assert!(manager
        .start(request("after", "ssh-endpoint", "other"), empty)
        .is_ok());
}

// ---------------------------------------------------------------------------
// Native tests: owned disposable fixtures only
// ---------------------------------------------------------------------------

const CROSS_SCHEMA: &str = r#"
CREATE SCHEMA crossdb;
CREATE TABLE crossdb.orders (
    id integer PRIMARY KEY,
    quantity integer DEFAULT 2 NOT NULL,
    label text DEFAULT 'source.literal ''quoted''',
    CONSTRAINT positive CHECK (quantity > 0)
);
CREATE INDEX cross_expression ON crossdb.orders ((quantity + 1)) INCLUDE (label) WHERE quantity > 0;
COMMENT ON COLUMN crossdb.orders.quantity IS 'quantity comment';
"#;

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixtures only"]
async fn native_independent_databases_second_minor_and_unsupported_major() {
    let primary = env_port(PRIMARY_PORT);
    let minor = env_port(MINOR_PORT);
    let other = env_port(OTHER_MAJOR_PORT);
    let (_dir, state) = crate::test_app_state().await;
    let a = stored("cross-a", primary, FIXTURE_DATABASE);
    let b = stored("cross-b", primary, "compare_b");
    let c = stored("cross-minor", minor, FIXTURE_DATABASE);
    let d = stored("cross-other", other, FIXTURE_DATABASE);
    {
        let admin = admin(&a).await;
        admin
            .client
            .batch_execute("CREATE DATABASE compare_b")
            .await
            .unwrap();
        admin.client.batch_execute(CROSS_SCHEMA).await.unwrap();
        admin
            .client
            .batch_execute(
                "CREATE ROLE cross_restricted LOGIN; GRANT USAGE ON SCHEMA crossdb TO cross_restricted",
            )
            .await
            .unwrap();
        admin.close().await;
    }
    let mut versions = Vec::new();
    for pg in [&b, &c, &d] {
        let admin = admin(pg).await;
        admin.client.batch_execute(CROSS_SCHEMA).await.unwrap();
        let row = admin
            .client
            .query_one(
                "SELECT current_setting('server_version_num')::integer, version()",
                &[],
            )
            .await
            .unwrap();
        versions.push((row.get::<_, i32>(0), row.get::<_, String>(1)));
        admin.close().await;
    }
    println!("independent endpoints: {versions:?}");
    for pg in [&a, &b, &c, &d] {
        store(&state, pg).await;
    }
    let reader = Reader::new(&state.pg_schema_compare);

    // Same server version, different databases: independent transactions,
    // supported expressions remain comparable and the result is equal.
    let status = run(
        &state,
        fresh(&a, "crossdb", &b, "crossdb"),
        Duration::from_secs(60),
    )
    .await;
    let request = result_request(&status);
    let metadata = reader.metadata(&request);
    assert_eq!(metadata["kind"], "equal");
    assert_eq!(
        metadata["metadata"]["consistency"],
        "independentTransactions"
    );
    assert_eq!(
        metadata["metadata"]["source"]["serverVersionNum"],
        metadata["metadata"]["target"]["serverVersionNum"]
    );
    assert_ne!(
        metadata["metadata"]["source"]["capturedAt"],
        metadata["metadata"]["target"]["capturedAt"]
    );
    let fields = reader.fields(&request, "orders");
    assert_eq!(column(&fields, "quantity", "default")["kind"], "equal");
    assert_eq!(column(&fields, "label", "default")["kind"], "equal");
    assert_eq!(
        reader.text(&request, &column(&fields, "label", "default")["target"]),
        "'source.literal ''quoted'''::text"
    );

    // A second PG16 minor: structured facts compare, rendered expressions are
    // explicitly incomparable for that pair, and nothing is reported changed.
    let status = run(
        &state,
        fresh(&a, "crossdb", &c, "crossdb"),
        Duration::from_secs(60),
    )
    .await;
    let request = result_request(&status);
    let metadata = reader.metadata(&request);
    assert_eq!(metadata["kind"], "notComparable");
    let (source_version, target_version) = (
        metadata["metadata"]["source"]["serverVersionNum"]
            .as_u64()
            .unwrap(),
        metadata["metadata"]["target"]["serverVersionNum"]
            .as_u64()
            .unwrap(),
    );
    assert_ne!(source_version, target_version);
    assert_eq!(source_version / 10_000, 16);
    assert_eq!(target_version / 10_000, 16);
    let objects = reader.objects(&request);
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["kind"], "notComparable");
    assert_eq!(objects[0]["changedFields"], 0);
    let fields = reader.fields(&request, "orders");
    for rendered in [
        column(&fields, "quantity", "default"),
        column(&fields, "label", "default"),
        field(&fields, |p| {
            p["kind"] == "constraint" && p["field"] == "expression" && p["name"] == "positive"
        }),
        field(&fields, |p| {
            p["kind"] == "index" && p["field"] == "predicate" && p["name"] == "cross_expression"
        }),
        field(&fields, |p| {
            p["kind"] == "indexKey" && p["field"] == "expression" && p["name"] == "cross_expression"
        }),
    ] {
        assert_eq!(rendered["kind"], "notComparable", "{}", rendered["path"]);
        assert_eq!(rendered["reason"], "renderingVersionDifference");
        assert_eq!(
            reader.text(&request, &rendered["source"]),
            reader.text(&request, &rendered["target"])
        );
    }
    for structured in [
        column(&fields, "quantity", "type"),
        column(&fields, "quantity", "nullable"),
        column(&fields, "quantity", "comment"),
        field(&fields, |p| {
            p["kind"] == "index" && p["field"] == "includedColumns"
        }),
        field(&fields, |p| {
            p["kind"] == "constraint" && p["field"] == "keys" && p["name"] == "orders_pkey"
        }),
    ] {
        assert_eq!(structured["kind"], "equal", "{}", structured["path"]);
    }
    assert_eq!(
        objects[0]["incomparableFields"].as_u64().unwrap(),
        fields
            .iter()
            .filter(|f| f["kind"] == "notComparable")
            .count() as u64
    );

    // Either endpoint outside major 16 is refused with its side and version.
    for (source, target, side) in [(&a, &d, Side::Target), (&d, &a, Side::Source)] {
        let status = run(
            &state,
            fresh(source, "crossdb", target, "crossdb"),
            Duration::from_secs(60),
        )
        .await;
        let StatusState::Failed {
            failure:
                CompareError::UnsupportedVersion {
                    side: failed,
                    version,
                },
        } = status.state
        else {
            panic!("{:?}", status.state)
        };
        assert_eq!(failed, side);
        assert!(!version.starts_with("16."), "{version}");
    }
    release_all(&state.pg_schema_compare);

    // A missing schema and a role without table SELECT fail closed.
    let status = run(
        &state,
        fresh(&a, "crossdb", &b, "absent"),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        status.state,
        StatusState::Failed {
            failure: CompareError::Unavailable
        }
    );
    let mut restricted = stored("cross-restricted", primary, FIXTURE_DATABASE);
    restricted.user = "cross_restricted".into();
    store(&state, &restricted).await;
    let status = run(
        &state,
        fresh(&a, "crossdb", &restricted, "crossdb"),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        status.state,
        StatusState::Failed {
            failure: CompareError::Unavailable
        }
    );
    release_all(&state.pg_schema_compare);
}

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixtures only"]
async fn native_tls_verification_failure_fails_closed() {
    let port = env_port(TLS_PORT);
    let ca = std::env::var(TLS_CA).expect("native.py exports the fixture CA path");
    let (_dir, state) = crate::test_app_state().await;
    let mut pg = stored("tls-endpoint", port, FIXTURE_DATABASE);
    pg.ssl = true;
    // Positive control: the server really negotiates TLS.
    {
        let mut spec = ResolvedPostgresConnectSpec::from_postgres(&pg);
        spec.tls = ResolvedTls::with_mode("127.0.0.1", PgTlsMode::Require);
        let admin = dedicated::connect(&spec, dedicated::NoticeSink::Ignore)
            .await
            .unwrap();
        let row = admin
            .client
            .query_one(
                "SELECT ssl, version() FROM pg_stat_ssl, (SELECT version()) v WHERE pid = pg_backend_pid()",
                &[],
            )
            .await
            .unwrap();
        assert!(row.get::<_, bool>(0));
        println!("tls endpoint: {}", row.get::<_, String>(1));
        admin
            .client
            .batch_execute(
                "CREATE SCHEMA tls_a; CREATE SCHEMA tls_b;
                 CREATE TABLE tls_a.t (id integer DEFAULT 1); CREATE TABLE tls_b.t (id integer DEFAULT 1)",
            )
            .await
            .unwrap();
        admin.close().await;
    }

    // verify-full against an untrusted issuer fails before any catalog read.
    pg.tls_options = Some(PgTlsOptions {
        mode: PgTlsMode::VerifyFull,
        ..Default::default()
    });
    store(&state, &pg).await;
    let status = run(
        &state,
        fresh(&pg, "tls_a", &pg, "tls_b"),
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        status.state,
        StatusState::Failed {
            failure: CompareError::Unavailable
        }
    );
    assert_eq!(status.source_objects, 0);
    assert_eq!(state.pg_schema_compare.budget.used(), 0);

    // The same mode with the fixture CA trusted succeeds, so the failure above
    // was certificate verification rather than connectivity.
    pg.tls_options = Some(PgTlsOptions {
        mode: PgTlsMode::VerifyFull,
        root_cert_path: Some(ca),
        ..Default::default()
    });
    store(&state, &pg).await;
    let status = run(
        &state,
        fresh(&pg, "tls_a", &pg, "tls_b"),
        Duration::from_secs(60),
    )
    .await;
    let request = result_request(&status);
    let reader = Reader::new(&state.pg_schema_compare);
    assert_eq!(reader.metadata(&request)["kind"], "equal");
    release_all(&state.pg_schema_compare);
}

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixtures only"]
async fn native_teardown_backend_loss_and_read_only_audit() {
    let port = env_port(PRIMARY_PORT);
    let (_dir, state) = crate::test_app_state().await;
    {
        let bootstrap = admin(&stored("bootstrap", port, FIXTURE_DATABASE)).await;
        bootstrap
            .client
            .batch_execute("CREATE DATABASE audit_db")
            .await
            .unwrap();
        bootstrap.close().await;
    }
    let pg = stored("audit-endpoint", port, "audit_db");
    store(&state, &pg).await;
    let admin = admin(&pg).await;
    admin
        .client
        .batch_execute(
            r#"CREATE SCHEMA td_a; CREATE SCHEMA td_b;
            CREATE TABLE td_a.orders (id integer PRIMARY KEY, quantity integer DEFAULT 1 CHECK (quantity > 0));
            CREATE TABLE td_b.orders (id integer PRIMARY KEY, quantity integer DEFAULT 2 CHECK (quantity > 0));
            CREATE INDEX td_a_qty ON td_a.orders (quantity); CREATE INDEX td_b_qty ON td_b.orders (quantity);
            INSERT INTO td_a.orders VALUES (1, 1), (2, 2); INSERT INTO td_b.orders VALUES (1, 1);
            CREATE TABLE public.ddl_audit (tag text);
            CREATE FUNCTION public.ddl_audit_fn() RETURNS event_trigger LANGUAGE plpgsql AS $$
              BEGIN INSERT INTO public.ddl_audit VALUES (tg_tag); END $$;
            CREATE EVENT TRIGGER ddl_audit ON ddl_command_start EXECUTE FUNCTION public.ddl_audit_fn();"#,
        )
        .await
        .unwrap();
    let backends = || async {
        admin
            .client
            .query_one(
                "SELECT count(*)::integer FROM pg_stat_activity WHERE datname = current_database() AND backend_type = 'client backend'",
                &[],
            )
            .await
            .unwrap()
            .get::<_, i32>(0)
    };
    let scans = || async {
        admin
            .client
            .query(
                "SELECT schemaname || '.' || relname, seq_scan, idx_scan, n_tup_ins FROM pg_stat_user_tables WHERE schemaname IN ('td_a','td_b') ORDER BY 1",
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row.get::<_, String>(0),
                    row.get::<_, i64>(1),
                    row.get::<_, Option<i64>>(2),
                    row.get::<_, i64>(3),
                )
            })
            .collect::<Vec<_>>()
    };
    let ddl = || async {
        admin
            .client
            .query("SELECT tag FROM public.ddl_audit", &[])
            .await
            .unwrap()
            .iter()
            .map(|row| row.get::<_, String>(0))
            .collect::<Vec<_>>()
    };
    let manager = state.pg_schema_compare.clone();
    let baseline_backends = backends().await;
    // Let the fixture session's own pending statistics flush before the
    // baseline so later deltas belong to the comparison backends only.
    admin
        .client
        .batch_execute("SELECT pg_stat_force_next_flush(); SELECT 1")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let baseline_scans = scans().await;
    println!("baseline user-table statistics: {baseline_scans:?}");
    assert!(ddl().await.is_empty());

    // A successful comparison uses exactly one dedicated backend and closes it.
    let status = run(
        &state,
        fresh(&pg, "td_a", &pg, "td_b"),
        Duration::from_secs(60),
    )
    .await;
    let request = result_request(&status);
    let reader = Reader::new(&manager);
    assert_eq!(reader.metadata(&request)["kind"], "changed");
    let settle = |expected: i32| async move {
        tokio::time::timeout(Duration::from_secs(10), async {
            while backends().await != expected {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("dedicated backends closed")
    };
    settle(baseline_backends).await;

    // Teardown of an endpoint through the canonical fence cancels a capture
    // blocked on its pre-snapshot lock, waits for the driver and invalidates
    // the job. The comparison never sees more than one backend.
    admin
        .client
        .batch_execute("BEGIN; LOCK TABLE ONLY td_a.orders IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let status = manager
        .start_native(fresh(&pg, "td_a", &pg, "td_b"), state.pool.clone())
        .unwrap();
    wait_for_lock(&admin.client, "td_a.orders").await;
    assert_eq!(backends().await, baseline_backends + 1);
    let job_id = status.job_id.clone();
    let mut done = done_signal(&manager, &job_id);
    let started = std::time::Instant::now();
    crate::socket_lifecycle::with_connection_fence(&state, &pg.id, async {
        assert_eq!(manager.get(&job_id), Err(CompareError::Unavailable));
        assert!(
            *done.borrow_and_update(),
            "fence waited for the worker join"
        );
        assert_eq!(manager.budget.used(), 0);
    })
    .await;
    let termination = started.elapsed();
    println!("teardown during lock wait terminated in {termination:?}");
    assert!(termination < Duration::from_secs(8), "{termination:?}");
    admin.client.batch_execute("ROLLBACK").await.unwrap();
    settle(baseline_backends).await;

    // Plain cancellation during the same wait terminates within the cleanup
    // grace and releases every reservation.
    admin
        .client
        .batch_execute("BEGIN; LOCK TABLE ONLY td_a.orders IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let status = manager
        .start_native(fresh(&pg, "td_a", &pg, "td_b"), state.pool.clone())
        .unwrap();
    wait_for_lock(&admin.client, "td_a.orders").await;
    let started = std::time::Instant::now();
    manager.cancel(&status.job_id).unwrap();
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(8)).await;
    println!(
        "cancellation during lock wait terminated in {:?}",
        started.elapsed()
    );
    assert_eq!(status.state, StatusState::Cancelled);
    assert_eq!(manager.budget.used(), 0);
    admin.client.batch_execute("ROLLBACK").await.unwrap();
    settle(baseline_backends).await;

    // Losing the catalog backend mid-capture is a failed job, never a
    // completed result or an inferred absence.
    admin
        .client
        .batch_execute("BEGIN; LOCK TABLE ONLY td_a.orders IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let status = manager
        .start_native(fresh(&pg, "td_a", &pg, "td_b"), state.pool.clone())
        .unwrap();
    let pid = wait_for_lock(&admin.client, "td_a.orders").await;
    assert!(admin
        .client
        .query_one("SELECT pg_terminate_backend($1)", &[&pid])
        .await
        .unwrap()
        .get::<_, bool>(0));
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(10)).await;
    assert_eq!(
        status.state,
        StatusState::Failed {
            failure: CompareError::Unavailable
        }
    );
    assert_eq!(manager.budget.used(), 0);
    admin.client.batch_execute("ROLLBACK").await.unwrap();
    settle(baseline_backends).await;

    // No comparison executed DDL or scanned user tables; the lock waits above
    // used AccessShareLock only.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(ddl().await, Vec::<String>::new());
    let final_scans = scans().await;
    println!("final user-table statistics: {final_scans:?}");
    assert_eq!(final_scans, baseline_scans);
    release_all(&manager);
    admin.close().await;
}

const DDL_SCHEMA: &str = r#"
CREATE TABLE {s}.orders (
    id integer PRIMARY KEY,
    quantity integer DEFAULT 2,
    CONSTRAINT positive CHECK (quantity > 0)
);
CREATE INDEX o_expr ON {s}.orders ((quantity + 1)) WHERE quantity > 0;
CREATE INDEX o_plain ON {s}.orders (quantity);
"#;

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixtures only"]
async fn native_concurrent_ddl_during_lock_wait_is_consistent_or_retried() {
    let port = env_port(PRIMARY_PORT);
    let (_dir, state) = crate::test_app_state().await;
    let pg = stored("ddl-endpoint", port, FIXTURE_DATABASE);
    store(&state, &pg).await;
    let admin = admin(&pg).await;
    for schema in ["ddl_s", "ddl_t"] {
        admin
            .client
            .batch_execute(&format!(
                "CREATE SCHEMA {schema}; {}",
                DDL_SCHEMA.replace("{s}", schema)
            ))
            .await
            .unwrap();
    }
    let manager = state.pg_schema_compare.clone();
    let reader = Reader::new(&manager);

    // A rename committed while the reader waits for its pre-snapshot lock is
    // observed completely: column facts, CHECK, index expression and predicate
    // all render the new name. No mixed pre/post-rename definition escapes.
    admin
        .client
        .batch_execute("BEGIN; LOCK TABLE ONLY ddl_t.orders IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let status = manager
        .start_native(fresh(&pg, "ddl_s", &pg, "ddl_t"), state.pool.clone())
        .unwrap();
    wait_for_lock(&admin.client, "ddl_t.orders").await;
    admin
        .client
        .batch_execute("ALTER TABLE ddl_t.orders RENAME COLUMN quantity TO amount; COMMIT")
        .await
        .unwrap();
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(60)).await;
    let request = result_request(&status);
    let objects = reader.objects(&request);
    assert_eq!(objects[0]["kind"], "changed");
    // 11 source-only + 11 target-only column facts, the CHECK's key list and
    // expression, the expression index key and predicate, and the plain key.
    assert_eq!(objects[0]["changedFields"], 27);
    let fields = reader.fields(&request, "orders");
    assert_eq!(column(&fields, "quantity", "type")["kind"], "sourceOnly");
    assert_eq!(column(&fields, "amount", "type")["kind"], "targetOnly");
    let keys = field(&fields, |p| {
        p["kind"] == "constraint" && p["field"] == "keys" && p["name"] == "positive"
    });
    assert_eq!(reader.text(&request, &keys["target"]), "[\"amount\"]");
    let expression = field(&fields, |p| {
        p["kind"] == "constraint" && p["field"] == "expression" && p["name"] == "positive"
    });
    assert_eq!(expression["kind"], "changed");
    assert_eq!(
        reader.text(&request, &expression["source"]),
        "(quantity > 0)"
    );
    assert_eq!(reader.text(&request, &expression["target"]), "(amount > 0)");
    let key = field(&fields, |p| {
        p["kind"] == "indexKey" && p["name"] == "o_expr" && p["field"] == "expression"
    });
    assert_eq!(reader.text(&request, &key["target"]), "(amount + 1)");
    let predicate = field(&fields, |p| {
        p["kind"] == "index" && p["name"] == "o_expr" && p["field"] == "predicate"
    });
    assert_eq!(reader.text(&request, &predicate["target"]), "(amount > 0)");
    let plain = field(&fields, |p| {
        p["kind"] == "indexKey" && p["name"] == "o_plain" && p["field"] == "column"
    });
    assert_eq!(reader.text(&request, &plain["target"]), "amount");
    for item in &fields {
        if let Some(target) = item.get("target").filter(|v| !v.is_null()) {
            let text = reader.text(&request, target);
            assert!(!text.contains("quantity"), "{}: {text}", item["path"]);
        }
    }

    // A drop committed during the wait fails that attempt and the single fresh
    // retry proves absence from a complete inventory: source-only, not an error.
    admin
        .client
        .batch_execute("BEGIN; LOCK TABLE ONLY ddl_t.orders IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let status = manager
        .start_native(fresh(&pg, "ddl_s", &pg, "ddl_t"), state.pool.clone())
        .unwrap();
    wait_for_lock(&admin.client, "ddl_t.orders").await;
    admin
        .client
        .batch_execute("DROP TABLE ddl_t.orders; COMMIT")
        .await
        .unwrap();
    let status = finished_within(&manager, &status.job_id, Duration::from_secs(60)).await;
    assert_eq!(status.source_objects, 1);
    assert_eq!(status.target_objects, 0);
    let request = result_request(&status);
    let objects = reader.objects(&request);
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["kind"], "sourceOnly");
    release_all(&manager);
    admin.close().await;
}

const UNUSUAL_SCHEMA: &str = r#"
CREATE SCHEMA {s};
CREATE COLLATION {s}.local_c FROM "C";
CREATE TYPE {s}.status AS ENUM ('a', 'b');
CREATE DOMAIN {s}.positive AS integer CHECK (VALUE > 0);
CREATE TYPE {s}.pair AS (x integer, y integer);
CREATE TYPE {s}.span AS RANGE (subtype = numeric);
CREATE TABLE {s}.things (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    parent integer CONSTRAINT things_parent_fkey REFERENCES ext.parents (id) ON DELETE SET NULL,
    label text COLLATE {s}.local_c,
    code character varying(20) COLLATE "C",
    mood ext.mood DEFAULT 'calm',
    status {s}.status,
    amount {s}.positive,
    pair {s}.pair,
    span {s}.span,
    moods ext.mood[],
    prices numeric(10,2)[],
    at timestamptz DEFAULT now(),
    payload jsonb,
    doubled numeric GENERATED ALWAYS AS (amount * 2) STORED,
    v integer,
    w integer,
    CONSTRAINT things_v_key UNIQUE (v)
);
CREATE INDEX things_label ON {s}.things (label COLLATE "C" text_pattern_ops);
CREATE INDEX things_span ON {s}.things USING gist (span);
"#;

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixtures only"]
async fn native_unusual_types_collations_identity_sequences_and_invalid_indexes() {
    let port = env_port(PRIMARY_PORT);
    let (_dir, state) = crate::test_app_state().await;
    let pg = stored("unusual-endpoint", port, FIXTURE_DATABASE);
    store(&state, &pg).await;
    let admin = admin(&pg).await;
    admin
        .client
        .batch_execute(
            "CREATE SCHEMA ext; CREATE TABLE ext.parents (id integer PRIMARY KEY); CREATE TYPE ext.mood AS ENUM ('calm')",
        )
        .await
        .unwrap();
    for schema in ["ut_s", "ut_t"] {
        admin
            .client
            .batch_execute(&UNUSUAL_SCHEMA.replace("{s}", schema))
            .await
            .unwrap();
    }
    let manager = state.pg_schema_compare.clone();
    let reader = Reader::new(&manager);
    let compare = || async {
        run(
            &state,
            fresh(&pg, "ut_s", &pg, "ut_t"),
            Duration::from_secs(60),
        )
        .await
    };

    let status = compare().await;
    let request = result_request(&status);
    let objects = reader.objects(&request);
    // The composite type's relation and the identity-owned sequence are
    // disclosed as excluded counterparts on both sides, never compared.
    assert_eq!(objects.len(), 3);
    let things = |objects: &[Value]| -> Value {
        objects
            .iter()
            .find(|o| o["source"]["name"] == "things")
            .cloned()
            .unwrap()
    };
    for (name, kind) in [("pair", "composite"), ("things_id_seq", "sequence")] {
        let excluded = objects
            .iter()
            .find(|o| o["source"]["name"] == name)
            .unwrap();
        assert_eq!(excluded["source"]["kind"], kind);
        assert_eq!(excluded["kind"], "notComparable");
        assert_eq!(excluded["reason"], "excludedCounterpart");
        assert_eq!(excluded["target"]["name"], name);
    }
    assert_eq!(things(&objects)["kind"], "notComparable");
    assert_eq!(things(&objects)["changedFields"], 0);
    let fields = reader.fields(&request, "things");
    let reference = |item: &Value, side: &str| -> Value {
        serde_json::from_str(&reader.text(&request, &item[side])).unwrap()
    };
    let selected =
        |name: &str| serde_json::json!({"namespace": {"kind": "selected"}, "name": name});
    let external = |schema: &str, name: &str| serde_json::json!({"namespace": {"kind": "external", "schema": schema}, "name": name});
    // Selected-schema definitions map to one namespace on both sides; external
    // references keep their exact schema. Their definitions stay out of scope.
    let collation = column(&fields, "label", "collation");
    assert_eq!(collation["kind"], "equal");
    assert_eq!(reference(collation, "source"), selected("local_c"));
    assert_eq!(reference(collation, "target"), selected("local_c"));
    assert_eq!(
        reference(column(&fields, "code", "collation"), "source"),
        external("pg_catalog", "C")
    );
    assert_eq!(
        reference(column(&fields, "status", "type"), "target"),
        selected("status")
    );
    assert_eq!(
        reference(column(&fields, "amount", "type"), "source"),
        selected("positive")
    );
    assert_eq!(
        reference(column(&fields, "pair", "type"), "source"),
        selected("pair")
    );
    assert_eq!(
        reference(column(&fields, "span", "type"), "source"),
        selected("span")
    );
    assert_eq!(
        reference(column(&fields, "mood", "type"), "source"),
        external("ext", "mood")
    );
    assert_eq!(
        reference(column(&fields, "moods", "type"), "source"),
        external("ext", "_mood")
    );
    assert_eq!(
        reader.text(
            &request,
            &column(&fields, "moods", "arrayDimensions")["source"]
        ),
        "1"
    );
    // Array columns retain the element typmod: numeric(10,2)[] is 655366.
    assert_eq!(
        reader.text(
            &request,
            &column(&fields, "prices", "typeModifier")["source"]
        ),
        "655366"
    );
    assert_eq!(
        reader.text(
            &request,
            &column(&fields, "prices", "arrayDimensions")["source"]
        ),
        "1"
    );
    assert_eq!(
        reader.text(&request, &column(&fields, "code", "typeModifier")["source"]),
        "24"
    );
    assert_eq!(
        reader.text(&request, &column(&fields, "id", "identity")["source"]),
        "always"
    );
    assert_eq!(
        reader.text(
            &request,
            &column(&fields, "doubled", "generatedKind")["source"]
        ),
        "stored"
    );
    let fk = field(&fields, |p| {
        p["kind"] == "constraint"
            && p["name"] == "things_parent_fkey"
            && p["field"] == "referencedTable"
    });
    assert_eq!(reference(fk, "target"), external("ext", "parents"));
    let opclass = field(&fields, |p| {
        p["kind"] == "indexKey" && p["name"] == "things_label" && p["field"] == "opclass"
    });
    assert_eq!(
        reference(opclass, "source"),
        external("pg_catalog", "text_pattern_ops")
    );
    let key_collation = field(&fields, |p| {
        p["kind"] == "indexKey" && p["name"] == "things_label" && p["field"] == "collation"
    });
    assert_eq!(
        reference(key_collation, "source"),
        external("pg_catalog", "C")
    );
    // A default depending on an external enum type, a function call and a
    // domain-typed generated expression are conservatively incomparable with
    // their specific reason, never asserted equal.
    for (rendered, reason) in [
        (column(&fields, "mood", "default"), "externalDependency"),
        (column(&fields, "at", "default"), "expressionOutsideSubset"),
        (
            column(&fields, "doubled", "generatedExpression"),
            "expressionOutsideSubset",
        ),
    ] {
        assert_eq!(rendered["kind"], "notComparable", "{}", rendered["path"]);
        assert_eq!(rendered["reason"], reason, "{}", rendered["path"]);
    }

    // Identity sequence configuration is outside the projection.
    admin
        .client
        .batch_execute(
            "ALTER TABLE ut_t.things ALTER COLUMN id SET INCREMENT BY 5; ALTER TABLE ut_t.things ALTER COLUMN id RESTART WITH 1000",
        )
        .await
        .unwrap();
    let status = compare().await;
    let request = result_request(&status);
    assert_eq!(things(&reader.objects(&request))["changedFields"], 0);

    // NULLS NOT DISTINCT on the owning unique constraint and an FK delete-column
    // subset are exact known changes.
    admin
        .client
        .batch_execute(
            "ALTER TABLE ut_t.things DROP CONSTRAINT things_v_key, ADD CONSTRAINT things_v_key UNIQUE NULLS NOT DISTINCT (v);
             ALTER TABLE ut_t.things DROP CONSTRAINT things_parent_fkey,
               ADD CONSTRAINT things_parent_fkey FOREIGN KEY (parent) REFERENCES ext.parents (id) ON DELETE SET NULL (parent)",
        )
        .await
        .unwrap();
    let status = compare().await;
    let request = result_request(&status);
    assert_eq!(things(&reader.objects(&request))["changedFields"], 2);
    let fields = reader.fields(&request, "things");
    let nulls = field(&fields, |p| {
        p["kind"] == "index" && p["owner"] == "things_v_key" && p["field"] == "nullsNotDistinct"
    });
    assert_eq!(nulls["kind"], "changed");
    assert_eq!(reader.text(&request, &nulls["target"]), "true");
    let subset = field(&fields, |p| {
        p["kind"] == "constraint"
            && p["name"] == "things_parent_fkey"
            && p["field"] == "deleteColumns"
    });
    assert_eq!(subset["kind"], "changed");
    assert_eq!(subset["source"]["valueKind"], "null");
    assert_eq!(reader.text(&request, &subset["target"]), "[\"parent\"]");

    // An invalid index left by a failed concurrent build is a known state
    // difference on the same-named index, not an absence.
    admin
        .client
        .batch_execute(
            "CREATE UNIQUE INDEX things_w_unique ON ut_s.things (w);
             INSERT INTO ut_t.things (v, w, amount) VALUES (1, 1, 1), (2, 1, 1)",
        )
        .await
        .unwrap();
    assert!(admin
        .client
        .batch_execute("CREATE UNIQUE INDEX CONCURRENTLY things_w_unique ON ut_t.things (w)")
        .await
        .is_err());
    let invalid = admin
        .client
        .query_one(
            "SELECT indisvalid, indisready FROM pg_index WHERE indexrelid = 'ut_t.things_w_unique'::regclass",
            &[],
        )
        .await
        .unwrap();
    assert!(!invalid.get::<_, bool>(0));
    let ready_differs = !invalid.get::<_, bool>(1);
    let status = compare().await;
    let request = result_request(&status);
    let fields = reader.fields(&request, "things");
    let valid = field(&fields, |p| {
        p["kind"] == "index" && p["name"] == "things_w_unique" && p["field"] == "valid"
    });
    assert_eq!(valid["kind"], "changed");
    assert_eq!(reader.text(&request, &valid["source"]), "true");
    assert_eq!(reader.text(&request, &valid["target"]), "false");
    assert_eq!(
        things(&reader.objects(&request))["changedFields"]
            .as_u64()
            .unwrap(),
        3 + u64::from(ready_differs)
    );
    release_all(&manager);
    admin.close().await;
}

/// Each realistic table yields 163 facts: 2 table, 8 x 11 column, two
/// constraints x 15, primary-key index 11 + 7, and a two-key index 11 + 14.
const REALISTIC_FACTS: usize = 163;
/// A one-column table yields 2 + 11 facts.
const NARROW_FACTS: usize = 13;
/// 250 tables x (2 + 18 x 11) facts is exactly the 50,000 per-endpoint cap.
const EXACT_TABLES: usize = 250;

async fn build_profile_schema(client: &tokio_postgres::Client, shape: &str, tables: usize) {
    for schema in ["prof_s", "prof_t"] {
        client
            .batch_execute(&format!(
                "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}"
            ))
            .await
            .unwrap();
        let body = match shape {
            "realistic" => format!(
                r#"DO $$ BEGIN FOR i IN 1..{tables} LOOP
                    EXECUTE format('CREATE TABLE {schema}.t%s (id integer PRIMARY KEY, c1 integer DEFAULT 1 CONSTRAINT t%s_c1 CHECK (c1 > 0), c2 text, c3 bigint, c4 numeric(10,2), c5 boolean, c6 timestamptz, c7 text[])', i, i);
                    EXECUTE format('CREATE INDEX t%s_c2 ON {schema}.t%s (c2, c3) WHERE c1 > 0', i, i);
                    EXECUTE format('COMMENT ON TABLE {schema}.t%s IS %L', i, 'table ' || i);
                    EXECUTE format('COMMENT ON COLUMN {schema}.t%s.c1 IS %L', i, 'first column');
                END LOOP; END $$"#
            ),
            "narrow" => format!(
                r#"DO $$ BEGIN FOR i IN 1..{tables} LOOP
                    EXECUTE format('CREATE TABLE {schema}.n%s (id integer)', i);
                END LOOP; END $$"#
            ),
            "exact" => format!(
                r#"DO $$ BEGIN FOR i IN 1..{tables} LOOP
                    EXECUTE format('CREATE TABLE {schema}.e%s (c1 integer, c2 integer, c3 integer, c4 integer, c5 integer, c6 integer, c7 integer, c8 integer, c9 integer, c10 integer, c11 integer, c12 integer, c13 integer, c14 integer, c15 integer, c16 integer, c17 integer, c18 integer)', i);
                END LOOP; END $$"#
            ),
            _ => unreachable!(),
        };
        client.batch_execute(&body).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "run infrastructure/test-db/schema-compare/native.py; owned disposable fixtures only"]
async fn native_memory_profile_with_increasing_schema_size_and_exact_limits() {
    let port = env_port(PRIMARY_PORT);
    let (_dir, state) = crate::test_app_state().await;
    {
        let bootstrap = admin(&stored("bootstrap", port, FIXTURE_DATABASE)).await;
        bootstrap
            .client
            .batch_execute("CREATE DATABASE profile_db")
            .await
            .unwrap();
        bootstrap.close().await;
    }
    let pg = stored("profile-endpoint", port, "profile_db");
    store(&state, &pg).await;
    let admin = admin(&pg).await;
    let manager = state.pg_schema_compare.clone();
    let reader = Reader::new(&manager);
    println!();
    println!("| shape | tables | facts/side | outcome | peak budget MiB | max RSS MiB | seconds |");
    println!("| --- | ---: | ---: | --- | ---: | ---: | ---: |");
    let mib = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
    println!(
        "| baseline | 0 | 0 | before any job | {:.1} | {:.1} | 0.00 |",
        mib(manager.budget.peak()),
        mib(max_rss_bytes())
    );
    let mut outcomes = Vec::new();
    for (shape, tables, facts) in [
        ("realistic", 10, 10 * REALISTIC_FACTS),
        ("realistic", 100, 100 * REALISTIC_FACTS),
        ("realistic", 300, 300 * REALISTIC_FACTS),
        ("narrow", TABLE_ENTRIES, TABLE_ENTRIES * NARROW_FACTS),
        (
            "narrow",
            TABLE_ENTRIES + 1,
            (TABLE_ENTRIES + 1) * NARROW_FACTS,
        ),
        ("exact", EXACT_TABLES, MAX_VALUES),
        ("exact+1", EXACT_TABLES, MAX_VALUES + 11),
    ] {
        if shape == "exact+1" {
            admin
                .client
                .batch_execute("ALTER TABLE prof_s.e1 ADD COLUMN extra integer")
                .await
                .unwrap();
        } else {
            build_profile_schema(&admin.client, shape, tables).await;
        }
        manager.budget.reset_peak();
        let started = std::time::Instant::now();
        let status = run(
            &state,
            fresh(&pg, "prof_s", &pg, "prof_t"),
            Duration::from_secs(60),
        )
        .await;
        let seconds = started.elapsed().as_secs_f64();
        let outcome = match &status.state {
            StatusState::Completed { .. } => {
                let request = result_request(&status);
                let metadata = reader.metadata(&request);
                assert_eq!(status.source_objects as usize, tables);
                let objects = reader.objects(&request);
                assert_eq!(objects.len(), tables);
                let observed: u64 = objects
                    .iter()
                    .map(|o| o["fieldCount"].as_u64().unwrap())
                    .sum();
                assert_eq!(observed as usize, facts, "{shape} {tables}");
                // Reading pages after completion stays within the same budget.
                let first_fields =
                    reader.fields(&request, objects[0]["source"]["name"].as_str().unwrap());
                assert!(!first_fields.is_empty());
                format!("completed ({})", metadata["kind"].as_str().unwrap())
            }
            StatusState::Failed { failure } => {
                format!("failed {}", serde_json::to_string(failure).unwrap())
            }
            other => panic!("{other:?}"),
        };
        let peak = manager.budget.peak();
        println!(
            "| {shape} | {tables} | {facts} | {outcome} | {:.1} | {:.1} | {seconds:.2} |",
            mib(peak),
            mib(max_rss_bytes())
        );
        release_all(&manager);
        assert!(peak <= GLOBAL_BYTES - CONTROL_BYTES);
        outcomes.push((shape, tables, status.state));
    }
    assert!(matches!(outcomes[0].2, StatusState::Completed { .. }));
    assert!(matches!(outcomes[1].2, StatusState::Completed { .. }));
    assert!(matches!(
        outcomes[2].2,
        StatusState::Completed { .. }
            | StatusState::Failed {
                failure: CompareError::LimitExceeded {
                    limit: Limit::ResultBytes
                }
            }
    ));
    assert!(matches!(outcomes[3].2, StatusState::Completed { .. }));
    assert_eq!(
        outcomes[4].2,
        StatusState::Failed {
            failure: CompareError::LimitExceeded {
                limit: Limit::Tables
            }
        }
    );
    assert!(matches!(outcomes[5].2, StatusState::Completed { .. }));
    assert_eq!(
        outcomes[6].2,
        StatusState::Failed {
            failure: CompareError::LimitExceeded {
                limit: Limit::ChildFacts
            }
        }
    );

    // Two concurrent jobs, two retained results and two in-flight pages share
    // the one global budget; a third page is busy rather than queued.
    build_profile_schema(&admin.client, "realistic", 100).await;
    let pg_two = stored("profile-endpoint-2", port, "profile_db");
    store(&state, &pg_two).await;
    manager.budget.reset_peak();
    let first = manager
        .start_native(fresh(&pg, "prof_s", &pg, "prof_t"), state.pool.clone())
        .unwrap();
    let second = manager
        .start_native(
            fresh(&pg_two, "prof_t", &pg_two, "prof_s"),
            state.pool.clone(),
        )
        .unwrap();
    let first = finished_within(&manager, &first.job_id, Duration::from_secs(60)).await;
    let second = finished_within(&manager, &second.job_id, Duration::from_secs(60)).await;
    let retained = manager.budget.used();
    assert!(retained > 0 && retained <= 2 * RESULT_BYTES, "{retained}");
    let (first, second) = (result_request(&first), result_request(&second));
    let held = AtomicUsize::new(0);
    for (request, id) in [(&first, "page-one"), (&second, "page-two")] {
        manager
            .read(
                WINDOW,
                &reader.transport,
                id,
                request,
                ReadRequest::Objects { offset: 0 },
                |_| {
                    held.fetch_add(1, Ordering::SeqCst);
                },
            )
            .unwrap();
    }
    assert_eq!(held.load(Ordering::SeqCst), 2);
    assert_eq!(
        manager.read(
            WINDOW,
            &reader.transport,
            "page-three",
            &first,
            ReadRequest::Metadata,
            |_| panic!()
        ),
        Err(CompareError::Busy)
    );
    assert_eq!(manager.budget.used(), retained + 2 * SERIALIZER_SCRATCH);
    println!(
        "| concurrent | 2 x 100 | 2 x {} | two results + two pages | {:.1} | {:.1} | - |",
        100 * REALISTIC_FACTS,
        mib(manager.budget.peak()),
        mib(max_rss_bytes())
    );
    for id in ["page-one", "page-two"] {
        manager.acknowledge(WINDOW, &reader.transport, id).unwrap();
    }
    manager.release(&first.identity.job_id).unwrap();
    manager.release(&second.identity.job_id).unwrap();
    assert_eq!(manager.budget.used(), 0);
    admin.close().await;
}
