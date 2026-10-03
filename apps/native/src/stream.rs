//! Admission for one session's ordered stream, before any UI state is changed.
use dbunk_lib::backend::QueryEventEnvelope;

pub struct Stream {
    session: String,
    connection: String,
    tab: String,
    generation: Option<u64>,
    sequence: u64,
    retired: bool,
}
impl Stream {
    pub fn new(session: String, connection: String) -> Self {
        Self::for_document(session, connection, "query".into())
    }
    pub fn for_document(session: String, connection: String, tab: String) -> Self {
        Self {
            session,
            connection,
            tab,
            generation: None,
            sequence: 0,
            retired: false,
        }
    }
    pub fn retire(&mut self) {
        self.retired = true;
    }
    pub fn admit(
        &mut self,
        event: &QueryEventEnvelope,
        execution: Option<&str>,
    ) -> Result<bool, &'static str> {
        if self.retired
            || event.session_id != self.session
            || event.connection_id != self.connection
            || event.tab_id != self.tab
        {
            return Ok(false);
        }
        if self
            .generation
            .is_some_and(|generation| generation != event.generation)
        {
            return Ok(false);
        }
        if event.sequence <= self.sequence {
            return Ok(false);
        }
        if event.sequence != self.sequence + 1 {
            self.retired = true;
            return Err("Query event sequence gap");
        }
        self.generation = Some(event.generation);
        self.sequence = event.sequence;
        // Sequence is session-wide. A late old-execution event must not mutate
        // results or produce an ACK for the current execution.
        Ok(event
            .execution_id
            .as_deref()
            .is_none_or(|id| Some(id) == execution))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::QueryEvent;
    fn event(
        session: &str,
        generation: u64,
        sequence: u64,
        execution: Option<&str>,
    ) -> QueryEventEnvelope {
        QueryEventEnvelope {
            session_id: session.into(),
            tab_id: "query".into(),
            connection_id: "fixture".into(),
            generation,
            sequence,
            execution_id: execution.map(str::to_owned),
            requires_ack: false,
            event: QueryEvent::SessionClosed,
        }
    }
    #[test]
    fn stale_sessions_generations_and_executions_cannot_change_current_results() {
        let mut stream = Stream::new("new".into(), "fixture".into());
        assert!(
            !stream
                .admit(&event("old", 1, 50, Some("old")), Some("run"))
                .unwrap()
        );
        assert!(
            stream
                .admit(&event("new", 2, 1, None), Some("run"))
                .unwrap()
        );
        assert!(
            !stream
                .admit(&event("new", 1, 2, Some("run")), Some("run"))
                .unwrap()
        );
        assert!(
            !stream
                .admit(&event("new", 2, 2, Some("old")), Some("run"))
                .unwrap()
        );
        assert!(
            stream
                .admit(&event("new", 2, 3, Some("run")), Some("run"))
                .unwrap()
        );
        assert!(
            !stream
                .admit(&event("new", 2, 3, Some("run")), Some("run"))
                .unwrap()
        );
    }
    #[test]
    fn document_identity_is_checked_before_advancing_sequence() {
        let mut stream = Stream::for_document("s".into(), "fixture".into(), "document-a".into());
        let mut incoming = event("s", 1, 1, None);
        assert!(!stream.admit(&incoming, None).unwrap());
        incoming.tab_id = "document-b".into();
        assert!(!stream.admit(&incoming, None).unwrap());
        incoming.tab_id = "document-a".into();
        assert!(stream.admit(&incoming, None).unwrap());
        incoming.sequence = 2;
        incoming.execution_id = Some("run".into());
        assert!(stream.admit(&incoming, Some("run")).unwrap());
    }
    #[test]
    fn gap_and_retirement_are_sticky() {
        let mut stream = Stream::new("s".into(), "fixture".into());
        assert!(stream.admit(&event("s", 1, 2, None), None).is_err());
        assert!(!stream.admit(&event("s", 1, 1, None), None).unwrap());
        let mut stream = Stream::new("s".into(), "fixture".into());
        stream.retire();
        assert!(!stream.admit(&event("s", 1, 1, None), None).unwrap());
    }
}
