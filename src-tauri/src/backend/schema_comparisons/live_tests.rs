//! Opt-in PG16 facade probe. Every helper transaction verifies the private
//! fixture sentinel; writes are confined to fresh UUID-owned schemas.
use super::*;
use crate::backend::{
    DevelopmentEnvironment, DevelopmentPostgresConnection, DevelopmentSafeMode,
    DevelopmentStorageMode, NativeProfileKind,
};
use futures_util::FutureExt;
use std::{io::Write, path::PathBuf, process::Stdio, time::Duration};

struct Fixture {
    instance: uuid::Uuid,
    psql: PathBuf,
}
impl Fixture {
    fn from_env() -> Self {
        assert_eq!(
            std::env::var("DBUNK_NATIVE_COMPARE_VERIFIED").as_deref(),
            Ok("1")
        );
        assert_eq!(
            std::env::var("DBUNK_NATIVE_COMPARE_PORT").as_deref(),
            Ok("15434")
        );
        let instance = std::env::var("DBUNK_NATIVE_COMPARE_FIXTURE_UUID")
            .unwrap()
            .parse()
            .unwrap();
        let psql = PathBuf::from(std::env::var_os("DBUNK_NATIVE_COMPARE_PSQL").unwrap());
        assert!(psql.is_absolute() && psql.is_file());
        Self { instance, psql }
    }
    async fn sql(&self, sql: String) -> Result<String, String> {
        let psql = self.psql.clone();
        let instance = self.instance;
        tokio::task::spawn_blocking(move || {
            let guard = format!("BEGIN; SET LOCAL statement_timeout='20s'; SET LOCAL lock_timeout='2s'; DO $fixture$ BEGIN IF current_database()<>'schema_compare_native' OR current_user<>'dbunk' OR inet_server_addr()<>'127.0.0.1'::inet OR inet_server_port()<>15434 OR current_setting('server_version_num')::integer NOT BETWEEN 160000 AND 169999 OR (SELECT count(*) FROM dbunk_native_comparison_fixture.identity)<>1 OR NOT EXISTS(SELECT 1 FROM dbunk_native_comparison_fixture.identity WHERE instance_uuid={}) THEN RAISE EXCEPTION 'Foreign comparison fixture'; END IF; END $fixture$;\n{sql}\nCOMMIT;", crate::quote_literal(&instance.to_string()));
            let mut child = std::process::Command::new(psql)
                .args(["-X", "-q", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1", "-p", "15434", "-U", "dbunk", "-d", "schema_compare_native"])
                .env("PGPASSWORD", "dbunk").env("PGCONNECT_TIMEOUT", "5")
                .env_remove("PGSERVICE").env_remove("PGSERVICEFILE").env_remove("PGOPTIONS")
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
                .spawn().map_err(|_| "Owned comparison SQL helper failed to start")?;
            let written = child.stdin.take().unwrap().write_all(guard.as_bytes());
            // Join even when writing fails. The helper cannot outlive its owner.
            let result = child.wait_with_output().map_err(|_| "Owned comparison SQL helper did not join")?;
            written.map_err(|_| "Owned comparison SQL input failed")?;
            if !result.status.success() { return Err("Identity-checked comparison SQL refused or failed".into()); }
            String::from_utf8(result.stdout).map(|text| text.trim().to_owned()).map_err(|_| "Invalid helper UTF-8".into())
        }).await.map_err(|_| "Owned comparison helper task failed")?
    }
    async fn count(&self) -> u64 {
        self.sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid();".into()).await.unwrap().parse().unwrap()
    }
}

fn identity_sql(schema: &str) -> String {
    format!("SELECT json_build_object('oid',n.oid::bigint,'owner',pg_get_userbyid(n.nspowner),'comment',obj_description(n.oid,'pg_namespace'),'tables',(SELECT json_agg(json_build_object('oid',c.oid::bigint,'name',c.relname,'owner',pg_get_userbyid(c.relowner),'kind',c.relkind::text) ORDER BY c.relname) FROM pg_catalog.pg_class c WHERE c.relnamespace=n.oid))::text FROM pg_catalog.pg_namespace n WHERE n.nspname={};", crate::quote_literal(schema))
}
async fn create(fixture: &Fixture, schema: &str, marker: &str) -> serde_json::Value {
    assert!(fixture.sql(identity_sql(schema)).await.unwrap().is_empty());
    let quoted = crate::quote_double(schema);
    let mut sql = format!(
        "CREATE SCHEMA {quoted}; COMMENT ON SCHEMA {quoted} IS {};",
        crate::quote_literal(marker)
    );
    for index in 0..101 {
        sql.push_str(&format!(
            "CREATE TABLE {quoted}.table_{index:03}(id integer, value text);"
        ));
    }
    sql.push_str(&format!(
        "COMMENT ON TABLE {quoted}.table_000 IS {};",
        crate::quote_literal(&"日".repeat(30_000))
    ));
    sql.push_str(&identity_sql(schema));
    let identity: serde_json::Value =
        serde_json::from_str(&fixture.sql(sql).await.unwrap()).unwrap();
    assert_eq!(identity["owner"], "dbunk");
    assert_eq!(identity["comment"], marker);
    assert_eq!(identity["tables"].as_array().unwrap().len(), 101);
    identity
}
async fn cleanup(
    fixture: &Fixture,
    schema: &str,
    identity: &serde_json::Value,
) -> Result<(), String> {
    let actual: serde_json::Value = serde_json::from_str(&fixture.sql(identity_sql(schema)).await?)
        .map_err(|_| "Missing or invalid owned schema identity; preserved")?;
    if &actual != identity {
        return Err("Owned schema identity changed; preserved".into());
    }
    let mut sql = String::new();
    let schema_oid = identity["oid"].as_u64().ok_or("Missing schema OID")?;
    let marker = identity["comment"]
        .as_str()
        .ok_or("Missing schema marker")?;
    sql.push_str(&format!("DO $owned$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE oid={schema_oid} AND nspname={} AND pg_get_userbyid(nspowner)='dbunk' AND obj_description(oid,'pg_namespace')={}) THEN RAISE EXCEPTION 'Schema identity changed'; END IF; END $owned$;", crate::quote_literal(schema), crate::quote_literal(marker)));
    for table in identity["tables"]
        .as_array()
        .ok_or("Missing owned tables")?
    {
        let oid = table["oid"].as_u64().ok_or("Missing table OID")?;
        let name = table["name"].as_str().ok_or("Missing table name")?;
        sql.push_str(&format!("DO $owned$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid={oid} AND relnamespace={schema_oid} AND relname={} AND relkind='r' AND pg_get_userbyid(relowner)='dbunk') THEN RAISE EXCEPTION 'Table identity changed'; END IF; END $owned$; DROP TABLE {}.{} RESTRICT;", crate::quote_literal(name), crate::quote_double(schema), crate::quote_double(name)));
    }
    sql.push_str(&format!(
        "DROP SCHEMA {} RESTRICT;",
        crate::quote_double(schema)
    ));
    fixture.sql(sql).await?;
    assert!(fixture.sql(identity_sql(schema)).await?.is_empty());
    println!("owned comparison schema cleaned: {schema} oid={schema_oid} RESTRICT");
    Ok(())
}
async fn require_absent(fixture: &Fixture, schema: &str) -> Result<(), String> {
    if fixture.sql(identity_sql(schema)).await?.is_empty() {
        Ok(())
    } else {
        Err("Creation did not return ownership identity; schema preserved for review".into())
    }
}
fn form(name: &str) -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: name.into(),
        host: "127.0.0.1".into(),
        port: 15434,
        database: "schema_compare_native".into(),
        user: "dbunk".into(),
        environment: DevelopmentEnvironment::Test,
        safe_mode: DevelopmentSafeMode::Strict,
        read_only: true,
        tls: Default::default(),
        driver_options: Default::default(),
    }
}
async fn completed(backend: &Backend, source: Endpoint, target: Endpoint) -> ResultRequest {
    let start = SchemaComparisonStart::new(source, target).unwrap();
    let status = backend.begin_schema_comparison(start.clone()).unwrap();
    assert_eq!(
        backend.begin_schema_comparison(start).unwrap().job_id,
        status.job_id
    );
    let status = tokio::time::timeout(Duration::from_secs(65), async {
        loop {
            let status = backend.get_schema_comparison(&status.job_id).unwrap();
            if matches!(
                status.state,
                StatusState::Completed { .. } | StatusState::Cancelled | StatusState::Failed { .. }
            ) {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let StatusState::Completed { result_id } = status.state else {
        panic!("comparison did not complete: {status:?}");
    };
    ResultRequest {
        identity: ResultIdentity {
            job_id: status.job_id,
            result_id,
        },
        source: status.source,
        target: status.target,
    }
}
async fn read(
    backend: &Backend,
    reader: &SchemaComparisonReader,
    request: ReadRequest,
) -> CompareReply {
    let response = backend
        .read_schema_comparison(reader, request)
        .await
        .unwrap();
    assert!(response.checked_heap_bytes().is_some());
    let page = response.into_page().unwrap();
    assert!(page.checked_heap_bytes().is_some());
    page.reply
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned PG16 loopback15434 fixture only; explicit UUID/environment guard; serial opt-in"]
async fn native_pg16_facade_pages_utf8_identity_invalidation_and_joined_cleanup() {
    let fixture = Fixture::from_env();
    let baseline = fixture.count().await;
    let run = uuid::Uuid::new_v4();
    let source_schema = format!("native_compare_{}_s", run.simple());
    let target_schema = format!("native_compare_{}_t", run.simple());
    let marker = format!("owned schema comparison {run}");
    let directory = tempfile::tempdir().unwrap();
    let mut backend = None;
    let mut source_identity = None;
    let mut target_identity = None;
    let operation = async {
        source_identity = Some(create(&fixture, &source_schema, &marker).await);
        target_identity = Some(create(&fixture, &target_schema, &marker).await);
        backend = Some(
            Backend::create_native_profile(
                &directory.path().canonicalize().unwrap().join("profile"),
            )
            .await
            .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        assert_eq!(
            backend.native_profile_kind(),
            Some(NativeProfileKind::GeneralPostgres)
        );
        assert!(backend
            .0
            .schema_comparisons
            .test_capture
            .lock()
            .unwrap()
            .is_none());
        backend
            .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
            .await
            .unwrap();
        let source = backend
            .save_development_connection(None, form("owned PG16 source"), "dbunk".into())
            .await
            .unwrap();
        let target = backend
            .save_development_connection(None, form("owned PG16 target"), "dbunk".into())
            .await
            .unwrap();
        let source_endpoint = Endpoint {
            connection_id: source.id.clone(),
            schema: source_schema.clone(),
        };
        let target_endpoint = Endpoint {
            connection_id: source.id.clone(),
            schema: target_schema.clone(),
        };
        fixture.sql("SELECT 1;".into()).await.unwrap();
        let request = completed(backend, source_endpoint.clone(), target_endpoint).await;
        let reader = backend
            .open_schema_comparison_reader("owned-live", "same", request.clone())
            .await
            .unwrap();
        let CompareReply::Metadata {
            metadata,
            object_count,
            ..
        } = read(backend, &reader, ReadRequest::Metadata).await
        else {
            panic!("metadata");
        };
        assert_eq!(metadata.consistency, SnapshotConsistency::SharedTransaction);
        assert_eq!(metadata.source.endpoint, request.source);
        assert_eq!(metadata.target.endpoint, request.target);
        assert_eq!(metadata.source.server_version_num / 10_000, 16);
        assert_eq!(metadata.source.captured_at, metadata.target.captured_at);
        assert_eq!(object_count, 101);
        let CompareReply::Objects {
            items, next_offset, ..
        } = read(backend, &reader, ReadRequest::Objects { offset: 0 }).await
        else {
            panic!("objects");
        };
        assert_eq!(items.len(), 100);
        assert_eq!(next_offset, Some(100));
        let CompareReply::Objects {
            items, next_offset, ..
        } = read(
            backend,
            &reader,
            ReadRequest::Objects {
                offset: next_offset.unwrap(),
            },
        )
        .await
        else {
            panic!("objects");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(next_offset, None);
        let object = RelationIdentity {
            kind: RelationKind::Table,
            name: "table_000".into(),
        };
        let CompareReply::Fields { items, .. } = read(
            backend,
            &reader,
            ReadRequest::Fields {
                object: object.clone(),
                offset: 0,
            },
        )
        .await
        else {
            panic!("fields");
        };
        let comment = items
            .iter()
            .find(|field| {
                field.path
                    == FieldPath::Table {
                        field: TableField::Comment,
                    }
            })
            .unwrap();
        let SummaryDifference::Equal { source: value, .. } = comment.difference else {
            panic!("exact comment");
        };
        let CompareReply::Value {
            text,
            next_offset,
            complete,
            ..
        } = read(backend, &reader, ReadRequest::Value { value, offset: 0 }).await
        else {
            panic!("value");
        };
        assert_eq!(text, "日".repeat(21_845));
        assert_eq!(next_offset, 65_535);
        assert!(!complete);
        let CompareReply::Value {
            text,
            next_offset,
            complete,
            ..
        } = read(
            backend,
            &reader,
            ReadRequest::Value {
                value,
                offset: next_offset,
            },
        )
        .await
        else {
            panic!("value");
        };
        assert_eq!(text, "日".repeat(8_155));
        assert_eq!(next_offset, 90_000);
        assert!(complete);
        assert!(matches!(
            read(
                backend,
                &reader,
                ReadRequest::Eligibility {
                    object,
                    side: Side::Source
                }
            )
            .await,
            CompareReply::Eligibility {
                eligibility: Eligibility::Eligible,
                ..
            }
        ));
        let queued = backend
            .read_schema_comparison(&reader, ReadRequest::Metadata)
            .await
            .unwrap();
        backend
            .release_schema_comparison(&request.identity.job_id)
            .unwrap();
        assert!(matches!(queued.into_page(), Err(CompareError::Unavailable)));
        backend
            .close_schema_comparison_reader(&reader)
            .await
            .unwrap();
        fixture.sql("SELECT 1;".into()).await.unwrap();
        let request = completed(
            backend,
            source_endpoint,
            Endpoint {
                connection_id: target.id.clone(),
                schema: target_schema.clone(),
            },
        )
        .await;
        let reader = backend
            .open_schema_comparison_reader("owned-live", "cross", request.clone())
            .await
            .unwrap();
        let CompareReply::Metadata { metadata, .. } =
            read(backend, &reader, ReadRequest::Metadata).await
        else {
            panic!("metadata");
        };
        assert_eq!(
            metadata.consistency,
            SnapshotConsistency::IndependentTransactions
        );
        assert_eq!(metadata.source.endpoint, request.source);
        assert_eq!(metadata.target.endpoint, request.target);
        let queued = backend
            .read_schema_comparison(&reader, ReadRequest::Metadata)
            .await
            .unwrap();
        backend
            .save_development_connection(Some(target.id), form("owned PG16 renamed"), String::new())
            .await
            .unwrap();
        assert!(matches!(queued.into_page(), Err(CompareError::Unavailable)));
        backend
            .close_schema_comparison_reader(&reader)
            .await
            .unwrap();
        backend
            .release_schema_comparison(&request.identity.job_id)
            .unwrap();
        println!("PG16 native facade: 101 objects, 100/1 pages, 65535/90000 UTF-8 cuts, shared/independent snapshots, release and endpoint invalidation");
    };
    // Do not cancel a helper JoinHandle with an outer timeout. SQL helpers have
    // finite server/connect timeouts and are always joined; comparison polling
    // has its own deadline, followed by the backend's owned shutdown.
    let outcome = std::panic::AssertUnwindSafe(operation).catch_unwind().await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let source_cleanup = match source_identity {
        Some(identity) => cleanup(&fixture, &source_schema, &identity).await,
        None => require_absent(&fixture, &source_schema).await,
    };
    let target_cleanup = match target_identity {
        Some(identity) => cleanup(&fixture, &target_schema, &identity).await,
        None => require_absent(&fixture, &target_schema).await,
    };
    assert!(shutdown.is_ok(), "joined backend shutdown: {shutdown:?}");
    assert!(source_cleanup.is_ok(), "source cleanup: {source_cleanup:?}");
    assert!(target_cleanup.is_ok(), "target cleanup: {target_cleanup:?}");
    assert_eq!(
        fixture.count().await,
        baseline,
        "owned backend activity baseline"
    );
    match outcome {
        Ok(()) => (),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
