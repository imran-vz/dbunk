//! Generator unit tests: create table.

use super::*;

#[test]
fn create_table_renders_columns_and_table_constraints_in_one_statement() {
    let op = PgObjectOp::CreateTable(CreateTableOp {
        schema: "lifecycle".into(),
        name: "designer_demo".into(),
        columns: vec![
            NewColumnSpec {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
                default: None,
                identity: Some(PgIdentity::Always),
            },
            NewColumnSpec {
                name: "status".into(),
                data_type: "text".into(),
                nullable: false,
                default: Some(PgDefaultValue::Literal {
                    value: "new".into(),
                }),
                identity: None,
            },
            NewColumnSpec {
                name: "created_at".into(),
                data_type: "timestamptz".into(),
                nullable: false,
                default: Some(PgDefaultValue::Expression {
                    sql: "now()".into(),
                }),
                identity: None,
            },
            column("order_id", "integer"),
        ],
        primary_key: Some(PgKeySpec {
            name: Some("designer_demo_pkey".into()),
            columns: vec!["id".into()],
        }),
        uniques: vec![PgKeySpec {
            name: None,
            columns: vec!["status".into(), "order_id".into()],
        }],
        checks: vec![PgCheckSpec {
            name: Some("status_present".into()),
            expression: "status <> ''".into(),
        }],
        foreign_keys: vec![PgForeignKeySpec {
            name: None,
            columns: vec!["order_id".into()],
            referenced_schema: "lifecycle".into(),
            referenced_table: "orders".into(),
            referenced_columns: vec!["id".into()],
            on_update: PgReferentialAction::NoAction,
            on_delete: PgReferentialAction::Cascade,
            deferrable: true,
            initially_deferred: true,
        }],
        unlogged: true,
        if_not_exists: true,
    });
    let preview = generate_object_ddl(&[op]).expect("create table");
    let statement = &preview.statements[0];
    assert_eq!(
        statement.sql,
        "CREATE UNLOGGED TABLE IF NOT EXISTS \"lifecycle\".\"designer_demo\" (\n  \
         \"id\" bigint NOT NULL GENERATED ALWAYS AS IDENTITY,\n  \
         \"status\" text NOT NULL DEFAULT E'new',\n  \
         \"created_at\" timestamptz NOT NULL DEFAULT (now()),\n  \
         \"order_id\" integer,\n  \
         CONSTRAINT \"designer_demo_pkey\" PRIMARY KEY (\"id\"),\n  \
         UNIQUE (\"status\", \"order_id\"),\n  \
         CONSTRAINT \"status_present\" CHECK (status <> ''),\n  \
         FOREIGN KEY (\"order_id\") REFERENCES \"lifecycle\".\"orders\" (\"id\") \
         ON UPDATE NO ACTION ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED\n);"
    );
    assert_eq!(
        statement.summary,
        "Create table lifecycle.designer_demo (4 columns, 4 constraints)"
    );
    assert!(!statement.destructive);
    assert!(statement.transactional);
}

#[test]
fn create_table_validation_names_the_op_index() {
    assert_eq!(
        invalid_reason(&[
            PgObjectOp::CreateSchema(CreateSchemaOp { name: "x".into() }),
            designer_table(vec![])
        ]),
        (1, "table columns cannot be empty".to_string())
    );
    assert!(
        sole_invalid(designer_table(vec![column("a", "int"), column("a", "int")]))
            .contains("declared more than once")
    );
    let undeclared = designer_table_with(
        vec![column("a", "int")],
        Some(PgKeySpec {
            name: None,
            columns: vec!["missing".into()],
        }),
        Vec::new(),
        Vec::new(),
    );
    assert!(sole_invalid(undeclared).contains("undeclared column missing"));

    let mut nullable_identity = column("id", "bigint");
    nullable_identity.identity = Some(PgIdentity::ByDefault);
    assert!(sole_invalid(designer_table(vec![nullable_identity])).contains("must be NOT NULL"));
    let mut defaulted_identity = column("id", "bigint");
    defaulted_identity.nullable = false;
    defaulted_identity.identity = Some(PgIdentity::ByDefault);
    defaulted_identity.default = Some(PgDefaultValue::Literal { value: "1".into() });
    assert!(
        sole_invalid(designer_table(vec![defaulted_identity])).contains("cannot have a default")
    );

    let fk_spec =
        |columns: Vec<&str>, referenced: Vec<&str>, deferrable, initially| PgForeignKeySpec {
            name: None,
            columns: columns.into_iter().map(String::from).collect(),
            referenced_schema: "s".into(),
            referenced_table: "r".into(),
            referenced_columns: referenced.into_iter().map(String::from).collect(),
            on_update: PgReferentialAction::NoAction,
            on_delete: PgReferentialAction::NoAction,
            deferrable,
            initially_deferred: initially,
        };
    let two_columns = || vec![column("a", "int"), column("b", "int")];
    let fk =
        |spec: PgForeignKeySpec| designer_table_with(two_columns(), None, Vec::new(), vec![spec]);
    assert!(
        sole_invalid(fk(fk_spec(vec!["a"], vec!["x", "y"], false, false)))
            .contains("counts must match")
    );
    assert!(sole_invalid(fk(fk_spec(vec!["a"], vec!["x"], false, true)))
        .contains("requires a deferrable"));
    assert!(
        sole_invalid(fk(fk_spec(vec!["zz"], vec!["x"], false, false)))
            .contains("undeclared column zz")
    );
    // A check expression is an embedded fragment like everywhere else.
    let smuggled_check = designer_table_with(
        two_columns(),
        None,
        vec![PgCheckSpec {
            name: None,
            expression: "true); DROP TABLE x; --".into(),
        }],
        Vec::new(),
    );
    assert!(generate_object_ddl(&[smuggled_check]).is_err());
}

#[test]
fn designer_batch_groups_atomically_until_a_concurrent_index() {
    let ops = vec![
        designer_table(vec![column("id", "integer"), column("email", "text")]),
        PgObjectOp::SetComment(SetCommentOp {
            target: PgCommentTarget::Column {
                schema: "lifecycle".into(),
                table: "designer_demo".into(),
                column: "email".into(),
            },
            comment: Some("Login".into()),
        }),
        PgObjectOp::CreateIndex(CreateIndexOp {
            schema: "lifecycle".into(),
            table: "designer_demo".into(),
            name: None,
            unique: false,
            method: "btree".into(),
            columns: vec![PgIndexColumn {
                expression: "email".into(),
                descending: false,
            }],
            include: Vec::new(),
            where_predicate: None,
            concurrently: true,
        }),
    ];
    let preview = generate_object_ddl(&ops).expect("designer batch");
    assert_eq!(
        preview.groups,
        vec![
            StatementGroup::Atomic {
                statement_indexes: vec![0, 1]
            },
            StatementGroup::Standalone { statement_index: 2 },
        ]
    );
}
