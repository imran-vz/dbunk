use super::*;
fn budget() -> Rc<Cell<usize>> {
    Rc::new(Cell::new(0))
}
fn target() -> CsvTarget {
    CsvTarget {
        schema: "Case.Sensitive".into(),
        table: "table name".into(),
    }
}
fn setup(budget: Rc<Cell<usize>>) -> Setup {
    Setup::new(
        "connection".into(),
        3,
        CsvDirection::Import,
        Some(target()),
        budget,
    )
    .unwrap()
}
fn column(name: &str) -> CsvTargetColumn {
    CsvTargetColumn {
        name: name.into(),
        data_type: "text".into(),
        nullable: true,
        has_default: false,
        generated: false,
        identity: false,
    }
}
fn inspection() -> CsvInspectionData {
    CsvInspectionData {
        inspection_id: CsvInspectionId::new(),
        connection_id: "connection".into(),
        target: target(),
        direction: CsvDirection::Import,
        file_name: Some("input.csv".into()),
        total_bytes: Some(0),
        workbook: None,
        source_columns: ["a", "a", "", "required", "generated", "identity"]
            .iter()
            .enumerate()
            .map(|(index, name)| CsvSourceColumn {
                index,
                name: (*name).into(),
            })
            .collect(),
        target_columns: vec![
            column("a"),
            column("blank"),
            CsvTargetColumn {
                nullable: false,
                ..column("required")
            },
            CsvTargetColumn {
                generated: true,
                ..column("generated")
            },
            CsvTargetColumn {
                identity: true,
                ..column("identity")
            },
        ],
        sample_rows: vec![vec![
            Some("".into()),
            None,
            Some("\\N".into()),
            Some("x".into()),
            None,
            None,
        ]],
        sample_truncated: true,
        options: CsvOptions::default(),
        connection: CsvConnectionTarget {
            connection_name: "Local".into(),
            host: "localhost".into(),
            port: 5432,
            database: "db".into(),
            user: "user".into(),
            environment: "development".into(),
            safe_mode: "off".into(),
            read_only: false,
        },
        expires_at: "2026-10-03T12:00:00Z".into(),
    }
}
fn job() -> CsvTransferObservation {
    CsvTransferObservation {
        attempt_id: CsvTransferAttemptId::new(),
        inspection_id: CsvInspectionId::new(),
        connection_id: "connection".into(),
        target: target(),
        direction: CsvDirection::Import,
        file_name: "input.csv".into(),
        phase: CsvTransferPhase::Completed,
        effect: CsvEffect::Succeeded,
        cleanup: CsvCleanup::Complete,
        started_at: "2026-10-03T11:00:00Z".into(),
        finished_at: Some("2026-10-03T11:01:00Z".into()),
        total_bytes: Some(0),
        workbook: None,
        bytes_processed: 0,
        rows_processed: Some(0),
        rows_committed: Some(0),
        failure: None,
        diagnostic: None,
        import_change_revision: Some(1),
    }
}
fn lists(jobs: Vec<CsvTransferObservation>, revision: u64) -> (CsvInspectionList, CsvTransferList) {
    (
        CsvInspectionList {
            inspections: vec![],
        },
        CsvTransferList {
            jobs,
            import_change_revision: revision,
            execution_reserved_bytes: 0,
        },
    )
}
#[test]
fn picker_owner_revision_target_and_connection_generation_are_exact() {
    let shared = budget();
    let mut first = setup(shared.clone());
    let second = setup(shared.clone());
    assert!(!second.is_current(&first.token()));
    let picker = first.token();
    first
        .accept_path(&picker, PathBuf::from("/tmp/../tmp/東京.csv"))
        .unwrap();
    assert_eq!(first.path(), Some(Path::new("/tmp/../tmp/東京.csv")));
    assert!(!first.is_current(&picker));
    let reviewed = first.token();
    let mut options = first.options().clone();
    options.header = false;
    first.set_options(options).unwrap();
    assert!(!first.is_current(&reviewed));
    assert!(first.path().is_some());
    let picker = first.token();
    first.retarget("connection".into(), 4).unwrap();
    assert!(
        first
            .accept_path(&picker, PathBuf::from("/tmp/stale.csv"))
            .is_err()
    );
    assert!(first.path().is_none());
    assert!(first.target().is_none());
    first.set_target(Some(target())).unwrap();
    first
        .accept_path(&first.token(), PathBuf::from("/tmp/input.csv"))
        .unwrap();
    let data = inspection();
    let token = first.token();
    assert!(!first.matches_inspection(&token, &data)); // header differs.
    first.set_direction(CsvDirection::Export).unwrap();
    assert_eq!(first.options(), &CsvOptions::default());
    assert!(first.path().is_none());
    assert!(first.inspection_intent().is_ok());
    drop(first);
    drop(second);
    assert_eq!(shared.get(), 0);
}
#[test]
fn option_and_path_refusals_preserve_last_valid_setup() {
    let mut setup = setup(budget());
    setup
        .accept_path(&setup.token(), PathBuf::from("/tmp/input.csv"))
        .unwrap();
    let token = setup.token();
    let mut options = CsvOptions {
        null_token: "東京".repeat(32),
        ..Default::default()
    };
    assert!(validate_options(&options).is_ok());
    options.null_token.push('x');
    assert!(setup.set_options(options).is_err());
    assert!(setup.is_current(&token));
    assert!(
        setup
            .set_options(CsvOptions {
                delimiter: "é".into(),
                ..Default::default()
            })
            .is_err()
    );
    assert!(
        setup
            .accept_path(&token, PathBuf::from("relative.csv"))
            .is_err()
    );
    assert_eq!(setup.path(), Some(Path::new("/tmp/input.csv")));
    let mut excess = String::with_capacity(2048);
    excess.push_str("\\N");
    assert!(
        validate_options(&CsvOptions {
            null_token: excess,
            ..Default::default()
        })
        .is_err()
    );
}
#[test]
fn mapping_preserves_duplicate_and_blank_source_indices_and_required_targets() {
    let shared = budget();
    let data = inspection();
    let mut mapping = Mapping::new(&data, shared.clone()).unwrap();
    assert_eq!(mapping.target_index(0), Some(0));
    assert_eq!(mapping.target_index(1), None);
    assert_eq!(mapping.target_index(2), None);
    assert!(mapping.set(&data, 1, Some(0)).is_err());
    assert!(mapping.set(&data, 4, Some(3)).is_err());
    assert!(mapping.set(&data, 5, Some(4)).is_err());
    mapping.set(&data, 2, Some(1)).unwrap();
    mapping.set(&data, 3, None).unwrap();
    assert_eq!(mapping.first_missing_required(&data), Some("required"));
    assert!(mapping.validate(&data).is_err());
    mapping.set(&data, 1, Some(2)).unwrap();
    let request = mapping.to_backend(&data).unwrap();
    assert_eq!(
        request,
        vec![
            CsvMapping {
                source_index: 0,
                target_column: "a".into()
            },
            CsvMapping {
                source_index: 1,
                target_column: "required".into()
            },
            CsvMapping {
                source_index: 2,
                target_column: "blank".into()
            }
        ]
    );
    let mut replacement = inspection();
    replacement.source_columns[0].name = "a".into();
    assert!(mapping.set(&replacement, 0, None).is_err());
    assert_eq!(data.sample_rows[0][0], Some(String::new()));
    assert_eq!(data.sample_rows[0][1], None);
    assert_eq!(data.sample_rows[0][2], Some("\\N".into()));
    drop(mapping);
    assert_eq!(shared.get(), 0);
}

#[test]
fn mapping_cycles_past_occupied_generated_and_identity_targets() {
    let mut data = inspection();
    data.target_columns.push(column("last"));
    let mut mapping = Mapping::new(&data, budget()).unwrap();
    // Source 0 owns target 0; source 3 owns target 2. Generated 3 and identity 4
    // must not hide the free target beyond them.
    assert_eq!(mapping.adjacent_target(&data, 1, true).unwrap(), Some(1));
    mapping.set(&data, 1, Some(1)).unwrap();
    assert_eq!(mapping.adjacent_target(&data, 1, true).unwrap(), Some(5));
    mapping.set(&data, 1, Some(5)).unwrap();
    assert_eq!(mapping.adjacent_target(&data, 1, false).unwrap(), Some(1));
    assert!(mapping.adjacent_target(&inspection(), 1, true).is_err());
}
#[test]
fn captures_charge_overlap_and_preserve_missing_selection_and_zero_counts() {
    let shared = budget();
    let first = job();
    let key = first.attempt_id;
    let (inspections, jobs) = lists(vec![first], 1);
    let capture = Capture::new(inspections, jobs, shared.clone()).unwrap();
    assert_eq!(capture.index_for_key(key), Some(0));
    assert!(capture.details(0).unwrap().contains("Rows committed: 0"));
    shared.set(WORKSPACE_BYTES);
    let (a, b) = lists(vec![], 1);
    assert!(Capture::new(a, b, shared.clone()).is_err());
    assert_eq!(capture.key(0), Some(key));
    shared.set(1024 * 1024);
    let (a, b) = lists(vec![], 1);
    let replacement = Capture::new(a, b, shared.clone()).unwrap();
    assert_eq!(shared.get(), 2 * 1024 * 1024);
    assert_eq!(replacement.index_for_key(key), None);
    drop(replacement);
    drop(capture);
    assert_eq!(shared.get(), 0);
    let lease = Lease::inspection(shared.clone()).unwrap();
    assert_eq!(shared.get(), INSPECTION_CAPTURE_BYTES);
    drop(lease);
    assert_eq!(shared.get(), 0);
}
#[test]
fn import_revision_gaps_require_all_sources_and_only_ack_after_invalidation() {
    let mut tracker = ImportChanges::default();
    let plan = tracker.plan(2, &[(2, "b"), (1, "a")]).unwrap();
    assert_eq!(
        plan.connections().unwrap().collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    let stale = tracker.plan(1, &[(1, "a")]).unwrap();
    tracker.acknowledge(plan).unwrap();
    assert!(tracker.acknowledge(stale).is_err());
    assert!(tracker.plan(2, &[]).is_none());
    let gap = tracker.plan(4, &[(4, "a")]).unwrap();
    assert!(gap.connections().is_none());
    tracker.acknowledge(gap).unwrap();
    assert!(tracker.plan(4, &[]).is_none());
    let mut order = ObservationOrder::default();
    let before = order.issue().unwrap();
    order.fence().unwrap();
    assert!(!order.accept(before));
    let latest = order.issue().unwrap();
    assert!(order.accept(latest));
    assert!(!order.accept(latest + 1));
}
#[test]
fn capacity_and_admission_guards_refuse_hostile_or_unreleased_observations() {
    let mut oversized = inspection();
    oversized.sample_rows[0][0] = Some(String::with_capacity(MAX_CSV_INSPECTION_BYTES));
    assert!(Mapping::new(&oversized, budget()).is_err());
    let mut active = job();
    active.phase = CsvTransferPhase::Running;
    active.effect = CsvEffect::Pending;
    active.finished_at = None;
    active.cleanup = CsvCleanup::Pending;
    active.import_change_revision = None;
    let mut duplicate_connection = active.clone();
    duplicate_connection.attempt_id = CsvTransferAttemptId::new();
    let (a, b) = lists(vec![active, duplicate_connection], 0);
    assert!(Capture::new(a, b, budget()).is_err());
    let mut unknown = job();
    unknown.phase = CsvTransferPhase::OutcomeUnknown;
    unknown.effect = CsvEffect::Unknown;
    unknown.cleanup = CsvCleanup::Failed;
    let (a, b) = lists(vec![unknown], 1);
    let capture = Capture::new(a, b, budget()).unwrap();
    assert!(capture.has_active());
    assert!(capture.unknown_import_on("connection"));
    assert!(!releasable(capture.row(0).unwrap()));
}

#[test]
fn workbook_mode_invalidates_csv_paths_and_requires_workbook_provenance() {
    let allowance = budget();
    let mut setup = setup(allowance.clone());
    let picked = setup.token();
    setup
        .accept_path(&picked, PathBuf::from("/tmp/owned.csv"))
        .unwrap();
    let csv_token = setup.token();
    setup.set_xlsx(true).unwrap();
    assert!(setup.xlsx());
    assert!(setup.path().is_none());
    assert!(!setup.is_current(&csv_token));
    let token = setup.token();
    assert!(!setup.matches_inspection(&token, &inspection()));
    setup
        .accept_path(&token, PathBuf::from("/tmp/owned.xlsx"))
        .unwrap();
    assert!(setup.inspection_intent().is_ok());
    let workbook_token = setup.token();
    let mut data = inspection();
    data.workbook = Some(CsvWorkbookSource {
        sheet_index: 0,
        sheet_name: "Numbers".into(),
        workbook_bytes: 100,
        canonical_bytes: 50,
        null_token: setup.options().null_token.clone(),
        header_detected: true,
        rows: 1,
        cached_formula_cells: 0,
    });
    assert!(setup.matches_inspection(&workbook_token, &data));
    data.workbook.as_mut().unwrap().null_token = "different".into();
    assert!(!setup.matches_inspection(&workbook_token, &data));
    setup.set_xlsx(false).unwrap();
    assert!(!setup.xlsx());
    assert!(setup.path().is_none());
    assert!(!setup.is_current(&workbook_token));
    drop(setup);
    assert_eq!(allowance.get(), 0);
}

#[test]
fn workbook_progress_separates_original_file_from_canonical_transfer() {
    let mut value = job();
    value.workbook = Some(CsvWorkbookSource {
        sheet_index: 1,
        sheet_name: "Numbers".into(),
        workbook_bytes: 100,
        canonical_bytes: 300,
        null_token: "NULL".into(),
        header_detected: true,
        rows: 2,
        cached_formula_cells: 1,
    });
    value.total_bytes = Some(300);
    value.bytes_processed = 300;
    value.rows_processed = Some(2);
    value.rows_committed = Some(2);
    let capture = Capture::new(
        CsvInspectionList {
            inspections: vec![],
        },
        CsvTransferList {
            jobs: vec![value],
            import_change_revision: 1,
            execution_reserved_bytes: 0,
        },
        budget(),
    )
    .unwrap();
    let text = capture.details(0).unwrap();
    assert!(text.contains("Import XLSX"));
    assert!(text.contains("Workbook bytes: 100"));
    assert!(text.contains("Canonical CSV bytes: 300"));
    assert!(text.contains("XLSX sheet 2: Numbers"));
}
