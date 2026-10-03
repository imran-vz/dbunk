use super::*;

fn text(value: &str) -> ServerText {
    ServerText::Value(value.into())
}
fn setting(name: &str) -> ServerSetting {
    ServerSetting {
        name: name.into(),
        category: "Client Connection Defaults".into(),
        source: "default".into(),
        setting: text("exact"),
        unit: ServerText::Null,
        short_desc: text("description"),
        boot_val: text("exact"),
        reset_val: text("exact"),
        inspection_override: matches!(name, "statement_timeout" | "lock_timeout"),
    }
}
fn snapshot() -> ServerDetailsSnapshot {
    ServerDetailsSnapshot {
        database: "db".into(),
        reader_pid: 1,
        collected_start: "2026-10-03T10:00:00Z".into(),
        collected_end: "2026-10-03T10:00:01Z".into(),
        reader: ReaderContext {
            current_user: text("reader_role"),
            session_user: text("login_role"),
            search_path: text("\"quoted schema\", public"),
            statement_timeout_ms: 10000,
            lock_timeout_ms: 2000,
        },
        facts: ServerSection::Loaded(ServerFacts {
            server_version: text("PostgreSQL"),
            encoding: text("UTF8"),
            locale: text("C"),
            timezone: text("UTC"),
        }),
        settings: ServerSection::Loaded(ServerRows {
            rows: vec![setting("a")],
            limit: None,
        }),
        extensions: ServerSection::Loaded(ServerRows {
            rows: vec![ServerExtension {
                name: "plpgsql".into(),
                schema: "pg_catalog".into(),
                version: text("1.0"),
                description: ServerText::Null,
            }],
            limit: None,
        }),
    }
}

#[test]
fn null_empty_omitted_and_malformed_wire_cells_remain_distinct() {
    assert!(matches!(
        reader::decode_cell(None, None).unwrap(),
        ServerText::Null
    ));
    assert!(matches!(
        reader::decode_cell(Some(""), Some(0)).unwrap(),
        ServerText::Value("")
    ));
    assert_eq!(
        reader::decode_cell(Some("日本"), Some(6)).unwrap(),
        ServerText::Value("日本")
    );
    assert_eq!(
        reader::decode_cell(None, Some(8193)).unwrap(),
        ServerText::Omitted { bytes: 8193 }
    );
    assert!(reader::decode_cell(Some(&"x".repeat(8192)), Some(8192)).is_ok());
    for (value, length) in [
        (None, Some(0)),
        (None, Some(-1)),
        (Some("x"), None),
        (Some("x"), Some(2)),
        (Some("x"), Some(8193)),
    ] {
        assert_eq!(
            reader::decode_cell(value, length).unwrap_err(),
            CatalogError::InvalidResponse
        );
    }
}

#[test]
fn loaded_empty_restricted_unavailable_and_partial_are_not_interchangeable() {
    let mut value = snapshot();
    for section in [
        ServerSection::Loaded(ServerRows {
            rows: vec![],
            limit: None,
        }),
        ServerSection::Loaded(ServerRows {
            rows: vec![],
            limit: Some(ServerLimit::ByteLimit),
        }),
        ServerSection::Restricted,
        ServerSection::Unavailable,
    ] {
        value.settings = section;
        assert!(value.checked_heap_bytes().is_some());
    }
    value.settings = ServerSection::Loaded(ServerRows {
        rows: vec![],
        limit: Some(ServerLimit::RowLimit),
    });
    assert!(
        value.checked_heap_bytes().is_none(),
        "row limit requires its actual cap"
    );
    value.settings = ServerSection::Loaded(ServerRows {
        rows: (0..MAX_SERVER_SETTINGS)
            .map(|i| setting(&format!("s{i}")))
            .collect(),
        limit: Some(ServerLimit::RowLimit),
    });
    assert!(value.checked_heap_bytes().is_some());
}

#[test]
fn borrowed_admission_accounts_escaping_and_full_backing_before_copy() {
    let (mut budget, rows) =
        bounds::RowBudget::new::<ServerExtension>(MAX_SERVER_EXTENSIONS, EXTENSIONS_BYTES).unwrap();
    assert_eq!(budget.heap, rows.capacity() * size_of::<ServerExtension>());
    let before = budget.heap;
    let escaped = "\n".repeat(MAX_SERVER_TEXT_BYTES);
    let value = ServerExtension {
        name: "example",
        schema: "public",
        version: ServerText::Value("1"),
        description: ServerText::Value(escaped.as_str()),
    };
    let size = bounds::extension(&value).unwrap();
    let mut admitted = 0;
    while budget.admit(&value, size) {
        admitted += 1;
    }
    assert!(admitted > 0 && admitted < MAX_SERVER_EXTENSIONS);
    let stopped = budget.heap;
    assert!(!budget.admit(&value, size));
    assert_eq!(
        budget.heap, stopped,
        "failed preflight must not charge or append"
    );
    assert!(budget.heap < EXTENSIONS_BYTES && budget.heap > before);
    assert!(
        rows.is_empty(),
        "admission uses borrowed metadata, no cloned row"
    );
    assert!(bounds::RowBudget::new::<ServerSetting>(MAX_SERVER_SETTINGS, 1).is_err());
}

#[test]
fn validator_checks_retained_capacity_identities_duplicate_keys_and_cell_states() {
    let valid = snapshot();
    assert!(valid.checked_heap_bytes().unwrap() < MAX_SERVER_DETAILS_BYTES);
    let mut value = valid.clone();
    value.database.reserve(MAX_SERVER_DETAILS_BYTES);
    assert!(
        value.checked_heap_bytes().is_none(),
        "retained capacity matters even for short text"
    );
    let mut value = valid.clone();
    if let ServerSection::Loaded(rows) = &mut value.settings {
        rows.rows.push(setting("a"));
    }
    assert!(value.checked_heap_bytes().is_none());
    let mut value = valid.clone();
    if let ServerSection::Loaded(rows) = &mut value.settings {
        rows.rows[0].category = "x".repeat(257);
    }
    assert!(value.checked_heap_bytes().is_none());
    let mut value = valid.clone();
    if let ServerSection::Loaded(rows) = &mut value.settings {
        rows.rows[0].setting = ServerText::Omitted { bytes: 8192 };
    }
    assert!(value.checked_heap_bytes().is_none());
    let mut value = valid;
    if let ServerSection::Loaded(rows) = &mut value.extensions {
        rows.rows.reserve(MAX_SERVER_EXTENSIONS + 1);
    }
    assert!(value.checked_heap_bytes().is_none());
}

#[test]
fn reader_limits_and_override_flags_do_not_masquerade_as_operator_changes() {
    for (configured, expected) in [
        (None, 10000),
        (Some(0), 10000),
        (Some(1), 1),
        (Some(500), 500),
        (Some(u32::MAX), 10000),
    ] {
        assert_eq!(reader::inspection_timeout(configured), expected);
    }
    let mut value = snapshot();
    if let ServerSection::Loaded(rows) = &mut value.settings {
        rows.rows = vec![
            setting("statement_timeout"),
            setting("lock_timeout"),
            setting("search_path"),
        ];
        rows.rows[0].source = "session".into();
        rows.rows[1].source = "session".into();
        rows.rows[2].source = "client".into();
    }
    assert!(value.checked_heap_bytes().is_some());
    if let ServerSection::Loaded(rows) = &mut value.settings {
        rows.rows[2].inspection_override = true;
    }
    assert!(value.checked_heap_bytes().is_none());
}

#[test]
fn debug_redacts_every_nested_setting_and_connection_value() {
    let mut value = snapshot();
    value.database = "SENSITIVE_DATABASE".into();
    if let ServerSection::Loaded(rows) = &mut value.settings {
        rows.rows[0].name = "SENSITIVE_NAME".into();
        rows.rows[0].setting = text("SENSITIVE_PASSWORD");
        assert!(!format!("{:?}", rows.rows[0]).contains("SENSITIVE"));
        assert!(!format!("{:?}", rows.rows[0].setting).contains("SENSITIVE"));
        assert!(!format!("{rows:?}").contains("SENSITIVE"));
    }
    assert!(!format!("{value:?}").contains("SENSITIVE"));
    assert!(
        serde_json::to_string(&value)
            .unwrap()
            .contains("SENSITIVE_PASSWORD"),
        "serialization carries data to the bounded explicit UI, unlike Debug"
    );
}
