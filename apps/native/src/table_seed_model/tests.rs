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
    let spec = draft.spec("value", true, true).unwrap();
    assert!(matches!(spec.source, TableSeedSource::Constant { value } if value.is_empty()));
    assert_eq!(spec.null_rate, Some(0.25));
    draft.mode = ColumnMode::Default;
    draft.null_percent = "invalid".into();
    assert!(matches!(
        draft.spec("value", true, true).unwrap().source,
        TableSeedSource::Default
    ));
    draft.mode = ColumnMode::Auto;
    assert_eq!(draft.spec("value", false, true).unwrap().null_rate, None);
    assert!(!draft.overridden(false, true));
}

#[test]
fn values_are_indexed_recipe_data_and_empty_lists_never_become_auto() {
    let mut draft = ColumnDraft {
        mode: ColumnMode::Values,
        ..Default::default()
    };
    assert!(draft.spec("value", true, true).is_err());
    draft.values_text = " one, 日本語, , one ".into();
    let TableSeedSource::Values { values } = draft.spec("value", true, true).unwrap().source else {
        panic!("values source expected");
    };
    assert_eq!(values, ["one", "日本語", "one"]);
    draft.values_text = std::iter::repeat_n("x", 1025).collect::<Vec<_>>().join(",");
    assert!(draft.spec("value", true, true).is_err());
    draft.values_text = "x".repeat(8193);
    assert!(draft.spec("value", true, true).is_err());
}

#[test]
fn lengths_parse_whole_ranges_and_refuse_inverted_or_out_of_bounds_ones() {
    assert_eq!(parse_length(" ", ""), Ok(None));
    assert_eq!(
        parse_length("10", "50"),
        Ok(Some(TableSeedLength {
            min: Some(10),
            max: Some(50)
        }))
    );
    assert_eq!(
        parse_length("", "5000"),
        Ok(Some(TableSeedLength {
            min: None,
            max: Some(5000)
        }))
    );
    for (min, max) in [
        ("0", ""),
        ("50", "10"),
        ("", "100001"),
        ("1.5", ""),
        ("-3", ""),
    ] {
        assert!(parse_length(min, max).is_err(), "{min}..{max}");
    }
}

#[test]
fn locales_parse_known_ids_once_each() {
    assert_eq!(parse_locales(""), Ok(vec![TableSeedLocale::EnUs]));
    assert_eq!(
        parse_locales(" ja, PT-br ,de"),
        Ok(vec![
            TableSeedLocale::Ja,
            TableSeedLocale::PtBr,
            TableSeedLocale::De
        ])
    );
    assert!(parse_locales("en-AU").is_err());
    assert!(parse_locales("fr, fr").is_err());
    assert_eq!(
        locales_text(&TableSeedLocale::ALL),
        "en-US, en-GB, en-IN, de, fr, es, pt-BR, ja"
    );
    assert_eq!(
        parse_locales(&locales_text(&TableSeedLocale::ALL)),
        Ok(TableSeedLocale::ALL.to_vec())
    );
}

#[test]
fn lengths_reach_the_spec_only_for_generated_text() {
    let mut draft = ColumnDraft {
        min_length: "10".into(),
        max_length: "50".into(),
        ..Default::default()
    };
    assert!(draft.overridden(false, true));
    let spec = draft.spec("email", false, true).unwrap();
    assert_eq!(
        spec.length,
        Some(TableSeedLength {
            min: Some(10),
            max: Some(50)
        })
    );
    draft.mode = ColumnMode::Constant;
    draft.constant = "x".into();
    assert_eq!(draft.spec("email", false, true).unwrap().length, None);
    draft.mode = ColumnMode::Generator(TableSeedGenerator::Integer);
    assert!(!draft.mode.accepts_length(true));
    assert_eq!(draft.spec("email", false, true).unwrap().length, None);
}

#[test]
fn auto_takes_a_length_only_where_the_backend_reports_text() {
    let draft = ColumnDraft {
        min_length: "5".into(),
        max_length: "10".into(),
        ..Default::default()
    };
    assert!(!draft.overridden(false, false));
    assert_eq!(draft.spec("count", false, false).unwrap().length, None);
    assert!(draft.overridden(false, true));
    assert!(ColumnMode::Generator(TableSeedGenerator::Email).accepts_length(false));
}
