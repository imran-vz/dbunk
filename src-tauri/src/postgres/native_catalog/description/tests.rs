use super::*;
use serde_json::json;

fn reference(kind: PgObjectKind) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: Some("Quoted.Schema".into()),
        name: "name\"with quote".into(),
        identity_args: None,
    }
}
fn payload(facts: serde_json::Value) -> serde_json::Value {
    json!({"owner":"owner", "comment":null, "definitionSql":null, "facts":facts, "sequenceOwner":null})
}

#[test]
fn overload_identity_is_required_exact_and_empty_zero_arg_identity_is_valid() {
    for kind in [
        PgObjectKind::Function,
        PgObjectKind::Procedure,
        PgObjectKind::Aggregate,
    ] {
        let mut reference = reference(kind);
        assert_eq!(validate(&reference), Err(CatalogError::InvalidReference));
        reference.identity_args = Some("".into());
        validate(&reference).unwrap();
        reference.identity_args =
            Some("\"quoted arg\" numeric(30, 10), VARIADIC \"Mixed\".\"Type\"[]".into());
        let expected = reference.clone();
        let mut body = payload(
            json!({"kind":"routine", "language":"sql", "returns":"numeric", "volatility":"stable", "arguments":"exact arguments", "body":"SELECT 90071992547409931234567890.000001", "strict":false, "securityDefiner":true, "parallel":"safe"}),
        );
        body["definitionSql"] = json!("CREATE FUNCTION exact returned text;");
        if kind == PgObjectKind::Aggregate {
            body["definitionSql"] = json!(null);
            body["facts"]["body"] = json!(null);
        }
        let result = decode(reference, &body.to_string()).unwrap();
        assert_eq!(result.reference, expected);
        if let PgObjectFacts::Routine { body, .. } = result.facts {
            if kind != PgObjectKind::Aggregate {
                assert_eq!(
                    body.as_deref(),
                    Some("SELECT 90071992547409931234567890.000001")
                );
            }
        } else {
            panic!("wrong fact kind");
        }
    }
}

#[test]
fn sequence_extremes_and_ownership_identifiers_never_round_or_split_on_dots() {
    let mut body = payload(
        json!({"kind":"sequence", "dataType":"bigint", "start":"9223372036854775807", "increment":"-1", "minValue":"-9223372036854775808", "maxValue":"9223372036854775807", "cycle":true, "cache":"1", "lastValue":null, "ownedBy":null}),
    );
    body["sequenceOwner"] = json!(["schema.with.dot", "table\"quoted", "column.with.dot"]);
    let result = decode(reference(PgObjectKind::Sequence), &body.to_string()).unwrap();
    let definition = result.definition_sql.unwrap();
    assert!(definition.contains(
        "MINVALUE -9223372036854775808 MAXVALUE 9223372036854775807 START WITH 9223372036854775807"
    ));
    assert!(definition
        .ends_with("OWNED BY \"schema.with.dot\".\"table\"\"quoted\".\"column.with.dot\";"));
    assert!(matches!(
        result.facts,
        PgObjectFacts::Sequence {
            last_value: None,
            ..
        }
    ));
}

#[test]
fn view_and_materialized_view_reconstruct_one_terminator_and_preserve_populated_state() {
    let result = decode(
        reference(PgObjectKind::View),
        &payload(json!({"kind":"view", "definition":" SELECT '日本語;😀'::text;\n "})).to_string(),
    )
    .unwrap();
    assert_eq!(
        result.definition_sql.as_deref(),
        Some("CREATE VIEW \"Quoted.Schema\".\"name\"\"with quote\" AS\nSELECT '日本語;😀'::text;")
    );
    let result = decode(
        reference(PgObjectKind::MaterializedView),
        &payload(json!({"kind":"materializedView", "definition":"SELECT 1;", "populated":false}))
            .to_string(),
    )
    .unwrap();
    assert!(result
        .definition_sql
        .unwrap()
        .ends_with("SELECT 1 WITH NO DATA;"));
    assert!(matches!(
        result.facts,
        PgObjectFacts::MaterializedView {
            populated: false,
            ..
        }
    ));
}

#[test]
fn schema_and_extension_quote_typed_identity_and_refuse_stale_extension_schema() {
    let mut schema = reference(PgObjectKind::Schema);
    schema.schema = None;
    let result = decode(schema, &payload(json!({"kind":"schema"})).to_string()).unwrap();
    assert_eq!(
        result.definition_sql.as_deref(),
        Some("CREATE SCHEMA \"name\"\"with quote\";")
    );
    let result = decode(
        reference(PgObjectKind::Extension),
        &payload(json!({"kind":"extension", "schema":"Quoted.Schema", "version":"1'2"}))
            .to_string(),
    )
    .unwrap();
    assert!(result.definition_sql.unwrap().ends_with("VERSION E'1''2';"));
    assert_eq!(
        decode(
            reference(PgObjectKind::Extension),
            &payload(json!({"kind":"extension", "schema":"moved", "version":"1"})).to_string()
        ),
        Err(CatalogError::ObjectNotFound)
    );
}

#[test]
fn every_kind_accepts_typed_identity_but_unsafe_or_oversized_reference_is_refused() {
    for kind in [
        PgObjectKind::Table,
        PgObjectKind::ForeignTable,
        PgObjectKind::Type,
        PgObjectKind::Domain,
    ] {
        validate(&reference(kind)).unwrap();
    }
    for name in [
        "bad\0name".into(),
        "x".repeat(MAX_TEXT_BYTES + 1),
        "".into(),
    ] {
        let mut reference = reference(PgObjectKind::View);
        reference.name = name;
        assert_eq!(validate(&reference), Err(CatalogError::InvalidReference));
    }
    let mut reference = reference(PgObjectKind::View);
    reference.identity_args = Some("".into());
    assert_eq!(validate(&reference), Err(CatalogError::InvalidReference));
}

#[test]
fn quoted_whitespace_identifiers_preserve_exact_schema_and_name() {
    let mut reference = reference(PgObjectKind::View);
    reference.schema = Some(" ".into());
    reference.name = "\t ".into();
    validate(&reference).unwrap();
    reference.schema = Some(String::new());
    assert_eq!(validate(&reference), Err(CatalogError::InvalidReference));
}

#[test]
fn descriptions_refuse_large_text_and_expanded_output_without_partial_values() {
    let body =
        payload(json!({"kind":"view", "definition":"x".repeat(MAX_DESCRIPTION_TEXT_BYTES + 1)}));
    assert_eq!(
        decode(reference(PgObjectKind::View), &body.to_string()),
        Err(CatalogError::DescriptionLimit)
    );
    let mut body = payload(json!({"kind":"schema"}));
    body["comment"] = json!("é".repeat(MAX_TEXT_BYTES / 2 + 1));
    let mut schema = reference(PgObjectKind::Schema);
    schema.schema = None;
    assert_eq!(
        decode(schema, &body.to_string()),
        Err(CatalogError::DescriptionLimit)
    );
    // One bounded source definition fits the wire JSON, but reconstructing both
    // facts and SQL doubles JSON escapes. Refuse the overall result atomically.
    let body =
        payload(json!({"kind":"view", "definition":"\0".repeat(MAX_DESCRIPTION_TEXT_BYTES)}));
    assert!(body.to_string().len() < MAX_DESCRIPTION_BYTES);
    assert_eq!(
        decode(reference(PgObjectKind::View), &body.to_string()),
        Err(CatalogError::DescriptionLimit)
    );
}

#[test]
fn incompatible_facts_cannot_be_published_under_another_kind() {
    assert_eq!(
        decode(
            reference(PgObjectKind::View),
            &payload(json!({"kind":"schema"})).to_string()
        ),
        Err(CatalogError::InvalidResponse)
    );
}
