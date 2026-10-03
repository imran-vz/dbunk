use super::*;

fn form() -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: "General".into(),
        host: "user-selected.invalid".into(),
        port: 5432,
        database: "user_database".into(),
        user: "user_name".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: true,
        tls: Default::default(),
        driver_options: Default::default(),
        ssh_tunnel: None,
    }
}

#[test]
fn general_authority_accepts_only_bounded_direct_postgres_metadata() {
    let authority = Authority {
        capability: EndpointCapability::GeneralPostgres,
        profile_id: uuid::Uuid::new_v4().to_string(),
    };
    let connection = form().into_stored("owned".into(), None).unwrap();
    assert!(authority.permits(&connection));
    let StoredConnection::PostgreSQL(pg) = connection else {
        unreachable!()
    };
    let mut default_port = pg.clone();
    default_port.port = 0;
    assert_eq!(default_port.effective_port(), 5432);
    assert!(authority.permits(&StoredConnection::PostgreSQL(default_port)));
    let mut bad_form = form();
    bad_form.port = 0;
    assert!(bad_form.into_stored("id".into(), None).is_err());
    for field in [
        "ssh", "empty", "nul", "role", "search", "tls-path", "tls-name",
    ] {
        let mut changed = pg.clone();
        match field {
            "ssh" => changed.ssh_tunnel.enabled = true,
            "empty" => changed.host = " ".into(),
            "nul" => changed.database = "a\0b".into(),
            "role" => changed.driver_options.as_mut().unwrap().default_role = Some("x".repeat(257)),
            "search" => {
                changed.driver_options.as_mut().unwrap().default_search_path =
                    Some(vec!["x".into(); 65])
            }
            "tls-path" => {
                changed.tls_options.as_mut().unwrap().root_cert_path = Some("x".repeat(4097))
            }
            "tls-name" => changed.tls_options.as_mut().unwrap().server_name = Some("a\0b".into()),
            _ => unreachable!(),
        }
        assert!(
            !authority.permits(&StoredConnection::PostgreSQL(changed)),
            "{field}"
        );
    }
    let mut json = serde_json::to_value(StoredConnection::PostgreSQL(pg)).unwrap();
    json["engine"] = "MySQL".into();
    let foreign: StoredConnection = serde_json::from_value(json).unwrap();
    assert!(!authority.permits(&foreign));
}

#[test]
fn tls_limits_match_saved_and_loaded_metadata_without_file_access() {
    let mut input = form();
    input.tls.root_cert_path = Some("x".repeat(4096));
    input.tls.client_cert_path = Some("é".repeat(2048));
    input.tls.client_key_path = Some("not-an-existing-path".into());
    input.tls.server_name = Some("x".repeat(256));
    assert!(supported_fields(
        &input.clone().into_stored("id".into(), None).unwrap()
    ));
    for field in ["root", "certificate", "key", "name", "nul"] {
        let mut changed = input.clone();
        match field {
            "root" => changed.tls.root_cert_path.as_mut().unwrap().push('x'),
            "certificate" => changed.tls.client_cert_path.as_mut().unwrap().push('é'),
            "key" => changed.tls.client_key_path = Some("x".repeat(4097)),
            "name" => changed.tls.server_name.as_mut().unwrap().push('x'),
            "nul" => changed.tls.client_key_path = Some("a\0b".into()),
            _ => unreachable!(),
        }
        assert!(changed.into_stored("id".into(), None).is_err(), "{field}");
    }
}

#[tokio::test]
async fn probe_cleanup_expires_at_original_deadline_and_joins_hung_driver() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Finished(Arc<AtomicBool>);
    impl Drop for Finished {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let tasks = crate::postgres::dedicated::DriverJoins::default();
    let done = Arc::new(AtomicBool::new(false));
    let finished = Finished(done.clone());
    tasks.track_task(tokio::spawn(async move {
        let _finished = finished;
        std::future::pending::<()>().await;
    }));
    let deadline = tokio::time::Instant::now();
    assert!(!tokio::time::timeout(
        std::time::Duration::from_secs(1),
        settle_probe(&tasks, std::future::pending(), deadline)
    )
    .await
    .unwrap());
    assert!(
        done.load(Ordering::SeqCst),
        "result requires observed driver termination"
    );
    assert!(
        settle_probe(
            &crate::postgres::dedicated::DriverJoins::default(),
            async {},
            tokio::time::Instant::now() + std::time::Duration::from_secs(1)
        )
        .await
    );
}
