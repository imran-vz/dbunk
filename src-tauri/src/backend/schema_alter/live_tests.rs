//! Explicit owned stage03 probe. Alters only one UUID-named empty schema it
//! created; every external SQL connection rechecks the fixture identity first.
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
async fn absent(name: &str) -> bool {
    fixture_sql(format!(
        "SELECT NOT EXISTS (SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={});",
        crate::quote_literal(name)
    ))
    .await
    .unwrap()
        == "t"
}
async fn apply(backend: &Backend, review: SchemaAlterReview) -> SchemaAlterReceipt {
    let attempt = review.attempt_id().clone();
    let result = match backend.apply_schema_alter(review).await.unwrap() {
        SchemaAlterSubmission::NeedsConfirmation(c) => {
            backend.confirm_schema_alter(*c).await.unwrap()
        }
        value => value,
    };
    let SchemaAlterSubmission::Finished(receipt) = result else {
        panic!("confirmation repeated")
    };
    assert_eq!(receipt.attempt_id, attempt);
    *receipt
}
/// Drops only the exact owned OID, empty, with RESTRICT, under whichever name it has.
async fn cleanup(oid: u32) -> Result<(), String> {
    if oid == 0 {
        return Ok(());
    }
    let query = format!("DO $guard$ DECLARE n text; BEGIN SELECT nspname INTO n FROM pg_catalog.pg_namespace WHERE oid={oid} AND pg_get_userbyid(nspowner)='dbunk'; IF n IS NULL THEN RETURN; END IF; IF EXISTS (SELECT 1 FROM pg_catalog.pg_depend WHERE refclassid='pg_catalog.pg_namespace'::regclass AND refobjid={oid}) THEN RAISE EXCEPTION 'Schema has unexpected children'; END IF; EXECUTE pg_catalog.format('DROP SCHEMA %I RESTRICT', n); END $guard$;");
    fixture_sql(query).await.map(|_| ())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "alters only a UUID-owned empty schema on exact verified stage03 fixture; serial opt-in"]
async fn native_schema_alter_rename_comment_and_stale_identity_refusal() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let name = format!("native_schema_alter_{}", uuid::Uuid::new_v4().simple());
    let renamed = format!("{name} renamed\"");
    assert!(absent(&name).await && absent(&renamed).await);
    let oid: u32 = fixture_sql(format!(
        "CREATE SCHEMA {}; SELECT oid FROM pg_catalog.pg_namespace WHERE nspname={};",
        crate::quote_double(&name),
        crate::quote_literal(&name)
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
            .open_data_document("schema-alter-live", "objects", &backend.fixture().id)
            .await
            .unwrap();
        let request = SchemaAlterRequest {
            schema: name.clone(),
            expected: None,
        };
        let target = backend
            .observe_schema_alter(&document, request.clone())
            .await
            .unwrap();
        assert_eq!(target.description().identity.schema_oid, oid);
        let comment = "owned ' \\ 日本語".to_owned();
        let review = backend
            .review_schema_alter(
                target,
                SchemaAlterIntent::SetComment {
                    comment: Some(comment.clone()),
                },
            )
            .await
            .unwrap();
        let receipt = apply(backend, review).await;
        assert!(matches!(
            receipt.outcome,
            SchemaAlterOutcome::Applied { .. }
        ));
        // A stale observation (pre-comment row/comment) refuses with no effect.
        let stale = backend
            .observe_schema_alter(&document, request.clone())
            .await
            .unwrap();
        let stale_review = backend
            .review_schema_alter(
                stale,
                SchemaAlterIntent::Rename {
                    new_name: renamed.clone(),
                },
            )
            .await
            .unwrap();
        fixture_sql(format!(
            "COMMENT ON SCHEMA {} IS 'concurrent';",
            crate::quote_double(&name)
        ))
        .await
        .unwrap();
        let receipt = apply(backend, stale_review).await;
        assert_eq!(
            receipt.outcome,
            SchemaAlterOutcome::NotDispatched {
                reason: SchemaAlterFailure::TargetChanged
            }
        );
        assert!(absent(&renamed).await);
        let target = backend
            .observe_schema_alter(&document, request)
            .await
            .unwrap();
        assert_eq!(target.description().comment.as_deref(), Some("concurrent"));
        let review = backend
            .review_schema_alter(
                target,
                SchemaAlterIntent::Rename {
                    new_name: renamed.clone(),
                },
            )
            .await
            .unwrap();
        let receipt = apply(backend, review).await;
        assert!(matches!(
            receipt.outcome,
            SchemaAlterOutcome::Applied { .. }
        ));
        assert!(absent(&name).await);
        let after = backend
            .observe_schema_alter(
                &document,
                SchemaAlterRequest {
                    schema: renamed.clone(),
                    expected: Some(receipt.target.identity),
                },
            )
            .await
            .unwrap();
        assert_eq!(after.description().identity.schema_oid, oid);
        backend.close_data_document(&document).await.unwrap();
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let cleaned = cleanup(oid).await;
    shutdown.expect("schema alter tasks and drivers joined");
    cleaned.expect("owned schema cleanup");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("schema alter live deadline");
}
