//! Generator unit tests: errors, grouping, wire format, and cross-domain plans.

use super::*;

#[test]
fn applied_statements_is_read_from_execution_errors() {
    let error = PgObjectError::Database {
        statement_index: Some(2),
        code: None,
        message: "boom".into(),
        position: None,
        applied_statements: 2,
        residue: None,
    };
    assert_eq!(error.applied_statements(), 2);
    assert_eq!(
        PgObjectError::PolicyBlocked {
            reason: "read-only".into()
        }
        .applied_statements(),
        0
    );
}

#[test]
fn renders_schema_object_and_comment_operations_with_quoting() {
    assert_eq!(
        statement(PgObjectOp::CreateSchema(CreateSchemaOp {
            name: "weird\"name".into(),
        }))
        .sql,
        "CREATE SCHEMA \"weird\"\"name\";"
    );
    assert_eq!(
        statement(PgObjectOp::RenameObject(RenameObjectOp {
            reference: table_ref(PgObjectKind::View, "orders"),
            new_name: "renamed".into(),
        }))
        .sql,
        "ALTER VIEW \"lifecycle\".\"orders\" RENAME TO \"renamed\";"
    );
    assert_eq!(
        statement(PgObjectOp::DropObject(DropObjectOp {
            reference: PgObjectRef {
                kind: PgObjectKind::Function,
                schema: Some("lifecycle".into()),
                name: "add_nums".into(),
                identity_args: Some("integer, integer".into()),
            },
            cascade: false,
        }))
        .sql,
        "DROP FUNCTION \"lifecycle\".\"add_nums\"(integer, integer) RESTRICT;"
    );
    assert_eq!(
        statement(PgObjectOp::SetComment(SetCommentOp {
            target: PgCommentTarget::Object {
                reference: table_ref(PgObjectKind::Table, "orders"),
            },
            comment: Some("it's 'quoted'".into()),
        }))
        .sql,
        "COMMENT ON TABLE \"lifecycle\".\"orders\" IS E'it''s ''quoted''';"
    );
}

#[test]
fn e_literals_escape_apostrophes_and_backslashes_in_every_literal_operation() {
    let preview = generate_object_ddl(&[
        PgObjectOp::SetComment(SetCommentOp {
            target: PgCommentTarget::Object {
                reference: table_ref(PgObjectKind::Table, "orders"),
            },
            comment: Some("owner's \\note".into()),
        }),
        PgObjectOp::SetColumnDefault(SetColumnDefaultOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "status".into(),
            default: Some(PgDefaultValue::Literal {
                value: "queued\\later".into(),
            }),
        }),
        PgObjectOp::CreateEnum(CreateEnumOp {
            schema: "lifecycle".into(),
            name: "escaped_status".into(),
            labels: vec!["it's\\done".into()],
        }),
    ])
    .expect("escaped E literals remain one classified DDL statement each");
    assert!(preview.statements[0].sql.ends_with("E'owner''s \\\\note';"));
    assert!(preview.statements[1]
        .sql
        .ends_with("DEFAULT E'queued\\\\later';"));
    assert!(preview.statements[2]
        .sql
        .contains("ENUM (E'it''s\\\\done')"));
}

#[test]
fn destructive_classification_matches_data_risk() {
    let cases = vec![
        (
            PgObjectOp::DropObject(DropObjectOp {
                reference: table_ref(PgObjectKind::Table, "orders"),
                cascade: false,
            }),
            true,
        ),
        (
            PgObjectOp::SetColumnNullable(SetColumnNullableOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "amount".into(),
                nullable: false,
            }),
            true,
        ),
        (
            PgObjectOp::AddColumn(AddColumnOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                column: NewColumnSpec {
                    name: "required_value".into(),
                    data_type: "integer".into(),
                    nullable: false,
                    default: None,
                    identity: None,
                },
            }),
            true,
        ),
        (
            PgObjectOp::CreateIndex(CreateIndexOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: Some("orders_unique_amount".into()),
                unique: true,
                method: "btree".into(),
                columns: vec![PgIndexColumn {
                    expression: "amount".into(),
                    descending: false,
                }],
                include: Vec::new(),
                where_predicate: None,
                concurrently: false,
            }),
            true,
        ),
        (
            PgObjectOp::AddCheck(AddCheckOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                expression: "amount > 0".into(),
                not_valid: true,
            }),
            false,
        ),
        (
            PgObjectOp::AddCheck(AddCheckOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: None,
                expression: "amount > 0".into(),
                not_valid: false,
            }),
            true,
        ),
        (
            PgObjectOp::CreateView(CreateViewOp {
                schema: "lifecycle".into(),
                name: "danger".into(),
                or_replace: false,
                sql_body: "SELECT pg_terminate_backend(1)".into(),
            }),
            true,
        ),
    ];
    for (op, expected) in cases {
        assert_eq!(statement(op).destructive, expected);
    }
}

#[test]
fn statement_body_validation_uses_the_renderer_normalized_boundary() {
    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::CreateMaterializedView(CreateMaterializedViewOp {
            schema: "lifecycle".into(),
            name: "unsafe_body".into(),
            sql_body: "SELECT 1 -- swallows renderer suffix\n".into(),
            with_data: false,
        })]),
        Err(PgObjectError::InvalidOp { reason, .. })
            if reason == "fragment escapes its typed SQL context"
    ));
}

#[test]
fn explicitly_deferred_extension_drop_is_rejected() {
    assert!(matches!(
        generate_object_ddl(&[PgObjectOp::DropObject(DropObjectOp {
            reference: PgObjectRef {
                kind: PgObjectKind::Extension,
                schema: Some("lifecycle".into()),
                name: "hstore".into(),
                identity_args: None,
            },
            cascade: false,
        })]),
        Err(PgObjectError::InvalidOp { reason, .. })
            if reason == "dropping extensions is not supported"
    ));
}

#[test]
fn wire_shapes_are_camel_case_and_tagged() {
    let value = serde_json::to_value(PgObjectOp::AlterColumnType(AlterColumnTypeOp {
        schema: "lifecycle".into(),
        table: "orders".into(),
        name: "amount".into(),
        new_type: "numeric".into(),
        using: None,
    }))
    .expect("serialize op");
    assert_eq!(value["op"], "alterColumnType");
    assert_eq!(value["newType"], "numeric");

    let sequence = serde_json::to_value(PgObjectOp::CreateSequence(CreateSequenceOp {
        schema: "lifecycle".into(),
        name: "wire_sequence".into(),
        data_type: Some("bigint".into()),
        start: Some(i64::MAX.to_string()),
        increment: Some("1".into()),
        min_value: None,
        max_value: Some(i64::MAX.to_string()),
        cycle: Some(false),
        cache: Some("1".into()),
    }))
    .expect("serialize sequence op");
    assert!(sequence["start"].is_string());
    assert_eq!(sequence["start"], i64::MAX.to_string());
    assert!(sequence["maxValue"].is_string());

    let error = serde_json::to_value(PgObjectError::PolicyNeedsConfirmation {
        statements: vec![DdlStatementSummary {
            index: 0,
            summary: "Drop table lifecycle.orders (RESTRICT)".into(),
            destructive: true,
            transactional: true,
        }],
    })
    .expect("serialize error");
    assert_eq!(error["kind"], "policyNeedsConfirmation");
    assert_eq!(error["statements"][0]["transactional"], true);

    let lock_without_residue = serde_json::to_value(PgObjectError::LockTimeout {
        statement_index: 1,
        applied_statements: 0,
        residue: None,
    })
    .expect("serialize lock timeout");
    assert!(lock_without_residue.get("residue").is_none());
    let lock_with_residue = serde_json::to_value(PgObjectError::LockTimeout {
        statement_index: 1,
        applied_statements: 0,
        residue: Some(Box::new(DdlResidue::InvalidIndex {
            schema: "lifecycle".into(),
            name: "orders_idx".into(),
        })),
    })
    .expect("serialize lock residue");
    assert_eq!(lock_with_residue["residue"]["kind"], "invalidIndex");
}

#[test]
fn operations_keep_the_internally_tagged_wire_format() {
    // Payload structs sit behind newtype variants; serde flattens them, so
    // the JSON the frontend sends and receives is unchanged: an `op` tag
    // beside camelCase fields.
    let op = PgObjectOp::AddColumn(AddColumnOp {
        schema: "s".into(),
        table: "t".into(),
        column: NewColumnSpec {
            name: "c".into(),
            data_type: "text".into(),
            nullable: false,
            default: Some(PgDefaultValue::Expression {
                sql: "now()".into(),
            }),
            identity: Some(PgIdentity::ByDefault),
        },
    });
    let json = serde_json::to_value(&op).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({
            "op": "addColumn",
            "schema": "s",
            "table": "t",
            "column": {
                "name": "c",
                "dataType": "text",
                "nullable": false,
                "default": { "kind": "expression", "sql": "now()" },
                "identity": "by-default"
            }
        })
    );
    let decoded: PgObjectOp = serde_json::from_value(json).expect("round trip");
    assert_eq!(decoded, op);

    let grant: PgObjectOp = serde_json::from_str(
        r#"{"op":"grantPrivileges","target":{"kind":"table","schema":"s","name":"t","identityArgs":null},
            "privileges":["select","maintain"],"allPrivileges":false,
            "grantee":{"kind":"role","name":"reader"},"withGrantOption":true}"#,
    )
    .expect("frontend grant payload");
    assert!(matches!(
        grant,
        PgObjectOp::GrantPrivileges(GrantPrivilegesOp {
            with_grant_option: true,
            ..
        })
    ));
    let trigger: PgObjectOp = serde_json::from_str(
        r#"{"op":"createTrigger","schema":"s","table":"t","name":"tg","timing":"instead-of",
            "events":[{"kind":"update","columns":["a"]},{"kind":"insert"}],"forEach":"row",
            "when":null,"functionSchema":"s","functionName":"f","arguments":[],"orReplace":false}"#,
    )
    .expect("frontend trigger payload");
    assert!(matches!(
        trigger,
        PgObjectOp::CreateTrigger(CreateTriggerOp {
            timing: PgTriggerTiming::InsteadOf,
            ..
        })
    ));
}
