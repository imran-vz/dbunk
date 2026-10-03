//! One execution's exact, bounded results. Identity and sequence admission belong
//! to the controller; this reducer decides retention before its caller ACKs.

use std::{io, rc::Rc};

use dbunk_lib::backend::{QueryDatabaseError, QueryEvent, RowLimitOutcome};
use serde::Serialize;

pub const MAX_ENCODED_BYTES: usize = 48 * 1024 * 1024;
const MAX_ROW_BYTES: usize = 32 * 1024 * 1024;
const MAX_ROWS_PER_SET: usize = 10_000;
const MAX_ROWS: usize = 50_000;
const MAX_RESULT_SETS: usize = 64;
pub const DISPLAY_CHARS: usize = 256;

pub type Row = Rc<[Option<String>]>;

#[derive(Default)]
pub struct ResultSet {
    pub index: u32,
    pub columns: Vec<Option<String>>,
    pub widths: crate::column_widths::ColumnWidths,
    pub rows: Vec<Row>,
    pub row_count: Option<u64>,
    pub partial: bool,
    pub limit: Option<RowLimitOutcome>,
    pub omitted_rows: u64,
}

pub struct Notice {
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone)]
pub struct Completion {
    pub status: TerminalStatus,
    pub omitted_rows: u64,
    pub omitted_result_sets: u32,
    pub omitted_notices: u32,
    pub omitted_metadata_bytes: u64,
    pub truncation_reasons: Vec<String>,
    pub error: Option<QueryDatabaseError>,
    pub refusal: Option<String>,
}

pub struct ResultModel {
    pub sets: Vec<ResultSet>,
    pub active: usize,
    pub notices: Vec<Notice>,
    pub completion: Option<Completion>,
    pub retained_bytes: usize,
    pub native_omitted_rows: u64,
    pub native_omitted_metadata: u64,
    pub native_omitted_result_sets: u64,
    pub retention_limited: bool,
    byte_limit: usize,
    row_bytes: usize,
    row_count: usize,
}

impl Default for ResultModel {
    fn default() -> Self {
        Self::with_byte_limit(MAX_ENCODED_BYTES)
    }
}

impl ResultModel {
    pub fn with_byte_limit(byte_limit: usize) -> Self {
        Self {
            sets: Vec::new(),
            active: 0,
            notices: Vec::new(),
            completion: None,
            retained_bytes: 0,
            native_omitted_rows: 0,
            native_omitted_metadata: 0,
            native_omitted_result_sets: 0,
            retention_limited: false,
            byte_limit: byte_limit.min(MAX_ENCODED_BYTES),
            row_bytes: 0,
            row_count: 0,
        }
    }

    /// Clamp subsequent retention to the remaining workspace allowance. This
    /// never drops rows already retained by this or any other document.
    pub fn set_byte_limit(&mut self, limit: usize) {
        self.byte_limit = limit.min(MAX_ENCODED_BYTES);
    }

    pub fn active_set(&self) -> Option<&ResultSet> {
        self.sets.get(self.active)
    }

    /// `false` is sticky for the current execution. Metadata and terminal events
    /// are still consumed after row retention stops, so credit can always drain.
    pub fn consume(&mut self, event: QueryEvent) -> bool {
        if self.completion.is_some() {
            return !self.retention_limited;
        }
        // Count serialized bytes without materializing a second payload. Row
        // batches are counted individually, permitting partial batch retention.
        let metadata_bytes = if matches!(event, QueryEvent::RowBatch { .. }) {
            0
        } else {
            encoded_size(&event)
        };
        match event {
            QueryEvent::ResultSetStarted {
                result_set_index,
                columns,
            } => {
                if self.sets.iter().any(|set| set.index == result_set_index) {
                    return !self.retention_limited;
                }
                let admitted_bytes = metadata_bytes.saturating_add(
                    crate::column_widths::ColumnWidths::storage_bytes(columns.len()),
                );
                if self.sets.len() >= MAX_RESULT_SETS || !self.retain(admitted_bytes) {
                    self.native_omitted_result_sets += 1;
                    self.native_omitted_metadata = self
                        .native_omitted_metadata
                        .saturating_add(metadata_bytes as u64);
                    self.retention_limited = true;
                } else {
                    self.sets.push(ResultSet {
                        index: result_set_index,
                        widths: crate::column_widths::ColumnWidths::new(&columns),
                        columns,
                        ..Default::default()
                    });
                }
            }
            QueryEvent::RowBatch {
                result_set_index,
                rows,
            } => {
                let set_index = self
                    .sets
                    .iter()
                    .position(|set| set.index == result_set_index);
                if self.retention_limited {
                    self.native_omitted_rows =
                        self.native_omitted_rows.saturating_add(rows.len() as u64);
                    if let Some(index) = set_index {
                        self.sets[index].omitted_rows = self.sets[index]
                            .omitted_rows
                            .saturating_add(rows.len() as u64);
                    }
                    return false;
                }
                let mut geometry_changed = false;
                for row in rows {
                    let bytes = encoded_size(&row);
                    let fits_set = set_index.is_some_and(|index| {
                        self.sets[index].rows.len() < MAX_ROWS_PER_SET
                            && self.sets[index].columns.len() == row.len()
                    });
                    if self.retention_limited
                        || !fits_set
                        || self.row_count >= MAX_ROWS
                        || bytes > 2 * 1024 * 1024
                        || self.row_bytes.saturating_add(bytes) > MAX_ROW_BYTES
                        || !self.retain(bytes)
                    {
                        self.native_omitted_rows += 1;
                        if let Some(index) = set_index {
                            self.sets[index].omitted_rows += 1;
                        }
                        self.retention_limited = true;
                    } else if let Some(index) = set_index {
                        self.row_bytes += bytes;
                        self.row_count += 1;
                        let set = &mut self.sets[index];
                        geometry_changed |= set.widths.sample(&row, set.rows.len());
                        set.rows.push(row.into());
                    }
                }
                if geometry_changed && let Some(index) = set_index {
                    self.sets[index].widths.rebuild_offsets();
                }
            }
            QueryEvent::ResultSetCompleted {
                result_set_index,
                row_count,
                partial,
                limit,
            } => {
                if let Some(set) = self
                    .sets
                    .iter_mut()
                    .find(|set| set.index == result_set_index)
                {
                    set.row_count = Some(row_count);
                    set.partial = partial;
                    set.limit = limit;
                }
            }
            QueryEvent::Notice { severity, message } => {
                if self.retain(metadata_bytes) {
                    self.notices.push(Notice { severity, message });
                } else {
                    self.native_omitted_metadata = self
                        .native_omitted_metadata
                        .saturating_add(metadata_bytes as u64);
                }
            }
            QueryEvent::ExecutionCompleted {
                status,
                omitted_rows,
                omitted_result_sets,
                omitted_notices,
                omitted_metadata_bytes,
                truncation_reasons,
                error,
                refusal,
                ..
            } => {
                let retain_details = self.retain(metadata_bytes);
                if !retain_details {
                    self.native_omitted_metadata = self
                        .native_omitted_metadata
                        .saturating_add(metadata_bytes as u64);
                }
                self.completion = Some(Completion {
                    status: match status.as_str() {
                        "completed" => TerminalStatus::Completed,
                        "cancelled" => TerminalStatus::Cancelled,
                        _ => TerminalStatus::Failed,
                    },
                    omitted_rows,
                    omitted_result_sets,
                    omitted_notices,
                    omitted_metadata_bytes,
                    truncation_reasons: if retain_details {
                        truncation_reasons
                    } else {
                        Vec::new()
                    },
                    error: if retain_details { error } else { None },
                    refusal: if retain_details { refusal } else { None },
                });
            }
            QueryEvent::SessionState { .. }
            | QueryEvent::ExecutionStarted
            | QueryEvent::SessionLost { .. }
            | QueryEvent::SessionClosed => {}
        }
        !self.retention_limited
    }

    fn retain(&mut self, bytes: usize) -> bool {
        if bytes > self.byte_limit.saturating_sub(self.retained_bytes) {
            self.retention_limited = true;
            false
        } else {
            self.retained_bytes += bytes;
            true
        }
    }
}

/// Exact JSON byte count, with fixed memory regardless of payload length.
pub fn encoded_size(value: &impl Serialize) -> usize {
    struct Counter(usize);
    impl io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(0);
    match serde_json::to_writer(&mut count, value) {
        Ok(()) => count.0,
        Err(_) => usize::MAX,
    }
}

/// NULL remains distinct from an empty string in rendering, copy and AX.
pub fn cell_text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("NULL")
}

pub fn display_text(value: &Option<String>) -> String {
    let text = cell_text(value);
    match text.char_indices().nth(DISPLAY_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::QueryTransactionSnapshot;

    fn start(index: u32, columns: usize) -> QueryEvent {
        QueryEvent::ResultSetStarted {
            result_set_index: index,
            columns: (0..columns).map(|i| Some(format!("column{i}"))).collect(),
        }
    }

    fn finish(status: &str) -> QueryEvent {
        QueryEvent::ExecutionCompleted {
            status: status.into(),
            transaction: QueryTransactionSnapshot::default(),
            omitted_rows: 4,
            omitted_result_sets: 2,
            omitted_notices: 1,
            omitted_metadata_bytes: 12,
            truncation_reasons: vec!["cellBytes".into()],
            error: None,
            refusal: None,
        }
    }

    #[test]
    fn preserves_null_empty_numeric_and_unicode_across_result_sets() {
        let mut model = ResultModel::default();
        model.consume(start(0, 4));
        let row = vec![
            None,
            Some(String::new()),
            Some("9007199254740993.0000".into()),
            Some("SQL '雪'".into()),
        ];
        model.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![row.clone()],
        });
        model.consume(start(1, 1));
        model.consume(QueryEvent::ResultSetCompleted {
            result_set_index: 1,
            row_count: 0,
            partial: false,
            limit: None,
        });
        assert_eq!(&*model.sets[0].rows[0], row);
        assert_eq!(cell_text(&row[0]), "NULL");
        assert_eq!(cell_text(&row[1]), "");
        assert_eq!(model.sets[1].row_count, Some(0));
        assert_eq!(model.sets[1].columns.len(), 1);
    }

    #[test]
    fn budget_accepts_partial_batch_and_keeps_draining_to_one_terminal() {
        let metadata =
            encoded_size(&start(0, 1)) + crate::column_widths::ColumnWidths::storage_bytes(1);
        let row = vec![Some("\\\"雪".to_string())];
        let mut model = ResultModel::with_byte_limit(metadata + encoded_size(&row));
        assert!(model.consume(start(0, 1)));
        assert!(!model.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![row.clone(), row]
        }));
        assert_eq!(model.sets[0].rows.len(), 1);
        assert_eq!(model.native_omitted_rows, 1);
        model.consume(finish("cancelled"));
        model.consume(finish("completed"));
        let completion = model.completion.as_ref().unwrap();
        assert_eq!(completion.status, TerminalStatus::Cancelled);
        assert_eq!(completion.omitted_rows, 4);
        assert!(model.native_omitted_metadata > 0);
        assert!(model.retained_bytes <= model.byte_limit);
    }

    #[test]
    fn column_geometry_is_admitted_before_allocation_and_is_result_local() {
        let metadata = encoded_size(&start(0, 1));
        let mut refused = ResultModel::with_byte_limit(metadata);
        assert!(!refused.consume(start(0, 1)));
        assert!(refused.sets.is_empty());
        assert_eq!(refused.native_omitted_result_sets, 1);
        assert_eq!(refused.native_omitted_metadata, metadata as u64);
        let mut model = ResultModel::default();
        model.consume(start(0, 1));
        model.consume(start(1, 1));
        model.consume(QueryEvent::RowBatch {
            result_set_index: 1,
            rows: vec![vec![Some("x".repeat(100))]],
        });
        assert_eq!(model.sets[1].widths.width(0), 400.);
        assert!(model.sets[0].widths.width(0) < 400.);
        assert_eq!(model.sets[1].widths.total_width(), 400.);
        let retained = model.retained_bytes;
        assert!(model.sets[1].widths.toggle_pin(0));
        assert_eq!(model.sets[1].widths.pinned_count(), 1);
        assert_eq!(model.sets[0].widths.pinned_count(), 0);
        assert_eq!(model.retained_bytes, retained);
    }

    #[test]
    fn shared_allowance_preserves_existing_rows_and_stops_new_retention() {
        let row = vec![Some("one exact row".into())];
        let metadata =
            encoded_size(&start(0, 1)) + crate::column_widths::ColumnWidths::storage_bytes(1);
        let row_bytes = encoded_size(&row);
        let total = metadata * 2 + row_bytes * 2;
        let mut first = ResultModel::default();
        first.set_byte_limit(total);
        first.consume(start(0, 1));
        first.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![row.clone()],
        });
        let mut second = ResultModel::default();
        second.set_byte_limit(total - first.retained_bytes);
        second.consume(start(0, 1));
        assert!(!second.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![row.clone(), row]
        }));
        assert_eq!(first.sets[0].rows.len(), 1);
        assert_eq!(second.sets[0].rows.len(), 1);
        assert!(first.retained_bytes + second.retained_bytes <= total);
        second.consume(finish("completed"));
        assert!(second.completion.is_some());
    }

    #[test]
    fn metadata_and_notices_share_budget_and_refuse_further_rows() {
        let mut model = ResultModel::with_byte_limit(
            encoded_size(&start(0, 1)) + crate::column_widths::ColumnWidths::storage_bytes(1),
        );
        model.consume(start(0, 1));
        assert!(!model.consume(QueryEvent::Notice {
            severity: "NOTICE".into(),
            message: "x".repeat(100)
        }));
        assert!(model.notices.is_empty());
        assert!(!model.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![vec![None]]
        }));
        assert!(model.sets[0].rows.is_empty());
    }

    #[test]
    fn terminal_preserves_error_position_and_omission_details() {
        let mut event = finish("failed");
        if let QueryEvent::ExecutionCompleted { error, .. } = &mut event {
            *error = Some(QueryDatabaseError {
                code: Some("42601".into()),
                message: "syntax error".into(),
                severity: Some("ERROR".into()),
                position: Some(9),
            });
        }
        let mut model = ResultModel::default();
        model.consume(event);
        let completion = model.completion.unwrap();
        assert_eq!(completion.error.unwrap().position, Some(9));
        assert_eq!(completion.truncation_reasons, ["cellBytes"]);
        assert_eq!(completion.omitted_result_sets, 2);
    }

    #[test]
    fn display_prefix_is_utf8_safe_but_exact_value_remains_available() {
        let value = Some("雪".repeat(DISPLAY_CHARS + 1));
        assert_eq!(
            display_text(&value),
            format!("{}…", "雪".repeat(DISPLAY_CHARS))
        );
        assert_eq!(cell_text(&value).chars().count(), DISPLAY_CHARS + 1);
    }

    #[test]
    fn row_count_limit_refuses_retention_but_records_server_completion() {
        let mut model = ResultModel::default();
        model.consume(start(0, 1));
        assert!(!model.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![vec![None]; MAX_ROWS_PER_SET + 1],
        }));
        model.consume(QueryEvent::ResultSetCompleted {
            result_set_index: 0,
            row_count: 100_000,
            partial: true,
            limit: Some(RowLimitOutcome::Drained),
        });
        assert_eq!(model.sets[0].rows.len(), MAX_ROWS_PER_SET);
        assert_eq!(model.sets[0].omitted_rows, 1);
        assert_eq!(model.sets[0].row_count, Some(100_000));
        assert!(model.sets[0].partial);
    }

    #[test]
    fn an_unretained_column_set_never_leaves_unrenderable_rows() {
        let mut model = ResultModel::with_byte_limit(1);
        assert!(!model.consume(start(0, 1)));
        assert!(!model.consume(QueryEvent::RowBatch {
            result_set_index: 0,
            rows: vec![vec![None]],
        }));
        assert!(model.sets.is_empty());
        assert_eq!(model.native_omitted_rows, 1);
        assert_eq!(model.native_omitted_result_sets, 1);
    }
}
