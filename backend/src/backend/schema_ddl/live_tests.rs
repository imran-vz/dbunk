//! Explicit owned stage03 write probe. Creates only two UUID-named empty schemas;
//! every external SQL connection rechecks the fixture identity before use.
use super::*;
use crate::backend::{data::DataCloseOutcome, objects::CatalogError, profile};
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};

const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";

async fn fixture_sql(query: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg("import sys; sys.path.insert(0,sys.argv[1]); import fixture; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,sys.stdin.read()))")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native"))
            .arg(FIXTURE)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|_| "Fixture helper did not start".to_owned())?;
        child.stdin.take().unwrap().write_all(query.as_bytes()).map_err(|_| "Fixture SQL input failed".to_owned())?;
        let output = child.wait_with_output().map_err(|_| "Fixture helper did not join".to_owned())?;
        if !output.status.success() { return Err("Identity-checked fixture SQL refused or failed".into()); }
        String::from_utf8(output.stdout).map(|text| text.trim().to_owned()).map_err(|_| "Invalid fixture response".into())
    }).await.map_err(|_| "Fixture helper task failed".to_owned())?
}
async fn count() -> u64 {
    fixture_sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid();".into()).await.unwrap().parse().unwrap()
}
fn reference(name: &str) -> PgObjectRef {
    PgObjectRef {
        kind: PgObjectKind::Schema,
        schema: None,
        name: name.into(),
        identity_args: None,
    }
}
async fn absent(name: &str) {
    let query = format!(
        "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={});",
        crate::quote_literal(name)
    );
    assert_eq!(
        fixture_sql(query).await.unwrap(),
        "t",
        "unique schema must be absent before ownership"
    );
}

// Only a name proved absent before this test's writes is supplied here. Inspect
// OID, owner and our unique comment, then assert that exact identity again in the
// same transaction as DROP. Reject all namespace children and use RESTRICT.
async fn cleanup(name: &str, comment: Option<&str>) -> Result<(), String> {
    let identity = fixture_sql(format!("SELECT json_build_object('oid',n.oid::bigint,'owner',pg_get_userbyid(n.nspowner),'comment',obj_description(n.oid,'pg_namespace'))::text FROM pg_catalog.pg_namespace n WHERE n.nspname={};", crate::quote_literal(name))).await?;
    if identity.is_empty() {
        return Ok(());
    }
    let row: serde_json::Value =
        serde_json::from_str(&identity).map_err(|_| "Invalid cleanup identity")?;
    let oid = row["oid"].as_u64().ok_or("Missing cleanup OID")?;
    if row["owner"].as_str() != Some("dbunk") || row["comment"].as_str() != comment {
        return Err("Cleanup identity/comment mismatch; schema preserved".into());
    }
    let comment_sql = comment
        .map(crate::quote_literal)
        .unwrap_or_else(|| "NULL".into());
    let drop_sql = format!("DROP SCHEMA {} RESTRICT", crate::quote_double(name));
    let query = format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace n WHERE n.oid={oid} AND n.nspname={} AND pg_get_userbyid(n.nspowner)='dbunk' AND obj_description(n.oid,'pg_namespace') IS NOT DISTINCT FROM {comment_sql}) THEN RAISE EXCEPTION 'Schema ownership changed'; END IF; IF EXISTS (SELECT 1 FROM pg_catalog.pg_depend WHERE refclassid='pg_catalog.pg_namespace'::regclass AND refobjid={oid}) THEN RAISE EXCEPTION 'Schema has unexpected children'; END IF; EXECUTE {}; END $guard$; COMMIT;", crate::quote_literal(name), crate::quote_literal(&drop_sql));
    fixture_sql(query).await?;
    absent(name).await;
    println!("owned schema cleaned: name={name} oid={oid} owner=dbunk mode=RESTRICT");
    Ok(())
}

async fn apply_review(backend: &Backend, review: CreateSchemaReview) -> CreateSchemaReceipt {
    let intent = review.intent().clone();
    let attempt = review.attempt_id().clone();
    let submission = backend.apply_create_schema(review).await.unwrap();
    let terminal = match submission {
        CreateSchemaSubmission::Finished(receipt) => return receipt,
        CreateSchemaSubmission::NeedsConfirmation(confirmation) => {
            assert_eq!(confirmation.intent(), &intent);
            assert_eq!(confirmation.attempt_id(), &attempt);
            backend.confirm_create_schema(*confirmation).await.unwrap()
        }
    };
    match terminal {
        CreateSchemaSubmission::Finished(receipt) => receipt,
        CreateSchemaSubmission::NeedsConfirmation(_) => {
            panic!("exact confirmation unexpectedly challenged again")
        }
    }
}

async fn forced_atomic_failure(
    backend: &Backend,
    document: &DataDocument,
    name: &str,
) -> CreateSchemaOutcome {
    let inner = backend.0.clone();
    let sql = format!("CREATE SCHEMA {};", crate::quote_double(name));
    // A repeated CREATE of this exact fresh schema deliberately fails statement
    // two. No production API accepts this arbitrary preview.
    let preview = CreateSchemaPreview {
        statements: vec![
            CreateSchemaStatement {
                sql: sql.clone(),
                summary: "owned rollback first".into(),
            },
            CreateSchemaStatement {
                sql,
                summary: "owned rollback deliberate duplicate".into(),
            },
        ],
    };
    backend
        .data_call(document, move |state, document, admission| async move {
            let permit = inner.documents.begin_write(&document).unwrap();
            let cancelled = document.0.read_cancellation();
            let connection = crate::app::find_connection(&state, &document.0.connection)
                .await
                .unwrap();
            let spec = ResolvedPostgresConnectSpec::from_connection(&connection).unwrap();
            let drivers = inner.tasks.child();
            drop(admission);
            Ok(native_schema_ddl::execute(&spec, &drivers, &permit, cancelled, &preview).await)
        })
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "writes only UUID-owned empty schemas on exact verified stage03 fixture; serial opt-in"]
async fn native_create_schema_receipt_duplicate_atomic_rollback_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let baseline = count().await; // fixture.check verifies process/container and exact SQL sentinel.
    let name = format!("native_schema_ddl_{}", uuid::Uuid::new_v4().simple());
    let rollback_name = format!("{name}_r");
    absent(&name).await;
    absent(&rollback_name).await;
    let comment = format!("owned exact ' \\ 日本語 {}", uuid::Uuid::new_v4());
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
            .open_data_document("schema-live", "schema", &backend.fixture().id)
            .await
            .unwrap();
        assert!(crate::storage::read_safety_overrides(
            &backend.0.state.pool,
            &backend.fixture().id
        )
        .await
        .unwrap()
        .is_empty());
        let intent = CreateSchemaIntent::new(name.clone(), Some(comment.clone())).unwrap();
        let review = backend
            .review_create_schema(&document, intent.clone())
            .await
            .unwrap();
        assert!(review.belongs_to(&document));
        assert_eq!(review.preview().statements.len(), 2);
        assert!(review.preview().checked_heap_bytes().is_some());
        let attempt = review.attempt_id().clone();
        let receipt = apply_review(backend, review).await;
        assert_eq!(receipt.attempt_id, attempt);
        assert_eq!(receipt.intent, intent);
        assert_eq!(receipt.connection_id, backend.fixture().id);
        assert!(matches!(
            receipt.outcome,
            CreateSchemaOutcome::Applied { statements: 2, .. }
        ));
        let audits =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        assert_eq!(
            audits.len(),
            1,
            "confirmed successful transaction audits exactly once"
        );
        assert_eq!(audits[0].command, "apply_object_ddl");
        assert_eq!(audits[0].classes, ["ddl"]);
        let success_audit_time = audits[0].occurred_at.clone();
        let description = backend
            .describe_object(&document, reference(&name))
            .await
            .unwrap();
        assert_eq!(description.reference, reference(&name));
        assert_eq!(description.owner.as_deref(), Some("dbunk"));
        assert_eq!(description.comment.as_deref(), Some(comment.as_str()));
        let duplicate = backend
            .review_create_schema(
                &document,
                CreateSchemaIntent::new(
                    name.clone(),
                    Some("must not replace first comment".into()),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let duplicate_attempt = duplicate.attempt_id().clone();
        let receipt = apply_review(backend, duplicate).await;
        assert_eq!(receipt.attempt_id, duplicate_attempt);
        assert!(
            matches!(receipt.outcome, CreateSchemaOutcome::NotApplied { reason: CreateSchemaFailure::Database { ref code } } if code.as_deref()==Some("42P06"))
        );
        let audits =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        assert_eq!(
            audits.len(),
            1,
            "duplicate known-not-applied attempt must not audit"
        );
        assert_eq!(audits[0].command, "apply_object_ddl");
        assert_eq!(audits[0].classes, ["ddl"]);
        assert_eq!(audits[0].occurred_at, success_audit_time);
        assert_eq!(
            backend
                .describe_object(&document, reference(&name))
                .await
                .unwrap()
                .comment
                .as_deref(),
            Some(comment.as_str())
        );
        assert!(
            matches!(forced_atomic_failure(backend, &document, &rollback_name).await, CreateSchemaOutcome::NotApplied { reason: CreateSchemaFailure::Database { ref code } } if code.as_deref()==Some("42P06"))
        );
        assert!(matches!(
            backend
                .describe_object(&document, reference(&rollback_name))
                .await,
            Err(super::super::data::DataError::Catalog(
                CatalogError::ObjectNotFound
            ))
        ));
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend
                .describe_object(&document, reference(&name))
                .await
                .unwrap()
                .comment
                .as_deref(),
            Some(comment.as_str())
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend.review_create_schema(&document, intent).await,
            Err(CreateSchemaError::Unavailable)
        ));
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), operation))
            .catch_unwind()
            .await;
    // Join first: an abandoned facade waiter must not commit after cleanup.
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let cleaned = cleanup(&name, Some(&comment)).await;
    let rollback_cleaned = cleanup(&rollback_name, None).await;
    let restored = tokio::time::timeout(Duration::from_secs(10), async {
        while count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    shutdown.expect("schema tasks and drivers joined");
    cleaned.expect("owned schema cleanup");
    rollback_cleaned.expect("owned rollback schema cleanup");
    restored.expect("fixture activity returned to baseline");
    println!("fixture={FIXTURE} endpoint=127.0.0.1:15432/dbunk_demo baseline={baseline} restored=true rollback_schema_absent=true confirmed_success_audit=1 duplicate_added_audit=0");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("schema live deadline");
}
