//! A bounded result model fed in batches, with the retention limits of
//! `src-tauri/src/query_session/postgres.rs`.

use std::rc::Rc;

use gpui::SharedString;

pub const MAX_ROWS_PER_RESULT_SET: usize = 10_000;
pub const MAX_BYTES_PER_EXECUTION: usize = 32 * 1024 * 1024;
pub const BATCH_ROWS: usize = 200;
pub const BATCH_BYTES: usize = 256 * 1024;
/// A cell is drawn from at most this many characters. The full value stays
/// in the model for copy and for a value inspector.
pub const DISPLAY_CHARS: usize = 256;

pub struct Cell {
    pub value: SharedString,
    /// What the grid draws: the value, or its bounded prefix.
    pub display: SharedString,
}

pub type Row = Rc<[Cell]>;

#[derive(Default)]
pub struct ResultModel {
    pub columns: Vec<SharedString>,
    pub rows: Vec<Row>,
    pub retained_bytes: usize,
    /// Rows the stream produced after a retention limit was reached.
    pub omitted_rows: usize,
    pub complete: bool,
}

impl ResultModel {
    pub fn start(columns: Vec<SharedString>) -> Self {
        Self {
            columns,
            ..Default::default()
        }
    }

    /// Appends one batch. Rows past a limit are counted, not kept.
    pub fn push_batch(&mut self, batch: Vec<Vec<String>>) {
        for row in batch {
            let bytes: usize = row.iter().map(String::len).sum();
            if self.rows.len() >= MAX_ROWS_PER_RESULT_SET
                || self.retained_bytes + bytes > MAX_BYTES_PER_EXECUTION
            {
                self.omitted_rows += 1;
                continue;
            }
            self.retained_bytes += bytes;
            self.rows.push(row.into_iter().map(cell).collect());
        }
    }
}

fn cell(value: String) -> Cell {
    let value = SharedString::from(value);
    let display = match value.char_indices().nth(DISPLAY_CHARS) {
        Some((end, _)) => SharedString::from(value[..end].to_string()),
        None => value.clone(),
    };
    Cell { value, display }
}

/// Splits a fixture into the batches a Query Session would deliver: at most
/// 200 rows or 256 KiB, whichever comes first.
pub fn next_batch(
    fixture: crate::fixture::Fixture,
    next_row: &mut usize,
) -> Option<Vec<Vec<String>>> {
    if *next_row > fixture.row_count() {
        return None;
    }
    let mut batch = Vec::new();
    let mut bytes = 0;
    while *next_row <= fixture.row_count() && batch.len() < BATCH_ROWS && bytes < BATCH_BYTES {
        let row = fixture.row(*next_row);
        bytes += row.iter().map(String::len).sum::<usize>();
        batch.push(row);
        *next_row += 1;
    }
    Some(batch)
}
