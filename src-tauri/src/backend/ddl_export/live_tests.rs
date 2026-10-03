//! Explicit owned stage03 probes: one read-only, one creating only a named
//! UUID schema whose marked object identities are checked before RESTRICT cleanup.
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit read-only owned stage03 DDL export probe; identity/activity guard required"]
async fn native_ddl_export_scopes_identity_and_joined_cleanup() {
    fixture_idle();
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let outcome = std::panic::AssertUnwindSafe(async {
        let document = backend
            .open_data_document("ddl-export-live", "ddl-export", &backend.fixture().id)
            .await
            .unwrap();
        let database = backend
            .export_ddl(&document, DdlExportRequest::default())
            .await
            .unwrap();
        assert!(database.checked_heap_bytes().is_some());
        assert_eq!(database.database, "dbunk_demo");
        assert!(
            !database.relations.is_empty(),
            "Owned demo fixture must contain relations"
        );
        let relation = &database.relations[0];
        let request = DdlExportRequest {
            scope: DdlExportScope::Relation {
                schema: relation.schema.clone(),
                name: relation.name.clone(),
                expected: Some(relation.identity),
            },
            expected_database_oid: Some(database.database_oid),
        };
        let exact = backend
            .export_ddl(&document, request.clone())
            .await
            .unwrap();
        assert_eq!(exact.relations.len(), 1);
        assert_eq!(exact.relations[0].identity, relation.identity);
        assert_eq!(exact.relation_sql(0), database.relation_sql(0));
        assert!(exact.checked_heap_bytes().is_some());
        assert!(exact.sql.contains("not a canonical database backup"));
        let schema = backend
            .export_ddl(
                &document,
                DdlExportRequest {
                    scope: DdlExportScope::Schema {
                        name: relation.schema.clone(),
                        expected_oid: Some(relation.schema_oid),
                    },
                    expected_database_oid: Some(database.database_oid),
                },
            )
            .await
            .unwrap();
        assert_eq!(schema.schemas.len(), 1);
        assert!(schema.schemas[0].declared);
        assert!(schema
            .relations
            .iter()
            .all(|r| r.schema_oid == relation.schema_oid));
        let mut wrong = request;
        if let DdlExportScope::Relation {
            expected: Some(identity),
            ..
        } = &mut wrong.scope
        {
            identity.relation_oid = identity.relation_oid.wrapping_add(1);
        }
        assert!(matches!(
            backend.export_ddl(&document, wrong).await,
            Err(DataError::Catalog(CatalogError::DdlExportIdentityChanged))
        ));
        backend.close_data_document(&document).await.unwrap();
        assert!(backend
            .export_ddl(&document, DdlExportRequest::default())
            .await
            .is_err());
    })
    .catch_unwind()
    .await;
    backend.shutdown().await.unwrap();
    fixture_idle();
    println!(
        "DDL_EXPORT_READ_ONLY_CLEANUP fixture={FIXTURE} activity=0 no_fixture_objects_created"
    );
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

const OWNED_SQL: &str = r#"import sys,signal
signal.signal(signal.SIGALRM,lambda *_: (_ for _ in ()).throw(TimeoutError('owned DDL helper deadline')))
signal.alarm(25)
sys.path.insert(0,sys.argv[1])
import fixture
owned,target=fixture.check()
assert owned['instance']==sys.argv[2]
print(fixture.sql(target,sys.stdin.read()))
"#;
const PARENT: &str = "parent \"雪";
const CHILD: &str = "child.dot";
const PARTITIONED: &str = "partition root";
const PARTITION: &str = "partition.0";
const VIEW: &str = "view \"雪";
const MATVIEW: &str = "materialized \"雪";
const UNIQUE_INDEX: &str = "referenced \"unique";
const RELATIONS: [(&str, &str, &str); 7] = [
    (PARENT, "TABLE", "r"),
    (CHILD, "TABLE", "r"),
    (PARTITIONED, "TABLE", "p"),
    (PARTITION, "TABLE", "r"),
    (VIEW, "VIEW", "v"),
    (MATVIEW, "MATERIALIZED VIEW", "m"),
    (" ", "VIEW", "v"),
];
struct MixedProbe {
    schema: String,
    marker: String,
}
impl MixedProbe {
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
            return Err("Owned DDL helper receipt exceeds bound".into());
        }
        String::from_utf8(output.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|e| e.to_string())
    }
    fn setup(&self) -> Result<(), String> {
        let schema = crate::quote_double(&self.schema);
        let marker = crate::quote_literal(&self.marker);
        let parent = self.qualified(PARENT);
        let child = self.qualified(CHILD);
        let partitioned = self.qualified(PARTITIONED);
        let partition = self.qualified(PARTITION);
        let view = self.qualified(VIEW);
        let matview = self.qualified(MATVIEW);
        let blank = self.qualified(" ");
        let index = crate::quote_double(UNIQUE_INDEX);
        let mut sql = format!(
            "BEGIN; CREATE SCHEMA {schema}; COMMENT ON SCHEMA {schema} IS {marker}; \
             CREATE TABLE {parent}(id integer NOT NULL, pk integer CONSTRAINT owned_pk PRIMARY KEY, uq integer CONSTRAINT owned_uq UNIQUE); \
             CREATE UNIQUE INDEX {index} ON {parent}(id); \
             CREATE TABLE {child}(parent_id integer CONSTRAINT parent_fk REFERENCES {parent}(id)); \
             CREATE TABLE {partitioned}(bucket integer) PARTITION BY RANGE(bucket); \
             CREATE TABLE {partition} PARTITION OF {partitioned} FOR VALUES FROM(0) TO(10); \
             CREATE VIEW {view} AS SELECT id FROM {parent}; \
             CREATE VIEW {blank} AS SELECT 1 AS value; \
             CREATE MATERIALIZED VIEW {matview} AS SELECT id FROM {parent} WITH NO DATA;"
        );
        for (name, kind, _) in RELATIONS {
            sql.push_str(&format!(
                "COMMENT ON {kind} {} IS {marker};",
                self.qualified(name)
            ));
        }
        for name in [UNIQUE_INDEX, "owned_pk", "owned_uq"] {
            sql.push_str(&format!(
                "COMMENT ON INDEX {} IS {marker};",
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
            return Err("Owned DDL schema identity mismatch".into());
        }
        let rows = value["relations"]
            .as_array()
            .ok_or("Missing relation receipt")?;
        if rows.len() != RELATIONS.len() + 3 {
            return Err("Owned DDL relation count mismatch".into());
        }
        for row in rows {
            let name = row["name"].as_str().ok_or("Missing relation name")?;
            let kind = RELATIONS
                .iter()
                .find(|r| r.0 == name)
                .map(|r| r.2)
                .or_else(|| {
                    [UNIQUE_INDEX, "owned_pk", "owned_uq"]
                        .contains(&name)
                        .then_some("i")
                });
            if kind.is_none()
                || row["kind"].as_str() != kind
                || row["owner"].as_str() != Some("dbunk")
                || row["marker"].as_str() != Some(self.marker.as_str())
                || row["oid"].as_u64().is_none()
            {
                return Err("Owned DDL relation identity mismatch".into());
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
            return Err("Owned DDL receipt changed; objects preserved".into());
        }
        let marker = crate::quote_literal(&self.marker);
        let schema_name = crate::quote_literal(&self.schema);
        let schema_oid = actual["schema"]["oid"]
            .as_u64()
            .ok_or("Missing schema OID")?;
        let mut sql = format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS(SELECT FROM pg_namespace WHERE oid={schema_oid} AND nspname={schema_name} AND pg_get_userbyid(nspowner)='dbunk' AND obj_description(oid,'pg_namespace')={marker}) THEN RAISE EXCEPTION 'DDL schema ownership changed'; END IF;");
        for row in actual["relations"].as_array().unwrap() {
            sql.push_str(&format!("IF NOT EXISTS(SELECT FROM pg_class WHERE oid={} AND relnamespace={schema_oid} AND relname={} AND relkind={} AND pg_get_userbyid(relowner)='dbunk' AND obj_description(oid,'pg_class')={marker}) THEN RAISE EXCEPTION 'DDL relation ownership changed'; END IF;", row["oid"].as_u64().unwrap(), crate::quote_literal(row["name"].as_str().unwrap()), crate::quote_literal(row["kind"].as_str().unwrap())));
        }
        sql.push_str(&format!("END $guard$; DROP VIEW {},{} RESTRICT; DROP MATERIALIZED VIEW {} RESTRICT; DROP TABLE {} RESTRICT; DROP TABLE {},{},{} RESTRICT; DROP SCHEMA {} RESTRICT; COMMIT;", self.qualified(VIEW), self.qualified(" "), self.qualified(MATVIEW), self.qualified(PARTITION), self.qualified(CHILD), self.qualified(PARENT), self.qualified(PARTITIONED), crate::quote_double(&self.schema)));
        self.sql(&sql)?;
        if !self.identity()?["schema"].is_null() {
            return Err("Owned DDL schema remains after cleanup".into());
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit owned stage03 mixed DDL probe; creates UUID schema with marker/OID-guarded cleanup"]
async fn native_ddl_export_referenced_unique_index_and_mixed_relations() {
    fixture_idle();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let probe = MixedProbe {
        schema: format!("native_ddl_{token}"),
        marker: format!("dbunk owned DDL export {token}"),
    };
    println!(
        "DDL_EXPORT_SETUP {}",
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
        println!("DDL_EXPORT_OBJECTS {receipt}");
        identity = Some(receipt);
        let document = backend
            .open_data_document("ddl-export-mixed", "ddl-export", &backend.fixture().id)
            .await
            .unwrap();
        let capture = backend
            .export_ddl(
                &document,
                DdlExportRequest {
                    scope: DdlExportScope::Schema {
                        name: probe.schema.clone(),
                        expected_oid: Some(
                            identity.as_ref().unwrap()["schema"]["oid"]
                                .as_u64()
                                .unwrap() as u32,
                        ),
                    },
                    expected_database_oid: None,
                },
            )
            .await
            .unwrap();
        assert!(capture.checked_heap_bytes().is_some());
        assert_eq!(capture.relations.len(), RELATIONS.len());
        let sql = |name: &str| {
            let index = capture
                .relations
                .iter()
                .position(|r| r.name == name)
                .expect("captured relation");
            capture.relation_sql(index).unwrap()
        };
        let parent = sql(PARENT);
        assert!(
            parent.contains(&format!(
                "CREATE UNIQUE INDEX {} ON",
                crate::quote_double(UNIQUE_INDEX)
            )),
            "Referenced standalone unique index must remain in export: {parent}"
        );
        assert!(parent.contains("CONSTRAINT \"owned_pk\" PRIMARY KEY"));
        assert!(parent.contains("CONSTRAINT \"owned_uq\" UNIQUE"));
        assert_eq!(
            parent.matches("CREATE UNIQUE INDEX").count(),
            1,
            "Constraint-owned indexes must not be emitted twice"
        );
        assert!(sql(CHILD).contains("FOREIGN KEY (parent_id) REFERENCES"));
        assert!(sql(PARTITIONED).contains("PARTITION BY RANGE (bucket)"));
        assert!(sql(PARTITION).contains(&format!("PARTITION OF {}", probe.qualified(PARTITIONED))));
        assert!(sql(PARTITION).contains("FOR VALUES FROM (0) TO (10)"));
        assert!(sql(VIEW).contains(&format!("CREATE VIEW {}", probe.qualified(VIEW))));
        assert!(sql(" ").contains(&format!("CREATE VIEW {}", probe.qualified(" "))));
        assert!(sql(MATVIEW).contains(&format!(
            "CREATE MATERIALIZED VIEW {}",
            probe.qualified(MATVIEW)
        )));
        assert!(sql(MATVIEW).contains("WITH NO DATA"));
        for relation in &capture.relations {
            let expected_oid = identity.as_ref().unwrap()["relations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"].as_str() == Some(relation.name.as_str()))
                .unwrap()["oid"]
                .as_u64()
                .unwrap();
            assert_eq!(u64::from(relation.identity.relation_oid), expected_oid);
            let exact = backend
                .export_ddl(
                    &document,
                    DdlExportRequest {
                        scope: DdlExportScope::Relation {
                            schema: probe.schema.clone(),
                            name: relation.name.clone(),
                            expected: Some(relation.identity),
                        },
                        expected_database_oid: Some(capture.database_oid),
                    },
                )
                .await
                .unwrap();
            assert_eq!(exact.relations.len(), 1);
            assert_eq!(exact.relation_sql(0), Some(sql(&relation.name)));
        }
        backend.close_data_document(&document).await.unwrap();
    })
    .catch_unwind()
    .await;
    let shutdown = backend.shutdown().await;
    let cleanup = probe.cleanup(identity.as_ref());
    fixture_idle();
    assert!(shutdown.is_ok(), "Backend shutdown: {shutdown:?}");
    assert!(cleanup.is_ok(), "Owned DDL cleanup: {cleanup:?}");
    println!(
        "DDL_EXPORT_CLEANUP {}",
        serde_json::json!({"fixture":FIXTURE,"schema":probe.schema,"schema_absent":true,"activity":0,"backend_joined":true})
    );
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
