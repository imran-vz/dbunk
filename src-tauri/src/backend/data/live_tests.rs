//! Opt-in public-facade acceptance against the verified disposable stage03 server.
//! Every database write is confined to one newly created UUID-named schema.
use super::*;
use futures_util::FutureExt;
use tokio_postgres::{Client, NoTls};

const WAIT: Duration = Duration::from_secs(10);

async fn fixture_count() -> u64 {
    tokio::task::spawn_blocking(|| {
        let output = std::process::Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native/fixture.py"))
            .arg("count")
            .output()
            .expect("run owned fixture verification");
        assert!(
            output.status.success(),
            "owned fixture verification failed: {}",
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

fn browse(schema: &str, page: u32) -> BrowseTableDataPayload {
    BrowseTableDataPayload {
        // The document must replace both caller-controlled routing fields.
        connection_id: "forged-connection".into(),
        tab_id: "forged-tab".into(),
        request_id: u64::from(page),
        schema: schema.into(),
        table: "rows".into(),
        filters: vec![BrowseFilter::Comparison {
            column: "id".into(),
            operator: ComparisonOperator::Gte,
            value: "2".into(),
        }],
        sort: vec![BrowseSortKey {
            column: "id".into(),
            direction: BrowseSortDirection::Desc,
            nulls: BrowseNulls::Default,
        }],
        page_request: BrowsePageRequest::Offset { page },
        page_size: 2,
        count_policy: BrowseCountPolicy::None,
        refresh_structure: false,
    }
}

async fn analyze(backend: &Backend, document: &DataDocument, schema: &str, request_id: u64) -> u64 {
    let analysis = backend
        .analyze_result(
            document,
            AnalyzeResultSetPayload {
                connection_id: "forged-connection".into(),
                tab_id: "forged-tab".into(),
                request_id,
                source: AnalyzeSource::Relation {
                    schema: schema.into(),
                    table: "rows".into(),
                },
                refresh_structure: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(analysis.statement, AnalysisStatement::Analyzed);
    assert_eq!(
        analysis.tables[0].identity.kind,
        MutationIdentityKind::PrimaryKey
    );
    assert!(analysis.tables[0].updatable.allowed);
    analysis.analysis_id
}

fn update(schema: &str, before: &str, after: &str) -> MutationPlan {
    let value = |column: &str, text: &str| MutationValue {
        column: column.into(),
        value: Some(text.into()),
    };
    MutationPlan {
        operations: vec![MutationOp::Update {
            table: MutationTable {
                schema: schema.into(),
                table: "rows".into(),
            },
            identity: vec![value("id", "1")],
            guards: vec![value("body", before)],
            set: vec![value("body", after)],
        }],
    }
}

async fn row(client: &Client, schema: &str) -> String {
    client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .unwrap()
        .get(0)
}

// Deliberately preserve the admitted document while changing only its disposable
// stored policy, proving confirmation rechecks policy independently of teardown.
async fn policy(backend: &Backend, read_only: bool) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        unreachable!()
    };
    pg.safe_mode = crate::SafeMode::Strict;
    pg.read_only = read_only;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}

async fn challenge(
    backend: &Backend,
    review: MutationReview,
    request: u64,
) -> MutationConfirmation {
    match backend.apply_review(review, request).await.unwrap() {
        MutationSubmission::NeedsConfirmation(confirmation) => *confirmation,
        MutationSubmission::Applied(_) => panic!("strict write executed before confirmation"),
    }
}

async fn exercise(backend: &Backend, foreign: &Backend, client: &Client, schema: &str) {
    let document = backend
        .open_data_document("data-live", "table", &backend.fixture().id)
        .await
        .unwrap();
    let first = backend
        .browse_table(&document, browse(schema, 1))
        .await
        .unwrap();
    assert_eq!(
        first
            .rows
            .iter()
            .map(|r| r[0].as_deref())
            .collect::<Vec<_>>(),
        [Some("5"), Some("4")]
    );
    assert!(first.page_info.has_more);
    let second = backend
        .browse_table(&document, browse(schema, 2))
        .await
        .unwrap();
    assert_eq!(
        second
            .rows
            .iter()
            .map(|r| r[0].as_deref())
            .collect::<Vec<_>>(),
        [Some("3"), Some("2")]
    );
    assert!(!second.page_info.has_more);
    let count = backend
        .count_table(
            &document,
            CountTableBrowseRowsPayload {
                connection_id: "forged-connection".into(),
                tab_id: "forged-tab".into(),
                request_id: 3,
                schema: schema.into(),
                table: "rows".into(),
                filters: browse(schema, 1).filters,
            },
        )
        .await
        .unwrap();
    assert_eq!((count.kind, count.value), (BrowseCountKind::Exact, 4));
    let prefs = TableGridPrefs(serde_json::json!({"version": 1, "pageSize": 2}));
    backend
        .save_table_preferences(&document, schema.into(), "rows".into(), prefs.clone())
        .await
        .unwrap();
    assert_eq!(
        backend
            .load_table_preferences(&document, schema.into(), "rows".into())
            .await
            .unwrap(),
        Some(prefs)
    );
    assert!(matches!(
        foreign.browse_table(&document, browse(schema, 1)).await,
        Err(DataError::Document(_))
    ));

    policy(backend, false).await;
    let analysis = analyze(backend, &document, schema, 4).await;
    let review = backend
        .review_mutations(&document, analysis, update(schema, "before", "after"))
        .await
        .unwrap();
    assert_eq!(review.preview().statements.len(), 1);
    let expected_preview = review.preview().clone();
    let confirmation = challenge(backend, review, 5).await;
    assert_eq!(confirmation.preview(), &expected_preview);
    assert!(!confirmation.statements().is_empty());
    assert_eq!(row(client, schema).await, "before");
    match backend.confirm_mutations(confirmation).await.unwrap() {
        MutationSubmission::Applied(applied) => assert_eq!(applied.operations[0].rows_affected, 1),
        MutationSubmission::NeedsConfirmation(_) => panic!("confirmed bound plan challenged twice"),
    }
    assert_eq!(row(client, schema).await, "after");

    let analysis = analyze(backend, &document, schema, 6).await;
    let review = backend
        .review_mutations(&document, analysis, update(schema, "after", "forbidden"))
        .await
        .unwrap();
    let confirmation = challenge(backend, review, 7).await;
    policy(backend, true).await;
    assert!(matches!(
        backend.confirm_mutations(confirmation).await,
        Err(DataError::Mutation(
            ResultMutationError::PolicyBlocked { .. }
        ))
    ));
    assert_eq!(row(client, schema).await, "after");
    policy(backend, false).await;

    let analysis = analyze(backend, &document, schema, 8).await;
    let review = backend
        .review_mutations(
            &document,
            analysis,
            update(schema, "after", "stale overwrite"),
        )
        .await
        .unwrap();
    let confirmation = challenge(backend, review, 9).await;
    client
        .batch_execute(&format!(
            "UPDATE {schema}.rows SET body = 'concurrent' WHERE id = 1"
        ))
        .await
        .unwrap();
    assert!(matches!(
        backend.confirm_mutations(confirmation).await,
        Err(DataError::Mutation(ResultMutationError::Conflict {
            op_index: 0
        }))
    ));
    assert_eq!(row(client, schema).await, "concurrent");
    assert_eq!(
        crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .len(),
        1,
        "only the successful confirmed write is audited"
    );

    let analysis = analyze(backend, &document, schema, 10).await;
    let review = backend
        .review_mutations(
            &document,
            analysis,
            update(schema, "concurrent", "closed overwrite"),
        )
        .await
        .unwrap();
    assert_eq!(
        backend.close_data_document(&document).await.unwrap(),
        DataCloseOutcome::Closed
    );
    let replacement = backend
        .open_data_document("data-live", "table", &backend.fixture().id)
        .await
        .unwrap();
    assert!(matches!(
        backend.apply_review(review, 11).await,
        Err(DataError::Document(_))
    ));
    assert!(matches!(
        backend.browse_table(&document, browse(schema, 1)).await,
        Err(DataError::Document(_))
    ));
    assert!(matches!(
        backend
            .load_table_preferences(&document, schema.into(), "rows".into())
            .await,
        Err(DataError::Document(_))
    ));
    assert_eq!(row(client, schema).await, "concurrent");
    assert_eq!(
        backend
            .browse_table(&replacement, browse(schema, 1))
            .await
            .unwrap()
            .rows
            .len(),
        2
    );
    backend.close_data_document(&replacement).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture only; run serially without window acceptance"]
async fn native_data_browse_review_policy_conflict_and_document_lifecycle() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1"),
        "explicit owned fixture opt-in required"
    );
    let baseline = fixture_count().await;
    let directory = profile::directory();
    let foreign_directory = profile::directory();
    let schema = format!("native_data_{}", uuid::Uuid::new_v4().simple());
    let mut backend = None;
    let mut foreign = None;
    let mut client = None;
    let mut driver = None;
    let mut schema_created = false;
    // Catch setup and assertion panics too, so all acquired resources are closed.
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        foreign = Some(
            Backend::open_fixture(&foreign_directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let (admin, connection) = tokio_postgres::Config::new()
            .host("127.0.0.1")
            .port(15432)
            .dbname("dbunk_demo")
            .user("dbunk")
            .password("dbunk")
            .connect_timeout(WAIT)
            .connect(NoTls)
            .await
            .unwrap();
        driver = Some(tokio::spawn(connection));
        client = Some(admin);
        let admin = client.as_ref().unwrap();
        admin
            .batch_execute(&format!("CREATE SCHEMA {schema}"))
            .await
            .unwrap();
        schema_created = true;
        admin
            .batch_execute(&format!(
                "CREATE TABLE {schema}.rows(id integer PRIMARY KEY, body text NOT NULL); \
                 INSERT INTO {schema}.rows VALUES \
                 (1, 'before'), (2, 'two'), (3, 'three'), (4, 'four'), (5, 'five')"
            ))
            .await
            .unwrap();
        exercise(
            backend.as_ref().unwrap(),
            foreign.as_ref().unwrap(),
            admin,
            &schema,
        )
        .await;
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), operation))
            .catch_unwind()
            .await;

    let shutdown = async |backend: Option<Backend>| match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let (first, second) = tokio::join!(shutdown(backend), shutdown(foreign));
    let dropped = if schema_created {
        Some(
            tokio::time::timeout(
                WAIT,
                client
                    .as_ref()
                    .unwrap()
                    .batch_execute(&format!("DROP SCHEMA {schema} CASCADE")),
            )
            .await,
        )
    } else {
        None
    };
    drop(client);
    let joined = if let Some(mut task) = driver {
        match tokio::time::timeout(WAIT, &mut task).await {
            Ok(result) => Some(result),
            Err(_) => {
                task.abort();
                let _ = task.await;
                panic!("admin driver required abort after schema cleanup and client drop");
            }
        }
    } else {
        None
    };
    let baseline_restored = tokio::time::timeout(WAIT, async {
        while fixture_count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    first.unwrap();
    second.unwrap();
    if let Some(result) = dropped {
        result
            .expect("schema cleanup deadline")
            .expect("owned schema cleanup");
    }
    if let Some(result) = joined {
        result
            .expect("admin driver join")
            .expect("admin driver shutdown");
    }
    baseline_restored.expect("fixture backends returned to baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("live facade case deadline");
}
