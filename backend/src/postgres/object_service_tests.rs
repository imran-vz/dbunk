use super::*;
use crate::postgres::object_ddl::*;
use crate::{SafeMode, StoredConnection};

pub(crate) use crate::app::test_postgres_connection as connection;

fn ops() -> Vec<PgObjectOp> {
    vec![PgObjectOp::CreateSchema(CreateSchemaOp {
        name: "object_policy_test".into(),
    })]
}

#[test]
fn postgres_character_positions_become_zero_based_utf8_byte_offsets() {
    let sql = "é🙂x";
    assert_eq!(postgres_position_to_byte_offset(sql, 1), Some(0));
    assert_eq!(postgres_position_to_byte_offset(sql, 2), Some(2));
    assert_eq!(postgres_position_to_byte_offset(sql, 3), Some(6));
    assert_eq!(postgres_position_to_byte_offset(sql, 4), Some(7));
    assert_eq!(postgres_position_to_byte_offset(sql, 0), None);
    assert_eq!(postgres_position_to_byte_offset(sql, 5), None);
}

#[test]
fn lock_timeout_preserves_concurrent_index_residue() {
    let residue = DdlResidue::InvalidIndex {
        schema: "lifecycle".into(),
        name: "orders_idx".into(),
    };
    assert_eq!(
        map_apply_database_error(
            Some("55P03".into()),
            "canceling statement due to lock timeout".into(),
            Some(4),
            Some(2),
            1,
            Some(residue.clone()),
        ),
        PgObjectError::LockTimeout {
            statement_index: 2,
            applied_statements: 1,
            residue: Some(Box::new(residue)),
        }
    );
}

#[tokio::test]
async fn object_ddl_preview_does_not_resolve_credentials_or_tunnels() {
    let (_directory, state) = crate::test_app_state().await;
    let mut stored = connection("pure-preview", SafeMode::Disabled, false);
    let StoredConnection::PostgreSQL(postgres) = &mut stored else {
        unreachable!("test connection is PostgreSQL")
    };
    postgres.password.clear();
    postgres.ssh_tunnel.enabled = true;
    postgres.ssh_tunnel.bastion_server_id = Some("missing-bastion".into());
    crate::storage::upsert_connection(&state.pool, &stored)
        .await
        .expect("persist unresolved connection record");

    let preview = preview(&state, "pure-preview", &ops())
        .await
        .expect("pure preview");
    assert_eq!(preview.statements.len(), 1);
}

#[test]
fn object_ddl_gate_returns_preview_summaries_and_honors_read_only() {
    let preview = generate_object_ddl(&ops()).expect("preview");
    assert!(matches!(
        authorize_object_ddl(
            &connection("read-only", SafeMode::Disabled, true),
            &preview,
            true
        ),
        Err(PgObjectError::PolicyBlocked { .. })
    ));

    let strict = connection("strict", SafeMode::Strict, false);
    assert_eq!(
        authorize_object_ddl(&strict, &preview, false),
        Err(PgObjectError::PolicyNeedsConfirmation {
            statements: statement_summaries(&preview)
        })
    );
    assert!(!statement_summaries(&preview).is_empty());
    assert!(matches!(
        authorize_object_ddl(&strict, &preview, true)
            .expect("confirmed strict authorization")
            .audit_disposition(),
        AuditDisposition::RequiredAfterSuccess
    ));
}

#[tokio::test]
#[serial_test::serial]
async fn object_ddl_refusals_never_audit() {
    let (_directory, state) = crate::test_app_state().await;
    for (connection_id, safe_mode, read_only, expected) in [
        (
            "object-read-only",
            SafeMode::Disabled,
            true,
            "policyBlocked",
        ),
        (
            "object-strict",
            SafeMode::Strict,
            false,
            "policyNeedsConfirmation",
        ),
    ] {
        crate::connections::save(&state, connection(connection_id, safe_mode, read_only))
            .await
            .expect("save policy connection");
        let result = apply(
            &state,
            ApplyObjectDdlPayload {
                connection_id: connection_id.into(),
                ops: ops(),
                confirmed: false,
            },
        )
        .await;
        assert_eq!(
            serde_json::to_value(result.expect_err("gate refusal")).expect("serialize refusal")
                ["kind"],
            expected
        );
        assert!(
            crate::storage::read_safety_overrides(&state.pool, connection_id)
                .await
                .expect("read audits")
                .is_empty()
        );
    }
}

#[tokio::test]
async fn committed_prefix_audits_once_and_preserves_failure_and_residue() {
    let (_directory, state) = crate::test_app_state().await;
    let preview = generate_object_ddl(&ops()).unwrap();
    let failure = |applied_statements| PgObjectError::LockTimeout {
        statement_index: 1,
        applied_statements,
        residue: Some(Box::new(DdlResidue::InvalidIndex {
            schema: "lifecycle".into(),
            name: "orders_idx".into(),
        })),
    };
    for (id, outcome, audited) in [
        ("no-commit", Err(failure(0)), false),
        ("committed-prefix", Err(failure(1)), true),
        ("complete", Ok(2), true),
        ("empty-success", Ok(0), true),
    ] {
        let stored = connection(id, SafeMode::Strict, false);
        crate::storage::upsert_connection(&state.pool, &stored)
            .await
            .unwrap();
        let authorization = authorize_object_ddl(&stored, &preview, true).unwrap();
        let expected = outcome.clone();
        let result = finish_apply(
            &state,
            id,
            authorization,
            std::time::Instant::now(),
            outcome,
        )
        .await;
        assert_eq!(result.map(|result| result.applied_statements), expected);
        let audits = crate::storage::read_safety_overrides(&state.pool, id)
            .await
            .unwrap();
        assert_eq!(audits.len(), if audited { 1 } else { 0 });
        if audited {
            assert_eq!(audits[0].command, "apply_object_ddl");
            assert_eq!(audits[0].classes, ["ddl"]);
        }
        assert_eq!(
            crate::storage::read_connection_by_id(&state.pool, id)
                .await
                .unwrap()
                .unwrap()
                .last_activity_at()
                .is_some(),
            audited,
        );
    }
}
