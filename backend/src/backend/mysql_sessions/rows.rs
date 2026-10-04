//! Bounded text rendering of MySQL result rows. User statements run over the
//! text protocol, so every value arrives as bytes; nothing here allocates
//! more than the per-cell and per-result budgets.
use sqlx::mysql::{MySql, MySqlRow};
use sqlx::{Column, Decode, Row, TypeInfo, ValueRef};

use super::MySqlResult;

/// Rows kept for display. Later rows are still read (and counted) so the
/// session stays in sync, but they are dropped.
pub const MYSQL_MAX_ROWS: usize = 1000;
/// Text kept across all retained cells of one result.
pub const MYSQL_MAX_RESULT_BYTES: usize = 16 * 1024 * 1024;
/// Text kept per cell; longer values end in `…` and count as truncated.
pub const MYSQL_MAX_CELL_BYTES: usize = 64 * 1024;

/// How a column's bytes are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CellKind {
    Text,
    /// BINARY/VARBINARY/BLOB: printable UTF-8 as text, otherwise hex.
    Bytes,
    /// BIT(n): big-endian bytes as an unsigned integer.
    Bit,
    /// Always hex (GEOMETRY).
    Hex,
}

pub(super) fn cell_kind(type_name: &str) -> CellKind {
    match type_name {
        "BIT" => CellKind::Bit,
        "GEOMETRY" => CellKind::Hex,
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" => CellKind::Bytes,
        _ => CellKind::Text,
    }
}

/// Renders one value within `limit` bytes. Returns the text and whether it
/// was shortened.
pub(super) fn render(bytes: &[u8], kind: CellKind, limit: usize) -> (String, bool) {
    match kind {
        CellKind::Bit if bytes.len() <= 8 => (
            bytes
                .iter()
                .fold(0u64, |value, byte| (value << 8) | u64::from(*byte))
                .to_string(),
            false,
        ),
        CellKind::Text => match std::str::from_utf8(bytes) {
            Ok(text) => clip(text, limit),
            Err(_) => clip(&String::from_utf8_lossy(bytes), limit),
        },
        CellKind::Bytes => match std::str::from_utf8(bytes) {
            Ok(text) if printable(text) => clip(text, limit),
            _ => hex(bytes, limit),
        },
        CellKind::Bit | CellKind::Hex => hex(bytes, limit),
    }
}

fn printable(text: &str) -> bool {
    text.chars()
        .all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
}

fn clip(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let mut end = limit.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}…", &text[..end]), true)
}

fn hex(bytes: &[u8], limit: usize) -> (String, bool) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    // "0x" plus two digits per byte, leaving room for the ellipsis.
    let room = limit.saturating_sub(2 + '…'.len_utf8()) / 2;
    let shown = bytes.len().min(room);
    let mut text = String::with_capacity(2 + shown * 2 + 3);
    text.push_str("0x");
    for byte in &bytes[..shown] {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    let truncated = shown < bytes.len();
    if truncated {
        text.push('…');
    }
    (text, truncated)
}

/// Collects the last row-returning result set of a script within the row,
/// cell and total byte budgets.
#[derive(Default)]
pub(super) struct Collector {
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
    kinds: Vec<CellKind>,
    /// Rows read in the current set, kept or not.
    seen: u64,
    bytes: usize,
    truncated_cells: u64,
    over_budget: bool,
    /// The current set has delivered at least one row.
    open: bool,
    result_sets: u32,
    rows_affected: u64,
}

impl Collector {
    pub(super) fn row(&mut self, row: &MySqlRow) {
        if !self.open {
            self.start(
                row.columns()
                    .iter()
                    .map(|column| column.name().to_owned())
                    .collect(),
                row.columns()
                    .iter()
                    .map(|column| cell_kind(column.type_info().name()))
                    .collect(),
            );
        }
        if !self.admit() {
            return;
        }
        let mut values = Vec::with_capacity(self.kinds.len());
        let mut bytes = 0usize;
        for (index, kind) in self.kinds.iter().enumerate() {
            let value = row.try_get_raw(index).ok().and_then(|raw| {
                if raw.is_null() {
                    return None;
                }
                let data = <&[u8] as Decode<MySql>>::decode(raw).unwrap_or_default();
                let (text, truncated) = render(data, *kind, MYSQL_MAX_CELL_BYTES);
                self.truncated_cells += u64::from(truncated);
                Some(text)
            });
            bytes += value.as_ref().map_or(0, String::len);
            values.push(value);
        }
        self.push(values, bytes);
    }

    /// A new result set replaces the previous one.
    fn start(&mut self, columns: Vec<String>, kinds: Vec<CellKind>) {
        self.open = true;
        self.result_sets += 1;
        self.columns = columns;
        self.kinds = kinds;
        self.rows.clear();
        self.seen = 0;
        self.bytes = 0;
        self.truncated_cells = 0;
        self.over_budget = false;
    }

    /// Counts a row; false when it must be dropped.
    fn admit(&mut self) -> bool {
        self.seen += 1;
        !self.over_budget && self.rows.len() < MYSQL_MAX_ROWS
    }

    fn push(&mut self, values: Vec<Option<String>>, bytes: usize) {
        if self.bytes.saturating_add(bytes) > MYSQL_MAX_RESULT_BYTES {
            self.over_budget = true;
            return;
        }
        self.bytes += bytes;
        self.rows.push(values);
    }

    /// End of one statement's result (sqlx reports it after its rows).
    pub(super) fn done(&mut self, rows_affected: u64) {
        if !self.open {
            self.result_sets += 1;
        }
        self.open = false;
        self.rows_affected = self.rows_affected.saturating_add(rows_affected);
    }

    pub(super) fn has_columns(&self) -> bool {
        !self.columns.is_empty()
    }

    pub(super) fn set_columns(&mut self, columns: Vec<String>) {
        self.columns = columns;
    }

    pub(super) fn finish(self, runtime_ms: u64) -> MySqlResult {
        let kept = self.rows.len() as u64;
        MySqlResult {
            columns: self.columns,
            rows: self.rows,
            total_rows: self.seen,
            truncated: kept < self.seen,
            truncated_cells: self.truncated_cells,
            rows_affected: self.rows_affected,
            result_sets: self.result_sets,
            has_more: false,
            runtime_ms,
        }
    }
}

#[cfg(test)]
impl Collector {
    /// Feeds a synthetic row of already-rendered values.
    pub(super) fn push_for_test(&mut self, columns: &[&str], values: Vec<Option<String>>) {
        if !self.open {
            let names = columns.iter().map(|c| (*c).to_owned()).collect();
            self.start(names, vec![CellKind::Text; columns.len()]);
        }
        if self.admit() {
            let bytes = values.iter().flatten().map(String::len).sum();
            self.push(values, bytes);
        }
    }
}
