//! Plan 031 step 4: non-PostgreSQL native connection records and authority.
use super::*;
use crate::backend::development::{Authority, DevelopmentFixtures, EndpointCapability};
use crate::backend::{
    Backend, DevelopmentConnectionOrganization, DevelopmentConnectionTest, DevelopmentStorageMode,
};
use crate::credentials;

fn general() -> Authority {
    Authority {
        capability: EndpointCapability::GeneralPostgres,
        profile_id: uuid::Uuid::new_v4().to_string(),
    }
}

fn fixture() -> Authority {
    Authority {
        capability: EndpointCapability::OwnedFixtures(Box::new(
            DevelopmentFixtures::from_json(
                &serde_json::json!({
                    "version": 1, "fixture": "dbunk-native-stage03",
                    "instance": "2283820d-33ec-4c4c-ae03-7051092bd410",
                    "host": "127.0.0.1", "port": 15432, "database": "dbunk_demo", "user": "dbunk"
                })
                .to_string(),
            )
            .unwrap(),
        )),
        profile_id: uuid::Uuid::new_v4().to_string(),
    }
}

fn mysql() -> DevelopmentMySqlConnection {
    DevelopmentMySqlConnection {
        name: "Orders".into(),
        host: "mysql.invalid".into(),
        port: 3306,
        database: "orders".into(),
        user: "app".into(),
        environment: DevelopmentEnvironment::Staging,
        safe_mode: DevelopmentSafeMode::Strict,
        read_only: true,
        ssl: true,
        ssh_tunnel: None,
    }
}

fn sqlite(path: &str) -> DevelopmentSqliteConnection {
    DevelopmentSqliteConnection {
        name: "Local file".into(),
        path: path.into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
    }
}

fn clickhouse() -> DevelopmentClickHouseConnection {
    DevelopmentClickHouseConnection {
        name: "Events".into(),
        host: "ch.invalid".into(),
        port: 8443,
        database: "analytics".into(),
        user: "default".into(),
        environment: DevelopmentEnvironment::Production,
        safe_mode: DevelopmentSafeMode::Inherit,
        read_only: false,
        use_https: true,
        url_path: "/clickhouse".into(),
        ssh_tunnel: None,
    }
}

fn redis() -> DevelopmentRedisConnection {
    DevelopmentRedisConnection {
        name: "Cache".into(),
        host: "redis.invalid".into(),
        port: 6380,
        db_number: 3,
        user: String::new(),
        environment: DevelopmentEnvironment::Test,
        safe_mode: DevelopmentSafeMode::Disabled,
        read_only: true,
        use_tls: true,
        verify_tls_cert: false,
        ssh_tunnel: None,
    }
}

fn all_forms() -> Vec<DevelopmentEngineConnection> {
    vec![
        DevelopmentEngineConnection::MySQL(mysql()),
        DevelopmentEngineConnection::SQLite(sqlite("/var/data/app.sqlite")),
        DevelopmentEngineConnection::ClickHouse(clickhouse()),
        DevelopmentEngineConnection::Redis(redis()),
    ]
}

#[test]
fn every_engine_round_trips_through_its_stored_variant_and_policy() {
    for form in all_forms() {
        let engine = form.engine();
        let stored = form.clone().into_stored("id".into(), None).unwrap();
        assert_eq!(stored.engine().as_str(), engine);
        assert_eq!(stored.password(), "");
        assert_eq!(stored.role(), "read/write");
        let policy = stored.policy();
        assert_eq!(environment_of(policy.environment), form.environment());
        let back = DevelopmentEngineConnection::from_stored(&stored);
        assert_eq!(
            serde_json::to_value(&back).unwrap(),
            serde_json::to_value(&form).unwrap(),
            "{engine}"
        );
        assert!(general().permits(&stored), "{engine}");
        assert!(!fixture().permits(&stored), "{engine}");
    }
    let StoredConnection::Redis(stored) = DevelopmentEngineConnection::Redis(redis())
        .into_stored("id".into(), None)
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        (stored.db_number, stored.use_tls, stored.verify_tls_cert),
        (3, true, false)
    );
    assert!(stored.read_only);
    let StoredConnection::ClickHouse(stored) =
        DevelopmentEngineConnection::ClickHouse(clickhouse())
            .into_stored("id".into(), None)
            .unwrap()
    else {
        unreachable!()
    };
    assert!(stored.use_https);
    assert_eq!(stored.url_path, "/clickhouse");
}

#[test]
fn wire_form_is_engine_tagged_strict_and_tolerates_missing_optional_route() {
    let json = serde_json::to_value(DevelopmentEngineConnection::Redis(redis())).unwrap();
    assert_eq!(json["engine"], "Redis");
    assert_eq!(json["dbNumber"], 3);
    assert!(json.get("sshTunnel").is_none(), "absent route is omitted");
    let parsed: DevelopmentEngineConnection = serde_json::from_value(json.clone()).unwrap();
    assert!(matches!(parsed, DevelopmentEngineConnection::Redis(_)));
    let mut unknown = json.clone();
    unknown["futureField"] = true.into();
    assert!(serde_json::from_value::<DevelopmentEngineConnection>(unknown).is_err());
    let mut wrong = json;
    wrong["engine"] = "Oracle".into();
    assert!(serde_json::from_value::<DevelopmentEngineConnection>(wrong).is_err());
    // PostgreSQL keeps its existing strict shape inside the tagged union.
    let pg = serde_json::json!({
        "engine": "PostgreSQL", "name": "Old", "host": "h", "port": 5432, "database": "d",
        "user": "u", "environment": "development", "safeMode": "protected", "readOnly": false,
        "tls": {"mode": "disable", "rootCertPath": null, "clientCertPath": null,
                "clientKeyPath": null, "serverName": null},
        "driverOptions": {"statementTimeoutMs": null, "idleInTransactionTimeoutMs": null,
                "connectTimeoutMs": null, "keepaliveSeconds": null,
                "defaultSearchPath": null, "defaultRole": null}
    });
    let parsed: DevelopmentEngineConnection = serde_json::from_value(pg.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), pg);
}

#[test]
fn edits_preserve_unedited_columns_and_refuse_engine_changes() {
    let mut previous = DevelopmentEngineConnection::Redis(redis())
        .into_stored("id".into(), None)
        .unwrap();
    let StoredConnection::Redis(stored) = &mut previous else {
        unreachable!()
    };
    stored.role = "read-only".into();
    stored.last_activity_at = Some("2026-01-01T00:00:00Z".into());
    stored.database = "legacy".into();
    stored.organization.project = "Billing".into();
    stored.ssh_tunnel = crate::SshTunnelConfig {
        enabled: true,
        bastion_server_id: Some("edge".into()),
        local_port: Some(16379),
        ..Default::default()
    };
    let mut edit = redis();
    edit.name = " Renamed ".into();
    let StoredConnection::Redis(saved) = DevelopmentEngineConnection::Redis(edit)
        .into_stored("id".into(), Some(&previous))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(saved.name, "Renamed");
    assert_eq!(saved.role, "read-only");
    assert_eq!(
        saved.last_activity_at.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(saved.database, "legacy");
    assert_eq!(saved.organization.project, "Billing");
    assert!(
        !saved.ssh_tunnel.enabled,
        "form without a route disables it"
    );
    assert_eq!(saved.ssh_tunnel.local_port, Some(16379), "options kept");

    let mut legacy_sqlite = DevelopmentEngineConnection::SQLite(sqlite("/a.db"))
        .into_stored("s".into(), None)
        .unwrap();
    if let StoredConnection::SQLite(stored) = &mut legacy_sqlite {
        stored.host = "sentinel".into();
    }
    let StoredConnection::SQLite(saved) = DevelopmentEngineConnection::SQLite(sqlite("/b.db"))
        .into_stored("s".into(), Some(&legacy_sqlite))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        (saved.database.as_str(), saved.host.as_str()),
        ("/b.db", "sentinel")
    );
    // A legacy relative location may be kept, but not newly chosen.
    if let StoredConnection::SQLite(stored) = &mut legacy_sqlite {
        stored.database = "sqlite:legacy.db".into();
    }
    assert!(
        DevelopmentEngineConnection::SQLite(sqlite("sqlite:legacy.db"))
            .into_stored("s".into(), Some(&legacy_sqlite))
            .is_ok()
    );
    assert!(
        DevelopmentEngineConnection::SQLite(sqlite("sqlite:other.db"))
            .into_stored("s".into(), Some(&legacy_sqlite))
            .is_err()
    );

    for form in all_forms() {
        let other = if form.engine() == "MySQL" {
            DevelopmentEngineConnection::Redis(redis())
        } else {
            DevelopmentEngineConnection::MySQL(mysql())
        };
        let previous = other.into_stored("id".into(), None).unwrap();
        assert!(form
            .into_stored("id".into(), Some(&previous))
            .unwrap_err()
            .contains("cannot change engine"));
    }
}

#[test]
fn bounds_are_enforced_on_save_and_on_loaded_records() {
    let mut cases: Vec<DevelopmentEngineConnection> = Vec::new();
    let mut m = mysql();
    m.user = " ".into();
    cases.push(DevelopmentEngineConnection::MySQL(m));
    let mut m = mysql();
    m.port = 0;
    cases.push(DevelopmentEngineConnection::MySQL(m));
    let mut c = clickhouse();
    c.host = String::new();
    cases.push(DevelopmentEngineConnection::ClickHouse(c));
    let mut c = clickhouse();
    c.url_path = "/a b".into();
    cases.push(DevelopmentEngineConnection::ClickHouse(c));
    let mut r = redis();
    r.name = "x".repeat(257);
    cases.push(DevelopmentEngineConnection::Redis(r));
    let mut r = redis();
    r.user = "a\0b".into();
    cases.push(DevelopmentEngineConnection::Redis(r));
    for path in ["", "relative.db", "/a?mode=rwc", "/a#b", "/a%3Fb", "/a\0b"] {
        cases.push(DevelopmentEngineConnection::SQLite(sqlite(path)));
    }
    cases.push(DevelopmentEngineConnection::SQLite(sqlite(&format!(
        "/{}",
        "x".repeat(4096)
    ))));
    for case in cases {
        let label = serde_json::to_string(&case).unwrap();
        assert!(case.into_stored("id".into(), None).is_err(), "{label}");
    }

    // Loaded legacy rows: port 0 means the engine default and stays supported.
    let mut stored = DevelopmentEngineConnection::MySQL(mysql())
        .into_stored("id".into(), None)
        .unwrap();
    if let StoredConnection::MySQL(c) = &mut stored {
        c.port = 0;
    }
    assert!(general().permits(&stored));
    assert_eq!(
        DevelopmentEngineConnection::from_stored(&stored)
            .endpoint()
            .port,
        Some(3306)
    );
    if let StoredConnection::MySQL(c) = &mut stored {
        c.host = "x".repeat(257);
    }
    assert!(!general().permits(&stored));
    let mut stored = DevelopmentEngineConnection::Redis(redis())
        .into_stored("id".into(), None)
        .unwrap();
    if let StoredConnection::Redis(c) = &mut stored {
        c.ssh_tunnel.enabled = true;
    }
    assert!(!general().permits(&stored), "enabled route needs a bastion");
}

#[test]
fn endpoint_labels_never_include_secrets() {
    let labels = all_forms()
        .iter()
        .map(|form| form.endpoint().label())
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            "mysql.invalid:3306/orders",
            "/var/data/app.sqlite",
            "ch.invalid:8443/analytics",
            "redis.invalid:6380/3",
        ]
    );
    let mut m = mysql();
    m.database = String::new();
    assert_eq!(
        DevelopmentEngineConnection::MySQL(m).endpoint().label(),
        "mysql.invalid:3306"
    );
}

#[test]
fn ping_errors_map_to_redacted_failure_classes() {
    use super::super::DevelopmentConnectionFailure as F;
    let cases = [
        (
            "Connection to h:1 timed out. The host may be unreachable",
            "Timeout",
        ),
        ("Access denied for user 'app'@'host'", "Authentication"),
        (
            "Authentication rejected by h:6379: wrong password.",
            "Authentication",
        ),
        (
            "Code: 516. DB::Exception: default: Authentication failed",
            "Authentication",
        ),
        (
            "Connected over rediss:// but the server returned non-TLS data",
            "Tls",
        ),
        (
            "Could not connect to h:1: connection refused. Is Redis running",
            "ConnectionLost",
        ),
        ("Could not resolve hostname \"x\"", "ConnectionLost"),
        ("Unknown database 'missing'", "Database"),
    ];
    for (error, expected) in cases {
        let class = match classify_engine_error(error) {
            F::Timeout => "Timeout",
            F::Authentication => "Authentication",
            F::Tls(_) => "Tls",
            F::ConnectionLost => "ConnectionLost",
            F::Database => "Database",
            F::SshTunnel | F::SshHostKey => "Ssh",
        };
        assert_eq!(class, expected, "{error}");
    }
}

#[tokio::test]
async fn sqlite_file_check_requires_an_existing_readable_file_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let missing = root.join("missing.sqlite");
    assert!(check_sqlite_file(missing.to_str().unwrap()).await.is_err());
    assert!(!missing.exists(), "the check never creates a file");
    assert!(check_sqlite_file(root.to_str().unwrap())
        .await
        .unwrap_err()
        .contains("regular file"));
    let file = root.join("present.sqlite");
    std::fs::write(&file, []).unwrap();
    check_sqlite_file(file.to_str().unwrap()).await.unwrap();
}

/// End-to-end through a general profile: save, list, edit with a blank
/// password, duplicate, refuse an engine change, test, and delete. Only a
/// local SQLite file and a refused loopback port are contacted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn general_profile_saves_tests_duplicates_and_deletes_every_engine() {
    const CASE: &str = "DBUNK_ENGINE_CONNECTION_TEST";
    if std::env::var_os(CASE).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::development::connections::engine_connections::tests::general_profile_saves_tests_duplicates_and_deletes_every_engine",
                "--nocapture",
            ])
            .env(CASE, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let backend = Backend::create_native_profile(&root.join("general"))
        .await
        .unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    let database = root.join("app.sqlite");
    std::fs::write(&database, []).unwrap();
    let missing = root.join("missing.sqlite");
    let organization = DevelopmentConnectionOrganization {
        folder: " Data ".into(),
        is_favorite: true,
        color: "blue".into(),
        project: " Platform ".into(),
    };

    // A SQLite path that does not exist is refused and never created.
    assert!(backend
        .save_development_engine_connection(
            None,
            DevelopmentEngineConnection::SQLite(sqlite(missing.to_str().unwrap())),
            String::new(),
            organization.clone(),
        )
        .await
        .is_err());
    assert!(!missing.exists());
    assert!(backend.development_connections().await.unwrap().is_empty());

    let mut forms = all_forms();
    forms[1] = DevelopmentEngineConnection::SQLite(sqlite(database.to_str().unwrap()));
    let mut saved = Vec::new();
    for form in forms {
        let engine = form.engine();
        let row = backend
            .save_development_engine_connection(
                None,
                form,
                format!("{engine}-secret"),
                organization.clone(),
            )
            .await
            .unwrap();
        assert_eq!(row.engine, engine);
        assert!(row.unsupported_reason.is_none());
        assert!(row.postgres.is_none());
        assert_eq!(row.organization.project, "Platform");
        assert_eq!(row.organization.folder, "Data");
        assert!(!serde_json::to_string(&row).unwrap().contains("-secret"));
        assert!(!format!("{row:?}").contains("-secret"));
        saved.push(row);
    }
    let listed = backend.development_connections().await.unwrap();
    assert_eq!(listed.len(), 4);
    for row in &listed {
        let settings = row.settings.as_ref().expect("supported engine settings");
        assert_eq!(settings.engine(), row.engine);
        assert_eq!(settings.environment(), row.environment);
        assert!(row.endpoint().is_some());
    }

    // Blank password on edit keeps the stored secret for every engine.
    for row in &saved {
        let mut settings = row.settings.clone().unwrap();
        match &mut settings {
            DevelopmentEngineConnection::MySQL(c) => c.name = "Edited".into(),
            DevelopmentEngineConnection::SQLite(c) => c.name = "Edited".into(),
            DevelopmentEngineConnection::ClickHouse(c) => c.name = "Edited".into(),
            DevelopmentEngineConnection::Redis(c) => c.name = "Edited".into(),
            DevelopmentEngineConnection::PostgreSQL(_) => unreachable!(),
        }
        let edited = backend
            .save_development_engine_connection(
                Some(row.id.clone()),
                settings,
                String::new(),
                row.organization.clone(),
            )
            .await
            .unwrap();
        assert_eq!(edited.name, "Edited");
        assert_eq!(edited.engine, row.engine);
    }
    let secrets = credentials::read_all(
        &backend.0.state.credentials,
        crate::CredentialStorageMode::PlainSqlite,
    )
    .await
    .unwrap();
    for row in &saved {
        assert_eq!(secrets[&row.id], format!("{}-secret", row.engine));
    }

    // A saved record cannot be rewritten as another engine.
    assert!(backend
        .save_development_engine_connection(
            Some(saved[0].id.clone()),
            DevelopmentEngineConnection::Redis(redis()),
            String::new(),
            organization.clone(),
        )
        .await
        .unwrap_err()
        .contains("cannot change engine"));
    assert!(backend
        .save_development_connection(
            Some(saved[3].id.clone()),
            DevelopmentPostgresConnection {
                name: "pg".into(),
                host: "h".into(),
                port: 5432,
                database: "d".into(),
                user: "u".into(),
                environment: DevelopmentEnvironment::Development,
                safe_mode: DevelopmentSafeMode::Protected,
                read_only: false,
                tls: Default::default(),
                driver_options: Default::default(),
                ssh_tunnel: None,
            },
            String::new(),
        )
        .await
        .is_err());

    // Duplicate copies metadata and secret; delete removes both.
    for row in &saved {
        let copy = backend
            .duplicate_development_connection(row.id.clone())
            .await
            .unwrap();
        assert_eq!(copy.engine, row.engine);
        assert_eq!(copy.name, "Edited copy");
        assert!(!copy.organization.is_favorite);
        assert!(copy.settings.is_some());
        let secrets = credentials::read_all(
            &backend.0.state.credentials,
            crate::CredentialStorageMode::PlainSqlite,
        )
        .await
        .unwrap();
        assert_eq!(secrets[&copy.id], format!("{}-secret", row.engine));
        backend
            .delete_development_connection(copy.id.clone())
            .await
            .unwrap();
        let secrets = credentials::read_all(
            &backend.0.state.credentials,
            crate::CredentialStorageMode::PlainSqlite,
        )
        .await
        .unwrap();
        assert!(!secrets.contains_key(&copy.id));
    }

    // Explicit unsaved tests: a local SQLite file, a missing one, and a
    // refused loopback port. None of them change stored metadata.
    let before = serde_json::to_string(&backend.development_connections().await.unwrap()).unwrap();
    assert!(matches!(
        backend
            .test_development_engine_connection(
                Some(saved[1].id.clone()),
                saved[1].settings.clone().unwrap(),
                String::new(),
            )
            .await
            .unwrap(),
        DevelopmentConnectionTest::Reachable { .. }
    ));
    assert!(backend
        .test_development_engine_connection(
            None,
            DevelopmentEngineConnection::SQLite(sqlite(missing.to_str().unwrap())),
            String::new(),
        )
        .await
        .is_err());
    assert!(!missing.exists());
    let mut refused = mysql();
    refused.host = "127.0.0.1".into();
    refused.port = 1;
    refused.ssl = false;
    assert!(matches!(
        backend
            .test_development_engine_connection(
                None,
                DevelopmentEngineConnection::MySQL(refused),
                "x".into(),
            )
            .await
            .unwrap(),
        DevelopmentConnectionTest::Failed { .. }
    ));
    assert_eq!(
        serde_json::to_string(&backend.development_connections().await.unwrap()).unwrap(),
        before
    );

    for row in &saved {
        backend
            .delete_development_connection(row.id.clone())
            .await
            .unwrap();
    }
    assert!(backend.development_connections().await.unwrap().is_empty());
    assert!(database.exists());
    backend.shutdown().await.unwrap();
}
