//! Server-reported (`GUC_REPORT`) session parameters, observed by the socket
//! driver. tokio-postgres applies each ParameterStatus inside `poll_message`
//! and can route later responses to the client during the same poll. The
//! driver therefore publishes after every poll under an odd/even epoch, and a
//! reader that has already received a response waits for that poll to settle.
//! Readers never send SQL on the observed session.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Order is part of the snapshot layout. `search_path` is reported only by
/// servers that mark it `GUC_REPORT`; older servers leave it absent.
pub(crate) const REPORTED: [&str; 4] = [
    "client_encoding",
    "DateStyle",
    "IntervalStyle",
    "search_path",
];
/// Longer values are recorded as unknown rather than retained.
pub(crate) const MAX_REPORTED_BYTES: usize = 4096;
/// A settle wait is a bounded number of scheduler yields, never a sleep.
const SETTLE_YIELDS: usize = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReportedSnapshot {
    /// Incremented whenever any tracked value changes (or becomes unknown).
    pub generation: u64,
    pub values: [Option<String>; 4],
}

impl ReportedSnapshot {
    pub(crate) fn value(&self, name: &str) -> Option<&str> {
        REPORTED
            .iter()
            .position(|candidate| *candidate == name)
            .and_then(|index| self.values[index].as_deref())
    }
}

#[derive(Default)]
pub(crate) struct ReportedParameters {
    epoch: AtomicU64,
    snapshot: Mutex<ReportedSnapshot>,
}

impl ReportedParameters {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Driver side: called immediately before polling the connection.
    pub(crate) fn enter(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Driver side: publishes the connection's current values, then closes the
    /// poll epoch. Only a changed value allocates.
    pub(crate) fn leave<'a>(&self, lookup: impl Fn(&str) -> Option<&'a str>) {
        {
            let mut snapshot = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
            let mut changed = false;
            for (index, name) in REPORTED.iter().enumerate() {
                let current = lookup(name).filter(|value| value.len() <= MAX_REPORTED_BYTES);
                if snapshot.values[index].as_deref() != current {
                    snapshot.values[index] = current.map(str::to_owned);
                    changed = true;
                }
            }
            if changed {
                snapshot.generation = snapshot.generation.wrapping_add(1);
            }
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Reader side: a snapshot that includes every ParameterStatus processed
    /// before the caller observed its last response. `None` when the driver
    /// does not settle within the bounded yield budget.
    pub(crate) async fn settled(&self) -> Option<ReportedSnapshot> {
        let first = self.epoch.load(Ordering::SeqCst);
        for _ in 0..SETTLE_YIELDS {
            let epoch = self.epoch.load(Ordering::SeqCst);
            if epoch.is_multiple_of(2) || epoch != first {
                return Some(
                    self.snapshot
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone(),
                );
            }
            tokio::task::yield_now().await;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn publish(reported: &ReportedParameters, values: &HashMap<&str, String>) {
        reported.enter();
        reported.leave(|name| values.get(name).map(String::as_str));
    }

    #[tokio::test]
    async fn changes_bump_generation_and_identical_polls_do_not() {
        let reported = ReportedParameters::new();
        let mut values = HashMap::from([
            ("client_encoding", "UTF8".to_owned()),
            ("DateStyle", "ISO, MDY".to_owned()),
        ]);
        publish(&reported, &values);
        let first = reported.settled().await.unwrap();
        assert_eq!(first.value("client_encoding"), Some("UTF8"));
        assert_eq!(first.value("search_path"), None);
        publish(&reported, &values);
        assert_eq!(reported.settled().await.unwrap(), first);
        values.insert("client_encoding", "LATIN1".into());
        publish(&reported, &values);
        let changed = reported.settled().await.unwrap();
        assert_ne!(changed.generation, first.generation);
        assert_eq!(changed.value("client_encoding"), Some("LATIN1"));
    }

    #[tokio::test]
    async fn oversized_values_are_unknown_not_retained() {
        let reported = ReportedParameters::new();
        let values = HashMap::from([("search_path", "x".repeat(MAX_REPORTED_BYTES + 1))]);
        publish(&reported, &values);
        assert_eq!(reported.settled().await.unwrap().value("search_path"), None);
    }

    #[tokio::test]
    async fn a_reader_waits_for_an_open_poll_and_gives_up_when_it_never_settles() {
        let reported = ReportedParameters::new();
        reported.enter();
        assert!(reported.settled().await.is_none());
        let waiting = tokio::spawn({
            let reported = reported.clone();
            async move { reported.settled().await }
        });
        tokio::task::yield_now().await;
        reported.leave(|name| (name == "client_encoding").then_some("UTF8"));
        let settled = waiting.await.unwrap().unwrap();
        assert_eq!(settled.value("client_encoding"), Some("UTF8"));
    }
}
