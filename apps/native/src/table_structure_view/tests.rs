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

#[test]
fn page_geometry_matches_section_layout_without_per_row_storage() {
    use crate::table_structure_model::Section::*;
    // Overview 1 row, Columns 3, Indexes empty, Relation grants 2, rest empty.
    let mut counts = [0; Section::ALL.len()];
    let slot = |section| Section::ALL.iter().position(|s| *s == section).unwrap();
    counts[slot(Overview)] = 1;
    counts[slot(Columns)] = 3;
    counts[slot(RelationGrants)] = 2;
    let page = Page::from_counts(counts);
    // The layout every lookup must agree with, built the obvious way.
    let mut expected = Vec::new();
    for (section, count) in Section::ALL.into_iter().zip(counts) {
        expected.push(Item::Title(section));
        if section == RelationGrants {
            expected.push(Item::Note(section));
        }
        if count > 0 {
            expected.push(Item::Head(section));
            expected.extend((0..count).map(|index| Item::Row(section, index)));
        }
    }
    assert_eq!(page.lines, expected.len());
    assert_eq!(page.rows, 6);
    for (position, item) in expected.iter().enumerate() {
        assert_eq!(page.item(position), Some(*item), "line {position}");
        assert_eq!(page.position(*item), Some(position), "{item:?}");
    }
    assert_eq!(page.item(expected.len()), None);
    assert_eq!(page.position(Item::Head(Indexes)), None);
    assert_eq!(page.position(Item::Row(Columns, 3)), None);
    assert_eq!(page.position(Item::Note(Columns)), None);
    let rows: Vec<_> = expected
        .iter()
        .filter_map(|item| match item {
            Item::Row(section, index) => Some((*section, *index)),
            _ => None,
        })
        .collect();
    for (number, (section, index)) in rows.iter().enumerate() {
        assert_eq!(page.row(number), Some((*section, *index)));
        assert_eq!(page.row_number(*section, *index), Some(number));
    }
    assert_eq!(page.row(rows.len()), None);
    assert_eq!(page.row_number(Columns, 3), None);
    assert_eq!(page.row_number(Indexes, 0), None);
}
