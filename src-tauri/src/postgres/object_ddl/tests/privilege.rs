//! Generator unit tests: grants and revokes.

use super::*;

#[test]
fn grants_render_per_kind_targets_and_revokes_are_destructive() {
    let table = statement(grant_op(
        table_ref(PgObjectKind::Table, "orders"),
        vec![PgPrivilege::Select, PgPrivilege::Insert],
        false,
    ));
    assert_eq!(
        table.sql,
        "GRANT SELECT, INSERT ON TABLE \"lifecycle\".\"orders\" TO \"lifecycle_reader\";"
    );
    assert_eq!(
        table.summary,
        "Grant SELECT, INSERT on table lifecycle.orders to lifecycle_reader"
    );
    assert!(!table.destructive);
    for kind in [
        PgObjectKind::View,
        PgObjectKind::MaterializedView,
        PgObjectKind::ForeignTable,
    ] {
        assert!(statement(grant_op(
            table_ref(kind, "rel"),
            vec![PgPrivilege::Select],
            false
        ))
        .sql
        .contains("ON TABLE \"lifecycle\".\"rel\""));
    }
    assert!(statement(grant_op(
        table_ref(PgObjectKind::Sequence, "order_seq"),
        vec![PgPrivilege::Usage],
        false
    ))
    .sql
    .contains("USAGE ON SEQUENCE \"lifecycle\".\"order_seq\""));
    let schema = PgObjectRef {
        kind: PgObjectKind::Schema,
        schema: None,
        name: "lifecycle".into(),
        identity_args: None,
    };
    assert_eq!(
        statement(PgObjectOp::GrantPrivileges(GrantPrivilegesOp {
            target: schema,
            privileges: Vec::new(),
            all_privileges: true,
            grantee: PgGrantee::Public,
            with_grant_option: true,
        }))
        .sql,
        "GRANT ALL PRIVILEGES ON SCHEMA \"lifecycle\" TO PUBLIC WITH GRANT OPTION;"
    );
    let function = PgObjectRef {
        kind: PgObjectKind::Function,
        schema: Some("lifecycle".into()),
        name: "add_nums".into(),
        identity_args: Some("integer, integer".into()),
    };
    assert!(statement(grant_op(
        function.clone(),
        vec![PgPrivilege::Execute],
        false
    ))
    .sql
    .contains("EXECUTE ON FUNCTION \"lifecycle\".\"add_nums\"(integer, integer) TO"));

    let revoke = statement(PgObjectOp::RevokePrivileges(RevokePrivilegesOp {
        target: table_ref(PgObjectKind::Table, "orders"),
        privileges: vec![PgPrivilege::Select],
        all_privileges: false,
        grantee: PgGrantee::Role {
            name: "lifecycle_reader".into(),
        },
        grant_option_for: true,
        cascade: true,
    }));
    assert_eq!(
        revoke.sql,
        "REVOKE GRANT OPTION FOR SELECT ON TABLE \"lifecycle\".\"orders\" FROM \"lifecycle_reader\" CASCADE;"
    );
    assert!(revoke.destructive);
    assert_eq!(
        revoke.summary,
        "Revoke grant option for SELECT on table lifecycle.orders from lifecycle_reader (CASCADE)"
    );

    let reason = sole_invalid;
    assert!(reason(grant_op(
        table_ref(PgObjectKind::Table, "orders"),
        vec![PgPrivilege::Execute],
        false
    ))
    .contains("EXECUTE is not a table privilege"));
    assert!(reason(grant_op(function, vec![PgPrivilege::Select], false))
        .contains("SELECT is not a function privilege"));
    assert!(reason(grant_op(
        table_ref(PgObjectKind::Table, "orders"),
        vec![PgPrivilege::Select],
        true
    ))
    .contains("cannot be combined"));
    assert!(reason(grant_op(
        table_ref(PgObjectKind::Table, "orders"),
        vec![],
        false
    ))
    .contains("privilege list cannot be empty"));
    assert!(reason(grant_op(
        table_ref(PgObjectKind::Table, "orders"),
        vec![PgPrivilege::Select, PgPrivilege::Select],
        false
    ))
    .contains("listed more than once"));
    assert!(reason(grant_op(
        table_ref(PgObjectKind::Type, "order_status"),
        vec![PgPrivilege::Usage],
        false
    ))
    .contains("privileges on types are not supported"));
    assert!(reason(PgObjectOp::GrantPrivileges(GrantPrivilegesOp {
        target: table_ref(PgObjectKind::Table, "orders"),
        privileges: vec![PgPrivilege::Select],
        all_privileges: false,
        grantee: PgGrantee::Role { name: "  ".into() },
        with_grant_option: false,
    }))
    .contains("role cannot be empty"));
}
