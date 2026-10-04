//! Per-connection ClickHouse session phases. Pure and generic over the session
//! handle so every transition is unit-tested. One attempt at a time per
//! connection; a result for an attempt that was superseded or disconnected is
//! handed back for closing instead of being installed. Nothing retries.
use crate::document_view::ConnectionPhase;
use std::collections::HashMap;

struct Entry<S> {
    phase: ConnectionPhase,
    session: Option<S>,
    attempt: u64,
}

pub struct SessionModel<S> {
    entries: HashMap<String, Entry<S>>,
    next_attempt: u64,
}

impl<S> Default for SessionModel<S> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            next_attempt: 0,
        }
    }
}

impl<S: Clone> SessionModel<S> {
    pub fn phase(&self, id: &str) -> ConnectionPhase {
        self.entries
            .get(id)
            .map_or(ConnectionPhase::Idle, |entry| entry.phase.clone())
    }

    pub fn session(&self, id: &str) -> Option<S> {
        self.entries.get(id)?.session.clone()
    }

    /// Starts an attempt unless one is open or opening; returns its token.
    pub fn begin(&mut self, id: &str) -> Option<u64> {
        if matches!(
            self.phase(id),
            ConnectionPhase::Connecting | ConnectionPhase::Connected
        ) {
            return None;
        }
        self.next_attempt += 1;
        self.entries.insert(
            id.to_owned(),
            Entry {
                phase: ConnectionPhase::Connecting,
                session: None,
                attempt: self.next_attempt,
            },
        );
        Some(self.next_attempt)
    }

    /// Settles `attempt`. A stale success is returned so the caller closes it.
    pub fn settle(&mut self, id: &str, attempt: u64, result: Result<S, String>) -> Option<S> {
        let current = self
            .entries
            .get_mut(id)
            .filter(|entry| entry.attempt == attempt && entry.phase == ConnectionPhase::Connecting);
        match (current, result) {
            (Some(entry), Ok(session)) => {
                entry.phase = ConnectionPhase::Connected;
                entry.session = Some(session);
                None
            }
            (Some(entry), Err(error)) => {
                entry.phase = ConnectionPhase::Failed(error);
                None
            }
            (None, Ok(session)) => Some(session),
            (None, Err(_)) => None,
        }
    }

    /// Explicit disconnect: forgets the connection and returns its session.
    pub fn end(&mut self, id: &str) -> Option<S> {
        self.entries.remove(id)?.session
    }

    /// A document saw the transport fail. Only the session it used is marked
    /// failed; a newer session from a later reconnect is left alone.
    pub fn lost(&mut self, id: &str, used: impl Fn(&S) -> bool, error: String) -> Option<S> {
        let entry = self.entries.get_mut(id)?;
        if !entry.session.as_ref().is_some_and(used) {
            return None;
        }
        entry.phase = ConnectionPhase::Failed(error);
        entry.session.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ConnectionPhase::*;

    #[test]
    fn one_attempt_at_a_time_and_failures_are_terminal() {
        let mut model = SessionModel::<u32>::default();
        let first = model.begin("a").unwrap();
        assert_eq!(model.phase("a"), Connecting);
        assert_eq!(model.begin("a"), None, "no second attempt while opening");
        assert_eq!(model.settle("a", first, Err("refused".into())), None);
        assert_eq!(model.phase("a"), Failed("refused".into()));
        assert_eq!(model.session("a"), None);
        // Only an explicit new attempt leaves Failed.
        let second = model.begin("a").unwrap();
        assert_eq!(model.settle("a", second, Ok(7)), None);
        assert_eq!(model.phase("a"), Connected);
        assert_eq!(model.begin("a"), None, "an open session is never restarted");
    }

    #[test]
    fn superseded_or_disconnected_attempts_hand_back_their_session() {
        let mut model = SessionModel::<u32>::default();
        let attempt = model.begin("a").unwrap();
        assert_eq!(model.end("a"), None);
        assert_eq!(model.phase("a"), Idle);
        assert_eq!(model.settle("a", attempt, Ok(1)), Some(1));
        assert_eq!(model.phase("a"), Idle);

        let old = model.begin("a").unwrap();
        model.end("a");
        let new = model.begin("a").unwrap();
        assert_eq!(model.settle("a", old, Ok(2)), Some(2));
        assert_eq!(model.phase("a"), Connecting);
        assert_eq!(model.settle("a", new, Ok(3)), None);
        assert_eq!(model.session("a"), Some(3));
    }

    #[test]
    fn loss_only_fails_the_session_that_was_used() {
        let mut model = SessionModel::<u32>::default();
        let attempt = model.begin("a").unwrap();
        model.settle("a", attempt, Ok(1));
        assert_eq!(model.lost("a", |s| *s == 9, "gone".into()), None);
        assert_eq!(model.phase("a"), Connected);
        assert_eq!(model.lost("a", |s| *s == 1, "gone".into()), Some(1));
        assert_eq!(model.phase("a"), Failed("gone".into()));
        assert_eq!(model.lost("a", |_| true, "again".into()), None);
    }
}
