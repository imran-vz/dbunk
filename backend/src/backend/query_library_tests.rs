use super::*;
use crate::backend::{profile, QuerySessionError};

fn saved(id: &str, favorite: bool) -> SavedQueryRecord {
    SavedQueryRecord {
        id: id.into(),
        name: "É漢字 draft".into(),
        body: "SELECT '東京';\n\n".into(),
        connection_id: Some("deleted-connection".into()),
        is_favorite: favorite,
        owner_id: Some("preserved-owner".into()),
        created_at: String::new(),
        updated_at: "ignored".into(),
    }
}
fn history(id: &str) -> HistoryRecord {
    HistoryRecord {
        id: id.into(),
        sql: "SELECT 'É漢字';\n".into(),
        connection_id: "deleted-connection".into(),
        connection_name: "Archived".into(),
        database: "demo".into(),
        engine: "PostgreSQL".into(),
        status: "success".into(),
        error_message: None,
        runtime_ms: 13,
        row_count: Some(7),
        started_at: "2026-10-03T00:00:00Z".into(),
    }
}
async fn backend() -> (tempfile::TempDir, Backend) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    (directory, backend)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_paging_preserves_exact_work_and_failed_write_does_not_replace_it() {
    let (_directory, backend) = backend().await;
    let first = backend
        .save_saved_query(saved("first", true))
        .await
        .unwrap();
    let second = backend
        .save_saved_query(saved("second", false))
        .await
        .unwrap();
    assert_eq!(first.body, "SELECT '東京';\n\n");
    assert_eq!(first.connection_id.as_deref(), Some("deleted-connection"));
    assert_eq!(first.created_at, first.updated_at);
    let request = LibraryRequest {
        limit: 1,
        ..Default::default()
    };
    let page = backend.load_saved_queries(request.clone()).await.unwrap();
    assert_eq!(page.entries[0].id, "first");
    let page = backend
        .load_saved_queries(LibraryRequest {
            cursor: page.next,
            ..request
        })
        .await
        .unwrap();
    assert_eq!(page.entries[0].id, second.id);
    assert!(page.next.is_none());
    let mut edit = first.clone();
    edit.name = "Renamed".into();
    edit.created_at = "different caller timestamp".into();
    edit.is_favorite = false;
    edit.connection_id = None;
    let edited = backend.save_saved_query(edit.clone()).await.unwrap();
    assert_eq!(edited.created_at, first.created_at);
    assert_eq!(edited.connection_id, None);
    assert_eq!(edited.owner_id, first.owner_id);
    sqlx::query("CREATE TRIGGER reject_saved BEFORE INSERT ON saved_queries BEGIN SELECT RAISE(ABORT,'sensitive literal must never escape'); END").execute(&backend.0.state.pool).await.unwrap();
    edit.body = "not saved".into();
    let error = backend.save_saved_query(edit).await.err().unwrap();
    assert!(matches!(error, LibraryError::Storage));
    assert!(!format!("{error:?} {error}").contains("sensitive literal"));
    let rows = backend
        .load_saved_queries(Default::default())
        .await
        .unwrap()
        .entries;
    assert_eq!(
        rows.iter().find(|row| row.id == "first").unwrap().body,
        edited.body
    );
    backend.delete_saved_query("first".into()).await.unwrap();
    backend.delete_saved_query("first".into()).await.unwrap();
    assert_eq!(
        backend
            .load_saved_queries(Default::default())
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_filters_unicode_tied_cursors_and_deletes_remain_profile_local() {
    let (_first, backend) = backend().await;
    let (_second, other) = self::backend().await;
    for id in ["a", "b", "c"] {
        backend.append_query_history(history(id)).await.unwrap();
    }
    let mut failed = history("d");
    failed.status = "error".into();
    failed.row_count = None;
    failed.error_message = Some("typed failure".into());
    backend.append_query_history(failed).await.unwrap();
    assert!(other
        .load_query_history(Default::default())
        .await
        .unwrap()
        .entries
        .is_empty());
    let request = LibraryRequest {
        limit: 1,
        search: " é漢字 ".into(),
        connection_id: Some("deleted-connection".into()),
        status: Some("success".into()),
        ..Default::default()
    };
    let mut next = None;
    let mut ids = Vec::new();
    loop {
        let page = backend
            .load_query_history(LibraryRequest {
                cursor: next,
                ..request.clone()
            })
            .await
            .unwrap();
        ids.extend(page.entries.into_iter().map(|entry| {
            assert_eq!(entry.row_count, Some(7));
            entry.id
        }));
        next = page.next;
        if next.is_none() {
            break;
        }
    }
    assert_eq!(ids, ["c", "b", "a"]);
    backend.delete_query_history("b".into()).await.unwrap();
    let errors = backend
        .load_query_history(LibraryRequest {
            status: Some("error".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(errors.entries[0].row_count, None);
    backend.clear_query_history().await.unwrap();
    assert!(backend
        .load_query_history(Default::default())
        .await
        .unwrap()
        .entries
        .is_empty());
    backend.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_retention_is_atomic_and_payload_or_legacy_row_limits_fail_explicitly() {
    let (_directory, backend) = backend().await;
    sqlx::query("WITH RECURSIVE rows(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM rows WHERE n < 1999) INSERT INTO query_history (id,sql,connection_id,connection_name,database_name,engine,status,runtime_ms,started_at) SELECT 'old-'||n,'SELECT 1','old','Old','demo','PostgreSQL','success',0,'2000' FROM rows").execute(&backend.0.state.pool).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_trim BEFORE DELETE ON query_history BEGIN SELECT RAISE(ABORT,'do not expose stored SQL'); END").execute(&backend.0.state.pool).await.unwrap();
    assert!(matches!(
        backend.append_query_history(history("new")).await,
        Err(LibraryError::Storage)
    ));
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM query_history WHERE id='new'")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(exists, 0);
    sqlx::query("DROP TRIGGER reject_trim")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    backend.append_query_history(history("new")).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM query_history")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(count, 2000);
    let mut oversized = saved("oversized", false);
    oversized.body = "x".repeat(1024 * 1024 + 1);
    assert!(matches!(
        backend.save_saved_query(oversized).await,
        Err(LibraryError::TooLarge)
    ));
    assert!(matches!(
        backend
            .load_query_history(LibraryRequest {
                limit: 201,
                ..Default::default()
            })
            .await,
        Err(LibraryError::InvalidInput(_))
    ));
    sqlx::query("INSERT INTO saved_queries (id,name,body,is_favorite,created_at,updated_at) VALUES ('legacy','legacy',CAST(zeroblob(3145728) AS TEXT),0,'old','old')").execute(&backend.0.state.pool).await.unwrap();
    assert!(matches!(
        backend.load_saved_queries(Default::default()).await,
        Err(LibraryError::TooLarge)
    ));
    backend.delete_saved_query("legacy".into()).await.unwrap();
    assert!(backend
        .load_saved_queries(Default::default())
        .await
        .unwrap()
        .entries
        .is_empty());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_admission_and_shutdown_fence_library_mutations() {
    let (_directory, backend) = backend().await;
    let permit = backend
        .0
        .admission
        .clone()
        .acquire_many_owned(16)
        .await
        .unwrap();
    assert!(matches!(
        backend.save_saved_query(saved("refused", false)).await,
        Err(LibraryError::Unavailable(QuerySessionError::Timeout { operation }))
            if operation == "nativeAdmission"
    ));
    drop(permit);
    assert!(backend
        .load_saved_queries(Default::default())
        .await
        .unwrap()
        .entries
        .is_empty());
    backend.shutdown().await.unwrap();
    assert!(matches!(
        backend.clear_query_history().await,
        Err(LibraryError::Unavailable(
            QuerySessionError::ConnectionClosing
        ))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scan_budget_and_encoded_byte_budget_keep_continuations_truthful() {
    let (_directory, backend) = backend().await;
    sqlx::query("WITH RECURSIVE rows(n) AS (SELECT 0 UNION ALL SELECT n + 1 FROM rows WHERE n < 2000) INSERT INTO saved_queries (id,name,body,is_favorite,created_at,updated_at) SELECT printf('%04d',n),CASE WHEN n=0 THEN 'needle' ELSE 'other' END,'SELECT 1',0,'old','old' FROM rows").execute(&backend.0.state.pool).await.unwrap();
    let first = backend
        .load_saved_queries(LibraryRequest {
            search: "needle".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(first.entries.is_empty());
    assert!(first.next.is_some());
    let last = backend
        .load_saved_queries(LibraryRequest {
            search: "needle".into(),
            cursor: first.next,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(last.entries[0].id, "0000");
    assert!(last.next.is_none());
    sqlx::query("DELETE FROM saved_queries")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    let mut escaped = saved("escaped-a", false);
    escaped.body = "\0".repeat(1024 * 1024);
    backend.save_saved_query(escaped.clone()).await.unwrap();
    escaped.id = "escaped-b".into();
    backend.save_saved_query(escaped).await.unwrap();
    let page = backend
        .load_saved_queries(Default::default())
        .await
        .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8 * 1024 * 1024);
    let next = backend
        .load_saved_queries(LibraryRequest {
            cursor: page.next,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(next.entries.len(), 1);
    assert_ne!(next.entries[0].id, page.entries[0].id);
    assert!(next.next.is_none());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escaped_continuation_is_included_in_the_complete_page_budget() {
    let (_directory, backend) = backend().await;
    let mut entry = history("a");
    entry.started_at = "\u{1}".repeat(8192);
    entry.sql.clear();
    let target = (8 * 1024 * 1024 - 18_000) / 4;
    let metadata_bytes = serde_json::to_vec(&entry).unwrap().len();
    entry.sql = "\0".repeat((target - metadata_bytes) / 6);
    for id in ["a", "b", "c", "d", "e"] {
        entry.id = id.into();
        backend.append_query_history(entry.clone()).await.unwrap();
    }
    let page = backend
        .load_query_history(Default::default())
        .await
        .unwrap();
    assert!(page.next.is_some());
    assert!(serde_json::to_vec(&page.next).unwrap().len() > 48 * 1024);
    assert!(serde_json::to_vec(&page).unwrap().len() <= 8 * 1024 * 1024);
    let mut ids = page
        .entries
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    let rest = backend
        .load_query_history(LibraryRequest {
            cursor: page.next,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(serde_json::to_vec(&rest).unwrap().len() <= 8 * 1024 * 1024);
    ids.extend(rest.entries.into_iter().map(|entry| entry.id));
    assert_eq!(ids, ["e", "d", "c", "b", "a"]);
    assert!(rest.next.is_none());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editor_save_preserves_current_organization_and_creation_atomically() {
    let (_directory, backend) = backend().await;
    let original = backend
        .save_saved_query(saved("editor-draft", true))
        .await
        .unwrap();
    let mut edit = original.clone();
    edit.body = "SELECT 'changed';\n".into();
    edit.name = "Edited title".into();
    edit.is_favorite = false;
    edit.owner_id = None;
    edit.created_at.clear();
    let changed = backend.save_query_draft(edit).await.unwrap();
    assert_eq!(changed.body, "SELECT 'changed';\n");
    assert_eq!(changed.name, "Edited title");
    assert!(changed.is_favorite);
    assert_eq!(changed.owner_id, original.owner_id);
    assert_eq!(changed.created_at, original.created_at);
    let record = HistoryRecord::started(
        "native-run".into(),
        "select 42".into(),
        "removed".into(),
        "Removed".into(),
        "db".into(),
    );
    backend.append_query_history(record).await.unwrap();
    backend.shutdown().await.unwrap();
}
