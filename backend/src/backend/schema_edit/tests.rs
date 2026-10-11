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
    SqliteSessionConfig {
        connection_id: "schema-edit-test".into(),
        path: path.into(),
        read_only,
        policy: resolve_policy(ConnectionPolicy {
            environment: Environment::Development,
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
