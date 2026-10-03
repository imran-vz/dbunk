use super::*;
use crate::backend::profile;

fn target(schema: &str, table: &str) -> ExportTarget {
    ExportTarget {
        connection_id: "removed-or-disconnected".into(),
        schema: schema.into(),
        table: table.into(),
    }
}
#[test]
fn configuration_bounds_preserve_exact_values() {
    assert!(target(" ", "雪.a").validate().is_ok());
    assert_ne!(target("a.b", "c"), target("a", "b.c"));
    assert!(target("a\0", "b").validate().is_err());
    let mut options = ExportOptions {
        null_token: "\n\0雪".into(),
        ..Default::default()
    };
    assert_eq!(
        serde_json::from_str::<ExportOptions>(&serde_json::to_string(&options).unwrap()).unwrap(),
        options
    );
    options.null_token.reserve(MAX_CONFIGURATION_HEAP_BYTES + 1);
    assert_eq!(options.validate(), Err(ExportConfigurationError::TooLarge));
    options.null_token = "x".repeat(MAX_NULL_TOKEN_BYTES + 1);
    assert_eq!(options.validate(), Err(ExportConfigurationError::TooLarge));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_latest_scope_and_stale_save_preserve_intervening_records() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let initial = backend.load_export_configurations().await.unwrap();
    assert!(initial.records().is_empty());
    let first = backend
        .save_export_configuration(
            target("a.b", "c"),
            ExportOptions::default(),
            initial.revision(),
        )
        .await
        .unwrap();
    let second = backend
        .save_export_configuration(
            target("a", "b.c"),
            ExportOptions {
                null_token: "NULL".into(),
                ..Default::default()
            },
            first.revision(),
        )
        .await
        .unwrap();
    let stale = backend
        .save_export_configuration(
            target("a.b", "c"),
            ExportOptions::default(),
            first.revision(),
        )
        .await
        .unwrap_err();
    assert_eq!(stale, ExportConfigurationError::StaleRevision);
    assert_eq!(
        second
            .latest(&target("a.b", "c"))
            .unwrap()
            .options
            .null_token,
        ""
    );
    assert_eq!(
        second
            .latest(&target("a", "b.c"))
            .unwrap()
            .options
            .null_token,
        "NULL"
    );
    let third = backend
        .save_export_configuration(
            target("a.b", "c"),
            ExportOptions {
                format: ExportFormat::Xlsx,
                null_token: "雪".into(),
                ..Default::default()
            },
            second.revision(),
        )
        .await
        .unwrap();
    assert_eq!(third.records().len(), 3);
    assert_eq!(
        third.latest(&target("a.b", "c")).unwrap().options.format,
        ExportFormat::Xlsx
    );
    assert_eq!(third.records()[2].id, first.records()[0].id);
    assert!(third.checked_heap_bytes().unwrap() <= MAX_CONFIGURATION_HEAP_BYTES);
    assert!(third.encoded_bytes() <= MAX_CONFIGURATION_BYTES);
    assert_eq!(
        backend
            .load_export_configurations()
            .await
            .unwrap()
            .revision(),
        third.revision()
    );
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_storage_is_not_absence_and_is_never_replaced() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let initial = backend.load_export_configurations().await.unwrap();
    for (raw, expected) in [
        ("{bad".to_owned(), ExportConfigurationError::Corrupt),
        (
            "{\"version\":2}".to_owned(),
            ExportConfigurationError::UnsupportedVersion,
        ),
        (
            "x".repeat(MAX_CONFIGURATION_BYTES + 1),
            ExportConfigurationError::TooLarge,
        ),
        (
            "{\"version\":1,\"revision\":null,\"configurations\":[]}".to_owned(),
            ExportConfigurationError::Corrupt,
        ),
    ] {
        sqlx::query("INSERT OR REPLACE INTO ui_state(key,value,updated_at) VALUES(?,?,'test')")
            .bind(storage::KEY)
            .bind(&raw)
            .execute(&backend.0.state.pool)
            .await
            .unwrap();
        assert_eq!(
            backend.load_export_configurations().await.unwrap_err(),
            expected
        );
        assert_eq!(
            backend
                .save_export_configuration(
                    target("a", "b"),
                    ExportOptions::default(),
                    initial.revision()
                )
                .await
                .unwrap_err(),
            expected
        );
        let preserved: String = sqlx::query_scalar("SELECT value FROM ui_state WHERE key=?")
            .bind(storage::KEY)
            .fetch_one(&backend.0.state.pool)
            .await
            .unwrap();
        assert_eq!(preserved, raw);
    }
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_limits_refuse_without_eviction_and_profiles_are_separate() {
    let directory = profile::directory();
    let other_directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let other = Backend::open_fixture(&other_directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let mut capture = backend.load_export_configurations().await.unwrap();
    for _ in 0..MAX_CONFIGURATIONS {
        capture = backend
            .save_export_configuration(
                target("a", "b"),
                ExportOptions::default(),
                capture.revision(),
            )
            .await
            .unwrap();
    }
    let first_id = capture.records()[MAX_CONFIGURATIONS - 1].id;
    assert_eq!(
        backend
            .save_export_configuration(
                target("a", "b"),
                ExportOptions::default(),
                capture.revision()
            )
            .await
            .unwrap_err(),
        ExportConfigurationError::Full
    );
    let after = backend.load_export_configurations().await.unwrap();
    assert_eq!(after.revision(), capture.revision());
    assert_eq!(after.records()[MAX_CONFIGURATIONS - 1].id, first_id);
    assert!(other
        .load_export_configurations()
        .await
        .unwrap()
        .records()
        .is_empty());
    backend.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn encoded_escape_expansion_refuses_before_commit() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let options = ExportOptions {
        null_token: "\0".repeat(MAX_NULL_TOKEN_BYTES),
        ..Default::default()
    };
    let mut capture = backend.load_export_configurations().await.unwrap();
    for _ in 0..5 {
        capture = backend
            .save_export_configuration(target("a", "b"), options.clone(), capture.revision())
            .await
            .unwrap();
    }
    assert_eq!(
        backend
            .save_export_configuration(target("a", "b"), options, capture.revision())
            .await
            .unwrap_err(),
        ExportConfigurationError::TooLarge
    );
    assert_eq!(
        backend
            .load_export_configurations()
            .await
            .unwrap()
            .revision(),
        capture.revision()
    );
    backend.shutdown().await.unwrap();
}
