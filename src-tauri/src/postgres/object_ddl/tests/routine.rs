//! Generator unit tests: functions and procedures.

use super::*;

#[test]
fn create_function_seals_the_body_in_a_body_derived_dollar_quote() {
    let preview = generate_object_ddl(&[function_with(
        "order_id integer",
        "numeric",
        "BEGIN\n  RETURN 1;\nEND;",
    )])
    .expect("function");
    assert_eq!(
        preview.statements[0].sql,
        "CREATE FUNCTION \"lifecycle\".\"order_total\"(order_id integer)\nRETURNS numeric\n\
         LANGUAGE plpgsql\nSTABLE\nAS $dbunk$\nBEGIN\n  RETURN 1;\nEND;\n$dbunk$;"
    );
    assert_eq!(
        preview.statements[0].summary,
        "Create function lifecycle.order_total(order_id integer)"
    );
    assert!(!preview.statements[0].destructive);

    // A body that mentions the default tag gets the next one.
    let preview =
        generate_object_ddl(&[function_with("", "text", "SELECT $dbunk$ literal $dbunk$")])
            .expect("tag collision");
    assert!(preview.statements[0].sql.contains("AS $dbunk1$\n"));
    assert!(preview.statements[0].sql.ends_with("$dbunk1$;"));
    assert_eq!(dollar_tag("x $dbunk$ $dbunk1$ y"), "$dbunk2$");

    // Statement boundaries and quote-closing attempts inside the body
    // still produce exactly one DDL statement.
    let hostile = "SELECT 1; DROP TABLE lifecycle.orders; $dbunk$; SELECT 2 $$";
    let preview = generate_object_ddl(&[function_with("", "integer", hostile)])
        .expect("hostile body is opaque");
    assert_eq!(preview.statements.len(), 1);
    assert!(preview.statements[0].sql.contains("AS $dbunk1$\n"));
    assert!(!preview.statements[0].destructive);
}

#[test]
fn create_function_header_attributes_and_or_replace() {
    let op = PgObjectOp::CreateFunction(CreateFunctionOp {
        schema: "s".into(),
        name: "f".into(),
        or_replace: true,
        arguments: "a int DEFAULT 1, VARIADIC rest text[]".into(),
        returns: "SETOF integer".into(),
        language: "sql".into(),
        body: "SELECT a".into(),
        volatility: PgVolatility::Immutable,
        strict: true,
        security_definer: true,
        parallel: Some(PgParallelSafety::Safe),
    });
    let preview = generate_object_ddl(&[op]).expect("function");
    let sql = &preview.statements[0].sql;
    assert!(sql.starts_with("CREATE OR REPLACE FUNCTION \"s\".\"f\"(a int DEFAULT 1, VARIADIC rest text[])\nRETURNS SETOF integer\nLANGUAGE sql\nIMMUTABLE STRICT SECURITY DEFINER PARALLEL SAFE\nAS $dbunk$\n"));
    assert!(preview.statements[0].destructive);
    assert!(preview.statements[0]
        .summary
        .starts_with("Create or replace function s.f("));

    let table_form = function_with("", "TABLE (id integer, name text)", "SELECT 1, 'x'");
    assert!(generate_object_ddl(&[table_form]).is_ok());
    assert!(generate_object_ddl(&[function_with("", "trigger", "BEGIN RETURN NEW; END")]).is_ok());
}

#[test]
fn routine_signature_fragments_refuse_clause_keywords_and_literals() {
    assert!(
        sole_invalid(function_with("", "int AS x", "SELECT 1")).contains("routine clause keyword")
    );
    assert!(
        sole_invalid(function_with("a int) LANGUAGE sql AS x --", "int", "x"))
            .contains("escapes its typed SQL context")
    );
    assert!(
        sole_invalid(function_with("a int LANGUAGE sql", "int", "x"))
            .contains("routine clause keyword")
    );
    assert!(sole_invalid(function_with("", "integer, text", "x")).contains("cannot contain a list"));
    assert!(sole_invalid(function_with("", "'text'", "x")).contains("cannot contain a literal"));
    assert!(sole_invalid(function_with("", "", "x")).contains("return type cannot be empty"));
    assert!(sole_invalid(function_with("", "int", "  \n")).contains("routine body cannot be empty"));
    // Keywords nested in parentheses are fine (TABLE(... ) column names
    // are identifiers, not clause starts).
    assert!(generate_object_ddl(&[function_with("", "TABLE (\"as\" int)", "x")]).is_ok());
    // Dollar signs are refused outright: a dollar quote would open a body.
    assert!(
        sole_invalid(function_with("a text DEFAULT $$x$$", "int", "x"))
            .contains("cannot contain a dollar sign")
    );
    assert!(sole_invalid(function_with("", "int$", "x")).contains("cannot contain a dollar sign"));
    // Quoted language names are valid and rendered as typed.
    let mut quoted_language = function_with("", "int", "x");
    if let PgObjectOp::CreateFunction(CreateFunctionOp { language, .. }) = &mut quoted_language {
        *language = "\"plpgsql\"".into();
    }
    assert!(generate_object_ddl(&[quoted_language])
        .expect("quoted language")
        .statements[0]
        .sql
        .contains("LANGUAGE \"plpgsql\"\n"));

    let mut bad_language = function_with("", "int", "x");
    if let PgObjectOp::CreateFunction(CreateFunctionOp { language, .. }) = &mut bad_language {
        *language = "plpgsql; DROP TABLE x".into();
    }
    assert!(sole_invalid(bad_language).contains("language must be a single identifier"));
}

#[test]
fn create_procedure_renders_language_and_security() {
    let op = PgObjectOp::CreateProcedure(CreateProcedureOp {
        schema: "lifecycle".into(),
        name: "bump_orders".into(),
        or_replace: true,
        arguments: String::new(),
        language: "sql".into(),
        body: "UPDATE lifecycle.orders SET retry_count = 1".into(),
        security_definer: true,
    });
    let preview = generate_object_ddl(&[op]).expect("procedure");
    assert_eq!(
        preview.statements[0].sql,
        "CREATE OR REPLACE PROCEDURE \"lifecycle\".\"bump_orders\"()\nLANGUAGE sql\nSECURITY DEFINER\n\
         AS $dbunk$\nUPDATE lifecycle.orders SET retry_count = 1\n$dbunk$;"
    );
    assert_eq!(
        preview.statements[0].summary,
        "Create or replace procedure lifecycle.bump_orders()"
    );
    assert!(preview.statements[0].destructive);
}
