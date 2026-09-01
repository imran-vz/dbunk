//! Generator unit tests: fragment validation.

use super::*;

#[test]
fn fragment_shape_uses_the_shared_lexer_for_dollar_identifiers() {
    // `d$x$` is one identifier to PostgreSQL and to lex_sql; a byte scanner
    // that treated `$x$` as a dollar-quote opener would hide the comma.
    let smuggled = add_column("d$x$ , ADD COLUMN \"evil\" text /*$x$*/", None);
    let error = generate_object_ddl(&[smuggled]).expect_err("smuggled action");
    assert!(matches!(error, PgObjectError::InvalidOp { .. }));

    let using = PgObjectOp::AlterColumnType(AlterColumnTypeOp {
        schema: "s".into(),
        table: "t".into(),
        name: "c".into(),
        new_type: "text".into(),
        using: Some("y$q$::text , DROP COLUMN \"secret\" /*$q$*/".into()),
    });
    assert!(generate_object_ddl(&[using]).is_err());
    assert!(generate_object_ddl(&[index_with(&["a$b$ , x$b$"])]).is_err());

    // Real dollar quotes and bracketed lists remain valid fragments.
    let array_default = add_column(
        "integer[]",
        Some(PgDefaultValue::Expression {
            sql: "ARRAY[1, 2]".into(),
        }),
    );
    assert!(generate_object_ddl(&[array_default]).is_ok());
    assert!(generate_object_ddl(&[add_check("name <> $tag$a, b$tag$")]).is_ok());
}

#[test]
fn plain_string_backslashes_are_valid_fragments() {
    let regex = add_check(r"email ~ '^[^@]+@[^@]+\.[a-z]{2,}$'");
    let preview = generate_object_ddl(&[regex]).expect("regex check");
    assert!(preview.statements[0]
        .sql
        .contains(r"'^[^@]+@[^@]+\.[a-z]{2,}$'"));
    let view = PgObjectOp::CreateView(CreateViewOp {
        schema: "s".into(),
        name: "paths".into(),
        or_replace: false,
        sql_body: r"SELECT 'C:\temp' AS path".into(),
    });
    assert!(generate_object_ddl(&[view]).is_ok());
    // A literal whose end depends on standard_conforming_strings is still
    // refused, because the boundary could move on the server.
    assert!(generate_object_ddl(&[add_check(r"name = 'a\'' OR true")]).is_err());
}

#[test]
fn rejects_every_fragment_boundary_and_invalid_shape() {
    let boundary_ops = vec![
        PgObjectOp::CreateView(CreateViewOp {
            schema: "lifecycle".into(),
            name: "bad".into(),
            or_replace: false,
            sql_body: "SELECT 1; DROP TABLE x".into(),
        }),
        PgObjectOp::AddCheck(AddCheckOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: None,
            expression: "amount > 0; DROP TABLE x".into(),
            not_valid: false,
        }),
        PgObjectOp::CreateIndex(CreateIndexOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: None,
            unique: false,
            method: "btree".into(),
            columns: vec![PgIndexColumn {
                expression: "amount; DROP TABLE x".into(),
                descending: false,
            }],
            include: Vec::new(),
            where_predicate: None,
            concurrently: false,
        }),
        PgObjectOp::AlterColumnType(AlterColumnTypeOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            new_type: "numeric".into(),
            using: Some("amount::numeric; DROP TABLE x".into()),
        }),
        PgObjectOp::SetColumnDefault(SetColumnDefaultOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            default: Some(PgDefaultValue::Expression {
                sql: "1; DROP TABLE x".into(),
            }),
        }),
    ];
    for op in boundary_ops {
        assert!(matches!(
            generate_object_ddl(&[op]),
            Err(PgObjectError::InvalidOp { reason, .. })
                if reason == "fragment contains a statement boundary"
        ));
    }

    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::DropObject(DropObjectOp {
            reference: PgObjectRef {
                kind: PgObjectKind::Function,
                schema: Some("lifecycle".into()),
                name: "add_nums".into(),
                identity_args: None,
            },
            cascade: false,
        })]),
        Err(PgObjectError::InvalidOp { .. })
    ));
    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::RenameObject(RenameObjectOp {
            reference: table_ref(PgObjectKind::Function, "f"),
            new_name: "g".into(),
        })]),
        Err(PgObjectError::InvalidOp { .. })
    ));
    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::AddUnique(AddUniqueOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: None,
            columns: Vec::new(),
        })]),
        Err(PgObjectError::InvalidOp { .. })
    ));
}

#[test]
fn embedded_fragments_cannot_escape_typed_operations() {
    let attacks = vec![
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "constraint_in_type".into(),
                data_type: "integer NOT NULL".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "check_in_type".into(),
                data_type: "integer CHECK(false)".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::CreateSequence(CreateSequenceOp {
            schema: "lifecycle".into(),
            name: "unsafe_sequence".into(),
            data_type: Some("bigint OWNED BY lifecycle.orders.id".into()),
            start: None,
            increment: None,
            min_value: None,
            max_value: None,
            cycle: None,
            cache: None,
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "unsafe_column".into(),
                data_type: "integer, DROP COLUMN amount".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "unsafe_default".into(),
                data_type: "integer".into(),
                nullable: true,
                default: Some(PgDefaultValue::Expression {
                    sql: "0, DROP COLUMN amount".into(),
                }),
                identity: None,
            },
        }),
        PgObjectOp::AlterColumnType(AlterColumnTypeOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            new_type: "numeric".into(),
            using: Some("amount::numeric, DROP COLUMN status".into()),
        }),
        PgObjectOp::AlterColumnType(AlterColumnTypeOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            new_type: "numeric USING amount::numeric".into(),
            using: None,
        }),
        PgObjectOp::DropObject(DropObjectOp {
            reference: PgObjectRef {
                kind: PgObjectKind::Function,
                schema: Some("lifecycle".into()),
                name: "add_nums".into(),
                identity_args: Some("integer) CASCADE --".into()),
            },
            cascade: false,
        }),
    ];
    for attack in attacks {
        assert!(matches!(
            generate_object_ddl(&[attack]),
            Err(PgObjectError::InvalidOp { .. })
        ));
    }

    let legitimate = vec![
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "created_in_zone".into(),
                data_type: "timestamp with time zone".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "ratio".into(),
                data_type: "double precision".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "elapsed".into(),
                data_type: "interval day to second".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "custom_value".into(),
                data_type: "\"Custom.Schema\".\"Type.Name\"[]".into(),
                nullable: true,
                default: None,
                identity: None,
            },
        }),
        PgObjectOp::CreateSequence(CreateSequenceOp {
            schema: "lifecycle".into(),
            name: "qualified_sequence".into(),
            data_type: Some("pg_catalog.int8".into()),
            start: None,
            increment: None,
            min_value: None,
            max_value: None,
            cycle: None,
            cache: None,
        }),
        PgObjectOp::AddColumn(AddColumnOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            column: NewColumnSpec {
                name: "amounts".into(),
                data_type: "numeric(12, 2)[]".into(),
                nullable: true,
                default: Some(PgDefaultValue::Expression {
                    sql: "ARRAY[1, 2]".into(),
                }),
                identity: None,
            },
        }),
        PgObjectOp::AlterColumnType(AlterColumnTypeOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "amount".into(),
            new_type: "numeric(12, 2)".into(),
            using: Some("coalesce(amount, 0)::numeric".into()),
        }),
        PgObjectOp::SetComment(SetCommentOp {
            target: PgCommentTarget::Object {
                reference: PgObjectRef {
                    kind: PgObjectKind::Function,
                    schema: Some("lifecycle".into()),
                    name: "mixed_args".into(),
                    identity_args: Some("numeric(12, 2), text[]".into()),
                },
            },
            comment: Some("safe".into()),
        }),
    ];
    assert_eq!(
        generate_object_ddl(&legitimate)
            .expect("legitimate nested commas")
            .statements
            .len(),
        legitimate.len()
    );

    let isolated_default = generate_object_ddl(&[PgObjectOp::AddColumn(AddColumnOp {
        schema: "lifecycle".into(),
        table: "orders".into(),
        column: NewColumnSpec {
            name: "typed_default".into(),
            data_type: "integer".into(),
            nullable: true,
            default: Some(PgDefaultValue::Expression {
                sql: "0 NOT NULL".into(),
            }),
            identity: None,
        },
    })])
    .expect("expression default remains inside its renderer-owned boundary");
    assert_eq!(
        isolated_default.statements[0].sql,
        "ALTER TABLE \"lifecycle\".\"orders\" ADD COLUMN \"typed_default\" integer DEFAULT (0 NOT NULL);"
    );

    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::AlterSequence(AlterSequenceOp {
            schema: "lifecycle".into(),
            name: "unsafe_sequence".into(),
            restart_with: Some("1; DROP TABLE lifecycle.orders".into()),
            increment_by: None,
            min_value: None,
            max_value: None,
            cycle: None,
            cache: None,
        })]),
        Err(PgObjectError::InvalidOp { reason, .. })
            if reason == "sequence restart must be a signed 64-bit integer"
    ));
}
