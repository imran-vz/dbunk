//! Ignored owned stage03 metadata probe. Setup is explicit and cleanup is OID/owner/marker guarded.
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
    function: u32,
}
#[derive(serde::Deserialize, serde::Serialize)]
struct Object {
    name: String,
    oid: u32,
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture only; creates unique objects with guarded RESTRICT cleanup"]
async fn native_structure_owned_complete_metadata_and_identity() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    assert_eq!(
        sql("SELECT current_database()".into()).await.unwrap(),
        "dbunk_demo"
    );
    assert_eq!(sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await.unwrap(), "0");
    let schema = format!("structure_{}", uuid::Uuid::new_v4().simple());
    let marker = format!("dbunk native structure owned {}", uuid::Uuid::new_v4());
    let q = qi(&schema);
    let mark = ql(&marker);
    println!(
        "STRUCTURE_CREATE_INTENT {}",
        serde_json::json!({"fixture":FIXTURE,"schema":schema,"marker":marker})
    );
    let setup = sql(format!(r#"BEGIN;
CREATE SCHEMA {q}; COMMENT ON SCHEMA {q} IS {mark};
CREATE TABLE {q}.parent(a integer,b integer,PRIMARY KEY(a,b));
CREATE TABLE {q}.child(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,x integer,y integer,junk text,body text DEFAULT '',calc integer GENERATED ALWAYS AS(x+1)STORED,CONSTRAINT pair FOREIGN KEY(y,x)REFERENCES {q}.parent(a,b)ON DELETE RESTRICT DEFERRABLE,CONSTRAINT positive CHECK(x>=0));
ALTER TABLE {q}.child DROP COLUMN junk;
COMMENT ON COLUMN {q}.child.body IS 'column 雪';
CREATE INDEX expression_include ON {q}.child(lower(body))INCLUDE(x)WHERE body IS NOT NULL;
CREATE FUNCTION {q}.touch()RETURNS trigger LANGUAGE plpgsql AS $body$ BEGIN RETURN NEW;END $body$;
COMMENT ON FUNCTION {q}.touch() IS {mark};
CREATE TRIGGER changed BEFORE UPDATE OF x,y ON {q}.child FOR EACH ROW EXECUTE FUNCTION {q}.touch();
ALTER TABLE {q}.child ENABLE ROW LEVEL SECURITY;
CREATE POLICY policy ON {q}.child TO PUBLIC USING(true)WITH CHECK(x>=0);
GRANT SELECT ON {q}.child TO PUBLIC;
CREATE RULE ignore_delete AS ON DELETE TO {q}.child DO INSTEAD NOTHING;
CREATE TABLE {q}.partitioned(k integer)PARTITION BY RANGE(k);
CREATE TABLE {q}.leaf PARTITION OF {q}.partitioned FOR VALUES FROM(0)TO(10);
CREATE TABLE {q}.inherited(extra text)INHERITS({q}.parent);
COMMENT ON TABLE {q}.parent IS {mark}; COMMENT ON TABLE {q}.child IS {mark}; COMMENT ON TABLE {q}.partitioned IS {mark}; COMMENT ON TABLE {q}.leaf IS {mark}; COMMENT ON TABLE {q}.inherited IS {mark}; COMMIT;
SELECT pg_catalog.json_build_object('schema',n.oid::bigint,'function',(SELECT p.oid::bigint FROM pg_catalog.pg_proc p WHERE p.pronamespace=n.oid AND p.proname='touch'),'tables',(SELECT pg_catalog.json_agg(pg_catalog.json_build_object('name',c.relname,'oid',c.oid::bigint)ORDER BY c.relname)FROM pg_catalog.pg_class c WHERE c.relnamespace=n.oid AND c.relkind IN('r','p')))::text FROM pg_catalog.pg_namespace n WHERE n.nspname={};"#, ql(&schema))).await.unwrap();
    let owned: Objects = serde_json::from_str(&setup).unwrap();
    assert_eq!(owned.tables.len(), 5);
    println!("STRUCTURE_OWNED {}", serde_json::to_string(&owned).unwrap());
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let result = std::panic::AssertUnwindSafe(async {
        let document = backend
            .open_data_document("structure-live", "structure", &backend.fixture().id)
            .await
            .unwrap();
        let request = |table: &str| TableStructureRequest {
            schema: schema.clone(),
            table: table.into(),
            expected: None,
        };
        let child = backend
            .table_structure(&document, request("child"))
            .await
            .unwrap();
        assert!(child.checked_heap_bytes().is_some());
        assert_eq!(
            child.identity.relation_oid,
            owned.tables.iter().find(|t| t.name == "child").unwrap().oid
        );
        assert_eq!(
            child.columns.iter().map(|c| c.number).collect::<Vec<_>>(),
            [1, 2, 3, 5, 6]
        );
        assert_eq!(child.columns[0].identity, StructureIdentityKind::Always);
        assert_eq!(child.columns[3].comment.as_deref(), Some("column 雪"));
        assert!(child.columns[1].comment.is_none());
        assert_eq!(child.columns[4].generated, StructureGeneratedKind::Stored);
        assert_eq!(
            child.outbound[0]
                .columns
                .iter()
                .map(|p| p.source.as_str())
                .collect::<Vec<_>>(),
            ["y", "x"]
        );
        assert_eq!(
            child.outbound[0]
                .columns
                .iter()
                .map(|p| p.target.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let index = child
            .indexes
            .iter()
            .find(|i| i.name == "expression_include")
            .unwrap();
        assert!(index.keys[0].column_number.is_none());
        assert!(index.keys[1].included);
        assert_eq!(index.keys[1].column_name.as_deref(), Some("x"));
        assert!(index.predicate.is_some());
        assert_eq!(
            child.triggers[0]
                .update_columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["x", "y"]
        );
        assert!(child.row_security.enabled);
        assert_eq!(child.policies[0].roles, ["public"]);
        assert!(child
            .privileges
            .iter()
            .any(|p| p.grantee == "PUBLIC" && p.privilege == "SELECT"));
        assert!(child.rules.iter().any(|r| r.name == "ignore_delete"));
        let parent = backend
            .table_structure(&document, request("parent"))
            .await
            .unwrap();
        assert_eq!(parent.inbound[0].source_table, "child");
        assert!(parent
            .partitions
            .iter()
            .any(|r| r.name == "inherited" && !r.is_partition));
        let leaf = backend
            .table_structure(&document, request("leaf"))
            .await
            .unwrap();
        assert!(leaf.is_partition && leaf.partition_bound.is_some());
        assert_eq!(leaf.parents[0].name, "partitioned");
        let mut changed = request("child");
        changed.expected = Some(TableIdentity {
            database_oid: child.identity.database_oid,
            relation_oid: parent.identity.relation_oid,
        });
        assert!(matches!(
            backend.table_structure(&document, changed).await,
            Err(DataError::Catalog(
                crate::postgres::native_catalog::CatalogError::StructureIdentityChanged
            ))
        ));
        backend.close_data_document(&document).await.unwrap();
        assert!(backend
            .table_structure(&document, request("child"))
            .await
            .is_err());
    })
    .catch_unwind()
    .await;
    backend.shutdown().await.unwrap();
    join_sql_helpers().await.unwrap();
    assert_eq!(sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await.unwrap(),"0");
    let mut guard = format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_namespace WHERE oid={} AND nspname={} AND pg_catalog.pg_get_userbyid(nspowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_namespace')={mark})THEN RAISE EXCEPTION 'Schema ownership changed';END IF;",owned.schema,ql(&schema));
    for table in &owned.tables {
        guard += &format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_class WHERE oid={} AND relnamespace={} AND relname={} AND relkind IN('r','p') AND pg_catalog.pg_get_userbyid(relowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_class')={mark})THEN RAISE EXCEPTION 'Table ownership changed';END IF;",table.oid,owned.schema,ql(&table.name));
    }
    guard += &format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_proc WHERE oid={} AND pronamespace={} AND proname='touch' AND pg_catalog.pg_get_userbyid(proowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_proc')={mark})THEN RAISE EXCEPTION 'Function ownership changed';END IF;",owned.function,owned.schema);
    for name in ["child", "inherited", "leaf", "partitioned", "parent"] {
        guard += &format!(
            "EXECUTE {};",
            ql(&format!("DROP TABLE {q}.{} RESTRICT", qi(name)))
        );
    }
    guard += &format!(
        "EXECUTE {};EXECUTE {};",
        ql(&format!("DROP FUNCTION {q}.touch() RESTRICT")),
        ql(&format!("DROP SCHEMA {q} RESTRICT"))
    );
    sql(format!("BEGIN;DO $guard$ BEGIN {guard} END $guard$;COMMIT"))
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
        "STRUCTURE_CLEANUP {}",
        serde_json::json!({"schema":schema,"absent":true,"joined":true})
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
