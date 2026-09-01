//! Live: grants and revokes.

use super::*;

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn privileges_live_grant_revoke_and_unknown_role() {
    let (_directory, state, connection) = live_state("live-privileges").await;
    let reset = "REVOKE INSERT ON lifecycle.orders FROM lifecycle_reader;";
    with_reset(&connection, reset, async {
        let orders = || lifecycle_ref(PgObjectKind::Table, "orders", None);
        let reader = || PgGrantee::Role {
            name: "lifecycle_reader".into(),
        };
        apply_live(
            &state,
            connection.id(),
            vec![PgObjectOp::GrantPrivileges(GrantPrivilegesOp {
                target: orders(),
                privileges: vec![PgPrivilege::Insert],
                all_privileges: false,
                grantee: reader(),
                with_grant_option: true,
            })],
        )
        .await
        .expect("grant insert");
        let structure = fetch_table_structure(&connection, "lifecycle", "orders")
            .await
            .expect("orders structure");
        assert!(structure.capabilities.privileges);
        assert!(structure.privileges.iter().any(|privilege| {
            privilege.grantee == "lifecycle_reader"
                && privilege.privilege == "INSERT"
                && privilege.grantable
        }));
        // The fixture's SELECT grant is explicit and therefore listed.
        assert!(structure.privileges.iter().any(|privilege| {
            privilege.grantee == "lifecycle_reader"
                && privilege.privilege == "SELECT"
                && !privilege.grantable
        }));

        apply_live(
            &state,
            connection.id(),
            vec![PgObjectOp::RevokePrivileges(RevokePrivilegesOp {
                target: orders(),
                privileges: vec![PgPrivilege::Insert],
                all_privileges: false,
                grantee: reader(),
                grant_option_for: false,
                cascade: false,
            })],
        )
        .await
        .expect("revoke insert");
        let structure = fetch_table_structure(&connection, "lifecycle", "orders")
            .await
            .expect("orders structure");
        assert!(!structure.privileges.iter().any(|privilege| {
            privilege.grantee == "lifecycle_reader" && privilege.privilege == "INSERT"
        }));

        // A grant to an unknown role is a typed database error at apply time.
        let missing = apply_live(
            &state,
            connection.id(),
            vec![PgObjectOp::GrantPrivileges(GrantPrivilegesOp {
                target: orders(),
                privileges: vec![PgPrivilege::Select],
                all_privileges: false,
                grantee: PgGrantee::Role {
                    name: "no_such_role_dbunk".into(),
                },
                with_grant_option: false,
            })],
        )
        .await
        .expect_err("unknown role");
        assert!(matches!(
            missing,
            PgObjectError::Database { ref code, applied_statements: 0, .. }
                if code.as_deref() == Some("42704")
        ));
    })
    .await;
}
