use super::*;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

fn zipped(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        zip.start_file(
            *name,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn replaced_part(bytes: Vec<u8>, part: &str, change: impl FnOnce(String) -> String) -> Vec<u8> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut parts = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let mut body = Vec::new();
        file.read_to_end(&mut body).unwrap();
        parts.push((file.name().to_owned(), body));
    }
    let (_, bytes) = parts.iter_mut().find(|(name, _)| name == part).unwrap();
    *bytes = change(String::from_utf8(std::mem::take(bytes)).unwrap()).into_bytes();
    let refs = parts
        .iter()
        .map(|(name, body)| (name.as_str(), body.as_slice()))
        .collect::<Vec<_>>();
    zipped(&refs)
}

fn package(sheet: &str, strings: Option<&str>) -> Vec<u8> {
    let root = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="office" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;
    let workbook = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="工作表" sheetId="1" r:id="sheet" state="hidden"/></sheets></workbook>"#;
    let rels = format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="sheet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>{}</Relationships>"#,
        if strings.is_some() {
            r#"<Relationship Id="sst" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>"#
        } else {
            ""
        }
    );
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet}</sheetData></worksheet>"#
    );
    let mut parts = vec![
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ];
    if let Some(strings) = strings {
        parts.push(("xl/sharedStrings.xml", strings.as_bytes()));
    }
    zipped(&parts)
}

fn convert(
    sheet: &str,
    strings: Option<&str>,
    null: &str,
) -> Result<(String, MaterializedSheet), XlsxError> {
    let cancel = AtomicBool::new(false);
    let mut book = Workbook::open(Cursor::new(package(sheet, strings)), &cancel)?;
    assert_eq!(book.sheets()[0].name, "工作表");
    assert_eq!(book.sheets()[0].visibility, SheetVisibility::Hidden);
    let mut output = Vec::new();
    let result = book.write_sheet(book.sheets()[0].id, null, &mut output, &cancel)?;
    Ok((String::from_utf8(output).unwrap(), result))
}

#[test]
fn exact_numeric_cache_boolean_date_and_unicode() {
    let sst = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><r><t> 你好</t></r><r><t>&amp;世界 </t></r><rPh sb="0" eb="2"><t>ignored phonetics</t></rPh></si></sst>"#;
    let (csv, report) = convert(r#"<row r="1"><c r="A1"><v>9223372036854775807</v></c><c r="B1"><v>0.123456789012345678901234567890</v></c><c r="C1" t="str"><f>ignored()</f><v>0000123</v></c><c r="D1" t="b"><v>1</v></c><c r="E1" s="4"><v>45200.5000</v></c><c r="F1" t="s"><v>0</v></c></row>"#, Some(sst), "\\N").unwrap();
    assert_eq!(csv, "\"column_1\",\"column_2\",\"column_3\",\"column_4\",\"column_5\",\"column_6\"\n\"9223372036854775807\",\"0.123456789012345678901234567890\",\"0000123\",\"true\",\"45200.5000\",\" 你好&世界 \"\n");
    assert_eq!(report.rows, 1);
    assert_eq!(report.cached_formula_cells, 1);
    assert!(!report.header_detected);
}

#[test]
fn baseline_headers_duplicate_names_and_null_are_indexed_exactly() {
    let (csv, report) = convert(r#"<row><c t="inlineStr"><is><t> id </t></is></c><c t="inlineStr"><is><t>id</t></is></c><c t="inlineStr"><is><t> </t></is></c></row><row><c><v>1</v></c><c t="inlineStr"><is><t>NULL</t></is></c><c t="inlineStr"><is><t></t></is></c></row><row><c t="inlineStr"><is><t>NULLx</t></is></c><c t="inlineStr"><is><t>a&quot;b,&#10;中</t></is></c></row>"#, None, "NULL").unwrap();
    assert_eq!(
        csv,
        "\"id\",\"id\",\"column_3\"\n\"1\",NULL,\"\"\n\"NULLx\",\"a\"\"b,\n中\",\"\"\n"
    );
    assert!(report.header_detected);
    assert_eq!(report.rows, 2);
}

#[test]
fn sparse_extent_matches_used_range_without_dense_allocation() {
    let (csv, report) = convert(r#"<row r="5"><c r="C5" t="inlineStr"><is><t>label</t></is></c></row><row r="7"><c r="D7"><v>2</v></c></row>"#, None, "\\N").unwrap();
    assert_eq!(csv, "\"label\",\"column_2\"\n\"\",\"\"\n\"\",\"2\"\n");
    assert_eq!(report.rows, 2);
    assert_eq!(
        convert(
            r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="E1048576"><v>2</v></c></row>"#,
            None,
            "\\N"
        ),
        Err(XlsxError::Limit(Limit::Sparse))
    );
}

#[test]
fn malformed_values_and_formula_caches_refuse() {
    for source in [
        r#"<row><c t="s"><v>999999</v></c></row>"#,
        r#"<row><c r="A1"><v>1</v></c><c r="A1"><v>2</v></c></row>"#,
        r#"<row><c t="b"><v>yes</v></c></row>"#,
        r#"<row><c><v>NaN</v></c></row>"#,
        r#"<row><c><v>1</v><v>2</v></c></row>"#,
    ] {
        assert!(convert(source, None, "\\N").is_err());
    }
    assert_eq!(
        convert(r#"<row><c><f>1+1</f></c></row>"#, None, "\\N"),
        Err(XlsxError::MissingFormulaCache)
    );
}

#[test]
fn empty_sheet_is_not_a_synthetic_data_row() {
    assert_eq!(
        convert("", None, "\\N").unwrap(),
        (
            String::new(),
            MaterializedSheet {
                rows: 0,
                columns: 0,
                header_detected: false,
                cached_formula_cells: 0
            }
        )
    );
}

#[test]
fn zip_admission_rejects_ambiguous_zip64_encryption_and_oversized_metadata() {
    let cancel = AtomicBool::new(false);
    let bytes = package("", None);
    let end = bytes.windows(4).position(|w| w == b"PK\x05\x06").unwrap();
    let cd = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    for (offset, value, expected) in [
        (end + 10, 65535_u16, XlsxError::UnsupportedZip),
        (cd + 8, 1, XlsxError::UnsupportedZip),
        (cd + 10, 12, XlsxError::UnsupportedZip),
        (cd + 28, 1025, XlsxError::Limit(Limit::Archive)),
    ] {
        let mut changed = bytes.clone();
        changed[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        if offset == end + 10 {
            changed[end + 8..end + 10].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(
            archive::preflight(&mut Cursor::new(changed), &cancel),
            Err(expected)
        );
    }
    let mut ambiguous = bytes;
    ambiguous.extend_from_slice(b"PK\x05\x06");
    assert_eq!(
        archive::preflight(&mut Cursor::new(ambiguous), &cancel),
        Err(XlsxError::AmbiguousZip)
    );
}

#[test]
fn xml_structure_entities_encoding_and_raw_field_limits() {
    let cancel = AtomicBool::new(false);
    for xml in [
        "<!DOCTYPE a [<!ENTITY x 'bad'>]><a>&x;</a>",
        "<a/><b/>",
        "<a>",
        "<a>&unknown;</a>",
        "<a>&#0;</a>",
        "<?xml version='1.0' encoding='UTF-16'?><a/>",
    ] {
        assert!(xml::validate(xml.as_bytes(), &cancel).is_err());
    }
    let deep = format!("{}{}", "<a>".repeat(65), "</a>".repeat(65));
    assert_eq!(
        xml::validate(deep.as_bytes(), &cancel),
        Err(XlsxError::Limit(Limit::Structure))
    );
    let wide = format!("<a>{}</a>", "x".repeat(MAX_FIELD_BYTES + 1));
    assert_eq!(
        xml::validate(wide.as_bytes(), &cancel),
        Err(XlsxError::Limit(Limit::Field))
    );
}

#[test]
fn existing_xlsx_writer_workbooks_keep_sheet_order_and_cells() {
    let mut written = rust_xlsxwriter::Workbook::new();
    written
        .add_worksheet()
        .set_name("Second alphabetically")
        .unwrap()
        .write_string(0, 0, "value")
        .unwrap()
        .write_string(1, 0, "9223372036854775807")
        .unwrap();
    written
        .add_worksheet()
        .set_name("A first alphabetically")
        .unwrap()
        .write_number(0, 0, 123.0)
        .unwrap();
    let cancel = AtomicBool::new(false);
    let mut read = Workbook::open(Cursor::new(written.save_to_buffer().unwrap()), &cancel).unwrap();
    assert_eq!(
        read.sheets()
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["Second alphabetically", "A first alphabetically"]
    );
    let mut csv = Vec::new();
    let summary = read
        .write_sheet(read.sheets()[0].id, "\\N", &mut csv, &cancel)
        .unwrap();
    assert_eq!(summary.rows, 1);
    assert_eq!(csv, b"\"value\"\n\"9223372036854775807\"\n");
    csv.clear();
    read.write_sheet(read.sheets()[1].id, "\\N", &mut csv, &cancel)
        .unwrap();
    assert_eq!(csv, b"\"column_1\"\n\"123\"\n");
}

#[test]
fn corrupt_zip_crc_and_declared_expansion_refuse() {
    let cancel = AtomicBool::new(false);
    let mut bytes = package("", None);
    let end = bytes.windows(4).position(|w| w == b"PK\x05\x06").unwrap();
    let cd = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    bytes[cd + 16] ^= 1; // CRC for the root relationship XML.
    assert!(Workbook::open(Cursor::new(bytes), &cancel).is_err());
    let mut bytes = package("", None);
    bytes[cd + 24..cd + 28].copy_from_slice(&((MAX_METADATA_BYTES + 1) as u32).to_le_bytes());
    assert!(matches!(
        Workbook::open(Cursor::new(bytes), &cancel),
        Err(XlsxError::Limit(Limit::Xml))
    ));
}

#[test]
fn record_escape_expansion_is_checked_before_writing_that_record() {
    let value = "\"".repeat(MAX_FIELD_BYTES);
    let sst = format!(
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t><![CDATA[{value}]]></t></si></sst>"#
    );
    let cells = r#"<c t="s"><v>0</v></c>"#.repeat(5);
    assert_eq!(
        convert(&format!("<row>{cells}</row>"), Some(&sst), "\\N"),
        Err(XlsxError::Limit(Limit::Record))
    );
}

#[test]
fn cancellation_and_output_failure_never_return_a_summary() {
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        Workbook::open(Cursor::new(package("", None)), &cancel),
        Err(XlsxError::Cancelled)
    ));
    cancel.store(false, Ordering::Relaxed);
    let mut workbook = Workbook::open(
        Cursor::new(package("<row><c><v>1</v></c></row>", None)),
        &cancel,
    )
    .unwrap();
    struct Fails;
    impl Write for Fails {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("secret path must not appear"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let error = workbook
        .write_sheet(workbook.sheets()[0].id, "\\N", &mut Fails, &cancel)
        .unwrap_err();
    assert_eq!(error, XlsxError::OutputIo);
    assert!(!error.to_string().contains("secret"));
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(
        workbook.write_sheet(workbook.sheets()[0].id, "\\N", &mut Vec::new(), &cancel),
        Err(XlsxError::Cancelled)
    );
}

#[test]
fn null_token_and_sheet_identity_are_validated_before_output() {
    let cancel = AtomicBool::new(false);
    let mut workbook = Workbook::open(Cursor::new(package("", None)), &cancel).unwrap();
    let mut out = Vec::new();
    assert_eq!(
        workbook.write_sheet(SheetId(255), "\\N", &mut out, &cancel),
        Err(XlsxError::InvalidSheet)
    );
    assert_eq!(
        workbook.write_sheet(SheetId(0), "bad,token", &mut out, &cancel),
        Err(XlsxError::InvalidNullToken)
    );
    assert!(out.is_empty());
}

#[test]
fn spreadsheet_string_escapes_and_surrogate_pairs_are_decoded_once() {
    let (csv, _) = convert(r#"<row><c t="inlineStr"><is><t>literal_x005F_x0041_ _xD83D__xDE00_ _x000D_</t></is></c></row>"#, None, "\\N").unwrap();
    assert_eq!(csv, "\"column_1\"\n\"literal_x0041_ 😀 \r\"\n");
    assert_eq!(
        convert(
            r#"<row><c t="inlineStr"><is><t>_x0000_</t></is></c></row>"#,
            None,
            "\\N"
        ),
        Err(XlsxError::UnsupportedCell)
    );
    assert_eq!(
        convert(
            r#"<row><c t="inlineStr"><is><t>_xD800_</t></is></c></row>"#,
            None,
            "\\N"
        ),
        Err(XlsxError::UnsupportedCell)
    );
}

#[test]
fn metadata_relationships_and_sheets_require_exact_structural_paths() {
    let cancel = AtomicBool::new(false);
    let misplaced = replaced_part(package("", None), "xl/workbook.xml", |s| {
        s.replace("<sheets>", "<definedNames>")
            .replace("</sheets>", "</definedNames>")
    });
    assert!(matches!(
        Workbook::open(Cursor::new(misplaced), &cancel),
        Err(XlsxError::InvalidWorkbook)
    ));
    let nested_root = replaced_part(package("", None), "xl/workbook.xml", |s| {
        format!(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">{s}</worksheet>"#
        )
    });
    assert!(matches!(
        Workbook::open(Cursor::new(nested_root), &cancel),
        Err(XlsxError::InvalidWorkbook)
    ));
    let nested_relation = replaced_part(package("", None), "_rels/.rels", |s| {
        s.replace("<Relationship Id=", "<Relationships><Relationship Id=")
            .replace("/></Relationships>", "/></Relationships></Relationships>")
    });
    assert!(matches!(
        Workbook::open(Cursor::new(nested_relation), &cancel),
        Err(XlsxError::InvalidWorkbook)
    ));
    let external = replaced_part(package("", None), "_rels/.rels", |s| {
        s.replace("Target=", "TargetMode=\"External\" Target=")
    });
    assert!(matches!(
        Workbook::open(Cursor::new(external), &cancel),
        Err(XlsxError::InvalidWorkbook)
    ));
}

#[test]
fn unsupported_nested_cell_values_cannot_silently_become_empty() {
    assert_eq!(
        convert(
            r#"<row><c t="str"><v><unknown>lost text</unknown></v></c></row>"#,
            None,
            "\\N"
        ),
        Err(XlsxError::UnsupportedCell)
    );
    assert_eq!(
        convert(
            r#"<row><c t="inlineStr"><is><unknown>lost text</unknown></is></c></row>"#,
            None,
            "\\N"
        ),
        Err(XlsxError::UnsupportedCell)
    );
}

#[test]
fn alternate_zip_filename_cannot_bypass_raw_name_admission() {
    let mut options =
        zip::write::FullFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    options
        .add_extra_data(0x7075, vec![1, 0, 0, 0, 0, b'a'].into_boxed_slice(), true)
        .unwrap();
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive.start_file("original", options).unwrap();
    archive.write_all(b"plain").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    assert_eq!(
        archive::preflight(&mut Cursor::new(bytes), &AtomicBool::new(false)),
        Err(XlsxError::UnsupportedZip)
    );
}
