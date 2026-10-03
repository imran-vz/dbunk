//! Bounded library models and execution summaries. Completion row counts already
//! include omitted rows; adding the terminal omission total would count twice.
use dbunk_lib::backend::{
    QueryEvent,
    query_library::{HistoryRecord, LibraryCursor, LibraryPage, SavedQueryRecord},
};
use serde::Serialize;
use std::{cell::Cell, rc::Rc, time::Instant};

#[derive(Serialize)]
pub enum Rows {
    History(LibraryPage<HistoryRecord>),
    Saved(LibraryPage<SavedQueryRecord>),
}
impl Rows {
    pub fn next(&self) -> Option<LibraryCursor> {
        match self {
            Self::History(page) => page.next.clone(),
            Self::Saved(page) => page.next.clone(),
        }
    }
    pub fn len(&self) -> usize {
        match self {
            Self::History(page) => page.entries.len(),
            Self::Saved(page) => page.entries.len(),
        }
    }
}

/// A displayed page replaces the previous page. Cursor existence, never row
/// count, controls continuation after the service's finite search scan.
pub struct Page {
    pub rows: Option<Rows>,
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Page {
    pub fn new(budget: Rc<Cell<usize>>) -> Self {
        Self {
            rows: None,
            budget,
            bytes: 0,
        }
    }
    pub fn replace(&mut self, rows: Rows) -> Result<(), &'static str> {
        let bytes = crate::results::encoded_size(&rows);
        let remaining = self.budget.get().saturating_sub(self.bytes);
        if bytes > (128 * 1024 * 1024_usize).saturating_sub(remaining) {
            return Err("Workspace memory budget is full. Clear results or close another tool.");
        }
        self.budget.set(remaining + bytes);
        self.bytes = bytes;
        self.rows = Some(rows);
        Ok(())
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

pub struct Capture {
    pub record: HistoryRecord,
    started: Instant,
    rows: u64,
    complete_counts: bool,
    opened_sets: u64,
    completed_sets: u64,
    execution_started: bool,
}
impl Capture {
    pub fn new(record: HistoryRecord) -> Self {
        Self {
            record,
            started: Instant::now(),
            rows: 0,
            complete_counts: true,
            opened_sets: 0,
            completed_sets: 0,
            execution_started: false,
        }
    }
    pub fn observe(&mut self, event: &QueryEvent) {
        if matches!(event, QueryEvent::ExecutionStarted) {
            self.execution_started = true;
            self.started = Instant::now();
            self.record.mark_started();
        }
        if let QueryEvent::ResultSetStarted {
            result_set_index, ..
        } = event
        {
            if *result_set_index < 64 {
                self.opened_sets |= 1_u64 << result_set_index;
            } else {
                self.complete_counts = false;
            }
        }
        if let QueryEvent::ResultSetCompleted {
            result_set_index,
            row_count,
            ..
        } = event
        {
            if *result_set_index >= 64 {
                self.complete_counts = false;
                return;
            }
            let bit = 1_u64 << result_set_index;
            if self.completed_sets & bit != 0 {
                self.complete_counts = false;
                return;
            }
            self.completed_sets |= bit;
            if let Some(total) = self.rows.checked_add(*row_count) {
                self.rows = total;
            } else {
                self.complete_counts = false;
            }
        }
    }
    pub fn finish(mut self, event: &QueryEvent) -> Option<HistoryRecord> {
        let QueryEvent::ExecutionCompleted {
            status,
            omitted_result_sets,
            error,
            refusal,
            ..
        } = event
        else {
            return None;
        };
        if status == "cancelled" {
            return None;
        }
        self.record.status = if status == "completed" {
            "success"
        } else {
            "error"
        }
        .into();
        self.record.error_message = error
            .as_ref()
            .map(|error| error.message.as_str())
            .or(refusal.as_deref())
            .map(history_error);
        self.record.runtime_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.record.row_count = (self.complete_counts
            && *omitted_result_sets == 0
            && self.opened_sets & !self.completed_sets == 0
            && (self.execution_started || status == "completed"))
            .then_some(self.rows);
        Some(self.record)
    }
}

/// Preserve UTF-8 and explicitly disclose a metadata truncation. The result
/// document retains the original backend error independently of history.
fn history_error(message: &str) -> String {
    const SUFFIX: &str = " [history message truncated]";
    if message.len() <= 8192 {
        return message.into();
    }
    let mut end = 8192 - SUFFIX.len();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{SUFFIX}", &message[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::QueryTransactionSnapshot;
    fn terminal(status: &str, omitted_result_sets: u32) -> QueryEvent {
        QueryEvent::ExecutionCompleted {
            status: status.into(),
            transaction: QueryTransactionSnapshot::default(),
            omitted_rows: 100,
            omitted_result_sets,
            omitted_notices: 0,
            omitted_metadata_bytes: 0,
            truncation_reasons: vec![],
            error: None,
            refusal: None,
            context: None,
        }
    }
    fn capture() -> Capture {
        Capture::new(HistoryRecord::started(
            "run".into(),
            "select 1".into(),
            "connection".into(),
            "Local".into(),
            "db".into(),
        ))
    }
    #[test]
    fn counts_use_authoritative_completions_without_adding_omissions() {
        let mut run = capture();
        run.observe(&QueryEvent::ResultSetCompleted {
            result_set_index: 0,
            row_count: 150,
            partial: true,
            limit: None,
        });
        run.observe(&QueryEvent::ResultSetCompleted {
            result_set_index: 1,
            row_count: 2,
            partial: false,
            limit: None,
        });
        assert_eq!(
            run.finish(&terminal("completed", 0)).unwrap().row_count,
            Some(152)
        );
        assert_eq!(
            capture()
                .finish(&terminal("completed", 1))
                .unwrap()
                .row_count,
            None
        );
        assert!(capture().finish(&terminal("cancelled", 0)).is_none());
        let mut unfinished = capture();
        unfinished.observe(&QueryEvent::ExecutionStarted);
        unfinished.observe(&QueryEvent::ResultSetStarted {
            result_set_index: 0,
            columns: vec![],
        });
        assert_eq!(
            unfinished.finish(&terminal("failed", 0)).unwrap().row_count,
            None
        );
        assert_eq!(
            capture().finish(&terminal("failed", 0)).unwrap().row_count,
            None
        );
        assert_eq!(
            capture().finish(&terminal("failed", 0)).unwrap().status,
            "error"
        );
    }
    #[test]
    fn empty_search_page_continues_and_retention_is_shared_and_released() {
        let budget = Rc::new(Cell::new(0));
        let rows = || {
            Rows::Saved(LibraryPage {
                entries: vec![],
                next: Some(LibraryCursor {
                    timestamp: "t".into(),
                    id: "id".into(),
                    favorite: false,
                }),
            })
        };
        let mut page = Page::new(budget.clone());
        page.replace(rows()).unwrap();
        let bytes = budget.get();
        assert!(page.rows.as_ref().unwrap().next().is_some());
        assert_eq!(page.rows.as_ref().unwrap().len(), 0);
        page.replace(rows()).unwrap();
        assert_eq!(budget.get(), bytes);
        budget.set(128 * 1024 * 1024 + bytes);
        assert!(page.replace(rows()).is_err());
        assert_eq!(page.bytes, bytes);
        drop(page);
        assert_eq!(budget.get(), 128 * 1024 * 1024);
    }
}
