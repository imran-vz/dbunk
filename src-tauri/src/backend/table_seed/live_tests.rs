//! Explicit opt-in owned stage03 acceptance; never part of ordinary tests.
use super::*;
use crate::backend::profile;
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
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

fn qi(s: &str) -> String {
    crate::quote_double(s)
}
fn ql(s: &str) -> String {
    crate::quote_literal(s)
}
#[derive(serde::Deserialize, serde::Serialize)]
struct Objects {
    schema: u32,
    tables: Vec<Object>,
}
#[derive(serde::Deserialize, serde::Serialize)]
struct Object {
    name: String,
    oid: u32,
}
async fn wait(backend: &Backend, id: TableSeedAttemptId) -> TableSeedObservation {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let o = backend.get_table_seed(id).unwrap();
            if o.phase != TableSeedPhase::Preparing
                && !matches!(
                    o.phase,
                    TableSeedPhase::Running
                        | TableSeedPhase::Committing
                        | TableSeedPhase::Cancelling
                )
            {
                return o;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn discard(backend: &Backend, id: TableSeedAttemptId) {
    backend.cancel_table_seed(id).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if backend.release_table_seed(id).is_ok() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn intent(
    schema: &str,
    table: &str,
    count: u32,
    columns: Vec<TableSeedColumnSpec>,
) -> TableSeedIntent {
    TableSeedIntent::new(
        TableSeedEndpoint {
            connection_id: profile::CONNECTION_ID.into(),
            schema: schema.into(),
            table: table.into(),
        },
        count,
        Some(u64::MAX),
        columns,
    )
    .unwrap()
}
async fn prepare(
    backend: &Backend,
    intent: TableSeedIntent,
) -> (TableSeedAttemptId, TableSeedObservation) {
    let id = TableSeedAttemptId::new();
    backend.begin_table_seed(id, intent).unwrap();
    (id, wait(backend, id).await)
}
fn apply(backend: &Backend, id: TableSeedAttemptId) {
    match backend
        .start_table_seed(backend.review_table_seed(id).unwrap())
        .unwrap()
    {
        TableSeedSubmission::Accepted(_) => {}
        TableSeedSubmission::NeedsConfirmation(c) => {
            assert!(matches!(
                backend.confirm_table_seed(*c).unwrap(),
                TableSeedSubmission::Accepted(_)
            ));
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicit verified owned stage03 PostgreSQL fixture"]
async fn native_seed_owned_transaction_recipe_and_composite_fk() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    assert_eq!(
        sql("SELECT current_database()".into()).await.unwrap(),
        "dbunk_demo"
    );
    let schema = format!("seed_{}", uuid::Uuid::new_v4().simple());
    let comment = format!("dbunk native seed owned {}", uuid::Uuid::new_v4());
    let q = qi(&schema);
    let mark = ql(&comment);
    assert_eq!(sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await.unwrap(),"0");
    println!(
        "SEED_CREATE_INTENT {}",
        serde_json::json!({"fixture":FIXTURE,"schema":schema,"marker":comment})
    );
    let setup=sql(format!(r#"BEGIN; CREATE SCHEMA {q}; COMMENT ON SCHEMA {q} IS {mark};
CREATE TABLE {q}.parent(a integer,b integer,PRIMARY KEY(a,b)); INSERT INTO {q}.parent VALUES(1,10),(2,20);
CREATE TABLE {q}.payload(id bigint GENERATED ALWAYS AS IDENTITY,a integer NOT NULL,b integer NOT NULL,body text NOT NULL,defaulted text DEFAULT 'owned default',derived integer GENERATED ALWAYS AS(length(body)) STORED,FOREIGN KEY(a,b) REFERENCES {q}.parent(a,b));
CREATE TABLE {q}.unsupported(value point NOT NULL);
CREATE TABLE {q}.late_failure(id integer UNIQUE CHECK(id<=550));
COMMENT ON TABLE {q}.parent IS {mark}; COMMENT ON TABLE {q}.payload IS {mark}; COMMENT ON TABLE {q}.unsupported IS {mark}; COMMENT ON TABLE {q}.late_failure IS {mark}; COMMIT;
SELECT pg_catalog.json_build_object('schema',n.oid::bigint,'tables',(SELECT pg_catalog.json_agg(pg_catalog.json_build_object('name',c.relname,'oid',c.oid::bigint)ORDER BY c.relname)FROM pg_catalog.pg_class c WHERE c.relnamespace=n.oid AND c.relkind='r'))::text FROM pg_catalog.pg_namespace n WHERE n.nspname={};"#,ql(&schema))).await.unwrap();
    let owned: Objects = serde_json::from_str(&setup).unwrap();
    assert_eq!(owned.tables.len(), 4);
    println!("SEED_OWNED {}", serde_json::to_string(&owned).unwrap());
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let result=std::panic::AssertUnwindSafe(async{
        let(id,o)=prepare(&backend,intent(&schema,"unsupported",100,vec![])).await;assert_eq!(o.phase,TableSeedPhase::NeedsRecipe);assert_eq!(backend.inspect_table_seed(id).unwrap().columns()[0].data_type,"point");assert!(backend.review_table_seed(id).is_err());discard(&backend,id).await;
        let columns=vec![TableSeedColumnSpec{column:"body".into(),source:TableSeedSource::Constant{value:"owned".into()},null_rate:None},TableSeedColumnSpec{column:"defaulted".into(),source:TableSeedSource::Default,null_rate:None}];
        let(id,o)=prepare(&backend,intent(&schema,"payload",25,columns.clone())).await;assert_eq!(o.phase,TableSeedPhase::ReadyReview);let review=backend.review_table_seed(id).unwrap();assert_eq!(review.description().seed_used,u64::MAX);assert!(review.columns().iter().filter(|c|c.generated||c.identity).all(|c|c.action==TableSeedColumnAction::Default));drop(review);apply(&backend,id);let done=wait(&backend,id).await;assert_eq!(done.outcome,TableSeedOutcome::Completed{rows:25});discard(&backend,id).await;
        assert_eq!(sql(format!("SELECT count(*) FROM {q}.payload c JOIN {q}.parent p USING(a,b) WHERE c.defaulted='owned default' AND c.derived=5")).await.unwrap(),"25");
        let(id,o)=prepare(&backend,intent(&schema,"late_failure",600,vec![])).await;assert_eq!(o.phase,TableSeedPhase::ReadyReview);apply(&backend,id);assert_eq!(wait(&backend,id).await.outcome,TableSeedOutcome::RolledBack);discard(&backend,id).await;assert_eq!(sql(format!("SELECT count(*) FROM {q}.late_failure")).await.unwrap(),"0");
        let(id,o)=prepare(&backend,intent(&schema,"payload",1,columns)).await;assert_eq!(o.phase,TableSeedPhase::ReadyReview);sql(format!("ALTER TABLE {q}.payload ADD COLUMN changed text")).await.unwrap();apply(&backend,id);assert_eq!(wait(&backend,id).await.failure,Some(TableSeedError::TargetChanged));discard(&backend,id).await;
    }).catch_unwind().await;
    backend.shutdown().await.unwrap();
    join_sql_helpers().await.unwrap();
    assert_eq!(sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await.unwrap(),"0");
    let mut guard=format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_namespace WHERE oid={} AND nspname={} AND pg_catalog.pg_get_userbyid(nspowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_namespace')={mark})THEN RAISE EXCEPTION 'Schema ownership changed';END IF;",owned.schema,ql(&schema));
    for t in &owned.tables {
        guard+=&format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_class WHERE oid={} AND relnamespace={} AND relname={} AND relkind='r' AND pg_catalog.pg_get_userbyid(relowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_class')={mark})THEN RAISE EXCEPTION 'Table ownership changed';END IF;",t.oid,owned.schema,ql(&t.name));
    }
    for name in ["payload", "late_failure", "unsupported", "parent"] {
        guard += &format!(
            "EXECUTE {};",
            ql(&format!("DROP TABLE {q}.{} RESTRICT", qi(name)))
        );
    }
    guard += &format!("EXECUTE {};", ql(&format!("DROP SCHEMA {q} RESTRICT")));
    sql(format!(
        "BEGIN; DO $guard$ BEGIN {guard} END $guard$; COMMIT"
    ))
    .await
    .unwrap();
    join_sql_helpers().await.unwrap();
    assert_eq!(
        sql(format!(
            "SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname={}",
            ql(&schema)
        ))
        .await
        .unwrap(),
        "0"
    );
    join_sql_helpers().await.unwrap();
    println!(
        "SEED_CLEANUP {}",
        serde_json::json!({"schema":schema,"absent":true,"joined":true})
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
