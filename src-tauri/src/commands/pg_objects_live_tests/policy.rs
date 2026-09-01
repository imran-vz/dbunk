//! Live: row-level security and policies.

use super::*;

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn row_security_live_forces_creates_and_drops_policies() {
    let (_directory, state, connection) = live_state("live-row-security").await;
    let reset = "DROP POLICY IF EXISTS live_policy ON lifecycle.tenant_rows;\n\
                 ALTER TABLE lifecycle.tenant_rows ENABLE ROW LEVEL SECURITY, \
                 NO FORCE ROW LEVEL SECURITY;";
    with_reset(&connection, reset, async {
        apply_live(
            &state,
            connection.id(),
            vec![
                PgObjectOp::SetRowLevelSecurity(SetRowLevelSecurityOp {
                    schema: "lifecycle".into(),
                    table: "tenant_rows".into(),
                    enabled: true,
                    force: Some(true),
                }),
                PgObjectOp::CreatePolicy(CreatePolicyOp {
                    schema: "lifecycle".into(),
                    table: "tenant_rows".into(),
                    name: "live_policy".into(),
                    permissive: false,
                    command: PgPolicyCommand::Update,
                    roles: vec![
                        PgGrantee::Public,
                        PgGrantee::Role {
                            name: "lifecycle_reader".into(),
                        },
                    ],
                    using: Some("tenant <> ''".into()),
                    with_check: Some("note IS NOT NULL".into()),
                }),
            ],
        )
        .await
        .expect("force RLS and create policy");

        let structure = fetch_table_structure(&connection, "lifecycle", "tenant_rows")
            .await
            .expect("tenant structure");
        let row_security = structure.row_security.expect("row security state");
        assert!(row_security.enabled && row_security.forced);
        let policy = structure
            .policies
            .iter()
            .find(|policy| policy.name == "live_policy")
            .expect("live policy listed");
        assert!(!policy.permissive);
        assert_eq!(policy.command, PolicyCommand::Update);
        // PostgreSQL collapses any role list that names PUBLIC to the
        // pseudo-role alone; the frontend must not expect the other roles back.
        assert_eq!(policy.roles, vec!["public".to_string()]);
        assert!(policy
            .using
            .as_deref()
            .is_some_and(|using| using.contains("tenant")));
        assert!(policy
            .with_check
            .as_deref()
            .is_some_and(|check| check.contains("note")));
        assert!(structure.policies.iter().any(|policy| {
            policy.name == "tenant_isolation" && policy.command == PolicyCommand::Select
        }));

        apply_live(
            &state,
            connection.id(),
            vec![
                PgObjectOp::DropPolicy(DropPolicyOp {
                    schema: "lifecycle".into(),
                    table: "tenant_rows".into(),
                    name: "live_policy".into(),
                }),
                PgObjectOp::SetRowLevelSecurity(SetRowLevelSecurityOp {
                    schema: "lifecycle".into(),
                    table: "tenant_rows".into(),
                    enabled: true,
                    force: Some(false),
                }),
            ],
        )
        .await
        .expect("drop policy and unforce");
        let structure = fetch_table_structure(&connection, "lifecycle", "tenant_rows")
            .await
            .expect("tenant structure");
        assert!(!structure
            .policies
            .iter()
            .any(|policy| policy.name == "live_policy"));
        assert_eq!(
            structure
                .row_security
                .map(|state| (state.enabled, state.forced)),
            Some((true, false))
        );

        // Preview refuses an INSERT policy with USING before any I/O.
        let rejected = preview_object_ddl_inner(
            &state,
            connection.id(),
            &[PgObjectOp::CreatePolicy(CreatePolicyOp {
                schema: "lifecycle".into(),
                table: "tenant_rows".into(),
                name: "bad".into(),
                permissive: true,
                command: PgPolicyCommand::Insert,
                roles: vec![PgGrantee::Public],
                using: Some("true".into()),
                with_check: None,
            })],
        )
        .await
        .expect_err("invalid policy");
        assert!(matches!(
            rejected,
            PgObjectError::InvalidOp { op_index: 0, .. }
        ));
    })
    .await;
}
