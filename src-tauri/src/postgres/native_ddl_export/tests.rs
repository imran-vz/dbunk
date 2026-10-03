use super::*;
fn relation() -> DdlExportRelation {
    DdlExportRelation {
        identity: DdlExportIdentity {
            database_oid: 42,
            relation_oid: 123,
        },
        schema_oid: 10,
        schema: "雪.schema".into(),
        name: "quoted\"table".into(),
        kind: DdlExportRelationKind::Table,
        sql_start: 0,
        sql_end: 0,
    }
}
fn artifact() -> DdlExportArtifact {
    let mut relation = relation();
    let mut sql = render::Sql::new().unwrap();
    sql.relation(
        &mut relation,
        "CREATE TABLE \"雪.schema\".\"quoted\"\"table\"(id int);",
    )
    .unwrap();
    DdlExportArtifact {
        connection_id: "connection".into(),
        database: "test".into(),
        database_oid: 42,
        reader_pid: 1,
        collected_start: "2026-10-03T00:00:00+00:00".into(),
        collected_end: "2026-10-03T00:00:01+00:00".into(),
        request: DdlExportRequest {
            scope: DdlExportScope::Relation {
                schema: relation.schema.clone(),
                name: relation.name.clone(),
                expected: Some(relation.identity),
            },
            expected_database_oid: Some(42),
        },
        schemas: vec![DdlExportSchema {
            oid: 10,
            name: relation.schema.clone(),
            declared: false,
        }],
        relations: vec![relation],
        omissions: types::OMISSIONS.to_vec(),
        sql: sql.finish(),
    }
}
#[test]
fn exact_ranges_preserve_unicode_and_quoted_names() {
    let mut value = artifact();
    assert!(value.checked_heap_bytes().is_some());
    assert_eq!(
        value.relation_sql(0),
        Some("CREATE TABLE \"雪.schema\".\"quoted\"\"table\"(id int);")
    );
    let position = value.sql.find('雪').unwrap();
    value.relations[0].sql_start = (position + 1) as u32;
    assert!(value.relation_sql(0).is_none());
    assert!(value.checked_heap_bytes().is_none());
}
#[test]
fn scope_and_database_replacement_are_not_valid_artifacts() {
    let mut value = artifact();
    value.database_oid = 43;
    assert!(value.checked_heap_bytes().is_none());
    let mut value = artifact();
    value.relations[0].identity.relation_oid = 124;
    assert!(value.checked_heap_bytes().is_none());
    let mut value = artifact();
    value.schemas[0].oid = 11;
    assert!(value.checked_heap_bytes().is_none());
    let mut value = artifact();
    value.request.scope = DdlExportScope::Schema {
        name: "other".into(),
        expected_oid: Some(10),
    };
    assert!(value.checked_heap_bytes().is_none());
}
#[test]
fn scoped_schema_declarations_and_omissions_are_exact() {
    let schema = DdlExportSchema {
        oid: 10,
        name: "quoted\"雪".into(),
        declared: true,
    };
    let mut sql = render::Sql::new().unwrap();
    sql.schema(&schema).unwrap();
    let text = sql.finish();
    assert!(text.ends_with("CREATE SCHEMA IF NOT EXISTS \"quoted\"\"雪\";\n\n"));
    for omission in types::OMISSIONS {
        assert!(text.contains(omission.explanation()));
    }
    let mut value = artifact();
    value.omissions.pop();
    assert!(value.checked_heap_bytes().is_none());
    let value = artifact();
    assert!(!format!("{value:?}").contains("CREATE TABLE"));
}
#[test]
fn source_and_encoded_budgets_refuse_before_append() {
    let mut sql = render::Sql::new().unwrap();
    let before = sql.finish();
    sql = render::Sql::new().unwrap();
    assert!(matches!(
        sql.push(&"x".repeat(MAX_DDL_EXPORT_SQL_BYTES)),
        Err(CatalogError::DdlExportLimit)
    ));
    assert_eq!(sql.finish(), before);
    let mut sql = render::Sql::new().unwrap();
    // Decoded text fits4MiB, but JSON escaping would exceed16MiB delivery.
    assert!(matches!(
        sql.push(&"\x01".repeat(3 * 1024 * 1024)),
        Err(CatalogError::DdlExportLimit)
    ));
    assert_eq!(sql.finish(), before);
}
#[test]
fn actual_capacity_and_request_bounds_are_enforced() {
    let mut value = artifact();
    value.sql.reserve(MAX_DDL_EXPORT_HEAP_BYTES);
    assert!(value.encoded_bytes().is_some());
    assert!(value.checked_heap_bytes().is_none());
    let mut name = String::with_capacity(8192);
    name.push('a');
    assert!(DdlExportRequest {
        scope: DdlExportScope::Schema {
            name,
            expected_oid: None
        },
        expected_database_oid: None
    }
    .validate()
    .is_err());
    for name in ["".into(), "x\0y".into(), "雪".repeat(22)] {
        assert!(DdlExportRequest {
            scope: DdlExportScope::Schema {
                name,
                expected_oid: None
            },
            expected_database_oid: None
        }
        .validate()
        .is_err());
    }
}
#[test]
fn empty_schema_remains_an_explicit_schema_export() {
    let mut value = artifact();
    value.request.scope = DdlExportScope::Schema {
        name: "雪.schema".into(),
        expected_oid: Some(10),
    };
    value.schemas[0].declared = true;
    value.relations.clear();
    let mut sql = render::Sql::new().unwrap();
    sql.schema(&value.schemas[0]).unwrap();
    value.sql = sql.finish();
    assert!(value.checked_heap_bytes().is_some());
    assert!(value.sql.contains("CREATE SCHEMA"));
    value.schemas.clear();
    assert!(value.checked_heap_bytes().is_none());
}
#[test]
fn only_permission_sqlstate_gets_permission_classification() {
    assert_eq!(
        reader::sqlstate(Some("42501")),
        CatalogError::DdlExportPermission
    );
    for code in [None, Some("57014"), Some("42P01")] {
        assert_eq!(reader::sqlstate(code), CatalogError::Database);
    }
}
