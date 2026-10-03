use super::*;
fn context() -> OverviewCapture {
    OverviewCapture {
        database: "db".into(),
        database_oid: 11,
        reader_pid: 12,
        collected_start: "2026-10-03T10:00:00Z".into(),
        collected_end: "2026-10-03T10:00:01Z".into(),
    }
}
fn snapshot() -> OverviewSnapshot {
    OverviewSnapshot {
        database: Some(DatabaseOverviewSnapshot {
            capture: context(),
            database_size_bytes: OverviewMetric::Value(100),
            table_size_bytes: OverviewMetric::Value(80),
            index_size_bytes: OverviewMetric::Value(20),
            table_count: 1,
            schema_count: 1,
            row_count_estimate: OverviewMetric::Value(42),
            known_row_count_estimate: 42,
            unknown_estimate_relations: 0,
            index_count: 1,
            connection_count: OverviewMetric::Value(1),
        }),
        relations: RelationStatsSnapshot {
            capture: context(),
            scope: RelationStatsScope::Database,
            schema_oid: None,
            relation_oid: None,
            totals: RelationStatsTotals {
                relation_count: 1,
                table_count: 1,
                view_count: 0,
                materialized_view_count: 0,
                row_count_estimate: OverviewMetric::Value(42),
                known_row_count_estimate: 42,
                unknown_estimate_relations: 0,
                total_size_bytes: OverviewMetric::Value(80),
            },
            rows: vec![RelationStats {
                identity: OverviewRelationIdentity {
                    database_oid: 11,
                    relation_oid: 13,
                },
                schema_oid: 14,
                schema: "quoted.schema".into(),
                name: "Mixed Table".into(),
                kind: OverviewRelationKind::Table,
                is_partition: false,
                row_count_estimate: OverviewMetric::Value(42),
                total_size_bytes: OverviewMetric::Value(80),
            }],
            next_cursor: None,
        },
    }
}
fn capture(data: OverviewSnapshot) -> Capture {
    Capture::new(data, Rc::new(Cell::new(0)), None).unwrap()
}
fn detail(c: &Capture, section: Section, index: usize) -> String {
    c.details(section, index).unwrap().unwrap()
}
#[test]
fn eight_metrics_keep_exact_large_integers_counts_bytes_and_estimates_separate() {
    let mut s = snapshot();
    let database = s.database.as_mut().unwrap();
    database.database_size_bytes = OverviewMetric::Value(9_007_199_254_740_993);
    database.index_count = 9_007_199_254_740_993;
    let c = capture(s);
    assert_eq!(c.count(Section::Database), 8);
    assert!(detail(&c, Section::Database, 0).contains("9007199254740993"));
    let count = detail(&c, Section::Database, 6);
    assert!(count.contains("Indexes: 9007199254740993"));
    assert!(!count.contains("Exact returned byte count"));
    assert!(detail(&c, Section::Database, 7).contains("Includes this inspection connection"));
    assert!(detail(&c, Section::Database, 4).contains("not empty schemas"));
    assert!(detail(&c, Section::Database, 5).contains("not COUNT(*)"));
}
#[test]
fn restricted_unknown_not_applicable_and_zero_never_collapse() {
    let mut s = snapshot();
    let d = s.database.as_mut().unwrap();
    d.database_size_bytes = OverviewMetric::Value(0);
    d.table_size_bytes = OverviewMetric::Restricted;
    d.row_count_estimate = OverviewMetric::Unknown;
    d.known_row_count_estimate = 0;
    d.unknown_estimate_relations = 1;
    s.relations.rows[0].row_count_estimate = OverviewMetric::Unknown;
    s.relations.rows[0].total_size_bytes = OverviewMetric::Restricted;
    s.relations.totals.row_count_estimate = OverviewMetric::Unknown;
    s.relations.totals.known_row_count_estimate = 0;
    s.relations.totals.unknown_estimate_relations = 1;
    let c = capture(s);
    assert_eq!(
        c.row_label(Section::Database, 0).unwrap(),
        "Database size (bytes): 0"
    );
    assert!(
        c.row_label(Section::Database, 1)
            .unwrap()
            .contains("Restricted (permission denied)")
    );
    assert!(detail(&c, Section::Database, 5).contains("Estimated rows: Unknown"));
    let mut view = snapshot();
    view.database = None;
    view.relations.rows[0].kind = OverviewRelationKind::View;
    view.relations.rows[0].row_count_estimate = OverviewMetric::NotApplicable;
    view.relations.rows[0].total_size_bytes = OverviewMetric::NotApplicable;
    view.relations.totals.table_count = 0;
    view.relations.totals.view_count = 1;
    view.relations.totals.row_count_estimate = OverviewMetric::Value(0);
    view.relations.totals.known_row_count_estimate = 0;
    let c = capture(view);
    assert!(detail(&c, Section::Relations, 0).contains("Estimated rows: Not applicable"));
    assert!(c.details(Section::Database, 0).unwrap().is_none());
}
#[test]
fn older_metrics_are_retained_only_for_same_database_pages_with_their_own_interval() {
    let budget = Rc::new(Cell::new(0));
    let old = Capture::new(snapshot(), budget.clone(), None).unwrap();
    let mut later = snapshot();
    later.database = None;
    later.relations.capture.collected_start = "2026-10-03T11:00:00Z".into();
    later.relations.capture.collected_end = "2026-10-03T11:00:01Z".into();
    later.relations.rows[0].name = "next".into();
    let next = Capture::new(later, budget.clone(), Some(&old)).unwrap();
    assert_eq!(next.count(Section::Relations), 1);
    assert!(
        next.row_label(Section::Relations, 0)
            .unwrap()
            .contains("next")
    );
    assert!(
        !next
            .row_label(Section::Relations, 0)
            .unwrap()
            .contains("Mixed Table")
    );
    let metrics = detail(&next, Section::Database, 0);
    assert!(metrics.contains("2026-10-03T10:00:00Z"));
    assert!(metrics.contains("2026-10-03T11:00:00Z"));
    assert!(next.status().contains("retained from their earlier"));
    let mut different = snapshot();
    different.database = None;
    different.relations.capture.database_oid = 99;
    different.relations.rows[0].identity.database_oid = 99;
    let other = Capture::new(different, budget.clone(), Some(&old)).unwrap();
    assert_eq!(other.count(Section::Database), 0);
    let mut renamed = snapshot();
    renamed.database = None;
    renamed.relations.capture.database = "renamed".into();
    assert_eq!(
        Capture::new(renamed, budget, Some(&old))
            .unwrap()
            .count(Section::Database),
        0
    );
}
#[test]
fn exact_scope_names_and_observed_oid_refresh_binding_are_preserved() {
    let request = Scope::Relation
        .request(" quoted.schema ", "Mixed Table")
        .unwrap();
    assert!(
        matches!(&request.scope,RelationStatsScope::Relation{schema,name,..} if schema==" quoted.schema "&&name=="Mixed Table")
    );
    assert!(Scope::Schema.request("", "").is_err());
    assert!(Scope::Relation.request("a", &"雪".repeat(22)).is_err());
    assert!(Scope::Schema.request("a\0b", "").is_err());
    let mut s = snapshot();
    s.database = None;
    s.relations.scope = RelationStatsScope::Relation {
        schema: "quoted.schema".into(),
        name: "Mixed Table".into(),
        expected: None,
    };
    s.relations.schema_oid = Some(14);
    s.relations.relation_oid = Some(13);
    let c = capture(s);
    let raw_request = || {
        Scope::Relation
            .request("quoted.schema", "Mixed Table")
            .unwrap()
    };
    // Staleness is deliberately not an input: repeated retries after failed or
    // cancelled reads preserve the same observed OIDs until explicit reset.
    for _ in 0..3 {
        let retry = c.refresh_for(raw_request());
        assert_eq!(retry.expected_database_oid, Some(11));
        assert!(matches!(
            retry.scope,
            RelationStatsScope::Relation {
                expected: Some(OverviewRelationIdentity {
                    database_oid: 11,
                    relation_oid: 13
                }),
                ..
            }
        ));
    }
    let changed = c.refresh_for(Scope::Schema.request("other", "").unwrap());
    assert_eq!(changed.expected_database_oid, None);
    assert!(matches!(
        changed.scope,
        RelationStatsScope::Schema {
            expected_oid: None,
            ..
        }
    ));
    let refreshed = c.refresh_request();
    assert_eq!(refreshed.expected_database_oid, Some(11));
    assert!(refreshed.cursor.is_none());
    assert!(matches!(
        refreshed.scope,
        RelationStatsScope::Relation {
            expected: Some(OverviewRelationIdentity {
                database_oid: 11,
                relation_oid: 13
            }),
            ..
        }
    ));
    assert!(
        c.matches(
            &Scope::Relation
                .request("quoted.schema", "Mixed Table")
                .unwrap()
        )
    );
    assert!(
        !c.matches(
            &Scope::Relation
                .request("quoted.schema", "mixed table")
                .unwrap()
        )
    );
    assert!(!c.matches(&RelationStatsRequest::default()));
    assert!(c.next_request().is_none());
    let details = detail(&c, Section::Relations, 0);
    assert!(details.contains("\"quoted.schema\".\"Mixed Table\""));
    assert!(details.contains("Relation OID: 13"));
    assert!(details.contains("Explicit relation scope may inspect a child"));
}
#[test]
fn larger_old_detail_and_smaller_replacement_are_charged_together() {
    let budget = Rc::new(Cell::new(0));
    let mut large = snapshot();
    large.database.as_mut().unwrap().capture.database = "\n".repeat(63);
    large.relations.capture.database = "\n".repeat(63);
    let old = Capture::new(large, budget.clone(), None).unwrap();
    let old_bytes = old.retained_bytes();
    let new = Capture::new(snapshot(), budget.clone(), Some(&old)).unwrap();
    assert!(old_bytes > new.retained_bytes());
    assert_eq!(budget.get(), old_bytes + new.retained_bytes());
    drop(new);
    assert_eq!(budget.get(), old_bytes);
    let other = SHARED_BYTES - old_bytes - 1;
    budget.set(budget.get() + other);
    assert!(Capture::new(snapshot(), budget.clone(), Some(&old)).is_err());
    assert_eq!(budget.get(), SHARED_BYTES - 1);
    assert!(old.details(Section::Database, 0).unwrap().is_some());
    budget.set(budget.get() - other);
    drop(old);
    assert_eq!(budget.get(), 0);
}
#[test]
fn invalid_capacity_or_identity_refusal_keeps_shared_allowance_unchanged() {
    let budget = Rc::new(Cell::new(123));
    let mut inflated = snapshot();
    inflated.relations.rows[0].name.reserve(MAX_OVERVIEW_BYTES);
    assert!(Capture::new(inflated, budget.clone(), None).is_err());
    assert_eq!(budget.get(), 123);
    let mut foreign = snapshot();
    foreign.database.as_mut().unwrap().capture.database_oid = 999;
    assert!(Capture::new(foreign, budget.clone(), None).is_err());
    assert_eq!(budget.get(), 123);
    let mut unordered = snapshot();
    let mut second = unordered.relations.rows[0].clone();
    second.name = "A".into();
    unordered.relations.rows.push(second);
    unordered.relations.totals.table_count = 2;
    unordered.relations.totals.relation_count = 2;
    assert!(Capture::new(unordered, budget.clone(), None).is_err());
    assert_eq!(budget.get(), 123);
}
