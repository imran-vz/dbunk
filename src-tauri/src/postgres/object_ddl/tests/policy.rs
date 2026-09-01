//! Generator unit tests: row-level security and policies.

use super::*;

#[test]
fn row_level_security_renders_one_alter_and_flags_disable() {
    let rls = |enabled: bool, force: Option<bool>| {
        statement(PgObjectOp::SetRowLevelSecurity(SetRowLevelSecurityOp {
            schema: "lifecycle".into(),
            table: "tenant_rows".into(),
            enabled,
            force,
        }))
    };
    let enabled = rls(true, Some(true));
    assert_eq!(
        enabled.sql,
        "ALTER TABLE \"lifecycle\".\"tenant_rows\" ENABLE ROW LEVEL SECURITY, FORCE ROW LEVEL SECURITY;"
    );
    assert_eq!(
        enabled.summary,
        "Enable row-level security on lifecycle.tenant_rows (forced)"
    );
    assert!(!enabled.destructive);
    assert!(!rls(true, Some(false)).destructive);
    let disabled = rls(false, None);
    assert_eq!(
        disabled.sql,
        "ALTER TABLE \"lifecycle\".\"tenant_rows\" DISABLE ROW LEVEL SECURITY;"
    );
    assert!(disabled.destructive);
    assert!(rls(false, Some(false))
        .sql
        .contains("DISABLE ROW LEVEL SECURITY, NO FORCE ROW LEVEL SECURITY"));
}

#[test]
fn policies_render_roles_and_expressions_and_validate_command_shapes() {
    let op = PgObjectOp::CreatePolicy(CreatePolicyOp {
        schema: "lifecycle".into(),
        table: "tenant_rows".into(),
        name: "tenant_isolation".into(),
        permissive: false,
        command: PgPolicyCommand::Update,
        roles: vec![
            PgGrantee::Public,
            PgGrantee::Role {
                name: "public".into(),
            },
            PgGrantee::Role {
                name: "app user".into(),
            },
        ],
        using: Some("tenant = current_setting('app.tenant', true)".into()),
        with_check: Some("tenant <> ''".into()),
    });
    let created = statement(op);
    assert_eq!(
        created.sql,
        "CREATE POLICY \"tenant_isolation\" ON \"lifecycle\".\"tenant_rows\" AS RESTRICTIVE FOR UPDATE \
         TO PUBLIC, \"public\", \"app user\" USING (tenant = current_setting('app.tenant', true)) \
         WITH CHECK (tenant <> '');"
    );
    assert!(!created.destructive);

    let dropped = statement(PgObjectOp::DropPolicy(DropPolicyOp {
        schema: "lifecycle".into(),
        table: "tenant_rows".into(),
        name: "tenant_isolation".into(),
    }));
    assert_eq!(
        dropped.sql,
        "DROP POLICY \"tenant_isolation\" ON \"lifecycle\".\"tenant_rows\";"
    );
    assert!(dropped.destructive);

    let reason = sole_invalid;
    assert!(
        reason(policy_op(PgPolicyCommand::Insert, Some("true"), None))
            .contains("INSERT policies cannot have a USING")
    );
    assert!(reason(policy_op(
        PgPolicyCommand::Select,
        Some("true"),
        Some("true")
    ))
    .contains("SELECT policies cannot have a WITH CHECK"));
    assert!(
        reason(policy_op(PgPolicyCommand::Delete, None, Some("true")))
            .contains("DELETE policies cannot have a WITH CHECK")
    );
    // Both expressions are optional in PostgreSQL's grammar.
    assert_eq!(
        statement(policy_op(PgPolicyCommand::All, None, None)).sql,
        "CREATE POLICY \"tenant_isolation\" ON \"lifecycle\".\"tenant_rows\" AS PERMISSIVE FOR ALL TO PUBLIC;"
    );
    // Refusals name the failing operation, not the first one.
    assert_eq!(
        invalid_reason(&[
            PgObjectOp::CreateSchema(CreateSchemaOp { name: "a".into() }),
            PgObjectOp::CreateSchema(CreateSchemaOp { name: "b".into() }),
            policy_op(PgPolicyCommand::Insert, Some("true"), None),
        ])
        .0,
        2
    );
    assert!(reason(policy_op(
        PgPolicyCommand::All,
        Some("true) WITH CHECK (false"),
        None
    ))
    .contains("escapes its typed SQL context"));
    let mut no_roles = policy_op(PgPolicyCommand::All, Some("true"), None);
    if let PgObjectOp::CreatePolicy(CreatePolicyOp { roles, .. }) = &mut no_roles {
        roles.clear();
    }
    assert!(reason(no_roles).contains("roles cannot be empty"));
    assert!(generate_object_ddl(&[policy_op(PgPolicyCommand::Insert, None, Some("true"))]).is_ok());
}
