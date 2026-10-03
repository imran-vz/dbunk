use super::*;

#[test]
fn duplicate_editor_json_preserves_exact_text_null_and_omission() {
    let values = vec![
        MutationValue {
            column: "quote\"key".into(),
            value: Some("9223372036854775807\n雪".into()),
        },
        MutationValue {
            column: "null".into(),
            value: None,
        },
        MutationValue {
            column: "empty".into(),
            value: Some(String::new()),
        },
    ];
    let text = duplicate_json(&values).unwrap();
    let actual = insert_values(&text).unwrap();
    for value in values {
        assert!(actual.contains(&value));
    }
    assert_eq!(duplicate_json(&[]).unwrap(), "{}");
}

#[test]
fn duplicate_editor_counts_escaped_json_before_allocating_it() {
    let values = [MutationValue {
        column: "text".into(),
        value: Some("\0".repeat(cell_value::MAX_VALUE_BYTES / 5)),
    }];
    assert_eq!(duplicate_json(&values), Err(ModelError::Budget));
}

#[test]
fn captured_edit_requires_the_same_page_owner_and_analysis() {
    let page = Rc::new(BrowseTableResult {
        request_id: 1,
        columns: vec![],
        rows: vec![],
        identity: BrowseIdentity {
            kind: BrowseIdentityKind::None,
            columns: vec![],
        },
        row_identity: None,
        page_info: BrowsePageInfo {
            mode: BrowsePageMode::Offset,
            page: Some(1),
            has_more: false,
            next_cursor: None,
        },
        count: BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        },
        inspection: BrowseInspection {
            sql: String::new(),
            params: vec![],
        },
        omitted_rows: 0,
        truncated_cells: 0,
        runtime_ms: 0,
    });
    let mut analysis = AnalyzeResultSetResult {
        request_id: 1,
        analysis_id: 2,
        statement: AnalysisStatement::Analyzed,
        columns: vec![],
        tables: vec![],
    };
    let context = EditContext::Duplicate(Capture {
        page: page.clone(),
        analysis_id: 2,
    });
    assert!(context.current(Some(&page), Some(&analysis)));
    let replacement = Rc::new((*page).clone());
    assert!(!context.current(Some(&replacement), Some(&analysis)));
    analysis.analysis_id = 3;
    assert!(!context.current(Some(&page), Some(&analysis)));
    assert!(!context.current(None, Some(&analysis)));
}
