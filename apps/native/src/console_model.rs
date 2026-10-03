//! Global console tail behind the Dock, matching the baseline slice: a
//! 500-event ring that never auto-opens and counts unread events while hidden.
//! Text is clipped on character boundaries so the tail stays bounded. Task
//! and export progress events are not yet routed here.
use std::{collections::VecDeque, time::SystemTime};

const EVENT_CAP: usize = 500;
const MESSAGE_CHARS: usize = 512;
const DETAIL_CHARS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}
impl Severity {
    /// Baseline `consoleSeverityForNotice`.
    pub fn for_notice(severity: &str) -> Self {
        let normalized = severity.to_uppercase();
        if normalized.contains("ERROR") || normalized == "FATAL" || normalized == "PANIC" {
            Self::Error
        } else if normalized.contains("WARN") {
            Self::Warning
        } else {
            Self::Info
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Connection,
    Notice,
    Query,
}
impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::Notice => "notice",
            Self::Query => "query",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub severity: Severity,
    pub source: Source,
    pub message: String,
    pub detail: Option<String>,
    pub connection: Option<String>,
    /// Wall time of a finished query, for the status bar.
    pub latency_ms: Option<u64>,
}
pub struct Event {
    pub at: SystemTime,
    pub entry: Entry,
}

fn clip(text: &str, chars: usize) -> String {
    let mut iter = text.chars();
    let mut out: String = iter.by_ref().take(chars).collect();
    if iter.next().is_some() {
        out.push('…');
    }
    out
}

#[derive(Default)]
pub struct Console {
    events: VecDeque<Event>,
    pub unread: usize,
    pub open: bool,
}
impl Console {
    pub fn append(&mut self, mut entry: Entry) {
        entry.message = clip(&entry.message, MESSAGE_CHARS);
        entry.detail = entry.detail.map(|detail| clip(&detail, DETAIL_CHARS));
        if self.events.len() == EVENT_CAP {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at: SystemTime::now(),
            entry,
        });
        if !self.open {
            self.unread = (self.unread + 1).min(EVENT_CAP);
        }
    }
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        if open {
            self.unread = 0;
        }
    }
    pub fn clear(&mut self) {
        self.events.clear();
        self.unread = 0;
    }
    pub fn visible(&self, filter: Option<Severity>) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|event| filter.is_none_or(|severity| event.entry.severity == severity))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(severity: Severity, message: &str) -> Entry {
        Entry {
            severity,
            source: Source::Query,
            message: message.into(),
            detail: None,
            connection: None,
            latency_ms: None,
        }
    }

    #[test]
    fn hidden_appends_count_unread_and_opening_clears_without_auto_open() {
        let mut console = Console::default();
        console.append(entry(Severity::Info, "Query on local · 3 rows · 12 ms"));
        console.append(entry(Severity::Warning, "WARNING: something"));
        assert_eq!((console.unread, console.open), (2, false));
        console.set_open(true);
        console.append(entry(Severity::Error, "Query failed on local"));
        assert_eq!(console.unread, 0);
        assert_eq!(console.visible(Some(Severity::Warning)).len(), 1);
        assert_eq!(console.visible(None).len(), 3);
        console.clear();
        assert!(console.visible(None).is_empty());
    }

    #[test]
    fn ring_and_text_are_bounded() {
        let mut console = Console::default();
        for i in 0..EVENT_CAP + 25 {
            console.append(entry(Severity::Info, &format!("event {i}")));
        }
        let visible = console.visible(None);
        assert_eq!(visible.len(), EVENT_CAP);
        assert_eq!(visible[0].entry.message, "event 25");
        assert_eq!(console.unread, EVENT_CAP);
        console.append(Entry {
            detail: Some("字".repeat(DETAIL_CHARS + 1)),
            ..entry(Severity::Info, &"é".repeat(MESSAGE_CHARS + 5))
        });
        let last = console.visible(None).pop().unwrap();
        assert_eq!(last.entry.message.chars().count(), MESSAGE_CHARS + 1);
        assert_eq!(
            last.entry.detail.as_ref().unwrap().chars().count(),
            DETAIL_CHARS + 1
        );
    }

    #[test]
    fn notice_severity_matches_baseline_mapping() {
        for (input, expected) in [
            ("ERROR", Severity::Error),
            ("fatal", Severity::Error),
            ("PANIC", Severity::Error),
            ("WARNING", Severity::Warning),
            ("NOTICE", Severity::Info),
            ("DEBUG", Severity::Info),
        ] {
            assert_eq!(Severity::for_notice(input), expected, "{input}");
        }
    }
}
