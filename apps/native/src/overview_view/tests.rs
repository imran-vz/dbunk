use super::*;
#[test]
fn overview_scope_field_admission_preserves_existing_retention() {
    let budget = Rc::new(Cell::new(127 * 1024 * 1024));
    assert!(FieldLease::new(budget.clone()).is_none());
    assert_eq!(budget.get(), 127 * 1024 * 1024);
    budget.set(0);
    let old = FieldLease::new(budget.clone()).unwrap();
    let new = FieldLease::new(budget.clone()).unwrap();
    assert_eq!(budget.get(), 2 * FIELD_BYTES);
    drop(old);
    assert_eq!(budget.get(), FIELD_BYTES);
    drop(new);
    assert_eq!(budget.get(), 0);
}
#[test]
fn keyboard_rows_and_scope_tabs_stop_at_exact_bounds() {
    for key in ["up", "down", "pageup", "pagedown", "home", "end"] {
        assert_eq!(actions::move_index(None, 0, key), None);
    }
    assert_eq!(actions::move_index(None, 256, "down"), Some(0));
    assert_eq!(actions::move_index(Some(250), 256, "pagedown"), Some(255));
    assert_eq!(actions::move_index(Some(4), 256, "pageup"), Some(0));
    assert_eq!(actions::move_tab(0, 3, "left"), Some(0));
    assert_eq!(actions::move_tab(2, 3, "right"), Some(2));
    assert_eq!(actions::move_tab(3, 3, "left"), None);
}
#[test]
fn runtime_status_clipping_preserves_utf8_and_exact_short_errors() {
    assert_eq!(
        bounded_status("Permission restricted"),
        "Permission restricted"
    );
    let clipped = bounded_status(&"雪".repeat(2000));
    assert!(clipped.len() <= 4096 + " [truncated]".len());
    assert!(clipped.ends_with(" [truncated]"));
}
