//! Live: the create-table designer batch.

use super::*;

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn table_designer_live_creates_table_comments_and_concurrent_index() {
    let (_directory, state, connection) = live_state("live-table-designer").await;
    let reset = "DROP TABLE IF EXISTS lifecycle.designer_demo;";
    with_reset(&connection, reset, async {
        let result = apply_live(
            &state,
            connection.id(),
            vec![
                PgObjectOp::CreateTable(CreateTableOp {
                    schema: "lifecycle".into(),
                    name: "designer_demo".into(),
                    columns: vec![
                        NewColumnSpec {
                            identity: Some(PgIdentity::Always),
                            ..column("id", "bigint", false)
                        },
                        column("order_id", "integer", false),
                        NewColumnSpec {
                            default: Some(PgDefaultValue::Literal {
                                value: "it's".into(),
                            }),
                            ..column("label", "text", false)
                        },
                    ],
                    primary_key: Some(PgKeySpec {
                        name: None,
                        columns: vec!["id".into()],
                    }),
                    uniques: vec![PgKeySpec {
                        name: Some("designer_demo_label_key".into()),
                        columns: vec!["label".into()],
                    }],
                    checks: vec![PgCheckSpec {
                        name: None,
                        expression: "label <> ''".into(),
                    }],
                    foreign_keys: vec![PgForeignKeySpec {
                        name: None,
                        columns: vec!["order_id".into()],
                        referenced_schema: "lifecycle".into(),
                        referenced_table: "orders".into(),
                        referenced_columns: vec!["id".into()],
                        on_update: PgReferentialAction::NoAction,
                        on_delete: PgReferentialAction::Cascade,
                        deferrable: false,
                        initially_deferred: false,
                    }],
                    unlogged: false,
                    if_not_exists: false,
                }),
                PgObjectOp::SetComment(SetCommentOp {
                    target: PgCommentTarget::Column {
                        schema: "lifecycle".into(),
                        table: "designer_demo".into(),
                        column: "label".into(),
                    },
                    comment: Some("Designer label".into()),
                }),
                PgObjectOp::CreateIndex(CreateIndexOp {
                    schema: "lifecycle".into(),
                    table: "designer_demo".into(),
                    name: None,
                    unique: false,
                    method: "btree".into(),
                    columns: vec![PgIndexColumn {
                        expression: "order_id".into(),
                        descending: false,
                    }],
                    include: Vec::new(),
                    where_predicate: None,
                    concurrently: true,
                }),
            ],
        )
        .await
        .expect("designer batch");
        assert_eq!(result.applied_statements, 3);

        let table = describe_pg_object(
            &connection,
            lifecycle_ref(PgObjectKind::Table, "designer_demo", None),
        )
        .await
        .expect("describe designed table");
        let definition = table.definition_sql.expect("table DDL");
        assert!(definition.contains("designer_demo"), "{definition}");
        assert!(definition.contains("label"), "{definition}");

        let structure = fetch_table_structure(&connection, "lifecycle", "designer_demo")
            .await
            .expect("designed structure");
        assert_eq!(
            structure.primary_key.as_deref(),
            Some(&["id".to_string()][..])
        );
        assert_eq!(structure.foreign_keys.len(), 1);
        assert!(structure
            .indexes
            .iter()
            .any(|index| index.name == "designer_demo_order_id_idx"));
        assert!(structure.capabilities.triggers && structure.capabilities.privileges);
        // A fresh table has no explicit ACL: the list is empty, not synthesized.
        assert!(structure.privileges.is_empty());
        assert!(structure.triggers.is_empty());
        assert_eq!(
            structure
                .row_security
                .map(|state| (state.enabled, state.forced)),
            Some((false, false))
        );
    })
    .await;
}
