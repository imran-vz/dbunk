use super::*;
use crate::backend::development::workspace::{
    decode, encode, load, save,
    tests::{pool, raw},
    WorkspaceSnapshot,
};
use crate::backend::table_seed::{TableSeedConnection, TableSeedEndpoint};
fn journal() -> WorkspaceTableSeed {
    WorkspaceTableSeed {
        attempt_id: TableSeedAttemptId::new(),
        description: TableSeedDescription {
            endpoint: TableSeedEndpoint {
                connection_id: "removed-connection".into(),
                schema: "資料".into(),
                table: "target".into(),
            },
            connection: TableSeedConnection {
                connection_name: "Owned test".into(),
                host: "localhost".into(),
                port: 5432,
                database: "db".into(),
                user: "user".into(),
                environment: "Development".into(),
                safe_mode: "Protected".into(),
                read_only: false,
            },
            database_oid: 1,
            relation_oid: 2,
            row_count: 100,
            seed_used: u64::MAX,
            clock_epoch_seconds: 1_700_000_000,
            recipe_sha256: "a".repeat(64),
            recipe_summary: "value: constant 雪".into(),
            recipe_summary_truncated: false,
            inserted_columns: 1,
            defaulted_columns: 2,
        },
        state: WorkspaceTableSeedState::Applying,
        failure: None,
        diagnostic: None,
    }
}
fn snapshot(job: WorkspaceTableSeed) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        seed_jobs: vec![job],
        ..Default::default()
    }
}
#[tokio::test]
async fn exact_seed_recovery_preserves_storage_but_never_replays_applying() {
    let pool = pool().await;
    let job = journal();
    save(&pool, None, snapshot(job.clone())).await.unwrap();
    let before = raw(&pool).await;
    let stored: serde_json::Value = serde_json::from_str(&before).unwrap();
    assert_eq!(stored["version"], 15);
    assert_eq!(
        stored["snapshot"]["seedJobs"][0]["description"]["seedUsed"],
        u64::MAX
    );
    let loaded = load(&pool).await.unwrap().snapshot.unwrap();
    let mut expected = job;
    expected.state = WorkspaceTableSeedState::Unknown;
    assert_eq!(loaded, snapshot(expected));
    assert_eq!(raw(&pool).await, before);
    assert!(loaded.documents.is_empty());
}
#[test]
fn old_versions_and_unknown_fields_cannot_silently_discard_seed_intent() {
    let mut current: serde_json::Value =
        serde_json::from_str(&encode(snapshot(journal())).unwrap()).unwrap();
    for version in 1..=10 {
        current["version"] = version.into();
        assert_eq!(
            decode(&current.to_string()).unwrap_err(),
            WorkspaceError::Corrupt
        );
    }
    current["version"] = 11.into();
    current["snapshot"]["seedJobs"][0]["description"]["future"] = true.into();
    assert_eq!(
        decode(&current.to_string()).unwrap_err(),
        WorkspaceError::Corrupt
    );
    let empty = WorkspaceSnapshot::default();
    let mut legacy: serde_json::Value =
        serde_json::from_str(&encode(empty.clone()).unwrap()).unwrap();
    for version in 1..=10 {
        legacy["version"] = version.into();
        assert_eq!(decode(&legacy.to_string()).unwrap(), empty);
    }
}
#[test]
fn journal_bounds_include_diagnostics_duplicates_and_truthful_committed_rows() {
    let mut job = journal();
    job.state = WorkspaceTableSeedState::Completed { rows: 100 };
    job.failure = Some(TableSeedError::Cleanup);
    job.diagnostic = Some(TableSeedDiagnostic {
        sqlstate: Some("08006".into()),
        constraint: Some("constraint".into()),
        column: Some("value".into()),
        parent_schema: Some("資料".into()),
        parent_table: Some("parent".into()),
    });
    let encoded = encode(snapshot(job.clone())).unwrap();
    assert_eq!(decode(&encoded).unwrap(), snapshot(job.clone()));
    job.state = WorkspaceTableSeedState::Completed { rows: 101 };
    assert!(encode(snapshot(job.clone())).is_err());
    job.state = WorkspaceTableSeedState::Unknown;
    job.diagnostic.as_mut().unwrap().constraint = Some("x".repeat(64));
    assert!(encode(snapshot(job.clone())).is_err());
    job.diagnostic = None;
    job.description.recipe_summary = "x".repeat(8193);
    assert!(encode(snapshot(job)).is_err());
    let job = journal();
    let duplicate = WorkspaceSnapshot {
        seed_jobs: vec![job.clone(), job],
        ..Default::default()
    };
    assert!(encode(duplicate).is_err());
}
