use super::*;
#[tokio::test]
async fn commit_fence_preserves_known_success_and_unknown_loss() {
    let c = Control::default();
    c.cancel();
    let result = commit_result(
        async { panic!("cancelled commit must not execute") },
        &c,
        9,
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.outcome, TableSeedOutcome::RolledBack);
    let c = Control::default();
    let result = commit_result(
        async { Err(TableSeedError::Database.into()) },
        &c,
        9,
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.outcome, TableSeedOutcome::OutcomeUnknown);
    let c = Control::default();
    let result = commit_result(
        async {
            c.cancel();
            Ok(())
        },
        &c,
        9,
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.outcome, TableSeedOutcome::Completed { rows: 9 });
}
#[test]
fn parameter_sql_keeps_server_types_and_text_binding_separate() {
    assert_eq!(insert_sql("INSERT VALUES ",&["integer".into(),"\"odd schema\".\"Type\"".into()],2),"INSERT VALUES ($1::text::integer,$2::text::\"odd schema\".\"Type\"),($3::text::integer,$4::text::\"odd schema\".\"Type\")");
}
#[test]
fn exact_seed_and_recipe_bounds() {
    let e = TableSeedEndpoint {
        connection_id: "test".into(),
        schema: "public".into(),
        table: "items".into(),
    };
    let intent = TableSeedIntent::new(e.clone(), 100, Some(u64::MAX), vec![]).unwrap();
    assert_eq!(intent.seed, Some(u64::MAX));
    assert!(TableSeedIntent::new(
        e.clone(),
        100,
        None,
        vec![TableSeedColumnSpec {
            column: "value".into(),
            source: TableSeedSource::Constant {
                value: "x".repeat(8193)
            },
            null_rate: None,
            length: None
        }]
    )
    .is_err());
    assert!(TableSeedIntent::new(
        e,
        100,
        None,
        vec![TableSeedColumnSpec {
            column: "value".into(),
            source: TableSeedSource::Auto { generator: None },
            null_rate: Some(f64::NAN),
            length: None
        }]
    )
    .is_err());
}

pub(crate) fn plan(intent: TableSeedIntent, target: TableSeedConnection) -> Plan {
    use crate::postgres::transfer::{
        protocol::TargetColumn,
        runner::catalog::{CatalogColumn, RelationState},
    };
    let catalog = catalog::Catalog {
        database: 1,
        relation: RelationState {
            oid: 2,
            kind: "r".into(),
            row_security: false,
            force_row_security: false,
            populated: true,
            columns: vec![CatalogColumn {
                number: 1,
                type_oid: 25,
                type_modifier: -1,
                collation_oid: 0,
                default_fingerprint: None,
                public: TargetColumn {
                    name: "value".into(),
                    data_type: "text".into(),
                    nullable: false,
                    has_default: false,
                    generated: false,
                    identity: false,
                },
            }],
        },
        keys: vec![],
        indexes: vec![],
        serial_columns: vec![],
        enum_labels: vec![],
    };
    let draft = planning::draft(&catalog, &intent).unwrap();
    let columns = planning::columns(&catalog, Some(&draft));
    let (summary, truncated) = planning::summary(&intent);
    let description = TableSeedDescription {
        endpoint: intent.endpoint.clone(),
        connection: target,
        database_oid: 1,
        relation_oid: 2,
        row_count: intent.row_count,
        seed_used: intent.seed.unwrap_or(7),
        clock_epoch_seconds: 1_700_000_000,
        recipe_sha256: "a".repeat(64),
        recipe_summary: summary,
        recipe_summary_truncated: truncated,
        inserted_columns: 1,
        defaulted_columns: 0,
        generator_version: 1,
    };
    Plan {
        seed_used: description.seed_used,
        clock: description.clock_epoch_seconds,
        description: Some(description),
        intent,
        columns,
        issue: None,
        catalog,
    }
}
#[test]
fn batch_budget_accounts_for_literal_expansion_and_parameter_count() {
    use crate::seed::{ColumnPlan, ColumnSource, SeedDialect, SeedPlan};
    let plan = SeedPlan {
        columns: (0..1600)
            .map(|i| ColumnPlan {
                name: format!("c{i}"),
                source: ColumnSource::Constant(Some("x".repeat(8192))),
                null_rate: 0.0,
            })
            .collect(),
        fk_pools: vec![],
        now_epoch_secs: 0,
        dialect: SeedDialect::Postgres,
        options: Default::default(),
    };
    assert_eq!(
        planning::batch_size(&plan, &vec!["text".into(); 1600], 100),
        Err(TableSeedError::Limit)
    );
    let mut plan = plan;
    plan.columns.truncate(1);
    let batch = planning::batch_size(&plan, &["text".into()], 100).unwrap();
    assert!(batch < 500);
    assert!(batch > 0);
}

#[test]
fn database_supplied_fk_columns_are_omitted_before_fk_precedence() {
    let endpoint = TableSeedEndpoint {
        connection_id: "c".into(),
        schema: "s".into(),
        table: "t".into(),
    };
    let target = TableSeedConnection {
        connection_name: "c".into(),
        host: "localhost".into(),
        port: 5432,
        database: "d".into(),
        user: "u".into(),
        environment: "Test".into(),
        safe_mode: "Strict".into(),
        read_only: false,
    };
    let mut p = plan(
        TableSeedIntent::new(endpoint, 10, Some(1), vec![]).unwrap(),
        target,
    );
    p.catalog.keys.push(catalog::Key {
        name: "fk".into(),
        kind: "f".into(),
        columns: vec!["value".into()],
        parent_oid: 3,
        parent_schema: "s".into(),
        parent_table: "parent".into(),
        parent_columns: vec!["id".into()],
        fingerprint: "x".into(),
    });
    for identity in [false, true] {
        p.catalog.relation.columns[0].public.identity = identity;
        p.catalog.relation.columns[0].public.generated = !identity;
        let draft = planning::draft(&p.catalog, &p.intent).unwrap();
        assert!(draft.needed_pools.is_empty());
        assert!(matches!(
            draft.columns()[0].source,
            crate::seed::ColumnSource::Skip
        ));
    }
}

fn endpoint() -> TableSeedEndpoint {
    TableSeedEndpoint {
        connection_id: "c".into(),
        schema: "s".into(),
        table: "t".into(),
    }
}

fn target() -> TableSeedConnection {
    TableSeedConnection {
        connection_name: "c".into(),
        host: "localhost".into(),
        port: 5432,
        database: "d".into(),
        user: "u".into(),
        environment: "Test".into(),
        safe_mode: "Strict".into(),
        read_only: false,
    }
}

fn enum_plan(columns: Vec<TableSeedColumnSpec>) -> Plan {
    let mut p = plan(
        TableSeedIntent::new(endpoint(), 10, Some(1), columns).unwrap(),
        target(),
    );
    let column = &mut p.catalog.relation.columns[0].public;
    column.data_type = "mood".into();
    column.nullable = true;
    p.catalog.enum_labels = vec![("value".into(), vec!["sad".into(), "ok".into()])];
    p
}

#[test]
fn auto_enum_columns_draw_from_their_declared_labels() {
    for columns in [
        vec![],
        vec![TableSeedColumnSpec {
            column: "value".into(),
            source: TableSeedSource::Auto { generator: None },
            null_rate: Some(0.0),
            length: None,
        }],
    ] {
        let p = enum_plan(columns);
        let draft = planning::draft(&p.catalog, &p.intent).unwrap();
        match &draft.columns()[0].source {
            crate::seed::ColumnSource::ValueList(values) => {
                assert_eq!(values, &vec!["sad".to_string(), "ok".to_string()]);
            }
            other => panic!("expected enum labels, got {other:?}"),
        }
        assert!(matches!(
            planning::columns(&p.catalog, Some(&draft))[0].action,
            TableSeedColumnAction::Values
        ));
    }
}

#[test]
fn explicit_enum_column_choices_are_kept() {
    let p = enum_plan(vec![TableSeedColumnSpec {
        column: "value".into(),
        source: TableSeedSource::Constant { value: "ok".into() },
        null_rate: None,
        length: None,
    }]);
    let draft = planning::draft(&p.catalog, &p.intent).unwrap();
    assert!(matches!(
        &draft.columns()[0].source,
        crate::seed::ColumnSource::Constant(Some(value)) if value == "ok"
    ));
}

#[test]
fn unique_fixed_point_runs_must_fit_the_integer_digits() {
    let mut p = plan(
        TableSeedIntent::new(endpoint(), 10, Some(1), vec![]).unwrap(),
        target(),
    );
    p.catalog.relation.columns[0].public.data_type = "numeric(3,2)".into();
    p.catalog.indexes.push(catalog::Index {
        name: "value_key".into(),
        columns: vec!["value".into()],
        primary: false,
        fingerprint: "x".into(),
    });
    let draft = planning::draft(&p.catalog, &p.intent).unwrap();
    // numeric(3,2) holds integer parts 0..=9, and row 10 needs 10.
    assert_eq!(
        planning::capacity(&draft, 10, &p.intent),
        Err(TableSeedError::IntegerRange)
    );
    assert_eq!(planning::capacity(&draft, 9, &p.intent), Ok(()));
}

fn auto(column: &str, length: Option<TableSeedLength>) -> TableSeedColumnSpec {
    TableSeedColumnSpec {
        column: column.into(),
        source: TableSeedSource::Auto { generator: None },
        null_rate: None,
        length,
    }
}

#[test]
fn intents_validate_lengths_and_locales() {
    let length = |min, max| Some(TableSeedLength { min, max });
    for bad in [
        length(None, None),
        length(Some(0), Some(5)),
        length(Some(9), Some(3)),
        length(None, Some(MAX_TABLE_SEED_TEXT_CHARS + 1)),
    ] {
        assert!(TableSeedIntent::new(endpoint(), 1, None, vec![auto("value", bad)]).is_err());
    }
    assert!(TableSeedIntent::new(
        endpoint(),
        1,
        None,
        vec![auto("value", length(Some(3), None))]
    )
    .is_ok());
    let constant_with_length = TableSeedColumnSpec {
        source: TableSeedSource::Constant { value: "x".into() },
        ..auto("value", length(Some(1), Some(2)))
    };
    assert!(TableSeedIntent::new(endpoint(), 1, None, vec![constant_with_length]).is_err());

    let intent = TableSeedIntent::new(endpoint(), 1, None, vec![]).unwrap();
    assert_eq!(intent.locales, vec![TableSeedLocale::EnUs]);
    assert_eq!(intent.realism, TableSeedRealism::FictionalSafe);
    for locales in [vec![], vec![TableSeedLocale::De, TableSeedLocale::De]] {
        assert!(intent
            .clone()
            .with_generation(locales, TableSeedRealism::Realistic)
            .is_err());
    }
}

#[test]
fn impossible_length_ranges_fail_review_as_length_errors() {
    let mut p = plan(
        TableSeedIntent::new(
            endpoint(),
            10,
            Some(1),
            vec![auto(
                "value",
                Some(TableSeedLength {
                    min: Some(50),
                    max: Some(60),
                }),
            )],
        )
        .unwrap(),
        target(),
    );
    p.catalog.relation.columns[0].public.data_type = "character varying(20)".into();
    assert!(matches!(
        planning::draft(&p.catalog, &p.intent),
        Err(TableSeedError::LengthRange)
    ));
}

#[test]
fn generation_options_and_lengths_reach_the_plan_and_summary() {
    let intent = TableSeedIntent::new(
        endpoint(),
        10,
        Some(1),
        vec![auto(
            "value",
            Some(TableSeedLength {
                min: Some(30),
                max: None,
            }),
        )],
    )
    .unwrap()
    .with_generation(
        vec![TableSeedLocale::Ja, TableSeedLocale::PtBr],
        TableSeedRealism::Realistic,
    )
    .unwrap();
    let options = planning::options(&intent);
    assert_eq!(
        options.locales,
        vec![crate::seed::Locale::Ja, crate::seed::Locale::PtBr]
    );
    assert_eq!(options.realism, crate::seed::Realism::Realistic);
    let p = plan(intent, target());
    let draft = planning::draft(&p.catalog, &p.intent).unwrap();
    assert!(matches!(
        draft.columns()[0].source,
        crate::seed::ColumnSource::Generated {
            length: Some(crate::seed::LengthPlan {
                lo: 30,
                uniform: true,
                ..
            }),
            ..
        }
    ));
    let (summary, _) = planning::summary(&p.intent);
    assert!(summary.contains("length 30..auto"), "{summary}");
    assert!(
        summary.contains("Locales ja, pt-BR; realistic"),
        "{summary}"
    );
}

#[test]
fn previews_use_the_inspected_catalog_and_reviews_checks() {
    let p = plan(
        TableSeedIntent::new(endpoint(), 10, Some(1), vec![]).unwrap(),
        target(),
    );
    let ranged = |min, max| {
        TableSeedIntent::new(
            endpoint(),
            10,
            Some(7),
            vec![auto("value", Some(TableSeedLength { min, max }))],
        )
        .unwrap()
        .with_generation(vec![TableSeedLocale::Ja], TableSeedRealism::Realistic)
        .unwrap()
    };
    let preview = p.preview(&ranged(Some(40), Some(60))).expect("preview");
    assert_eq!(preview.seed, 7);
    assert_eq!(preview.columns.len(), 1);
    assert_eq!(preview.columns[0].name, "value");
    assert_eq!(preview.columns[0].cells.len(), 8);
    for cell in &preview.columns[0].cells {
        let TableSeedPreviewCell::Value { chars, .. } = cell else {
            panic!("expected a value, got {cell:?}");
        };
        assert!((40..=60).contains(chars), "{chars}");
    }
    let mut narrow = p;
    narrow.catalog.relation.columns[0].public.data_type = "character varying(20)".into();
    assert_eq!(
        narrow.preview(&ranged(Some(40), Some(60))),
        Err(TableSeedError::LengthRange)
    );
    let elsewhere = TableSeedIntent::new(
        TableSeedEndpoint {
            table: "other".into(),
            ..endpoint()
        },
        10,
        None,
        vec![],
    )
    .unwrap();
    assert_eq!(narrow.preview(&elsewhere), Err(TableSeedError::StaleReview));
}

#[test]
fn previewed_values_are_cut_for_display_but_keep_their_length() {
    let p = plan(
        TableSeedIntent::new(endpoint(), 10, Some(1), vec![]).unwrap(),
        target(),
    );
    let intent = TableSeedIntent::new(
        endpoint(),
        10,
        Some(1),
        vec![auto(
            "value",
            Some(TableSeedLength {
                min: Some(4000),
                max: Some(5000),
            }),
        )],
    )
    .unwrap();
    for cell in &p.preview(&intent).unwrap().columns[0].cells {
        let TableSeedPreviewCell::Value { text, chars } = cell else {
            panic!("expected a value");
        };
        assert_eq!(text.chars().count(), TABLE_SEED_PREVIEW_CHARS);
        assert!((4000..=5000).contains(chars));
    }
}

#[test]
fn columns_report_whether_auto_takes_a_length() {
    let mut p = plan(
        TableSeedIntent::new(endpoint(), 10, Some(1), vec![]).unwrap(),
        target(),
    );
    let auto_length = |p: &Plan| planning::columns(&p.catalog, None)[0].auto_length;
    assert!(auto_length(&p), "text takes a length");
    p.catalog.relation.columns[0].public.data_type = "integer".into();
    assert!(!auto_length(&p), "integers do not");
    p.catalog.relation.columns[0].public.data_type = "mood".into();
    p.catalog.enum_labels = vec![("value".into(), vec!["ok".into()])];
    assert!(!auto_length(&p), "enums draw their labels");
}

#[test]
fn generators_report_whether_they_take_a_length() {
    use TableSeedGenerator as G;
    for generator in [
        G::Email,
        G::Phone,
        G::FullAddress,
        G::Paragraph,
        G::CountryCode,
    ] {
        assert!(generator.accepts_length(), "{generator:?}");
    }
    for generator in [G::Integer, G::Uuid, G::Timestamp, G::Json, G::Boolean] {
        assert!(!generator.accepts_length(), "{generator:?}");
    }
}

#[test]
fn previews_refuse_a_recipe_that_inserts_nothing_like_prepare() {
    let p = plan(
        TableSeedIntent::new(endpoint(), 10, Some(1), vec![]).unwrap(),
        target(),
    );
    let all_default = TableSeedIntent::new(
        endpoint(),
        10,
        Some(1),
        vec![TableSeedColumnSpec {
            column: "value".into(),
            source: TableSeedSource::Default,
            null_rate: None,
            length: None,
        }],
    )
    .unwrap();
    assert_eq!(p.preview(&all_default), Err(TableSeedError::InvalidRecipe));
    let unseeded = TableSeedIntent::new(endpoint(), 10, None, vec![]).unwrap();
    let preview = p.preview(&unseeded).unwrap();
    assert!(!preview.seed_chosen);
}
