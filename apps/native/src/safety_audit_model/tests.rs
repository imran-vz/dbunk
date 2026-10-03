use super::*;
use dbunk_lib::backend::safety_audit::{
    MAX_AUDIT_COMMAND_BYTES, MAX_AUDIT_PAGE_ROWS, SafetyAuditClass, SafetyAuditRow,
};

fn row(id: i64) -> SafetyAuditRow {
    SafetyAuditRow {
        id,
        command: "apply_object_ddl".into(),
        classes: vec![SafetyAuditClass::Ddl],
        occurred_at: "2026-10-03T10:00:00Z".into(),
    }
}
fn page(ids: &[i64]) -> SafetyAuditPage {
    SafetyAuditPage {
        connection_id: "exact-connection".into(),
        rows: ids.iter().copied().map(row).collect(),
        next_cursor: None,
        limit: None,
    }
}

#[test]
fn capture_checks_requested_connection_without_retargeting_or_charging_refusal() {
    let budget = Rc::new(Cell::new(19));
    assert!(Capture::new(page(&[3]), "another-connection", budget.clone()).is_err());
    assert_eq!(budget.get(), 19);
    let capture = Capture::new(page(&[3]), "exact-connection", budget.clone()).unwrap();
    assert_eq!(capture.connection(), "exact-connection");
    assert!(
        capture
            .details(0)
            .unwrap()
            .contains("Connection: exact-connection")
    );
    drop(capture);
    assert_eq!(budget.get(), 19);
}

#[test]
fn stable_ids_survive_reordering_and_expired_selection_does_not_fall_back() {
    let budget = Rc::new(Cell::new(0));
    let first = Capture::new(page(&[9, 8, 7]), "exact-connection", budget.clone()).unwrap();
    let selected = first.key(1).unwrap();
    let replacement = Capture::new(page(&[8, 7]), "exact-connection", budget.clone()).unwrap();
    assert_eq!(replacement.index_for_key(selected), Some(0));
    assert_eq!(replacement.index_for_key(9), None);
    assert_eq!(replacement.key(2), None);
    assert!(replacement.details(2).is_none());
    assert_eq!(budget.get(), RETAINED_BYTES * 2);
}

#[test]
fn replacement_refusal_retains_original_and_drop_releases_only_its_lease() {
    let budget = Rc::new(Cell::new(WORKSPACE_BYTES - RETAINED_BYTES));
    let first = Capture::new(page(&[4]), "exact-connection", budget.clone()).unwrap();
    assert!(Capture::new(page(&[5]), "exact-connection", budget.clone()).is_err());
    assert_eq!(budget.get(), WORKSPACE_BYTES);
    assert_eq!(first.key(0), Some(4));
    drop(first);
    assert_eq!(budget.get(), WORKSPACE_BYTES - RETAINED_BYTES);
}

#[test]
fn malformed_and_oversized_pages_refuse_before_retaining() {
    let budget = Rc::new(Cell::new(0));
    let mut cases = vec![page(&[1, 1]), page(&[1, 2])];
    let mut oversized = page(&[1]);
    oversized.rows[0].command = "x".repeat(MAX_AUDIT_COMMAND_BYTES + 1);
    cases.push(oversized);
    let mut inflated = page(&[1]);
    inflated.connection_id.reserve(MAX_AUDIT_PAGE_BYTES);
    cases.push(inflated);
    let ids: Vec<_> = (1..=(MAX_AUDIT_PAGE_ROWS + 1) as i64).rev().collect();
    cases.push(page(&ids));
    let mut invalid_boundary = page(&[1]);
    invalid_boundary.limit = Some(SafetyAuditLimit::ByteLimit);
    cases.push(invalid_boundary);
    for candidate in cases {
        assert!(Capture::new(candidate, "exact-connection", budget.clone()).is_err());
        assert_eq!(budget.get(), 0);
    }
}

#[test]
fn details_preserve_command_classes_time_and_disclose_scope_without_full_audit_claim() {
    let mut data = page(&[2]);
    data.rows[0].classes = vec![SafetyAuditClass::Ddl, SafetyAuditClass::Session];
    let capture = Capture::new(data, "exact-connection", Rc::new(Cell::new(0))).unwrap();
    let details = capture.details(0).unwrap();
    for expected in [
        "Record ID: 2",
        "apply_object_ddl",
        "2026-10-03T10:00:00Z",
        "ddl, session",
        "across all connections",
        "not a complete security audit",
    ] {
        assert!(details.contains(expected));
    }
    assert!(capture.row_label(0).unwrap().contains("ddl, session"));
    let empty = Capture::new(page(&[]), "exact-connection", Rc::new(Cell::new(0))).unwrap();
    assert_eq!(empty.count(), 0);
    assert!(empty.next_cursor().is_none());
    assert!(empty.empty_label().contains("page range"));
    assert!(boundary(Some(SafetyAuditLimit::RowLimit), true).contains("row limit"));
    assert!(boundary(Some(SafetyAuditLimit::ByteLimit), true).contains("byte limit"));
    assert!(boundary(None, true).contains("continuation"));
    assert!(boundary(None, false).contains("no further cursor"));
}
