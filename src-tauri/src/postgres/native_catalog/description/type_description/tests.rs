use super::*;
fn reference(kind: PgObjectKind) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: Some("types.with.dot".into()),
        name: "name\"quoted".into(),
        identity_args: None,
    }
}
fn header(kind: &str) -> Header {
    Header {
        oid: 1,
        kind: kind.into(),
        owner: Some("owner".into()),
        comment: Some("exact comment".into()),
        version: 170_000,
    }
}
fn range() -> Range {
    Range {
        subtype: "integer".into(),
        range_schema: "types.with.dot".into(),
        range_name: "range_name".into(),
        subtype_schema: "pg_catalog".into(),
        subtype_name: "int4".into(),
        multirange_schema: Some("types.with.dot".into()),
        multirange_name: Some("name\"quoted".into()),
        opclass_schema: Some("pg_catalog".into()),
        opclass_name: Some("int4_ops".into()),
        collation_schema: Some("collation.schema".into()),
        collation_name: Some("C".into()),
        canonical_schema: Some("functions".into()),
        canonical_name: Some("canonical".into()),
        diff_schema: Some("functions".into()),
        diff_name: Some("difference".into()),
    }
}

#[test]
fn enum_order_empty_label_unicode_and_literals_are_preserved() {
    let labels = vec!["".into(), "日本語".into(), "quo'te\\slash".into()];
    let result = render_enum(reference(PgObjectKind::Type), header("e"), labels.clone()).unwrap();
    assert_eq!(result.definition_sql.as_deref(), Some("CREATE TYPE \"types.with.dot\".\"name\"\"quoted\" AS ENUM (E'', E'日本語', E'quo''te\\\\slash');"));
    assert!(
        matches!(result.facts, PgObjectFacts::Type { enum_labels: Some(actual), .. } if actual == labels)
    );
}
#[test]
fn composite_attributes_keep_order_type_spelling_and_nullable_facts() {
    let attributes = vec![
        PgTypeAttribute {
            name: "amount".into(),
            data_type: "numeric(30,10)".into(),
            nullable: false,
        },
        PgTypeAttribute {
            name: "x\"y".into(),
            data_type: "\"Elsewhere\".\"Type\"[]".into(),
            nullable: true,
        },
    ];
    let result = render_composite(
        reference(PgObjectKind::Type),
        header("c"),
        attributes.clone(),
    )
    .unwrap();
    assert!(result
        .definition_sql
        .unwrap()
        .contains("\"amount\" numeric(30,10),\n  \"x\"\"y\" \"Elsewhere\".\"Type\"[]"));
    assert!(
        matches!(result.facts, PgObjectFacts::Type { attributes: Some(actual), .. } if actual == attributes)
    );
}
#[test]
fn range_options_and_multirange_parent_definition_keep_qualified_components() {
    let result = render_range(reference(PgObjectKind::Type), header("m"), range()).unwrap();
    let sql = result.definition_sql.unwrap();
    assert!(sql.starts_with("CREATE TYPE \"types.with.dot\".\"range_name\" AS RANGE"));
    for clause in [
        "SUBTYPE = \"pg_catalog\".\"int4\"",
        "SUBTYPE_OPCLASS = \"pg_catalog\".\"int4_ops\"",
        "COLLATION = \"collation.schema\".\"C\"",
        "CANONICAL = \"functions\".\"canonical\"",
        "SUBTYPE_DIFF = \"functions\".\"difference\"",
        "MULTIRANGE_TYPE_NAME = \"types.with.dot\".\"name\"\"quoted\"",
    ] {
        assert!(sql.contains(clause), "{clause}");
    }
    assert!(
        matches!(result.facts, PgObjectFacts::Type { class: PgTypeClass::Multirange, subtype: Some(value), .. } if value == "integer")
    );
    let mut stale = range();
    stale.multirange_name = Some("other".into());
    assert_eq!(
        render_range(reference(PgObjectKind::Type), header("m"), stale),
        Err(CatalogError::InvalidResponse)
    );
}
#[test]
fn domain_preserves_null_vs_empty_default_and_ordered_checks() {
    let domain = Domain {
        oid: 1,
        owner: None,
        comment: Some("".into()),
        base_type: "text".into(),
        not_null: true,
        default_value: Some("''::text".into()),
    };
    let checks = vec![
        "CHECK (VALUE <> 'x'::text)".into(),
        "CHECK (length(VALUE) < 10)".into(),
    ];
    let result = render_domain(reference(PgObjectKind::Domain), domain, checks.clone()).unwrap();
    assert_eq!(result.definition_sql.as_deref(), Some("CREATE DOMAIN \"types.with.dot\".\"name\"\"quoted\" AS text DEFAULT ''::text NOT NULL\n  CHECK (VALUE <> 'x'::text)\n  CHECK (length(VALUE) < 10);"));
    assert_eq!(result.comment.as_deref(), Some(""));
    assert!(
        matches!(result.facts, PgObjectFacts::Domain { checks: actual, default_value: Some(value), .. } if actual == checks && value == "''::text")
    );
}
#[test]
fn legacy_range_sql_never_names_post14_multirange_catalog_column() {
    assert!(!range_sql(130_000).contains("rngmultitypid"));
    assert!(range_sql(140_000).contains("rngmultitypid"));
}
