use super::*;
use crate::backend::profile;
use crate::types::StoredConnection;

const CONNECTION: &str = profile::CONNECTION_ID;
const TIME: &str = "2026-10-03T00:00:00Z";

async fn backend() -> (tempfile::TempDir, Backend) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    (directory, backend)
}
async fn seed(backend: &Backend, connection: &str, count: usize, classes: &str) {
    let mut transaction = backend.0.state.pool.begin().await.unwrap();
    for _ in 0..count {
        sqlx::query("INSERT INTO safety_overrides(connection_id,command,classes,occurred_at) VALUES(?, 'apply_object_ddl', ?, ?)")
            .bind(connection).bind(classes).bind(TIME).execute(&mut *transaction).await.unwrap();
    }
    transaction.commit().await.unwrap();
}
async fn other_connection(backend: &Backend) {
    let mut connection = crate::storage::read_connections(&backend.0.state.pool)
        .await
        .unwrap()
        .remove(0);
    let StoredConnection::PostgreSQL(connection) = &mut connection else {
        panic!("fixture PostgreSQL")
    };
    connection.id = "other".into();
    crate::storage::upsert_connection(
        &backend.0.state.pool,
        &StoredConnection::PostgreSQL(connection.clone()),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuation_is_exact_stable_and_profile_bound() {
    let (_directory, backend) = backend().await;
    other_connection(&backend).await;
    seed(&backend, CONNECTION, 205, "[\"ddl\",\"ddl\",\"unknown\"]").await;
    seed(&backend, "other", 3, "[]").await;
    let first = backend
        .load_safety_audit(CONNECTION.into(), None)
        .await
        .unwrap();
    assert_eq!(first.limit, Some(SafetyAuditLimit::RowLimit));
    assert_eq!(first.rows.first().unwrap().id, 205);
    assert_eq!(first.rows.last().unwrap().id, 106);
    assert_eq!(
        first.rows[0].classes,
        [
            SafetyAuditClass::Ddl,
            SafetyAuditClass::Ddl,
            SafetyAuditClass::Unknown
        ]
    );
    assert!(first.checked_heap_bytes().unwrap() <= MAX_AUDIT_PAGE_BYTES);
    let cursor = first.next_cursor.unwrap();
    assert!(cursor.checked_heap_bytes().unwrap() <= MAX_AUDIT_CURSOR_BYTES);
    assert_eq!(
        backend
            .load_safety_audit("other".into(), Some(cursor.clone()))
            .await
            .unwrap_err(),
        SafetyAuditError::ForeignCursor
    );
    let (_second_directory, second_backend) = self::backend().await;
    assert_eq!(
        second_backend
            .load_safety_audit(CONNECTION.into(), Some(cursor.clone()))
            .await
            .unwrap_err(),
        SafetyAuditError::ForeignCursor
    );
    // Later inserts, including a backdated row, cannot enter this continuation.
    seed(&backend, CONNECTION, 1, "[]").await;
    sqlx::query("UPDATE safety_overrides SET occurred_at='2020-01-01T00:00:00Z' WHERE id=(SELECT max(id) FROM safety_overrides)").execute(&backend.0.state.pool).await.unwrap();
    let second = backend
        .load_safety_audit(CONNECTION.into(), Some(cursor))
        .await
        .unwrap();
    assert_eq!((second.rows[0].id, second.rows[99].id), (105, 6));
    let last = backend
        .load_safety_audit(CONNECTION.into(), second.next_cursor)
        .await
        .unwrap();
    assert_eq!(
        last.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        [5, 4, 3, 2, 1]
    );
    assert!(last.next_cursor.is_none());
    assert!(last.limit.is_none());
    let other = backend
        .load_safety_audit("other".into(), None)
        .await
        .unwrap();
    assert_eq!(other.rows.len(), 3);
    assert!(other.rows.iter().all(|row| row.classes.is_empty()));
    second_backend.shutdown().await.unwrap();
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_retention_can_end_a_continuation_with_an_empty_page() {
    let (_directory, backend) = backend().await;
    other_connection(&backend).await;
    seed(&backend, CONNECTION, 101, "[]").await;
    let first = backend
        .load_safety_audit(CONNECTION.into(), None)
        .await
        .unwrap();
    seed(&backend, "other", 1000, "[]").await;
    crate::storage::insert_safety_override(
        &backend.0.state.pool,
        "other",
        "apply_object_ddl",
        &["ddl".into()],
    )
    .await
    .unwrap();
    let remaining = backend
        .load_safety_audit(CONNECTION.into(), first.next_cursor)
        .await
        .unwrap();
    assert!(remaining.rows.is_empty());
    assert!(remaining.next_cursor.is_none());
    assert!(remaining.checked_heap_bytes().is_some());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(count, i64::from(SAFETY_AUDIT_GLOBAL_RETENTION));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encoded_byte_limit_continues_without_skipping_a_row() {
    let (_directory, backend) = backend().await;
    let classes = serde_json::to_string(&vec!["transaction"; MAX_AUDIT_CLASSES]).unwrap();
    seed(&backend, CONNECTION, 100, &classes).await;
    let first = backend
        .load_safety_audit(CONNECTION.into(), None)
        .await
        .unwrap();
    assert_eq!(first.limit, Some(SafetyAuditLimit::ByteLimit));
    assert!(first.rows.len() < 100);
    assert!(first.encoded_bytes().unwrap() <= MAX_AUDIT_PAGE_BYTES);
    let mut ids: Vec<_> = first.rows.iter().map(|row| row.id).collect();
    let second = backend
        .load_safety_audit(CONNECTION.into(), first.next_cursor)
        .await
        .unwrap();
    assert!(second.next_cursor.is_none());
    ids.extend(second.rows.iter().map(|row| row.id));
    assert_eq!(ids, (1..=100).rev().collect::<Vec<_>>());
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persisted_corruption_is_refused_without_leaking_its_values() {
    let (_directory, backend) = backend().await;
    seed(&backend, CONNECTION, 1, "[]").await;
    for classes in [
        "null".to_string(),
        "{}".into(),
        "[\"secret-invalid-class\"]".into(),
        "[\"DDL\"]".into(),
        serde_json::to_string(&vec!["ddl"; MAX_AUDIT_CLASSES + 1]).unwrap(),
    ] {
        sqlx::query("UPDATE safety_overrides SET classes=?")
            .bind(classes)
            .execute(&backend.0.state.pool)
            .await
            .unwrap();
        let error = backend
            .load_safety_audit(CONNECTION.into(), None)
            .await
            .unwrap_err();
        assert_eq!(error, SafetyAuditError::Corrupt);
        assert!(!format!("{error:?} {error}").contains("secret-invalid-class"));
    }
    sqlx::query("UPDATE safety_overrides SET classes='[]'")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    for (value, expected) in [
        (
            "x".repeat(MAX_AUDIT_COMMAND_BYTES + 1),
            SafetyAuditError::TooLarge,
        ),
        ("secret\0command".into(), SafetyAuditError::Corrupt),
    ] {
        sqlx::query("UPDATE safety_overrides SET command=?")
            .bind(value)
            .execute(&backend.0.state.pool)
            .await
            .unwrap();
        assert_eq!(
            backend
                .load_safety_audit(CONNECTION.into(), None)
                .await
                .unwrap_err(),
            expected
        );
    }
    sqlx::query("UPDATE safety_overrides SET command=x'736563726574'")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(
        backend
            .load_safety_audit(CONNECTION.into(), None)
            .await
            .unwrap_err(),
        SafetyAuditError::Corrupt
    );
    sqlx::query("UPDATE safety_overrides SET command='ok',occurred_at='not a timestamp'")
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(
        backend
            .load_safety_audit(CONNECTION.into(), None)
            .await
            .unwrap_err(),
        SafetyAuditError::Corrupt
    );
    sqlx::query("UPDATE safety_overrides SET occurred_at=?,classes=?")
        .bind(TIME)
        .bind(" ".repeat(types::MAX_CLASSES_JSON_BYTES + 1))
        .execute(&backend.0.state.pool)
        .await
        .unwrap();
    assert_eq!(
        backend
            .load_safety_audit(CONNECTION.into(), None)
            .await
            .unwrap_err(),
        SafetyAuditError::TooLarge
    );
    backend.shutdown().await.unwrap();
}

#[test]
fn public_capture_validation_accounts_for_capacities_order_and_redacts() {
    let row = SafetyAuditRow {
        id: 1,
        command: "secret-command".into(),
        classes: vec![SafetyAuditClass::Ddl],
        occurred_at: TIME.into(),
    };
    let mut page = SafetyAuditPage {
        connection_id: CONNECTION.into(),
        rows: vec![row],
        next_cursor: None,
        limit: None,
    };
    assert!(page.checked_heap_bytes().is_some());
    assert!(!format!("{page:?} {:?}", page.rows[0]).contains("secret-command"));
    page.rows[0].command = "é".repeat(MAX_AUDIT_COMMAND_BYTES / 2);
    assert!(page.checked_heap_bytes().is_some());
    page.rows[0].command.reserve(MAX_AUDIT_COMMAND_BYTES);
    assert!(page.checked_heap_bytes().is_none());
    page.rows[0].command = "ok".into();
    page.rows.push(page.rows[0].clone());
    assert!(page.checked_heap_bytes().is_none());
    page.rows.pop();
    page.rows.reserve(MAX_AUDIT_PAGE_ROWS + 1);
    assert!(page.checked_heap_bytes().is_none());
}
