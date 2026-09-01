//! Live: apply groups, typed errors, residue, lock timeout, and audit (Plan 013).

use super::*;

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn object_ddl_apply_live_covers_groups_errors_residue_lock_and_audit() {
    let (_directory, state) = crate::test_app_state().await;
    let connection_id = "object-ddl-live";
    let connection = connection(connection_id, SafeMode::Strict, false);
    crate::commands::connections::save_connection_inner(&state, connection.clone())
        .await
        .expect("save live connection");
    let mut setup = crate::postgres::connect(&connection)
        .await
        .expect("connect");
    sqlx::raw_sql(
        r#"
DROP SCHEMA IF EXISTS object_apply CASCADE;
CREATE SCHEMA object_apply;
CREATE TABLE object_apply.parent (id integer PRIMARY KEY);
INSERT INTO object_apply.parent VALUES (1), (2);
CREATE TABLE object_apply.scratch (id integer, value integer, parent_id integer);
INSERT INTO object_apply.scratch VALUES (1, 0, 1), (2, 2, 2);
CREATE TABLE object_apply.cast_table (value integer);
INSERT INTO object_apply.cast_table VALUES (7);
"#,
    )
    .execute(&mut *setup)
    .await
    .expect("set up object DDL fixture");
    drop(setup);

    let result = apply_live(
        &state,
        connection_id,
        vec![
            PgObjectOp::CreateSchema(CreateSchemaOp {
                name: "object_roundtrip".into(),
            }),
            PgObjectOp::RenameObject(RenameObjectOp {
                reference: PgObjectRef {
                    kind: PgObjectKind::Schema,
                    schema: None,
                    name: "object_roundtrip".into(),
                    identity_args: None,
                },
                new_name: "object_roundtrip_renamed".into(),
            }),
            PgObjectOp::SetComment(SetCommentOp {
                target: PgCommentTarget::Object {
                    reference: PgObjectRef {
                        kind: PgObjectKind::Schema,
                        schema: None,
                        name: "object_roundtrip_renamed".into(),
                        identity_args: None,
                    },
                },
                comment: Some("round trip".into()),
            }),
            PgObjectOp::DropObject(DropObjectOp {
                reference: PgObjectRef {
                    kind: PgObjectKind::Schema,
                    schema: None,
                    name: "object_roundtrip_renamed".into(),
                    identity_args: None,
                },
                cascade: false,
            }),
        ],
    )
    .await
    .expect("schema round trip");
    assert_eq!(result.applied_statements, 4);

    apply_live(
        &state,
        connection_id,
        vec![
            PgObjectOp::AddCheck(AddCheckOp {
                schema: "object_apply".into(),
                table: "scratch".into(),
                name: Some("scratch_value_check".into()),
                expression: "value >= 0".into(),
                not_valid: false,
            }),
            PgObjectOp::AddUnique(AddUniqueOp {
                schema: "object_apply".into(),
                table: "scratch".into(),
                name: Some("scratch_id_unique".into()),
                columns: vec!["id".into()],
            }),
            PgObjectOp::AddForeignKey(AddForeignKeyOp {
                schema: "object_apply".into(),
                table: "scratch".into(),
                name: Some("scratch_parent_fk".into()),
                columns: vec!["parent_id".into()],
                referenced_schema: "object_apply".into(),
                referenced_table: "parent".into(),
                referenced_columns: vec!["id".into()],
                on_update: PgReferentialAction::NoAction,
                on_delete: PgReferentialAction::Restrict,
                deferrable: false,
                initially_deferred: false,
                not_valid: true,
            }),
            PgObjectOp::DropConstraint(DropConstraintOp {
                schema: "object_apply".into(),
                table: "scratch".into(),
                name: "scratch_value_check".into(),
                cascade: false,
            }),
            PgObjectOp::DropConstraint(DropConstraintOp {
                schema: "object_apply".into(),
                table: "scratch".into(),
                name: "scratch_id_unique".into(),
                cascade: false,
            }),
            PgObjectOp::DropConstraint(DropConstraintOp {
                schema: "object_apply".into(),
                table: "scratch".into(),
                name: "scratch_parent_fk".into(),
                cascade: false,
            }),
        ],
    )
    .await
    .expect("constraint round trip");

    apply_live(
        &state,
        connection_id,
        vec![PgObjectOp::AlterColumnType(AlterColumnTypeOp {
            schema: "object_apply".into(),
            table: "cast_table".into(),
            name: "value".into(),
            new_type: "text".into(),
            using: Some("value::text".into()),
        })],
    )
    .await
    .expect("alter column type with USING");

    apply_live(
        &state,
        connection_id,
        vec![PgObjectOp::CreateEnum(CreateEnumOp {
            schema: "object_apply".into(),
            name: "state".into(),
            labels: vec!["new".into(), "done".into()],
        })],
    )
    .await
    .expect("create enum");
    apply_live(
        &state,
        connection_id,
        vec![PgObjectOp::AddEnumValue(AddEnumValueOp {
            schema: "object_apply".into(),
            name: "state".into(),
            value: "queued".into(),
            position: Some(PgEnumPosition::Before {
                neighbor: "done".into(),
            }),
        })],
    )
    .await
    .expect("add enum value");
    apply_live(
        &state,
        connection_id,
        vec![PgObjectOp::RenameEnumValue(RenameEnumValueOp {
            schema: "object_apply".into(),
            name: "state".into(),
            from: "queued".into(),
            to: "processing".into(),
        })],
    )
    .await
    .expect("rename enum value");

    apply_live(
        &state,
        connection_id,
        vec![index_op("scratch_id_idx", "id")],
    )
    .await
    .expect("concurrent index outside a transaction");

    let atomic_failure = apply_live(
        &state,
        connection_id,
        vec![
            PgObjectOp::CreateSchema(CreateSchemaOp {
                name: "atomic_rollback".into(),
            }),
            PgObjectOp::CreateSchema(CreateSchemaOp {
                name: "atomic_rollback".into(),
            }),
        ],
    )
    .await
    .expect_err("atomic group must fail");
    assert!(matches!(
        atomic_failure,
        PgObjectError::Database {
            statement_index: Some(1),
            applied_statements: 0,
            ref code,
            ..
        } if code.as_deref() == Some("42P06")
    ));
    assert!(
        crate::postgres::objects::load_pg_object_catalog(&connection)
            .await
            .is_ok()
    );
    let mut verifier = crate::postgres::connect(&connection).await.expect("verify");
    let rolled_back: bool = sqlx::query_scalar("SELECT to_regnamespace('atomic_rollback') IS NULL")
        .fetch_one(&mut *verifier)
        .await
        .expect("verify rollback");
    assert!(rolled_back);
    drop(verifier);

    let audits_before_partial = crate::storage::read_safety_overrides(&state.pool, connection_id)
        .await
        .expect("read audits before partial failure")
        .len();
    let partial_failure = apply_live(
        &state,
        connection_id,
        vec![
            index_op("standalone_first_idx", "parent_id"),
            PgObjectOp::CreateSchema(CreateSchemaOp {
                name: "object_apply".into(),
            }),
        ],
    )
    .await
    .expect_err("post-standalone failure");
    assert!(matches!(
        partial_failure,
        PgObjectError::Database {
            statement_index: Some(1),
            applied_statements: 1,
            ..
        }
    ));
    // The committed standalone group is a confirmed schema change and is
    // audited even though the plan as a whole failed.
    assert_eq!(
        crate::storage::read_safety_overrides(&state.pool, connection_id)
            .await
            .expect("read audits after partial failure")
            .len(),
        audits_before_partial + 1
    );
    assert!(
        crate::postgres::objects::load_pg_object_catalog(&connection)
            .await
            .is_ok()
    );

    let residue_failure = apply_live(
        &state,
        connection_id,
        vec![index_op("residue_idx", "sqrt(value - 1)")],
    )
    .await
    .expect_err("concurrent expression failure");
    assert!(matches!(
        residue_failure,
        PgObjectError::Database {
            residue: Some(ref residue),
            ..
        } if matches!(
            residue.as_ref(),
            DdlResidue::InvalidIndex { schema, name }
                if schema == "object_apply" && name == "residue_idx"
        )
    ));
    assert!(
        crate::postgres::objects::load_pg_object_catalog(&connection)
            .await
            .is_ok()
    );
    apply_live(
        &state,
        connection_id,
        ["residue_idx", "standalone_first_idx"]
            .into_iter()
            .map(|name| {
                PgObjectOp::DropIndex(DropIndexOp {
                    schema: "object_apply".into(),
                    name: name.into(),
                    concurrently: false,
                    cascade: false,
                })
            })
            .collect(),
    )
    .await
    .expect("clean index residue");

    let mut locker = crate::postgres::connect(&connection).await.expect("locker");
    let mut lock_transaction = locker.begin().await.expect("begin lock");
    sqlx::query("SELECT * FROM object_apply.scratch FOR UPDATE")
        .execute(&mut *lock_transaction)
        .await
        .expect("hold row lock");
    let lock_started = std::time::Instant::now();
    let lock_failure = apply_live(
        &state,
        connection_id,
        vec![PgObjectOp::AddColumn(AddColumnOp {
            schema: "object_apply".into(),
            table: "scratch".into(),
            column: NewColumnSpec {
                name: "blocked_column".into(),
                data_type: "text".into(),
                nullable: true,
                default: Some(PgDefaultValue::Literal {
                    value: "waiting".into(),
                }),
                identity: None,
            },
        })],
    )
    .await
    .expect_err("lock timeout");
    assert!(matches!(
        lock_failure,
        PgObjectError::LockTimeout {
            statement_index: 0,
            applied_statements: 0,
            residue: None
        }
    ));
    assert!(lock_started.elapsed() < std::time::Duration::from_secs(15));
    lock_transaction.rollback().await.expect("release lock");
    assert!(
        crate::postgres::objects::load_pg_object_catalog(&connection)
            .await
            .is_ok()
    );

    // DROP INDEX CONCURRENTLY marks the index invalid before waiting for
    // writers, so a lock timeout in its second phase leaves residue.
    let mut writer = crate::postgres::connect(&connection).await.expect("writer");
    let mut write_transaction = writer.begin().await.expect("begin write");
    sqlx::query("UPDATE object_apply.scratch SET value = value WHERE false")
        .execute(&mut *write_transaction)
        .await
        .expect("hold row exclusive lock");
    let drop_failure = apply_live(
        &state,
        connection_id,
        vec![PgObjectOp::DropIndex(DropIndexOp {
            schema: "object_apply".into(),
            name: "scratch_id_idx".into(),
            concurrently: true,
            cascade: false,
        })],
    )
    .await
    .expect_err("concurrent drop lock timeout");
    assert!(matches!(
        drop_failure,
        PgObjectError::LockTimeout {
            statement_index: 0,
            applied_statements: 0,
            residue: Some(ref residue),
        } if matches!(
            residue.as_ref(),
            DdlResidue::InvalidIndex { schema, name }
                if schema == "object_apply" && name == "scratch_id_idx"
        )
    ));
    write_transaction
        .rollback()
        .await
        .expect("release write lock");

    let audits = crate::storage::read_safety_overrides(&state.pool, connection_id)
        .await
        .expect("read DDL audits");
    assert!(audits
        .iter()
        .any(|audit| audit.command == "apply_object_ddl"));

    let mut cleanup = crate::postgres::connect(&connection)
        .await
        .expect("cleanup");
    sqlx::query("DROP SCHEMA object_apply CASCADE")
        .execute(&mut *cleanup)
        .await
        .expect("drop apply fixture");
}
