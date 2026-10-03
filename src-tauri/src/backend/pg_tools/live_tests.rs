//! Ignored real-client probe, never part of deterministic tests. Requires the
//! exact owned stage03 fixture and patched libpq clients selected through PATH.
use super::*;
use crate::{backend::profile, postgres::backup::runner};
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
async fn sql(query: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move||{
    let mut child=std::process::Command::new("python3").arg("-c").arg("import sys; sys.path.insert(0,sys.argv[1]); import fixture; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,sys.stdin.read()))").arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_|"Fixture helper refused")?;
    child.stdin.take().unwrap().write_all(query.as_bytes()).map_err(|_|"Fixture SQL input failed")?;let output=child.wait_with_output().map_err(|_|"Fixture helper failed to join")?;if !output.status.success(){return Err("Identity-checked fixture SQL failed".to_owned());}String::from_utf8(output.stdout).map(|s|s.trim().to_owned()).map_err(|_|"Invalid fixture response".into())
}).await.map_err(|_|"Fixture helper task failed".to_owned())?
}
async fn count() -> u64 {
    sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await.unwrap().parse().unwrap()
}
async fn absent(schema: &str) {
    assert_eq!(
        sql(format!(
            "SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={})",
            crate::quote_literal(schema)
        ))
        .await
        .unwrap(),
        "t"
    );
}
async fn drop_table(schema: &str, comment: &str) -> Result<(), String> {
    let identity=sql(format!("SELECT json_build_object('oid',c.oid::bigint,'owner',pg_get_userbyid(c.relowner),'comment',obj_description(c.oid,'pg_class'),'kind',c.relkind)::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname={} AND c.relname='payload'",crate::quote_literal(schema))).await?;
    if identity.is_empty() {
        return Ok(());
    }
    let row: serde_json::Value =
        serde_json::from_str(&identity).map_err(|_| "Invalid table identity")?;
    let oid = row["oid"].as_u64().ok_or("Missing table OID")?;
    if row["owner"].as_str() != Some("dbunk")
        || row["comment"].as_str() != Some(comment)
        || row["kind"].as_str() != Some("r")
    {
        return Err("Owned table identity changed; preserved".into());
    }
    let drop = format!(
        "DROP TABLE {}.payload RESTRICT",
        crate::quote_double(schema)
    );
    sql(format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE c.oid={oid} AND n.nspname={} AND c.relname='payload' AND c.relkind='r' AND pg_get_userbyid(c.relowner)='dbunk' AND obj_description(c.oid,'pg_class')={}) THEN RAISE EXCEPTION 'Owned table changed'; END IF; EXECUTE {}; END $guard$; COMMIT",crate::quote_literal(schema),crate::quote_literal(comment),crate::quote_literal(&drop))).await?;
    println!("owned tool table removed: schema={schema} oid={oid} mode=RESTRICT");
    Ok(())
}
async fn cleanup(schema: &str, comment: &str) -> Result<(), String> {
    drop_table(schema, comment).await?;
    let identity=sql(format!("SELECT json_build_object('oid',oid::bigint,'owner',pg_get_userbyid(nspowner),'comment',obj_description(oid,'pg_namespace'))::text FROM pg_catalog.pg_namespace WHERE nspname={}",crate::quote_literal(schema))).await?;
    if identity.is_empty() {
        return Ok(());
    }
    let row: serde_json::Value =
        serde_json::from_str(&identity).map_err(|_| "Invalid schema identity")?;
    let oid = row["oid"].as_u64().ok_or("Missing schema OID")?;
    if row["owner"].as_str() != Some("dbunk") || row["comment"].as_str() != Some(comment) {
        return Err("Owned schema identity changed; preserved".into());
    }
    let drop = format!("DROP SCHEMA {} RESTRICT", crate::quote_double(schema));
    sql(format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE oid={oid} AND nspname={} AND pg_get_userbyid(nspowner)='dbunk' AND obj_description(oid,'pg_namespace')={}) THEN RAISE EXCEPTION 'Owned schema changed'; END IF; IF EXISTS(SELECT 1 FROM pg_catalog.pg_depend WHERE refclassid='pg_catalog.pg_namespace'::regclass AND refobjid={oid}) THEN RAISE EXCEPTION 'Unexpected schema children'; END IF; EXECUTE {}; END $guard$; COMMIT",crate::quote_literal(schema),crate::quote_literal(comment),crate::quote_literal(&drop))).await?;
    absent(schema).await;
    println!("owned tool schema removed: name={schema} oid={oid} mode=RESTRICT");
    Ok(())
}
async fn wait(backend: &Backend, id: PgToolAttemptId, ready: bool) -> PgToolObservation {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let row = backend.get_pg_tool_job(id).unwrap();
            if (ready && row.phase == PgToolPhase::ReadyReview)
                || (row.phase.terminal() && row.cleanup == PgToolCleanup::Complete)
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap()
}
fn submit(backend: &Backend, id: PgToolAttemptId, restore: bool) {
    let review = backend.review_pg_tool_job(id).unwrap();
    assert_eq!(review.target().host, "127.0.0.1");
    assert_eq!(review.target().port, 15432);
    assert_eq!(review.target().database, "dbunk_demo");
    assert_eq!(review.target().user, "dbunk");
    let submission = backend.start_pg_tool_job(review).unwrap();
    match submission {
        PgToolSubmission::NeedsConfirmation(confirmation) => {
            assert!(restore);
            assert_eq!(confirmation.review().attempt_id(), id);
            assert!(matches!(
                backend.confirm_pg_tool_job(*confirmation).unwrap(),
                PgToolSubmission::Accepted(_)
            ));
        }
        PgToolSubmission::Accepted(_) => {
            assert!(!restore, "strict fixture restore requires confirmation")
        }
    }
}
async fn data(schema: &str) -> String {
    sql(format!("SELECT json_agg(json_build_array(id::text,body,n::text) ORDER BY id)::text FROM {}.payload",crate::quote_double(schema))).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 only; requires DBUNK_NATIVE_FIXTURE_VERIFIED=1 and patched libpq PATH; serial explicit opt-in"]
async fn native_plain_custom_backup_restore_owned_table_and_joined_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let baseline = count().await;
    let schema = format!("native_tools_{}", uuid::Uuid::new_v4().simple());
    let comment = format!("owned native tool probe {}", uuid::Uuid::new_v4());
    absent(&schema).await;
    let directory = profile::directory();
    let archives = tempfile::tempdir().unwrap();
    let mut backend = None;
    let operation = async {
        sql(format!("BEGIN; CREATE SCHEMA {}; COMMENT ON SCHEMA {} IS {}; CREATE TABLE {}.payload(id bigint PRIMARY KEY,body text,n numeric(40,10)); COMMENT ON TABLE {}.payload IS {}; INSERT INTO {}.payload VALUES(1,{},12345678901234567890.1234567890),(9223372036854775807,NULL,-0.0000000001); COMMIT",crate::quote_double(&schema),crate::quote_double(&schema),crate::quote_literal(&comment),crate::quote_double(&schema),crate::quote_double(&schema),crate::quote_literal(&comment),crate::quote_double(&schema),crate::quote_literal("exact ' \\ 日本語\nline"))).await.unwrap();
        let original = data(&schema).await;
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        // Unit builds prohibit implicit real clients. This one explicit bridge
        // rechecks the environment and exact SQL fixture sentinel for EVERY run.
        *backend.0.tool_jobs.test_runner.lock().unwrap() =
            Some(Arc::new(|context, connection, request| {
                async move {
                    assert_eq!(
                        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
                        Ok("1")
                    );
                    assert_eq!(
                        sql("SELECT current_database()".into()).await.unwrap(),
                        "dbunk_demo"
                    );
                    runner::run(context, connection, request).await
                }
                .boxed()
            }));
        let choices = [
            (PgToolFormat::Plain, archives.path().join("owned.sql")),
            (PgToolFormat::Custom, archives.path().join("owned.dump")),
        ];
        for (format, path) in &choices {
            let id = PgToolAttemptId::new();
            backend
                .begin_pg_tool_job(
                    id,
                    profile::CONNECTION_ID.into(),
                    PgToolIntent::backup(
                        path.clone(),
                        *format,
                        PgToolScope::Table {
                            schema: schema.clone(),
                            table: "payload".into(),
                        },
                        false,
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(
                wait(backend, id, true).await.phase,
                PgToolPhase::ReadyReview
            );
            submit(backend, id, false);
            let done = wait(backend, id, false).await;
            assert_eq!(done.phase, PgToolPhase::Completed, "{done:?}");
            assert_eq!(done.effect, PgToolEffect::Succeeded);
            let bytes = std::fs::metadata(path).unwrap().len();
            assert!(bytes > 0);
            assert_eq!(done.bytes_processed, Some(bytes));
            assert!(done
                .tool_version
                .as_ref()
                .is_some_and(|v| v.starts_with("pg_dump")));
            println!("owned backup complete: format={format:?} bytes={bytes}");
        }
        for (index, (format, path)) in choices.iter().enumerate() {
            let id = PgToolAttemptId::new();
            backend
                .begin_pg_tool_job(
                    id,
                    profile::CONNECTION_ID.into(),
                    PgToolIntent::restore(path.clone(), *format, false).unwrap(),
                )
                .unwrap();
            let prepared = wait(backend, id, true).await;
            assert_eq!(prepared.phase, PgToolPhase::ReadyReview);
            assert!(prepared.source_bytes.is_some_and(|bytes| bytes > 0));
            if index == 0 {
                std::fs::rename(path, archives.path().join("preserved-original.sql")).unwrap();
                std::fs::write(path, b"selected pathname replaced after snapshot\n").unwrap();
            }
            drop_table(&schema, &comment).await.unwrap();
            submit(backend, id, true);
            let done = wait(backend, id, false).await;
            assert_eq!(done.phase, PgToolPhase::Completed, "{done:?}");
            assert_eq!(done.effect, PgToolEffect::Succeeded);
            assert_eq!(done.restore_change_revision, Some(index as u64 + 1));
            assert!(done.tool_version.is_some());
            assert_eq!(data(&schema).await, original);
            let actual_comment = sql(format!(
                "SELECT obj_description('{}.payload'::regclass,'pg_class')",
                schema
            ))
            .await
            .unwrap();
            assert_eq!(actual_comment, comment);
            let audits:i64=sqlx::query_scalar("SELECT count(*) FROM safety_overrides WHERE connection_id=? AND command='start_pg_restore'").bind(profile::CONNECTION_ID).fetch_one(&backend.0.state.pool).await.unwrap();
            assert_eq!(audits, index as i64 + 1);
            println!("owned restore complete: format={format:?} rows=2 exact_bigint_numeric_text=true audit_count={audits}");
        }
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(240), operation))
            .catch_unwind()
            .await;
    let joined = match backend.as_ref() {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let cleaned = if joined.is_ok() {
        cleanup(&schema, &comment).await
    } else {
        Err("Cleanup preserved owned objects because job joins failed".into())
    };
    assert!(joined.is_ok(), "{joined:?}");
    assert!(cleaned.is_ok(), "{cleaned:?}");
    assert_eq!(
        count().await,
        baseline,
        "owned probe must restore activity baseline"
    );
    assert!(
        matches!(result, Ok(Ok(()))),
        "owned tool probe failed after guarded cleanup"
    );
}
