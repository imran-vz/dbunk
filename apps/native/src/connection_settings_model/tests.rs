use super::*;
use dbunk_lib::backend::*;
fn record(id: &str) -> DevelopmentConnection {
    DevelopmentConnection {
        id: id.into(),
        name: "same name".into(),
        engine: "PostgreSQL".into(),
        organization: Default::default(),
        unsupported_reason: None,
        postgres: Some(DevelopmentPostgresConnection {
            name: "same name".into(),
            host: "localhost".into(),
            port: 5432,
            database: "quoted\"db".into(),
            user: "user".into(),
            environment: Default::default(),
            safe_mode: Default::default(),
            read_only: false,
            tls: Default::default(),
            driver_options: Default::default(),
            ssh_tunnel: None,
        }),
        environment: Default::default(),
        settings: None,
        last_activity_at: None,
    }
}
#[test]
fn identity_and_supported_dispatch_are_exact() {
    let mut records = vec![record("one"), record("two")];
    assert_eq!(editable_connection(&records, "two").unwrap().id, "two");
    assert!(editable_connection(&records, "gone").is_none());
    records[1].postgres = None;
    assert!(editable_connection(&records, "two").is_none());
    // Plan 031: a supported non-PostgreSQL record edits through its settings.
    records[1].engine = "SQLite".into();
    records[1].settings = Some(DevelopmentEngineConnection::SQLite(
        DevelopmentSqliteConnection {
            name: "file".into(),
            path: "/data/app.db".into(),
            environment: Default::default(),
            safe_mode: Default::default(),
            read_only: false,
        },
    ));
    assert_eq!(editable_connection(&records, "two").unwrap().id, "two");
    let capture = Capture::new(&records[1], "two", Rc::default()).unwrap();
    assert!(capture.editable());
    assert!(
        (0..capture.count())
            .any(|i| capture.details(i).as_deref() == Some("Database\n/data/app.db"))
    );
    records[1].engine = "Redis".into();
    assert!(
        editable_connection(&records, "two").is_none(),
        "engine mismatch"
    );
    assert!(Capture::new(&records[0], "two", Rc::default()).is_err());
}
#[test]
fn exact_absence_empty_zero_and_search_path_order() {
    let mut record = record("one");
    let p = record.postgres.as_mut().unwrap();
    p.tls.server_name = Some(String::new());
    p.driver_options.statement_timeout_ms = Some(0);
    p.driver_options.default_search_path = Some(vec!["a,b".into(), "a\nb".into()]);
    let c = Capture::new(&record, "one", Rc::default()).unwrap();
    let texts: Vec<_> = (0..c.count()).map(|i| c.details(i).unwrap()).collect();
    assert!(texts.contains(&"TLS server name\nConfigured: ".into()));
    assert!(texts.contains(&"Default role\nNot configured".into()));
    assert!(texts.contains(&"Statement timeout (ms)\nConfigured: 0".into()));
    assert!(texts.ends_with(&[
        "Search path entry\nPosition 1\na,b".into(),
        "Search path entry\nPosition 2\na\nb".into()
    ]));
}
#[test]
fn overlap_refuses_atomically_and_drop_releases() {
    let budget = Rc::new(Cell::new(0));
    let old = Capture::new(&record("one"), "one", budget.clone()).unwrap();
    let charged = budget.get();
    budget.set(SHARED_BYTES - charged + 1);
    let before = budget.get();
    assert!(Capture::new(&record("one"), "one", budget.clone()).is_err());
    assert_eq!(budget.get(), before);
    assert_eq!(old.id(), "one");
    budget.set(charged);
    drop(old);
    assert_eq!(budget.get(), 0);
}
#[test]
fn newline_heavy_and_oversize_refuse_before_formatting() {
    let mut record = record("one");
    record.postgres.as_mut().unwrap().tls.root_cert_path = Some("\n".repeat(40_000));
    let budget = Rc::new(Cell::new(0));
    assert!(Capture::new(&record, "one", budget.clone()).is_err());
    assert_eq!(budget.get(), 0);
    record.name = "x".repeat(TEXT_BYTES + 1);
    assert!(Capture::new(&record, "one", budget).is_err());
}

#[test]
fn replacement_accounts_for_both_captures_and_does_not_clone_spare_capacity() {
    let budget = Rc::new(Cell::new(0));
    let mut large = record("one");
    large.postgres.as_mut().unwrap().tls.root_cert_path = Some("x".repeat(4096));
    let old = Capture::new(&large, "one", budget.clone()).unwrap();
    let old_bytes = budget.get();
    let mut small = record("one");
    small.name.reserve(1024 * 1024);
    small.name = {
        let mut text = small.name;
        text.clear();
        text.push_str("renamed");
        text
    };
    let next = Capture::new(&small, "one", budget.clone()).unwrap();
    let next_bytes = budget.get() - old_bytes;
    assert!(next_bytes < old_bytes);
    assert_eq!(next.details(1).as_deref(), Some("Name\nrenamed"));
    assert_eq!(old.details(1).as_deref(), Some("Name\nsame name"));
    drop(old);
    assert_eq!(budget.get(), next_bytes);
    drop(next);
    assert_eq!(budget.get(), 0);
}

#[test]
fn unsupported_record_remains_readable_without_edit_authority() {
    let mut record = record("one");
    record.postgres = None;
    record.unsupported_reason = Some("Unsupported connection options".into());
    let capture = Capture::new(&record, "one", Rc::default()).unwrap();
    assert!(!capture.editable());
    assert_eq!(capture.details(1).as_deref(), Some("Name\nsame name"));
    assert_eq!(
        capture.details(6).as_deref(),
        Some("Unavailable\nUnsupported connection options")
    );
    assert!(capture.details(7).unwrap().contains("cannot be edited"));
}
