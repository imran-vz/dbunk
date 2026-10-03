//! Explicit owned stage03 probe. Changes only objects inside one UUID-named
//! schema it created; every external SQL connection rechecks the fixture
//! identity first. Not run by default.
use super::*;
use crate::backend::profile;
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};

/// The operator names the exact owned instance; the helper then refuses any
/// other fixture. Fixture instances are disposable, so none is hard-coded.
fn fixture() -> String {
    std::env::var("DBUNK_NATIVE_FIXTURE_INSTANCE")
        .expect("set DBUNK_NATIVE_FIXTURE_INSTANCE to the verified owned stage03 instance")
}

async fn fixture_sql(query: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg("import sys; sys.path.insert(0,sys.argv[1]); import fixture; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,sys.stdin.read()))")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native"))
            .arg(fixture())
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|_| "Fixture helper did not start".to_owned())?;
        child.stdin.take().unwrap().write_all(query.as_bytes()).map_err(|_| "Fixture SQL input failed".to_owned())?;
        let output = child.wait_with_output().map_err(|_| "Fixture helper did not join".to_owned())?;
        if !output.status.success() { return Err("Identity-checked fixture SQL refused or failed".into()); }
        String::from_utf8(output.stdout).map(|text| text.trim().to_owned()).map_err(|_| "Invalid fixture response".into())
    }).await.map_err(|_| "Fixture helper task failed".to_owned())?
}
async fn relation_exists(schema: &str, name: &str) -> bool {
    fixture_sql(format!(
        "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname={} AND c.relname={});",
        crate::quote_literal(schema),
        crate::quote_literal(name)
    ))
    .await
    .unwrap()
        == "t"
}
async fn apply(backend: &Backend, review: ObjectDdlReview) -> ObjectDdlReceipt {
    let attempt = review.attempt_id().clone();
    let result = match backend.apply_object_ddl(review).await.unwrap() {
        ObjectDdlSubmission::NeedsConfirmation(c) => backend.confirm_object_ddl(*c).await.unwrap(),
        value => value,
    };
    let ObjectDdlSubmission::Finished(receipt) = result else {
        panic!("confirmation repeated")
    };
    assert_eq!(receipt.attempt_id, attempt);
    *receipt
}
async fn reviewed(
    backend: &Backend,
    document: &DataDocument,
    operations: Vec<ObjectDdlOperation>,
) -> ObjectDdlReview {
    let target = backend
        .observe_object_ddl(document, ObjectDdlRequest { operations })
        .await
        .unwrap();
    backend.review_object_ddl(target).await.unwrap()
}
/// Drops only the exact owned schema OID, under whichever name it has.
async fn cleanup(oid: u32) -> Result<(), String> {
    if oid == 0 {
        return Ok(());
    }
    let query = format!("DO $guard$ DECLARE n text; BEGIN SELECT nspname INTO n FROM pg_catalog.pg_namespace WHERE oid={oid} AND pg_get_userbyid(nspowner)='dbunk'; IF n IS NULL THEN RETURN; END IF; EXECUTE pg_catalog.format('DROP SCHEMA %I CASCADE', n); END $guard$;");
    fixture_sql(query).await.map(|_| ())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "changes only a UUID-owned schema on exact verified stage03 fixture; serial opt-in"]
async fn native_object_ddl_groups_prefix_stale_identity_and_cascade_impact() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let schema = format!("native_object_ddl_{}", uuid::Uuid::new_v4().simple());
    let q = crate::quote_double(&schema);
    let oid: u32 = fixture_sql(format!(
        "CREATE SCHEMA {q}; CREATE TABLE {q}.t (id integer); CREATE TYPE {q}.state AS ENUM ('a'); CREATE VIEW {q}.v AS SELECT 1 AS one; SELECT oid FROM pg_catalog.pg_namespace WHERE nspname={};",
        crate::quote_literal(&schema)
    ))
    .await
    .unwrap()
    .lines()
    .last()
    .unwrap()
    .parse()
    .unwrap();
    let directory = profile::directory();
    let mut backend = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let document = backend
            .open_data_document("object-ddl-live", "objects", &backend.fixture().id)
            .await
            .unwrap();
        // Mixed groups: an atomic view, then two standalone statements.
        let review = reviewed(
            backend,
            &document,
            vec![
                ObjectDdlOperation::CreateView {
                    schema: schema.clone(),
                    name: "fresh".into(),
                    or_replace: false,
                    sql_body: "SELECT 2 AS two".into(),
                },
                ObjectDdlOperation::CreateIndex {
                    schema: schema.clone(),
                    table: "t".into(),
                    name: "t_id_idx".into(),
                    unique: false,
                    method: "btree".into(),
                    columns: vec![ObjectDdlIndexColumn {
                        expression: "id".into(),
                        descending: false,
                    }],
                    concurrently: true,
                },
                ObjectDdlOperation::AddEnumValue {
                    schema: schema.clone(),
                    name: "state".into(),
                    value: "b".into(),
                    position: None,
                },
            ],
        )
        .await;
        assert!(review.preview().standalone());
        let receipt = apply(backend, review).await;
        assert!(
            matches!(receipt.outcome, ObjectDdlOutcome::Applied { .. }),
            "{:?}",
            receipt.outcome
        );
        assert!(relation_exists(&schema, "fresh").await);
        assert!(relation_exists(&schema, "t_id_idx").await);

        // Committed prefix: the view commits, the invalid index is rejected.
        let review = reviewed(
            backend,
            &document,
            vec![
                ObjectDdlOperation::CreateView {
                    schema: schema.clone(),
                    name: "prefix".into(),
                    or_replace: false,
                    sql_body: "SELECT 3 AS three".into(),
                },
                ObjectDdlOperation::CreateIndex {
                    schema: schema.clone(),
                    table: "t".into(),
                    name: "t_missing_idx".into(),
                    unique: false,
                    method: "btree".into(),
                    columns: vec![ObjectDdlIndexColumn {
                        expression: "missing_column".into(),
                        descending: false,
                    }],
                    concurrently: true,
                },
            ],
        )
        .await;
        let receipt = apply(backend, review).await;
        assert!(
            matches!(
                receipt.outcome,
                ObjectDdlOutcome::Stopped {
                    committed: 1,
                    stopped_at: 1,
                    stop: ObjectDdlStop::Rejected,
                    ..
                }
            ),
            "{:?}",
            receipt.outcome
        );
        assert!(relation_exists(&schema, "prefix").await);

        // A recreated view under the same name refuses with no effect.
        let drop = vec![ObjectDdlOperation::DropObject {
            reference: PgObjectRef {
                kind: PgObjectKind::View,
                schema: Some(schema.clone()),
                name: "v".into(),
                identity_args: None,
            },
            cascade: false,
        }];
        let stale = reviewed(backend, &document, drop.clone()).await;
        fixture_sql(format!(
            "DROP VIEW {q}.v; CREATE VIEW {q}.v AS SELECT 1 AS one;"
        ))
        .await
        .unwrap();
        let receipt = apply(backend, stale).await;
        assert_eq!(
            receipt.outcome,
            ObjectDdlOutcome::NotDispatched {
                reason: ObjectDdlFailure::TargetChanged
            }
        );
        assert!(relation_exists(&schema, "v").await);

        // CASCADE reviews the exact dependent captured with the identity.
        fixture_sql(format!("CREATE VIEW {q}.dep AS SELECT one FROM {q}.v;"))
            .await
            .unwrap();
        let cascade = vec![ObjectDdlOperation::DropObject {
            reference: PgObjectRef {
                kind: PgObjectKind::View,
                schema: Some(schema.clone()),
                name: "v".into(),
                identity_args: None,
            },
            cascade: true,
        }];
        let review = reviewed(backend, &document, cascade).await;
        let impact = review.impacts()[0].as_ref().unwrap();
        assert!(impact
            .dependents
            .iter()
            .any(|dependent| dependent.identity.ends_with(".dep")));
        let receipt = apply(backend, review).await;
        assert!(matches!(receipt.outcome, ObjectDdlOutcome::Applied { .. }));
        assert!(!relation_exists(&schema, "v").await);
        assert!(!relation_exists(&schema, "dep").await);
        backend.close_data_document(&document).await.unwrap();
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(120), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let cleaned = cleanup(oid).await;
    shutdown.expect("object DDL tasks and drivers joined");
    cleaned.expect("owned schema cleanup");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("object DDL live deadline");
}
