use super::*;

fn column(number: i32, name: &str) -> StructureColumn {
    StructureColumn {
        number,
        name: name.into(),
        data_type: "integer".into(),
        nullable: true,
        default_expression: None,
        comment: None,
        identity: StructureIdentityKind::None,
        generated: StructureGeneratedKind::None,
        primary_key_position: None,
        collation_schema: None,
        collation_name: None,
    }
}
fn snapshot() -> TableStructureSnapshot {
    TableStructureSnapshot {
        identity: TableIdentity {
            database_oid: 1,
            relation_oid: 2,
        },
        schema: "public".into(),
        table: "child".into(),
        kind: StructureRelationKind::Table,
        owner: "dbunk".into(),
        comment: None,
        server_version: 160000,
        captured_at: "2026-10-03 00:00:00+00".into(),
        columns: vec![column(1, "a"), column(3, "b")],
        primary_key: None,
        outbound: vec![],
        inbound: vec![],
        indexes: vec![],
        constraints: vec![],
        triggers: vec![],
        row_security: StructureRowSecurity {
            enabled: false,
            forced: false,
        },
        policies: vec![],
        privileges: vec![],
        rules: vec![],
        partition_key: None,
        is_partition: false,
        partition_bound: None,
        parents: vec![],
        partitions: vec![],
    }
}
#[test]
fn structure_preserves_attnum_gaps_primary_key_order_and_empty_text() {
    let mut s = snapshot();
    s.columns[0].primary_key_position = Some(2);
    s.columns[1].primary_key_position = Some(1);
    s.columns[0].comment = Some(String::new());
    s.columns[0].default_expression = Some(String::new());
    s.primary_key = Some(StructurePrimaryKey {
        oid: 3,
        name: "pk".into(),
        columns: vec![
            StructureKeyColumn {
                number: 3,
                name: "b".into(),
            },
            StructureKeyColumn {
                number: 1,
                name: "a".into(),
            },
        ],
        deferrable: false,
        initially_deferred: false,
    });
    assert!(s.checked_heap_bytes().is_some());
    let value = serde_json::to_value(&s).unwrap();
    assert_eq!(value["columns"][0]["comment"], "");
    assert!(value["columns"][1]["comment"].is_null());
    s.primary_key.as_mut().unwrap().columns.swap(0, 1);
    assert!(s.checked_heap_bytes().is_none());
}
#[test]
fn structure_composite_foreign_keys_keep_pair_order_and_validate_local_identity() {
    let mut s = snapshot();
    s.outbound.push(StructureForeignKey {
        oid: 4,
        name: "fk".into(),
        source_oid: 2,
        source_schema: "public".into(),
        source_table: "child".into(),
        target_oid: 5,
        target_schema: "Other.Schema".into(),
        target_table: "parent".into(),
        columns: vec![
            StructureKeyPair {
                source_number: 3,
                source: "b".into(),
                target_number: 8,
                target: "x".into(),
            },
            StructureKeyPair {
                source_number: 1,
                source: "a".into(),
                target_number: 2,
                target: "y".into(),
            },
        ],
        on_update: StructureReferentialAction::Cascade,
        on_delete: StructureReferentialAction::Restrict,
        match_type: "FULL".into(),
        deferrable: true,
        initially_deferred: false,
        validated: false,
    });
    assert!(s.checked_heap_bytes().is_some());
    assert_eq!(s.outbound[0].columns[0].target, "x");
    let original = s.clone();
    s.outbound[0].source_oid = 77;
    assert!(s.checked_heap_bytes().is_none());
    s = original;
    s.outbound[0].columns[1].source_number = 2; // Dropped source attribute.
    assert!(s.checked_heap_bytes().is_none());
}
#[test]
fn structure_expression_and_include_positions_are_not_flattened() {
    let mut s = snapshot();
    s.indexes.push(StructureIndex {
        oid: 6,
        name: "expression_index".into(),
        method: "btree".into(),
        unique: false,
        primary: false,
        valid: true,
        ready: true,
        keys: vec![
            StructureIndexKey {
                position: 1,
                column_number: None,
                column_name: None,
                definition: "(a + 1)".into(),
                included: false,
            },
            StructureIndexKey {
                position: 2,
                column_number: Some(3),
                column_name: Some("b".into()),
                definition: "b".into(),
                included: true,
            },
        ],
        predicate: Some("a > 0".into()),
        definition: "CREATE INDEX ...".into(),
        constraint_oid: None,
    });
    assert!(s.checked_heap_bytes().is_some());
    let original = s.clone();
    s.indexes[0].keys[0].included = true;
    assert!(s.checked_heap_bytes().is_none());
    s = original;
    s.indexes[0].keys[1].position = 3;
    assert!(s.checked_heap_bytes().is_none());
}
#[test]
fn structure_partition_bound_and_closed_catalog_enums_are_truthful() {
    let mut s = snapshot();
    s.is_partition = true;
    assert!(s.checked_heap_bytes().is_none());
    s.partition_bound = Some("FOR VALUES FROM (0) TO (10)".into());
    assert!(s.checked_heap_bytes().is_some());
    assert!(serde_json::from_str::<StructureIdentityKind>("\"future-mode\"").is_err());
    assert!(serde_json::from_str::<StructureTriggerEnabled>("\"unknown\"").is_err());
}
#[test]
fn structure_refuses_spare_capacity_text_and_nested_component_overflow() {
    let mut s = snapshot();
    s.columns[0].name.reserve(MAX_STRUCTURE_BYTES);
    assert!(s.checked_heap_bytes().is_none());
    s = snapshot();
    s.comment = Some("x".repeat(MAX_STRUCTURE_METADATA_BYTES + 1));
    assert!(s.checked_heap_bytes().is_none());
    s = snapshot();
    s.policies = (1..=5)
        .map(|oid| StructurePolicy {
            oid,
            name: format!("policy_{oid}"),
            permissive: true,
            command: StructurePolicyCommand::All,
            roles: vec!["public".into(); 1024],
            using_expression: None,
            with_check: None,
        })
        .collect();
    assert!(s.checked_heap_bytes().is_none());
    // Escaping may exceed the encoded cap even when decoded text fits.
    s = snapshot();
    s.partition_key = Some("\u{1}".repeat(MAX_STRUCTURE_DEFINITION_BYTES));
    assert!(s.encoded_bytes().is_none());
}
#[test]
fn structure_predecode_admission_refusal_does_not_consume_budget() {
    let mut budget = reader::Budget::default();
    assert!(budget.admit("{}", 0).is_err());
    assert!(budget.admit("{}", 4097).is_err());
    assert!(budget.admit(&"x".repeat(MAX_STRUCTURE_BYTES), 1).is_err());
    // A previous refusal leaves the exact component allowance intact.
    assert!(budget.admit("{}", 4096).is_ok());
    assert!(budget.admit("{}", 1).is_err());
}
#[test]
fn structure_request_exact_names_and_identity_are_validated() {
    let mut r = TableStructureRequest {
        schema: "Quoted.Schema".into(),
        table: "Mixed Table".into(),
        expected: Some(TableIdentity {
            database_oid: 1,
            relation_oid: 2,
        }),
    };
    assert!(r.validate().is_ok());
    r.table = "é".repeat(32);
    assert!(r.validate().is_err());
    r.table = "table".into();
    r.expected.as_mut().unwrap().database_oid = 0;
    assert!(r.validate().is_err());
}

#[test]
fn structure_capability_floor_precedes_version_specific_catalogs() {
    for old in [90600, 100_000, 110_000, 120_022] {
        assert_eq!(
            reader::supported_version(old),
            Err(CatalogError::StructureVersion)
        );
    }
    assert_eq!(
        reader::supported_version(-1),
        Err(CatalogError::InvalidResponse)
    );
    for supported in [130_000, 140_015, 160_004, 180_000] {
        assert_eq!(reader::supported_version(supported), Ok(supported as u32));
    }
    let mut s = snapshot();
    s.server_version = 120_000;
    assert!(s.checked_heap_bytes().is_none());
}
#[test]
fn structure_permission_failure_never_becomes_empty_metadata() {
    assert_eq!(
        reader::classify_sqlstate(Some("42501")),
        CatalogError::StructurePermission
    );
    // Missing objects, unknown columns, cancelled statements and transport
    // failures must not be mislabeled as permission restrictions.
    for code in [Some("42P01"), Some("42703"), Some("57014"), None] {
        assert_eq!(reader::classify_sqlstate(code), CatalogError::Database);
    }
}
