use super::{
    cells, check_cancel, Limit, MaterializedSheet, XlsxError, MAX_COLUMNS, MAX_LOGICAL_CELLS,
    MAX_RECORD_BYTES,
};
use std::{io::Write, mem::size_of, sync::atomic::AtomicBool};

#[derive(Default)]
struct Extent {
    first: u32,
    last: u32,
    left: u32,
    right: u32,
    first_values: Vec<(u32, String)>,
    first_bytes: usize,
    first_text: bool,
    second_numeric: bool,
    second_cells: usize,
    formulas: u64,
}

pub(super) fn write(
    bytes: &[u8],
    strings: &[String],
    null: &str,
    output: &mut impl Write,
    cancel: &AtomicBool,
) -> Result<MaterializedSheet, XlsxError> {
    let mut extent = Extent::default();
    cells::scan(bytes, strings, cancel, |cell| {
        if extent.first == 0 {
            extent.first = cell.row;
            extent.left = cell.col;
        }
        extent.last = cell.row;
        extent.left = extent.left.min(cell.col);
        extent.right = extent.right.max(cell.col);
        let width = (extent.right - extent.left + 1) as usize;
        if width > MAX_COLUMNS {
            return Err(XlsxError::Limit(Limit::Columns));
        }
        let logical = u64::from(extent.last - extent.first + 1) * width as u64;
        if logical > MAX_LOGICAL_CELLS {
            return Err(XlsxError::Limit(Limit::Sparse));
        }
        if cell.row == extent.first {
            extent.first_text |= cell.value.chars().any(|c| c.is_alphabetic() || c == '_');
            let size = extent
                .first_bytes
                .checked_add(cell.value.len())
                .ok_or(XlsxError::Limit(Limit::Record))?;
            if size + MAX_COLUMNS * size_of::<(u32, String)>() > MAX_RECORD_BYTES {
                return Err(XlsxError::Limit(Limit::Record));
            }
            if extent.first_values.is_empty() {
                extent.first_values.reserve_exact(MAX_COLUMNS);
            }
            let value = cell.value.into_owned();
            extent.first_bytes += value.capacity();
            if extent.first_bytes + extent.first_values.capacity() * size_of::<(u32, String)>()
                > MAX_RECORD_BYTES
            {
                return Err(XlsxError::Limit(Limit::Record));
            }
            extent.first_values.push((cell.col, value));
        } else if cell.row == extent.first + 1 {
            extent.second_cells += 1;
            let value = cell.value.trim();
            // Classification only; no parsed float is retained or used as a value.
            extent.second_numeric |= value.is_empty() || value.parse::<f64>().is_ok();
        }
        extent.formulas += u64::from(cell.formula);
        Ok(())
    })?;
    if extent.first == 0 {
        return Ok(MaterializedSheet {
            rows: 0,
            columns: 0,
            header_detected: false,
            cached_formula_cells: 0,
        });
    }
    let width = (extent.right - extent.left + 1) as usize;
    let header = extent.last > extent.first
        && extent.first_text
        && (extent.second_numeric || extent.second_cells < width);
    let mut columns = Vec::with_capacity(width);
    // Synthetic names are small. Existing headings borrow the first row until the
    // header is written, so normalization cannot clone an 8 MiB row.
    for i in 0..width {
        columns.push(format!("column_{}", i + 1));
    }
    let mut labels = Vec::with_capacity(width);
    for (i, generated) in columns.iter().enumerate() {
        let col = extent.left + i as u32;
        let label = if header {
            extent
                .first_values
                .iter()
                .find(|(c, _)| *c == col)
                .map(|(_, s)| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(generated)
        } else {
            generated
        };
        labels.push(label);
    }
    write_record(&labels, None, output, cancel)?;
    drop(labels);
    drop(columns);
    extent.first_values.clear();
    let first_data = extent.first + u32::from(header);
    let mut row_number = first_data;
    let mut row: Vec<String> = (0..width).map(|_| String::new()).collect();
    let mut row_bytes = row.capacity() * size_of::<String>();
    cells::scan(bytes, strings, cancel, |cell| {
        if cell.row < first_data {
            return Ok(());
        }
        while row_number < cell.row {
            write_owned_row(&row, null, output, cancel)?;
            for value in &mut row {
                *value = String::new();
            }
            row_bytes = row.capacity() * size_of::<String>();
            row_number += 1;
        }
        let index = (cell.col - extent.left) as usize;
        if row_bytes
            .checked_add(cell.value.len())
            .is_none_or(|n| n > MAX_RECORD_BYTES)
        {
            return Err(XlsxError::Limit(Limit::Record));
        }
        let value = cell.value.into_owned();
        row_bytes += value.capacity();
        if row_bytes > MAX_RECORD_BYTES {
            return Err(XlsxError::Limit(Limit::Record));
        }
        row[index] = value;
        Ok(())
    })?;
    if row_number <= extent.last {
        write_owned_row(&row, null, output, cancel)?;
    }
    Ok(MaterializedSheet {
        rows: u64::from(extent.last - first_data + 1),
        columns: width as u16,
        header_detected: header,
        cached_formula_cells: extent.formulas,
    })
}

fn write_owned_row(
    row: &[String],
    null: &str,
    output: &mut impl Write,
    cancel: &AtomicBool,
) -> Result<(), XlsxError> {
    let borrowed: Vec<&str> = row.iter().map(String::as_str).collect();
    write_record(&borrowed, Some(null), output, cancel)
}

fn write_record(
    values: &[&str],
    null: Option<&str>,
    output: &mut impl Write,
    cancel: &AtomicBool,
) -> Result<(), XlsxError> {
    check_cancel(cancel)?;
    let mut encoded = values.len(); // commas plus newline
    for value in values {
        let length = if null == Some(*value) {
            value.len()
        } else {
            value
                .len()
                .checked_add(2 + value.bytes().filter(|b| *b == b'"').count())
                .ok_or(XlsxError::Limit(Limit::Record))?
        };
        encoded = encoded
            .checked_add(length)
            .ok_or(XlsxError::Limit(Limit::Record))?;
        if encoded > MAX_RECORD_BYTES {
            return Err(XlsxError::Limit(Limit::Record));
        }
    }
    for (i, value) in values.iter().enumerate() {
        check_cancel(cancel)?;
        if i != 0 {
            put(output, b",")?;
        }
        if null == Some(*value) {
            put(output, value.as_bytes())?;
        } else {
            put(output, b"\"")?;
            let mut start = 0;
            for (at, byte) in value.bytes().enumerate() {
                if byte == b'"' {
                    put(output, &value.as_bytes()[start..at])?;
                    put(output, b"\"\"")?;
                    start = at + 1;
                }
            }
            put(output, &value.as_bytes()[start..])?;
            put(output, b"\"")?;
        }
    }
    put(output, b"\n")
}

fn put(output: &mut impl Write, bytes: &[u8]) -> Result<(), XlsxError> {
    output.write_all(bytes).map_err(|_| XlsxError::OutputIo)
}
