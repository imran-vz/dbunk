use super::*;
fn data() -> TableExportData {
    TableExportData {
        connection_id: "connection".into(),
        database: "db".into(),
        schema: "a.b".into(),
        table: "t\"q".into(),
        identity: TableExportIdentity {
            database_oid: 1,
            relation_oid: 2,
        },
        schema_oid: 3,
        kind: TableExportKind::Table,
        row_security: false,
        captured_start: "2026-10-03T00:00:00Z".into(),
        captured_end: "2026-10-03T00:00:01Z".into(),
        columns: vec![TableExportColumn {
            name: "value".into(),
            attnum: 1,
            type_oid: 25,
            type_modifier: -1,
            collation_oid: 100,
        }],
        rows: vec![
            vec![None],
            vec![Some(String::new())],
            vec![Some("18446744073709551615".into())],
        ],
    }
}
#[test]
fn exact_values_and_capacity_and_encoded_limits_are_independent() {
    let mut value = data();
    assert!(value.checked_heap_bytes().is_some());
    let capture = TableExportCapture(Arc::new(value));
    let clone = capture.clone();
    assert!(Arc::ptr_eq(&capture.0, &clone.0));
    assert!(!format!("{capture:?}").contains("18446744073709551615"));
    assert_eq!(capture.data().rows[0][0], None);
    assert_eq!(capture.data().rows[1][0].as_deref(), Some(""));
    value = data();
    value.rows[0][0] = Some(String::with_capacity(MAX_TABLE_EXPORT_HEAP_BYTES));
    assert!(value.checked_heap_bytes().is_none());
    value = data();
    value.rows = (0..8)
        .map(|_| vec![Some("\u{1}".repeat(MAX_TABLE_EXPORT_FIELD_BYTES - 1))])
        .collect();
    assert!(value.encoded_bytes().is_none());
    assert!(value.checked_heap_bytes().is_none());
}
#[test]
fn headers_count_toward_cell_limit_and_inconsistent_rows_refuse() {
    assert_eq!(bounds::row_limit(1), 99_999);
    assert_eq!(bounds::row_limit(1024), 96);
    assert_eq!(bounds::row_limit(0), 100_000);
    let mut value = data();
    value.rows = vec![vec![]];
    assert!(value.checked_heap_bytes().is_none());
    value.rows.clear();
    assert!(value.checked_heap_bytes().is_some());
    value.columns[0].name.clear();
    assert!(value.checked_heap_bytes().is_none());
}
#[test]
fn generated_query_keeps_exact_names_canonical_output_and_empty_relation_witness() {
    let value = data();
    let request = TableExportRequest {
        schema: value.schema.clone(),
        table: value.table.clone(),
        expected: None,
    };
    let sql = sql::capture(&request, &value.columns).unwrap();
    assert!(sql.contains("FROM \"a.b\".\"t\"\"q\" AS t"));
    assert!(sql.contains("CASE WHEN false THEN t ELSE NULL END AS whole"));
    assert!(sql.contains("pg_catalog.pg_typeof(g.whole)::oid"));
    assert!(sql.contains("RIGHT JOIN (VALUES(1))"));
    assert!(sql.contains("pg_catalog.format('%s',t.\"value\")"));
    assert!(!sql.contains("t.\"value\"::text"));
    let mut invalid = request;
    invalid.expected = Some(TableExportIdentity {
        database_oid: 0,
        relation_oid: 1,
    });
    assert!(invalid.validate().is_err());
}
