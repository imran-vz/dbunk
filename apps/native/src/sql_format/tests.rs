use super::*;
fn tokens(sql: &str) -> Vec<(scanner::Kind, &str)> {
    scanner::scan(sql)
        .unwrap()
        .iter()
        .map(|t| (t.kind, &sql[t.start..t.end]))
        .collect()
}
fn preserved(sql: &str) -> String {
    let formatted = format_sql(sql).unwrap();
    let before = tokens(sql);
    let after = tokens(&formatted);
    assert_eq!(before.len(), after.len(), "token count change: {sql:?}");
    for ((kind, original), (new_kind, result)) in before.iter().zip(&after) {
        assert_eq!(kind, new_kind);
        if matches!(kind, Kind::Word | Kind::CaseOpen | Kind::CaseClose) {
            assert!(original == result || original.to_ascii_uppercase() == *result);
        } else {
            assert_eq!(original, result, "protected token changed: {sql:?}");
        }
    }
    assert_eq!(
        format_sql(&formatted).unwrap(),
        formatted,
        "not idempotent: {sql:?}"
    );
    let edits = format_edits(sql).unwrap();
    assert!(edits.len() <= MAX_EDITS);
    for edit in edits {
        let original = &sql[edit.range];
        if original
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0c))
        {
            assert!(edit.text.bytes().all(|b| matches!(b, b' ' | b'\n')));
        } else {
            assert_eq!(edit.text, original.to_ascii_uppercase());
        }
    }
    formatted
}
#[test]
fn useful_clause_layout_normalizes_keywords_and_multiple_statements() {
    let result = preserved("select id,name from public.users where id=$1; select 2;");
    assert_eq!(
        result,
        "SELECT\n  id,\n  name\nFROM\n  public.users\nWHERE\n  id=$1;\n\nSELECT\n  2;"
    );
    let result = preserved(
        "select a.id,b.name from a left outer join b on a.id=b.id group by a.id,b.name order by a.id;",
    );
    assert!(result.contains(
        "\nLEFT OUTER JOIN b\nON a.id=b.id\nGROUP BY\n  a.id,\n  b.name\nORDER BY\n  a.id;"
    ));
    preserved("with a as (select 1 as n), b as (select n from a) select * from b;");
    preserved("select case when a=1 then (select b from t) else 3 end from u;");
}
#[test]
fn comments_and_dollar_bodies_are_byte_exact() {
    for sql in [
        "select /* outer /* nested */ still comment */ 1;",
        "select /* first\n  body untouched */ 1;",
        "select 1-- comment ;\r\n+2;",
        "select $tag$a  b\nselect x; -- text$tag$ as body;",
        "select $$abc ' ; SELECT  1$$;",
        "do $$ begin raise notice 'a  b'; end $$;",
        "select $雪$雪 \n'body'$雪$;",
        "-- eof comment\n",
    ] {
        preserved(sql);
    }
    assert!(preserved("select 1--tail\nfrom t;").contains("--tail\n"));
}
#[test]
fn quoted_prefixes_escapes_and_unicode_are_exact() {
    for sql in [
        r"select E'a\'b  c', e'\n';",
        r"select E'\\', E'\\\'x';",
        r"select U&'d\0061t\+000061', U&'d!0061t' UESCAPE '!';",
        r#"select U&"d\0061t" from public.t;"#,
        "select B'101010', X'FF', b'01', x'ff', N'x', n'x';",
        "select 'é雪😀é', \"雪 name\", \"a\"\"b\" from \"Sch.é\".\"T\";",
        "select café, 雪 from public.表;",
        "select 'a\u{a0}b', \"a\u{a0}b\";",
    ] {
        preserved(sql);
    }
}
#[test]
fn continuation_gaps_remain_exact_even_across_comments() {
    for gap in [
        "\n",
        " \r\n\t",
        " /*comment*/ ",
        " /*one\ntwo*/ ",
        " -- join\n",
    ] {
        let sql = format!("select 'foo'{gap}'bar';");
        let result = preserved(&sql);
        assert!(result.contains(&format!("'foo'{gap}'bar'")));
    }
    preserved("select E'first'\n'second';");
    assert_eq!(
        format_sql("select E'first'\n'\\\'';"),
        Err(FormatError::AmbiguousString)
    );
}
#[test]
fn maximal_operators_identifiers_parameters_and_numbers_do_not_split() {
    for sql in [
        "select data->'a',data->>'b',data#>>'{a,b}',data ?| array['x'],data @> '{}'::jsonb from t;",
        "select a <@ b,a @@ b,a !~* b,a OPERATOR(public.===) b from t;",
        "select a=-1,a-1,a - -1,a+-1,a#b from t;",
        "select foo$bar,$1,:ID,:id,:name::text;",
        "select 1_000,0xFF,0b101,0o777,1.2e+10,.5,1.,9223372036854775807;",
        "select ARRAY[1,2],a[1:2] from t;",
        "select 1 . 2, a . b from t;",
    ] {
        preserved(sql);
    }
    assert!(preserved("select a <@ b;").contains("a <@ b"));
    assert!(preserved("select a @> b;").contains("a @> b"));
}
#[test]
fn incomplete_or_ambiguous_input_never_produces_partial_edits() {
    for sql in [
        "select 'unterminated",
        "select \"unterminated",
        "select $tag$body",
        "select /* unfinished",
    ] {
        assert_eq!(format_edits(sql), Err(FormatError::Incomplete));
    }
    for sql in ["select (1;", "select [1);", "select case when true then 1;"] {
        assert_eq!(format_edits(sql), Err(FormatError::Unbalanced));
    }
    assert_eq!(
        format_edits("select 'a\\'b';"),
        Err(FormatError::AmbiguousString)
    );
    assert_eq!(
        format_edits("select a\u{a0}b from t;"),
        Err(FormatError::UnsupportedWhitespace)
    );
    assert_eq!(format_edits("select \0;"), Err(FormatError::InvalidToken));
    assert_eq!(
        format_edits("select $1suffix;"),
        Err(FormatError::InvalidToken)
    );
    assert_eq!(format_edits(" \t\n").unwrap(), []);
}
#[test]
fn input_tokens_depth_and_output_are_bounded_before_growth() {
    assert_eq!(
        format_edits(&" ".repeat(MAX_INPUT_BYTES + 1)),
        Err(FormatError::InputLimit)
    );
    assert!(
        format_edits(&" ".repeat(MAX_INPUT_BYTES))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        format_edits(&"a ".repeat(MAX_TOKENS + 1)),
        Err(FormatError::TokenLimit)
    );
    assert!(format_edits(&"a ".repeat(MAX_TOKENS)).is_ok());
    let nested = format!("{}1{}", "(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
    assert!(format_edits(&nested).is_ok());
    let nested = format!(
        "{}1{}",
        "(".repeat(MAX_DEPTH + 1),
        ")".repeat(MAX_DEPTH + 1)
    );
    assert_eq!(format_edits(&nested), Err(FormatError::DepthLimit));
    let nested = format!(
        "{}x{}",
        "/*".repeat(MAX_DEPTH + 1),
        "*/".repeat(MAX_DEPTH + 1)
    );
    assert_eq!(format_edits(&nested), Err(FormatError::DepthLimit));
    let sql = format!(
        "{}select {}{}",
        "(".repeat(MAX_DEPTH),
        "x,".repeat(500),
        ")".repeat(MAX_DEPTH)
    );
    assert!(sql.len() < MAX_INPUT_BYTES);
    assert_eq!(format_edits(&sql), Err(FormatError::OutputLimit));
}
#[test]
fn model_working_allowance_covers_all_simultaneous_storage() {
    let tokens = MAX_TOKENS * std::mem::size_of::<Token>();
    let gaps = MAX_EDITS * std::mem::size_of::<Gap>();
    let edits = MAX_EDITS * std::mem::size_of::<FormatEdit>();
    let keyword_flags = MAX_TOKENS * std::mem::size_of::<bool>();
    // Caller-owned input + exact-capacity output/edit text + bounded vectors.
    let bound = MAX_INPUT_BYTES + MAX_OUTPUT_BYTES * 2 + tokens + gaps + edits + keyword_flags;
    assert!(bound < WORKING_BYTES);
}

#[test]
fn keyword_case_preserves_types_functions_names_and_bound_parameters() {
    let sql = "select count(*),cast(x as integer),current_date,coalesce(x,1),a::timestamp with time zone,\"select\",t.select,t./* name */case,:select,$1 from MyTable;";
    let result = preserved(sql);
    let words: Vec<_> = tokens(&result).into_iter().map(|(_, text)| text).collect();
    assert_eq!(
        words,
        vec![
            "SELECT",
            "count",
            "(",
            "*",
            ")",
            ",",
            "cast",
            "(",
            "x",
            "AS",
            "integer",
            ")",
            ",",
            "current_date",
            ",",
            "coalesce",
            "(",
            "x",
            ",",
            "1",
            ")",
            ",",
            "a",
            "::",
            "timestamp",
            "with",
            "time",
            "zone",
            ",",
            "\"select\"",
            ",",
            "t",
            ".",
            "select",
            ",",
            "t",
            ".",
            "/* name */",
            "case",
            ",",
            ":select",
            ",",
            "$1",
            "FROM",
            "MyTable",
            ";"
        ]
    );
    preserved(
        "select 'select',E'from',U&'where',B'01',X'ff',n'and',$tag$select from$tag$,select$Id,éselect from t;",
    );
}

#[test]
fn keyword_phrases_do_not_case_unrelated_identifiers_or_span_comments() {
    let result = preserved(
        "create table t (id integer generated by default as identity primary key, stamp timestamp without time zone); select key,value,role,by,rows,first,last from t order by value nulls first;",
    );
    assert!(result.contains("GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY"));
    assert!(result.contains("NULLS FIRST"));
    let result = preserved("select x from t group /* boundary */ by x;");
    assert!(result.contains("GROUP /* boundary */ by x"));
    // Type phrases have precedence over WITH. Clause phrases have precedence
    // over function names such as LEFT; standalone left() retains its spelling.
    let result = preserved("select left(x,1) from a left join b on a.id=b.id;");
    assert!(result.contains("left(x, 1)"));
    assert!(result.contains("LEFT JOIN"));
}

#[test]
fn maximal_keyword_and_gap_edits_are_ordered_and_bounded() {
    let sql = " grant ".repeat(MAX_TOKENS);
    let edits = format_edits(&sql).unwrap();
    assert_eq!(edits.len(), MAX_EDITS);
    assert!(
        edits
            .windows(2)
            .all(|pair| pair[0].range.end <= pair[1].range.start)
    );
    preserved(&sql);
}
