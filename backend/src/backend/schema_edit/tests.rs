use super::*;
use crate::backend::development::WorkspaceError;
use crate::backend::sqlite_session::SqliteSessionConfig;
use crate::safety::policy::resolve_policy;
use crate::schema_edit::fixtures::users;
use crate::{ConnectionPolicy, Environment, SafeMode};

#[test]
fn draft_journal_validates_its_request_against_the_baseline() {
    let baseline = users(SchemaEngine::Sqlite);
    let draft = SchemaDraft::from_baseline(&baseline);
    let request = SchemaEditRequest::new("main", "users");
    let record = WorkspaceSchemaEdit::draft(request, baseline.clone(), draft.clone()).unwrap();
    assert!(!record.outcome_unknown());
    let text = serde_json::to_string(&record).unwrap();
    assert_eq!(
        serde_json::from_str::<WorkspaceSchemaEdit>(&text).unwrap(),
        record
    );

    let other = SchemaEditRequest::new("main", "people");
    assert_eq!(
        WorkspaceSchemaEdit::draft(other, baseline, draft).unwrap_err(),
        WorkspaceError::InvalidSnapshot
    );
}

#[test]
fn journal_validates_preview_structure_but_not_its_rendering() {
    let baseline = users(SchemaEngine::Postgres);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("age", "integer");
    let forged = SchemaEditPreview {
        engine: SchemaEngine::Postgres,
        changes: diff(&baseline, &draft),
        statements: vec![PreviewStatement {
            sql: "DROP TABLE users".into(),
            summary: "Add column".into(),
            destructive: false,
        }],
        groups: vec![PreviewGroup::Standalone { statement: 0 }],
        notes: Vec::new(),
        options: PlanOptions::default(),
    };
    let record = WorkspaceSchemaEdit {
        engine: SchemaEngine::Postgres,
        request: SchemaEditRequest::new("public", "users"),
        baseline,
        draft,
        attempt: Some(WorkspaceSchemaEditAttempt {
            attempt_id: SchemaEditAttemptId::new(),
            preview: forged,
            apply_state: crate::backend::development::WorkspaceApplyState::OutcomeUnknown,
        }),
    };
    assert!(record.outcome_unknown());
    // A preview from an older planner still loads, so the unknown outcome
    // stays visible for reconciliation.
    assert_eq!(record.validate(), Ok(()));
    // Groups that do not cover the statements are refused.
    let mut broken = record.clone();
    broken.attempt.as_mut().unwrap().preview.groups.clear();
    assert_eq!(broken.validate(), Err(WorkspaceError::InvalidSnapshot));
    // So is a preview for another engine.
    let mut other = record;
    other.attempt.as_mut().unwrap().preview.engine = SchemaEngine::MySql;
    assert_eq!(other.validate(), Err(WorkspaceError::InvalidSnapshot));
}

fn session_config(path: &str, read_only: bool, safe_mode: SafeMode) -> SqliteSessionConfig {
    session_config_in(path, read_only, safe_mode, Environment::Development)
}

fn session_config_in(
    path: &str,
    read_only: bool,
    safe_mode: SafeMode,
    environment: Environment,
) -> SqliteSessionConfig {
    SqliteSessionConfig {
        connection_id: "schema-edit-test".into(),
        path: path.into(),
        read_only,
        policy: resolve_policy(ConnectionPolicy {
            environment,
            safe_mode,
            read_only,
        }),
        audit: None,
    }
}

async fn database(dir: &tempfile::TempDir) -> String {
    use sqlx::{sqlite::SqliteConnectOptions, ConnectOptions, Connection};
    let path = dir.path().join("edit.db");
    let mut connection = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL)")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn read_only_sqlite_session_refuses_apply_before_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = database(&dir).await;
    let session = SqliteSession::open(session_config(&path, true, SafeMode::Inherit))
        .await
        .unwrap();
    let baseline = users(SchemaEngine::Sqlite);
    let draft = SchemaDraft::from_baseline(&baseline);
    let preview = SchemaEditPreview {
        engine: SchemaEngine::Sqlite,
        changes: SchemaChangeSet::default(),
        statements: Vec::new(),
        groups: Vec::new(),
        notes: Vec::new(),
        options: PlanOptions::default(),
    };
    let result = session
        .apply_schema_edit(session.ticket(), baseline, draft, preview, false)
        .await
        .unwrap();
    assert!(matches!(result, SessionApply::Blocked(_)), "{result:?}");
    // Nothing changed on disk.
    let structure = session
        .structure(session.ticket(), "main".into(), "users".into())
        .await
        .unwrap();
    assert_eq!(structure.columns.len(), 2);
    session
        .close(std::time::Duration::from_secs(5))
        .await
        .unwrap();
}

async fn backend() -> (tempfile::TempDir, Backend) {
    let directory = crate::backend::profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    (directory, backend)
}

/// Observes `users` through the facade and reviews adding a column.
async fn review_add_column(backend: &Backend, session: &Arc<SqliteSession>) -> SchemaEditReview {
    let target = backend
        .observe_schema_edit(
            SchemaEditSource::Sqlite(session.clone()),
            SchemaEditRequest::new("main", "users"),
        )
        .await
        .unwrap();
    let mut draft = SchemaDraft::from_baseline(&target.baseline);
    draft.add_column("nickname", "TEXT");
    backend
        .review_schema_edit(target, draft, PlanOptions::default())
        .unwrap_or_else(|refusal| panic!("{:?}", refusal.error))
}

async fn column_count(session: &SqliteSession) -> usize {
    session
        .structure(session.ticket(), "main".into(), "users".into())
        .await
        .unwrap()
        .columns
        .len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_policy_needs_confirmation_before_dispatch_then_applies() {
    let (_profile, backend) = backend().await;
    for (safe_mode, environment) in [
        (SafeMode::Inherit, Environment::Production),
        (SafeMode::Strict, Environment::Development),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = database(&dir).await;
        let session = Arc::new(
            SqliteSession::open(session_config_in(&path, false, safe_mode, environment))
                .await
                .unwrap(),
        );
        let review = review_add_column(&backend, &session).await;
        let confirmation = match backend.apply_schema_edit(review).await.unwrap() {
            SchemaEditSubmission::NeedsConfirmation(confirmation) => confirmation,
            SchemaEditSubmission::Finished(receipt) => {
                panic!(
                    "{safe_mode:?}: applied without confirmation: {:?}",
                    receipt.outcome
                )
            }
        };
        assert_eq!(column_count(&session).await, 2, "{safe_mode:?}");
        match backend.confirm_schema_edit(*confirmation).await.unwrap() {
            SchemaEditSubmission::Finished(receipt) => {
                assert!(receipt.outcome.applied(), "{:?}", receipt.outcome)
            }
            SchemaEditSubmission::NeedsConfirmation(_) => panic!("confirmed apply asked again"),
        }
        assert_eq!(column_count(&session).await, 3);
        session
            .close(std::time::Duration::from_secs(5))
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_review_whose_preview_no_longer_renders_the_same_is_refused() {
    let (_profile, backend) = backend().await;
    let dir = tempfile::tempdir().unwrap();
    let path = database(&dir).await;
    let session = Arc::new(
        SqliteSession::open(session_config(&path, false, SafeMode::Inherit))
            .await
            .unwrap(),
    );
    let mut review = review_add_column(&backend, &session).await;
    review.preview.statements[0].sql = "DROP TABLE \"main\".\"users\"".into();
    assert!(matches!(
        backend.apply_schema_edit(review).await,
        Err(SchemaEditError::PreviewChanged)
    ));
    assert_eq!(column_count(&session).await, 2);
    session
        .close(std::time::Duration::from_secs(5))
        .await
        .unwrap();
}

#[test]
fn session_refusals_map_to_typed_errors() {
    assert_eq!(
        sqlite_error(SqliteSessionError::Busy),
        SchemaEditError::Busy
    );
    assert_eq!(
        mysql_error(MySqlSessionError::ReadOnly("read-only".into())),
        SchemaEditError::PolicyBlocked("read-only".into())
    );
    assert!(matches!(
        sqlite_error(SqliteSessionError::Closed),
        SchemaEditError::Unavailable(_)
    ));
}

#[test]
fn apply_errors_after_queueing_are_unknown_outcomes() {
    assert_eq!(
        sqlite_apply_error(SqliteSessionError::Busy),
        SchemaEditError::Busy
    );
    assert_eq!(
        sqlite_apply_error(SqliteSessionError::Closed),
        SchemaEditError::OutcomeUnavailable
    );
    assert_eq!(
        mysql_apply_error(MySqlSessionError::Closed(None)),
        SchemaEditError::OutcomeUnavailable
    );
}

/// Native spawns these on the tokio runtime and inside `BoxFuture`s, so they
/// must be `Send + 'static` for every engine. Compile-time only.
#[allow(dead_code, clippy::extra_unused_type_parameters)]
fn facade_futures_are_send() {
    fn send<T: Send + 'static>(_: T) {}
    fn check(
        backend: Backend,
        source: SchemaEditSource,
        review: SchemaEditReview,
        confirmation: SchemaEditConfirmation,
    ) {
        let request = SchemaEditRequest::new("s", "t");
        let b = backend.clone();
        send(async move { b.observe_schema_edit(source, request).await });
        let b = backend.clone();
        send(async move { b.apply_schema_edit(review).await });
        send(async move { backend.confirm_schema_edit(confirmation).await });
    }
    let _ = check;
}

async fn postgres_fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    // Synthetic profile only; the injected loader never opens a socket.
    let directory = crate::backend::profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("schema-edit-test", "one", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}

async fn postgres_policy(backend: &Backend, read_only: bool, mode: SafeMode) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        panic!("PostgreSQL")
    };
    pg.read_only = read_only;
    pg.safe_mode = mode;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}

fn postgres_review(backend: &Backend, document: &DataDocument) -> SchemaEditReview {
    let baseline = users(SchemaEngine::Postgres);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("nickname", "text");
    let target = SchemaEditTarget {
        source: SchemaEditSource::Postgres(document.clone()),
        request: SchemaEditRequest::new("public", "users"),
        baseline,
        generation: Some(*document.0.read_cancellation().borrow()),
    };
    backend
        .review_schema_edit(target, draft, PlanOptions::default())
        .unwrap_or_else(|refusal| panic!("{:?}", refusal.error))
}

/// A loader that records the call and finds no connection, so a permitted
/// apply ends `NotDispatched { Connection }` without a socket.
fn no_connection(
    called: Arc<std::sync::atomic::AtomicBool>,
) -> impl FnOnce(
    Arc<crate::app::AppState>,
    String,
) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
       + Send
       + 'static {
    move |_, _| {
        called.store(true, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async { None })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_policy_is_checked_before_the_connection_is_loaded() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (_directory, backend, document) = postgres_fixture().await;

    // Strict: confirmation first, and nothing is loaded until it is given.
    postgres_policy(&backend, false, SafeMode::Strict).await;
    let called = Arc::new(AtomicBool::new(false));
    let submission = backend
        .submit_postgres(
            document.clone(),
            postgres_review(&backend, &document),
            false,
            no_connection(called.clone()),
        )
        .await
        .unwrap();
    let SchemaEditSubmission::NeedsConfirmation(confirmation) = submission else {
        panic!("Strict applied without confirmation")
    };
    assert!(!called.load(Ordering::SeqCst));
    let submission = backend
        .submit_postgres(
            document.clone(),
            confirmation.review,
            true,
            no_connection(called.clone()),
        )
        .await
        .unwrap();
    let SchemaEditSubmission::Finished(receipt) = submission else {
        panic!("confirmed apply asked again")
    };
    assert!(called.load(Ordering::SeqCst));
    assert_eq!(
        receipt.outcome,
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Connection
        }
    );

    // Read-only: refused, confirmed or not, before anything is loaded.
    postgres_policy(&backend, true, SafeMode::Inherit).await;
    for confirmed in [false, true] {
        let called = Arc::new(AtomicBool::new(false));
        let result = backend
            .submit_postgres(
                document.clone(),
                postgres_review(&backend, &document),
                confirmed,
                no_connection(called.clone()),
            )
            .await;
        assert!(
            matches!(result, Err(SchemaEditError::PolicyBlocked(_))),
            "read-only, confirmed {confirmed}"
        );
        assert!(!called.load(Ordering::SeqCst));
    }

    // Development with no safe mode: straight to the loader.
    postgres_policy(&backend, false, SafeMode::Disabled).await;
    let called = Arc::new(AtomicBool::new(false));
    let result = backend
        .submit_postgres(
            document.clone(),
            postgres_review(&backend, &document),
            false,
            no_connection(called.clone()),
        )
        .await
        .unwrap();
    assert!(matches!(result, SchemaEditSubmission::Finished(_)));
    assert!(called.load(Ordering::SeqCst));
}
