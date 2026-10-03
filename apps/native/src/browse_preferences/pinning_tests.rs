use super::*;
#[test]
fn pin_patches_merge_latest_pins_and_are_idempotent_desired_state() {
    let patch = PreferencePatch::PinColumn {
        selected: "id".into(),
        pinned: true,
    };
    let latest = TableGridPrefs(
        json!({"version":1,"pinnedColumns":["absent","value"],"hiddenColumns":["absent"],"columnOrder":["value","id"],"columnWidths":{"id":199},"future":{"keep":true},"rawFilterText":"id > 10"}),
    );
    let saved = patch.apply(Some(latest), "now").unwrap();
    assert_eq!(saved.0["pinnedColumns"], json!(["absent", "value", "id"]));
    assert_eq!(patch.apply(Some(saved.clone()), "later").unwrap(), saved);
    let unpin = PreferencePatch::PinColumn {
        selected: "value".into(),
        pinned: false,
    };
    let saved = unpin.apply(Some(saved), "later").unwrap();
    assert_eq!(saved.0["pinnedColumns"], json!(["absent", "id"]));
    assert_eq!(saved.0["hiddenColumns"], json!(["absent"]));
    assert_eq!(saved.0["columnOrder"], json!(["value", "id"]));
    assert_eq!(saved.0["columnWidths"]["id"], 199);
    assert_eq!(saved.0["future"], json!({"keep":true}));
    assert_eq!(saved.0["rawFilterText"], "id > 10");
}
#[test]
fn reorder_merges_latest_names_and_refuses_changed_group_or_visibility() {
    let patch = PreferencePatch::MoveColumn {
        selected: "id".into(),
        adjacent: "value".into(),
        left: false,
        pinned: false,
        source_order: vec!["id".into(), "value".into(), "last".into()],
    };
    let latest = TableGridPrefs(
        json!({"version":1,"columnOrder":["absent","id","value"],"pinnedColumns":["last"],"future":4}),
    );
    let saved = patch.apply(Some(latest.clone()), "now").unwrap();
    assert_eq!(
        saved.0["columnOrder"],
        json!(["absent", "value", "id", "last"])
    );
    assert_eq!(saved.0["pinnedColumns"], json!(["last"]));
    assert_eq!(saved.0["future"], 4);
    for (key, value) in [
        ("pinnedColumns", json!(["id"])),
        ("hiddenColumns", json!(["value"])),
    ] {
        let mut changed = latest.clone();
        changed.0[key] = value;
        assert!(patch.apply(Some(changed), "later").is_err());
    }
    let reversed =
        TableGridPrefs(json!({"version":1,"columnOrder":["value","id"],"pinnedColumns":["last"]}));
    assert!(patch.apply(Some(reversed), "later").is_err());
    let intervening = TableGridPrefs(json!({"version":1,"columnOrder":["id","last","value"]}));
    assert!(patch.apply(Some(intervening.clone()), "later").is_err());
    let mut hidden_between = intervening;
    hidden_between.0["hiddenColumns"] = json!(["last"]);
    assert!(patch.apply(Some(hidden_between), "later").is_ok());
    let pinned = PreferencePatch::MoveColumn {
        selected: "id".into(),
        adjacent: "value".into(),
        left: false,
        pinned: true,
        source_order: vec!["id".into(), "value".into(), "other".into()],
    };
    let latest = TableGridPrefs(
        json!({"version":1,"pinnedColumns":["absent","id","value","other"],"columnOrder":["id","value"]}),
    );
    let saved = pinned.apply(Some(latest), "now").unwrap();
    assert_eq!(
        saved.0["pinnedColumns"],
        json!(["absent", "value", "id", "other"])
    );
    assert_eq!(saved.0["columnOrder"], json!(["id", "value"]));
}
#[test]
fn pin_overflow_and_invalid_identity_preserve_the_latest_record() {
    let mut prefs = json!({"version":1,"future":""});
    let overhead = encoded_size(&prefs);
    prefs["future"] = json!("x".repeat(PREFS_BYTES - overhead));
    let latest = TableGridPrefs(prefs);
    let before = latest.clone();
    for name in ["id".to_owned(), String::new(), "x".repeat(64)] {
        assert!(
            PreferencePatch::PinColumn {
                selected: name,
                pinned: true
            }
            .apply(Some(latest.clone()), "now")
            .is_err()
        );
    }
    assert_eq!(latest, before);
}
