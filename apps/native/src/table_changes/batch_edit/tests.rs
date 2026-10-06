use super::*;

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
    let context = EditContext::Bulk(BulkEdit {
        capture: Capture {
            page: page.clone(),
            analysis_id: 2,
        },
        rows: vec![0],
        column: 0,
    });
    assert!(context.current(Some(&page), Some(&analysis)));
    assert!(EditContext::Ordinary.current(None, None));
    let replacement = Rc::new((*page).clone());
    assert!(!context.current(Some(&replacement), Some(&analysis)));
    analysis.analysis_id = 3;
    assert!(!context.current(Some(&page), Some(&analysis)));
    assert!(!context.current(None, Some(&analysis)));
}
