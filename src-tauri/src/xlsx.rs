//! XLSX import (read) and export (write) via native Rust libraries.
//!
//! - **Import**: `calamine` reads `.xlsx` / `.xls` / `.ods` files and
//!   returns sheets as `Vec<Vec<String>>`. The frontend sends the file
//!   as a base64 blob; we decode, parse, and return structured sheets.
//! - **Export**: `rust_xlsxwriter` builds a proper `.xlsx` file from
//!   columns + rows and returns it as base64 for the frontend to
//!   trigger a download.

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use calamine::{open_workbook_from_rs, Reader, Xlsx};
use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

// ---------------------------------------------------------------------------
// Import (read)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedSheet {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseXlsxPayload {
    /// Base64-encoded file content.
    pub data_base64: String,
}

/// Parse an XLSX file (sent as base64) into sheets with auto-detected
/// headers. Returns one `ParsedSheet` per worksheet.
#[tauri::command]
pub async fn parse_xlsx(payload: ParseXlsxPayload) -> Result<Vec<ParsedSheet>, String> {
    let bytes = B64
        .decode(&payload.data_base64)
        .map_err(|e| format!("Invalid base64: {e}"))?;

    let cursor = Cursor::new(bytes);
    let mut workbook: Xlsx<_> =
        open_workbook_from_rs(cursor).map_err(|e| format!("Failed to open XLSX: {e}"))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();
    let mut sheets = Vec::with_capacity(sheet_names.len());

    for name in &sheet_names {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| format!("Failed to read sheet '{name}': {e}"))?;

        let mut raw_rows: Vec<Vec<String>> = Vec::new();
        for row in range.rows() {
            let string_row: Vec<String> = row
                .iter()
                .map(|cell| match cell {
                    calamine::Data::Empty => String::new(),
                    calamine::Data::String(s) => s.clone(),
                    calamine::Data::Int(n) => n.to_string(),
                    calamine::Data::Float(f) => {
                        // Avoid trailing ".0" for whole numbers
                        if f.fract() == 0.0 && f.abs() < i64::MAX as f64 {
                            format!("{}", *f as i64)
                        } else {
                            f.to_string()
                        }
                    }
                    calamine::Data::Bool(b) => b.to_string(),
                    calamine::Data::DateTime(dt) => dt.to_string(),
                    calamine::Data::DateTimeIso(s) => s.clone(),
                    calamine::Data::DurationIso(s) => s.clone(),
                    calamine::Data::Error(e) => format!("#ERR:{e:?}"),
                })
                .collect();
            raw_rows.push(string_row);
        }

        let (columns, data_rows) = normalize_sheet(&raw_rows);
        sheets.push(ParsedSheet {
            name: name.clone(),
            columns,
            rows: data_rows,
        });
    }

    Ok(sheets)
}

/// Auto-detect header and normalize rows to equal width.
fn normalize_sheet(rows: &[Vec<String>]) -> (Vec<String>, Vec<Vec<String>>) {
    if rows.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let has_header = detect_header(rows);

    let columns: Vec<String> = if has_header {
        (0..width)
            .map(|i| {
                let header = rows[0].get(i).map(|s| s.trim()).unwrap_or("");
                if header.is_empty() {
                    format!("column_{}", i + 1)
                } else {
                    header.to_string()
                }
            })
            .collect()
    } else {
        (0..width).map(|i| format!("column_{}", i + 1)).collect()
    };

    let start = if has_header { 1 } else { 0 };
    let data_rows: Vec<Vec<String>> = rows[start..]
        .iter()
        .map(|row| {
            (0..width)
                .map(|i| row.get(i).cloned().unwrap_or_default())
                .collect()
        })
        .collect();

    (columns, data_rows)
}

/// Heuristic: first row is a header if it contains alphabetic text
/// and the second row has numeric or empty cells.
fn detect_header(rows: &[Vec<String>]) -> bool {
    if rows.len() < 2 {
        return false;
    }
    let first_has_text = rows[0]
        .iter()
        .any(|cell| cell.chars().any(|c| c.is_alphabetic() || c == '_'));
    let second_has_data = rows[1].iter().any(|cell| {
        let trimmed = cell.trim();
        trimmed.is_empty() || trimmed.parse::<f64>().is_ok()
    });
    first_has_text && second_has_data
}

// ---------------------------------------------------------------------------
// Export (write)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportXlsxPayload {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Optional sheet name (defaults to "Export").
    #[serde(default = "default_sheet_name")]
    pub sheet_name: String,
}

fn default_sheet_name() -> String {
    "Export".to_string()
}

/// Build an XLSX file from columns + rows and return it as base64.
#[tauri::command]
pub async fn export_xlsx(payload: ExportXlsxPayload) -> Result<String, String> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(&payload.sheet_name)
        .map_err(|e| format!("Failed to set sheet name: {e}"))?;

    // Write header row
    for (col, header) in payload.columns.iter().enumerate() {
        worksheet
            .write_string(0, col as u16, header)
            .map_err(|e| format!("Failed to write header: {e}"))?;
    }

    // Write data rows
    for (row_idx, row) in payload.rows.iter().enumerate() {
        let xlsx_row = (row_idx + 1) as u32; // offset by 1 for header
        for (col_idx, value) in row.iter().enumerate() {
            // Try to write as number if parseable, otherwise as string
            if let Ok(num) = value.parse::<f64>() {
                worksheet
                    .write_number(xlsx_row, col_idx as u16, num)
                    .map_err(|e| format!("Failed to write cell: {e}"))?;
            } else {
                worksheet
                    .write_string(xlsx_row, col_idx as u16, value)
                    .map_err(|e| format!("Failed to write cell: {e}"))?;
            }
        }
    }

    let buffer = workbook
        .save_to_buffer()
        .map_err(|e| format!("Failed to save XLSX: {e}"))?;

    Ok(B64.encode(&buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_header_works() {
        let rows = vec![
            vec!["Name".into(), "Age".into(), "City".into()],
            vec!["Alice".into(), "30".into(), "NYC".into()],
        ];
        assert!(detect_header(&rows));
    }

    #[test]
    fn detect_header_numeric_first_row() {
        let rows = vec![
            vec!["1".into(), "2".into(), "3".into()],
            vec!["4".into(), "5".into(), "6".into()],
        ];
        assert!(!detect_header(&rows));
    }

    #[test]
    fn normalize_sheet_with_header() {
        let rows = vec![
            vec!["id".into(), "name".into()],
            vec!["1".into(), "Alice".into()],
            vec!["2".into(), "Bob".into()],
        ];
        let (cols, data) = normalize_sheet(&rows);
        assert_eq!(cols, vec!["id", "name"]);
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], vec!["1", "Alice"]);
    }

    #[test]
    fn normalize_sheet_pads_short_rows() {
        let rows = vec![vec!["a".into(), "b".into(), "c".into()], vec!["1".into()]];
        let (cols, data) = normalize_sheet(&rows);
        assert_eq!(cols, vec!["a", "b", "c"]);
        assert_eq!(data[0], vec!["1", "", ""]);
    }

    #[tokio::test]
    async fn export_xlsx_roundtrip() {
        let payload = ExportXlsxPayload {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec!["1".into(), "Alice".into()],
                vec!["2".into(), "Bob".into()],
            ],
            sheet_name: "Test".into(),
        };
        let base64 = export_xlsx(payload).await.unwrap();
        // Should produce valid base64 that decodes to a ZIP (XLSX is a ZIP)
        let bytes = B64.decode(&base64).unwrap();
        assert_eq!(&bytes[0..4], b"PK\x03\x04"); // ZIP magic
    }

    #[tokio::test]
    async fn parse_xlsx_from_exported() {
        // Export then re-import to test the roundtrip
        let export_payload = ExportXlsxPayload {
            columns: vec!["id".into(), "name".into()],
            rows: vec![
                vec!["1".into(), "Alice".into()],
                vec!["2".into(), "Bob".into()],
            ],
            sheet_name: "Sheet1".into(),
        };
        let base64 = export_xlsx(export_payload).await.unwrap();

        let parse_payload = ParseXlsxPayload {
            data_base64: base64,
        };
        let sheets = parse_xlsx(parse_payload).await.unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].name, "Sheet1");
        assert_eq!(sheets[0].columns, vec!["id", "name"]);
        assert_eq!(sheets[0].rows.len(), 2);
        assert_eq!(sheets[0].rows[0], vec!["1", "Alice"]);
        assert_eq!(sheets[0].rows[1], vec!["2", "Bob"]);
    }
}
