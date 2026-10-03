use super::*;

fn session() -> AdminSession {
    AdminSession {
        pid: 42,
        user: Some("reader".into()),
        database: Some("db".into()),
        application_name: None,
        client_addr: None,
        state: None,
        wait_event_type: None,
        wait_event: None,
        query_age_seconds: None,
        transaction_age_seconds: None,
        query: Some("private SQL".into()),
        query_clipped: false,
        details_restricted: false,
        backend_start: None,
        query_start: None,
        xact_start: None,
    }
}
fn lock() -> AdminLock {
    AdminLock {
        pid: None,
        lock_type: "transactionid".into(),
        relation: None,
        mode: "ExclusiveLock".into(),
        granted: true,
        blocked_by: vec![],
        blocked_by_clipped: false,
        blocked_by_unavailable: true,
        query: None,
        query_clipped: false,
        details_restricted: false,
        backend_start: None,
        query_start: None,
    }
}
#[test]
fn metrics_preserve_null_zero_restricted_and_unavailable() {
    assert_eq!(counter(None, false).unwrap(), AdminMetric::Null);
    assert_eq!(counter(Some(0), false).unwrap(), AdminMetric::Value(0));
    assert_eq!(
        counter(Some(i64::MAX), false).unwrap(),
        AdminMetric::Value(i64::MAX)
    );
    assert_eq!(counter(Some(0), true).unwrap(), AdminMetric::Restricted);
    assert_eq!(
        AdminStats::unavailable().active_sessions,
        AdminMetric::Unavailable
    );
    assert_eq!(
        AdminStats::restricted().active_sessions,
        AdminMetric::Restricted
    );
    assert!(counter(Some(-1), false).is_err());
    assert_eq!(ratio(None).unwrap(), AdminMetric::Null);
    assert_eq!(ratio(Some(0.0)).unwrap(), AdminMetric::Value(0.0));
    for value in [f64::NAN, f64::INFINITY, -0.01, 1.01] {
        assert!(ratio(Some(value)).is_err());
    }
    let encoded = serde_json::to_string(&counter(Some(i64::MAX), false).unwrap()).unwrap();
    assert!(encoded.contains("9223372036854775807"));
}
#[test]
fn query_clipping_preserves_unicode_and_distinguishes_missing_text() {
    let text = "雪".repeat(MAX_ADMIN_QUERY_CHARS);
    assert_eq!(
        query_text(Some(&text), false, true).unwrap().as_deref(),
        Some(text.as_str())
    );
    assert!(query_text(Some(&(text + "x")), false, true).is_err());
    assert_eq!(query_text(None, false, false).unwrap(), None);
    assert_eq!(
        query_text(Some(""), false, false).unwrap(),
        Some(String::new())
    );
    assert_eq!(
        query_text(Some("<insufficient privilege>"), true, false).unwrap(),
        None
    );
    assert!(query_text(None, false, true).is_err());
    assert!(query_text(Some("short"), false, true).is_err());
}
#[test]
fn blockers_preserve_zero_prepared_transaction_and_refuse_invalid_clipping() {
    assert!(validate_blockers(Some(&[0, 1]), false).is_ok());
    assert!(validate_blockers(None, false).is_ok());
    assert!(validate_blockers(None, true).is_err());
    assert!(validate_blockers(Some(&[-1]), false).is_err());
    assert!(validate_blockers(Some(&[1; MAX_ADMIN_BLOCKERS]), true).is_ok());
    assert!(validate_blockers(Some(&[1; MAX_ADMIN_BLOCKERS + 1]), false).is_err());
    assert!(validate_blockers(Some(&[1]), true).is_err());
}
#[test]
fn each_section_discloses_only_observed_overflow() {
    let mut builder = Builder::new();
    for _ in 0..MAX_ADMIN_ROWS {
        assert!(builder.session(session(), false).unwrap());
        assert!(builder.session(session(), true).unwrap());
        assert!(builder.lock(lock()).unwrap());
    }
    assert!(!builder.snapshot.sessions_truncated);
    assert!(!builder.snapshot.pending_transactions_truncated);
    assert!(!builder.snapshot.locks_truncated);
    assert!(!builder.session(session(), false).unwrap());
    assert!(!builder.session(session(), true).unwrap());
    assert!(!builder.lock(lock()).unwrap());
    let snapshot = builder.finish().unwrap();
    assert_eq!(snapshot.sessions.len(), MAX_ADMIN_ROWS);
    assert_eq!(snapshot.pending_transactions.len(), MAX_ADMIN_ROWS);
    assert_eq!(snapshot.locks.len(), MAX_ADMIN_ROWS);
    assert!(
        snapshot.sessions_truncated
            && snapshot.pending_transactions_truncated
            && snapshot.locks_truncated
    );
    assert!(encoded(&snapshot).unwrap() <= MAX_ADMIN_BYTES);
    assert!(!format!("{snapshot:?}").contains("private SQL"));
}
#[test]
fn escaped_bytes_are_charged_before_retaining_a_row() {
    let mut builder = Builder::new();
    let mut item = session();
    item.query = Some("\u{0001}".repeat(MAX_ADMIN_QUERY_CHARS));
    let size = encoded(&item).unwrap();
    assert!(size > 6 * MAX_ADMIN_QUERY_CHARS);
    builder.charged = MAX_ADMIN_BYTES - size + 1;
    assert!(matches!(
        builder.session(item, false),
        Err(CatalogError::AdminLimit)
    ));
    assert!(builder.snapshot.sessions.is_empty());
}
#[test]
fn combined_sections_refuse_byte_overflow_without_a_successful_partial_snapshot() {
    let mut builder = Builder::new();
    let mut item = session();
    item.query = Some("\u{0001}".repeat(MAX_ADMIN_QUERY_CHARS));
    item.application_name = Some("\u{0001}".repeat(MAX_ADMIN_TEXT_BYTES));
    let mut refused = false;
    for _ in 0..MAX_ADMIN_ROWS {
        match builder.session(item.clone(), false) {
            Err(CatalogError::AdminLimit) => {
                refused = true;
                break;
            }
            Ok(true) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(refused);
    assert!(!builder.snapshot.sessions_truncated);
}
