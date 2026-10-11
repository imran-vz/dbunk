use super::*;
#[test]
fn setup_admission_includes_replacement_overlap_and_releases_exactly() {
    let budget = Rc::new(Cell::new(126 * MIB));
    assert!(Lease::new(budget.clone(), SETUP_BYTES).is_none());
    assert_eq!(budget.get(), 126 * MIB);
    budget.set(0);
    let first = Lease::new(budget.clone(), SETUP_BYTES).unwrap();
    let second = Lease::new(budget.clone(), SETUP_BYTES).unwrap();
    assert_eq!(budget.get(), 2 * SETUP_BYTES);
    drop(first);
    assert_eq!(budget.get(), SETUP_BYTES);
    drop(second);
    assert_eq!(budget.get(), 0);
}
#[test]
fn all_generator_choices_have_stable_exact_mapping() {
    assert_eq!(recipe::mode_label(recipe::mode_at(0).unwrap()), "Auto");
    assert_eq!(
        recipe::mode_label(recipe::mode_at(1).unwrap()),
        "Skip (use DEFAULT)"
    );
    for (index, (generator, label)) in GENERATORS.iter().enumerate() {
        assert!(recipe::mode_at(index + 4) == Some(ColumnMode::Generator(*generator)));
        assert_eq!(
            recipe::mode_label(recipe::mode_at(index + 4).unwrap()),
            *label
        );
    }
    assert!(recipe::mode_at(recipe::MODE_COUNT).is_none());
    assert!(recipe::mode_at(recipe::MODE_COUNT - 1).is_some());
}
#[test]
fn column_navigation_never_wraps_or_invents_a_selection() {
    assert_eq!(keyboard::move_index(None, 0, "down"), None);
    assert_eq!(keyboard::move_index(None, 1600, "up"), Some(1599));
    assert_eq!(keyboard::move_index(Some(1599), 1600, "down"), Some(1599));
    assert_eq!(keyboard::move_index(Some(0), 1600, "up"), Some(0));
}
#[test]
fn unchanged_auto_columns_do_not_inflate_spec_capacity() {
    let mut recipe = recipe_with_columns(1600);
    assert_eq!(recipe.specs().unwrap().capacity(), 0);
    recipe.drafts[1599] = ColumnDraft {
        mode: ColumnMode::Constant,
        constant: "9223372036854775807".into(),
        ..Default::default()
    };
    let specs = recipe.specs().unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs.capacity(), 1);
    assert_eq!(specs[0].column, "column1599");
    assert!(
        matches!(&specs[0].source,TableSeedSource::Constant{value} if value=="9223372036854775807")
    );
}
#[test]
fn recipe_size_refusal_preserves_prior_column() {
    let mut recipe = recipe_with_columns(1);
    let oversized = ColumnDraft {
        mode: ColumnMode::Values,
        values_text: "a".repeat(513 * 1024),
        ..Default::default()
    };
    assert!(recipe.save(0, oversized).is_err());
    assert!(recipe.drafts[0].mode == ColumnMode::Auto);
    assert!(!recipe.changed);
    let empty = ColumnDraft {
        mode: ColumnMode::Values,
        values_text: " , ".into(),
        ..Default::default()
    };
    assert!(recipe.save(0, empty).is_err());
    assert!(recipe.drafts[0].mode == ColumnMode::Auto);
}
fn recipe_with_columns(count: usize) -> Recipe {
    let endpoint = TableSeedEndpoint {
        connection_id: "test".into(),
        schema: "public".into(),
        table: "rows".into(),
    };
    let intent = TableSeedIntent::new(endpoint, 100, None, vec![]).unwrap();
    Recipe {
        setup: recipe::SetupAnchor::of(TableSeedAttemptId::new(), &intent),
        columns: (0..count)
            .map(|index| recipe::Column {
                name: format!("column{index}"),
                data_type: "text".into(),
                nullable: true,
                generated: false,
                identity: false,
                has_default: false,
                auto_length: true,
            })
            .collect(),
        drafts: vec![ColumnDraft::default(); count],
        changed: false,
        discarded: false,
    }
}

#[test]
fn setup_anchor_compares_exact_endpoint_count_and_optional_u64_seed() {
    let recipe = recipe_with_columns(0);
    let mut intent =
        TableSeedIntent::new(recipe.setup.endpoint, 100, Some(u64::MAX), vec![]).unwrap();
    let anchor = recipe::SetupAnchor::of(recipe.setup.attempt, &intent);
    assert!(anchor.matches(&intent));
    intent.seed = Some(u64::MAX - 1);
    assert!(!anchor.matches(&intent));
    intent.seed = Some(u64::MAX);
    intent.row_count = 101;
    assert!(!anchor.matches(&intent));
    intent.row_count = 100;
    intent.endpoint.table = "other".into();
    assert!(!anchor.matches(&intent));
}

#[test]
fn locales_and_realism_are_part_of_the_reviewed_setup() {
    let recipe = recipe_with_columns(0);
    let intent = TableSeedIntent::new(recipe.setup.endpoint, 100, None, vec![]).unwrap();
    let anchor = recipe::SetupAnchor::of(recipe.setup.attempt, &intent);
    let realistic = intent
        .clone()
        .with_generation(vec![TableSeedLocale::EnUs], TableSeedRealism::Realistic)
        .unwrap();
    assert!(!anchor.matches(&realistic));
    let japanese = intent
        .with_generation(vec![TableSeedLocale::Ja], TableSeedRealism::FictionalSafe)
        .unwrap();
    assert!(!anchor.matches(&japanese));
}

#[test]
fn saving_a_length_range_marks_the_recipe_changed() {
    let mut recipe = recipe_with_columns(1);
    recipe
        .save(
            0,
            ColumnDraft {
                min_length: "10".into(),
                max_length: "50".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(recipe.changed);
    let specs = recipe.specs().unwrap();
    assert_eq!(
        specs[0].length,
        Some(TableSeedLength {
            min: Some(10),
            max: Some(50)
        })
    );
    let inverted = ColumnDraft {
        min_length: "50".into(),
        max_length: "10".into(),
        ..Default::default()
    };
    assert!(recipe.save(0, inverted).is_err());
}

#[test]
fn saving_an_unchanged_column_keeps_review_clean_but_a_saved_edit_invalidates_it() {
    let mut recipe = recipe_with_columns(1);
    recipe.save(0, ColumnDraft::default()).unwrap();
    assert!(!recipe.changed);
    recipe
        .save(
            0,
            ColumnDraft {
                mode: ColumnMode::Constant,
                constant: "NULL".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(recipe.changed);
    assert!(
        matches!(&recipe.specs().unwrap()[0].source,TableSeedSource::Constant{value} if value=="NULL")
    );
}

#[test]
fn tiny_reviewed_null_rates_fit_the_editor_without_being_dropped() {
    for rate in [0., 1., 0.125, 1e-320, f64::from_bits(1)] {
        let text = recipe::percent_text(rate);
        assert!(text.len() <= 32);
        let parsed = crate::table_seed_model::parse_null_rate(&text)
            .unwrap()
            .unwrap();
        assert_eq!(parsed, rate);
    }
}

#[test]
fn preview_text_shows_values_lengths_and_non_values() {
    let value = |text: &str, chars| TableSeedPreviewCell::Value {
        text: text.into(),
        chars,
    };
    let preview = TableSeedPreview {
        seed: 7,
        seed_chosen: true,
        columns: vec![
            TableSeedPreviewColumn {
                name: "email".into(),
                cells: vec![
                    value("a@b.test", 8),
                    value("long", 300),
                    TableSeedPreviewCell::Null,
                ],
            },
            TableSeedPreviewColumn {
                name: "id".into(),
                cells: vec![TableSeedPreviewCell::Default],
            },
            TableSeedPreviewColumn {
                name: "customer_id".into(),
                cells: vec![TableSeedPreviewCell::FromParent],
            },
        ],
    };
    let text = recipe::preview_text(&preview);
    assert!(text.starts_with("Sample values (seed 7;"), "{text}");
    assert!(
        text.contains("email [8–300 chars]: a@b.test | long… | NULL"),
        "{text}"
    );
    assert!(text.contains("id: DEFAULT"), "{text}");
    assert!(text.contains("customer_id: (parent row)"), "{text}");
}

#[test]
fn preview_text_is_bounded() {
    let preview = TableSeedPreview {
        seed: 1,
        seed_chosen: false,
        columns: (0..1600)
            .map(|index| TableSeedPreviewColumn {
                name: format!("column{index}"),
                cells: vec![
                    TableSeedPreviewCell::Value {
                        text: "x".repeat(TABLE_SEED_PREVIEW_CHARS),
                        chars: 5000,
                    };
                    8
                ],
            })
            .collect(),
    };
    let text = recipe::preview_text(&preview);
    assert!(text.len() <= 64 * 1024 + 64);
    assert!(
        text.ends_with("more columns"),
        "{}",
        &text[text.len() - 40..]
    );
}

#[test]
fn unchosen_seeds_are_called_illustrative() {
    let preview = TableSeedPreview {
        seed: 1,
        seed_chosen: false,
        columns: vec![],
    };
    let text = recipe::preview_text(&preview);
    assert!(text.contains("illustrative seed"), "{text}");
    assert!(!text.contains("seed 1"), "{text}");
}
