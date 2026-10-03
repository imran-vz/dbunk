//! Generator unit tests: enum types.

use super::*;

#[test]
fn add_enum_value_runs_outside_atomic_groups() {
    let ops = vec![
        PgObjectOp::AddEnumValue(AddEnumValueOp {
            schema: "s".into(),
            name: "order_status".into(),
            value: "queued".into(),
            position: None,
        }),
        PgObjectOp::SetColumnDefault(SetColumnDefaultOp {
            schema: "s".into(),
            table: "orders".into(),
            name: "status".into(),
            default: Some(PgDefaultValue::Literal {
                value: "queued".into(),
            }),
        }),
    ];
    let preview = generate_object_ddl(&ops).expect("enum plan");
    assert!(!preview.statements[0].transactional);
    assert_eq!(
        preview.groups,
        vec![
            StatementGroup::Standalone { statement_index: 0 },
            StatementGroup::Atomic {
                statement_indexes: vec![1]
            },
        ]
    );
}

#[test]
fn renders_views_sequences_and_enums() {
    let ops = vec![
        PgObjectOp::CreateView(CreateViewOp {
            schema: "lifecycle".into(),
            name: "orders_view".into(),
            or_replace: true,
            sql_body: "SELECT * FROM lifecycle.orders;".into(),
        }),
        PgObjectOp::CreateMaterializedView(CreateMaterializedViewOp {
            schema: "lifecycle".into(),
            name: "orders_mat".into(),
            sql_body: "SELECT 'WITH  DATA' AS marker FROM lifecycle.orders_view".into(),
            with_data: false,
        }),
        PgObjectOp::CreateSequence(CreateSequenceOp {
            schema: "lifecycle".into(),
            name: "order_seq".into(),
            data_type: Some("bigint".into()),
            start: Some("10".into()),
            increment: Some("5".into()),
            min_value: Some("10".into()),
            max_value: Some("10000".into()),
            cycle: Some(false),
            cache: Some("20".into()),
        }),
        PgObjectOp::AlterSequence(AlterSequenceOp {
            schema: "lifecycle".into(),
            name: "order_seq".into(),
            restart_with: Some("50".into()),
            increment_by: None,
            min_value: None,
            max_value: None,
            cycle: Some(true),
            cache: None,
        }),
        PgObjectOp::CreateEnum(CreateEnumOp {
            schema: "lifecycle".into(),
            name: "order_status".into(),
            labels: vec!["new".into(), "it's done".into()],
        }),
        PgObjectOp::AddEnumValue(AddEnumValueOp {
            schema: "lifecycle".into(),
            name: "order_status".into(),
            value: "queued".into(),
            position: Some(PgEnumPosition::Before {
                neighbor: "done".into(),
            }),
        }),
        PgObjectOp::RenameEnumValue(RenameEnumValueOp {
            schema: "lifecycle".into(),
            name: "order_status".into(),
            from: "new".into(),
            to: "fresh".into(),
        }),
    ];
    let preview = generate_object_ddl(&ops).expect("lifecycle DDL");
    assert_eq!(
        preview.statements[0].sql,
        "CREATE OR REPLACE VIEW \"lifecycle\".\"orders_view\" AS SELECT * FROM lifecycle.orders;"
    );
    assert!(preview.statements[1].sql.ends_with("WITH NO DATA;"));
    assert!(preview.statements[1].sql.contains("'WITH  DATA'"));
    assert!(preview.statements[2]
        .sql
        .contains("START WITH 10 CACHE 20 NO CYCLE"));
    assert!(preview.statements[4].sql.contains("E'it''s done'"));
    assert!(preview.statements[5].sql.contains("BEFORE E'done'"));
}
