//! Explicit owned stage03 whole-relation capture probe, with marked OID cleanup.
use super::*;
use crate::{backend::profile, postgres::native_catalog::CatalogError};
use futures_util::FutureExt;
use std::{path::Path, process::Command};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
fn fixture_idle() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let result = Command::new("python3")
        .arg("-c")
        .arg("import sys;sys.path.insert(0,sys.argv[1]);import fixture;owned,target=fixture.check();assert owned['instance']==sys.argv[2];assert fixture.sql(target,\"SET statement_timeout='5s'; SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()\").strip()=='0';print('verified owned fixture idle: '+owned['instance'])")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native"))
        .arg(FIXTURE)
        .output()
        .expect("fixture ownership helper");
    assert!(
        result.status.success(),
        "Owned fixture identity/activity check refused"
    );
    println!("{}", String::from_utf8(result.stdout).unwrap());
}
const OWNED_SQL: &str = r#"import sys,signal
signal.signal(signal.SIGALRM,lambda *_: (_ for _ in ()).throw(TimeoutError('owned table export helper deadline')))
signal.alarm(25)
sys.path.insert(0,sys.argv[1])
import fixture
owned,target=fixture.check()
assert owned['instance']==sys.argv[2]
print(fixture.sql(target,sys.stdin.read()))
"#;
const TABLE: &str = "table \"雪";
const VIEW: &str = "view.dot";
const MATVIEW: &str = "materialized 雪";
const EMPTY: &str = "empty";
const OVERSIZE: &str = "oversize";
const PARTITIONED: &str = "partition root";
const PARTITION: &str = "partition.0";
const RELATIONS: [(&str, &str, &str); 7] = [
    (TABLE, "TABLE", "r"),
    (VIEW, "VIEW", "v"),
    (MATVIEW, "MATERIALIZED VIEW", "m"),
    (EMPTY, "VIEW", "v"),
    (OVERSIZE, "VIEW", "v"),
    (PARTITIONED, "TABLE", "p"),
    (PARTITION, "TABLE", "r"),
];
struct CaptureProbe {
    schema: String,
    marker: String,
}
impl CaptureProbe {
    fn qualified(&self, name: &str) -> String {
        format!(
            "{}.{}",
            crate::quote_double(&self.schema),
            crate::quote_double(name)
        )
    }
    // Synchronous, bounded helper lifetime: every spawned process is waited,
    // including stdin failure. No detached process or timeout-aborted waiter.
    fn sql(&self, sql: &str) -> Result<String, String> {
        use std::{io::Write, process::Stdio};
        let mut child = Command::new("python3")
            .arg("-c")
            .arg(OWNED_SQL)
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native"))
            .arg(FIXTURE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let sql = format!(
            "SET application_name={}; SET statement_timeout='10s'; SET lock_timeout='2s'; {sql}",
            crate::quote_literal(&self.schema)
        );
        let written = child
            .stdin
            .take()
            .ok_or("Missing helper stdin".to_owned())
            .and_then(|mut stdin| stdin.write_all(sql.as_bytes()).map_err(|e| e.to_string()));
        if let Err(error) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        if output.stdout.len() > 32768 {
            return Err("Owned table export helper receipt exceeds bound".into());
        }
        String::from_utf8(output.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|e| e.to_string())
    }
    fn setup(&self) -> Result<(), String> {
        let schema = crate::quote_double(&self.schema);
        let marker = crate::quote_literal(&self.marker);
        let table = self.qualified(TABLE);
        let root = self.qualified(PARTITIONED);
        let mut sql = format!(
            "BEGIN; CREATE SCHEMA {schema}; COMMENT ON SCHEMA {schema} IS {marker}; \
             CREATE TABLE {table}(id bigint, note text, amount numeric); \
             INSERT INTO {table} VALUES(9007199254740993,NULL,12345678901234567890.123400),(2,'',NULL),(3,E'雪\\nline',-0.001); \
             CREATE VIEW {} AS SELECT * FROM {table}; \
             CREATE MATERIALIZED VIEW {} AS SELECT * FROM {table}; \
             CREATE VIEW {} AS SELECT * FROM {table} WHERE false; \
             CREATE VIEW {} AS SELECT repeat('x',1048577) AS big; \
             CREATE TABLE {root}(id integer) PARTITION BY RANGE(id); \
             CREATE TABLE {} PARTITION OF {root} FOR VALUES FROM(0) TO(10); \
             INSERT INTO {root} VALUES(1),(2);",
            self.qualified(VIEW), self.qualified(MATVIEW), self.qualified(EMPTY),
            self.qualified(OVERSIZE), self.qualified(PARTITION)
        );
        for (name, kind, _) in RELATIONS {
            sql.push_str(&format!(
                "COMMENT ON {kind} {} IS {marker};",
                self.qualified(name)
            ));
        }
        sql.push_str("COMMIT;");
        self.sql(&sql).map(|_| ())
    }
    fn identity(&self) -> Result<serde_json::Value, String> {
        let schema = crate::quote_literal(&self.schema);
        let sql = format!(
            "SELECT json_build_object('schema',(SELECT json_build_object('oid',oid::bigint,'name',nspname,'owner',pg_get_userbyid(nspowner),'marker',obj_description(oid,'pg_namespace')) FROM pg_namespace WHERE nspname={schema}), 'relations',(SELECT json_agg(json_build_object('oid',c.oid::bigint,'name',c.relname,'kind',c.relkind,'owner',pg_get_userbyid(c.relowner),'marker',obj_description(c.oid,'pg_class')) ORDER BY c.oid) FROM (SELECT c.* FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname={schema} ORDER BY c.oid LIMIT 32)c))"
        );
        serde_json::from_str(&self.sql(&sql)?).map_err(|e| e.to_string())
    }
    fn validate_identity(&self, value: &serde_json::Value) -> Result<(), String> {
        let schema = &value["schema"];
        if schema["name"].as_str() != Some(self.schema.as_str())
            || schema["owner"].as_str() != Some("dbunk")
            || schema["marker"].as_str() != Some(self.marker.as_str())
            || schema["oid"].as_u64().is_none()
        {
            return Err("Owned table export schema identity mismatch".into());
        }
        let rows = value["relations"]
            .as_array()
            .ok_or("Missing relation receipt")?;
        if rows.len() != RELATIONS.len() {
            return Err("Owned table export relation count mismatch".into());
        }
        for row in rows {
            let name = row["name"].as_str().ok_or("Missing relation name")?;
            let kind = RELATIONS.iter().find(|r| r.0 == name).map(|r| r.2);
            if kind.is_none()
                || row["kind"].as_str() != kind
                || row["owner"].as_str() != Some("dbunk")
                || row["marker"].as_str() != Some(self.marker.as_str())
                || row["oid"].as_u64().is_none()
            {
                return Err("Owned table export relation identity mismatch".into());
            }
        }
        Ok(())
    }
    fn cleanup(&self, expected: Option<&serde_json::Value>) -> Result<(), String> {
        let actual = self.identity()?;
        if actual["schema"].is_null() {
            return Ok(());
        }
        self.validate_identity(&actual)?;
        if expected.is_some_and(|expected| *expected != actual) {
            return Err("Owned table export receipt changed; objects preserved".into());
        }
        let marker = crate::quote_literal(&self.marker);
        let schema_name = crate::quote_literal(&self.schema);
        let schema_oid = actual["schema"]["oid"]
            .as_u64()
            .ok_or("Missing schema OID")?;
        let mut sql = format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS(SELECT FROM pg_namespace WHERE oid={schema_oid} AND nspname={schema_name} AND pg_get_userbyid(nspowner)='dbunk' AND obj_description(oid,'pg_namespace')={marker}) THEN RAISE EXCEPTION 'table export schema ownership changed'; END IF;");
        for row in actual["relations"].as_array().unwrap() {
            sql.push_str(&format!("IF NOT EXISTS(SELECT FROM pg_class WHERE oid={} AND relnamespace={schema_oid} AND relname={} AND relkind={} AND pg_get_userbyid(relowner)='dbunk' AND obj_description(oid,'pg_class')={marker}) THEN RAISE EXCEPTION 'table export relation ownership changed'; END IF;", row["oid"].as_u64().unwrap(), crate::quote_literal(row["name"].as_str().unwrap()), crate::quote_literal(row["kind"].as_str().unwrap())));
        }
        sql.push_str(&format!("END $guard$; DROP VIEW {},{},{} RESTRICT; DROP MATERIALIZED VIEW {} RESTRICT; DROP TABLE {} RESTRICT; DROP TABLE {},{} RESTRICT; DROP SCHEMA {} RESTRICT; COMMIT;", self.qualified(VIEW), self.qualified(EMPTY), self.qualified(OVERSIZE), self.qualified(MATVIEW), self.qualified(PARTITION), self.qualified(TABLE), self.qualified(PARTITIONED), crate::quote_double(&self.schema)));
        self.sql(&sql)?;
        if !self.identity()?["schema"].is_null() {
            return Err("Owned table export schema remains after cleanup".into());
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit owned stage03 capture probe; UUID schema and OID/marker cleanup"]
async fn native_table_export_exact_values_views_empty_and_refusal() {
    fixture_idle();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let probe = CaptureProbe {
        schema: format!("native_export_{token}"),
        marker: format!("dbunk owned table export {token}"),
    };
    println!(
        "TABLE_EXPORT_SETUP {}",
        serde_json::json!({"fixture":FIXTURE,"schema":probe.schema,"marker":probe.marker})
    );
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let mut identity = None;
    let outcome = std::panic::AssertUnwindSafe(async {
        probe.setup().unwrap();
        let receipt = probe.identity().unwrap();
        probe.validate_identity(&receipt).unwrap();
        println!("TABLE_EXPORT_OBJECTS {receipt}");
        identity = Some(receipt);
        let document = backend
            .open_data_document("table-export-live", "table-export", &backend.fixture().id)
            .await
            .unwrap();
        let request = |name: &str| TableExportRequest {
            schema: probe.schema.clone(),
            table: name.into(),
            expected: None,
        };
        for (name, kind, rows) in [
            (TABLE, TableExportKind::Table, 3),
            (VIEW, TableExportKind::View, 3),
            (MATVIEW, TableExportKind::MaterializedView, 3),
            (EMPTY, TableExportKind::View, 0),
            (PARTITIONED, TableExportKind::PartitionedTable, 2),
        ] {
            let capture = backend
                .capture_table_export(&document, request(name))
                .await
                .unwrap();
            let data = capture.data();
            assert_eq!(data.kind, kind);
            assert_eq!(data.rows.len(), rows);
            assert!(capture.checked_heap_bytes().is_some());
            assert!(capture.encoded_bytes().is_some());
            let expected_oid = identity.as_ref().unwrap()["relations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == name)
                .unwrap()["oid"]
                .as_u64()
                .unwrap();
            assert_eq!(u64::from(data.identity.relation_oid), expected_oid);
            if rows == 3 {
                let row = |id: &str| {
                    data.rows
                        .iter()
                        .find(|r| r[0].as_deref() == Some(id))
                        .unwrap()
                };
                assert_eq!(row("9007199254740993")[1], None);
                assert_eq!(
                    row("9007199254740993")[2].as_deref(),
                    Some("12345678901234567890.123400")
                );
                assert_eq!(row("2")[1].as_deref(), Some(""));
                assert_eq!(row("3")[1].as_deref(), Some("雪\nline"));
            }
            let mut exact = request(name);
            exact.expected = Some(data.identity);
            assert_eq!(
                backend
                    .capture_table_export(&document, exact.clone())
                    .await
                    .unwrap()
                    .data()
                    .rows
                    .len(),
                rows
            );
            exact.expected.as_mut().unwrap().relation_oid += 1;
            assert!(matches!(
                backend.capture_table_export(&document, exact).await,
                Err(DataError::Catalog(CatalogError::TableExportIdentityChanged))
            ));
        }
        assert!(matches!(
            backend
                .capture_table_export(&document, request(OVERSIZE))
                .await,
            Err(DataError::Catalog(CatalogError::TableExportLimit))
        ));
        backend.close_data_document(&document).await.unwrap();
        assert!(matches!(
            backend
                .capture_table_export(&document, request(TABLE))
                .await,
            Err(DataError::Document(_))
        ));
    })
    .catch_unwind()
    .await;
    let shutdown = backend.shutdown().await;
    let cleanup = probe.cleanup(identity.as_ref());
    fixture_idle();
    assert!(shutdown.is_ok(), "Backend shutdown: {shutdown:?}");
    assert!(cleanup.is_ok(), "Owned table export cleanup: {cleanup:?}");
    println!(
        "TABLE_EXPORT_CLEANUP {}",
        serde_json::json!({"fixture":FIXTURE,"schema":probe.schema,"schema_absent":true,"activity":0,"backend_joined":true})
    );
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
