use super::*;

#[test]
fn planner_rewrite_preserves_exact_text_and_parameter_grammar_without_values() {
    let sql = "; SELECT :id::bigint, ':secret', $$:hidden$$, ARRAY[:name, :id], a[1:n], :name FROM public.rows -- 東京\n; ";
    let source = QueryMutationSource::new(sql.into(), true).unwrap();
    assert_eq!(source.original_sql(), sql);
    assert_eq!(source.parameter_names(), &["id", "name"]);
    assert_eq!(
        source.statement_sql(),
        "SELECT $1::bigint, ':secret', $$:hidden$$, ARRAY[$2, $1], a[1:n], $2 FROM public.rows"
    );
    assert!(source.parameter_mode());
    let encoded = serde_json::to_string(&source).unwrap();
    assert_eq!(
        serde_json::from_str::<QueryMutationSource>(&encoded).unwrap(),
        source
    );
    assert_eq!(
        source.analysis_source().unwrap(),
        AnalyzeSource::NativeStatement {
            sql: source.statement_sql().into()
        }
    );
    assert!(!format!("{source:?}").contains("secret"));
    let raw = QueryMutationSource::new("SELECT :id FROM public.rows".into(), false).unwrap();
    assert_eq!(raw.statement_sql(), "SELECT :id FROM public.rows");
    assert!(raw.parameter_names().is_empty());
    let empty_mode =
        QueryMutationSource::new("SELECT ':id' FROM public.rows".into(), true).unwrap();
    assert!(empty_mode.parameter_names().is_empty());
}

#[test]
fn forged_derived_fields_and_unknown_fields_are_rejected_on_decode() {
    let source =
        QueryMutationSource::new("SELECT :id, :name, :id FROM public.rows".into(), true).unwrap();
    let value = serde_json::to_value(source).unwrap();
    for (key, forged) in [
        ("statementSql", serde_json::json!("SELECT $2, $1, $2")),
        ("parameterMode", serde_json::json!(false)),
        ("parameterNames", serde_json::json!(["name", "id"])),
        ("boundValues", serde_json::json!(["secret"])),
    ] {
        let mut changed = value.clone();
        changed[key] = forged;
        assert!(
            serde_json::from_value::<QueryMutationSource>(changed).is_err(),
            "{key}"
        );
    }
}

#[test]
fn statement_and_input_limits_refuse_before_source_creation() {
    for sql in [
        "",
        "; ;",
        "SELECT 1; SELECT 2",
        "SELECT 'unterminated",
        "SELECT (1",
        "SELECT \0",
    ] {
        assert!(
            QueryMutationSource::new(sql.into(), false).is_err(),
            "{sql:?}"
        );
    }
    assert_eq!(
        QueryMutationSource::new("x".repeat(MAX_QUERY_SOURCE_BYTES + 1), false),
        Err(QueryMutationSourceError::TooLarge)
    );
    assert_eq!(
        QueryMutationSource::new(format!("SELECT :{}", "a".repeat(64)), true),
        Err(QueryMutationSourceError::ParameterNameTooLong)
    );
    let sql = format!(
        "SELECT {} FROM public.rows",
        (0..257)
            .map(|i| format!(":p{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(
        QueryMutationSource::new(sql, true),
        Err(QueryMutationSourceError::TooManyParameters)
    );
    assert_eq!(
        QueryMutationSource::new("SELECT $1, :id".into(), true),
        Err(QueryMutationSourceError::ParametersRejected)
    );
    let sql = format!(
        "SELECT {} FROM public.rows",
        (0..256)
            .map(|i| format!(":p{i}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    assert_eq!(
        QueryMutationSource::new(sql, true)
            .unwrap()
            .parameter_names()
            .len(),
        256
    );
}

#[test]
fn dense_sql_is_refused_before_planner_and_rewrites_stay_within_token_cap() {
    let dense = format!("SELECT ({})", "1,".repeat(MAX_QUERY_SOURCE_TOKENS));
    assert!(dense.len() < MAX_QUERY_SOURCE_BYTES);
    for mode in [false, true] {
        assert_eq!(
            QueryMutationSource::new(dense.clone(), mode),
            Err(QueryMutationSourceError::TooManyTokens)
        );
    }
    // Repeated names, including $256, collapse two lexical tokens to one.
    let sql = format!(
        "SELECT {} FROM public.rows",
        (0..4096)
            .map(|i| format!(":p{}", i % 256))
            .collect::<Vec<_>>()
            .join(",")
    );
    let source = QueryMutationSource::new(sql.clone(), true).unwrap();
    let original_count = lex_sql_spanned_bounded(&sql, MAX_QUERY_SOURCE_TOKENS)
        .unwrap()
        .len();
    let rewritten_count = lex_sql_spanned_bounded(source.statement_sql(), MAX_QUERY_SOURCE_TOKENS)
        .unwrap()
        .len();
    assert!(rewritten_count < original_count);
    // A large quoted literal stays one token; the byte limit remains independent.
    let wrapper = "SELECT '' FROM public.rows";
    let sql = format!(
        "SELECT '{}' FROM public.rows",
        "x".repeat(MAX_QUERY_SOURCE_BYTES - wrapper.len())
    );
    assert_eq!(sql.len(), MAX_QUERY_SOURCE_BYTES);
    assert!(QueryMutationSource::new(sql, false).is_ok());
}

#[test]
fn separate_session_targets_require_all_qualified_non_temp_range_variables() {
    for sql in [
        "SELECT a.id, b.value FROM public.a a JOIN other.b b ON a.id=b.id",
        "SELECT a.id FROM \"Mixed.Schema\".\"Odd Rows\" a",
        "SELECT * FROM database_name.public.rows",
        "SELECT id FROM public.rows WHERE note='FROM hidden JOIN pg_temp.secret'",
    ] {
        assert!(QueryMutationSource::new(sql.into(), false).is_ok(), "{sql}");
    }
    for sql in [
        "SELECT * FROM rows",
        "SELECT a.id FROM public.rows a JOIN other b ON a.id=b.id",
        "SELECT * FROM pg_temp.rows",
        "SELECT * FROM pg_temp_123.rows",
        "SELECT * FROM \"pg_temp\".rows",
        "SELECT * FROM PG_TEMP.rows",
        "SELECT 1",
        "SELECT * FROM public.f()",
        "SELECT * FROM (SELECT * FROM public.rows) a",
        "WITH a AS (SELECT * FROM public.rows) SELECT * FROM a",
    ] {
        assert_eq!(
            QueryMutationSource::new(sql.into(), false),
            Err(QueryMutationSourceError::SessionDependentTarget),
            "{sql}"
        );
        let stored = serde_json::json!({ "originalSql":sql, "statementSql":sql, "parameterMode":false, "parameterNames":[] });
        assert!(serde_json::from_value::<QueryMutationSource>(stored).is_err());
    }
}
