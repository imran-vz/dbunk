//! Bounded "Recent queries" for the overview. Profile-local history is read
//! through the document's library lane in at most `MAX_PAGES` explicit pages;
//! a reply is admitted only for the exact generation and connection that asked.
use dbunk_lib::backend::query_library::{
    HistoryRecord, LibraryCursor, LibraryPage, LibraryRequest,
};
use std::{cell::Cell, mem::size_of, rc::Rc};

pub const RECENT_LIMIT: usize = 20;
/// An empty filtered page may still carry a continuation. Never follow more
/// than this many pages for one explicit refresh.
pub const MAX_PAGES: usize = 3;
/// Retained exact SQL and labels for the whole list, charged to the shared
/// workspace allowance in addition to this cap.
pub const RECENT_BYTES: usize = 4 * 1024 * 1024;
const SHARED_BYTES: usize = 128 * 1024 * 1024;
const PREVIEW_CHARS: usize = 160;
const FIELD_CHARS: usize = 64;

/// Shared-budget allowance; released on drop.
struct Lease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Lease {
    fn grow(&mut self, bytes: usize) -> bool {
        if bytes > SHARED_BYTES.saturating_sub(self.budget.get()) {
            return false;
        }
        self.budget.set(self.budget.get() + bytes);
        self.bytes += bytes;
        true
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

pub struct Entry {
    pub sql: String,
    pub label: String,
}
impl Entry {
    fn new(record: HistoryRecord) -> Self {
        let status = match record.status.as_str() {
            "success" => "OK".to_owned(),
            "error" => "Error".to_owned(),
            other => single_line(other, FIELD_CHARS),
        };
        let label = format!(
            "{status} · {} · {} · {}",
            duration(record.runtime_ms),
            single_line(&record.started_at, FIELD_CHARS),
            single_line(&record.sql, PREVIEW_CHARS)
        );
        Self {
            sql: record.sql,
            label,
        }
    }
    fn bytes(&self) -> usize {
        size_of::<Self>() + self.sql.capacity() + self.label.capacity()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// The service reported no continuation.
    Exhausted,
    /// The cap was reached with at least one further record or continuation.
    Limit,
    /// `MAX_PAGES` were read and a continuation remained.
    Pages,
    /// The next record exceeded the retained byte cap or shared allowance.
    Memory,
}

/// A finished, immutable list for one connection.
pub struct Recent {
    pub connection: String,
    pub entries: Vec<Entry>,
    pub ended: Ended,
    pub revision: u64,
    _lease: Lease,
}
impl Recent {
    #[cfg(test)]
    fn more_may_exist(&self) -> bool {
        self.ended != Ended::Exhausted
    }
    pub fn summary(&self) -> String {
        let count = self.entries.len();
        match self.ended {
            Ended::Exhausted if count == 0 => {
                "No recorded queries for this connection in this profile".into()
            }
            Ended::Exhausted => format!("{count} most recent recorded queries; no more exist"),
            Ended::Limit => format!("{count} most recent recorded queries; older history exists"),
            Ended::Pages => format!(
                "{count} recorded queries after {MAX_PAGES} pages; more may exist. Use the History tool for the rest"
            ),
            Ended::Memory => format!(
                "{count} recorded queries retained; the next record exceeds the memory allowance. Use the History tool"
            ),
        }
    }
}

/// One explicit refresh in progress. Holds the records admitted so far.
pub struct Loader {
    generation: u64,
    connection: String,
    pages: usize,
    entries: Vec<Entry>,
    lease: Lease,
}

pub enum Step {
    /// Send this request next; it continues the same generation.
    Continue(LibraryRequest),
    Done(Recent),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Different generation or connection; discard silently.
    Stale,
    /// The reply violated the requested filter; keep the previous list.
    Invalid(&'static str),
}

/// Owns request generations across refreshes so a late reply from an older
/// refresh cannot be admitted after a reset or rebind.
#[derive(Default)]
pub struct Requests {
    generation: u64,
    loader: Option<Loader>,
}
impl Requests {
    pub fn loading(&self) -> bool {
        self.loader.is_some()
    }
    pub fn generation(&self) -> Option<u64> {
        self.loader.as_ref().map(|loader| loader.generation)
    }
    /// Drops any in-flight accumulation; its late reply becomes stale.
    pub fn reset(&mut self) {
        self.loader = None;
    }
    pub fn begin(
        &mut self,
        connection: &str,
        budget: Rc<Cell<usize>>,
    ) -> Result<(u64, LibraryRequest), &'static str> {
        if self.loader.is_some() {
            return Err("Recent queries are already loading");
        }
        if connection.is_empty() || connection.len() > 256 {
            return Err("Recent queries need a bound connection");
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("Recent query identity exhausted; reopen this tab")?;
        self.generation = generation;
        self.loader = Some(Loader {
            generation,
            connection: connection.to_owned(),
            pages: 0,
            entries: Vec::new(),
            lease: Lease { budget, bytes: 0 },
        });
        Ok((generation, request(connection, None, RECENT_LIMIT)))
    }
    /// Admits one page. Any refusal other than `Stale` drops the in-flight
    /// accumulation; there is no automatic retry.
    pub fn receive(
        &mut self,
        generation: u64,
        connection: Option<&str>,
        page: LibraryPage<HistoryRecord>,
    ) -> Result<Step, Refusal> {
        let Some(loader) = self.loader.as_mut() else {
            return Err(Refusal::Stale);
        };
        // An older generation's late reply never disturbs the current request.
        if loader.generation != generation {
            return Err(Refusal::Stale);
        }
        // The document was rebound while this generation was in flight.
        if connection != Some(loader.connection.as_str()) {
            self.loader = None;
            return Err(Refusal::Stale);
        }
        if page
            .entries
            .iter()
            .any(|record| record.connection_id != loader.connection)
        {
            self.loader = None;
            return Err(Refusal::Invalid(
                "History reply contained another connection's record; previous list retained",
            ));
        }
        loader.pages += 1;
        let LibraryPage { entries, next } = page;
        let received = entries.len();
        let mut admitted = 0;
        let mut ended = None;
        for record in entries {
            if loader.entries.len() == RECENT_LIMIT {
                ended = Some(Ended::Limit);
                break;
            }
            let entry = Entry::new(record);
            let bytes = entry.bytes();
            if loader.lease.bytes + bytes > RECENT_BYTES || !loader.lease.grow(bytes) {
                ended = Some(Ended::Memory);
                break;
            }
            loader.entries.push(entry);
            admitted += 1;
        }
        let ended = match ended {
            Some(ended) => Some(ended),
            None if loader.entries.len() == RECENT_LIMIT => {
                Some(if admitted < received || next.is_some() {
                    Ended::Limit
                } else {
                    Ended::Exhausted
                })
            }
            None if next.is_none() => Some(Ended::Exhausted),
            None if loader.pages >= MAX_PAGES => Some(Ended::Pages),
            None => None,
        };
        match ended {
            Some(ended) => {
                let loader = self.loader.take().expect("loader checked above");
                Ok(Step::Done(Recent {
                    connection: loader.connection,
                    entries: loader.entries,
                    ended,
                    revision: loader.generation,
                    _lease: loader.lease,
                }))
            }
            None => Ok(Step::Continue(request(
                &loader.connection,
                next,
                RECENT_LIMIT - loader.entries.len(),
            ))),
        }
    }
}

fn request(connection: &str, cursor: Option<LibraryCursor>, limit: usize) -> LibraryRequest {
    LibraryRequest {
        limit: limit.clamp(1, RECENT_LIMIT) as u32,
        cursor,
        connection_id: Some(connection.to_owned()),
        search: String::new(),
        status: None,
    }
}

pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{}.{} s", ms / 1000, (ms % 1000) / 100)
    } else {
        format!("{} min {} s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// Collapses whitespace/control runs to one space and clips by characters,
/// reading at most `max + 1` visible characters of the input.
pub fn single_line(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_control() {
            space = count > 0;
            continue;
        }
        if count == max {
            out.push('…');
            return out;
        }
        if space {
            out.push(' ');
            count += 1;
            space = false;
            if count == max {
                out.push('…');
                return out;
            }
        }
        out.push(ch);
        count += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(connection: &str, index: usize, sql: &str) -> HistoryRecord {
        HistoryRecord {
            id: format!("id-{index}"),
            sql: sql.into(),
            connection_id: connection.into(),
            connection_name: "Local".into(),
            database: "db".into(),
            engine: "postgres".into(),
            status: if index.is_multiple_of(2) {
                "success"
            } else {
                "error"
            }
            .into(),
            error_message: None,
            runtime_ms: 1500,
            row_count: None,
            started_at: format!("2026-10-03T10:00:{index:02}Z"),
        }
    }
    fn cursor() -> Option<LibraryCursor> {
        Some(LibraryCursor {
            timestamp: "t".into(),
            id: "id".into(),
            favorite: false,
        })
    }
    fn page(
        connection: &str,
        count: usize,
        next: Option<LibraryCursor>,
    ) -> LibraryPage<HistoryRecord> {
        LibraryPage {
            entries: (0..count)
                .map(|index| record(connection, index, "select 1"))
                .collect(),
            next,
        }
    }
    fn done(step: Result<Step, Refusal>) -> Recent {
        match step {
            Ok(Step::Done(recent)) => recent,
            _ => panic!("expected a finished list"),
        }
    }

    #[test]
    fn recent_queries_cap_at_twenty_and_disclose_older_history() {
        let budget = Rc::new(Cell::new(0));
        let mut requests = Requests::default();
        let (generation, request) = requests.begin("c1", budget.clone()).unwrap();
        assert_eq!(request.limit, RECENT_LIMIT as u32);
        assert_eq!(request.connection_id.as_deref(), Some("c1"));
        assert!(request.cursor.is_none() && request.search.is_empty());
        let recent = done(requests.receive(generation, Some("c1"), page("c1", 25, None)));
        assert_eq!(recent.entries.len(), RECENT_LIMIT);
        assert_eq!(recent.ended, Ended::Limit);
        assert!(recent.more_may_exist());
        assert!(budget.get() > 0);
        drop(recent);
        assert_eq!(budget.get(), 0);
        let (generation, _) = requests.begin("c1", budget.clone()).unwrap();
        let recent = done(requests.receive(generation, Some("c1"), page("c1", 20, None)));
        assert_eq!(recent.ended, Ended::Exhausted);
        assert!(!recent.more_may_exist());
    }

    #[test]
    fn empty_continuation_pages_are_followed_at_most_max_pages() {
        let budget = Rc::new(Cell::new(0));
        let mut requests = Requests::default();
        let (generation, _) = requests.begin("c1", budget.clone()).unwrap();
        for _ in 1..MAX_PAGES {
            match requests.receive(generation, Some("c1"), page("c1", 0, cursor())) {
                Ok(Step::Continue(request)) => {
                    assert_eq!(request.cursor, cursor());
                    assert_eq!(request.limit, RECENT_LIMIT as u32);
                }
                _ => panic!("empty page with continuation must continue"),
            }
        }
        let recent = done(requests.receive(generation, Some("c1"), page("c1", 0, cursor())));
        assert_eq!(recent.ended, Ended::Pages);
        assert!(recent.entries.is_empty() && recent.more_may_exist());
        assert!(!requests.loading());
    }

    #[test]
    fn short_page_continues_for_only_the_remaining_records() {
        let budget = Rc::new(Cell::new(0));
        let mut requests = Requests::default();
        let (generation, _) = requests.begin("c1", budget.clone()).unwrap();
        let Ok(Step::Continue(request)) =
            requests.receive(generation, Some("c1"), page("c1", 15, cursor()))
        else {
            panic!("short page with continuation must continue");
        };
        assert_eq!(request.limit, 5);
        let recent = done(requests.receive(generation, Some("c1"), page("c1", 5, cursor())));
        assert_eq!(recent.entries.len(), RECENT_LIMIT);
        assert_eq!(recent.ended, Ended::Limit);
    }

    #[test]
    fn stale_generation_or_rebound_connection_is_discarded() {
        let budget = Rc::new(Cell::new(0));
        let mut requests = Requests::default();
        assert_eq!(
            requests.receive(1, Some("c1"), page("c1", 1, None)).err(),
            Some(Refusal::Stale)
        );
        let (old, _) = requests.begin("c1", budget.clone()).unwrap();
        assert!(requests.begin("c1", budget.clone()).is_err());
        requests.reset();
        let (current, _) = requests.begin("c1", budget.clone()).unwrap();
        assert_ne!(old, current);
        assert_eq!(
            requests.receive(old, Some("c1"), page("c1", 1, None)).err(),
            Some(Refusal::Stale)
        );
        assert_eq!(requests.generation(), Some(current));
        let recent = done(requests.receive(current, Some("c1"), page("c1", 1, None)));
        assert_eq!(recent.revision, current);
        drop(recent);
        let (current, _) = requests.begin("c1", budget.clone()).unwrap();
        assert_eq!(
            requests
                .receive(current, Some("c2"), page("c1", 1, None))
                .err(),
            Some(Refusal::Stale)
        );
        let (current, _) = requests.begin("c1", budget.clone()).unwrap();
        assert!(matches!(
            requests.receive(current, Some("c1"), page("c2", 1, None)),
            Err(Refusal::Invalid(_))
        ));
        assert!(!requests.loading());
        assert_eq!(budget.get(), 0);
    }

    #[test]
    fn retention_stops_at_memory_allowance_and_discloses_it() {
        let budget = Rc::new(Cell::new(SHARED_BYTES - 1));
        let mut requests = Requests::default();
        let (generation, _) = requests.begin("c1", budget.clone()).unwrap();
        let recent = done(requests.receive(generation, Some("c1"), page("c1", 3, None)));
        assert_eq!(recent.ended, Ended::Memory);
        assert!(recent.entries.is_empty() && recent.more_may_exist());
        drop(recent);
        assert_eq!(budget.get(), SHARED_BYTES - 1);

        budget.set(0);
        let large = "x".repeat(RECENT_BYTES / 2);
        let (generation, _) = requests.begin("c1", budget.clone()).unwrap();
        let big = LibraryPage {
            entries: (0..3).map(|index| record("c1", index, &large)).collect(),
            next: None,
        };
        let recent = done(requests.receive(generation, Some("c1"), big));
        assert_eq!(recent.ended, Ended::Memory);
        assert_eq!(recent.entries.len(), 1);
        assert_eq!(recent.entries[0].sql, large);
    }

    #[test]
    fn labels_are_single_line_unicode_safe_and_keep_exact_sql() {
        assert_eq!(
            single_line("  select\n\t1\r\n from  t ", 64),
            "select 1 from t"
        );
        let clipped = single_line(&"雪".repeat(500), 10);
        assert_eq!(clipped.chars().count(), 11);
        assert!(clipped.ends_with('…'));
        assert_eq!(single_line("ab cd", 3), "ab …");
        assert_eq!(single_line("abc", 3), "abc");
        assert_eq!(duration(12), "12 ms");
        assert_eq!(duration(1500), "1.5 s");
        assert_eq!(duration(61_000), "1 min 1 s");
        let sql = "select '🙂'\n  from t";
        let entry = Entry::new(record("c1", 0, sql));
        assert_eq!(entry.sql, sql);
        assert_eq!(
            entry.label,
            "OK · 1.5 s · 2026-10-03T10:00:00Z · select '🙂' from t"
        );
        assert!(!entry.label.contains('\n'));
    }
}
