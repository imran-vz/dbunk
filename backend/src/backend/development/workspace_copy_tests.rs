use super::*;
use crate::backend::development::workspace::{
    decode, encode, load, save,
    tests::{pool, raw, seed},
    WorkspaceRevision, WorkspaceSnapshot, NATIVE_WORKSPACE_MAX_BYTES,
};
use crate::backend::table_copy::{
    TableCopyConnection, TableCopyEndpoint, TableCopyIntent, TableCopyRelation, TableCopySide,
};

fn journal() -> WorkspaceTableCopy {
    let connection = TableCopyConnection {
        connection_name: "exact connection 資料".into(),
        host: "localhost".into(),
        port: 5432,
        database: "database".into(),
        user: "user".into(),
        environment: "Development".into(),
        safe_mode: "Protected".into(),
        read_only: false,
    };
    WorkspaceTableCopy {
        attempt_id: TableCopyAttemptId::new(),
        description: TableCopyDescription {
            intent: TableCopyIntent {
                source: TableCopyEndpoint {
                    connection_id: "removed-source-connection".into(),
                    schema: "exact \" schema".into(),
                    table: "資料 source".into(),
                },
                destination: TableCopyEndpoint {
                    connection_id: "removed-destination-connection".into(),
                    schema: "public".into(),
                    table: "destination".into(),
                },
            },
            source_connection: connection.clone(),
            destination_connection: connection,
            source_relation: TableCopyRelation {
                database_oid: 1,
                relation_oid: 2,
                kind: "r".into(),
            },
            destination_relation: TableCopyRelation {
                database_oid: 3,
                relation_oid: 4,
                kind: "p".into(),
            },
            mapping_sha256: "a".repeat(64),
            copied_columns: 2,
            defaulted_columns: 1,
            generated_columns: 1,
            identity_columns: 1,
        },
        state: WorkspaceTableCopyState::Staged,
        failure: None,
        diagnostic: None,
    }
}
fn snapshot(job: WorkspaceTableCopy) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        copy_jobs: vec![job],
        ..Default::default()
    }
}

#[tokio::test]
async fn global_copy_journal_survives_no_tabs_and_restores_applying_as_unknown_without_writing() {
    let pool = pool().await;
    let mut job = journal();
    let mut revision = None;
    for state in [
        WorkspaceTableCopyState::Staged,
        WorkspaceTableCopyState::Applying,
        WorkspaceTableCopyState::Unknown,
        WorkspaceTableCopyState::Completed { rows: u64::MAX },
        WorkspaceTableCopyState::RolledBack,
        WorkspaceTableCopyState::NotStarted,
        WorkspaceTableCopyState::Reconciled,
    ] {
        job.state = state;
        revision = Some(save(&pool, revision, snapshot(job.clone())).await.unwrap());
        let original = raw(&pool).await;
        let stored: serde_json::Value = serde_json::from_str(&original).unwrap();
        assert_eq!(stored["version"], 16);
        if state == WorkspaceTableCopyState::Applying {
            assert_eq!(stored["snapshot"]["copyJobs"][0]["state"], "applying");
        }
        let loaded = load(&pool).await.unwrap().snapshot.unwrap();
        let mut expected = job.clone();
        expected.restore();
        assert_eq!(loaded, snapshot(expected));
        assert!(loaded.documents.is_empty());
        assert_eq!(raw(&pool).await, original);
    }
    job.state = WorkspaceTableCopyState::Unknown;
    job.failure = Some(TableCopyError::Database);
    job.diagnostic = Some(TableCopyDiagnostic {
        side: TableCopySide::Destination,
        sqlstate: Some("08006".into()),
        field_limit: false,
        record_limit: true,
    });
    let input = snapshot(job);
    assert_eq!(decode(&encode(input.clone()).unwrap()).unwrap(), input);
    let mut completed_cleanup = input;
    completed_cleanup.copy_jobs[0].state = WorkspaceTableCopyState::Completed { rows: 3 };
    completed_cleanup.copy_jobs[0].failure = Some(TableCopyError::Cleanup);
    assert_eq!(
        decode(&encode(completed_cleanup.clone()).unwrap()).unwrap(),
        completed_cleanup
    );
}

#[tokio::test]
async fn copy_unknown_future_and_pre10_fields_preserve_original_storage() {
    let pool = pool().await;
    let base: serde_json::Value =
        serde_json::from_str(&encode(snapshot(journal())).unwrap()).unwrap();
    for defect in 0..8 {
        let mut value = base.clone();
        match defect {
            0 => value["version"] = 9.into(),
            1 => {
                value["version"] = 9.into();
                value["snapshot"]["copyJobs"] = serde_json::json!([]);
            }
            2 => value["version"] = 17.into(),
            3 => value["snapshot"]["copyJobs"][0]["reviewToken"] = "not-authority".into(),
            4 => value["snapshot"]["copyJobs"][0]["state"] = "replay".into(),
            5 => value["snapshot"]["copyJobs"][0]["attemptId"] = "invalid".into(),
            6 => {
                value["snapshot"]["copyJobs"][0]["description"]["sourceRelation"]["sql"] =
                    "select 1".into()
            }
            _ => {
                value["snapshot"]["copyJobs"][0]["state"] =
                    serde_json::json!({"completed":{"rows":1,"extra":true}})
            }
        }
        let original = value.to_string();
        seed(&pool, &original).await;
        assert!(load(&pool).await.is_err(), "defect {defect}");
        assert!(save(
            &pool,
            Some(WorkspaceRevision("seed".into())),
            WorkspaceSnapshot::default()
        )
        .await
        .is_err());
        assert_eq!(raw(&pool).await, original);
    }
    for version in 1..=9 {
        let mut value = base.clone();
        value["version"] = version.into();
        value["snapshot"]
            .as_object_mut()
            .unwrap()
            .remove("copyJobs");
        assert!(decode(&value.to_string()).unwrap().copy_jobs.is_empty());
    }
}

#[test]
fn copy_rejects_duplicate_attempts_invalid_states_and_unbounded_capacities() {
    let job = journal();
    let duplicate = WorkspaceSnapshot {
        copy_jobs: vec![job.clone(), job.clone()],
        ..Default::default()
    };
    assert_eq!(encode(duplicate), Err(WorkspaceError::InvalidSnapshot));
    let mut jobs: Vec<_> = (0..17).map(|_| journal()).collect();
    assert_eq!(
        encode(WorkspaceSnapshot {
            copy_jobs: jobs.clone(),
            ..Default::default()
        }),
        Err(WorkspaceError::InvalidSnapshot)
    );
    jobs.clear();
    assert_eq!(
        encode(WorkspaceSnapshot {
            copy_jobs: jobs,
            ..Default::default()
        }),
        Err(WorkspaceError::InvalidSnapshot)
    );
    for state in [
        WorkspaceTableCopyState::Staged,
        WorkspaceTableCopyState::Applying,
        WorkspaceTableCopyState::Completed { rows: 0 },
    ] {
        let mut changed = job.clone();
        changed.state = state;
        changed.failure = Some(TableCopyError::Database);
        assert!(changed.checked_heap_bytes().is_none());
    }
    let mut changed = job.clone();
    changed.description.intent.source.table = "界".repeat(22);
    assert!(changed.validate().is_err());
    let mut changed = job.clone();
    changed
        .description
        .mapping_sha256
        .reserve(MAX_TABLE_COPY_DESCRIPTION_BYTES);
    assert!(changed.checked_heap_bytes().is_none());
    let mut changed = job;
    changed.state = WorkspaceTableCopyState::Unknown;
    changed.diagnostic = Some(TableCopyDiagnostic {
        side: TableCopySide::Source,
        sqlstate: Some("bad".into()),
        field_limit: false,
        record_limit: false,
    });
    assert!(changed.validate().is_err());
}

#[test]
fn copy_description_escape_expansion_stays_bounded_and_workspace_limit_is_independent() {
    let mut job = journal();
    for connection in [
        &mut job.description.source_connection,
        &mut job.description.destination_connection,
    ] {
        for text in [
            &mut connection.connection_name,
            &mut connection.host,
            &mut connection.database,
            &mut connection.user,
        ] {
            *text = "\u{1}".repeat(256);
        }
    }
    for endpoint in [
        &mut job.description.intent.source,
        &mut job.description.intent.destination,
    ] {
        endpoint.connection_id = "\u{1}".repeat(128);
        endpoint.schema = "\u{1}".repeat(63);
        endpoint.table = "\u{1}".repeat(63);
    }
    let heap = job.description.checked_heap_bytes().unwrap();
    let encoded = serde_json::to_vec(&job.description).unwrap().len();
    // Fixed environment/safe-mode labels make even maximally escaped variable
    // fields fit the description cap, while encoding still exceeds heap size.
    assert!(encoded > heap);
    assert!(encoded <= MAX_TABLE_COPY_DESCRIPTION_BYTES);
    assert!(job.checked_heap_bytes().is_some());
    let mut input = super::super::tests::snapshot();
    input.copy_jobs.push(journal());
    input.documents[0].sql = "x".repeat(NATIVE_WORKSPACE_MAX_BYTES);
    assert_eq!(encode(input), Err(WorkspaceError::TooLarge));
}
