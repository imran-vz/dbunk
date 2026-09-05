use super::csv::{
    encode_cell, CsvCell, CsvError, CsvOptions, Parser, CSV_OUTPUT_CHUNK_BYTES, MAX_COLUMNS,
    MAX_FIELD_BYTES, MAX_RECORD_BYTES,
};

fn parse(input: &[u8], options: &CsvOptions) -> Result<Vec<Vec<CsvCell>>, CsvError> {
    let mut parser = Parser::new(options)?;
    let mut records = Vec::new();
    for &byte in input {
        if let Some(record) = parser.push(byte)? {
            records.push(record);
        }
    }
    if let Some(record) = parser.finish()? {
        records.push(record);
    }
    Ok(records)
}

fn encode(value: Option<&str>, options: &CsvOptions) -> Vec<u8> {
    encode_cell(value, options)
        .expect("valid encoder options")
        .flatten()
        .collect()
}

#[test]
fn defaults_and_camel_case_options_are_stable() {
    let options = CsvOptions::default();
    assert_eq!(options.delimiter, ",");
    assert_eq!(options.quote, "\"");
    assert_eq!(options.escape, "\"");
    assert_eq!(options.null_token, "\\N");
    assert!(options.header);
    options.validate().expect("default options are valid");

    let serialized = serde_json::to_value(&options).expect("options serialize");
    assert_eq!(serialized["nullToken"], "\\N");
    assert!(serialized.get("null_token").is_none());

    let partial: CsvOptions =
        serde_json::from_str(r#"{"header":false}"#).expect("missing settings use defaults");
    assert_eq!(partial.delimiter, ",");
    assert!(!partial.header);
}

#[test]
fn invalid_options_return_structural_errors_without_values() {
    let cases = [
        CsvOptions {
            delimiter: "||".into(),
            ..CsvOptions::default()
        },
        CsvOptions {
            quote: ",".into(),
            ..CsvOptions::default()
        },
        CsvOptions {
            null_token: "bad,value".into(),
            ..CsvOptions::default()
        },
    ];

    for options in cases {
        let error = options.validate().expect_err("options must be rejected");
        assert_eq!(error.record, 0);
        assert_eq!(error.column, None);
        assert!(!error.reason.contains("||"));
        assert!(!error.reason.contains("bad,value"));
    }
}

#[test]
fn parses_one_byte_at_a_time_with_quoted_newlines_and_crlf() {
    let input = b"name,notes\r\nalice,\"line one\nline two\"\r\nbob,\"said \"\"hi\"\"\"\n";
    let rows = parse(input, &CsvOptions::default()).expect("CSV parses");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0].value, "name");
    assert_eq!(rows[1][1].value, "line one\nline two");
    assert!(rows[1][1].quoted);
    assert_eq!(rows[2][1].value, "said \"hi\"");
}

#[test]
fn parses_a_distinct_escape_character() {
    let options = CsvOptions {
        escape: "\\".into(),
        ..CsvOptions::default()
    };
    let rows = parse(
        br#""a\"b\\c\path",tail
"#,
        &options,
    )
    .expect("distinct escape parses");

    assert_eq!(rows[0][0].value, "a\"b\\c\\path");
    assert!(rows[0][0].quoted);
    assert_eq!(rows[0][1].value, "tail");
}

#[test]
fn handles_a_bom_incrementally_and_rejects_invalid_utf8() {
    let input = b"\xef\xbb\xbfword,\xe2\x82\xac\n";
    let rows = parse(input, &CsvOptions::default()).expect("BOM and UTF-8 parse");
    assert_eq!(rows[0][0].value, "word");
    assert_eq!(rows[0][1].value, "€");

    let error = parse(b"ok,\xff\n", &CsvOptions::default()).expect_err("invalid UTF-8 fails");
    assert_eq!(error.record, 1);
    assert_eq!(error.column, Some(2));
    assert_eq!(error.reason, "field contains invalid UTF-8");

    let mut partial_bom = Parser::new(&CsvOptions::default()).expect("parser constructs");
    partial_bom.push(0xef).expect("partial BOM is buffered");
    assert!(partial_bom.has_pending_input());
    let error = partial_bom
        .finish()
        .expect_err("partial BOM is invalid UTF-8");
    assert_eq!(error.reason, "field contains invalid UTF-8");
}

#[test]
fn reports_preview_pending_state_without_finishing_a_partial_record() {
    let mut parser = Parser::new(&CsvOptions::default()).expect("parser constructs");
    for byte in UTF8_BOM_FOR_TEST {
        assert!(parser.push(byte).expect("BOM byte parses").is_none());
    }
    assert!(!parser.has_pending_input());

    for byte in b"partial,\"row" {
        assert!(parser.push(*byte).expect("partial row parses").is_none());
    }
    assert!(parser.has_pending_input());
    assert_eq!(parser.next_record_number(), 1);
}

const UTF8_BOM_FOR_TEST: [u8; 3] = [0xef, 0xbb, 0xbf];

#[test]
fn preserves_null_markers_and_empty_strings_through_encoding() {
    let options = CsvOptions::default();
    let source = parse(b"\\N,\"\\N\",,\"\"\n", &options).expect("source parses");
    let source = &source[0];
    assert!(source[0].is_null(&options));
    assert!(!source[1].is_null(&options));
    assert!(!source[2].is_null(&options));
    assert!(!source[3].is_null(&options));

    let order = [1, 0, 3, 2];
    let mut output = Vec::new();
    for (output_index, &source_index) in order.iter().enumerate() {
        if output_index != 0 {
            output.push(b',');
        }
        let cell = &source[source_index];
        let value = (!cell.is_null(&options)).then_some(cell.value.as_str());
        output.extend(encode(value, &options));
    }
    output.push(b'\n');

    let round_trip = parse(&output, &options).expect("encoded row parses");
    assert_eq!(round_trip[0][0].value, "\\N");
    assert!(round_trip[0][0].quoted);
    assert!(round_trip[0][1].is_null(&options));
    assert_eq!(round_trip[0][2].value, "");
    assert!(round_trip[0][2].quoted);
    assert_eq!(round_trip[0][3].value, "");
    assert!(round_trip[0][3].quoted);
}

#[test]
fn encoder_chunks_large_escaped_values_and_round_trips_them() {
    let options = CsvOptions::default();
    let value = "\"".repeat(MAX_FIELD_BYTES);
    let chunks = encode_cell(Some(&value), &options)
        .expect("bounded value encodes")
        .collect::<Vec<_>>();
    assert!(chunks.len() > 1);
    assert!(chunks
        .iter()
        .all(|chunk| !chunk.is_empty() && chunk.len() <= CSV_OUTPUT_CHUNK_BYTES));

    let mut encoded = chunks.concat();
    encoded.push(b'\n');
    let decoded = parse(&encoded, &options).expect("encoded value parses");
    assert_eq!(decoded[0][0].value, value);
}

#[test]
fn rejects_malformed_quotes_and_line_endings_at_the_exact_record() {
    let cases: &[(&[u8], &str)] = &[
        (b"ok\na\"b\n", "unexpected quote in unquoted field"),
        (b"ok\n\"open", "unterminated quoted field"),
        (
            b"ok\n\"closed\"x",
            "unexpected character after closing quote",
        ),
        (
            b"ok\nsecond\rthird",
            "carriage return must be followed by line feed",
        ),
    ];

    for &(input, reason) in cases {
        let error = parse(input, &CsvOptions::default()).expect_err("malformed CSV fails");
        assert_eq!(error.record, 2);
        assert_eq!(error.reason, reason);
        assert!(!error.to_string().contains("closed"));
        assert!(!error.to_string().contains("third"));
    }
}

#[test]
fn enforces_field_limit_before_growing_the_field() {
    let options = CsvOptions::default();
    let mut parser = Parser::new(&options).expect("parser constructs");
    for _ in 0..MAX_FIELD_BYTES {
        parser.push(b'a').expect("bytes through the limit fit");
    }
    let error = parser.push(b'a').expect_err("byte beyond the limit fails");
    assert_eq!(error.record, 1);
    assert_eq!(error.column, Some(1));
    assert_eq!(error.reason, "field exceeds the 1 MiB limit");
}

#[test]
fn enforces_record_limit_before_accepting_another_byte() {
    let options = CsvOptions::default();
    let mut parser = Parser::new(&options).expect("parser constructs");
    for column in 0..8 {
        let field_len = if column == 7 {
            MAX_FIELD_BYTES
        } else {
            MAX_FIELD_BYTES - 1
        };
        for _ in 0..field_len {
            parser.push(b'a').expect("record through limit fits");
        }
        if column != 7 {
            parser.push(b',').expect("delimiter through limit fits");
        }
    }
    debug_assert_eq!(
        7 * (MAX_FIELD_BYTES - 1) + 7 + MAX_FIELD_BYTES,
        MAX_RECORD_BYTES
    );
    let error = parser
        .push(b'a')
        .expect_err("byte beyond record limit fails");
    assert_eq!(error.record, 1);
    assert_eq!(error.column, None);
    assert_eq!(error.reason, "record exceeds the 8 MiB limit");
}

#[test]
fn accepts_1600_columns_and_rejects_a_1601st() {
    let options = CsvOptions::default();
    let valid = format!("{}\n", ",".repeat(MAX_COLUMNS - 1));
    let rows = parse(valid.as_bytes(), &options).expect("1600 fields fit");
    assert_eq!(rows[0].len(), MAX_COLUMNS);

    let invalid = ",".repeat(MAX_COLUMNS);
    let error = parse(invalid.as_bytes(), &options).expect_err("1601st field fails");
    assert_eq!(error.record, 1);
    assert_eq!(error.column, Some(MAX_COLUMNS + 1));
    assert_eq!(error.reason, "record exceeds the 1600-column limit");
}

#[test]
fn header_rows_are_returned_for_the_caller_to_handle() {
    let rows = parse(b"name,name,,name\n1,2,3,4\n", &CsvOptions::default())
        .expect("duplicate and blank headers parse");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0]
            .iter()
            .map(|cell| cell.value.as_str())
            .collect::<Vec<_>>(),
        ["name", "name", "", "name"]
    );
}
