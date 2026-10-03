//! Explicit opt-in probe against the exact owned stage03 fixture. This is not
//! window, keyboard/AX, or general PostgreSQL parity evidence.
use super::*;
use crate::{
    backend::profile,
    postgres::transfer::{csv, runner},
};
use futures_util::FutureExt;
use std::{
    io::{Read, Write},
    path::Path,
    process::Stdio,
    time::Duration,
};

const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
const HEADER: &str = "dup,dup,,notes,quoted_null,skip\n";
static SQL_HELPERS: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>> =
    std::sync::Mutex::new(Vec::new());

async fn sql(query: String) -> Result<String, String> {
    // An outer probe timeout may drop this receiver. Retain the non-abortable
    // helper join separately so no late fixture SQL can race object cleanup.
    let (send, receive) = tokio::sync::oneshot::channel();
    let helper = tokio::task::spawn_blocking(move || {
        let result = (|| {
            let mut child = std::process::Command::new("python3")
            .arg("-c")
            .arg("import os,sys; sys.path.insert(0,sys.argv[1]); import fixture; assert os.environ.get('DBUNK_NATIVE_FIXTURE_VERIFIED')=='1'; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,\"SET statement_timeout='10s'; SET lock_timeout='3s'; \"+sys.stdin.read()))")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native"))
            .arg(FIXTURE)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|_| "Fixture helper refused")?;
            let written = child
                .stdin
                .take()
                .ok_or("Missing helper input")
                .and_then(|mut input| {
                    input
                        .write_all(query.as_bytes())
                        .map_err(|_| "Fixture SQL input failed")
                });
            if written.is_err() {
                let _ = child.kill();
                child.wait().map_err(|_| "Fixture helper failed to join")?;
                return Err("Fixture SQL input failed".to_owned());
            }
            let output = child
                .wait_with_output()
                .map_err(|_| "Fixture helper failed to join")?;
            if !output.status.success() {
                return Err("Identity-checked fixture SQL failed".to_owned());
            }
            String::from_utf8(output.stdout)
                .map(|s| s.trim().to_owned())
                .map_err(|_| "Invalid fixture response".into())
        })();
        let _ = send.send(result);
    });
    SQL_HELPERS.lock().unwrap().push(helper);
    receive
        .await
        .map_err(|_| "Fixture helper task failed".to_owned())?
}

async fn join_sql_helpers() -> Result<(), String> {
    let helpers = std::mem::take(&mut *SQL_HELPERS.lock().unwrap());
    let mut failed = false;
    for mut helper in helpers {
        match tokio::time::timeout(Duration::from_secs(30), &mut helper).await {
            Ok(result) => failed |= result.is_err(),
            Err(_) => {
                SQL_HELPERS.lock().unwrap().push(helper);
                failed = true;
            }
        }
    }
    if failed {
        Err("Fixture helper joins incomplete; objects preserved".into())
    } else {
        Ok(())
    }
}

async fn check_fixture() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    assert_eq!(
        sql("SELECT current_database()".into()).await.unwrap(),
        "dbunk_demo"
    );
}

async fn activity() -> u64 {
    sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into())
        .await.unwrap().parse().unwrap()
}

#[derive(serde::Deserialize)]
struct Identity {
    schema_oid: u64,
    table_oid: u64,
}

async fn create_target(schema: &str, comment: &str) -> Identity {
    let name = crate::quote_double(schema);
    let marker = crate::quote_literal(comment);
    let response = sql(format!(
        "BEGIN; CREATE SCHEMA {name}; COMMENT ON SCHEMA {name} IS {marker}; \
         CREATE TABLE {name}.payload(\
         row_id bigint GENERATED ALWAYS AS IDENTITY, id bigint PRIMARY KEY, \
         n numeric(40,10), body text, notes text, quoted_null text, \
         defaulted text NOT NULL DEFAULT 'owned default', \
         body_length integer GENERATED ALWAYS AS (length(body)) STORED); \
         COMMENT ON TABLE {name}.payload IS {marker}; COMMIT; \
         SELECT json_build_object('schema_oid',n.oid::bigint,'table_oid',c.oid::bigint)::text \
         FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid \
         WHERE n.nspname={} AND c.relname='payload'",
        crate::quote_literal(schema)
    ))
    .await
    .unwrap();
    serde_json::from_str(&response).unwrap()
}

// OIDs are captured at creation, never adopted during cleanup. A changed name,
// owner, marker, or additional child preserves the objects for investigation.
async fn cleanup(schema: &str, comment: &str, identity: Option<&Identity>) -> Result<(), String> {
    let Some(identity) = identity else {
        let absent = sql(format!(
            "SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={})",
            crate::quote_literal(schema)
        ))
        .await?;
        return if absent == "t" {
            Ok(())
        } else {
            Err("Uncaptured schema identity; preserved".into())
        };
    };
    let name = crate::quote_literal(schema);
    let marker = crate::quote_literal(comment);
    let table_drop = crate::quote_literal(&format!(
        "DROP TABLE {}.payload RESTRICT",
        crate::quote_double(schema)
    ));
    let schema_drop = crate::quote_literal(&format!(
        "DROP SCHEMA {} RESTRICT",
        crate::quote_double(schema)
    ));
    sql(format!(
        "BEGIN; DO $guard$ BEGIN \
         IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE oid={} \
         AND nspname={name} AND pg_get_userbyid(nspowner)='dbunk' \
         AND obj_description(oid,'pg_namespace')={marker}) \
         THEN RAISE EXCEPTION 'Owned schema changed'; END IF; \
         IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid={} \
         AND relnamespace={} AND relname='payload' AND relkind='r' \
         AND pg_get_userbyid(relowner)='dbunk' AND obj_description(oid,'pg_class')={marker}) \
         THEN RAISE EXCEPTION 'Owned table changed'; END IF; \
         EXECUTE {table_drop}; \
         IF EXISTS(SELECT 1 FROM pg_catalog.pg_depend \
         WHERE refclassid='pg_catalog.pg_namespace'::regclass AND refobjid={}) \
         THEN RAISE EXCEPTION 'Unexpected schema children'; END IF; \
         EXECUTE {schema_drop}; END $guard$; COMMIT",
        identity.schema_oid, identity.table_oid, identity.schema_oid, identity.schema_oid
    ))
    .await?;
    let absent = sql(format!(
        "SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={name})"
    ))
    .await?;
    if absent != "t" {
        return Err("Owned schema remains".into());
    }
    println!(
        "owned CSV cleanup: schema={schema} schema_oid={} table_oid={} mode=RESTRICT",
        identity.schema_oid, identity.table_oid
    );
    Ok(())
}

fn target(schema: &str) -> CsvTarget {
    CsvTarget {
        schema: schema.into(),
        table: "payload".into(),
    }
}

fn mapping() -> Vec<CsvMapping> {
    ["id", "n", "body", "notes", "quoted_null"]
        .into_iter()
        .enumerate()
        .map(|(source_index, column)| CsvMapping {
            source_index,
            target_column: column.into(),
        })
        .collect()
}

async fn inspect(backend: &Backend, intent: CsvInspectionIntent) -> CsvInspection {
    let id = CsvInspectionId::new();
    backend
        .begin_csv_inspection(id, profile::CONNECTION_ID.into(), intent)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let row = backend.get_csv_inspection(id).unwrap();
            match row.phase {
                CsvInspectionPhase::Ready => return backend.csv_inspection(id).unwrap(),
                CsvInspectionPhase::Failed | CsvInspectionPhase::Cancelled => {
                    panic!("Inspection failed: {row:?}")
                }
                _ => tokio::time::sleep(Duration::from_millis(25)).await,
            }
        }
    })
    .await
    .unwrap()
}

fn start(backend: &Backend, review: CsvTransferReview) -> CsvTransferAttemptId {
    let id = CsvTransferAttemptId::new();
    let direction = review.direction();
    let inspection_id = review.inspection_id();
    assert_eq!(review.target().host, "127.0.0.1");
    assert_eq!(review.target().port, 15432);
    assert_eq!(review.target().database, "dbunk_demo");
    assert_eq!(review.target().user, "dbunk");
    match backend.begin_csv_transfer(id, review).unwrap() {
        CsvTransferSubmission::NeedsConfirmation(confirmation) => {
            assert_eq!(direction, CsvDirection::Import);
            assert_eq!(confirmation.attempt_id(), id);
            assert_eq!(confirmation.review().inspection_id(), inspection_id);
            assert_eq!(
                backend.get_csv_transfer(id).unwrap().phase,
                CsvTransferPhase::AwaitingConfirmation
            );
            assert!(matches!(
                backend.confirm_csv_transfer(*confirmation).unwrap(),
                CsvTransferSubmission::Accepted(_)
            ));
        }
        CsvTransferSubmission::Accepted(_) => assert_eq!(
            direction,
            CsvDirection::Export,
            "Strict fixture import requires explicit confirmation"
        ),
    }
    id
}

async fn wait(backend: &Backend, id: CsvTransferAttemptId) -> CsvTransferObservation {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let row = backend.get_csv_transfer(id).unwrap();
            if row.phase.terminal() && row.cleanup != CsvCleanup::Pending {
                assert_eq!(row.cleanup, CsvCleanup::Complete, "{row:?}");
                return row;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap()
}

async fn rows(schema: &str) -> serde_json::Value {
    let result = sql(format!("SELECT coalesce(json_agg(json_build_array(id::text,n::text,body,notes,quoted_null,defaulted,row_id::text,body_length::text) ORDER BY id),'[]'::json)::text FROM {}.payload", crate::quote_double(schema))).await.unwrap();
    serde_json::from_str(&result).unwrap()
}

async fn audit_count(backend: &Backend) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM safety_overrides WHERE connection_id=? AND command='start_pg_csv_import'")
        .bind(profile::CONNECTION_ID).fetch_one(&backend.0.state.pool).await.unwrap()
}

// Parse the small owned output incrementally. This deliberately verifies the
// emitted file, including NULL quoting, rather than trusting job row counters.
fn exported_rows(path: &Path) -> serde_json::Value {
    let options = csv::CsvOptions::default();
    let mut parser = csv::Parser::new(&options).unwrap();
    let mut input = std::fs::File::open(path).unwrap();
    let mut buffer = [0_u8; 8192];
    let mut records = Vec::new();
    loop {
        let count = input.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        for byte in &buffer[..count] {
            if let Some(record) = parser.push(*byte).unwrap() {
                assert_eq!(record.len(), 8);
                records.push(record);
                assert!(records.len() <= 3);
            }
        }
    }
    if let Some(record) = parser.finish().unwrap() {
        records.push(record);
    }
    assert_eq!(records.len(), 3);
    let header = records.remove(0);
    let columns = [
        "id",
        "n",
        "body",
        "notes",
        "quoted_null",
        "defaulted",
        "row_id",
        "body_length",
    ]
    .map(|name| header.iter().position(|cell| cell.value == name).unwrap());
    let mut values: Vec<Vec<Option<String>>> = records
        .into_iter()
        .map(|record| {
            assert_eq!(record.len(), 8);
            columns
                .map(|index| {
                    (!record[index].is_null(&options)).then(|| record[index].value.clone())
                })
                .to_vec()
        })
        .collect();
    values.sort_by_key(|row| row[0].as_ref().unwrap().parse::<i64>().unwrap());
    serde_json::to_value(values).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 only; DBUNK_NATIVE_FIXTURE_VERIFIED=1; serial explicit opt-in"]
async fn native_csv_owned_import_export_rollback_and_joined_cleanup() {
    println!("CSV probe fixture target: {FIXTURE} at 127.0.0.1:15432/dbunk_demo");
    check_fixture().await;
    let baseline = activity().await;
    println!("owned CSV baseline activity: {baseline}");
    let schema = format!("native_csv_{}", uuid::Uuid::new_v4().simple());
    let comment = format!("owned native CSV probe {}", uuid::Uuid::new_v4());
    let directory = profile::directory();
    let files = tempfile::tempdir().unwrap();
    println!("owned CSV targets: fixture={FIXTURE} endpoint=127.0.0.1:15432/dbunk_demo schema={schema} files={} profile={}", files.path().display(), directory.path().display());
    let mut identity = None;
    let mut backend = None;
    let operation = async {
        identity = Some(create_target(&schema, &comment).await);
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let mut stored =
            crate::storage::read_connection_by_id(&backend.0.state.pool, profile::CONNECTION_ID)
                .await
                .unwrap()
                .unwrap();
        let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
            panic!("PostgreSQL fixture profile required")
        };
        pg.safe_mode = crate::SafeMode::Strict;
        crate::storage::upsert_connection(&backend.0.state.pool, &stored)
            .await
            .unwrap();
        // Unit builds do not invoke real sockets implicitly. Both explicit
        // bridges recheck the owned fixture before every real backend action.
        *backend.0.csv_transfers.test_inspector.lock().unwrap() =
            Some(Arc::new(|connection, payload, io| {
                async move {
                    check_fixture().await;
                    runner::inspect_in(connection, payload, io).await
                }
                .boxed()
            }));
        *backend.0.csv_transfers.test_runner.lock().unwrap() =
            Some(Arc::new(|context, connection, review, request| {
                async move {
                    check_fixture().await;
                    runner::run(context, connection, review, request).await
                }
                .boxed()
            }));

        let source = files.path().join("exact.csv");
        std::fs::write(&source, format!("{HEADER}9223372036854775807,12345678901234567890.1234567890,\"雪,\n\"\"quoted\"\"\",\\N,\"\\N\",ignored\n-9223372036854775808,-0.0000000001,\"\",text,\"\",ignored\n")).unwrap();
        let inspection = inspect(
            backend,
            CsvInspectionIntent::import(source, target(&schema), CsvOptions::default()).unwrap(),
        )
        .await;
        assert_eq!(
            inspection
                .data()
                .source_columns
                .iter()
                .map(|c| (c.index, c.name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (0, "dup"),
                (1, "dup"),
                (2, ""),
                (3, "notes"),
                (4, "quoted_null"),
                (5, "skip")
            ]
        );
        assert_eq!(inspection.data().sample_rows[0][3], None);
        assert_eq!(inspection.data().sample_rows[0][4].as_deref(), Some("\\N"));
        for (name, identity_column, generated, default) in [
            ("row_id", true, false, false),
            ("body_length", false, true, true),
            ("defaulted", false, false, true),
        ] {
            let column = inspection
                .data()
                .target_columns
                .iter()
                .find(|column| column.name == name)
                .unwrap();
            assert_eq!(column.identity, identity_column);
            assert_eq!(column.generated, generated);
            if default {
                assert!(column.has_default);
            }
        }
        for excluded in ["row_id", "body_length"] {
            let mut invalid = mapping();
            invalid.push(CsvMapping {
                source_index: 5,
                target_column: excluded.into(),
            });
            assert!(matches!(
                backend.review_csv_import(inspection.clone(), invalid),
                Err(CsvError::InvalidMapping)
            ));
        }
        let review = backend.review_csv_import(inspection, mapping()).unwrap();
        let imported = wait(backend, start(backend, review)).await;
        assert_eq!(imported.phase, CsvTransferPhase::Completed, "{imported:?}");
        assert_eq!(imported.effect, CsvEffect::Succeeded);
        assert_eq!(imported.rows_processed, Some(2));
        assert_eq!(imported.rows_committed, Some(2));
        assert_eq!(imported.import_change_revision, Some(1));
        let original = rows(&schema).await;
        assert_eq!(
            original,
            serde_json::json!([
                [
                    "-9223372036854775808",
                    "-0.0000000001",
                    "",
                    "text",
                    "",
                    "owned default",
                    "2",
                    "0"
                ],
                [
                    "9223372036854775807",
                    "12345678901234567890.1234567890",
                    "雪,\n\"quoted\"",
                    null,
                    "\\N",
                    "owned default",
                    "1",
                    "11"
                ]
            ])
        );
        assert_eq!(audit_count(backend).await, 1);

        let inspection = inspect(
            backend,
            CsvInspectionIntent::export(target(&schema), CsvOptions::default()).unwrap(),
        )
        .await;
        let destination = files.path().join("whole-table.csv");
        let review = backend
            .review_csv_export(inspection, destination.clone())
            .unwrap();
        let exported = wait(backend, start(backend, review)).await;
        assert_eq!(exported.phase, CsvTransferPhase::Completed, "{exported:?}");
        assert_eq!(exported.effect, CsvEffect::Succeeded);
        assert_eq!(exported.rows_processed, None);
        assert_eq!(exported.rows_committed, None);
        assert_eq!(
            exported.bytes_processed,
            std::fs::metadata(&destination).unwrap().len()
        );
        assert_eq!(exported_rows(&destination), original);
        println!("owned CSV export bytes: {}", exported.bytes_processed);

        let changed = files.path().join("changed.csv");
        std::fs::write(&changed, format!("{HEADER}7,1,old,\\N,\"\\N\",ignored\n")).unwrap();
        let inspection = inspect(
            backend,
            CsvInspectionIntent::import(changed.clone(), target(&schema), CsvOptions::default())
                .unwrap(),
        )
        .await;
        let review = backend.review_csv_import(inspection, mapping()).unwrap();
        std::fs::rename(&changed, files.path().join("preserved-original.csv")).unwrap();
        std::fs::write(
            &changed,
            format!("{HEADER}8,2,replaced,\\N,\"\\N\",ignored\n"),
        )
        .unwrap();
        let refused = wait(backend, start(backend, review)).await;
        assert_eq!(refused.phase, CsvTransferPhase::Failed, "{refused:?}");
        assert_eq!(refused.effect, CsvEffect::NotApplied);
        assert_eq!(refused.failure, Some(CsvError::SourceChanged));
        assert_eq!(refused.import_change_revision, None);
        assert_eq!(rows(&schema).await, original);

        // The malformed record is beyond the 256 KiB inspection scan. Earlier
        // records enter COPY, but the entire transaction must roll back.
        let malformed = files.path().join("late-malformed.csv");
        let mut input = std::fs::File::create(&malformed).unwrap();
        input.write_all(HEADER.as_bytes()).unwrap();
        let body = "x".repeat(4096);
        for id in 1000..1080 {
            writeln!(input, "{id},1,{body},\\N,\"\\N\",ignored").unwrap();
        }
        writeln!(input, "2000,1,missing-fields").unwrap();
        drop(input);
        assert!(std::fs::metadata(&malformed).unwrap().len() > 256 * 1024);
        let inspection = inspect(
            backend,
            CsvInspectionIntent::import(malformed, target(&schema), CsvOptions::default()).unwrap(),
        )
        .await;
        assert!(inspection.data().sample_truncated);
        let review = backend.review_csv_import(inspection, mapping()).unwrap();
        let rolled_back = wait(backend, start(backend, review)).await;
        assert_eq!(
            rolled_back.phase,
            CsvTransferPhase::Failed,
            "{rolled_back:?}"
        );
        assert_eq!(rolled_back.effect, CsvEffect::NotApplied);
        assert_eq!(rolled_back.failure, Some(CsvError::Csv));
        assert!(rolled_back.rows_processed.is_some_and(|rows| rows > 0));
        assert_eq!(rolled_back.import_change_revision, None);
        assert_eq!(rows(&schema).await, original);
        assert_eq!(audit_count(backend).await, 1);
        let jobs = backend
            .list_csv_transfers(Some(profile::CONNECTION_ID))
            .unwrap();
        assert_eq!(jobs.import_change_revision, 1);
        assert_eq!(jobs.execution_reserved_bytes, 0);
        println!("owned CSV result: imported=2 exported=2 exact_values=true source_replacement_refused=true late_error_rolled_back=true successful_audits=1 import_revision=1");
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(240), operation))
            .catch_unwind()
            .await;
    let joined = match backend.as_ref() {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let helpers_joined = join_sql_helpers().await;
    if joined.is_err() || helpers_joined.is_err() {
        // A failed join is not permission to unlink files/profile state still
        // reachable by the retained backend or an unfinished fixture helper.
        let preserved_files = files.keep();
        let preserved_profile = directory.keep();
        println!(
            "owned CSV residue preserved: files={} profile={}",
            preserved_files.display(),
            preserved_profile.display()
        );
    }
    let cleaned = if joined.is_ok() && helpers_joined.is_ok() {
        cleanup(&schema, &comment, identity.as_ref()).await
    } else {
        Err("Owned CSV objects preserved because backend joins failed".into())
    };
    let cleanup_helpers_joined = join_sql_helpers().await;
    assert!(joined.is_ok(), "{joined:?}");
    assert!(helpers_joined.is_ok(), "{helpers_joined:?}");
    assert!(cleanup_helpers_joined.is_ok(), "{cleanup_helpers_joined:?}");
    assert!(cleaned.is_ok(), "{cleaned:?}");
    let final_activity = activity().await;
    println!("owned CSV final activity: {final_activity}");
    let activity_helper_joined = join_sql_helpers().await;
    assert_eq!(
        final_activity, baseline,
        "Owned probe must restore activity baseline"
    );
    assert!(activity_helper_joined.is_ok(), "Cleanup helpers must join");
    assert!(
        matches!(result, Ok(Ok(()))),
        "Owned CSV probe failed after guarded cleanup"
    );
}

mod xlsx;
