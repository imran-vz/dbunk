use super::mutations::{ApplyTicket, ReviewPlan};
use super::*;

#[test]
fn borrowed_snapshot_measure_matches_exact_durable_encoding_before_and_during_apply() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    draft
        .stage_insert(
            0,
            vec![value("name", Some("\0\n東京🙂")), value("note", None)],
        )
        .unwrap();
    draft
        .stage_update(
            0,
            &row(),
            None,
            false,
            vec![value("name", Some("quoted \" text"))],
        )
        .unwrap();
    assert_eq!(
        draft.snapshot_bytes(),
        crate::results::encoded_size(&draft.snapshot())
    );
    let review = draft.review().unwrap();
    let _ticket = draft.begin_apply(&review).unwrap();
    assert_eq!(
        draft.snapshot_bytes(),
        crate::results::encoded_size(&draft.snapshot())
    );
    assert_eq!(
        draft.snapshot().apply_state,
        WorkspaceApplyState::OutcomeUnknown
    );
}

fn relation() -> MutationTable {
    MutationTable {
        schema: "Mixed.Schema".into(),
        table: "rows".into(),
    }
}
fn document() -> TableDocument {
    TableDocument::new("connection".into(), "tab".into(), relation()).unwrap()
}
fn page(id: u64) -> BrowseTableResult {
    BrowseTableResult {
        request_id: id,
        columns: vec![BrowseColumn {
            name: "id".into(),
            cast_type: "text".into(),
            nullable: false,
        }],
        rows: vec![vec![Some("first".into())]],
        identity: BrowseIdentity {
            kind: BrowseIdentityKind::PrimaryKey,
            columns: vec!["id".into()],
        },
        row_identity: Some(vec![vec!["first".into()]]),
        page_info: BrowsePageInfo {
            mode: BrowsePageMode::Keyset,
            page: None,
            has_more: true,
            next_cursor: Some(BrowseCursor {
                values: vec!["first".into()],
            }),
        },
        count: BrowseCount {
            kind: BrowseCountKind::Estimated,
            value: Some(300),
        },
        inspection: BrowseInspection {
            sql: "SELECT...".into(),
            params: vec![],
        },
        omitted_rows: 0,
        truncated_cells: 0,
        runtime_ms: 1,
    }
}
fn value(column: &str, value: Option<&str>) -> MutationValue {
    MutationValue {
        column: column.into(),
        value: value.map(str::to_owned),
    }
}
fn analysis(kind: MutationIdentityKind) -> AnalyzeResultSetResult {
    let table = relation();
    AnalyzeResultSetResult {
        request_id: 1,
        analysis_id: 9,
        statement: AnalysisStatement::Analyzed,
        columns: ["id", "name", "note", "computed"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| AnalyzedColumn {
                name: format!("alias_{name}"),
                origin: ColumnOrigin::Table {
                    schema: table.schema.clone(),
                    table: table.table.clone(),
                    column: name.into(),
                    attnum: i as i16 + 1,
                },
                cast_type: "text".into(),
                nullable: name != "id",
                writability: if name == "computed" {
                    ColumnWritability::Generated
                } else {
                    ColumnWritability::Writable
                },
            })
            .collect(),
        tables: vec![AnalyzedTable {
            schema: table.schema,
            table: table.table,
            identity: MutationIdentity {
                kind,
                columns: vec![if kind == MutationIdentityKind::CtidFallback {
                    "ctid".into()
                } else {
                    "id".into()
                }],
            },
            identity_projected: kind != MutationIdentityKind::CtidFallback,
            identity_projection_indexes: vec![0],
            updatable: CapabilityVerdict {
                allowed: true,
                reason: None,
            },
            deletable: CapabilityVerdict {
                allowed: true,
                reason: None,
            },
            insertable: CapabilityVerdict {
                allowed: kind != MutationIdentityKind::CtidFallback,
                reason: None,
            },
        }],
    }
}
fn row() -> Vec<Option<String>> {
    vec![
        Some("1".into()),
        None,
        Some("before".into()),
        Some("derived".into()),
    ]
}

#[test]
fn query_changes_fence_page_and_count_replies_without_relabelling_old_rows() {
    let mut doc = document();
    let (first, payload): (RequestTicket, BrowseTableDataPayload) =
        doc.browse(PageAction::First, false).unwrap();
    assert!(matches!(
        payload.page_request,
        BrowsePageRequest::Keyset { cursor: None }
    ));
    assert!(doc.receive_page(first, page(payload.request_id)).unwrap());
    let (count, count_payload) = doc.count().unwrap();
    let mut query = TableQuery::default();
    query.filters.push(BrowseFilter::IsNull {
        column: "name".into(),
    });
    query.sort.push(BrowseSortKey {
        column: "id".into(),
        direction: BrowseSortDirection::Desc,
        nulls: BrowseNulls::Last,
    });
    doc.set_query(query, "  id > 0  ").unwrap();
    assert!(!doc.page_is_current());
    assert_eq!(doc.result().unwrap().rows[0][0].as_deref(), Some("first"));
    assert!(
        !doc.receive_count(
            count,
            BrowseExactCountResult {
                kind: BrowseCountKind::Exact,
                value: 77,
                request_id: count_payload.request_id
            }
        )
        .unwrap()
    );
    let (second, payload) = doc.browse(PageAction::First, false).unwrap();
    assert_eq!(payload.filters.len(), 2);
    assert_eq!(
        payload.filters[1],
        BrowseFilter::RawSql {
            text: "id > 0".into()
        }
    );
    assert_eq!(payload.page_request, BrowsePageRequest::Offset { page: 1 });
    assert!(!doc.receive_page(first, page(first.sequence)).unwrap());
    assert!(!doc.failed(first));
    assert!(doc.failed(second));
    assert!(!doc.page_is_current());
    assert!(doc.exact_count().is_none());
}

#[test]
fn paging_keeps_server_cursor_and_uses_offset_for_backward_and_direct_navigation() {
    let mut doc = document();
    let (first, _) = doc.browse(PageAction::First, false).unwrap();
    doc.receive_page(first, page(first.sequence)).unwrap();
    let (next, payload) = doc.browse(PageAction::Next, false).unwrap();
    assert_eq!(
        payload.page_request,
        BrowsePageRequest::Keyset {
            cursor: Some(BrowseCursor {
                values: vec!["first".into()]
            })
        }
    );
    doc.receive_page(next, page(next.sequence)).unwrap();
    assert_eq!(doc.page(), 2);
    let (_, previous) = doc.browse(PageAction::Previous, false).unwrap();
    assert_eq!(previous.page_request, BrowsePageRequest::Offset { page: 1 });
    let (_, jump) = doc.browse(PageAction::Jump(25), false).unwrap();
    assert_eq!(jump.page_request, BrowsePageRequest::Offset { page: 25 });
    let (_, last) = doc.browse(PageAction::Last, false).unwrap();
    assert_eq!(last.page_request, BrowsePageRequest::Offset { page: 3 });
    let (count, _) = doc.count().unwrap();
    doc.receive_count(
        count,
        BrowseExactCountResult {
            kind: BrowseCountKind::Exact,
            value: 0,
            request_id: count.sequence,
        },
    )
    .unwrap();
    let (_, last) = doc.browse(PageAction::Last, false).unwrap();
    assert_eq!(last.page_request, BrowsePageRequest::Offset { page: 1 });
    let (refresh, _) = doc.browse(PageAction::Refresh, true).unwrap();
    assert!(doc.exact_count().is_none());
    doc.invalidate();
    assert!(!doc.receive_page(refresh, page(refresh.sequence)).unwrap());
}

#[test]
fn foreign_tickets_bad_shapes_and_over_budget_queries_cannot_replace_good_state() {
    let mut doc = document();
    let (ticket, _) = doc.browse(PageAction::First, false).unwrap();
    let (foreign, _) = document().browse(PageAction::First, false).unwrap();
    assert!(!doc.receive_page(foreign, page(foreign.sequence)).unwrap());
    let mut bad = page(ticket.sequence);
    bad.rows[0].clear();
    assert_eq!(doc.receive_page(ticket, bad), Err(ModelError::InvalidReply));
    let before = doc.query().clone();
    assert_eq!(
        doc.set_query(TableQuery::default(), &"界".repeat(QUERY_BYTES)),
        Err(ModelError::Budget)
    );
    assert!(doc.query() == &before);
    let (ticket, _) = doc.browse(PageAction::First, false).unwrap();
    let mut oversized = page(ticket.sequence);
    oversized.rows[0][0] = Some("x".repeat(PAGE_BYTES));
    assert_eq!(
        doc.receive_page_with_limit(ticket, oversized, usize::MAX),
        Err(ModelError::Budget)
    );
    assert!(doc.result().is_none());
}

#[test]
fn keyed_updates_keep_first_null_original_and_reverting_removes_only_that_cell() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    draft
        .stage_update(
            0,
            &row(),
            None,
            false,
            vec![value("name", Some("")), value("note", Some("漢字"))],
        )
        .unwrap();
    let old_review: ReviewPlan = draft.review().unwrap();
    let MutationOp::Update {
        guards, set, table, ..
    } = &old_review.plan().operations[0]
    else {
        panic!()
    };
    assert_eq!(table, &relation());
    assert_eq!(
        guards,
        &[value("name", None), value("note", Some("before"))]
    );
    assert_eq!(set[0], value("name", Some("")));
    let mut refreshed = row();
    refreshed[1] = Some("external edit".into());
    draft
        .stage_update(
            0,
            &refreshed,
            None,
            false,
            vec![value("name", Some("after"))],
        )
        .unwrap();
    assert!(matches!(
        draft.begin_apply(&old_review),
        Err(ModelError::Stale)
    ));
    let current = draft.review().unwrap();
    let MutationOp::Update { guards, .. } = &current.plan().operations[0] else {
        panic!()
    };
    assert_eq!(guards[0], value("name", None));
    draft
        .stage_update(0, &refreshed, None, false, vec![value("name", None)])
        .unwrap();
    let current = draft.review().unwrap();
    let MutationOp::Update { set, guards, .. } = &current.plan().operations[0] else {
        panic!()
    };
    assert_eq!(set, &[value("note", Some("漢字"))]);
    assert_eq!(guards, &[value("note", Some("before"))]);
    // A retained review stays immutable after later staging.
    let MutationOp::Update { set, .. } = &old_review.plan().operations[0] else {
        panic!()
    };
    assert_eq!(set[0], value("name", Some("")));
}

#[test]
fn virtual_and_ctid_changes_guard_full_projected_row_and_generated_columns_refuse() {
    for kind in [
        MutationIdentityKind::VirtualKey,
        MutationIdentityKind::CtidFallback,
    ] {
        let mut draft = MutationDraft::new(analysis(kind)).unwrap();
        let hidden = vec!["(0,1)".into()];
        let hidden = (kind == MutationIdentityKind::CtidFallback).then_some(hidden.as_slice());
        draft
            .stage_update(0, &row(), hidden, false, vec![value("name", Some("new"))])
            .unwrap();
        let review = draft.review().unwrap();
        let MutationOp::Update {
            guards, identity, ..
        } = &review.plan().operations[0]
        else {
            panic!()
        };
        assert!(guards.contains(&value("name", None)));
        assert!(guards.contains(&value("computed", Some("derived"))));
        assert!(guards.contains(&identity[0]));
        assert_eq!(
            guards.len(),
            if kind == MutationIdentityKind::CtidFallback {
                5
            } else {
                4
            }
        );
        draft.stage_delete(0, &row(), hidden, false).unwrap();
        assert_eq!(draft.len(), 1);
        let review = draft.review().unwrap();
        assert!(
            matches!(&review.plan().operations[0], MutationOp::Delete { guards: deleted, .. } if deleted == guards)
        );
    }
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    assert_eq!(
        draft.stage_update(
            0,
            &row(),
            None,
            false,
            vec![value("computed", Some("not writable"))]
        ),
        Err(ModelError::Unavailable)
    );
    assert_eq!(
        draft.stage_delete(0, &row(), None, true),
        Err(ModelError::Unavailable)
    );
    let mut null_key = row();
    null_key[0] = None;
    assert_eq!(
        draft.stage_delete(0, &null_key, None, false),
        Err(ModelError::Unavailable)
    );
    assert_eq!(draft.len(), 0);
}

#[test]
fn selection_changes_require_new_review_even_when_the_plan_returns_to_its_prior_values() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let first = draft.stage_insert(0, vec![value("name", None)]).unwrap();
    let second = draft.stage_insert(0, vec![]).unwrap();
    let before_selection = draft.review().unwrap();
    draft.include(first, false).unwrap();
    assert!(matches!(
        draft.begin_apply(&before_selection),
        Err(ModelError::Stale)
    ));
    let excluded = draft.review().unwrap();
    assert_eq!(excluded.plan().operations.len(), 1);
    draft.include(first, true).unwrap();
    assert_eq!(draft.review().unwrap().plan(), before_selection.plan());
    for review in [&before_selection, &excluded] {
        assert!(matches!(draft.begin_apply(review), Err(ModelError::Stale)));
    }
    let before_removal = draft.review().unwrap();
    draft.remove(second).unwrap();
    assert!(matches!(
        draft.begin_apply(&before_removal),
        Err(ModelError::Stale)
    ));
    let fresh = draft.review().unwrap();
    assert_eq!(fresh.plan().operations.len(), 1);
    assert!(draft.begin_apply(&fresh).is_ok());
}

#[test]
fn disconnected_and_restored_drafts_allow_selection_and_removal_without_granting_analysis() {
    let mut original = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    original
        .stage_update(0, &row(), None, false, vec![value("name", Some("東京🙂"))])
        .unwrap();
    let kept = original.changes().next().unwrap().0;
    let removed = original.stage_insert(0, vec![value("name", None)]).unwrap();
    let saved = document().snapshot(Some(original.snapshot())).unwrap();
    let restored = MutationDraft::restore(&saved).unwrap();
    original.invalidate();
    for mut draft in [original, restored] {
        let captured = draft.snapshot().changes[0].clone();
        draft.include(kept, false).unwrap();
        assert!(!draft.changes().next().unwrap().1);
        draft.remove(removed).unwrap();
        assert_eq!(draft.len(), 1);
        draft.include(kept, true).unwrap();
        assert!(draft.snapshot().changes == vec![captured]);
        assert!(matches!(draft.review(), Err(ModelError::Stale)));
        assert_eq!(draft.stage_insert(0, vec![]), Err(ModelError::Stale));
        assert_eq!(
            draft.stage_update(0, &row(), None, false, vec![value("name", None)]),
            Err(ModelError::Stale)
        );
        assert_eq!(
            draft.stage_delete(0, &row(), None, false),
            Err(ModelError::Stale)
        );
        draft
            .refresh_analysis(analysis(MutationIdentityKind::PrimaryKey))
            .unwrap();
        assert_eq!(draft.review().unwrap().plan().operations.len(), 1);
        draft.invalidate();
        draft.remove(kept).unwrap();
        assert!(draft.is_empty());
    }
}

#[test]
fn selection_and_removal_preserve_applying_and_uncertain_changes() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let id = draft.stage_insert(0, vec![value("name", None)]).unwrap();
    let ticket = draft.begin_apply(&draft.review().unwrap()).unwrap();
    let applying = draft.snapshot();
    draft.invalidate();
    assert_eq!(draft.include(id, false), Err(ModelError::Applying));
    assert_eq!(draft.remove(id), Err(ModelError::Applying));
    assert!(draft.snapshot() == applying);
    draft
        .finish_apply(ticket, Err(ResultMutationError::ConnectionLost))
        .unwrap();
    assert_eq!(draft.include(id, false), Err(ModelError::OutcomeUnknown));
    assert_eq!(draft.remove(id), Err(ModelError::OutcomeUnknown));
    assert!(draft.snapshot() == applying);
    draft.mark_outcome_reconciled().unwrap();
    draft.include(id, false).unwrap();
    draft.remove(id).unwrap();
    assert!(draft.is_empty());
}

#[test]
fn apply_failure_preserves_draft_attribution_and_success_only_removes_reviewed_changes() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::UniqueIndex)).unwrap();
    let excluded = draft.stage_insert(0, vec![]).unwrap();
    draft.include(excluded, false).unwrap();
    let inserted = draft.stage_insert(0, vec![value("name", None)]).unwrap();
    let review = draft.review().unwrap();
    assert_eq!(review.analysis_id(), 9);
    assert!(
        matches!(&review.plan().operations[0], MutationOp::Insert { values, .. } if values == &[value("name", None)])
    );
    let mut foreign = MutationDraft::new(analysis(MutationIdentityKind::UniqueIndex)).unwrap();
    assert!(matches!(
        foreign.begin_apply(&review),
        Err(ModelError::Stale)
    ));
    let ticket: ApplyTicket = draft.begin_apply(&review).unwrap();
    assert_eq!(draft.remove(excluded), Err(ModelError::Applying));
    let failed = draft
        .finish_apply(ticket, Err(ResultMutationError::Conflict { op_index: 0 }))
        .unwrap();
    assert!(
        matches!(failed, ApplyResolution::Failed { change: Some(id), error: ResultMutationError::Conflict { op_index: 0 } } if id == inserted)
    );
    assert_eq!(draft.len(), 2);
    let ticket = draft.begin_apply(&draft.review().unwrap()).unwrap();
    draft.invalidate();
    // Invalidation does not lose ownership of an already dispatched apply.
    assert!(matches!(
        draft
            .finish_apply(
                ticket,
                Ok(ApplyResult {
                    operations: vec![AppliedOperation {
                        op_index: 0,
                        rows_affected: 1
                    }],
                    runtime_ms: 2
                })
            )
            .unwrap(),
        ApplyResolution::Applied
    ));
    assert_eq!(draft.len(), 1);
    assert_eq!(draft.changes().next().unwrap().0, excluded);
    assert!(matches!(draft.review(), Err(ModelError::Stale)));
}

#[test]
fn budgets_and_malformed_apply_success_never_shed_unresolved_changes() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    draft
        .stage_insert(0, vec![value("name", Some("keep me"))])
        .unwrap();
    assert_eq!(
        draft.stage_insert(0, vec![value("name", Some(&"x".repeat(DRAFT_BYTES)))]),
        Err(ModelError::Budget)
    );
    assert_eq!(draft.len(), 1);
    let review = draft.review().unwrap();
    let ticket = draft.begin_apply(&review).unwrap();
    assert!(matches!(
        draft.finish_apply(
            ticket,
            Ok(ApplyResult {
                operations: vec![],
                runtime_ms: 0
            })
        ),
        Err(ModelError::InvalidReply)
    ));
    assert_eq!(draft.len(), 1);
    assert!(matches!(draft.review(), Err(ModelError::OutcomeUnknown)));
    let mut capped = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    for _ in 0..CHANGE_LIMIT {
        capped.stage_insert(0, vec![]).unwrap();
    }
    assert_eq!(capped.stage_insert(0, vec![]), Err(ModelError::Budget));
    assert_eq!(capped.len(), CHANGE_LIMIT);
}

#[test]
fn header_sort_cycle_keeps_order_for_appended_keys_and_resets_paging() {
    let mut doc = document();
    doc.cycle_sort("id", false).unwrap();
    doc.cycle_sort("name", true).unwrap();
    assert_eq!(
        doc.query()
            .sort
            .iter()
            .map(|key| key.column.as_str())
            .collect::<Vec<_>>(),
        ["id", "name"]
    );
    doc.cycle_sort("id", true).unwrap();
    assert_eq!(doc.query().sort[0].direction, BrowseSortDirection::Desc);
    doc.cycle_sort("id", true).unwrap();
    assert_eq!(doc.query().sort.len(), 1);
    doc.cycle_sort("name", false).unwrap();
    assert_eq!(doc.query().sort[0].direction, BrowseSortDirection::Desc);
    doc.cycle_sort("name", false).unwrap();
    assert!(doc.query().sort.is_empty());
    let (_, request) = doc.browse(PageAction::First, false).unwrap();
    assert_eq!(
        request.page_request,
        BrowsePageRequest::Keyset { cursor: None }
    );
}

#[test]
fn ambiguous_virtual_keys_and_reused_ctids_do_not_merge_distinct_captured_rows() {
    for kind in [
        MutationIdentityKind::VirtualKey,
        MutationIdentityKind::CtidFallback,
    ] {
        let mut draft = MutationDraft::new(analysis(kind)).unwrap();
        let hidden = vec!["(0,1)".into()];
        let hidden = (kind == MutationIdentityKind::CtidFallback).then_some(hidden.as_slice());
        draft
            .stage_update(
                0,
                &row(),
                hidden,
                false,
                vec![value("name", Some("first edit"))],
            )
            .unwrap();
        let before = draft.review().unwrap();
        let mut distinct = row();
        distinct[2] = Some("a different row with the same claimed identity".into());
        assert_eq!(
            draft.stage_update(
                0,
                &distinct,
                hidden,
                false,
                vec![value("name", Some("second edit"))]
            ),
            Err(ModelError::AmbiguousIdentity)
        );
        assert_eq!(
            draft.stage_delete(0, &distinct, hidden, false),
            Err(ModelError::AmbiguousIdentity)
        );
        assert_eq!(draft.len(), 1);
        assert_eq!(draft.review().unwrap().plan(), before.plan());
        // Refusal neither changes staged values nor invalidates their exact review.
        let ticket = draft.begin_apply(&before).unwrap();
        draft
            .finish_apply(ticket, Err(ResultMutationError::Cancelled))
            .unwrap();
        draft
            .stage_update(
                0,
                &row(),
                hidden,
                false,
                vec![value("name", Some("restaged same row"))],
            )
            .unwrap();
        let review = draft.review().unwrap();
        let MutationOp::Update { guards, set, .. } = &review.plan().operations[0] else {
            panic!()
        };
        assert!(guards.contains(&value("name", None)));
        assert_eq!(set, &[value("name", Some("restaged same row"))]);
    }
}

#[test]
fn durable_table_restore_keeps_intent_without_pages_and_requires_new_analysis_review() {
    let mut doc = document();
    doc.cycle_sort("name", false).unwrap();
    let (request, _) = doc.browse(PageAction::First, false).unwrap();
    doc.receive_page(request, page(request.sequence)).unwrap();
    doc.clear_page();
    assert!(doc.result().is_none());
    assert!(!doc.page_is_current());
    assert!(!doc.failed(request));
    let (_, refresh) = doc.browse(PageAction::Refresh, false).unwrap();
    assert_eq!(refresh.page_request, BrowsePageRequest::Offset { page: 1 });
    assert_eq!(refresh.sort[0].column, "name");

    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    draft
        .stage_update(0, &row(), None, false, vec![value("name", Some("東京🙂"))])
        .unwrap();
    let old_review = draft.review().unwrap();
    let saved = doc.snapshot(Some(draft.snapshot())).unwrap();
    let restored_doc = TableDocument::restore(
        "deleted-connection".into(),
        "same-visible-tab".into(),
        &saved,
    )
    .unwrap();
    assert!(restored_doc.result().is_none());
    assert!(!restored_doc.page_is_current());
    assert_eq!(restored_doc.query().sort, doc.query().sort);
    let mut restored = MutationDraft::restore(&saved).unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(
        restored.changes().next().unwrap().2,
        &old_review.plan().operations[0]
    );
    assert!(matches!(restored.review(), Err(ModelError::Stale)));
    let mut fresh = analysis(MutationIdentityKind::PrimaryKey);
    fresh.analysis_id = 99;
    restored.refresh_analysis(fresh).unwrap();
    assert!(matches!(
        restored.begin_apply(&old_review),
        Err(ModelError::Stale)
    ));
    let new_review = restored.review().unwrap();
    assert_eq!(new_review.analysis_id(), 99);
    assert_eq!(new_review.plan(), old_review.plan());
}

#[test]
fn restored_inflight_apply_is_explicitly_uncertain_and_cannot_be_retried_by_reanalysis() {
    let doc = document();
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    draft.stage_insert(0, vec![value("name", None)]).unwrap();
    let review = draft.review().unwrap();
    let ticket = draft.begin_apply(&review).unwrap();
    let saved = doc.snapshot(Some(draft.snapshot())).unwrap();
    assert!(saved.draft.as_ref().unwrap().apply_state == WorkspaceApplyState::OutcomeUnknown);
    let mut restored = MutationDraft::restore(&saved).unwrap();
    assert!(restored.outcome_unknown());
    assert!(matches!(restored.review(), Err(ModelError::OutcomeUnknown)));
    assert_eq!(
        restored.refresh_analysis(analysis(MutationIdentityKind::PrimaryKey)),
        Err(ModelError::OutcomeUnknown)
    );
    restored.mark_outcome_reconciled().unwrap();
    assert!(matches!(restored.review(), Err(ModelError::Stale)));
    restored
        .refresh_analysis(analysis(MutationIdentityKind::PrimaryKey))
        .unwrap();
    assert_eq!(restored.review().unwrap().plan(), review.plan());
    draft
        .finish_apply(ticket, Err(ResultMutationError::ConnectionLost))
        .unwrap();
    assert!(draft.outcome_unknown());
    assert!(draft.snapshot().apply_state == WorkspaceApplyState::OutcomeUnknown);
}

#[test]
fn changed_catalog_or_malformed_saved_guards_preserve_draft_and_refuse_recovery() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::VirtualKey)).unwrap();
    draft
        .stage_update(0, &row(), None, false, vec![value("name", Some("new"))])
        .unwrap();
    let before = draft.review().unwrap();
    let mut saved = document().snapshot(Some(draft.snapshot())).unwrap();
    let mut restored = MutationDraft::restore(&saved).unwrap();
    assert_eq!(
        restored.refresh_analysis(analysis(MutationIdentityKind::PrimaryKey)),
        Err(ModelError::Stale)
    );
    assert_eq!(
        restored.changes().next().unwrap().2,
        &before.plan().operations[0]
    );
    let MutationOp::Update { guards, .. } = &mut saved.draft.as_mut().unwrap().changes[0].operation
    else {
        panic!()
    };
    guards.clear();
    assert!(matches!(
        MutationDraft::restore(&saved),
        Err(ModelError::InvalidInput)
    ));
}

#[test]
fn shared_page_and_cached_account_survive_refused_replacements_and_clear_together() {
    use std::rc::Rc;
    let mut doc = document();
    let (first, _) = doc.browse(PageAction::First, false).unwrap();
    let initial = page(first.sequence);
    let bytes = crate::results::encoded_size(&initial) + initial.columns.len() * size_of::<f32>();
    assert!(doc.receive_page_with_limit(first, initial, bytes).unwrap());
    let shared = doc.shared_result().unwrap();
    assert!(Rc::ptr_eq(&shared, &doc.shared_result().unwrap()));
    assert!(std::ptr::eq(doc.result().unwrap(), shared.as_ref()));
    assert_eq!(doc.retained_bytes(), bytes);

    let (next, _) = doc.browse(PageAction::Next, false).unwrap();
    // Stale delivery cannot consume the current ticket or replace its account.
    assert!(
        !doc.receive_page_with_limit(first, page(first.sequence), 0)
            .unwrap()
    );
    assert_eq!(
        doc.receive_page_with_limit(next, page(next.sequence), bytes - 1),
        Err(ModelError::Budget)
    );
    assert_eq!(doc.retained_bytes(), bytes);
    assert!(Rc::ptr_eq(&shared, &doc.shared_result().unwrap()));
    assert_eq!(doc.page(), 1);

    let (next, _) = doc.browse(PageAction::Next, false).unwrap();
    let mut malformed = page(next.sequence);
    malformed.rows[0].clear();
    assert_eq!(
        doc.receive_page_with_limit(next, malformed, usize::MAX),
        Err(ModelError::InvalidReply)
    );
    assert_eq!(doc.retained_bytes(), bytes);
    assert!(Rc::ptr_eq(&shared, &doc.shared_result().unwrap()));

    let (next, _) = doc.browse(PageAction::Next, false).unwrap();
    let mut replacement = page(next.sequence);
    replacement.rows[0][0] = Some("東京🙂".into());
    let next_bytes =
        crate::results::encoded_size(&replacement) + replacement.columns.len() * size_of::<f32>();
    assert!(
        doc.receive_page_with_limit(next, replacement, next_bytes)
            .unwrap()
    );
    assert!(!Rc::ptr_eq(&shared, &doc.shared_result().unwrap()));
    assert_eq!(doc.retained_bytes(), next_bytes);
    assert_eq!(doc.page(), 2);
    // The old grid reference remains valid until the host replaces it.
    assert_eq!(shared.rows[0][0].as_deref(), Some("first"));
    doc.clear_page();
    assert_eq!(doc.retained_bytes(), 0);
    assert!(doc.result().is_none());
    assert!(doc.shared_result().is_none());
}

#[test]
fn reopening_staged_cells_preserves_intent_and_original_guards() {
    let original = row();
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    draft
        .stage_update(
            0,
            &original,
            None,
            false,
            vec![
                value("name", Some("你好")),
                value("id", Some("new-key")),
                value("note", None),
            ],
        )
        .unwrap();
    // The source index resolves aliases; edited key values never change lookup identity.
    assert_eq!(
        draft
            .edit_value(0, &original, None, false, 0)
            .unwrap()
            .as_deref(),
        Some("new-key")
    );
    let current = draft
        .edit_value(0, &original, None, false, 1)
        .unwrap()
        .clone();
    assert_eq!(current.as_deref(), Some("你好"));
    assert_eq!(
        draft.edit_value(0, &original, None, false, 2).unwrap(),
        &None
    );
    draft
        .stage_update(
            0,
            &original,
            None,
            false,
            vec![MutationValue {
                column: "name".into(),
                value: current,
            }],
        )
        .unwrap();
    let review = draft.review().unwrap();
    let MutationOp::Update {
        guards,
        identity,
        set,
        ..
    } = &review.plan().operations[0]
    else {
        panic!("update")
    };
    assert_eq!(identity[0].value, original[0]);
    assert!(
        guards
            .iter()
            .any(|value| value.column == "name" && value.value == original[1])
    );
    assert!(
        set.iter()
            .any(|value| value.column == "name" && value.value.as_deref() == Some("你好"))
    );
    assert!(draft.edit_value(0, &original, None, false, 3).is_err());
    draft.stage_delete(0, &original, None, false).unwrap();
    assert!(draft.edit_value(0, &original, None, false, 1).is_err());

    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::VirtualKey)).unwrap();
    draft
        .stage_update(
            0,
            &original,
            None,
            false,
            vec![value("name", Some("pending"))],
        )
        .unwrap();
    let mut other = original.clone();
    other[2] = Some("another row with same unproven key".into());
    assert!(matches!(
        draft.edit_value(0, &other, None, false, 1),
        Err(ModelError::AmbiguousIdentity)
    ));
}

#[test]
fn query_updates_resolve_join_origin_and_restore_exact_source_independent_of_editor() {
    let mut joined = analysis(MutationIdentityKind::PrimaryKey);
    let mut other = joined.tables[0].clone();
    other.table = "other".into();
    other.identity_projection_indexes = vec![4];
    joined.tables.push(other);
    for column in joined.columns.clone().into_iter().take(3) {
        let mut column = column;
        column.name = format!("other_{}", column.name);
        if let ColumnOrigin::Table { table, .. } = &mut column.origin {
            *table = "other".into();
        }
        joined.columns.push(column);
    }
    let mut values = row();
    values.extend([Some("2".into()), Some("old other".into()), None]);
    let (target, name) = crate::query_result::editable_target(&joined, &values, 5, false).unwrap();
    assert_eq!((target, name.as_str()), (1, "name"));
    let mut draft = MutationDraft::new(joined.clone()).unwrap();
    draft
        .stage_update(
            target,
            &values,
            None,
            false,
            vec![value(&name, Some("new other"))],
        )
        .unwrap();
    draft
        .stage_update(
            0,
            &values,
            None,
            false,
            vec![value("name", Some("new first"))],
        )
        .unwrap();
    let saved = dbunk_lib::backend::WorkspaceQueryChanges {
        source: dbunk_lib::backend::QueryMutationSource::new("SELECT a.id, a.name, a.note, a.computed, b.id, b.name, b.note FROM \"Mixed.Schema\".rows a JOIN \"Mixed.Schema\".other b ON a.id=b.id".into(), false).unwrap(),
        draft: draft.snapshot(),
    };
    saved.validate().unwrap();
    let mut restored = MutationDraft::restore_query(&saved).unwrap();
    assert!(restored.review().is_err());
    restored.refresh_analysis(joined.clone()).unwrap();
    assert!(restored.snapshot() == saved.draft);
    let MutationOp::Update {
        table,
        identity,
        guards,
        ..
    } = restored.changes().next().unwrap().2
    else {
        panic!("update expected")
    };
    assert_eq!(table.table, "other");
    assert_eq!(identity, &[value("id", Some("2"))]);
    assert_eq!(guards, &[value("name", Some("old other"))]);
    values[5] = Some("雪".into());
    assert!(crate::query_result::editable_target(&joined, &values, 5, false).is_err());
    values[5] = Some("old other".into());
    values[4] = None;
    assert!(crate::query_result::editable_target(&joined, &values, 5, false).is_err());
    assert!(crate::query_result::editable_target(&joined, &values, 3, false).is_err());
}

#[test]
fn projected_ctid_query_identity_uses_full_original_guards_without_hidden_browse_identity() {
    let mut analyzed = analysis(MutationIdentityKind::CtidFallback);
    analyzed.tables[0].identity_projected = true;
    analyzed.columns[0].origin = ColumnOrigin::Table {
        schema: relation().schema,
        table: relation().table,
        column: "ctid".into(),
        attnum: -1,
    };
    analyzed.columns[0].writability = ColumnWritability::SystemColumn;
    let mut values = row();
    values[0] = Some("(0,7)".into());
    assert_eq!(
        crate::query_result::editable_target(&analyzed, &values, 1, false)
            .unwrap()
            .0,
        0
    );
    let mut draft = MutationDraft::new(analyzed.clone()).unwrap();
    draft
        .stage_update(
            0,
            &values,
            None,
            false,
            vec![value("name", Some("updated"))],
        )
        .unwrap();
    let MutationOp::Update {
        identity, guards, ..
    } = draft.changes().next().unwrap().2
    else {
        panic!("update expected")
    };
    assert_eq!(identity, &[value("ctid", Some("(0,7)"))]);
    assert_eq!(guards.len(), 4);
    assert!(guards.contains(&value("note", Some("before"))));
    values[0] = None;
    assert!(crate::query_result::editable_target(&analyzed, &values, 1, false).is_err());
    analyzed.tables[0].identity.kind = MutationIdentityKind::VirtualKey;
    assert!(crate::query_result::editable_target(&analyzed, &values, 1, false).is_err());
}

#[test]
fn sampled_table_widths_defer_to_saved_source_names_after_reordering() {
    let mut page = page(1);
    page.columns.push(BrowseColumn {
        name: "value".into(),
        cast_type: "text".into(),
        nullable: true,
    });
    page.rows = vec![vec![Some("1234567890".into()), Some("x".into())]; 100];
    page.rows
        .push(vec![Some("x".repeat(1000)), Some("x".repeat(1000))]);
    let mut columns = crate::grid_columns::GridColumns::default();
    columns.table_columns(&page);
    assert_eq!(columns.width(0), 91.);
    assert_eq!(columns.width(1), 76.);
    columns
        .load(Some(TableGridPrefs(
            serde_json::json!({"version":1,"columnOrder":["value","id"],"columnWidths":{"id":222}}),
        )))
        .unwrap();
    assert_eq!(columns.source(0), Some(1));
    assert_eq!(columns.width(0), 76.);
    assert_eq!(columns.width(1), 222.);
    // New pages can resize sampled defaults; explicit saved widths still win.
    page.rows[0][1] = Some("x".repeat(1000));
    columns.table_columns(&page);
    assert_eq!(columns.width(0), 400.);
    assert_eq!(columns.width(1), 222.);
}
