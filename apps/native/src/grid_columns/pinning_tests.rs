use super::*;

#[test]
fn all_hidden_recovery_requires_show_all_before_pin_changes() {
    let mut columns = columns();
    columns
        .load(Some(TableGridPrefs(json!({
            "version":1, "hiddenColumns":["id","value","amount","tail"], "pinnedColumns":["id"]
        }))))
        .unwrap();
    assert_eq!(columns.len(), 1);
    let before = columns.prefs();
    let error = columns
        .patch(Some(0), ColumnAction::TogglePin)
        .err()
        .unwrap();
    assert!(error.contains("Show all columns"));
    assert_eq!(columns.prefs(), before);
    let shown = columns.change(None, ColumnAction::ShowAll).unwrap();
    assert_eq!(shown.pinned_count(), 1);
    let unpinned = shown.change(Some(0), ColumnAction::TogglePin).unwrap();
    assert_eq!(unpinned.pinned_count(), 0);
}

fn columns() -> GridColumns {
    let mut columns = GridColumns::default();
    columns.columns(
        ["id", "value", "amount", "tail"]
            .into_iter()
            .map(str::to_owned),
    );
    columns
}
fn sources(columns: &GridColumns) -> Vec<usize> {
    (0..columns.len())
        .map(|display| columns.source(display).unwrap())
        .collect()
}
#[test]
fn pins_append_in_order_and_unpin_restores_underlying_column_order() {
    let columns = columns().change(Some(2), ColumnAction::Left).unwrap();
    assert_eq!(sources(&columns), [0, 2, 1, 3]);
    let columns = columns
        .change(Some(2), ColumnAction::TogglePin)
        .unwrap()
        .change(Some(3), ColumnAction::TogglePin)
        .unwrap();
    assert_eq!(sources(&columns), [1, 3, 0, 2]);
    assert_eq!(columns.pinned_count(), 2);
    let columns = columns.change(Some(0), ColumnAction::TogglePin).unwrap();
    assert_eq!(sources(&columns), [3, 0, 2, 1]);
    let columns = columns.change(Some(0), ColumnAction::TogglePin).unwrap();
    assert_eq!(sources(&columns), [0, 2, 1, 3]);
    assert_eq!(columns.pinned_count(), 0);
}
#[test]
fn pinned_hide_show_resize_and_group_reorder_preserve_source_mapping() {
    let columns = columns()
        .change(Some(2), ColumnAction::TogglePin)
        .unwrap()
        .change(Some(2), ColumnAction::TogglePin)
        .unwrap();
    assert_eq!(sources(&columns), [2, 1, 0, 3]);
    let columns = columns
        .change(Some(1), ColumnAction::Left)
        .unwrap()
        .change(Some(0), ColumnAction::Widen)
        .unwrap();
    assert_eq!(sources(&columns), [1, 2, 0, 3]);
    assert_eq!(columns.width(0), 192.);
    assert_eq!(columns.offset(2), 352.);
    // Neither boundary move implicitly pins/unpins or changes the other group.
    assert_eq!(
        sources(&columns.change(Some(1), ColumnAction::Right).unwrap()),
        sources(&columns)
    );
    assert_eq!(
        sources(&columns.change(Some(2), ColumnAction::Left).unwrap()),
        sources(&columns)
    );
    let hidden = columns.change(Some(0), ColumnAction::Hide).unwrap();
    assert_eq!(hidden.pinned_count(), 1);
    assert_eq!(
        hidden.prefs().0["pinnedColumns"],
        json!(["value", "amount"])
    );
    let shown = hidden.change(None, ColumnAction::ShowAll).unwrap();
    assert_eq!(sources(&shown), [1, 2, 0, 3]);
    assert_eq!(shown.width(0), 192.);
    let moved = shown.change(Some(2), ColumnAction::Right).unwrap();
    assert_eq!(sources(&moved), [1, 2, 3, 0]);
    assert_eq!(moved.prefs().0["pinnedColumns"], json!(["value", "amount"]));
}
#[test]
fn absent_and_duplicate_preferences_never_duplicate_or_misidentify_source_columns() {
    let mut columns = columns();
    columns.load(Some(TableGridPrefs(json!({"version":1,"pinnedColumns":["gone","value","value"],"hiddenColumns":["value"],"future":{"keep":true}})))).unwrap();
    assert_eq!(columns.pinned_count(), 0);
    assert_eq!(sources(&columns), [0, 2, 3]);
    columns = columns.change(None, ColumnAction::ShowAll).unwrap();
    assert_eq!(sources(&columns), [1, 0, 2, 3]);
    assert_eq!(columns.pinned_count(), 1);
    assert_eq!(columns.prefs().0["future"], json!({"keep":true}));
    assert_eq!(
        columns.prefs().0["pinnedColumns"],
        json!(["gone", "value", "value"])
    );
    columns.columns(["value", "value", "gone"].into_iter().map(str::to_owned));
    assert_eq!(sources(&columns), [2, 0, 1]);
    assert_eq!(columns.pinned_count(), 1);
    assert!(columns.change(Some(1), ColumnAction::TogglePin).is_err());
    assert!(columns.change(Some(1), ColumnAction::Right).is_err());
    assert!(columns.change(None, ColumnAction::TogglePin).is_err());
    assert!(columns.change(Some(99), ColumnAction::TogglePin).is_err());
}
#[test]
fn malformed_pins_and_overflow_refuse_without_replacing_valid_layout() {
    let mut columns = columns();
    let initial = columns.prefs();
    for pins in [
        json!("id"),
        json!([3]),
        json!(["x".repeat(64)]),
        json!(["a\0b"]),
    ] {
        assert!(
            columns
                .load(Some(TableGridPrefs(
                    json!({"version":1,"pinnedColumns":pins})
                )))
                .is_err()
        );
        assert_eq!(columns.prefs(), initial);
    }
    let mut prefs = json!({"version":1,"future":""});
    let overhead = crate::results::encoded_size(&prefs);
    prefs["future"] = json!("x".repeat(PREFS_BYTES - overhead));
    columns.load(Some(TableGridPrefs(prefs))).unwrap();
    let original = columns.prefs();
    assert!(columns.change(Some(0), ColumnAction::TogglePin).is_err());
    assert_eq!(columns.prefs(), original);
}
