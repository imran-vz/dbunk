use super::*;
fn builder() -> Builder {
    Builder::new(
        "Odd.Schema".into(),
        "quoted\"table".into(),
        42,
        CompletionRelationKind::Table,
    )
    .unwrap()
}
fn column(ordinal_position: i32) -> CompletionColumn {
    CompletionColumn {
        name: format!("c{ordinal_position}"),
        data_type: "numeric(20,-2)".into(),
        ordinal_position,
        is_primary_key: false,
    }
}
#[test]
fn names_are_exact_and_bounded_in_utf8_bytes() {
    assert!(validate("Odd.Schema", "quoted\"table").is_ok());
    assert!(validate(&"雪".repeat(21), "x").is_ok());
    assert!(matches!(
        validate(&"雪".repeat(22), "x"),
        Err(CatalogError::CompletionLimit)
    ));
    assert!(matches!(
        validate("", "x"),
        Err(CatalogError::InvalidReference)
    ));
    assert!(matches!(
        validate("x", "a\0b"),
        Err(CatalogError::InvalidReference)
    ));
    let result = builder().finish().unwrap();
    assert_eq!(result.schema, "Odd.Schema");
    assert_eq!(result.relation, "quoted\"table");
    assert_eq!(result.relation_oid, 42);
    assert!(result.columns.is_empty());
}
#[test]
fn kinds_cover_relations_and_refuse_sequence_index_and_composite_type() {
    for (input, expected) in [
        ("r", CompletionRelationKind::Table),
        ("p", CompletionRelationKind::PartitionedTable),
        ("v", CompletionRelationKind::View),
        ("m", CompletionRelationKind::MaterializedView),
        ("f", CompletionRelationKind::ForeignTable),
    ] {
        assert_eq!(CompletionRelationKind::decode(input).unwrap(), expected);
    }
    for input in ["S", "i", "c", "", "unknown"] {
        assert_eq!(
            CompletionRelationKind::decode(input),
            Err(CatalogError::UnsupportedObjectKind)
        );
    }
}
#[test]
fn dropped_attribute_gaps_and_exact_type_and_pk_metadata_are_preserved() {
    let mut result = builder();
    let mut first = column(1);
    first.name = "雪 \"quoted\"".into();
    first.is_primary_key = true;
    let mut second = column(4);
    second.data_type = "\"Other.Schema\".\"domain[]\"[]".into();
    result.push(first.clone()).unwrap();
    result.push(second.clone()).unwrap();
    assert_eq!(result.finish().unwrap().columns, [first, second]);
    let mut invalid = builder();
    invalid.push(column(3)).unwrap();
    assert!(matches!(
        invalid.push(column(2)),
        Err(CatalogError::InvalidResponse)
    ));
    assert_eq!(invalid.result.columns.len(), 1);
    assert!(matches!(
        builder().push(column(0)),
        Err(CatalogError::InvalidResponse)
    ));
}
#[test]
fn exactly_column_limit_succeeds_and_extra_row_refuses_whole_result() {
    let mut result = builder();
    for ordinal in 1..=MAX_COMPLETION_COLUMNS {
        result.push(column(ordinal as i32)).unwrap();
    }
    assert!(encoded(&result.result).unwrap() <= MAX_COMPLETION_BYTES);
    assert!(matches!(
        result.push(column(MAX_COMPLETION_COLUMNS as i32 + 1)),
        Err(CatalogError::CompletionLimit)
    ));
    assert_eq!(result.result.columns.len(), MAX_COMPLETION_COLUMNS);
}
#[test]
fn type_and_encoded_escape_expansion_refuse_before_retention() {
    let mut result = builder();
    let mut first = column(1);
    first.data_type = "t".repeat(MAX_COMPLETION_TYPE_BYTES + 1);
    assert!(matches!(
        result.push(first),
        Err(CatalogError::CompletionLimit)
    ));
    assert!(result.result.columns.is_empty());
    let mut escaped = column(1);
    escaped.data_type = "\u{0001}".repeat(MAX_COMPLETION_TYPE_BYTES);
    let bytes = encoded(&escaped).unwrap() + 1;
    assert!(bytes >= 6 * MAX_COMPLETION_TYPE_BYTES);
    result.bytes = MAX_COMPLETION_BYTES - bytes + 1;
    assert!(matches!(
        result.push(escaped),
        Err(CatalogError::CompletionLimit)
    ));
    assert!(result.result.columns.is_empty());
}
#[test]
fn aggregate_byte_limit_is_independent_of_column_count() {
    let mut result = builder();
    let mut refused = false;
    for ordinal in 1..=MAX_COMPLETION_COLUMNS {
        let mut item = column(ordinal as i32);
        item.data_type = "t".repeat(MAX_COMPLETION_TYPE_BYTES);
        match result.push(item) {
            Ok(()) => {}
            Err(CatalogError::CompletionLimit) => {
                refused = true;
                break;
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
    assert!(refused);
    assert!(result.result.columns.len() < MAX_COMPLETION_COLUMNS);
}
