use super::*;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub(super) async fn pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().in_memory(true))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE ui_state (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)")
        .execute(&pool).await.unwrap();
    pool
}

pub(super) fn snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        copy_jobs: Vec::new(),
        seed_jobs: Vec::new(),
        documents: vec![
            WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                tool: None,
                saved_query_id: None,
                id: "query-1".into(),
                name: "Recovered SQL".into(),
                connection_id: Some("deleted-connection".into()),
                sql: "SELECT '東京🙂';\n-- exact draft  \n".into(),
                pinned: true,
                table: None,
                selection: WorkspaceSelection {
                    anchor: 2,
                    head: 20,
                },
            },
            WorkspaceDocument {
                query_changes: None,
                schema_changes: None,
                table_ddl: None,
                schema_alter: None,
                object_ddl: None,
                admin_control: None,
                maintenance: None,
                tool: None,
                saved_query_id: None,
                id: "query-2".into(),
                name: "Second".into(),
                connection_id: None,
                sql: String::new(),
                pinned: false,
                table: None,
                selection: WorkspaceSelection::default(),
            },
        ],
        active_document_id: Some("query-1".into()),
        layout: Layout::SideBySide,
        density: WorkspaceDensity::Compact,
        navigator_width: 320.0,
        shell: None,
    }
}

pub(super) async fn seed(pool: &SqlitePool, value: &str) {
    sqlx::query("INSERT OR REPLACE INTO ui_state VALUES (?, ?, 'seed')")
        .bind(KEY)
        .bind(value)
        .execute(pool)
        .await
        .unwrap();
}

pub(super) async fn raw(pool: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT value FROM ui_state WHERE key = ?")
        .bind(KEY)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn schema_map_tab_round_trips_only_identity_and_old_workspaces_remain_read_only() {
    let pool = pool().await;
    let mut input = snapshot();
    let tab = &mut input.documents[0];
    tab.tool = Some(WorkspaceTool::SchemaMap);
    tab.sql.clear();
    tab.selection = WorkspaceSelection::default();
    save(&pool, None, input.clone()).await.unwrap();
    let encoded = raw(&pool).await;
    let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["version"], 16);
    assert_eq!(load(&pool).await.unwrap().snapshot, Some(input));
    assert_eq!(raw(&pool).await, encoded);
    for version in 1..=11 {
        value["version"] = version.into();
        assert_eq!(decode(&value.to_string()), Err(WorkspaceError::Corrupt));
    }
    let legacy = snapshot();
    let mut value: serde_json::Value =
        serde_json::from_str(&encode(legacy.clone()).unwrap()).unwrap();
    for version in 1..=11 {
        value["version"] = version.into();
        let encoded = value.to_string();
        seed(&pool, &encoded).await;
        assert_eq!(load(&pool).await.unwrap().snapshot, Some(legacy.clone()));
        assert_eq!(raw(&pool).await, encoded);
    }
}

#[tokio::test]
async fn shell_state_round_trips_and_pre_v16_records_load_with_defaults() {
    let pool = pool().await;
    let mut input = snapshot();
    input.shell = Some(WorkspaceShell {
        sidebar_collapsed: true,
        status_bar_collapsed: true,
        project: Some("Payments 東京".into()),
        environment: Some(crate::backend::DevelopmentEnvironment::Staging),
    });
    save(&pool, None, input.clone()).await.unwrap();
    let encoded = raw(&pool).await;
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["version"], 16);
    assert_eq!(value["snapshot"]["shell"]["environment"], "staging");
    assert_eq!(
        load(&pool).await.unwrap().snapshot,
        Some(validate(input).unwrap())
    );

    // A version-15 record has no shell state: it loads with the defaults and
    // the stored bytes are not upgraded by loading.
    let mut old = value.clone();
    old["version"] = 15.into();
    old["snapshot"].as_object_mut().unwrap().remove("shell");
    let old = old.to_string();
    seed(&pool, &old).await;
    let loaded = load(&pool).await.unwrap().snapshot.unwrap();
    assert_eq!(loaded.shell, None);
    assert_eq!(loaded.documents, validate(snapshot()).unwrap().documents);
    assert_eq!(raw(&pool).await, old);

    // An older envelope cannot carry the newer field, and unknown shell
    // fields are refused rather than dropped.
    let mut forged = value.clone();
    forged["version"] = 15.into();
    assert_eq!(decode(&forged.to_string()), Err(WorkspaceError::Corrupt));
    let mut future = value.clone();
    future["snapshot"]["shell"]["pinnedPanel"] = true.into();
    assert_eq!(decode(&future.to_string()), Err(WorkspaceError::Corrupt));
    let mut empty = value;
    empty["snapshot"]["shell"]["project"] = "".into();
    assert_eq!(decode(&empty.to_string()), Err(WorkspaceError::Corrupt));
}

#[tokio::test]
async fn exact_drafts_order_binding_and_geometry_round_trip_without_connections() {
    let pool = pool().await;
    sqlx::query("INSERT INTO ui_state VALUES ('ui.v1.session', 'react state', 'now')")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        load(&pool).await.unwrap(),
        WorkspaceLoad {
            revision: None,
            snapshot: None
        }
    );
    let expected = validate(snapshot()).unwrap();
    let revision = save(&pool, None, snapshot()).await.unwrap();
    assert_eq!(
        load(&pool).await.unwrap(),
        WorkspaceLoad {
            revision: Some(revision),
            snapshot: Some(expected)
        }
    );
    let react: String =
        sqlx::query_scalar("SELECT value FROM ui_state WHERE key = 'ui.v1.session'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(react, "react state");
}

#[tokio::test]
async fn corrupt_and_future_records_are_preserved_until_explicit_reset() {
    let pool = pool().await;
    for (value, expected) in [
        ("broken", WorkspaceError::Corrupt),
        (
            r#"{"version":17,"snapshot":{"future":"data"}}"#,
            WorkspaceError::UnsupportedVersion(17),
        ),
        (r#"{"version":1,"snapshot":{}}"#, WorkspaceError::Corrupt),
    ] {
        seed(&pool, value).await;
        assert_eq!(load(&pool).await.unwrap_err(), expected);
        assert_eq!(
            save(&pool, Some(WorkspaceRevision("seed".into())), snapshot())
                .await
                .unwrap_err(),
            expected
        );
        let mut conn = pool.acquire().await.unwrap();
        assert_eq!(
            read_record(&mut conn, crate::storage::UI_STATE_MAX_VALUE_BYTES)
                .await
                .unwrap()
                .unwrap()
                .0,
            value
        );
        drop(conn);
        assert_eq!(raw(&pool).await, value);
    }
    let revision = reset(&pool).await.unwrap();
    assert_eq!(
        load(&pool).await.unwrap(),
        WorkspaceLoad {
            revision: Some(revision),
            snapshot: Some(WorkspaceSnapshot::default())
        }
    );
}

#[tokio::test]
async fn oversized_unicode_and_json_expansion_preserve_last_committed_draft() {
    let pool = pool().await;
    let revision = save(&pool, None, snapshot()).await.unwrap();
    let last_saved = raw(&pool).await;
    for sql in [
        "🙂".repeat(NATIVE_WORKSPACE_MAX_BYTES / 4),
        "\0".repeat(NATIVE_WORKSPACE_MAX_BYTES / 2),
    ] {
        let mut oversized = snapshot();
        oversized.documents[0].sql = sql.clone();
        assert_eq!(
            save(&pool, Some(revision.clone()), oversized.clone())
                .await
                .unwrap_err(),
            WorkspaceError::TooLarge
        );
        assert_eq!(oversized.documents[0].sql, sql);
        assert_eq!(raw(&pool).await, last_saved);
    }
    seed(&pool, &"x".repeat(NATIVE_WORKSPACE_MAX_BYTES + 1)).await;
    assert_eq!(load(&pool).await.unwrap_err(), WorkspaceError::TooLarge);
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        read_record(&mut conn, crate::storage::UI_STATE_MAX_VALUE_BYTES)
            .await
            .unwrap()
            .unwrap()
            .0
            .len(),
        NATIVE_WORKSPACE_MAX_BYTES + 1
    );
}

#[tokio::test]
async fn encoded_budget_accepts_exact_limit_and_rejects_one_more_byte() {
    let pool = pool().await;
    let mut value = snapshot();
    value.documents[0].sql.clear();
    value.documents[0].selection = WorkspaceSelection::default();
    let overhead = encode(value.clone()).unwrap().len();
    value.documents[0].sql = "a".repeat(NATIVE_WORKSPACE_MAX_BYTES - overhead);
    assert_eq!(
        encode(value.clone()).unwrap().len(),
        NATIVE_WORKSPACE_MAX_BYTES
    );
    let revision = save(&pool, None, value.clone()).await.unwrap();
    assert_eq!(load(&pool).await.unwrap().snapshot, Some(value.clone()));
    value.documents[0].sql.push('b');
    assert_eq!(
        save(&pool, Some(revision), value).await.unwrap_err(),
        WorkspaceError::TooLarge
    );
}

#[test]
fn selection_and_geometry_are_clamped_without_changing_sql() {
    let mut input = snapshot();
    input.documents[0].sql = "a🙂東京".into();
    input.documents[0].selection = WorkspaceSelection {
        anchor: 3,
        head: usize::MAX,
    };
    input.navigator_width = f32::NAN;
    let output = validate(input.clone()).unwrap();
    assert_eq!(output.documents[0].sql, input.documents[0].sql);
    assert_eq!(
        output.documents[0].selection,
        WorkspaceSelection {
            anchor: 1,
            head: 11
        }
    );
    assert_eq!(output.navigator_width, 240.0);
    input.navigator_width = 9000.0;
    assert_eq!(validate(input).unwrap().navigator_width, 480.0);
}

#[tokio::test]
async fn older_writers_and_pre_reset_writers_cannot_overwrite_newer_commits() {
    let pool = pool().await;
    let first = save(&pool, None, snapshot()).await.unwrap();
    let mut second = snapshot();
    second.documents.reverse();
    second.documents[1].sql = "newest draft".into();
    let newer = save(&pool, Some(first.clone()), second.clone())
        .await
        .unwrap();
    assert_eq!(
        save(&pool, Some(first), snapshot()).await.unwrap_err(),
        WorkspaceError::StaleRevision
    );
    assert_eq!(
        load(&pool).await.unwrap().snapshot,
        Some(validate(second).unwrap())
    );
    reset(&pool).await.unwrap();
    assert_eq!(
        save(&pool, Some(newer), snapshot()).await.unwrap_err(),
        WorkspaceError::StaleRevision
    );
    assert_eq!(
        save(&pool, None, snapshot()).await.unwrap_err(),
        WorkspaceError::StaleRevision
    );
}

#[tokio::test]
async fn sqlite_failure_does_not_advance_revision_and_retry_succeeds() {
    let pool = pool().await;
    let revision = save(&pool, None, snapshot()).await.unwrap();
    let last_saved = raw(&pool).await;
    sqlx::query("CREATE TRIGGER reject_save BEFORE UPDATE ON ui_state BEGIN SELECT RAISE(ABORT, 'injected failure'); END")
        .execute(&pool).await.unwrap();
    assert_eq!(
        save(&pool, Some(revision.clone()), WorkspaceSnapshot::default())
            .await
            .unwrap_err(),
        WorkspaceError::Storage
    );
    assert_eq!(raw(&pool).await, last_saved);
    assert_eq!(load(&pool).await.unwrap().revision, Some(revision.clone()));
    sqlx::query("DROP TRIGGER reject_save")
        .execute(&pool)
        .await
        .unwrap();
    save(&pool, Some(revision), WorkspaceSnapshot::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn concurrent_saves_commit_once_and_survive_disk_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(directory.path().join("workspace.sqlite"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options.clone())
        .await
        .unwrap();
    sqlx::query("CREATE TABLE ui_state (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL)")
        .execute(&pool).await.unwrap();
    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(synchronous, 2);
    let mut other = snapshot();
    other.documents[0].sql = "SELECT 'other';".into();
    let (first, second) = tokio::join!(save(&pool, None, snapshot()), save(&pool, None, other));
    assert!(matches!(
        (&first, &second),
        (Ok(_), Err(WorkspaceError::StaleRevision)) | (Err(WorkspaceError::StaleRevision), Ok(_))
    ));
    let committed = load(&pool).await.unwrap();
    pool.close().await;
    let reopened = SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .unwrap();
    assert_eq!(load(&reopened).await.unwrap(), committed);
    reopened.close().await;
}

#[test]
fn document_identity_and_admission_limits_reject_invalid_snapshots() {
    let mut value = snapshot();
    value.documents[1].id = value.documents[0].id.clone();
    assert_eq!(
        validate(value).unwrap_err(),
        WorkspaceError::InvalidSnapshot
    );
    let mut value = snapshot();
    value.active_document_id = Some("missing".into());
    assert_eq!(
        validate(value).unwrap_err(),
        WorkspaceError::InvalidSnapshot
    );
    let mut value = snapshot();
    value.documents = (0..17)
        .map(|index| {
            let mut document = value.documents[0].clone();
            document.id = format!("query-{index}");
            document
        })
        .collect();
    assert_eq!(
        validate(value).unwrap_err(),
        WorkspaceError::InvalidSnapshot
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stage03_facade_cannot_read_write_export_or_reset_native_workspace() {
    let directory = crate::backend::profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    assert_eq!(
        backend.load_development_workspace().await.unwrap_err(),
        WorkspaceError::NotDevelopmentProfile
    );
    assert_eq!(
        backend
            .save_development_workspace(None, snapshot())
            .await
            .unwrap_err(),
        WorkspaceError::NotDevelopmentProfile
    );
    assert_eq!(
        backend.export_development_workspace().await.unwrap_err(),
        WorkspaceError::NotDevelopmentProfile
    );
    assert_eq!(
        backend.reset_development_workspace(true).await.unwrap_err(),
        WorkspaceError::NotDevelopmentProfile
    );
    backend.shutdown().await.unwrap();
}

pub(super) fn table_state() -> WorkspaceTableState {
    use crate::result_mutation::protocol::{
        MutationIdentityKind, MutationOp, MutationTable, MutationValue,
    };
    use crate::table_browse::protocol::{
        BrowseFilter, BrowseNulls, BrowseSortDirection, BrowseSortKey,
    };
    let old = MutationValue {
        column: "name".into(),
        value: None,
    };
    let key = MutationValue {
        column: "id".into(),
        value: Some("42".into()),
    };
    WorkspaceTableState {
        schema: "Mixed.Schema".into(),
        table: "東京".into(),
        filters: vec![
            BrowseFilter::IsNull {
                column: "name".into(),
            },
            BrowseFilter::RawSql {
                text: "id > 0".into(),
            },
        ],
        sort: vec![BrowseSortKey {
            column: "name".into(),
            direction: BrowseSortDirection::Desc,
            nulls: BrowseNulls::Last,
        }],
        page_size: 50,
        draft: Some(WorkspaceMutationDraft {
            apply_state: WorkspaceApplyState::OutcomeUnknown,
            changes: vec![WorkspaceStagedChange {
                id: uuid::Uuid::new_v4().to_string(),
                included: true,
                identity_kind: Some(MutationIdentityKind::PrimaryKey),
                originals: vec![key.clone(), old.clone()],
                operation: MutationOp::Update {
                    table: MutationTable {
                        schema: "Mixed.Schema".into(),
                        table: "東京".into(),
                    },
                    identity: vec![key],
                    guards: vec![old],
                    set: vec![MutationValue {
                        column: "name".into(),
                        value: Some("🙂 exact\n\t".into()),
                    }],
                },
            }],
        }),
    }
}

#[tokio::test]
async fn v3_table_and_sql_drafts_round_trip_exactly_with_uncertain_outcome_and_cas() {
    let pool = pool().await;
    let mut input = snapshot();
    input.documents[0].table = Some(table_state());
    let revision = save(&pool, None, input.clone()).await.unwrap();
    let encoded: serde_json::Value = serde_json::from_str(&raw(&pool).await).unwrap();
    assert_eq!(encoded["version"], 16);
    let table = &encoded["snapshot"]["documents"][0]["table"];
    assert_eq!(table["draft"]["applyState"], "outcomeUnknown");
    assert_eq!(
        table["draft"]["changes"][0]["originals"][1]["value"],
        serde_json::Value::Null
    );
    assert!(table.get("analysisId").is_none());
    assert!(table.get("sessionId").is_none());
    assert_eq!(
        load(&pool).await.unwrap().snapshot,
        Some(validate(input.clone()).unwrap())
    );
    let mut newer = input.clone();
    newer.documents[0].table.as_mut().unwrap().page_size = 100;
    save(&pool, Some(revision.clone()), newer.clone())
        .await
        .unwrap();
    assert_eq!(
        save(&pool, Some(revision), input).await.unwrap_err(),
        WorkspaceError::StaleRevision
    );
    assert_eq!(
        load(&pool).await.unwrap().snapshot,
        Some(validate(newer).unwrap())
    );
}

#[tokio::test]
async fn sql_only_v1_load_is_read_only_and_next_explicit_save_upgrades_to_v16() {
    let pool = pool().await;
    let mut value: serde_json::Value = serde_json::from_str(&encode(snapshot()).unwrap()).unwrap();
    value["version"] = 1.into();
    let legacy = serde_json::to_string(&value).unwrap();
    seed(&pool, &legacy).await;
    let loaded = load(&pool).await.unwrap();
    assert_eq!(raw(&pool).await, legacy);
    assert!(loaded
        .snapshot
        .as_ref()
        .unwrap()
        .documents
        .iter()
        .all(|doc| doc.table.is_none()));
    save(&pool, loaded.revision, loaded.snapshot.unwrap())
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw(&pool).await).unwrap()["version"],
        16
    );
}

#[tokio::test]
async fn malformed_or_future_table_intent_remains_exportable_without_automatic_overwrite() {
    let pool = pool().await;
    let mut input = snapshot();
    input.documents[0].table = Some(table_state());
    let valid: serde_json::Value = serde_json::from_str(&encode(input.clone()).unwrap()).unwrap();
    let mut variants = Vec::new();
    let mut v1 = valid.clone();
    v1["version"] = 1.into();
    variants.push(v1);
    let mut guards = valid.clone();
    guards["snapshot"]["documents"][0]["table"]["draft"]["changes"][0]["operation"]["guards"] =
        serde_json::json!([]);
    variants.push(guards);
    let mut authority = valid.clone();
    authority["snapshot"]["documents"][0]["table"]["draft"]["changes"][0]["operation"]
        ["confirmed"] = true.into();
    variants.push(authority);
    let mut filters = valid.clone();
    filters["snapshot"]["documents"][0]["table"]["filters"][0]["futurePredicate"] = true.into();
    variants.push(filters);
    let mut state = valid;
    state["snapshot"]["documents"][0]["table"]["draft"]["applyState"] = "futureOutcome".into();
    variants.push(state);
    for value in variants {
        let original = serde_json::to_string(&value).unwrap();
        seed(&pool, &original).await;
        assert_eq!(load(&pool).await.unwrap_err(), WorkspaceError::Corrupt);
        assert_eq!(
            save(&pool, Some(WorkspaceRevision("seed".into())), input.clone())
                .await
                .unwrap_err(),
            WorkspaceError::Corrupt
        );
        let mut conn = pool.acquire().await.unwrap();
        assert_eq!(
            read_record(&mut conn, crate::storage::UI_STATE_MAX_VALUE_BYTES)
                .await
                .unwrap()
                .unwrap()
                .0,
            original
        );
        drop(conn);
        assert_eq!(raw(&pool).await, original);
    }
}

#[tokio::test]
async fn aggregate_budget_refuses_oversized_staged_values_without_losing_saved_drafts() {
    let pool = pool().await;
    let mut input = snapshot();
    input.documents[0].table = Some(table_state());
    let revision = save(&pool, None, input.clone()).await.unwrap();
    let original = raw(&pool).await;
    let draft = input.documents[0]
        .table
        .as_mut()
        .unwrap()
        .draft
        .as_mut()
        .unwrap();
    let crate::result_mutation::protocol::MutationOp::Update { set, .. } =
        &mut draft.changes[0].operation
    else {
        panic!()
    };
    set[0].value = Some("\0🙂".repeat(NATIVE_WORKSPACE_MAX_BYTES / 5));
    assert_eq!(
        save(&pool, Some(revision), input).await.unwrap_err(),
        WorkspaceError::TooLarge
    );
    assert_eq!(raw(&pool).await, original);
}

#[test]
fn tool_documents_round_trip_and_reject_mixed_document_intent() {
    let mut snapshot = WorkspaceSnapshot {
        active_document_id: Some("history".into()),
        ..Default::default()
    };
    snapshot.documents.push(WorkspaceDocument {
        query_changes: None,
        schema_changes: None,
        table_ddl: None,
        schema_alter: None,
        object_ddl: None,
        admin_control: None,
        maintenance: None,
        id: "history".into(),
        name: "History".into(),
        connection_id: None,
        sql: String::new(),
        pinned: true,
        selection: Default::default(),
        table: None,
        tool: Some(WorkspaceTool::History),
        saved_query_id: None,
    });
    assert_eq!(
        decode(&encode(snapshot.clone()).unwrap()).unwrap(),
        snapshot
    );
    snapshot.documents[0].connection_id = Some("owned-connection".into());
    for tool in [WorkspaceTool::Objects, WorkspaceTool::Administration] {
        snapshot.documents[0].tool = Some(tool);
        assert_eq!(
            decode(&encode(snapshot.clone()).unwrap()).unwrap(),
            snapshot
        );
    }
    snapshot.documents[0].sql = "select 1".into();
    assert!(matches!(
        encode(snapshot),
        Err(WorkspaceError::InvalidSnapshot)
    ));
}

fn schema_snapshot(comment: Option<&str>) -> WorkspaceSnapshot {
    use crate::backend::schema_ddl::{CreateSchemaAttemptId, CreateSchemaIntent};
    let mut input = snapshot();
    let document = &mut input.documents[0];
    document.sql.clear();
    document.selection = WorkspaceSelection::default();
    document.tool = Some(WorkspaceTool::Objects);
    document.schema_changes = Some(WorkspaceSchemaChanges {
        attempt_id: CreateSchemaAttemptId::new(),
        intent: CreateSchemaIntent::new(" Exact \"東京\" ".into(), comment.map(str::to_owned))
            .unwrap(),
        apply_state: WorkspaceApplyState::OutcomeUnknown,
    });
    input
}

#[tokio::test]
async fn schema_journal_round_trips_exact_attempt_comment_and_unknown_state_without_replay_fields()
{
    let pool = pool().await;
    let mut revision = None;
    for comment in [None, Some(""), Some("Exact 'quoted'\n東京🙂  ")] {
        let input = schema_snapshot(comment);
        let changes = input.documents[0].schema_changes.as_ref().unwrap();
        let attempt = changes.attempt_id.as_str().to_owned();
        assert!(!format!("{changes:?}").contains("東京"));
        revision = Some(save(&pool, revision, input.clone()).await.unwrap());
        let encoded = raw(&pool).await;
        let json: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(json["version"], 16);
        assert_eq!(
            json["snapshot"]["documents"][0]["schemaChanges"]["attemptId"],
            attempt
        );
        assert_eq!(load(&pool).await.unwrap().snapshot, Some(input));
        assert_eq!(
            raw(&pool).await,
            encoded,
            "loading never rewrites or dispatches"
        );
        for executable_field in ["reviewId", "confirmation", "statements", "sessionId"] {
            assert!(!encoded.contains(executable_field));
        }
    }
}

#[tokio::test]
async fn old_future_and_corrupt_schema_journals_preserve_their_original_bytes() {
    let pool = pool().await;
    let base: serde_json::Value =
        serde_json::from_str(&encode(schema_snapshot(Some("exact"))).unwrap()).unwrap();
    let mut variants = Vec::new();
    for version in 1..=3 {
        let mut value = base.clone();
        value["version"] = version.into();
        variants.push((value, WorkspaceError::Corrupt));
    }
    let mut future = base.clone();
    future["version"] = 17.into();
    variants.push((future, WorkspaceError::UnsupportedVersion(17)));
    for attempt in [
        "not-a-uuid",
        "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
        "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
    ] {
        let mut value = base.clone();
        value["snapshot"]["documents"][0]["schemaChanges"]["attemptId"] = attempt.into();
        variants.push((value, WorkspaceError::Corrupt));
    }
    for (field, invalid) in [
        ("name", " ".to_owned()),
        ("name", "界".repeat(22)),
        ("comment", "x".repeat(4097)),
        ("comment", "a\0b".into()),
    ] {
        let mut value = base.clone();
        value["snapshot"]["documents"][0]["schemaChanges"]["intent"][field] = invalid.into();
        variants.push((value, WorkspaceError::Corrupt));
    }
    let mut executable = base;
    executable["snapshot"]["documents"][0]["schemaChanges"]["confirmed"] = true.into();
    variants.push((executable, WorkspaceError::Corrupt));
    for (value, expected) in variants {
        let original = serde_json::to_string(&value).unwrap();
        seed(&pool, &original).await;
        assert_eq!(load(&pool).await.unwrap_err(), expected);
        assert_eq!(
            save(
                &pool,
                Some(WorkspaceRevision("seed".into())),
                schema_snapshot(None)
            )
            .await
            .unwrap_err(),
            expected
        );
        assert_eq!(raw(&pool).await, original);
    }
}

#[test]
fn schema_recovery_requires_an_objects_connection_and_rejects_mixed_document_intent() {
    for kind in [
        None,
        Some(WorkspaceTool::History),
        Some(WorkspaceTool::SavedQueries),
        Some(WorkspaceTool::Administration),
    ] {
        let mut input = schema_snapshot(None);
        input.documents[0].tool = kind;
        assert_eq!(encode(input).unwrap_err(), WorkspaceError::InvalidSnapshot);
    }
    for defect in 0..5 {
        let mut input = schema_snapshot(Some(""));
        let document = &mut input.documents[0];
        match defect {
            0 => document.connection_id = None,
            1 => document.connection_id = Some(String::new()),
            2 => document.table = Some(table_state()),
            3 => document.saved_query_id = Some("saved-query".into()),
            _ => document.sql = "CREATE SCHEMA forbidden_runtime_sql".into(),
        }
        assert_eq!(encode(input).unwrap_err(), WorkspaceError::InvalidSnapshot);
    }
    let mut staged = schema_snapshot(None);
    staged.documents[0]
        .schema_changes
        .as_mut()
        .unwrap()
        .apply_state = WorkspaceApplyState::Staged;
    assert_eq!(decode(&encode(staged.clone()).unwrap()).unwrap(), staged);
}

#[tokio::test]
async fn schema_recovery_keeps_the_existing_aggregate_byte_and_document_budgets() {
    let pool = pool().await;
    let mut input = schema_snapshot(Some(&"\u{1}".repeat(4096)));
    let revision = save(&pool, None, input.clone()).await.unwrap();
    let original = raw(&pool).await;
    input.documents[1].sql = "x".repeat(NATIVE_WORKSPACE_MAX_BYTES);
    assert_eq!(
        save(&pool, Some(revision), input).await.unwrap_err(),
        WorkspaceError::TooLarge
    );
    assert_eq!(raw(&pool).await, original);
    let mut input = schema_snapshot(None);
    input.documents = (0..=NATIVE_WORKSPACE_MAX_DOCUMENTS)
        .map(|index| {
            let mut document = input.documents[0].clone();
            document.id = format!("schema-{index}");
            document
        })
        .collect();
    input.active_document_id = Some("schema-0".into());
    assert_eq!(encode(input).unwrap_err(), WorkspaceError::InvalidSnapshot);
}

#[tokio::test]
async fn backup_restore_tab_persists_only_identity_and_refuses_old_or_extended_records() {
    let pool = pool().await;
    let mut input = snapshot();
    let document = &mut input.documents[0];
    document.sql.clear();
    document.selection = WorkspaceSelection::default();
    document.tool = Some(WorkspaceTool::BackupRestore);
    let encoded = encode(input.clone()).unwrap();
    assert_eq!(decode(&encoded).unwrap(), input);
    let base: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(base["version"], 16);
    for defect in 0..3 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 4.into(),
            1 => {
                value["snapshot"]["documents"][0]["sourcePath"] =
                    "/private/tmp/should-not-persist".into()
            }
            _ => value["snapshot"]["documents"][0]["jobId"] = "must-not-replay".into(),
        }
        let encoded = serde_json::to_string(&value).unwrap();
        seed(&pool, &encoded).await;
        assert!(load(&pool).await.is_err());
        assert!(
            save(&pool, Some(WorkspaceRevision("seed".into())), snapshot())
                .await
                .is_err()
        );
        assert_eq!(raw(&pool).await, encoded);
    }
    input.documents[0].saved_query_id = Some("not-a-query".into());
    assert!(encode(input).is_err());
}

#[tokio::test]
async fn csv_transfer_tab_refuses_persisted_source_samples_or_execution() {
    let pool = pool().await;
    let mut input = snapshot();
    let document = &mut input.documents[0];
    document.sql.clear();
    document.selection = WorkspaceSelection::default();
    document.tool = Some(WorkspaceTool::CsvTransfer);
    let encoded = encode(input.clone()).unwrap();
    assert_eq!(decode(&encoded).unwrap(), input);
    let base: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(base["version"], 16);
    for defect in 0..4 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 5.into(),
            1 => {
                value["snapshot"]["documents"][0]["sourcePath"] =
                    "/private/tmp/transient.csv".into()
            }
            2 => value["snapshot"]["documents"][0]["samples"] = serde_json::json!([["transient"]]),
            _ => value["snapshot"]["documents"][0]["attemptId"] = "must-not-replay".into(),
        }
        let encoded = serde_json::to_string(&value).unwrap();
        seed(&pool, &encoded).await;
        assert!(load(&pool).await.is_err());
        assert!(
            save(&pool, Some(WorkspaceRevision("seed".into())), snapshot())
                .await
                .is_err()
        );
        assert_eq!(raw(&pool).await, encoded);
    }
    input.documents[0].saved_query_id = Some("not-a-query".into());
    assert!(encode(input).is_err());
}

#[test]
fn comparison_tabs_persist_only_identity_and_binding() {
    let mut input = snapshot();
    input.documents[0].sql.clear();
    input.documents[0].tool = Some(WorkspaceTool::SchemaCompare);
    input.documents[0].selection = WorkspaceSelection::default();
    let encoded = encode(input.clone()).unwrap();
    assert_eq!(decode(&encoded).unwrap(), input);
    let base: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    for field in ["source", "target", "requestId", "jobId", "resultId", "page"] {
        let mut value = base.clone();
        value["snapshot"]["documents"][0][field] = "transient".into();
        assert!(matches!(
            decode(&value.to_string()),
            Err(WorkspaceError::Corrupt)
        ));
    }
    let mut older = base;
    older["version"] = 6.into();
    assert!(matches!(
        decode(&older.to_string()),
        Err(WorkspaceError::Corrupt)
    ));
    input.documents[0].saved_query_id = Some("not-a-query".into());
    assert!(encode(input).is_err());
}

fn admin_snapshot() -> WorkspaceSnapshot {
    let mut input = snapshot();
    let document = &mut input.documents[0];
    document.sql.clear();
    document.selection = WorkspaceSelection::default();
    document.tool = Some(WorkspaceTool::Administration);
    document.admin_control = Some(WorkspaceAdminControl {
        attempt_id: uuid::Uuid::new_v4().to_string(),
        action: WorkspaceAdminAction::CancelQuery,
        pid: 12345,
        backend_start: "2026-10-03T01:02:03.123456Z".into(),
        query_start: Some("2026-10-03T01:03:03.123456Z".into()),
        database: None,
        apply_state: WorkspaceApplyState::OutcomeUnknown,
    });
    input
}
#[tokio::test]
async fn admin_recovery_roundtrips_unknown_identity_and_preserves_invalid_records() {
    let pool = pool().await;
    let input = admin_snapshot();
    save(&pool, None, input.clone()).await.unwrap();
    assert_eq!(load(&pool).await.unwrap().snapshot, Some(input.clone()));
    let base: serde_json::Value = serde_json::from_str(&raw(&pool).await).unwrap();
    for defect in 0..9 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 7.into(),
            1 => value["snapshot"]["documents"][0]["connectionId"] = serde_json::Value::Null,
            2 => value["snapshot"]["documents"][0]["tool"] = "objects".into(),
            3 => value["snapshot"]["documents"][0]["adminControl"]["confirmation"] = true.into(),
            4 => value["snapshot"]["documents"][0]["adminControl"]["pid"] = 0.into(),
            5 => value["snapshot"]["documents"][0]["adminControl"]["attemptId"] = "invalid".into(),
            6 => {
                value["snapshot"]["documents"][0]["adminControl"]["backendStart"] = "unknown".into()
            }
            7 => {
                value["snapshot"]["documents"][0]["adminControl"]["database"] =
                    "x".repeat(1025).into()
            }
            _ => value["snapshot"]["documents"][0]["adminControl"]["queryStart"] = "unknown".into(),
        }
        let original = value.to_string();
        seed(&pool, &original).await;
        assert_eq!(load(&pool).await.unwrap_err(), WorkspaceError::Corrupt);
        assert_eq!(
            save(&pool, Some(WorkspaceRevision("seed".into())), input.clone())
                .await
                .unwrap_err(),
            WorkspaceError::Corrupt
        );
        assert_eq!(raw(&pool).await, original);
    }
    // Existing version-seven tabs still load, without fabricating recovery.
    let mut old = base;
    old["version"] = 7.into();
    old["snapshot"]["documents"][0]
        .as_object_mut()
        .unwrap()
        .remove("adminControl");
    assert!(decode(&old.to_string()).unwrap().documents[0]
        .admin_control
        .is_none());
}
#[test]
fn admin_receipt_matching_requires_every_observed_identity_component() {
    use crate::backend::admin::{AdminControlAction, AdminControlTarget};
    let mut journal = admin_snapshot().documents[0].admin_control.take().unwrap();
    let target = AdminControlTarget::test_target();
    journal.database = target.database().map(str::to_owned);
    let attempt = journal.attempt_id.clone();
    assert!(journal.matches(&attempt, AdminControlAction::CancelQuery, &target));
    for defect in 0..6 {
        let mut changed = journal.clone();
        match defect {
            0 => changed.attempt_id = uuid::Uuid::new_v4().to_string(),
            1 => changed.action = WorkspaceAdminAction::TerminateSession,
            2 => changed.pid += 1,
            3 => changed.backend_start = "2026-10-03T02:02:03.123456Z".into(),
            4 => changed.query_start = None,
            _ => changed.database = None,
        }
        assert!(!changed.matches(&attempt, AdminControlAction::CancelQuery, &target));
    }
}
