//! C08.a connection-record compatibility and route admission.
use super::*;

fn form(tunnel: Option<DevelopmentSshTunnel>) -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: "Routed".into(),
        host: "db.internal.invalid".into(),
        port: 5432,
        database: "app".into(),
        user: "app".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: Default::default(),
        driver_options: Default::default(),
        ssh_tunnel: tunnel,
    }
}

fn general() -> Authority {
    Authority {
        capability: EndpointCapability::GeneralPostgres,
        profile_id: uuid::Uuid::new_v4().to_string(),
    }
}

#[test]
fn records_written_before_ssh_support_still_deserialize_and_serialize_unchanged() {
    let old = serde_json::json!({
        "name": "Old", "host": "127.0.0.1", "port": 5432, "database": "d", "user": "u",
        "environment": "development", "safeMode": "protected", "readOnly": false,
        "tls": {"mode": "disable", "rootCertPath": null, "clientCertPath": null,
                "clientKeyPath": null, "serverName": null},
        "driverOptions": {"statementTimeoutMs": null, "idleInTransactionTimeoutMs": null,
                "connectTimeoutMs": null, "keepaliveSeconds": null,
                "defaultSearchPath": null, "defaultRole": null}
    });
    let parsed: DevelopmentPostgresConnection = serde_json::from_value(old.clone()).unwrap();
    assert!(parsed.ssh_tunnel.is_none());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), old);

    let mut routed = old;
    routed["sshTunnel"] = serde_json::json!({"bastionId": "b1"});
    let parsed: DevelopmentPostgresConnection = serde_json::from_value(routed.clone()).unwrap();
    let tunnel = parsed.ssh_tunnel.unwrap();
    assert_eq!(tunnel, DevelopmentSshTunnel::new("b1"));
    assert!(tunnel.keepalive_want_reply, "migration-safe default");

    routed["sshTunnel"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<DevelopmentPostgresConnection>(routed).is_err());
}

#[test]
fn every_stored_tunnel_option_round_trips_and_disabling_preserves_them() {
    let tunnel = DevelopmentSshTunnel {
        bastion_id: "final".into(),
        jump_chain: vec!["first".into(), "second".into()],
        local_bind_host: Some("127.0.0.1".into()),
        local_port: Some(25432),
        compression: true,
        keepalive_interval_seconds: Some(30),
        keepalive_want_reply: false,
        proxy_command: Some("ssh -W %h:%p edge".into()),
    };
    let stored = form(Some(tunnel.clone()))
        .into_stored("routed".into(), None)
        .unwrap();
    assert!(general().permits(&stored));
    let StoredConnection::PostgreSQL(pg) = &stored else {
        unreachable!()
    };
    assert_eq!(
        pg.ssh_tunnel.referenced_bastion_ids(),
        ["first", "second", "final"]
    );
    assert_eq!(
        DevelopmentPostgresConnection::from_stored(pg).ssh_tunnel,
        Some(tunnel)
    );

    let disabled = form(None)
        .into_stored("routed".into(), Some(&stored))
        .unwrap();
    let StoredConnection::PostgreSQL(disabled_pg) = &disabled else {
        unreachable!()
    };
    assert!(!disabled_pg.ssh_tunnel.enabled);
    assert_eq!(disabled_pg.ssh_tunnel.jump_chain, ["first", "second"]);
    assert!(DevelopmentPostgresConnection::from_stored(disabled_pg)
        .ssh_tunnel
        .is_none());
    assert!(general().permits(&disabled));
    assert!(
        !credentials::destination_matches(&disabled, &stored),
        "a route change requires the password again"
    );
}

#[test]
fn route_admission_refuses_exposed_or_unbounded_options() {
    let refused = |change: fn(&mut DevelopmentSshTunnel)| {
        let mut tunnel = DevelopmentSshTunnel::new("final");
        change(&mut tunnel);
        form(Some(tunnel)).into_stored("id".into(), None).is_err()
    };
    assert!(refused(|t| t.local_bind_host = Some("0.0.0.0".into())));
    assert!(refused(|t| t.local_bind_host = Some("10.0.0.5".into())));
    assert!(refused(|t| t.local_bind_host = Some("[::1]".into())));
    assert!(refused(|t| t.bastion_id = " ".into()));
    assert!(refused(|t| t.jump_chain = vec!["final".into()]));
    assert!(refused(
        |t| t.jump_chain = (0..8).map(|i| i.to_string()).collect()
    ));
    assert!(refused(|t| t.keepalive_interval_seconds = Some(1)));
    assert!(refused(|t| t.local_port = Some(0)));
    assert!(refused(|t| t.proxy_command = Some("x".repeat(4097))));
    assert!(refused(|t| t.bastion_id = "a\0b".into()));
    for loopback in ["localhost", "::1", "127.0.0.2"] {
        let mut tunnel = DevelopmentSshTunnel::new("final");
        tunnel.local_bind_host = Some(loopback.into());
        assert!(
            form(Some(tunnel)).into_stored("id".into(), None).is_ok(),
            "{loopback}"
        );
    }
}

#[test]
fn owned_fixture_profiles_never_admit_a_route() {
    let fixtures = DevelopmentFixtures::from_json(
        &serde_json::json!({
            "version":1,"fixture":"dbunk-native-stage03",
            "instance":"2283820d-33ec-4c4c-ae03-7051092bd410",
            "host":"127.0.0.1","port":15432,"database":"dbunk_demo","user":"dbunk"
        })
        .to_string(),
    )
    .unwrap();
    let authority = Authority {
        capability: EndpointCapability::OwnedFixtures(Box::new(fixtures)),
        profile_id: uuid::Uuid::new_v4().to_string(),
    };
    let mut routed = form(Some(DevelopmentSshTunnel::new("final")));
    routed.host = "127.0.0.1".into();
    routed.port = 15432;
    routed.database = "dbunk_demo".into();
    routed.user = "dbunk".into();
    let mut direct = routed.clone();
    direct.ssh_tunnel = None;
    assert!(authority.permits(&direct.into_stored("id".into(), None).unwrap()));
    assert!(!authority.permits(&routed.into_stored("id".into(), None).unwrap()));
}

#[test]
fn summaries_carry_the_stored_last_activity_and_omit_it_when_unset() {
    let StoredConnection::PostgreSQL(mut pg) = form(None).into_stored("id".into(), None).unwrap()
    else {
        unreachable!()
    };
    let unused = summary(&StoredConnection::PostgreSQL(pg.clone()), &general());
    assert_eq!(unused.last_activity_at, None);
    assert!(serde_json::to_value(&unused)
        .unwrap()
        .get("lastActivityAt")
        .is_none());

    pg.last_activity_at = Some("2026-08-24T00:00:00Z".into());
    let used = summary(&StoredConnection::PostgreSQL(pg), &general());
    assert_eq!(
        used.last_activity_at.as_deref(),
        Some("2026-08-24T00:00:00Z")
    );
    assert_eq!(
        serde_json::to_value(&used).unwrap()["lastActivityAt"],
        "2026-08-24T00:00:00Z"
    );
}
