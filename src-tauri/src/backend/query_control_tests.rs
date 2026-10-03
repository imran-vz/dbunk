//! Public-facade tests. Default cases never connect or access OS Keychain.
//! Ignored cases require the owned stage03 fixture and use session-local tables.
use super::*;
use futures_util::FutureExt;
use std::pin::Pin;
use tokio::sync::mpsc;

const WINDOW: &str = "query-control-test";
const WAIT: Duration = Duration::from_secs(10);
static LIVE_LOCK: Mutex<()> = Mutex::const_new(());

fn request(session: &str, sql: &str) -> ExecutePayload {
    ExecutePayload {
        session_id: session.into(),
        execution_id: uuid::Uuid::new_v4().to_string(),
        sql: sql.into(),
        confirmed: false,
        parameters: None,
        row_limit: None,
    }
}

#[test]
fn parameter_description_preserves_lexical_boundaries_and_first_use_order() {
    let sql = "SELECT 'α :hidden', \"β:quoted\", $$:body$$, :second::text, :first, :second, ARRAY[:last] -- :comment\n/* :also_hidden */";
    assert_eq!(
        Backend::describe_query_parameters(sql).unwrap().names,
        ["second", "first", "last"]
    );
    for sql in [
        "SELECT 'unterminated :value",
        "SELECT (:value",
        "SELECT /* :value",
    ] {
        assert!(matches!(
            Backend::describe_query_parameters(sql),
            Err(QuerySessionError::ParametersRejected {
                reason: ParameterRejectionReason::Unlexable,
                names
            }) if names.is_empty()
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_and_closed_sessions_refuse_new_control_operations_without_connecting() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let controls = [
        TransactionControl::Mode(QueryTransactionMode::Manual),
        TransactionControl::Isolation(QueryTransactionIsolation::Serializable),
        TransactionControl::Commit,
        TransactionControl::Rollback,
        TransactionControl::Recheck,
    ];
    for control in controls {
        assert!(matches!(
            backend.control_transaction(WINDOW, "absent", control).await,
            Err(QuerySessionError::SessionNotFound)
        ));
    }
    assert!(matches!(
        backend
            .submit_query(WINDOW, request("absent", "SELECT 1"))
            .await,
        Err(QuerySessionError::SessionNotFound)
    ));
    backend.shutdown().await.unwrap();
    for control in controls {
        assert!(matches!(
            backend.control_transaction(WINDOW, "absent", control).await,
            Err(QuerySessionError::ConnectionClosing)
        ));
    }
    assert!(matches!(
        backend
            .submit_query(WINDOW, request("absent", "SELECT 1"))
            .await,
        Err(QuerySessionError::ConnectionClosing)
    ));
}

struct Session {
    id: String,
    generation: u64,
    events: mpsc::Receiver<QueryEventEnvelope>,
}
struct Outcome {
    rows: Vec<Vec<Option<String>>>,
    status: String,
    transaction: QueryTransactionSnapshot,
}
impl Session {
    async fn open(backend: &Backend, id: &str) -> Self {
        let (send, events) = mpsc::channel(64);
        backend
            .open(
                WINDOW,
                OpenSessionPayload {
                    owner_id: "owner".into(),
                    session_id: id.into(),
                    tab_id: format!("tab-{id}"),
                    connection_id: backend.fixture().id,
                },
                Arc::new(move |event: QueryEventEnvelope| {
                    send.try_send(event).map_err(|_| SinkClosed)
                }),
            )
            .await
            .unwrap();
        let mut session = Self {
            id: id.into(),
            generation: 0,
            events,
        };
        let state = session.next().await;
        assert!(matches!(state.event, QueryEvent::SessionState { .. }));
        session.generation = state.generation;
        session
    }
    async fn next(&mut self) -> QueryEventEnvelope {
        tokio::time::timeout(WAIT, self.events.recv())
            .await
            .expect("event deadline")
            .expect("event stream closed")
    }
    async fn finish(&mut self, backend: &Backend, execution: &str) -> Outcome {
        let mut rows = Vec::new();
        loop {
            let event = self.next().await;
            assert_eq!(event.session_id, self.id);
            if event.execution_id.is_none() {
                assert!(matches!(event.event, QueryEvent::SessionState { .. }));
                continue;
            }
            assert_eq!(event.execution_id.as_deref(), Some(execution));
            if event.requires_ack {
                backend
                    .ack(
                        WINDOW,
                        AckPayload {
                            session_id: self.id.clone(),
                            execution_id: execution.into(),
                            ack_through_sequence: event.sequence,
                            retain_more_rows: true,
                        },
                    )
                    .await
                    .unwrap();
            }
            match event.event {
                QueryEvent::RowBatch { rows: batch, .. } => rows.extend(batch),
                QueryEvent::ExecutionCompleted {
                    status,
                    transaction,
                    ..
                } => {
                    return Outcome {
                        rows,
                        status,
                        transaction,
                    }
                }
                QueryEvent::SessionLost { reason } => panic!("Session lost: {reason}"),
                QueryEvent::SessionClosed => panic!("Session closed before completion"),
                _ => {}
            }
        }
    }
    async fn query(&mut self, backend: &Backend, sql: &str) -> Outcome {
        let payload = request(&self.id, sql);
        let execution = payload.execution_id.clone();
        assert!(matches!(
            backend.submit_query(WINDOW, payload).await.unwrap(),
            QuerySubmission::Accepted(_)
        ));
        self.finish(backend, &execution).await
    }
}

async fn challenge(backend: &Backend, payload: ExecutePayload) -> QueryConfirmation {
    match backend.submit_query(WINDOW, payload).await.unwrap() {
        QuerySubmission::NeedsConfirmation(value) => *value,
        QuerySubmission::Accepted(_) => panic!("Expected policy refusal, not dispatched SQL"),
    }
}

/// Change only fixture metadata, deliberately without invalidating the session:
/// this proves confirmation re-reads policy instead of relying on teardown.
async fn stored_policy(backend: &Backend, mode: crate::SafeMode, read_only: bool) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        unreachable!()
    };
    pg.safe_mode = mode;
    pg.read_only = read_only;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}

async fn fixture_count() -> u64 {
    tokio::task::spawn_blocking(|| {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native/fixture.py");
        let output = std::process::Command::new("python3")
            .arg(script)
            .arg("count")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "owned fixture check failed: {}",
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
type Case<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
async fn live(case: impl for<'a> FnOnce(&'a Backend, &'a Backend) -> Case<'a>) {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1"),
        "Explicit owned fixture opt-in required"
    );
    let _serial = LIVE_LOCK.lock().await;
    let baseline = fixture_count().await;
    let directory = profile::directory();
    let foreign_directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let foreign = Backend::open_fixture(&foreign_directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    backend
        .register_owner(
            WINDOW,
            RegisterOwnerPayload {
                owner_id: "owner".into(),
            },
        )
        .await
        .unwrap();
    let result = std::panic::AssertUnwindSafe(case(&backend, &foreign))
        .catch_unwind()
        .await;
    let (first, second) = tokio::join!(backend.shutdown(), foreign.shutdown());
    first.unwrap();
    second.unwrap();
    tokio::time::timeout(WAIT, async {
        while fixture_count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("fixture backends returned to baseline");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 PostgreSQL fixture; no simultaneous window acceptance"]
async fn native_query_confirmation_binds_request_backend_window_and_session_instance() {
    live(|backend, foreign| {
        Box::pin(async move {
        let mut session = Session::open(backend, "same-session-id").await;
        let _peer = Session::open(backend, "peer").await;
        let setup = session.query(backend, "CREATE TEMP TABLE facade_confirmation(value text); INSERT INTO facade_confirmation VALUES ('bound'), ('other');").await;
        assert_eq!(setup.status, "completed");
        stored_policy(backend, crate::SafeMode::Strict, false).await;
        let mut original = request(&session.id, "DELETE FROM pg_temp.facade_confirmation WHERE value = :needle");
        original.parameters = Some(vec![ParameterValue { name: "needle".into(), value: Some("bound".into()) }]);
        original.confirmed = true;
        // Both facade entry points discard a caller-forged confirmed flag.
        assert!(matches!(backend.execute(WINDOW, original.clone()).await, Err(QuerySessionError::PolicyNeedsConfirmation { .. })));
        let confirmation = challenge(backend, original.clone()).await;
        assert_eq!(confirmation.sql(), original.sql);
        assert_eq!(confirmation.execution_id(), original.execution_id);
        assert!(!confirmation.statements().is_empty());
        let debug = format!("{confirmation:?}");
        assert!(!debug.contains("facade_confirmation") && !debug.contains("needle") && !debug.contains("\"bound\""));
        original.sql = "DELETE FROM pg_temp.facade_confirmation".into();
        original.parameters.as_mut().unwrap()[0].value = Some("other".into());
        let execution = confirmation.execution_id().to_owned();
        backend.clone().confirm_query(WINDOW, confirmation).await.unwrap();
        assert_eq!(session.finish(backend, &execution).await.status, "completed");
        assert_eq!(crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id).await.unwrap().len(), 1, "only the successful confirmed write is audited");
        assert_eq!(session.query(backend, "SELECT value FROM pg_temp.facade_confirmation;").await.rows, vec![vec![Some("other".into())]]);

        let sql = "DELETE FROM pg_temp.facade_confirmation";
        let wrong_window = challenge(backend, request(&session.id, sql)).await;
        assert!(matches!(backend.confirm_query("foreign-window", wrong_window).await, Err(QuerySessionError::OwnerMismatch)));
        let wrong_backend = challenge(backend, request(&session.id, sql)).await;
        assert!(matches!(foreign.confirm_query(WINDOW, wrong_backend).await, Err(QuerySessionError::ConnectionLost)));
        assert_eq!(session.query(backend, "SELECT count(*)::text FROM pg_temp.facade_confirmation;").await.rows[0][0].as_deref(), Some("1"));

        let stale = challenge(backend, request(&session.id, sql)).await;
        let generation = session.generation;
        backend.close(WINDOW, SessionPayload { session_id: session.id.clone() }).await.unwrap();
        // Stage03 joins retained native workers when the owning window retires.
        // Re-registering the same owner must not revive its old confirmation.
        backend.retire_window(WINDOW).await.unwrap();
        backend.register_owner(WINDOW, RegisterOwnerPayload { owner_id: "owner".into() }).await.unwrap();
        let replacement = Session::open(backend, &session.id).await;
        assert_eq!(replacement.generation, generation, "connection generation alone must not identify a session instance");
        assert!(matches!(backend.confirm_query(WINDOW, stale).await, Err(QuerySessionError::SessionNotFound)));
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 PostgreSQL fixture; no simultaneous window acceptance"]
async fn native_query_confirmation_rechecks_read_only_and_validates_before_confirmation() {
    live(|backend, _| {
        Box::pin(async move {
        let mut session = Session::open(backend, "policy").await;
        session.query(backend, "CREATE TEMP TABLE facade_policy(value text); INSERT INTO facade_policy VALUES ('original');").await;
        let payload = request(&session.id, "DELETE FROM pg_temp.facade_policy");
        let confirmation = challenge(backend, payload.clone()).await;
        stored_policy(backend, crate::SafeMode::Disabled, true).await;
        assert!(matches!(backend.confirm_query(WINDOW, confirmation).await, Err(QuerySessionError::PolicyBlocked { .. })));
        let mut forged = payload.clone(); forged.confirmed = true;
        assert!(matches!(backend.submit_query(WINDOW, forged).await, Err(QuerySessionError::PolicyBlocked { .. })));
        assert_eq!(session.query(backend, "SELECT value FROM pg_temp.facade_policy;").await.rows, vec![vec![Some("original".into())]]);

        stored_policy(backend, crate::SafeMode::Strict, false).await;
        let mut missing = request(&session.id, "DELETE FROM pg_temp.facade_policy WHERE value = :missing");
        missing.parameters = Some(vec![]);
        assert!(matches!(backend.submit_query(WINDOW, missing).await, Err(QuerySessionError::ParametersRejected { reason: ParameterRejectionReason::MissingValue, .. })));
        for limit in [0, -1, 10_001] {
            let mut limited = request(&session.id, "SELECT value FROM pg_temp.facade_policy"); limited.row_limit = Some(limit);
            assert!(matches!(backend.submit_query(WINDOW, limited).await, Err(QuerySessionError::InvalidRowLimit)));
        }
        let mut too_large = request(&session.id, "DELETE FROM pg_temp.facade_policy WHERE value = :large");
        too_large.parameters = Some(vec![ParameterValue { name: "large".into(), value: Some("x".repeat(1024 * 1024 + 1)) }]);
        assert!(matches!(backend.submit_query(WINDOW, too_large).await, Err(QuerySessionError::ParametersRejected { reason: ParameterRejectionReason::ValueTooLarge, .. })));
        assert!(crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id).await.unwrap().is_empty(), "refused operations are not audited as successful overrides");
        // A final query also proves rejected inputs did not occupy execution credit.
        assert_eq!(session.query(backend, "SELECT count(*)::text FROM pg_temp.facade_policy;").await.rows[0][0].as_deref(), Some("1"));
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 PostgreSQL fixture; no simultaneous window acceptance"]
async fn native_transactions_are_independent_and_failed_commit_requires_rollback() {
    live(|backend, _| {
        Box::pin(async move {
            let mut first = Session::open(backend, "transaction-a").await;
            let mut second = Session::open(backend, "transaction-b").await;
            let control = |session: &'static str, action| {
                backend.control_transaction(WINDOW, session, action)
            };
            assert_eq!(
                control(
                    "transaction-a",
                    TransactionControl::Mode(QueryTransactionMode::Manual)
                )
                .await
                .unwrap()
                .mode,
                QueryTransactionMode::Manual
            );
            assert_eq!(
                control(
                    "transaction-a",
                    TransactionControl::Isolation(QueryTransactionIsolation::Serializable)
                )
                .await
                .unwrap()
                .manual_isolation,
                QueryTransactionIsolation::Serializable
            );
            assert_eq!(
                control("transaction-b", TransactionControl::Recheck)
                    .await
                    .unwrap(),
                QueryTransactionSnapshot::default()
            );
            let first_result = first
                .query(backend, "SELECT current_setting('transaction_isolation');")
                .await;
            assert_eq!(first_result.rows[0][0].as_deref(), Some("serializable"));
            assert_eq!(
                first_result.transaction.status,
                QueryTransactionStatus::Active
            );
            let second_result = second
                .query(backend, "SELECT current_setting('transaction_isolation');")
                .await;
            assert_eq!(second_result.rows[0][0].as_deref(), Some("read committed"));
            assert_eq!(
                second_result.transaction.status,
                QueryTransactionStatus::Idle
            );
            for action in [
                TransactionControl::Mode(QueryTransactionMode::Autocommit),
                TransactionControl::Isolation(QueryTransactionIsolation::ReadCommitted),
            ] {
                assert!(matches!(
                    control("transaction-a", action).await,
                    Err(QuerySessionError::InvalidTransactionTransition {
                        status: QueryTransactionStatus::Active,
                        ..
                    })
                ));
            }
            assert!(matches!(
                backend
                    .control_transaction("foreign-window", &first.id, TransactionControl::Commit)
                    .await,
                Err(QuerySessionError::OwnerMismatch)
            ));
            assert_eq!(
                control("transaction-a", TransactionControl::Commit)
                    .await
                    .unwrap()
                    .status,
                QueryTransactionStatus::Idle
            );
            assert_eq!(
                first
                    .query(backend, "SELECT 1 / 0;")
                    .await
                    .transaction
                    .status,
                QueryTransactionStatus::Failed
            );
            assert!(matches!(
                control("transaction-a", TransactionControl::Commit).await,
                Err(QuerySessionError::InvalidTransactionTransition {
                    status: QueryTransactionStatus::Failed,
                    ..
                })
            ));
            assert_eq!(
                control("transaction-a", TransactionControl::Recheck)
                    .await
                    .unwrap()
                    .status,
                QueryTransactionStatus::Failed
            );
            assert_eq!(
                control("transaction-a", TransactionControl::Rollback)
                    .await
                    .unwrap()
                    .status,
                QueryTransactionStatus::Idle
            );
            assert_eq!(
                control("transaction-b", TransactionControl::Recheck)
                    .await
                    .unwrap(),
                QueryTransactionSnapshot::default()
            );
            assert_eq!(
                control(
                    "transaction-a",
                    TransactionControl::Mode(QueryTransactionMode::Autocommit)
                )
                .await
                .unwrap()
                .mode,
                QueryTransactionMode::Autocommit
            );
            assert_eq!(first.query(backend, "SELECT 42;").await.status, "completed");
        })
    })
    .await;
}
