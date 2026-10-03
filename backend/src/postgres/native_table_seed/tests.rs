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
            null_rate: None
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
            null_rate: Some(f64::NAN)
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
