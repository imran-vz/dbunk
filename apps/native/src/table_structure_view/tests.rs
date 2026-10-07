use super::*;

#[test]
fn list_navigation_is_bounded_and_never_creates_an_empty_selection() {
    for key in ["up", "down", "pageup", "pagedown", "home", "end"] {
        assert_eq!(move_index(None, 0, key), None);
    }
    assert_eq!(move_index(Some(0), 30, "up"), Some(0));
    assert_eq!(move_index(None, 30, "down"), Some(0));
    assert_eq!(move_index(Some(28), 30, "pagedown"), Some(29));
    assert_eq!(move_index(Some(29), 30, "pageup"), Some(9));
    assert_eq!(move_index(Some(100), 30, "end"), Some(29));
    assert_eq!(move_index(Some(29), 30, "home"), Some(0));
    assert_eq!(move_index(Some(1), 30, "a"), None);
}

#[test]
fn section_arrows_preserve_order_and_stop_at_the_edges() {
    assert_eq!(move_section(0, 13, "left"), Some(0));
    assert_eq!(move_section(0, 13, "right"), Some(1));
    assert_eq!(move_section(12, 13, "right"), Some(12));
    assert_eq!(move_section(12, 13, "left"), Some(11));
    assert_eq!(move_section(13, 13, "right"), None);
    assert_eq!(move_section(0, 0, "right"), None);
    // The outline is vertical: up/down mirror left/right.
    assert_eq!(move_section(3, 13, "up"), Some(2));
    assert_eq!(move_section(3, 13, "down"), Some(4));
    assert_eq!(move_section(12, 13, "down"), Some(12));
}

#[test]
fn long_runtime_status_is_bounded_without_splitting_utf8() {
    let status = "雪".repeat(2000);
    let bounded = bounded_status(&status);
    assert!(bounded.len() <= 4096 + " [truncated]".len());
    assert!(bounded.ends_with(" [truncated]"));
    assert_eq!(bounded_status("Exact error"), "Exact error");
}
