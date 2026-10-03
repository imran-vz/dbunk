use super::*;
use dbunk_lib::backend::ddl_export::{
    DdlExportOmission as O, DdlExportRelation, DdlExportRelationKind, DdlExportSchema,
};
fn artifact(sql: String) -> DdlExportArtifact {
    DdlExportArtifact {
        connection_id: "connection".into(),
        database: "db".into(),
        database_oid: 42,
        reader_pid: 7,
        collected_start: "2026-10-03T00:00:00Z".into(),
        collected_end: "2026-10-03T00:00:01Z".into(),
        request: DdlExportRequest {
            scope: DdlExportScope::Relation {
                schema: "雪.schema".into(),
                name: "quoted\"table".into(),
                expected: None,
            },
            expected_database_oid: None,
        },
        schemas: vec![DdlExportSchema {
            oid: 10,
            name: "雪.schema".into(),
            declared: false,
        }],
        relations: vec![DdlExportRelation {
            identity: DdlExportIdentity {
                database_oid: 42,
                relation_oid: 123,
            },
            schema_oid: 10,
            schema: "雪.schema".into(),
            name: "quoted\"table".into(),
            kind: DdlExportRelationKind::Table,
            sql_start: 0,
            sql_end: sql.len() as u32,
        }],
        omissions: vec![
            O::Data,
            O::NonRelationObjects,
            O::DependencyOrdering,
            O::OwnershipAndPrivileges,
            O::TriggersRulesAndRowSecurity,
            O::StorageAndOrdinaryInheritance,
            O::ForeignServerAndUserMappings,
            O::SequenceObjectsAndCurrentValues,
            O::CommentsAndMaterializedViewIndexes,
        ],
        sql,
    }
}
#[test]
fn utf8_pages_cover_every_byte_and_bound_dense_breaks_and_long_lines() {
    for text in [
        "雪".repeat(22000),
        "\r\n".repeat(1000),
        format!(
            "{}雪\n{}",
            "x".repeat(PAGE_BYTES - 1),
            "e\u{301}".repeat(22000)
        ),
        String::new(),
    ] {
        let pages = Ranges::new(&text).collect::<Vec<_>>();
        let joined = pages
            .iter()
            .map(|range| &text[range.clone()])
            .collect::<String>();
        assert_eq!(joined, text);
        for range in pages {
            let page = &text[range];
            assert!(page.len() <= PAGE_BYTES);
            assert!(page.bytes().filter(|b| matches!(b, b'\r' | b'\n')).count() <= PAGE_BREAKS);
        }
    }
}
#[test]
fn capture_preserves_exact_sql_and_every_metadata_omission() {
    let sql =
        "CREATE TABLE \"雪.schema\".\"quoted\"\"table\"(n numeric DEFAULT 9223372036854775807);\n"
            .repeat(900);
    let c = Capture::new(artifact(sql.clone()), "connection", Rc::new(Cell::new(0))).unwrap();
    assert_eq!(
        (0..c.pages(Section::Sql))
            .map(|i| c.page(Section::Sql, i).unwrap())
            .collect::<String>(),
        sql
    );
    let metadata = (0..c.pages(Section::Metadata))
        .map(|i| c.page(Section::Metadata, i).unwrap())
        .collect::<String>();
    for omission in &c.artifact.omissions {
        assert!(metadata.contains(omission.explanation()));
    }
    assert!(metadata.contains("relation OID 123"));
    assert!(metadata.contains("CREATE SCHEMA included: false"));
    assert!(c.page(Section::Sql, c.pages(Section::Sql)).is_none());
}
#[test]
fn stale_retry_keeps_exact_observed_oids_until_capture_is_discarded() {
    let c = Capture::new(
        artifact("SELECT 1;".into()),
        "connection",
        Rc::new(Cell::new(0)),
    )
    .unwrap();
    for _ in 0..3 {
        let r = c.refresh_for(
            Scope::Relation
                .request("雪.schema", "quoted\"table")
                .unwrap(),
        );
        assert_eq!(r.expected_database_oid, Some(42));
        assert!(matches!(
            r.scope,
            DdlExportScope::Relation {
                expected: Some(DdlExportIdentity {
                    database_oid: 42,
                    relation_oid: 123
                }),
                ..
            }
        ));
    }
    let r = c.refresh_for(Scope::Relation.request("雪.schema", "different").unwrap());
    assert_eq!(r.expected_database_oid, None);
    let fresh = Scope::Relation
        .request("雪.schema", "quoted\"table")
        .unwrap();
    assert_eq!(fresh.expected_database_oid, None);
    assert!(Scope::Schema.request(&"雪".repeat(22), "").is_err());
    assert!(Scope::Schema.request("a\0b", "").is_err());
}
#[test]
fn failed_replacement_preserves_old_capture_and_file_owner_keeps_lease() {
    let budget = Rc::new(Cell::new(19));
    let c = Capture::new(artifact("\n".repeat(10000)), "connection", budget.clone()).unwrap();
    let old = budget.get();
    let lease = c.lease.clone();
    let work = c.file_lease().unwrap();
    let held = budget.get();
    let blocker = Lease::new(budget.clone(), SHARED_BYTES - held).unwrap();
    assert!(Capture::new(artifact("x".into()), "connection", budget.clone()).is_err());
    assert_eq!(budget.get(), SHARED_BYTES);
    assert_eq!(c.page(Section::Sql, 0).unwrap().len(), 128);
    drop(blocker);
    drop(c);
    assert_eq!(budget.get(), held);
    drop(work);
    assert_eq!(budget.get(), old);
    drop(lease);
    assert_eq!(budget.get(), 19);
}
#[test]
fn replacement_overlap_is_admitted_before_releasing_larger_capture() {
    let budget = Rc::new(Cell::new(0));
    let old = Capture::new(artifact("\n".repeat(10000)), "connection", budget.clone()).unwrap();
    let old_bytes = budget.get();
    let next = Capture::new(artifact("x".into()), "connection", budget.clone()).unwrap();
    let both = budget.get();
    assert!(both > old_bytes);
    drop(old);
    assert_eq!(budget.get(), both - old_bytes);
    drop(next);
    assert_eq!(budget.get(), 0);
}
#[test]
fn malformed_and_foreign_artifacts_refuse_without_retention() {
    let budget = Rc::new(Cell::new(7));
    assert!(Capture::new(artifact("x".into()), "other", budget.clone()).is_err());
    let mut bad = artifact("雪".into());
    bad.relations[0].sql_start = 1;
    assert!(Capture::new(bad, "connection", budget.clone()).is_err());
    let mut bad = artifact("x".into());
    bad.omissions.pop();
    assert!(Capture::new(bad, "connection", budget.clone()).is_err());
    let mut bad = artifact("x".into());
    bad.sql.reserve(9 * 1024 * 1024);
    assert!(Capture::new(bad, "connection", budget.clone()).is_err());
    assert_eq!(budget.get(), 7);
}
