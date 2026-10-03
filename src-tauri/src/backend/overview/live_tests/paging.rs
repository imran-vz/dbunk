use super::*;
use std::{io::Write, process::Stdio};
#[derive(serde::Deserialize, serde::Serialize)]
struct Owned {
    schema: u32,
    views: Vec<View>,
}
#[derive(serde::Deserialize, serde::Serialize)]
struct View {
    name: String,
    oid: u32,
}
fn sql(query: &str) -> String {
    // This helper is synchronously joined. No detached SQL can outlive a test
    // assertion or race the identity-checked cleanup that follows it.
    let mut child=Command::new("python3").arg("-c")
        .arg("import os,sys;sys.path.insert(0,sys.argv[1]);import fixture;assert os.environ.get('DBUNK_NATIVE_FIXTURE_VERIFIED')=='1';owned,target=fixture.check();assert owned['instance']==sys.argv[2];print(fixture.sql(target,\"SET statement_timeout='10s'; SET lock_timeout='2s'; \"+sys.stdin.read()))")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let written = child.stdin.take().unwrap().write_all(query.as_bytes());
    if written.is_err() {
        let _ = child.kill();
        child.wait().unwrap();
        panic!("Fixture SQL input refused");
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Owned fixture SQL refused; recorded resources preserved"
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit owned stage03 pagination probe;257 unique marked constant views with guarded RESTRICT cleanup"]
async fn native_overview_owned_257_views_page_and_cleanup() {
    fixture_idle();
    let schema = format!("overview_{}", uuid::Uuid::new_v4().simple());
    let marker = format!("dbunk overview {}", uuid::Uuid::new_v4());
    let q = crate::quote_double(&schema);
    let mark = crate::quote_literal(&marker);
    println!(
        "OVERVIEW_CREATE_INTENT {}",
        serde_json::json!({"fixture":FIXTURE,"schema":schema,"marker":marker,"views":257})
    );
    let mut setup = format!("BEGIN;CREATE SCHEMA {q};COMMENT ON SCHEMA {q} IS {mark};");
    for index in 0..257 {
        setup+=&format!("CREATE VIEW {q}.v{index:03} AS SELECT {index}::integer AS value;COMMENT ON VIEW {q}.v{index:03} IS {mark};");
    }
    setup+=&format!("COMMIT;SELECT pg_catalog.json_build_object('schema',n.oid::bigint,'views',(SELECT pg_catalog.json_agg(pg_catalog.json_build_object('name',c.relname,'oid',c.oid::bigint)ORDER BY c.relname)FROM pg_catalog.pg_class c WHERE c.relnamespace=n.oid AND c.relkind='v'))::text FROM pg_catalog.pg_namespace n WHERE n.nspname={}",crate::quote_literal(&schema));
    let owned: Owned = serde_json::from_str(&sql(&setup)).unwrap();
    println!("OVERVIEW_OWNED {}", serde_json::to_string(&owned).unwrap());
    assert_eq!(owned.views.len(), 257);
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let outcome = std::panic::AssertUnwindSafe(async {
        let doc = backend
            .open_data_document(
                "overview-page-live",
                "overview-pages",
                &backend.fixture().id,
            )
            .await
            .unwrap();
        let mut request = RelationStatsRequest {
            scope: RelationStatsScope::Schema {
                name: schema.clone(),
                expected_oid: Some(owned.schema),
            },
            ..Default::default()
        };
        let first = backend.overview(&doc, request.clone()).await.unwrap();
        assert!(first.database.is_none());
        assert_eq!(first.relations.rows.len(), 256);
        assert_eq!(first.relations.totals.relation_count, 257);
        assert_eq!(first.relations.totals.view_count, 257);
        assert_eq!(first.relations.totals.table_count, 0);
        assert_eq!(
            first.relations.totals.total_size_bytes,
            OverviewMetric::Value(0)
        );
        assert!(first
            .relations
            .rows
            .iter()
            .all(|r| r.row_count_estimate == OverviewMetric::NotApplicable
                && r.total_size_bytes == OverviewMetric::NotApplicable));
        request.cursor = Some(
            first
                .relations
                .next_cursor
                .clone()
                .expect("257th row must retain continuation"),
        );
        let other = backend
            .open_data_document("overview-page-live", "replacement", &backend.fixture().id)
            .await
            .unwrap();
        assert!(matches!(
            backend.overview(&other, request.clone()).await,
            Err(DataError::Catalog(
                crate::postgres::native_catalog::CatalogError::OverviewIdentityChanged
            ))
        ));
        backend.close_data_document(&other).await.unwrap();
        let next = backend.overview(&doc, request).await.unwrap();
        assert_eq!(next.relations.rows.len(), 1);
        assert!(next.relations.next_cursor.is_none());
        assert_eq!(next.relations.totals.relation_count, 257);
        for (actual, expected) in first
            .relations
            .rows
            .iter()
            .chain(&next.relations.rows)
            .zip(&owned.views)
        {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.identity.relation_oid, expected.oid);
            assert_eq!(actual.schema_oid, owned.schema);
        }
        backend.close_data_document(&doc).await.unwrap();
        println!("OVERVIEW_PAGES rows=[256,1] scope_total=257 exact_order_and_oids=true");
    })
    .catch_unwind()
    .await;
    backend.shutdown().await.unwrap();
    fixture_idle();
    let mut guard=format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_namespace WHERE oid={} AND nspname={} AND pg_catalog.pg_get_userbyid(nspowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_namespace')={mark})THEN RAISE EXCEPTION 'Schema identity changed';END IF;",owned.schema,crate::quote_literal(&schema));
    for view in &owned.views {
        guard+=&format!("IF NOT EXISTS(SELECT FROM pg_catalog.pg_class WHERE oid={} AND relnamespace={} AND relname={} AND relkind='v' AND pg_catalog.pg_get_userbyid(relowner)='dbunk' AND pg_catalog.obj_description(oid,'pg_class')={mark})THEN RAISE EXCEPTION 'View identity changed';END IF;",view.oid,owned.schema,crate::quote_literal(&view.name));
    }
    for view in &owned.views {
        guard += &format!(
            "EXECUTE {};",
            crate::quote_literal(&format!(
                "DROP VIEW {q}.{} RESTRICT",
                crate::quote_double(&view.name)
            ))
        );
    }
    guard += &format!(
        "EXECUTE {};",
        crate::quote_literal(&format!("DROP SCHEMA {q} RESTRICT"))
    );
    sql(&format!(
        "BEGIN;DO $guard$ BEGIN {guard} END $guard$;COMMIT;"
    ));
    assert_eq!(
        sql(&format!(
            "SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname={}",
            crate::quote_literal(&schema)
        )),
        "0"
    );
    fixture_idle();
    println!(
        "OVERVIEW_CLEANUP {}",
        serde_json::json!({"fixture":FIXTURE,"schema":schema,"schema_oid":owned.schema,"absent":true,"joined":true,"activity":0})
    );
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
