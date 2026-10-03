use super::*;
fn reference(kind: PgObjectKind) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: Some("schema.with.dot".into()),
        name: "table\"name".into(),
        identity_args: None,
    }
}
fn header() -> Header {
    Header {
        oid: 42,
        owner: Some("owner".into()),
        comment: None,
        version: 170_000,
        partition_key: None,
        partition_bound: None,
        parent_schema: None,
        parent_name: None,
        server: None,
    }
}
fn column() -> Column {
    Column {
        number: 1,
        name: "id".into(),
        data_type: "bigint".into(),
        not_null: true,
        default_expr: None,
        identity: String::new(),
        generated: String::new(),
        collation_schema: None,
        collation_name: None,
        seq_start: None,
        seq_increment: None,
        seq_min: None,
        seq_max: None,
        seq_cache: None,
        seq_cycle: None,
    }
}

#[test]
fn identity_and_generated_columns_never_become_false_defaults() {
    let mut identity = column();
    identity.identity = "a".into();
    identity.seq_start = Some("9223372036854775800".into());
    identity.seq_increment = Some("-1".into());
    identity.seq_min = Some("-9223372036854775808".into());
    identity.seq_max = Some("9223372036854775807".into());
    identity.seq_cache = Some("10".into());
    identity.seq_cycle = Some(false);
    let mut generated = column();
    generated.number = 2;
    generated.name = "computed".into();
    generated.generated = "s".into();
    generated.default_expr = Some("id + 1".into());
    let description = render(
        reference(PgObjectKind::Table),
        header(),
        vec![identity, generated],
        vec![],
        vec![],
        BTreeMap::new(),
    )
    .unwrap();
    let sql = description.definition_sql.unwrap();
    assert!(sql.contains("GENERATED ALWAYS AS IDENTITY (START WITH 9223372036854775800 INCREMENT BY -1 MINVALUE -9223372036854775808 MAXVALUE 9223372036854775807 CACHE 10 NO CYCLE)"));
    assert!(sql.contains("\"computed\" bigint GENERATED ALWAYS AS (id + 1) STORED NOT NULL"));
    assert!(!sql.contains(" DEFAULT "));
    assert!(matches!(description.facts, PgObjectFacts::Table));
}

#[test]
fn partitions_omit_columns_and_preserve_qualified_parent_bound_and_local_constraints() {
    let mut leaf = header();
    leaf.parent_schema = Some("parent.schema".into());
    leaf.parent_name = Some("parent\"table".into());
    leaf.partition_bound = Some("FOR VALUES FROM ('2024-01-01') TO ('2025-01-01')".into());
    let description = render(
        reference(PgObjectKind::Table),
        leaf,
        vec![],
        vec![],
        vec![],
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(description.definition_sql.unwrap(), "CREATE TABLE \"schema.with.dot\".\"table\"\"name\" PARTITION OF \"parent.schema\".\"parent\"\"table\" FOR VALUES FROM ('2024-01-01') TO ('2025-01-01');");
    let mut parent = header();
    parent.partition_key = Some("RANGE (id)".into());
    let sql = render(
        reference(PgObjectKind::Table),
        parent,
        vec![column()],
        vec![Constraint {
            name: "positive".into(),
            definition: "CHECK (id > 0)".into(),
        }],
        vec![Index {
            definition: "CREATE INDEX child_idx ON x(id);".into(),
        }],
        BTreeMap::new(),
    )
    .unwrap()
    .definition_sql
    .unwrap();
    assert!(sql.contains("CONSTRAINT \"positive\" CHECK (id > 0)\n) PARTITION BY RANGE (id);\nCREATE INDEX child_idx ON x(id);"));
}

#[test]
fn foreign_options_collation_defaults_and_checks_are_exact_catalog_text() {
    let mut h = header();
    h.server = Some("server.with.dot".into());
    let mut c = column();
    c.data_type = "text".into();
    c.default_expr = Some("'x'::text".into());
    c.collation_schema = Some("pg_catalog".into());
    c.collation_name = Some("C".into());
    let options = BTreeMap::from([
        (0, vec!["schema_name=foreign=with=equals".into()]),
        (1, vec!["column_name=x\\y'z".into()]),
    ]);
    let result = render(
        reference(PgObjectKind::ForeignTable),
        h,
        vec![c],
        vec![Constraint {
            name: "nonempty".into(),
            definition: "CHECK (id <> ''::text)".into(),
        }],
        vec![],
        options,
    )
    .unwrap();
    let sql = result.definition_sql.unwrap();
    assert!(sql.contains("OPTIONS (\"column_name\" E'x\\\\y''z') COLLATE \"pg_catalog\".\"C\" DEFAULT 'x'::text NOT NULL"));
    assert!(sql
        .ends_with("SERVER \"server.with.dot\" OPTIONS (\"schema_name\" E'foreign=with=equals');"));
    assert!(
        matches!(result.facts, PgObjectFacts::ForeignTable { server } if server == "server.with.dot")
    );
}

#[test]
fn malformed_options_or_missing_identity_metadata_refuse_instead_of_rewriting() {
    for option in ["no equals", "=empty-name"] {
        assert!(matches!(
            render_options(&mut Sql::default(), Some(&[option.into()])),
            Err(CatalogError::InvalidResponse)
        ));
    }
    let mut c = column();
    c.identity = "d".into();
    assert!(matches!(
        render_column(&mut Sql::default(), &c, None, false),
        Err(CatalogError::InvalidResponse)
    ));
    c.identity.clear();
    c.generated = "?".into();
    c.default_expr = Some("id + 1".into());
    assert!(matches!(
        render_column(&mut Sql::default(), &c, None, false),
        Err(CatalogError::InvalidResponse)
    ));
}

#[test]
fn oversized_reconstruction_is_refused_before_concatenating_components() {
    let mut a = column();
    a.default_expr = Some("x".repeat(MAX_DESCRIPTION_TEXT_BYTES / 2));
    let mut b = column();
    b.default_expr = a.default_expr.clone();
    assert_eq!(
        render(
            reference(PgObjectKind::Table),
            header(),
            vec![a, b],
            vec![],
            vec![],
            BTreeMap::new()
        ),
        Err(CatalogError::DescriptionLimit)
    );
}

#[test]
fn pre12_columns_do_not_reference_an_absent_generated_attribute() {
    assert!(!columns_sql(110_000).contains("a.attgenerated"));
    assert!(columns_sql(120_000).contains("a.attgenerated::text"));
    // Catalog predicates prevent inherited/constraint-owned index duplication.
    assert!(INDEXES.contains("con.conindid = idx.oid"));
    assert!(INDEXES.contains("inherited.inhrelid = idx.oid"));
    assert!(CONSTRAINTS.contains("con.conislocal"));
}
