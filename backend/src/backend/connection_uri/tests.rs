use super::*;
use crate::backend::{DevelopmentEnvironment, DevelopmentSafeMode, DevelopmentTlsOptions};

fn input() -> DevelopmentPostgresConnection {
    DevelopmentPostgresConnection {
        name: "Never encoded".into(),
        host: "db.internal".into(),
        port: 5433,
        user: "app_user".into(),
        database: "orders".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: true,
        tls: DevelopmentTlsOptions {
            mode: DevelopmentTlsMode::Prefer,
            ..Default::default()
        },
        driver_options: Default::default(),
        ssh_tunnel: None,
    }
}

#[test]
fn baseline_postgres_defaults_password_and_component_encoding() {
    let built = build_postgres_uri(&input()).unwrap();
    assert_eq!(built.uri, "postgres://app_user@db.internal:5433/orders");
    assert_eq!(built.omissions, UriOmissions::default());
    let parsed = parse_postgres_uri(" postgresql://admin:s%40cret@10.0.0.1/app ").unwrap();
    assert_eq!(
        (&*parsed.host, parsed.port, &*parsed.user, &*parsed.database),
        ("10.0.0.1", 5432, "admin", "app")
    );
    assert_eq!(parsed.password.as_deref(), Some("s@cret"));
    assert_eq!(parsed.tls_mode, None);
    let parsed =
        parse_postgres_uri("POSTGRES://u%2Bname:p%2Bass+word@h/sales%20db%2Farchive").unwrap();
    assert_eq!(parsed.user, "u+name");
    assert_eq!(parsed.password.as_deref(), Some("p+ass+word"));
    assert_eq!(parsed.database, "sales db/archive");
    let mut connection = input();
    connection.user = "a !'()*~_-.+@:/?#%界".into();
    connection.database = "db/😀".into();
    assert_eq!(
        build_postgres_uri(&connection).unwrap().uri,
        "postgres://a%20!'()*~_-.%2B%40%3A%2F%3F%23%25%E7%95%8C@db.internal:5433/db%2F%F0%9F%98%80"
    );
    connection.user.clear();
    connection.database.clear();
    connection.host.clear();
    connection.port = 0;
    assert_eq!(
        build_postgres_uri(&connection).unwrap().uri,
        "postgres://localhost:5432"
    );
    for uri in ["postgres://u@h/db", "postgres://u:@h/db"] {
        assert!(parse_postgres_uri(uri).unwrap().password.is_none());
    }
}

#[test]
fn export_round_trips_exact_identifiers_hosts_and_raw_dot_database_names() {
    for host in [
        "DB.example",
        "127.0.0.1",
        "::1",
        "[2001:db8::1]",
        "münchen.example",
    ] {
        for name in ["", ".", "..", "a/b", "a?b#c", "name%20literal", "未処理😀"] {
            let mut connection = input();
            connection.host = host.into();
            connection.user = name.into();
            connection.database = name.into();
            let exported = build_postgres_uri(&connection).unwrap();
            let parsed = parse_postgres_uri(&exported.uri).unwrap();
            assert_eq!(parsed.user, name);
            assert_eq!(
                parsed.database, name,
                "raw database must not undergo URL dot-segment normalization"
            );
            assert_eq!(parsed.host, host.trim_matches(['[', ']']));
            assert_eq!(parsed.port, connection.port);
            assert!(parsed.password.is_none());
        }
    }
    assert_eq!(
        parse_postgres_uri("postgres://u@[::1]:5432/db")
            .unwrap()
            .host,
        "::1"
    );
}

#[test]
fn only_exact_sslmode_applies_and_ignored_keys_are_ordered_and_unique() {
    for (mode, name) in [
        (DevelopmentTlsMode::Disable, "disable"),
        (DevelopmentTlsMode::Prefer, "prefer"),
        (DevelopmentTlsMode::Require, "require"),
        (DevelopmentTlsMode::VerifyCa, "verify-ca"),
        (DevelopmentTlsMode::VerifyFull, "verify-full"),
    ] {
        let parsed =
            parse_postgres_uri(&format!("postgres://u@h/db?sslmode={name}&sslmode={name}"))
                .unwrap();
        assert_eq!(parsed.tls_mode, Some(mode));
        assert!(parsed.ignored_params.is_empty());
        let mut connection = input();
        connection.tls.mode = mode;
        let exported = build_postgres_uri(&connection).unwrap();
        if mode == DevelopmentTlsMode::Prefer {
            assert!(!exported.uri.contains('?'));
        } else {
            assert!(exported.uri.ends_with(&format!("?sslmode={name}")));
        }
    }
    let parsed = parse_postgres_uri("postgres://u@h/db?sslmode=allow&sslrootcert=/private/ca&sslcert=cert&sslkey=key&connect_timeout=5&sslrootcert=again&password=not-applied&search+path=secret").unwrap();
    assert_eq!(parsed.tls_mode, None);
    assert!(parsed.password.is_none());
    assert_eq!(
        parsed.ignored_params,
        [
            "sslmode",
            "sslrootcert",
            "sslcert",
            "sslkey",
            "connect_timeout",
            "password",
            "search path"
        ]
    );
    assert_eq!(
        parse_postgres_uri("postgres://u@h/db?sslmode=REQUIRE")
            .unwrap()
            .ignored_params,
        ["sslmode"]
    );
    for modes in [
        "require&sslmode=disable",
        "require&sslmode=unknown",
        "unknown&sslmode=other",
    ] {
        assert_eq!(
            parse_postgres_uri(&format!("postgres://h/db?sslmode={modes}")).unwrap_err(),
            UriError::ConflictingTlsModes
        );
    }
    assert_eq!(
        parse_postgres_uri("postgres://h/db?sslmode=unknown&sslmode=unknown")
            .unwrap()
            .ignored_params,
        ["sslmode"]
    );
}

#[test]
fn ambiguous_or_unsupported_uri_shapes_refuse_without_endpoint_normalization() {
    for uri in ["postgres://", "postgres:///db", "postgres://u@/db"] {
        assert_eq!(parse_postgres_uri(uri).unwrap_err(), UriError::MissingHost);
    }
    for uri in [
        "postgres://a,b/db",
        "postgres://h:5432,h2:5433/db",
        "postgres://%2Ftmp/db",
        "postgres://[::1%25eth0]/db",
        "postgres://h:0/db",
        "postgres://h:65536/db",
        "postgres://h:/db",
        "postgres://h:+1/db",
        "postgres://::1/db",
        "postgres://h/a/b",
        "postgres://h/db/",
        "postgres://h/db#fragment",
        "postgres://h/db#",
        "postgres://h/db%",
        "postgres://h/%ZZ",
        "postgres://h/%FF",
        "postgres://%C3@h/db",
        "postgres://h/db?ignored=%F0%80%80%80",
        "postgres://h/db%00",
        "postgres://h/db?bad%0Akey=x",
        "postgres://h/db?bad=x%7F",
        "postgres://h/db?bad=%C2%85",
        "\npostgres://h/db",
        "postgres://h/\tdb",
        "postgres://h/db\r",
        "mysql://h/db",
        "host=h dbname=db",
        "postgres:h/db",
    ] {
        assert!(parse_postgres_uri(uri).is_err(), "must refuse {uri:?}");
    }
    for host in [
        "a>b",
        "a<b",
        "a^b",
        "a|b",
        "a b",
        "u@h",
        "h/path",
        "h?query",
        "h#fragment",
        "a,b",
        "h:5432",
        "%2Ftmp",
        "[h]",
        "::1%eth0",
        "h\n",
    ] {
        let mut connection = input();
        connection.host = host.into();
        assert!(
            build_postgres_uri(&connection).is_err(),
            "must refuse host {host:?}"
        );
    }
}

#[test]
fn parser_and_export_bounds_refuse_atomically_at_byte_limits() {
    let prefix = "postgres://h/db?ignored=";
    let maximum = format!("{prefix}{}", "x".repeat(MAX_URI_BYTES - prefix.len()));
    assert!(parse_postgres_uri(&maximum).is_ok());
    assert_eq!(
        parse_postgres_uri(&(maximum + "x")).unwrap_err(),
        UriError::TooLarge
    );
    for size in [256, 257] {
        let name = "x".repeat(size);
        for uri in [
            format!("postgres://{name}@h/db"),
            format!("postgres://h/{name}"),
            format!("postgres://{name}/db"),
        ] {
            assert_eq!(parse_postgres_uri(&uri).is_ok(), size == 256);
        }
        let mut connection = input();
        connection.user = name.clone();
        connection.database = name;
        assert_eq!(build_postgres_uri(&connection).is_ok(), size == 256);
    }
    for (password, expected) in [
        ("x".repeat(4096), true),
        ("界".repeat(1366), false),
        ("x".repeat(4097), false),
    ] {
        assert_eq!(
            parse_postgres_uri(&format!("postgres://u:{password}@h/db")).is_ok(),
            expected
        );
    }
    let keys = (0..32)
        .map(|i| format!("k{i}=value"))
        .collect::<Vec<_>>()
        .join("&");
    assert_eq!(
        parse_postgres_uri(&format!("postgres://h/db?{keys}"))
            .unwrap()
            .ignored_params
            .len(),
        32
    );
    assert_eq!(
        parse_postgres_uri(&format!("postgres://h/db?{keys}&more=1")).unwrap_err(),
        UriError::TooManyQueryParameters
    );
    for (key, expected) in [("%61".repeat(128), true), ("%61".repeat(129), false)] {
        assert_eq!(
            parse_postgres_uri(&format!("postgres://h/db?{key}=x")).is_ok(),
            expected
        );
    }
}

#[test]
fn secrets_never_enter_export_omission_flags_debug_or_errors() {
    let parsed = parse_postgres_uri(
        "postgres://SENTINEL_USER:SENTINEL_PASSWORD@h/SENTINEL_DB?ignored=SENTINEL_VALUE",
    )
    .unwrap();
    let debug = format!("{parsed:?}");
    assert!(!debug.contains("SENTINEL"));
    let error =
        parse_postgres_uri("postgres://u:SENTINEL_PASSWORD@h/db#SENTINEL_FRAGMENT").unwrap_err();
    assert!(!format!("{error:?}: {error}").contains("SENTINEL"));
    let mut connection = input();
    connection.tls.root_cert_path = Some("SENTINEL_CA".into());
    connection.tls.client_cert_path = Some("SENTINEL_CERT".into());
    connection.tls.client_key_path = Some("SENTINEL_KEY".into());
    connection.tls.server_name = Some("SENTINEL_SERVER".into());
    connection.driver_options.default_role = Some("SENTINEL_ROLE".into());
    let exported = build_postgres_uri(&connection).unwrap();
    assert_eq!(
        exported.omissions,
        UriOmissions {
            tls_files: true,
            tls_server_name: true,
            driver_options: true
        }
    );
    assert!(!format!("{exported:?}").contains("SENTINEL"));
    assert_eq!(exported.uri, "postgres://app_user@db.internal:5433/orders");
}
