use super::*;
use crate::results::{Completion, ResultSet};
use dbunk_lib::backend::QueryExecutionContext;

fn completed() -> ResultModel {
    let mut model = ResultModel::default();
    model.sets.push(ResultSet {
        columns: vec![Some("id".into()), Some("name".into())],
        rows: vec![vec![Some("1".into()), Some("東京".into())].into()],
        row_count: Some(1),
        ..ResultSet::default()
    });
    model.completion = Some(Completion {
        status: TerminalStatus::Completed,
        omitted_rows: 0,
        omitted_result_sets: 0,
        omitted_notices: 0,
        omitted_metadata_bytes: 0,
        truncation_reasons: vec![],
        error: None,
        refusal: None,
    });
    model
}

#[test]
fn provenance_is_exact_parameter_rewrite_and_runtime_identity_not_current_editor() {
    let budget = Rc::new(Cell::new(7));
    let sql = "SELECT id, name FROM public.rows WHERE id = :id";
    let source = ExecutedSource::prepare(
        sql,
        true,
        "connection",
        "session",
        "execution",
        budget.clone(),
    )
    .unwrap();
    assert!(source.matches("connection", "session", "execution"));
    assert!(!source.matches("connection", "session", "next"));
    assert!(!source.matches("other", "session", "execution"));
    assert_eq!(source.provenance.source.original_sql(), sql);
    assert_eq!(source.provenance.source.parameter_names(), ["id"]);
    assert!(source.provenance.source.statement_sql().contains("$1"));
    let model = completed();
    let rows = QueryRows::capture(source.clone(), &model, budget.clone()).unwrap();
    assert!(Rc::ptr_eq(&model.sets[0].rows[0], &rows.rows[0]));
    assert!(Rc::ptr_eq(&source, &rows.origin));
    drop(rows);
    drop(source);
    assert_eq!(budget.get(), 7);
}

#[test]
fn capture_refuses_partial_values_unknown_disclosure_and_failed_execution() {
    let budget = Rc::new(Cell::new(0));
    let source = ExecutedSource::prepare(
        "SELECT id, name FROM public.rows",
        false,
        "c",
        "s",
        "e",
        budget.clone(),
    )
    .unwrap();
    let mut model = completed();
    for reason in ["cellBytes", "rowBytes", "newUnknownReason"] {
        model.completion.as_mut().unwrap().truncation_reasons = vec![reason.into()];
        assert!(QueryRows::capture(source.clone(), &model, budget.clone()).is_err());
    }
    model.completion.as_mut().unwrap().truncation_reasons = vec!["rowCount".into()];
    assert!(QueryRows::capture(source.clone(), &model, budget.clone()).is_ok());
    model.native_omitted_metadata = 1;
    assert!(QueryRows::capture(source.clone(), &model, budget.clone()).is_err());
    model.native_omitted_metadata = 0;
    for status in [TerminalStatus::Cancelled, TerminalStatus::Failed] {
        model.completion.as_mut().unwrap().status = status;
        assert!(QueryRows::capture(source.clone(), &model, budget.clone()).is_err());
    }
    model.completion.as_mut().unwrap().status = TerminalStatus::Completed;
    model.sets[0].columns[0] = None;
    assert!(QueryRows::capture(source.clone(), &model, budget.clone()).is_err());
}

#[test]
fn source_and_capture_refusal_preserve_other_budget_owners() {
    let budget = Rc::new(Cell::new(WORKSPACE_BYTES - SOURCE_WORK_BYTES + 1));
    let before = budget.get();
    assert!(
        ExecutedSource::prepare(
            "SELECT id FROM public.rows",
            false,
            "c",
            "s",
            "e",
            budget.clone()
        )
        .is_err()
    );
    assert_eq!(budget.get(), before);
    budget.set(0);
    assert!(
        ExecutedSource::prepare("SELECT 1; SELECT 2", false, "c", "s", "e", budget.clone())
            .is_err()
    );
    assert_eq!(budget.get(), 0);
    let source = ExecutedSource::prepare(
        "SELECT id FROM public.rows",
        false,
        "c",
        "s",
        "e",
        budget.clone(),
    )
    .unwrap();
    let owned = budget.get();
    budget.set(WORKSPACE_BYTES);
    assert!(QueryRows::capture(source.clone(), &completed(), budget.clone()).is_err());
    assert_eq!(budget.get(), WORKSPACE_BYTES);
    budget.set(owned);
    drop(source);
    assert_eq!(budget.get(), 0);
}

#[test]
fn encoding_guard_refusal_preserves_unicode_replacements_but_never_reinterprets_old_literals() {
    use dbunk_lib::backend::data::{MutationOp, MutationTable, MutationValue};
    let value = |column: &str, text: &str| MutationValue {
        column: column.into(),
        value: Some(text.into()),
    };
    let mut operation = MutationOp::Update {
        table: MutationTable {
            schema: "public".into(),
            table: "rows".into(),
        },
        identity: vec![value("id", "1")],
        guards: vec![value("name", "before")],
        set: vec![value("name", "東京")],
    };
    assert!(guards_supported(&operation, false));
    let MutationOp::Update { guards, .. } = &mut operation else {
        unreachable!()
    };
    guards[0].value = Some("東京".into());
    assert!(!guards_supported(&operation, false));
    // Only this provenance's proven UTF8 execution admits the same literal.
    assert!(guards_supported(&operation, true));
}

#[test]
fn many_short_parameter_names_charge_owned_headers_and_analysis_statement_copy() {
    let budget = Rc::new(Cell::new(0));
    let filters = (0..256)
        .map(|index| format!("id = :p{index}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!("SELECT id FROM public.rows WHERE {filters}");
    let source = ExecutedSource::prepare(&sql, true, "c", "s", "e", budget.clone()).unwrap();
    let provenance = &source.provenance.source;
    let names = provenance.parameter_names();
    assert_eq!(names.len(), 256);
    let lower_bound = provenance.original_sql().len()
        + 2 * provenance.statement_sql().len()
        + std::mem::size_of_val(names)
        + names.iter().map(String::len).sum::<usize>();
    assert!(budget.get() >= lower_bound);
    drop(source);
    assert_eq!(budget.get(), 0);
}

fn utf8_context() -> Box<QueryExecutionContext> {
    Box::new(QueryExecutionContext {
        client_encoding: Some("UTF8".into()),
        date_style: Some("ISO, MDY".into()),
        interval_style: Some("postgres".into()),
        search_path: Some("\"$user\", public".into()),
        autocommit: true,
    })
}

fn unicode_analysis() -> AnalyzeResultSetResult {
    use dbunk_lib::backend::data::{
        AnalysisStatement, AnalyzedColumn, AnalyzedTable, CapabilityVerdict, ColumnOrigin,
        ColumnWritability, MutationIdentity, MutationIdentityKind,
    };
    let allowed = CapabilityVerdict {
        allowed: true,
        reason: None,
    };
    AnalyzeResultSetResult {
        request_id: 1,
        analysis_id: 2,
        statement: AnalysisStatement::Analyzed,
        columns: ["id", "name"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| AnalyzedColumn {
                name: name.into(),
                origin: ColumnOrigin::Table {
                    schema: "public".into(),
                    table: "rows".into(),
                    column: name.into(),
                    attnum: index as i16 + 1,
                },
                cast_type: "text".into(),
                nullable: name != "id",
                writability: ColumnWritability::Writable,
            })
            .collect(),
        tables: vec![AnalyzedTable {
            schema: "public".into(),
            table: "rows".into(),
            identity: MutationIdentity {
                kind: MutationIdentityKind::PrimaryKey,
                columns: vec!["id".into()],
            },
            identity_projected: true,
            identity_projection_indexes: vec![0],
            updatable: allowed.clone(),
            deletable: allowed.clone(),
            insertable: allowed,
        }],
    }
}

#[test]
fn unicode_identity_and_guards_need_the_matched_executions_reported_utf8() {
    let analysis = unicode_analysis();
    let row = [Some("東京🙂".to_owned()), Some("before ü".to_owned())];
    assert!(editable_target(&analysis, &row, 1, false).is_err());
    assert_eq!(
        editable_target(&analysis, &row, 1, true).unwrap(),
        (0, "name".to_owned())
    );
    // ASCII rows never needed the proof.
    let ascii = [Some("1".to_owned()), Some("before".to_owned())];
    assert!(editable_target(&analysis, &ascii, 1, false).is_ok());
}

#[test]
fn execution_context_attaches_once_is_charged_and_restored_sources_have_none() {
    let budget = Rc::new(Cell::new(0));
    let source = ExecutedSource::prepare(
        "SELECT id, name FROM rows",
        false,
        "c",
        "s",
        "e",
        budget.clone(),
    )
    .unwrap();
    // Unqualified sources prepare, but cannot analyze without a context.
    assert!(!source.provenance.source.qualified());
    assert!(source.provenance.analysis_source().is_err());
    assert!(!source.provenance.utf8());
    let before = budget.get();
    assert!(!source.complete(None));
    assert!(source.complete(Some(utf8_context())));
    assert!(budget.get() > before);
    assert!(source.provenance.utf8());
    assert!(source.provenance.analysis_source().is_ok());
    // A second completion can never replace the matched execution's context.
    let mut latin1 = utf8_context();
    latin1.client_encoding = Some("LATIN1".into());
    assert!(!source.complete(Some(latin1)));
    assert!(source.provenance.utf8());

    let restored = Provenance::restore(source.provenance.source.clone(), budget.clone());
    assert!(restored.context().is_none());
    assert!(!restored.utf8());
    assert!(restored.analysis_source().is_err());
    drop(restored);
    drop(source);
    assert_eq!(budget.get(), 0);
}

#[test]
fn a_context_over_the_shared_allowance_is_dropped_not_retained() {
    let budget = Rc::new(Cell::new(0));
    let source = ExecutedSource::prepare(
        "SELECT id FROM public.rows",
        false,
        "c",
        "s",
        "e",
        budget.clone(),
    )
    .unwrap();
    let owned = budget.get();
    budget.set(WORKSPACE_BYTES);
    assert!(!source.complete(Some(utf8_context())));
    assert!(source.provenance.context().is_none());
    assert_eq!(budget.get(), WORKSPACE_BYTES);
    budget.set(owned);
    // The qualified subset remains available without a context.
    assert!(source.provenance.analysis_source().is_ok());
    drop(source);
    assert_eq!(budget.get(), 0);
}
