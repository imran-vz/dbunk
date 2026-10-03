use super::*;

#[test]
fn seed_preserves_every_unsigned_integer_bit() {
    assert_eq!(parse_seed("18446744073709551615"), Ok(Some(u64::MAX)));
    assert_eq!(
        parse_seed("9007199254740993"),
        Ok(Some(9_007_199_254_740_993))
    );
    assert_eq!(parse_seed("  00042  "), Ok(Some(42)));
    assert_eq!(parse_seed("0"), Ok(Some(0)));
    assert_eq!(parse_seed("  "), Ok(None));
    for invalid in ["18446744073709551616", "-1", "+1", "1e2", "1.0", "١"] {
        assert!(parse_seed(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn row_and_null_admission_refuses_invalid_input_without_clamping() {
    assert_eq!(parse_row_count("1000000"), Ok(1_000_000));
    for invalid in ["", "0", "1000001", "1.5", "-1"] {
        assert!(parse_row_count(invalid).is_err());
    }
    assert_eq!(parse_null_rate(""), Ok(None));
    assert_eq!(parse_null_rate("12.5"), Ok(Some(0.125)));
    assert_eq!(parse_null_rate("100"), Ok(Some(1.0)));
    for invalid in ["NaN", "inf", "-1", "100.01"] {
        assert!(parse_null_rate(invalid).is_err());
    }
}

#[test]
fn recipe_preserves_empty_constants_and_ignores_inapplicable_null_settings() {
    let mut draft = ColumnDraft {
        mode: ColumnMode::Constant,
        constant: String::new(),
        null_percent: "25".into(),
        ..Default::default()
    };
    let spec = draft.spec("value", true).unwrap();
    assert!(matches!(spec.source, TableSeedSource::Constant { value } if value.is_empty()));
    assert_eq!(spec.null_rate, Some(0.25));
    draft.mode = ColumnMode::Default;
    draft.null_percent = "invalid".into();
    assert!(matches!(
        draft.spec("value", true).unwrap().source,
        TableSeedSource::Default
    ));
    draft.mode = ColumnMode::Auto;
    assert_eq!(draft.spec("value", false).unwrap().null_rate, None);
    assert!(!draft.overridden(false));
}

#[test]
fn values_are_indexed_recipe_data_and_empty_lists_never_become_auto() {
    let mut draft = ColumnDraft {
        mode: ColumnMode::Values,
        ..Default::default()
    };
    assert!(draft.spec("value", true).is_err());
    draft.values_text = " one, 日本語, , one ".into();
    let TableSeedSource::Values { values } = draft.spec("value", true).unwrap().source else {
        panic!("values source expected");
    };
    assert_eq!(values, ["one", "日本語", "one"]);
    draft.values_text = std::iter::repeat_n("x", 1025).collect::<Vec<_>>().join(",");
    assert!(draft.spec("value", true).is_err());
    draft.values_text = "x".repeat(8193);
    assert!(draft.spec("value", true).is_err());
}
