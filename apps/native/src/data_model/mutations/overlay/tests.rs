use super::*;

fn value(column: &str, value: Option<&str>) -> MutationValue {
    MutationValue {
        column: column.into(),
        value: value.map(str::to_owned),
    }
}
fn cells(values: [Option<&str>; 3]) -> Vec<Option<String>> {
    values.into_iter().map(|v| v.map(str::to_owned)).collect()
}
fn table(name: &str, kind: MutationIdentityKind) -> AnalyzedTable {
    let allowed = CapabilityVerdict {
        allowed: true,
        reason: None,
    };
    AnalyzedTable {
        schema: "s".into(),
        table: name.into(),
        identity: MutationIdentity {
            kind,
            columns: vec![if kind == MutationIdentityKind::CtidFallback {
                "ctid"
            } else {
                "id"
            }
            .into()],
        },
        identity_projected: kind != MutationIdentityKind::CtidFallback,
        identity_projection_indexes: vec![0],
        updatable: allowed.clone(),
        deletable: allowed.clone(),
        insertable: allowed,
    }
}
fn analysis(kind: MutationIdentityKind) -> AnalyzeResultSetResult {
    AnalyzeResultSetResult {
        request_id: 1,
        analysis_id: 2,
        statement: AnalysisStatement::Analyzed,
        columns: ["id", "name", "note"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| AnalyzedColumn {
                name: name.into(),
                origin: ColumnOrigin::Table {
                    schema: "s".into(),
                    table: "t".into(),
                    column: name.into(),
                    attnum: index as i16 + 1,
                },
                cast_type: "text".into(),
                nullable: index != 0,
                writability: ColumnWritability::Writable,
            })
            .collect(),
        tables: vec![table("t", kind)],
    }
}
fn page(rows: Vec<Vec<Option<String>>>, row_identity: Option<Vec<Vec<String>>>) -> BrowseTableResult {
    BrowseTableResult {
        request_id: 1,
        columns: ["id", "name", "note"]
            .into_iter()
            .map(|name| BrowseColumn {
                name: name.into(),
                cast_type: "text".into(),
                nullable: true,
            })
            .collect(),
        rows,
        identity: BrowseIdentity {
            kind: BrowseIdentityKind::PrimaryKey,
            columns: vec!["id".into()],
        },
        row_identity,
        page_info: BrowsePageInfo {
            mode: BrowsePageMode::Keyset,
            page: None,
            has_more: false,
            next_cursor: None,
        },
        count: BrowseCount {
            kind: BrowseCountKind::Estimated,
            value: None,
        },
        inspection: BrowseInspection {
            sql: "SELECT".into(),
            params: vec![],
        },
        omitted_rows: 0,
        truncated_cells: 0,
        runtime_ms: 1,
    }
}
fn text(value: &str) -> OverlayValue {
    OverlayValue {
        text: Some(value.into()),
        truncated: false,
    }
}
const NULL: OverlayValue = OverlayValue {
    text: None,
    truncated: false,
};

#[test]
fn keyed_update_follows_its_row_after_the_page_is_resorted() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let ada = cells([Some("1"), Some("Ada"), Some("x")]);
    let bob = cells([Some("2"), Some("Bob"), None]);
    draft
        .stage_update(0, &ada, None, false, vec![value("name", Some("Ann")), value("note", None)])
        .unwrap();
    let change = draft.changes[0].id;
    let failed = Some(change);
    let overlay = draft.overlay(&page(vec![bob.clone(), ada.clone()], None), 7, failed);
    assert_eq!(
        overlay.key,
        OverlayKey {
            draft: Some((draft.owner(), draft.revision())),
            page: 7,
        }
    );
    assert_eq!(overlay.failed, failed);
    assert_eq!(overlay.mark(0), None);
    assert_eq!(
        overlay.mark(1),
        Some(&RowMark::Updated {
            change,
            included: true,
            cells: vec![(1, text("Ann")), (2, NULL)],
        })
    );
    assert_eq!(overlay.cell(1, 1), Some(&text("Ann")));
    assert_eq!(overlay.cell(1, 0), None);
    assert_eq!(overlay.cell(0, 1), None);
    assert_eq!(overlay.mark(9), None);
    assert!(!overlay.is_empty());

    // Another sort order: the tint moves with the row, not the position.
    let overlay = draft.overlay(&page(vec![ada, bob], None), 8, None);
    assert!(matches!(overlay.mark(0), Some(RowMark::Updated { .. })));
    assert_eq!(overlay.mark(1), None);
}

#[test]
fn reused_ctid_with_different_values_is_not_painted() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::CtidFallback)).unwrap();
    let original = cells([Some("1"), Some("Ada"), None]);
    let ctid = vec!["(0,1)".to_owned()];
    draft
        .stage_update(0, &original, Some(ctid.as_slice()), false, vec![value("name", Some("Ann"))])
        .unwrap();
    let same = draft.overlay(&page(vec![original], Some(vec![ctid.clone()])), 1, None);
    assert_eq!(same.cell(0, 1), Some(&text("Ann")));
    // VACUUM reused (0,1) for a different row.
    let reused = cells([Some("9"), Some("Zed"), None]);
    let other = draft.overlay(&page(vec![reused], Some(vec![ctid])), 2, None);
    assert!(other.is_empty());
    // Without hidden identity the row cannot be matched at all.
    let original = cells([Some("1"), Some("Ada"), None]);
    assert!(draft.overlay(&page(vec![original], None), 3, None).is_empty());
}

#[test]
fn deletes_and_excluded_changes_carry_their_marks() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let first = cells([Some("1"), Some("Ada"), None]);
    let second = cells([Some("2"), Some("Bob"), None]);
    draft.stage_delete(0, &first, None, false).unwrap();
    draft
        .stage_update(0, &second, None, false, vec![value("note", Some("n"))])
        .unwrap();
    let deleted = draft.changes[0].id;
    let updated = draft.changes[1].id;
    draft.include(updated, false).unwrap();
    let overlay = draft.overlay(&page(vec![first, second], None), 1, None);
    assert_eq!(
        overlay.mark(0),
        Some(&RowMark::Deleted {
            change: deleted,
            included: true,
        })
    );
    assert_eq!(overlay.cell(0, 0), None);
    assert!(matches!(
        overlay.mark(1),
        Some(RowMark::Updated { change, included: false, .. }) if *change == updated
    ));
}

#[test]
fn inserts_show_defaults_nulls_and_values_per_page_column() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let id = draft
        .stage_insert(0, vec![value("note", Some("v")), value("name", None)])
        .unwrap();
    let blank = draft.stage_insert(0, vec![]).unwrap();
    let overlay = draft.overlay(&page(vec![], None), 1, None);
    assert!(overlay.rows.is_empty());
    assert_eq!(
        overlay.inserts,
        vec![
            InsertRow {
                change: id,
                included: true,
                cells: vec![
                    InsertCell::Default,
                    InsertCell::Value(NULL),
                    InsertCell::Value(text("v")),
                ],
            },
            InsertRow {
                change: blank,
                included: true,
                cells: vec![InsertCell::Default; 3],
            },
        ]
    );
    assert!(!overlay.is_empty());
}

#[test]
fn overlay_survives_invalidation_without_analysis() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let row = cells([Some("1"), Some("Ada"), None]);
    draft
        .stage_update(0, &row, None, false, vec![value("name", Some("Ann"))])
        .unwrap();
    draft.stage_insert(0, vec![]).unwrap();
    let page = page(vec![row], None);
    let before = draft.overlay(&page, 1, None);
    draft.invalidate();
    assert!(draft.analysis.is_none());
    let after = draft.overlay(&page, 1, None);
    assert_eq!(after.rows, before.rows);
    assert_eq!(after.inserts, before.inserts);
    assert_eq!(after.cell(0, 1), Some(&text("Ann")));
}

#[test]
fn changes_on_another_table_are_ignored() {
    let mut metadata = analysis(MutationIdentityKind::PrimaryKey);
    metadata
        .tables
        .push(table("other", MutationIdentityKind::PrimaryKey));
    let mut draft = MutationDraft::new(metadata).unwrap();
    let row = cells([Some("1"), Some("Ada"), None]);
    draft.stage_insert(1, vec![]).unwrap();
    draft
        .stage_update(0, &row, None, false, vec![value("name", Some("Ann"))])
        .unwrap();
    let overlay = draft.overlay(&page(vec![row.clone()], None), 1, None);
    assert!(overlay.inserts.is_empty());
    assert_eq!(overlay.cell(0, 1), Some(&text("Ann")));

    // A restored draft with no analysis cannot tell which table the page
    // shows when its changes span two tables, so it paints nothing.
    draft.invalidate();
    assert!(draft.overlay(&page(vec![row], None), 1, None).is_empty());
}

#[test]
fn staged_text_is_capped_on_a_utf8_boundary() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let long = "東".repeat(OVERLAY_TEXT_BYTES);
    draft
        .stage_insert(0, vec![value("note", Some(&long))])
        .unwrap();
    let overlay = draft.overlay(&page(vec![], None), 1, None);
    let InsertCell::Value(value) = &overlay.inserts[0].cells[2] else {
        panic!("expected a staged value");
    };
    assert!(value.truncated);
    // 2048 is not a multiple of the 3-byte character, so the cut backs off.
    assert_eq!(value.text.as_deref(), Some("東".repeat(682).as_str()));
    let exact = "a".repeat(OVERLAY_TEXT_BYTES);
    assert_eq!(overlay_value(&Some(exact.clone())), text(&exact));
}

#[test]
fn an_empty_draft_paints_nothing() {
    let draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let overlay = draft.overlay(&page(vec![cells([Some("1"), None, None])], None), 3, None);
    assert!(overlay.is_empty());
    assert_eq!(overlay.rows, vec![None]);
    assert_eq!(overlay.key.draft, Some((draft.owner(), 0)));
    let empty = DraftOverlay::empty(2);
    assert!(empty.is_empty());
    assert_eq!(empty.rows.len(), 2);
    assert_eq!(empty.key.draft, None);
}
