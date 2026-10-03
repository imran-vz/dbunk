//! Explicit read-only probe. No roles, schemas, data or settings are created.
use super::*;
use crate::backend::profile;
use futures_util::FutureExt;
use std::{path::Path, process::Command};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
fn fixture_idle() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let result = Command::new("python3").arg("-c")
        .arg("import sys;sys.path.insert(0,sys.argv[1]);import fixture;owned,target=fixture.check();assert owned['instance']==sys.argv[2];assert fixture.sql(target,\"SET statement_timeout='5s'; SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()\").strip()=='0';print('verified owned fixture idle: '+owned['instance'])")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE).output().expect("fixture ownership helper");
    assert!(
        result.status.success(),
        "Owned fixture identity/activity check refused"
    );
    println!("{}", String::from_utf8(result.stdout).unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "explicit read-only owned stage03 overview probe, identity/activity guard required"]
async fn native_overview_owned_identity_scopes_and_joined_cleanup() {
    fixture_idle();
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let outcome = std::panic::AssertUnwindSafe(async {
        let doc = backend
            .open_data_document("overview-live", "overview", &backend.fixture().id)
            .await
            .unwrap();
        let all = backend
            .overview(&doc, RelationStatsRequest::default())
            .await
            .unwrap();
        assert!(all.checked_heap_bytes().is_some());
        assert_eq!(all.relations.capture.database, "dbunk_demo");
        let db = all.database.as_ref().unwrap();
        assert!(matches!(db.database_size_bytes,OverviewMetric::Value(n) if n>0));
        assert!(matches!(db.connection_count,OverviewMetric::Value(n) if n>=1));
        if let Some(cursor) = all.relations.next_cursor.clone() {
            let next = backend
                .overview(
                    &doc,
                    RelationStatsRequest {
                        cursor: Some(cursor),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert!(next.database.is_none());
            assert!(next.relations.rows.iter().all(|r| !all
                .relations
                .rows
                .iter()
                .any(|old| old.identity == r.identity)));
        }
        // Explicit relation scope also supports system catalogs, unlike the
        // intentionally user-scoped database-wide listing.
        let request = RelationStatsRequest {
            scope: RelationStatsScope::Relation {
                schema: "pg_catalog".into(),
                name: "pg_class".into(),
                expected: None,
            },
            expected_database_oid: Some(db.capture.database_oid),
            cursor: None,
        };
        let one = backend.overview(&doc, request.clone()).await.unwrap();
        assert!(one.database.is_none());
        assert_eq!(one.relations.rows.len(), 1);
        assert_eq!(one.relations.totals.relation_count, 1);
        let identity = one.relations.rows[0].identity;
        let mut exact = request;
        exact.scope = RelationStatsScope::Relation {
            schema: "pg_catalog".into(),
            name: "pg_class".into(),
            expected: Some(identity),
        };
        backend.overview(&doc, exact.clone()).await.unwrap();
        if let RelationStatsScope::Relation {
            expected: Some(id), ..
        } = &mut exact.scope
        {
            id.relation_oid = id.relation_oid.wrapping_add(1);
        }
        assert!(matches!(
            backend.overview(&doc, exact).await,
            Err(DataError::Catalog(
                crate::postgres::native_catalog::CatalogError::OverviewIdentityChanged
            ))
        ));
        let schema = backend
            .overview(
                &doc,
                RelationStatsRequest {
                    scope: RelationStatsScope::Schema {
                        name: "pg_catalog".into(),
                        expected_oid: one.relations.schema_oid,
                    },
                    expected_database_oid: Some(identity.database_oid),
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert!(schema.database.is_none());
        assert!(schema
            .relations
            .rows
            .iter()
            .all(|r| r.schema == "pg_catalog" && !r.is_partition));
        assert!(schema.relations.totals.relation_count >= schema.relations.rows.len() as i64);
        backend.close_data_document(&doc).await.unwrap();
        assert!(backend
            .overview(&doc, RelationStatsRequest::default())
            .await
            .is_err());
    })
    .catch_unwind()
    .await;
    backend.shutdown().await.unwrap();
    fixture_idle();
    println!("OVERVIEW_READ_ONLY_CLEANUP fixture={FIXTURE} activity=0 no_fixture_objects_created");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

mod paging;
