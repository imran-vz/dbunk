//! Opt-in owned stage03 probe. No existing relations are modified. Helpers check
//! the fixture before every SQL statement group; cleanup validates exact owned
//! schema/table/function OIDs, owner and comments, then uses RESTRICT only.
use super::*;
use crate::{backend::profile, postgres::dedicated::DriverJoins};
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
const HELPER: &str = r#"import sys,signal
signal.signal(signal.SIGALRM,lambda *_: (_ for _ in ()).throw(TimeoutError('fixture helper deadline')))
signal.alarm(25)
sys.path.insert(0,sys.argv[1])
import fixture
owned,target=fixture.check()
assert owned['instance']==sys.argv[2]
print(fixture.sql(target,sys.stdin.read()))
"#;
struct Probe {
    schema: String,
    other: String,
    marker: String,
    application: String,
    helpers: DriverJoins,
}
impl Probe {
    fn qualified(&self, other: bool, name: &str) -> String {
        format!(
            "{}.{}",
            crate::quote_double(if other { &self.other } else { &self.schema }),
            crate::quote_double(name)
        )
    }
    async fn sql(&self, sql: String) -> Result<String, String> {
        let application = self.application.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result=tokio::task::spawn_blocking(move||{
                let mut child=std::process::Command::new("python3").arg("-c").arg(HELPER).arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e|e.to_string())?;
                let sql=format!("SET application_name={}; SET statement_timeout='10s'; SET lock_timeout='2s'; {sql}",crate::quote_literal(&application));
                if let Err(error) = child.stdin.take().unwrap().write_all(sql.as_bytes()) { let _ = child.kill(); let _ = child.wait(); return Err(error.to_string()); }
                let output=child.wait_with_output().map_err(|e|e.to_string())?;
                if !output.status.success(){return Err(String::from_utf8_lossy(&output.stderr).into_owned());}
                String::from_utf8(output.stdout).map(|s|s.trim().to_owned()).map_err(|e|e.to_string())
            }).await.map_err(|e|e.to_string()).and_then(|r|r);
            let _ = send.send(result);
        });
        self.helpers.track_task(task);
        receive.await.map_err(|e| e.to_string())?
    }
    async fn activity(&self) -> usize {
        self.sql("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()".into()).await.unwrap().parse().unwrap()
    }
    async fn read(
        &self,
        backend: &Backend,
        document: &DataDocument,
        request: SchemaMapRequest,
    ) -> Result<SchemaMapSnapshot, DataError> {
        self.activity().await;
        backend.schema_map(document, request).await
    }
    fn tables(&self) -> Vec<String> {
        vec![
            self.qualified(false, "parent"),
            self.qualified(false, "group"),
            self.qualified(false, "links"),
            self.qualified(false, "neighbor.parent"),
            self.qualified(true, "parent"),
        ]
    }
    async fn setup(&self) -> Result<(), String> {
        let a = crate::quote_double(&self.schema);
        let b = crate::quote_double(&self.other);
        let marker = crate::quote_literal(&self.marker);
        let parent = self.qualified(false, "parent");
        let group = self.qualified(false, "group");
        let links = self.qualified(false, "links");
        let dotted = self.qualified(false, "neighbor.parent");
        let external = self.qualified(true, "parent");
        let function = self.qualified(false, "map_trigger");
        let mut sql=format!("BEGIN; CREATE SCHEMA {a}; CREATE SCHEMA {b}; COMMENT ON SCHEMA {a} IS {marker}; COMMENT ON SCHEMA {b} IS {marker}; CREATE TABLE {parent}(tenant int NOT NULL,id bigint NOT NULL,parent_tenant int,parent_id bigint,PRIMARY KEY(tenant,id),CONSTRAINT self_link FOREIGN KEY(parent_tenant,parent_id) REFERENCES {parent}(tenant,id)); CREATE TABLE {group}(id bigint PRIMARY KEY); CREATE TABLE {external}(id bigint PRIMARY KEY); CREATE TABLE {dotted}(id bigint PRIMARY KEY,other bigint,CONSTRAINT same_fk FOREIGN KEY(other) REFERENCES {external}(id)); CREATE UNIQUE INDEX dotted_included ON {dotted}(other) INCLUDE(id); CREATE TABLE {links}(tenant int NOT NULL,account_id bigint NOT NULL,group_id bigint NOT NULL,optional_id bigint,PRIMARY KEY(tenant,account_id,group_id),CONSTRAINT same_fk FOREIGN KEY(tenant,account_id) REFERENCES {parent}(tenant,id) ON UPDATE CASCADE,CONSTRAINT to_group FOREIGN KEY(group_id) REFERENCES {group}(id),CONSTRAINT to_external FOREIGN KEY(optional_id) REFERENCES {external}(id) ON DELETE SET NULL); CREATE UNIQUE INDEX links_partial ON {links}(optional_id) WHERE optional_id IS NOT NULL; CREATE UNIQUE INDEX links_expression ON {links}((account_id+0)); COMMENT ON COLUMN {links}.account_id IS 'quoted 雪'; CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $owned$ BEGIN RETURN NEW; END $owned$; COMMENT ON FUNCTION {function}() IS {marker}; CREATE TRIGGER map_update BEFORE UPDATE OF account_id ON {links} FOR EACH ROW EXECUTE FUNCTION {function}();");
        for table in self.tables() {
            sql.push_str(&format!(" COMMENT ON TABLE {table} IS {marker};"));
        }
        sql.push_str(" COMMIT;");
        self.sql(sql).await.map(|_| ())
    }
    async fn identity(&self) -> Result<serde_json::Value, String> {
        self.sql(format!("SELECT json_build_object('schemas',(SELECT json_agg(json_build_object('oid',n.oid::bigint,'name',n.nspname,'owner',pg_get_userbyid(n.nspowner),'comment',obj_description(n.oid,'pg_namespace')) ORDER BY n.oid) FROM pg_namespace n WHERE n.nspname IN({},{})),'tables',(SELECT json_agg(json_build_object('oid',c.oid::bigint,'schema',n.nspname,'name',c.relname,'owner',pg_get_userbyid(c.relowner),'comment',obj_description(c.oid,'pg_class')) ORDER BY c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN({},{}) AND c.relkind IN('r','p')),'functions',(SELECT json_agg(json_build_object('oid',p.oid::bigint,'schema',n.nspname,'name',p.proname,'owner',pg_get_userbyid(p.proowner),'comment',obj_description(p.oid,'pg_proc')) ORDER BY p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname IN({},{})))",crate::quote_literal(&self.schema),crate::quote_literal(&self.other),crate::quote_literal(&self.schema),crate::quote_literal(&self.other),crate::quote_literal(&self.schema),crate::quote_literal(&self.other))).await.and_then(|s|serde_json::from_str(&s).map_err(|e|e.to_string()))
    }
    async fn cleanup(&self, expected: Option<&serde_json::Value>) -> Result<(), String> {
        let actual = self.identity().await?;
        if actual["schemas"].is_null() {
            return Ok(());
        }
        if expected.is_some_and(|e| *e != actual) {
            return Err("owned map identity changed; preserving objects".into());
        }
        for section in ["schemas", "tables", "functions"] {
            let rows = actual[section]
                .as_array()
                .ok_or("missing owned map cleanup identity")?;
            for row in rows {
                if row["owner"].as_str() != Some("dbunk")
                    || row["comment"].as_str() != Some(self.marker.as_str())
                {
                    return Err("owned map owner/comment changed; preserving objects".into());
                }
            }
        }
        if actual["schemas"].as_array().map(Vec::len) != Some(2)
            || actual["tables"].as_array().map(Vec::len) != Some(5)
            || actual["functions"].as_array().map(Vec::len) != Some(1)
        {
            return Err("owned map component count changed; preserving objects".into());
        }
        // The same transaction rechecks OID/owner/comment immediately before
        // explicit object drops. All generated names remain separately quoted.
        let mut sql = String::from("BEGIN; DO $guard$ BEGIN ");
        for row in actual["schemas"].as_array().unwrap() {
            let oid = row["oid"].as_u64().ok_or("schema OID")?;
            sql.push_str(&format!("IF NOT EXISTS(SELECT FROM pg_namespace WHERE oid={oid} AND nspname={} AND pg_get_userbyid(nspowner)='dbunk' AND obj_description(oid,'pg_namespace')={}) THEN RAISE EXCEPTION 'map schema ownership changed'; END IF;",crate::quote_literal(row["name"].as_str().ok_or("schema name")?),crate::quote_literal(&self.marker)));
        }
        for row in actual["tables"].as_array().unwrap() {
            let oid = row["oid"].as_u64().ok_or("table OID")?;
            sql.push_str(&format!("IF NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.oid={oid} AND n.nspname={} AND c.relname={} AND pg_get_userbyid(c.relowner)='dbunk' AND obj_description(c.oid,'pg_class')={}) THEN RAISE EXCEPTION 'map table ownership changed'; END IF;",crate::quote_literal(row["schema"].as_str().ok_or("table schema")?),crate::quote_literal(row["name"].as_str().ok_or("table name")?),crate::quote_literal(&self.marker)));
        }
        let function = &actual["functions"][0];
        sql.push_str(&format!("IF NOT EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE p.oid={} AND n.nspname={} AND p.proname='map_trigger' AND p.pronargs=0 AND pg_get_userbyid(p.proowner)='dbunk' AND obj_description(p.oid,'pg_proc')={}) THEN RAISE EXCEPTION 'map function ownership changed'; END IF; END $guard$; DROP TABLE {} RESTRICT; DROP FUNCTION {}() RESTRICT; DROP SCHEMA {},{} RESTRICT; COMMIT;",function["oid"].as_u64().ok_or("function OID")?,crate::quote_literal(&self.schema),crate::quote_literal(&self.marker),self.tables().join(","),self.qualified(false,"map_trigger"),crate::quote_double(&self.schema),crate::quote_double(&self.other)));
        self.sql(sql).await?;
        if !self.identity().await?["schemas"].is_null() {
            return Err("owned schema map cleanup residue".into());
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit owned stage03 schema-map probe; creates only named UUID schemas and owned objects"]
async fn native_schema_map_complete_scopes_identity_metadata_and_joined_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let token = uuid::Uuid::new_v4().simple().to_string();
    let probe = Probe {
        schema: format!("native_map_{token}"),
        other: format!("native_map_{token}.neighbor"),
        marker: format!("dbunk native schema map {token}"),
        application: format!("native_map_{token}"),
        helpers: DriverJoins::default(),
    };
    println!(
        "Owned target {FIXTURE} at 127.0.0.1:15432/dbunk_demo; schemas {:?}, {:?}; helper {}",
        probe.schema, probe.other, probe.application
    );
    assert_eq!(
        probe.activity().await,
        0,
        "fixture must be idle before setup"
    );
    let directory = profile::directory();
    println!("Owned temporary profile {}", directory.path().display());
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let mut identity = None;
    let outcome =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(180), async {
            probe.setup().await.unwrap();
            identity = Some(probe.identity().await.unwrap());
            println!("SCHEMA_MAP_OBJECTS {}", identity.as_ref().unwrap());
            let document = backend
                .open_data_document("schema-map-live", "map", &backend.fixture().id)
                .await
                .unwrap();
            let scoped = probe
                .read(
                    &backend,
                    &document,
                    SchemaMapRequest {
                        scope: SchemaMapScope::Schema {
                            name: probe.schema.clone(),
                            expected_oid: None,
                        },
                        expected_database_oid: None,
                    },
                )
                .await
                .unwrap();
            let database = probe
                .read(&backend, &document, SchemaMapRequest::default())
                .await
                .unwrap();
            assert!(database.tables.iter().all(|t| !t.external));
            assert!(scoped
                .tables
                .iter()
                .all(|t| database.tables.iter().any(|all| all.identity == t.identity)));
            assert!(scoped.checked_heap_bytes().is_some());
            assert_eq!(scoped.tables.len(), 5);
            assert_eq!(scoped.foreign_keys.len(), 5);
            let external = scoped
                .tables
                .iter()
                .find(|t| t.schema == probe.other)
                .unwrap();
            assert!(external.external);
            assert_eq!(external.columns.len(), 1);
            let collision = scoped
                .tables
                .iter()
                .find(|t| t.name == "neighbor.parent")
                .unwrap();
            assert_ne!(collision.identity, external.identity);
            let links = scoped.tables.iter().find(|t| t.name == "links").unwrap();
            assert!(links.junction);
            assert_eq!(links.triggers.len(), 1);
            assert_eq!(links.triggers[0].columns, vec![2]);
            assert_eq!(links.columns[1].comment.as_deref(), Some("quoted 雪"));
            let composite = scoped
                .foreign_keys
                .iter()
                .find(|f| f.source == links.identity && f.name == "same_fk")
                .unwrap();
            assert_eq!(
                composite.columns,
                vec![
                    SchemaMapColumnPair {
                        source: 1,
                        target: 1
                    },
                    SchemaMapColumnPair {
                        source: 2,
                        target: 2
                    }
                ]
            );
            assert!(!composite.columns_nullable);
            assert!(!composite.columns_unique);
            assert!(composite.junction_participant);
            let optional = scoped
                .foreign_keys
                .iter()
                .find(|f| f.name == "to_external")
                .unwrap();
            assert!(optional.columns_nullable);
            assert!(!optional.columns_unique);
            assert!(!optional.junction_participant);
            let unique = scoped
                .foreign_keys
                .iter()
                .find(|f| f.source == collision.identity)
                .unwrap();
            assert!(unique.columns_unique);
            assert_eq!(unique.cardinality, SchemaMapCardinality::OneToOne);
            let exact = SchemaMapRequest {
                scope: SchemaMapScope::Relation {
                    schema: probe.schema.clone(),
                    table: "links".into(),
                    expected: Some(links.identity),
                },
                expected_database_oid: Some(scoped.database_oid),
            };
            let focused = probe
                .read(&backend, &document, exact.clone())
                .await
                .unwrap();
            assert_eq!(focused.tables.len(), 4);
            assert_eq!(focused.foreign_keys.len(), 3);
            assert!(!focused.foreign_keys.iter().any(|f| f.name == "self_link"));
            let mut changed = exact;
            if let SchemaMapScope::Relation {
                expected: Some(id), ..
            } = &mut changed.scope
            {
                id.relation_oid = id.relation_oid.wrapping_add(1);
            }
            assert!(matches!(
                probe.read(&backend, &document, changed).await,
                Err(DataError::Catalog(
                    crate::postgres::native_catalog::CatalogError::SchemaMapIdentityChanged
                ))
            ));
            // Oversized metadata is refused, never omitted or truncated into a graph.
            probe
                .sql(format!(
                    "COMMENT ON COLUMN {}.account_id IS {}",
                    probe.qualified(false, "links"),
                    crate::quote_literal(&"x".repeat(MAX_SCHEMA_MAP_COMMENT_BYTES + 1))
                ))
                .await
                .unwrap();
            assert!(matches!(
                probe
                    .read(&backend, &document, scoped.refresh_request())
                    .await,
                Err(DataError::Catalog(
                    crate::postgres::native_catalog::CatalogError::SchemaMapLimit
                ))
            ));
            probe
                .sql(format!(
                    "COMMENT ON COLUMN {}.account_id IS 'quoted 雪'",
                    probe.qualified(false, "links")
                ))
                .await
                .unwrap();
            backend.close_data_document(&document).await.unwrap();
            assert!(matches!(
                backend
                    .schema_map(&document, SchemaMapRequest::default())
                    .await,
                Err(DataError::Document(_))
            ));
        }))
        .catch_unwind()
        .await;
    let shutdown = backend.shutdown().await;
    probe.helpers.drain().await;
    let cleanup = probe.cleanup(identity.as_ref()).await;
    probe.helpers.drain().await;
    let final_activity = probe.activity().await;
    probe.helpers.drain().await;
    println!("Owned schema-map teardown: shutdown={shutdown:?}; cleanup={cleanup:?}; activity 0 -> {final_activity}");
    shutdown.unwrap();
    cleanup.unwrap();
    assert_eq!(final_activity, 0);
    match outcome {
        Ok(result) => result.expect("bounded schema-map probe deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
