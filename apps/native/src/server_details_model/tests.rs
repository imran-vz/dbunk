use super::*;
use dbunk_lib::backend::server_details::{ReaderContext, ServerFacts};

fn value(text: &str) -> ServerText {
    ServerText::Value(text.into())
}
fn setting(name: &str, source: &str) -> ServerSetting {
    ServerSetting {
        name: name.into(),
        category: "Resource Usage".into(),
        source: source.into(),
        setting: value("SECRET_SETTING"),
        unit: ServerText::Null,
        short_desc: value("Description İstanbul"),
        boot_val: value("SECRET_BOOT"),
        reset_val: value("SECRET_RESET"),
        inspection_override: matches!(name, "statement_timeout" | "lock_timeout"),
    }
}
fn snapshot() -> ServerDetailsSnapshot {
    ServerDetailsSnapshot {
        database: "postgres".into(),
        reader_pid: 42,
        collected_start: "2026-10-03T00:00:00Z".into(),
        collected_end: "2026-10-03T00:00:01Z".into(),
        reader: ReaderContext {
            current_user: value("reader"),
            session_user: value("session"),
            search_path: value("public"),
            statement_timeout_ms: 10_000,
            lock_timeout_ms: 2000,
        },
        facts: LoadedSection::Loaded(ServerFacts {
            server_version: value("PostgreSQL"),
            encoding: value("UTF8"),
            locale: ServerText::Null,
            timezone: ServerText::Omitted { bytes: 9000 },
        }),
        settings: LoadedSection::Loaded(ServerRows {
            rows: vec![
                setting("alpha", "default"),
                setting("beta", "configuration file"),
                setting("statement_timeout", "session"),
            ],
            limit: None,
        }),
        extensions: LoadedSection::Loaded(ServerRows {
            rows: vec![ServerExtension {
                name: "plpgsql".into(),
                schema: "pg_catalog".into(),
                version: value("1.0"),
                description: ServerText::Null,
            }],
            limit: None,
        }),
    }
}
fn capture() -> Capture {
    Capture::new(snapshot(), Rc::new(Cell::new(0))).unwrap()
}

#[test]
fn null_omitted_restricted_and_unavailable_are_not_empty_success() {
    let capture = capture();
    assert_eq!(capture.count(ServerSection::Facts), 4);
    assert!(
        capture
            .details(ServerSection::Facts, 2)
            .unwrap()
            .contains("Database LC_COLLATE:\nNULL")
    );
    assert!(
        capture
            .details(ServerSection::Facts, 3)
            .unwrap()
            .contains("Omitted (9000 bytes")
    );
    let mut data = snapshot();
    data.facts = LoadedSection::Restricted;
    data.extensions = LoadedSection::Unavailable;
    let capture = Capture::new(data, Rc::new(Cell::new(0))).unwrap();
    assert_eq!(capture.count(ServerSection::Facts), 0);
    assert!(
        capture
            .empty_label(ServerSection::Facts)
            .contains("restricted")
    );
    assert!(
        capture
            .empty_label(ServerSection::Extensions)
            .contains("unavailable")
    );
    assert!(capture.key(ServerSection::Facts, 0).is_none());
}

#[test]
fn non_default_source_excludes_reader_overrides_and_values_are_never_searched() {
    let mut capture = capture();
    capture.set_filter("", true).unwrap();
    assert_eq!(capture.count(ServerSection::Settings), 1);
    assert_eq!(capture.key(ServerSection::Settings, 0), Some("beta"));
    for query in ["SECRET_SETTING", "SECRET_BOOT", "SECRET_RESET"] {
        capture.set_filter(query, false).unwrap();
        assert_eq!(capture.count(ServerSection::Settings), 0);
    }
    for query in ["İSTANBUL", "resource usage", "CONFIGURATION FILE", "Beta"] {
        capture.set_filter(query, false).unwrap();
        assert!(
            capture
                .index_for_key(ServerSection::Settings, "beta")
                .is_some()
        );
    }
    capture.set_filter("statement_timeout", false).unwrap();
    assert!(
        capture
            .row_label(ServerSection::Settings, 0)
            .unwrap()
            .contains("inspection override")
    );
    assert!(
        capture
            .details(ServerSection::Settings, 0)
            .unwrap()
            .contains("not SQL-tab or server-wide")
    );
}

#[test]
fn filtered_empty_keeps_partial_scope_and_stable_selected_identity() {
    let mut data = snapshot();
    if let LoadedSection::Loaded(rows) = &mut data.settings {
        rows.limit = Some(ServerLimit::ByteLimit);
    }
    let mut capture = Capture::new(data, Rc::new(Cell::new(0))).unwrap();
    let selected = capture.key(ServerSection::Settings, 1).unwrap().to_owned();
    capture.set_filter("beta", false).unwrap();
    assert_eq!(
        capture.index_for_key(ServerSection::Settings, &selected),
        Some(0)
    );
    capture.set_filter("alpha", false).unwrap();
    assert_eq!(
        capture.index_for_key(ServerSection::Settings, &selected),
        None
    );
    capture.set_filter("no such setting", false).unwrap();
    assert_eq!(capture.count(ServerSection::Settings), 0);
    assert!(
        capture
            .empty_label(ServerSection::Settings)
            .contains("incomplete")
    );
    assert!(
        capture
            .limits(ServerSection::Settings)
            .contains("byte limit")
    );
    capture.set_filter("", false).unwrap();
    assert_eq!(
        capture.index_for_key(ServerSection::Settings, &selected),
        Some(1)
    );
}

#[test]
fn invalid_search_preserves_last_applied_filter_and_budget() {
    let mut capture = capture();
    capture.set_filter("beta", true).unwrap();
    let bytes = capture.budget.get();
    for query in [
        "x".repeat(MAX_SEARCH_BYTES + 1),
        "a\nb".into(),
        "界".repeat(342),
    ] {
        assert!(capture.set_filter(&query, false).is_err());
        assert_eq!(capture.query(), "beta");
        assert!(capture.non_default());
        assert_eq!(capture.key(ServerSection::Settings, 0), Some("beta"));
        assert_eq!(capture.budget.get(), bytes);
    }
    capture
        .set_filter(&"x".repeat(MAX_SEARCH_BYTES), false)
        .unwrap();
}

#[test]
fn replacement_admission_keeps_old_capture_and_releases_each_lease() {
    let budget = Rc::new(Cell::new(WORKSPACE_BYTES - RETAINED_BYTES));
    let original = Capture::new(snapshot(), budget.clone()).unwrap();
    assert_eq!(budget.get(), WORKSPACE_BYTES);
    assert!(Capture::new(snapshot(), budget.clone()).is_err());
    assert_eq!(original.count(ServerSection::Facts), 4);
    assert_eq!(budget.get(), WORKSPACE_BYTES);
    drop(original);
    assert_eq!(budget.get(), WORKSPACE_BYTES - RETAINED_BYTES);
    budget.set(0);
    let first = Capture::new(snapshot(), budget.clone()).unwrap();
    let second = Capture::new(snapshot(), budget.clone()).unwrap();
    assert_eq!(budget.get(), 2 * RETAINED_BYTES);
    drop(first);
    drop(second);
    assert_eq!(budget.get(), 0);
}

#[test]
fn malformed_identity_and_excess_capacity_refuse_before_admission() {
    let budget = Rc::new(Cell::new(17));
    let mut duplicate = snapshot();
    if let LoadedSection::Loaded(rows) = &mut duplicate.settings {
        rows.rows[1].name = "alpha".into();
    }
    assert!(Capture::new(duplicate, budget.clone()).is_err());
    let mut inflated = snapshot();
    inflated.database.reserve(2 * MAX_SERVER_DETAILS_BYTES);
    assert!(Capture::new(inflated, budget.clone()).is_err());
    assert_eq!(budget.get(), 17);
}
