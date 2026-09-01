//! Generator unit tests: indexes.

use super::*;

#[test]
fn index_elements_parenthesize_everything_but_columns_and_calls() {
    let preview = generate_object_ddl(&[index_with(&[
        "created_at",
        "lower(email)",
        "public.norm(a, b)",
        "a || b",
        "n::text",
        "price * quantity",
        "data->>'k'",
        "lower(a) || b",
        "(already)",
    ])])
    .expect("index DDL");
    assert!(preview.statements[0].sql.contains(
        "(created_at DESC, lower(email) DESC, public.norm(a, b) DESC, (a || b) DESC, \
         (n::text) DESC, (price * quantity) DESC, (data->>'k') DESC, \
         (lower(a) || b) DESC, ((already)) DESC)"
    ));
}

#[test]
fn renders_constraint_and_index_operations() {
    let ops = vec![
        PgObjectOp::AddPrimaryKey(AddPrimaryKeyOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: Some("orders_pk".into()),
            columns: vec!["id".into()],
        }),
        PgObjectOp::AddUnique(AddUniqueOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: None,
            columns: vec!["external_id".into()],
        }),
        PgObjectOp::AddForeignKey(AddForeignKeyOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: Some("orders_customer_fk".into()),
            columns: vec!["customer_id".into()],
            referenced_schema: "lifecycle".into(),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: PgReferentialAction::Cascade,
            on_delete: PgReferentialAction::SetNull,
            deferrable: true,
            initially_deferred: true,
            not_valid: true,
        }),
        PgObjectOp::AddCheck(AddCheckOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: Some("amount_positive".into()),
            expression: "amount > 0".into(),
            not_valid: false,
        }),
        PgObjectOp::DropConstraint(DropConstraintOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount_positive".into(),
            cascade: false,
        }),
        PgObjectOp::CreateIndex(CreateIndexOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: None,
            unique: false,
            method: "btree".into(),
            columns: vec![PgIndexColumn {
                expression: "created_at".into(),
                descending: true,
            }],
            include: vec!["status".into()],
            where_predicate: Some("status = 'open'".into()),
            concurrently: true,
        }),
        PgObjectOp::DropIndex(DropIndexOp {
            schema: "lifecycle".into(),
            name: "orders_created_at_idx".into(),
            concurrently: true,
            cascade: false,
        }),
    ];
    let preview = generate_object_ddl(&ops).expect("constraint DDL");
    assert!(preview.statements[2]
        .sql
        .contains("ON UPDATE CASCADE ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED NOT VALID"));
    assert!(preview.statements[5]
        .sql
        .starts_with("CREATE INDEX CONCURRENTLY \"orders_created_at_idx\""));
    assert!(!preview.statements[2].destructive);
    assert!(preview.statements[3].destructive);
    assert!(!preview.statements[5].transactional);
    assert!(!preview.statements[6].transactional);
}

#[test]
fn groups_transactional_runs_around_concurrent_indexes() {
    let ops = vec![
        PgObjectOp::CreateSchema(CreateSchemaOp { name: "one".into() }),
        PgObjectOp::CreateSchema(CreateSchemaOp { name: "two".into() }),
        PgObjectOp::CreateIndex(CreateIndexOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: Some("orders_idx".into()),
            unique: false,
            method: "btree".into(),
            columns: vec![PgIndexColumn {
                expression: "id".into(),
                descending: false,
            }],
            include: Vec::new(),
            where_predicate: None,
            concurrently: true,
        }),
        PgObjectOp::CreateSchema(CreateSchemaOp {
            name: "three".into(),
        }),
    ];
    assert_eq!(
        generate_object_ddl(&ops).expect("grouped plan").groups,
        vec![
            StatementGroup::Atomic {
                statement_indexes: vec![0, 1]
            },
            StatementGroup::Standalone { statement_index: 2 },
            StatementGroup::Atomic {
                statement_indexes: vec![3]
            },
        ]
    );
}

#[test]
fn derived_index_names_are_explicit_and_utf8_safe() {
    let columns = vec![PgIndexColumn {
        expression: "customer_id".into(),
        descending: false,
    }];
    assert_eq!(
        derived_index_name("orders", &columns),
        "orders_customer_id_idx"
    );
    let long = derived_index_name(&"é".repeat(40), &columns);
    assert!(long.len() <= 63);
    assert!(long.is_char_boundary(long.len()));

    let too_long = "é".repeat(32);
    assert_eq!(too_long.len(), 64);
    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::CreateIndex(CreateIndexOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: Some(too_long),
            unique: false,
            method: "btree".into(),
            columns,
            include: Vec::new(),
            where_predicate: None,
            concurrently: true,
        })]),
        Err(PgObjectError::InvalidOp { reason, .. })
            if reason == "index name exceeds PostgreSQL's 63-byte limit"
    ));
}
