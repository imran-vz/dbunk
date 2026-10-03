use super::reader::Key;
use super::*;
fn key(table_oid: u32, oid: u32, columns: &[i16]) -> Key {
    Key {
        table_oid,
        oid,
        columns: columns.to_vec(),
    }
}
fn table(db: u32, oid: u32, schema: &str, name: &str) -> SchemaMapTable {
    SchemaMapTable {
        identity: SchemaMapIdentity {
            database_oid: db,
            relation_oid: oid,
        },
        schema_oid: if schema == "a" { 10 } else { 11 },
        schema: schema.into(),
        name: name.into(),
        kind: SchemaMapTableKind::Table,
        external: false,
        columns: vec![SchemaMapColumn {
            attnum: 1,
            name: "id".into(),
            data_type: "integer".into(),
            nullable: false,
            primary_key: true,
            comment: Some("quoted 雪".into()),
        }],
        triggers: vec![],
        junction: false,
    }
}
fn graph() -> SchemaMapSnapshot {
    let source = table(1, 2, "a", "b.c");
    let mut target = table(1, 3, "a.b", "c");
    target.external = true;
    let edge = SchemaMapForeignKey {
        database_oid: 1,
        constraint_oid: 4,
        name: "same.name".into(),
        source: source.identity,
        target: target.identity,
        columns: vec![SchemaMapColumnPair {
            source: 1,
            target: 1,
        }],
        on_update: SchemaMapAction::Cascade,
        on_delete: SchemaMapAction::NoAction,
        match_type: "SIMPLE".into(),
        validated: true,
        deferrable: false,
        columns_nullable: false,
        columns_unique: true,
        cardinality: SchemaMapCardinality::OneToOne,
        cardinality_reason: "Referencing columns are constrained unique on the referencing table"
            .into(),
        junction_participant: false,
    };
    SchemaMapSnapshot {
        database: "db".into(),
        database_oid: 1,
        captured_at: "2026-10-03T12:00:00.000000Z".into(),
        server_version: 170000,
        scope: SchemaMapScope::Schema {
            name: "a".into(),
            expected_oid: None,
        },
        schema_oid: Some(10),
        focus: None,
        tables: vec![source, target],
        foreign_keys: vec![edge],
    }
}
#[test]
fn key_subset_uniqueness_and_junctions_use_relation_and_constraint_identity() {
    let keys = [key(10, 100, &[1]), key(11, 101, &[1, 2])];
    assert!(classify::unique(&[1, 2], &[&keys[0]]));
    assert!(!classify::unique(&[1], &[&keys[1]]));
    let outgoing = [
        key(10, 200, &[1]),
        key(10, 201, &[2]),
        key(11, 202, &[1]),
        key(11, 203, &[2]),
    ];
    let junctions = classify::junctions(&outgoing, &keys);
    assert!(!junctions.tables.contains(&10));
    assert!(junctions.tables.contains(&11));
    assert_eq!(
        junctions.constraints.into_iter().collect::<Vec<_>>(),
        vec![202, 203]
    );
    // A single FK covering the whole identity is inheritance, not a junction.
    assert!(
        classify::junctions(&[key(11, 204, &[1, 2]), key(11, 205, &[1])], &keys)
            .tables
            .is_empty()
    );
}
#[test]
fn dotted_identifiers_external_targets_and_exact_refresh_do_not_collide() {
    let g = graph();
    assert!(g.checked_heap_bytes().is_some());
    assert_ne!(g.tables[0].identity, g.tables[1].identity);
    let request = g.refresh_request();
    assert_eq!(request.expected_database_oid, Some(1));
    assert!(matches!(
        request.scope,
        SchemaMapScope::Schema {
            expected_oid: Some(10),
            ..
        }
    ));
    assert!(SchemaMapRequest {
        scope: SchemaMapScope::Relation {
            schema: "a.b".into(),
            table: "c".into(),
            expected: Some(g.tables[1].identity)
        },
        expected_database_oid: Some(1)
    }
    .validate()
    .is_ok());
    let mut broken = g;
    broken.tables[1].external = false;
    assert!(broken.checked_heap_bytes().is_none());
}
#[test]
fn relation_scope_refuses_neighbor_edges_missing_anchors_and_orphan_nodes() {
    let mut g = graph();
    g.tables[1].external = false;
    let focus = g.tables[0].identity;
    g.scope = SchemaMapScope::Relation {
        schema: "a".into(),
        table: "b.c".into(),
        expected: Some(focus),
    };
    g.focus = Some(focus);
    assert!(g.checked_heap_bytes().is_some());
    let unrelated = table(1, 8, "a", "neighbor");
    g.tables.push(unrelated.clone());
    assert!(g.checked_heap_bytes().is_none());
    let mut edge = g.foreign_keys[0].clone();
    edge.constraint_oid = 9;
    edge.source = g.tables[1].identity;
    edge.target = unrelated.identity;
    g.foreign_keys.push(edge);
    assert!(g.checked_heap_bytes().is_none());
    g.tables.pop();
    g.foreign_keys.pop();
    g.foreign_keys[0].columns[0].target = 2;
    assert!(g.checked_heap_bytes().is_none());
}
#[test]
fn actual_capacity_and_text_bounds_refuse_without_constructing_a_partial_graph() {
    let mut g = graph();
    g.tables[0].name.reserve(MAX_SCHEMA_MAP_BYTES);
    assert!(g.checked_heap_bytes().is_none());
    let mut g = graph();
    g.tables[0].columns[0].comment = Some("x".repeat(MAX_SCHEMA_MAP_COMMENT_BYTES + 1));
    assert!(g.checked_heap_bytes().is_none());
    let mut request = SchemaMapRequest {
        scope: SchemaMapScope::Schema {
            name: "a".into(),
            expected_oid: None,
        },
        expected_database_oid: None,
    };
    if let SchemaMapScope::Schema { name, .. } = &mut request.scope {
        name.reserve(8192);
    }
    assert!(request.validate().is_err());
}
#[test]
fn predecode_accounting_refuses_before_json_or_vector_growth_and_does_not_advance() {
    let mut budget = reader::Budget::default();
    budget.admit(100, 2).unwrap();
    assert_eq!(
        budget.admit(MAX_SCHEMA_MAP_BYTES, 1),
        Err(CatalogError::SchemaMapLimit)
    );
    // Failed admission must not poison a later small component.
    assert!(budget.admit(100, 2).is_ok());
    assert_eq!(budget.admit(0, 0), Err(CatalogError::InvalidResponse));
}
#[test]
fn permission_errors_and_unknown_metadata_stay_errors() {
    assert_eq!(
        reader::sqlstate(Some("42501")),
        CatalogError::SchemaMapPermission
    );
    assert_eq!(reader::sqlstate(Some("57014")), CatalogError::Database);
    let mut g = graph();
    g.foreign_keys[0].columns_nullable = true;
    assert!(g.checked_heap_bytes().is_none());
    let mut g = graph();
    g.foreign_keys[0].constraint_oid = 0;
    assert!(g.checked_heap_bytes().is_none());
}
#[test]
fn compact_trigger_column_anchors_and_events_are_validated() {
    let mut g = graph();
    g.tables[0].triggers.push(SchemaMapTrigger {
        oid: 20,
        name: "trg".into(),
        columns: vec![1],
        timing: "BEFORE".into(),
        events: vec!["UPDATE".into()],
        orientation: "ROW".into(),
        enabled: SchemaMapTriggerEnabled::Origin,
        function_oid: 21,
        function_schema: "a".into(),
        function_name: "trigger_fn".into(),
    });
    assert!(g.checked_heap_bytes().is_some());
    g.tables[0].triggers[0].columns[0] = 2;
    assert!(g.checked_heap_bytes().is_none());
}
