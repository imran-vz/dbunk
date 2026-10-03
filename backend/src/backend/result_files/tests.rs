use super::*;
use calamine::{Data, Reader, Xlsx};
use std::io::Read;

fn source(rows: usize) -> Source {
    Source {
        completeness: Completeness::Complete,
        scope: Scope::CompleteResult,
        row_count: rows,
    }
}
fn table<'a>(columns: &'a [&'a str], rows: &'a [&'a [Option<String>]]) -> XlsxTable<'a> {
    XlsxTable {
        columns,
        rows,
        source_columns: None,
        sheet_name: "Export",
        null_as: "NULL",
        source: source(rows.len()),
    }
}
fn prepared(bytes: &[u8], cancel: &Cancellation) -> PreparedFile {
    prepare_bytes(bytes.to_vec(), Compression::None, source(1), cancel).unwrap()
}

#[test]
fn gzip_preserves_encoded_bytes_and_partial_disclosure() {
    let cancel = Cancellation::default();
    let partial = Source {
        completeness: Completeness::Partial,
        scope: Scope::CompleteResult,
        row_count: 7,
    };
    assert!(matches!(
        prepare_bytes(vec![], Compression::Gzip, partial, &cancel),
        Err(Error::PartialSource)
    ));
    let source = Source {
        scope: Scope::RetainedRows,
        ..partial
    };
    let bytes = vec![0xff, 0xfe, 0x65, 0x96, 0x3d, 0xd8, 0x00, 0xde, 0, 0];
    let compressed = prepare_bytes(bytes.clone(), Compression::Gzip, source, &cancel).unwrap();
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(compressed.bytes())
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, bytes);
    assert_eq!(compressed.source(), source);
    assert!(matches!(
        prepare_bytes(
            vec![0; MAX_FILE_BYTES + 1],
            Compression::None,
            source,
            &cancel
        ),
        Err(Error::InputTooLarge)
    ));
}

#[test]
fn xlsx_exact_strings_projection_null_tokens_and_no_formulas() {
    let data = vec![
        Some("hidden".into()),
        Some("9223372036854775807".into()),
        Some("0.123456789012345678901".into()),
        Some("000123".into()),
        Some("=SUM(A1:A2)".into()),
        Some("https://example.invalid".into()),
        Some("雪😀\n<&>\r\t'\"".into()),
        Some("_x0000_".into()),
        None,
        Some(String::new()),
        Some("NULL".into()),
    ];
    let headings = [
        "big", "decimal", "zero", "formula", "url", "unicode", "escape", "null", "empty", "literal",
    ];
    let rows = [data.as_slice()];
    let projection = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let mut input = table(&headings, &rows);
    input.source_columns = Some(&projection);
    let file = prepare_xlsx(&input, Compression::None, &Cancellation::default()).unwrap();
    let mut workbook: Xlsx<_> = calamine::open_workbook_from_rs(Cursor::new(file.bytes())).unwrap();
    let values = workbook.worksheet_range("Export").unwrap();
    for (column, source_column) in projection.iter().enumerate() {
        let expected = data[*source_column].as_deref().unwrap_or("NULL");
        if expected.is_empty() {
            assert_eq!(values.get_value((1, column as u32)), Some(&Data::Empty));
        } else {
            // Calamine 0.26 leaves SpreadsheetML control/literal escapes encoded.
            // Excel decodes these escapes; the library writer retains CR rather
            // than allowing XML newline normalization to replace it with LF.
            let encoded = expected
                .replace("_x0000_", "_x005F_x0000_")
                .replace('\r', "_x000D_");
            assert_eq!(
                values.get_value((1, column as u32)),
                Some(&Data::String(encoded))
            );
        }
    }
    assert!(workbook
        .worksheet_formula("Export")
        .unwrap()
        .used_cells()
        .next()
        .is_none());
}

#[test]
fn xlsx_refuses_projection_width_cell_and_library_unicode_hazard_before_workbook() {
    let cancel = Cancellation::default();
    let cells = vec![Some("a".into()), Some("_x雪雪_".into())];
    let rows = [cells.as_slice()];
    let columns = ["value"];
    let mut input = table(&columns, &rows);
    assert!(matches!(
        prepare_xlsx(&input, Compression::None, &cancel),
        Err(Error::InvalidRowWidth)
    ));
    input.source_columns = Some(&[2]);
    assert!(matches!(
        prepare_xlsx(&input, Compression::None, &cancel),
        Err(Error::InvalidProjection)
    ));
    input.source_columns = Some(&[1]);
    assert!(matches!(
        prepare_xlsx(&input, Compression::None, &cancel),
        Err(Error::UnsupportedCellText)
    ));
    input.source_columns = Some(&[0]);
    assert!(
        prepare_xlsx(&input, Compression::None, &cancel).is_ok(),
        "hidden hazardous text must not be scanned"
    );
    let long = vec![Some("😀".repeat(MAX_XLSX_CELL_UNITS / 2 + 1))];
    let rows = [long.as_slice()];
    assert!(matches!(
        prepare_xlsx(&table(&columns, &rows), Compression::None, &cancel),
        Err(Error::CellTooLong)
    ));
}

#[test]
fn xlsx_limits_include_headers_and_repeated_null_expansion() {
    let cancel = Cancellation::default();
    let row = vec![None];
    let rows = vec![row.as_slice(); MAX_XLSX_CELLS];
    assert!(matches!(
        validate_xlsx(&table(&["a"], &rows), &cancel),
        Err(Error::InputTooLarge)
    ));
    let rows = vec![row.as_slice(); 512];
    let token = "x".repeat(MAX_XLSX_CELL_UNITS);
    let mut input = table(&["a"], &rows);
    input.null_as = &token;
    assert!(matches!(
        validate_xlsx(&input, &cancel),
        Err(Error::InputTooLarge)
    ));
    input.source.row_count = 1;
    assert!(matches!(
        validate_xlsx(&input, &cancel),
        Err(Error::InvalidRowCount)
    ));
}

#[test]
fn output_limit_refuses_growth_and_out_of_bounds_seeks() {
    let cancel = Cancellation::default();
    let mut sink = BoundedBuffer::new(&cancel);
    sink.seek(SeekFrom::Start(MAX_FILE_BYTES as u64 - 1))
        .unwrap();
    sink.write_all(b"x").unwrap();
    assert_eq!(sink.bytes.get_ref().len(), MAX_FILE_BYTES);
    assert!(sink.write_all(b"y").is_err());
    assert_eq!(sink.bytes.get_ref().len(), MAX_FILE_BYTES);
    assert!(matches!(
        sink.check_result(Ok(())),
        Err(Error::OutputTooLarge)
    ));
    assert!(sink
        .seek(SeekFrom::Start(MAX_FILE_BYTES as u64 + 1))
        .is_err());
    let mut sink = BoundedBuffer::new(&cancel);
    assert!(sink.seek(SeekFrom::Current(-1)).is_err());
    assert!(sink.bytes.get_ref().is_empty());
}

#[test]
fn publication_is_private_no_clobber_and_cleans_temp_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("result.csv");
    let cancel = Cancellation::default();
    let result = publish_new(prepared(b"exact\0bytes", &cancel), &destination, &cancel).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"exact\0bytes");
    assert_eq!(result.path, destination);
    assert_eq!(result.bytes, 11);
    assert_eq!(result.source, source(1));
    assert!(!cancel.cancel(), "publication admission already won");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let retry = Cancellation::default();
    assert!(matches!(
        publish_new(prepared(b"overwrite", &retry), &destination, &retry),
        Err(Error::DestinationExists)
    ));
    assert_eq!(std::fs::read(&destination).unwrap(), b"exact\0bytes");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    let absent_parent = directory.path().join("absent").join("result");
    assert!(publish_new(prepared(b"x", &retry), &absent_parent, &retry).is_err());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn cancel_and_publication_admission_have_one_winner() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("cancelled");
    let cancel = Cancellation::default();
    let file = prepared(b"x", &cancel);
    assert!(cancel.clone().cancel());
    assert!(cancel.is_cancelled());
    assert!(matches!(
        publish_new(file, &destination, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    assert!(matches!(
        prepare_bytes(vec![], Compression::Gzip, source(0), &cancel),
        Err(Error::Cancelled)
    ));
    let token = Cancellation::default();
    token.admit_publication().unwrap();
    assert!(!token.cancel());
    assert!(!token.is_cancelled());
    assert!(matches!(token.admit_publication(), Err(Error::JobFinished)));
}

#[test]
fn late_collision_cancel_and_io_failure_remove_private_partial_without_touching_destination() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("destination");
    let partial = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    // Another publisher wins after our temp file has been prepared.
    std::fs::write(&destination, b"winner").unwrap();
    let token = Cancellation::default();
    assert!(matches!(
        publish_temporary(partial, &destination, &token),
        Err(Error::DestinationExists)
    ));
    assert_eq!(std::fs::read(&destination).unwrap(), b"winner");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    assert!(
        !token.cancel(),
        "failed filesystem commit still owns completed admission"
    );
    let partial = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    let token = Cancellation::default();
    token.cancel();
    assert!(matches!(
        publish_temporary(partial, &directory.path().join("cancelled"), &token),
        Err(Error::Cancelled)
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    let partial = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
    let token = Cancellation::default();
    assert!(matches!(
        publish_temporary(
            partial,
            &directory.path().join("missing").join("file"),
            &token
        ),
        Err(Error::Io(_))
    ));
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
