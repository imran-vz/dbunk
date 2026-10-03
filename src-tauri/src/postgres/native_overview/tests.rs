use super::*;
fn capture() -> OverviewCapture {
    OverviewCapture {
        database: "test".into(),
        database_oid: 42,
        reader_pid: 123,
        collected_start: "2026-10-03T00:00:00+00:00".into(),
        collected_end: "2026-10-03T00:00:01+00:00".into(),
    }
}
fn row(name: &str, oid: u32) -> RelationStats {
    RelationStats {
        identity: OverviewRelationIdentity {
            database_oid: 42,
            relation_oid: oid,
        },
        schema_oid: 123,
        schema: "public".into(),
        name: name.into(),
        kind: OverviewRelationKind::Table,
        is_partition: false,
        row_count_estimate: OverviewMetric::Unknown,
        total_size_bytes: OverviewMetric::Value(8192),
    }
}
fn page() -> RelationStatsSnapshot {
    RelationStatsSnapshot {
        capture: capture(),
        scope: RelationStatsScope::Database,
        schema_oid: None,
        relation_oid: None,
        totals: RelationStatsTotals {
            relation_count: 2,
            table_count: 2,
            view_count: 0,
            materialized_view_count: 0,
            row_count_estimate: OverviewMetric::Unknown,
            known_row_count_estimate: 0,
            unknown_estimate_relations: 2,
            total_size_bytes: OverviewMetric::Value(16384),
        },
        rows: vec![row("a", 1), row("雪", 2)],
        next_cursor: None,
    }
}
fn cursor() -> RelationStatsCursor {
    RelationStatsCursor {
        connection: "connection".into(),
        document: "document".into(),
        database: "test".into(),
        database_oid: 42,
        scope: RelationStatsScope::Database,
        schema_oid: None,
        relation_oid: None,
        schema: "public".into(),
        name: "雪".into(),
        oid: 2,
    }
}
#[test]
fn permission_is_not_timeout_or_arbitrary_database_error() {
    assert_eq!(
        reader::sqlstate(Some("42501")),
        CatalogError::OverviewPermission
    );
    for state in [None, Some("57014"), Some("42P01"), Some("22003")] {
        assert_eq!(reader::sqlstate(state), CatalogError::Database);
    }
}
#[test]
fn unknown_estimates_do_not_become_empty_tables() {
    assert_eq!(reader::estimate(12, 1), OverviewMetric::Unknown);
    assert_eq!(reader::estimate(0, 0), OverviewMetric::Value(0));
    let mut value = page();
    assert!(value.checked_heap_bytes().is_some());
    value.totals.row_count_estimate = OverviewMetric::Value(0);
    assert!(value.checked_heap_bytes().is_none());
    value.totals.row_count_estimate = OverviewMetric::Unknown;
    value.rows[0].kind = OverviewRelationKind::View;
    assert!(value.checked_heap_bytes().is_none());
    value.rows[0].row_count_estimate = OverviewMetric::NotApplicable;
    value.rows[0].total_size_bytes = OverviewMetric::NotApplicable;
    value.totals.table_count = 1;
    value.totals.view_count = 1;
    value.totals.unknown_estimate_relations = 1;
    assert!(value.checked_heap_bytes().is_some());
}
#[test]
fn cursor_refuses_connection_scope_or_oid_replacement() {
    let request = RelationStatsRequest {
        cursor: Some(cursor()),
        ..Default::default()
    };
    assert!(request.validate().is_ok());
    assert!(request.bind_document("document").is_ok());
    assert_eq!(
        request.bind_document("replacement"),
        Err(CatalogError::OverviewIdentityChanged)
    );
    assert_eq!(
        reader::check_cursor(&request, "connection", 42, "renamed_database", None, None),
        Err(CatalogError::OverviewIdentityChanged)
    );
    assert!(reader::check_cursor(&request, "connection", 42, "test", None, None).is_ok());
    for (connection, db, schema, relation) in [
        ("other", 42, None, None),
        ("connection", 43, None, None),
        ("connection", 42, Some(123), None),
        ("connection", 42, None, Some(2)),
    ] {
        assert_eq!(
            reader::check_cursor(&request, connection, db, "test", schema, relation),
            Err(CatalogError::OverviewIdentityChanged)
        );
    }
    let mut changed = request.clone();
    changed.scope = RelationStatsScope::Schema {
        name: "public".into(),
        expected_oid: Some(123),
    };
    assert!(changed.validate().is_err());
    changed = request;
    changed.expected_database_oid = Some(43);
    assert!(changed.validate().is_err());
}
#[test]
fn rows_and_continuations_preserve_exact_order_and_identity() {
    let mut value = page();
    value.next_cursor = Some(cursor());
    assert!(value.checked_heap_bytes().is_some());
    value.next_cursor.as_mut().unwrap().name = "different".into();
    assert!(value.checked_heap_bytes().is_none());
    value.next_cursor = None;
    value.rows.swap(0, 1);
    assert!(value.checked_heap_bytes().is_none());
    value.rows.swap(0, 1);
    value.rows[1].identity.relation_oid = 1;
    assert!(value.checked_heap_bytes().is_none());
}
#[test]
fn actual_spare_capacity_and_oversized_requests_are_refused() {
    let mut value = page();
    value.rows[0].name.reserve(MAX_OVERVIEW_BYTES);
    assert!(value.encoded_bytes().is_some());
    assert!(value.checked_heap_bytes().is_none());
    let mut name = String::with_capacity(9000);
    name.push('s');
    let request = RelationStatsRequest {
        scope: RelationStatsScope::Schema {
            name,
            expected_oid: None,
        },
        ..Default::default()
    };
    assert!(request.validate().is_err());
    let request = RelationStatsRequest {
        scope: RelationStatsScope::Schema {
            name: "雪".repeat(22),
            expected_oid: None,
        },
        ..Default::default()
    };
    assert!(request.validate().is_err());
}
#[test]
fn combined_capture_rejects_mixed_database_or_reader() {
    let database = DatabaseOverviewSnapshot {
        capture: capture(),
        database_size_bytes: OverviewMetric::Restricted,
        table_size_bytes: OverviewMetric::Value(0),
        index_size_bytes: OverviewMetric::Unknown,
        table_count: 0,
        schema_count: 0,
        row_count_estimate: OverviewMetric::Value(0),
        known_row_count_estimate: 0,
        unknown_estimate_relations: 0,
        index_count: 0,
        connection_count: OverviewMetric::Value(1),
    };
    let mut combined = OverviewSnapshot {
        database: Some(database),
        relations: page(),
    };
    assert!(combined.checked_heap_bytes().is_some());
    combined.database.as_mut().unwrap().capture.reader_pid += 1;
    assert!(combined.checked_heap_bytes().is_none());
    combined.database.as_mut().unwrap().capture.reader_pid -= 1;
    combined.relations.capture.database_oid += 1;
    assert!(combined.checked_heap_bytes().is_none());
}
#[test]
fn explicit_scope_is_not_silently_treated_as_database_scope() {
    let mut value = page();
    value.scope = RelationStatsScope::Relation {
        schema: "public".into(),
        name: "a".into(),
        expected: None,
    };
    value.schema_oid = Some(123);
    value.relation_oid = Some(1);
    assert!(value.checked_heap_bytes().is_none());
    value.rows.pop();
    value.totals.relation_count = 1;
    value.totals.table_count = 1;
    value.totals.unknown_estimate_relations = 1;
    assert!(value.checked_heap_bytes().is_some());
    value.rows[0].is_partition = true;
    assert!(value.checked_heap_bytes().is_some());
    value.scope = RelationStatsScope::Schema {
        name: "public".into(),
        expected_oid: Some(123),
    };
    assert!(value.checked_heap_bytes().is_none());
}
