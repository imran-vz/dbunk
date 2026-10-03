use super::*;

fn value(column: &str, value: Option<&str>) -> MutationValue {
    MutationValue {
        column: column.into(),
        value: value.map(str::to_owned),
    }
}
fn row(id: &str) -> Vec<Option<String>> {
    vec![
        Some(id.into()),
        None,
        Some("original".into()),
        Some("derived".into()),
    ]
}
fn analysis(kind: MutationIdentityKind) -> AnalyzeResultSetResult {
    AnalyzeResultSetResult {
        request_id: 1,
        analysis_id: 3,
        statement: AnalysisStatement::Analyzed,
        columns: ["id", "name", "note", "computed"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| AnalyzedColumn {
                name: format!("alias_{name}"),
                origin: ColumnOrigin::Table {
                    schema: "s".into(),
                    table: "t".into(),
                    column: name.into(),
                    attnum: index as i16 + 1,
                },
                cast_type: "text".into(),
                nullable: index != 0,
                writability: if index == 3 {
                    ColumnWritability::Generated
                } else {
                    ColumnWritability::Writable
                },
            })
            .collect(),
        tables: vec![AnalyzedTable {
            schema: "s".into(),
            table: "t".into(),
            identity: MutationIdentity {
                kind,
                columns: vec![
                    if kind == MutationIdentityKind::CtidFallback {
                        "ctid"
                    } else {
                        "id"
                    }
                    .into(),
                ],
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
                allowed: true,
                reason: None,
            },
        }],
    }
}
fn snapshot(draft: &MutationDraft) -> String {
    serde_json::to_string(&draft.snapshot()).unwrap()
}

#[test]
fn duplicate_uses_original_values_and_retains_writable_keys_and_nulls() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let original = row("9223372036854775807");
    draft
        .stage_update(
            0,
            &original,
            None,
            false,
            vec![value("id", Some("2")), value("name", Some("staged"))],
        )
        .unwrap();
    assert_eq!(
        draft.duplicate_values(0, &original, false).unwrap(),
        vec![
            value("id", Some("9223372036854775807")),
            value("name", None),
            value("note", Some("original"))
        ]
    );
    let mut no_key = analysis(MutationIdentityKind::None);
    no_key.tables[0].updatable.allowed = false;
    no_key.columns[0].writability = ColumnWritability::IdentityAlways;
    no_key.columns[2].writability = ColumnWritability::SystemColumn;
    let draft = MutationDraft::new(no_key).unwrap();
    assert_eq!(
        draft.duplicate_values(0, &original, false).unwrap(),
        vec![value("name", None)]
    );
    let mut generated = analysis(MutationIdentityKind::None);
    for column in &mut generated.columns {
        column.writability = ColumnWritability::Generated;
    }
    let mut draft = MutationDraft::new(generated).unwrap();
    let values = draft.duplicate_values(0, &original, false).unwrap();
    assert!(values.is_empty());
    draft.stage_insert(0, values).unwrap();
}

#[test]
fn duplicate_projection_deduplicates_equal_values_and_refuses_conflicts_or_partial_source() {
    let mut metadata = analysis(MutationIdentityKind::PrimaryKey);
    metadata.columns.push(metadata.columns[1].clone());
    let draft = MutationDraft::new(metadata).unwrap();
    let mut original = row("1");
    original.push(None);
    assert_eq!(
        draft.duplicate_values(0, &original, false).unwrap().len(),
        3
    );
    original[4] = Some("different".into());
    assert_eq!(
        draft.duplicate_values(0, &original, false),
        Err(ModelError::InvalidInput)
    );
    assert_eq!(
        draft.duplicate_values(0, &original, true),
        Err(ModelError::Unavailable)
    );
}

#[test]
fn bulk_preserves_original_identity_other_edits_inclusion_and_reports_reverts() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let first = row("1");
    let second = row("2");
    draft
        .stage_update(
            0,
            &first,
            None,
            false,
            vec![
                value("id", Some("new-key")),
                value("note", Some("staged-note")),
            ],
        )
        .unwrap();
    let id = draft.changes[0].id;
    draft.include(id, false).unwrap();
    let rows = [(first.as_slice(), None), (second.as_slice(), None)];
    let before_revision = draft.revision;
    assert_eq!(
        draft
            .stage_bulk_update(0, &rows, false, value("name", Some("雪\0")))
            .unwrap(),
        BulkOutcome {
            selected: 2,
            changed: 2
        }
    );
    assert_eq!(draft.revision, before_revision + 1);
    assert_eq!(draft.changes[0].id, id);
    assert!(!draft.changes[0].included);
    let MutationOp::Update {
        identity,
        guards,
        set,
        ..
    } = &draft.changes[0].operation
    else {
        panic!()
    };
    assert_eq!(identity, &[value("id", Some("1"))]);
    assert!(guards.contains(&value("name", None)));
    assert!(set.contains(&value("id", Some("new-key"))));
    assert!(set.contains(&value("note", Some("staged-note"))));
    let revision = draft.revision;
    assert_eq!(
        draft
            .stage_bulk_update(0, &rows, false, value("name", Some("雪\0")))
            .unwrap()
            .changed,
        0
    );
    assert_eq!(draft.revision, revision);
    assert_eq!(
        draft
            .stage_bulk_update(0, &rows, false, value("name", None))
            .unwrap()
            .changed,
        2
    );
    assert_eq!(draft.len(), 1);
    assert_eq!(draft.changes[0].id, id);
    let MutationOp::Update { set, .. } = &draft.changes[0].operation else {
        panic!()
    };
    assert_eq!(
        set,
        &[
            value("id", Some("new-key")),
            value("note", Some("staged-note"))
        ]
    );
}

#[test]
fn late_invalid_or_deleted_row_does_not_partially_stage_or_invalidate_review() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let first = row("1");
    let second = row("2");
    draft.stage_delete(0, &second, None, false).unwrap();
    let saved = snapshot(&draft);
    let revision = draft.revision;
    let rows = [(first.as_slice(), None), (second.as_slice(), None)];
    assert_eq!(
        draft.stage_bulk_update(0, &rows, false, value("name", Some("new"))),
        Err(ModelError::Unavailable)
    );
    assert_eq!(snapshot(&draft), saved);
    assert_eq!(draft.revision, revision);
    let invalid = vec![Some("3".into())];
    assert!(
        draft
            .stage_bulk_update(
                0,
                &[(first.as_slice(), None), (invalid.as_slice(), None)],
                false,
                value("name", Some("new"))
            )
            .is_err()
    );
    assert_eq!(snapshot(&draft), saved);
}

#[test]
fn ambiguous_virtual_rows_refuse_atomically_and_ctid_keeps_full_guards() {
    let mut virtual_draft = MutationDraft::new(analysis(MutationIdentityKind::VirtualKey)).unwrap();
    let first = row("same");
    let mut other = first.clone();
    other[2] = Some("different".into());
    assert_eq!(
        virtual_draft.stage_bulk_update(
            0,
            &[(first.as_slice(), None), (other.as_slice(), None)],
            false,
            value("name", Some("x"))
        ),
        Err(ModelError::AmbiguousIdentity)
    );
    assert!(virtual_draft.is_empty());
    let mut ctid = MutationDraft::new(analysis(MutationIdentityKind::CtidFallback)).unwrap();
    let hidden = ["(0,1)".into()];
    ctid.stage_bulk_update(
        0,
        &[(first.as_slice(), Some(&hidden))],
        false,
        value("name", Some("x")),
    )
    .unwrap();
    let MutationOp::Update {
        identity, guards, ..
    } = &ctid.changes[0].operation
    else {
        panic!()
    };
    assert_eq!(identity, &[value("ctid", Some("(0,1)"))]);
    assert!(guards.contains(&value("computed", Some("derived"))));
    assert!(guards.contains(&identity[0]));
}

#[test]
fn count_and_repeated_input_bounds_refuse_without_partial_changes() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    for index in 0..CHANGE_LIMIT - 1 {
        draft
            .stage_insert(0, vec![value("name", Some(&index.to_string()))])
            .unwrap();
    }
    let saved = snapshot(&draft);
    let first = row("1");
    let second = row("2");
    assert_eq!(
        draft.stage_bulk_update(
            0,
            &[(first.as_slice(), None), (second.as_slice(), None)],
            false,
            value("name", Some("x"))
        ),
        Err(ModelError::Budget)
    );
    assert_eq!(snapshot(&draft), saved);
    let large = "x".repeat(DRAFT_BYTES / 2);
    assert_eq!(
        draft.stage_bulk_update(
            0,
            &[(first.as_slice(), None), (second.as_slice(), None)],
            false,
            value("name", Some(&large))
        ),
        Err(ModelError::Budget)
    );
    assert_eq!(snapshot(&draft), saved);
    assert_eq!(
        draft.stage_bulk_update(
            0,
            &vec![(first.as_slice(), None); CHANGE_LIMIT + 1],
            false,
            value("name", None)
        ),
        Err(ModelError::Budget)
    );
}

#[test]
fn late_draft_byte_overflow_keeps_every_original_change() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let megabyte = "x".repeat(1024 * 1024);
    for _ in 0..3 {
        draft
            .stage_insert(0, vec![value("name", Some(&megabyte))])
            .unwrap();
    }
    let saved = snapshot(&draft);
    let first = row("1");
    let second = row("2");
    let replacement = "y".repeat(700 * 1024);
    assert_eq!(
        draft.stage_bulk_update(
            0,
            &[(first.as_slice(), None), (second.as_slice(), None)],
            false,
            value("name", Some(&replacement))
        ),
        Err(ModelError::Budget)
    );
    assert_eq!(snapshot(&draft), saved);
}

#[test]
fn generated_stale_applying_and_unknown_drafts_refuse() {
    let mut draft = MutationDraft::new(analysis(MutationIdentityKind::PrimaryKey)).unwrap();
    let first = row("1");
    let rows = [(first.as_slice(), None)];
    assert_eq!(
        draft.stage_bulk_update(0, &rows, false, value("computed", Some("x"))),
        Err(ModelError::Unavailable)
    );
    assert_eq!(
        draft.stage_bulk_update(0, &rows, true, value("name", None)),
        Err(ModelError::Unavailable)
    );
    draft
        .stage_update(0, &first, None, false, vec![value("name", Some("x"))])
        .unwrap();
    let plan = draft.review().unwrap();
    let _ticket = draft.begin_apply(&plan).unwrap();
    assert_eq!(
        draft.stage_bulk_update(0, &rows, false, value("name", None)),
        Err(ModelError::Applying)
    );
    draft.applying = None;
    draft.outcome_unknown = true;
    assert_eq!(
        draft.duplicate_values(0, &first, false),
        Err(ModelError::OutcomeUnknown)
    );
    assert_eq!(
        draft.stage_bulk_update(0, &rows, false, value("name", None)),
        Err(ModelError::OutcomeUnknown)
    );
    draft.outcome_unknown = false;
    draft.invalidate();
    assert_eq!(
        draft.stage_bulk_update(0, &rows, false, value("name", None)),
        Err(ModelError::Stale)
    );
}
