use super::{control_types::document_bytes, *};
use std::mem::size_of;

/// An immutable observation bound to its original document lease. DTO-only
/// callers retain admin_snapshot; only this backend-minted capture can review a
/// signal. Retirement is checked here and again during backend submission.
pub struct AdminCapture {
    document: DataDocument,
    snapshot: AdminSnapshot,
    bytes: usize,
}
impl AdminCapture {
    pub(super) fn new(document: DataDocument, snapshot: AdminSnapshot) -> Result<Self, DataError> {
        let heap = snapshot_bytes(&snapshot);
        if heap > MAX_ADMIN_BYTES
            || super::super::schema_ddl::encoded_bytes(&snapshot) > MAX_ADMIN_BYTES
        {
            return Err(DataError::Catalog(
                super::super::objects::CatalogError::AdminLimit,
            ));
        }
        let bytes = size_of::<Self>() + heap + document_bytes(&document);
        Ok(Self {
            document,
            snapshot,
            bytes,
        })
    }
    pub fn snapshot(&self) -> &AdminSnapshot {
        &self.snapshot
    }
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        std::sync::Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn review(
        &self,
        row: AdminRow,
        action: AdminControlAction,
    ) -> Result<AdminControlReview, AdminControlError> {
        self.document
            .0
            .check_open()
            .map_err(|_| AdminControlError::Unavailable)?;
        let session = match row.section {
            AdminSection::Sessions => self.snapshot.sessions.get(row.index),
            AdminSection::Pending => self.snapshot.pending_transactions.get(row.index),
            AdminSection::Locks => {
                let lock = self
                    .snapshot
                    .locks
                    .get(row.index)
                    .ok_or(AdminControlError::InvalidTarget)?;
                // A lock has no database identity. Resolve its exact observed
                // holder through this same capture, never guess from a blocker PID.
                self.snapshot
                    .sessions
                    .iter()
                    .chain(&self.snapshot.pending_transactions)
                    .find(|session| {
                        Some(session.pid) == lock.pid
                            && session.backend_start.is_some()
                            && session.backend_start == lock.backend_start
                            && session.query_start == lock.query_start
                    })
            }
        }
        .ok_or(AdminControlError::InvalidTarget)?;
        let start = session
            .backend_start
            .as_deref()
            .ok_or(AdminControlError::InvalidTarget)?;
        if session.pid <= 0
            || session.pid == self.snapshot.reader_pid
            || !timestamp(start)
            || session
                .query_start
                .as_deref()
                .is_some_and(|value| !timestamp(value))
            || session
                .database
                .as_ref()
                .is_some_and(|value| value.len() > MAX_ADMIN_TEXT_BYTES)
        {
            return Err(AdminControlError::InvalidTarget);
        }
        let review = AdminControlReview {
            document: self.document.clone(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
            action,
            target: AdminControlTarget {
                pid: session.pid,
                backend_start: start.into(),
                query_start: session.query_start.clone(),
                database: session.database.clone(),
            },
        };
        if review.retained_bytes() > MAX_ADMIN_CONTROL_BYTES {
            return Err(AdminControlError::InvalidTarget);
        }
        Ok(review)
    }
}
fn timestamp(value: &str) -> bool {
    value.len() == 27 && value.ends_with('Z') && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}
fn strings(values: &[Option<&String>]) -> usize {
    values
        .iter()
        .map(|value| value.map_or(0, String::capacity))
        .sum()
}
fn session_bytes(session: &AdminSession) -> usize {
    strings(&[
        session.user.as_ref(),
        session.database.as_ref(),
        session.application_name.as_ref(),
        session.client_addr.as_ref(),
        session.state.as_ref(),
        session.wait_event_type.as_ref(),
        session.wait_event.as_ref(),
        session.query.as_ref(),
        session.backend_start.as_ref(),
        session.query_start.as_ref(),
        session.xact_start.as_ref(),
    ])
}
fn snapshot_bytes(snapshot: &AdminSnapshot) -> usize {
    size_of::<AdminSnapshot>()
        + snapshot.database.capacity()
        + snapshot.collected_start.capacity()
        + snapshot.collected_end.capacity()
        + snapshot.scope_note.capacity()
        + (snapshot.sessions.capacity() + snapshot.pending_transactions.capacity())
            * size_of::<AdminSession>()
        + snapshot.locks.capacity() * size_of::<AdminLock>()
        + snapshot
            .sessions
            .iter()
            .chain(&snapshot.pending_transactions)
            .map(session_bytes)
            .sum::<usize>()
        + snapshot
            .locks
            .iter()
            .map(|lock| {
                lock.lock_type.capacity()
                    + lock.mode.capacity()
                    + lock.blocked_by.capacity() * size_of::<i32>()
                    + strings(&[
                        lock.relation.as_ref(),
                        lock.query.as_ref(),
                        lock.backend_start.as_ref(),
                        lock.query_start.as_ref(),
                    ])
            })
            .sum::<usize>()
}
