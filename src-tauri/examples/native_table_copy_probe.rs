//! Explicit owned stage03 acceptance only. This example never adopts a profile or
//! table, and never runs without the fixture verification opt-in.
use dbunk_lib::backend::{table_copy::*, *};
use futures_util::FutureExt;
use serde::Deserialize;
use std::{io::Write, path::Path, process::Stdio, sync::Mutex, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
static HELPERS: Mutex<Vec<tokio::task::JoinHandle<()>>> = Mutex::new(Vec::new());
fn qi(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn ql(s: &str) -> String {
    format!("E'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
}
async fn sql(query: String) -> Result<String, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = tokio::task::spawn_blocking(move || {
        let result = (|| {
            let mut child=std::process::Command::new("python3").arg("-c").arg("import os,sys; sys.path.insert(0,sys.argv[1]); import fixture; assert os.environ.get('DBUNK_NATIVE_FIXTURE_VERIFIED')=='1'; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,\"SET statement_timeout='15s'; SET lock_timeout='3s'; \"+sys.stdin.read()))").arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_|"Fixture helper failed to start")?;
            let written = child
                .stdin
                .take()
                .ok_or("Missing helper stdin")
                .and_then(|mut input| {
                    input
                        .write_all(query.as_bytes())
                        .map_err(|_| "Fixture helper input failed")
                });
            if let Err(error) = written {
                let _ = child.kill();
                child.wait().map_err(|_| "Fixture helper did not join")?;
                return Err(error.to_owned());
            }
            let result = child
                .wait_with_output()
                .map_err(|_| "Fixture helper did not join")?;
            if !result.status.success() {
                return Err("Identity-checked fixture SQL refused or failed".into());
            }
            String::from_utf8(result.stdout)
                .map(|s| s.trim().to_owned())
                .map_err(|_| "Invalid fixture helper output".into())
        })();
        let _ = send.send(result);
    });
    HELPERS.lock().unwrap().push(handle);
    receive
        .await
        .map_err(|_| "Fixture helper panicked".to_string())?
}
async fn join_helpers() -> Result<(), String> {
    let handles = std::mem::take(&mut *HELPERS.lock().unwrap());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut failed = false;
    for mut handle in handles {
        match tokio::time::timeout_at(deadline, &mut handle).await {
            Ok(result) => failed |= result.is_err(),
            Err(_) => {
                HELPERS.lock().unwrap().push(handle);
                failed = true;
            }
        }
    }
    if failed {
        Err("Fixture helper cleanup unresolved; objects preserved".into())
    } else {
        Ok(())
    }
}
async fn activity() -> Result<u64, String> {
    sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await?.parse().map_err(|_|"Invalid backend count".into())
}
#[derive(Deserialize)]
struct Identities {
    schema: u32,
    function: u32,
    tables: Vec<TableIdentity>,
}
#[derive(Deserialize)]
struct TableIdentity {
    name: String,
    oid: u32,
}
struct Owned {
    schema: String,
    comment: String,
    identities: Identities,
}
async fn setup(schema: String, comment: String) -> Result<Owned, String> {
    if sql(format!(
        "SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname={}",
        ql(&schema)
    ))
    .await?
        != "0"
    {
        return Err("Unique probe schema already exists".into());
    }
    let q = qi(&schema);
    let mark = ql(&comment);
    let result=sql(format!(r#"BEGIN;
CREATE SCHEMA {q}; COMMENT ON SCHEMA {q} IS {mark};
CREATE TABLE {q}.source(id bigint, n numeric(40,10), body text, nullable text);
INSERT INTO {q}.source VALUES (9223372036854775807,123456789012345678901234567890.1234567890,'NULL',NULL),(-9223372036854775808,-0.0000000001,'','NULL');
CREATE TABLE {q}.destination(id bigint GENERATED ALWAYS AS IDENTITY, n numeric(40,10), body text, nullable text, defaulted text NOT NULL DEFAULT 'owned default', body_length integer GENERATED ALWAYS AS(length(body)) STORED);
CREATE TABLE {q}.failure_source(id integer, body text);
INSERT INTO {q}.failure_source VALUES(1,'first'),(1,'second');
CREATE TABLE {q}.failure_destination(id integer UNIQUE,body text);
CREATE TABLE {q}.slow_destination(id bigint,n numeric(40,10),body text,nullable text);
CREATE FUNCTION {q}.slow() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN PERFORM pg_catalog.pg_sleep(1); RETURN NEW; END';
CREATE TRIGGER owned_slow BEFORE INSERT ON {q}.slow_destination FOR EACH ROW EXECUTE FUNCTION {q}.slow();
COMMENT ON FUNCTION {q}.slow() IS {mark};
COMMENT ON TABLE {q}.source IS {mark}; COMMENT ON TABLE {q}.destination IS {mark};
COMMENT ON TABLE {q}.failure_source IS {mark}; COMMENT ON TABLE {q}.failure_destination IS {mark}; COMMENT ON TABLE {q}.slow_destination IS {mark};
COMMIT;
SELECT pg_catalog.json_build_object('schema',n.oid::bigint,'function',(SELECT p.oid::bigint FROM pg_catalog.pg_proc p WHERE p.pronamespace=n.oid AND p.proname='slow' AND p.pronargs=0),'tables',(SELECT pg_catalog.json_agg(pg_catalog.json_build_object('name',c.relname,'oid',c.oid::bigint) ORDER BY c.relname) FROM pg_catalog.pg_class c WHERE c.relnamespace=n.oid AND c.relkind='r'))::text FROM pg_catalog.pg_namespace n WHERE n.nspname={};"#,ql(&schema))).await?;
    let identities: Identities =
        serde_json::from_str(&result).map_err(|_| "Probe ownership identity response invalid")?;
    if identities.tables.len() != 5 {
        return Err("Probe table identity set differs".into());
    }
    println!(
        "OWNED {}",
        serde_json::json!({"schema":schema,"schema_oid":identities.schema,"function_oid":identities.function,"table_oids":identities.tables.iter().map(|t|serde_json::json!({"name":t.name,"oid":t.oid})).collect::<Vec<_>>(),"fixture":FIXTURE})
    );
    Ok(Owned {
        schema,
        comment,
        identities,
    })
}
async fn cleanup(owned: &Owned) -> Result<(), String> {
    let schema = &owned.schema;
    let mark = ql(&owned.comment);
    let q = qi(schema);
    let oid = owned.identities.schema;
    let mut body=format!("IF NOT EXISTS (SELECT FROM pg_catalog.pg_namespace WHERE oid={oid} AND nspname={} AND pg_catalog.pg_get_userbyid(nspowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_namespace')={mark}) THEN RAISE EXCEPTION 'Schema ownership changed'; END IF;",ql(schema));
    for table in &owned.identities.tables {
        body+=&format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_class WHERE oid={} AND relnamespace={oid} AND relname={} AND relkind='r' AND pg_catalog.pg_get_userbyid(relowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_class')={mark}) THEN RAISE EXCEPTION 'Table ownership changed'; END IF;",table.oid,ql(&table.name));
    }
    body+=&format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_proc WHERE oid={} AND pronamespace={oid} AND proname='slow' AND pronargs=0 AND pg_catalog.pg_get_userbyid(proowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_proc')={mark}) THEN RAISE EXCEPTION 'Function ownership changed'; END IF;",owned.identities.function);
    // Every DROP is exact, guarded, and RESTRICT. An unexpected dependency or
    // additional schema object makes this whole cleanup transaction roll back.
    for table in &owned.identities.tables {
        body += &format!(
            "EXECUTE {};",
            ql(&format!("DROP TABLE {q}.{} RESTRICT", qi(&table.name)))
        );
    }
    body += &format!(
        "EXECUTE {}; EXECUTE {};",
        ql(&format!("DROP FUNCTION {q}.slow() RESTRICT")),
        ql(&format!("DROP SCHEMA {q} RESTRICT"))
    );
    sql(format!(
        "BEGIN; DO $guard$ BEGIN {body} END $guard$; COMMIT;"
    ))
    .await?;
    if sql(format!(
        "SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname={}",
        ql(schema)
    ))
    .await?
        != "0"
    {
        return Err("Owned schema remains after cleanup".into());
    }
    Ok(())
}
fn intent(connection: &str, schema: &str, source: &str, destination: &str) -> TableCopyIntent {
    TableCopyIntent::new(
        TableCopyEndpoint {
            connection_id: connection.into(),
            schema: schema.into(),
            table: source.into(),
        },
        TableCopyEndpoint {
            connection_id: connection.into(),
            schema: schema.into(),
            table: destination.into(),
        },
    )
    .unwrap()
}
async fn observed(
    backend: &Backend,
    id: TableCopyAttemptId,
    predicate: impl Fn(&TableCopyObservation) -> bool,
) -> Result<TableCopyObservation, String> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let row = backend.get_table_copy(id).map_err(|e| e.to_string())?;
            if predicate(&row) {
                return Ok(row);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| "Table copy observation timed out".to_string())?
}
async fn reviewed(backend: &Backend, intent: TableCopyIntent) -> Result<TableCopyReview, String> {
    sql("SELECT current_database()".into()).await?;
    let id = TableCopyAttemptId::new();
    backend
        .begin_table_copy(id, intent)
        .map_err(|e| e.to_string())?;
    let row = observed(backend, id, |o| {
        o.phase == TableCopyPhase::ReadyReview || o.phase.terminal()
    })
    .await?;
    if row.phase != TableCopyPhase::ReadyReview {
        return Err(format!("Copy review refused: {:?}", row.failure));
    }
    backend.review_table_copy(id).map_err(|e| e.to_string())
}
async fn submit(backend: &Backend, review: TableCopyReview) -> Result<TableCopyAttemptId, String> {
    sql("SELECT current_database()".into()).await?;
    let id = review.attempt_id();
    let description = review.description().clone();
    let TableCopySubmission::NeedsConfirmation(confirmation) = backend
        .start_table_copy(review)
        .map_err(|e| e.to_string())?
    else {
        return Err("Strict destination did not ask for confirmation".into());
    };
    if confirmation.attempt_id() != id || *confirmation.review().description() != description {
        return Err("Confirmation changed exact reviewed identity".into());
    }
    backend
        .confirm_table_copy(*confirmation)
        .map_err(|e| e.to_string())?;
    Ok(id)
}
async fn completed(backend: &Backend, review: TableCopyReview, rows: u64) -> Result<(), String> {
    let description = review.description().clone();
    let id = submit(backend, review).await?;
    let result = observed(backend, id, |o| {
        o.phase.terminal() && o.cleanup != TableCopyCleanup::Pending
    })
    .await?;
    if result.outcome != (TableCopyOutcome::Completed { rows })
        || result.cleanup != TableCopyCleanup::Complete
    {
        return Err(format!("Copy terminal result differed: {:?}", result));
    }
    let receipt = result.receipt.ok_or("Missing exact receipt")?;
    if receipt.attempt_id != id
        || receipt.description != description
        || receipt.outcome != result.outcome
    {
        return Err("Receipt identity changed".into());
    }
    Ok(())
}
async fn exercise(backend: &Backend, connection: &str, owned: &Owned) -> Result<(), String> {
    let schema = &owned.schema;
    let q = qi(schema);
    let review = reviewed(backend, intent(connection, schema, "source", "destination")).await?;
    if review.description().identity_columns != 1
        || review.description().generated_columns != 1
        || review.description().defaulted_columns != 1
    {
        return Err("Identity/generated/default review differs".into());
    }
    completed(backend, review, 2).await?;
    let exact=sql(format!("SELECT count(*)=2 AND bool_and((id=9223372036854775807 AND n=123456789012345678901234567890.1234567890 AND body='NULL' AND nullable IS NULL AND body_length=4) OR (id=(-9223372036854775808)::bigint AND n=-0.0000000001 AND body='' AND nullable='NULL' AND body_length=0)) AND bool_and(defaulted='owned default') FROM {q}.destination")).await?;
    if exact != "t" {
        return Err("Exact numeric/null/default/generated result differed".into());
    }
    completed(
        backend,
        reviewed(backend, intent(connection, schema, "source", "source")).await?,
        2,
    )
    .await?;
    if sql(format!("SELECT count(*) FROM {q}.source")).await? != "4" {
        return Err("Self-copy snapshot was not finite/exact".into());
    }
    let failure = submit(
        backend,
        reviewed(
            backend,
            intent(connection, schema, "failure_source", "failure_destination"),
        )
        .await?,
    )
    .await?;
    let failed = observed(backend, failure, |o| o.phase.terminal()).await?;
    if failed.outcome != TableCopyOutcome::RolledBack
        || failed
            .diagnostic
            .as_ref()
            .and_then(|d| d.sqlstate.as_deref())
            != Some("23505")
        || sql(format!("SELECT count(*) FROM {q}.failure_destination")).await? != "0"
    {
        return Err("Late duplicate constraint did not atomically roll back".into());
    }
    let stale = reviewed(backend, intent(connection, schema, "source", "destination")).await?;
    sql(format!(
        "ALTER TABLE {q}.destination ADD COLUMN changed_after_review text"
    ))
    .await?;
    let stale = submit(backend, stale).await?;
    let result = observed(backend, stale, |o| o.phase.terminal()).await?;
    if result.failure != Some(TableCopyError::TargetChanged)
        || result.outcome == TableCopyOutcome::OutcomeUnknown
    {
        return Err("Stale target metadata did not refuse before COMMIT".into());
    }
    let mut foreign = intent(connection, schema, "source", "destination");
    foreign.source.connection_id = uuid::Uuid::new_v4().to_string();
    let foreign_id = TableCopyAttemptId::new();
    backend
        .begin_table_copy(foreign_id, foreign)
        .map_err(|e| e.to_string())?;
    let result = observed(backend, foreign_id, |o| o.phase.terminal()).await?;
    if result.outcome != TableCopyOutcome::NotStarted
        || result.failure != Some(TableCopyError::StaleReview)
    {
        return Err("Foreign connection authority did not refuse".into());
    }
    let slow = submit(
        backend,
        reviewed(
            backend,
            intent(connection, schema, "source", "slow_destination"),
        )
        .await?,
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(10),async{loop{if sql(format!("SELECT EXISTS(SELECT FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid() AND query LIKE {} AND state='active')",ql(&format!("COPY {q}.\"slow_destination\"%")))).await?=="t"{return Ok::<_,String>(());}tokio::time::sleep(Duration::from_millis(20)).await;}}).await.map_err(|_|"Did not observe owned in-flight destination COPY")??;
    backend.cancel_table_copy(slow).map_err(|e| e.to_string())?;
    let stopped = observed(backend, slow, |o| {
        o.phase.terminal() && o.cleanup != TableCopyCleanup::Pending
    })
    .await?;
    if stopped.outcome != TableCopyOutcome::RolledBack
        || stopped.cleanup != TableCopyCleanup::Complete
        || sql(format!("SELECT count(*) FROM {q}.slow_destination")).await? != "0"
    {
        return Err("Cancelled copy did not roll back and join".into());
    }
    let audit = backend
        .load_safety_audit(connection.to_owned(), None)
        .await
        .map_err(|_| "Audit read failed")?;
    let count = audit
        .rows
        .iter()
        .filter(|r| r.command == "native_table_copy")
        .count();
    if count != 2 {
        return Err("Successful strict copies did not audit exactly twice".into());
    }
    println!("PASS exact values, identity/default/generated review, finite self-copy, late rollback, stale target/authority, in-flight cancel, success-only audit");
    Ok(())
}
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Owned table-copy probe: {error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    if std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref() != Ok("1") {
        return Err("Requires explicitly verified owned stage03 fixture".into());
    }
    if sql("SELECT current_database()".into()).await? != "dbunk_demo" || activity().await? != 0 {
        return Err("Fixture database/idle baseline differs".into());
    }
    let directory = tempfile::tempdir().map_err(|_| "Private profile allocation failed")?;
    let profile = directory
        .path()
        .canonicalize()
        .map_err(|_| "Private profile path is not canonical")?
        .join("profile");
    println!("TARGET private disposable profile {}", profile.display());
    let fixtures=DevelopmentFixtures::from_json(&serde_json::json!({"version":1,"fixture":"dbunk-native-stage03","instance":FIXTURE,"host":"127.0.0.1","port":15432,"database":"dbunk_demo","user":"dbunk"}).to_string())?;
    let backend = Backend::create_development(&profile, fixtures).await?;
    let setup_result = async {
        backend
            .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
            .await?;
        backend
            .save_development_connection(
                None,
                DevelopmentPostgresConnection {
                    name: "Owned copy probe".into(),
                    host: "127.0.0.1".into(),
                    port: 15432,
                    database: "dbunk_demo".into(),
                    user: "dbunk".into(),
                    environment: DevelopmentEnvironment::Test,
                    safe_mode: DevelopmentSafeMode::Strict,
                    read_only: false,
                    tls: Default::default(),
                    driver_options: Default::default(),
                    ssh_tunnel: None,
                },
                "dbunk".into(),
            )
            .await
    }
    .await;
    let connection = match setup_result {
        Ok(c) => c,
        Err(error) => {
            backend.shutdown().await?;
            return Err(error);
        }
    };
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let schema = format!("native_copy_{suffix}");
    let comment = format!("dbunk owned table-copy probe {suffix}");
    println!(
        "TARGET owned stage03 {FIXTURE} schema {schema}; five tables and slow trigger function"
    );
    let owned = match setup(schema.clone(), comment).await {
        Ok(owned) => owned,
        Err(error) => {
            backend.shutdown().await?;
            join_helpers().await?;
            return Err(format!("{error}; ownership setup failed, inspect unique schema {schema} without adopting it"));
        }
    };
    let result = std::panic::AssertUnwindSafe(async {
        tokio::time::timeout(
            Duration::from_secs(120),
            exercise(&backend, &connection.id, &owned),
        )
        .await
        .map_err(|_| "Copy probe timed out".to_string())?
    })
    .catch_unwind()
    .await
    .unwrap_or_else(|_| Err("Copy probe assertion panicked".into()));
    // Stop/join every app-owned attempt before any cleanup SQL. No setup-tab
    // lifetime or dropped waiter is treated as proof that COPY stopped.
    let shutdown = backend.shutdown().await;
    let joined = join_helpers().await;
    shutdown?;
    joined?;
    cleanup(&owned).await?;
    join_helpers().await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while activity().await? != 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "Activity did not return to zero")??;
    result?;
    println!("PASS owned cleanup RESTRICT, joined shutdown, activity 0→0");
    Ok(())
}
