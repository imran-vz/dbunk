//! Bounded immutable administration captures and request identities. Inspection
//! keeps server NULLs, clipping and incomplete scope distinct from empty data.
use dbunk_lib::backend::admin::{
    AdminCapture, AdminControlAction, AdminControlError, AdminControlReview, AdminLock,
    AdminMetric, AdminRow, AdminSection, AdminSession, AdminSnapshot, MAX_ADMIN_BLOCKERS,
    MAX_ADMIN_ROWS, MAX_ADMIN_TEXT_BYTES,
};
use std::{cell::Cell, rc::Rc};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const SNAPSHOT_BYTES: usize = 1024 * 1024;
const RETAINED_BYTES: usize = 2 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Sessions,
    Locks,
    Pending,
}
impl Section {
    pub const ALL: [Self; 3] = [Self::Sessions, Self::Locks, Self::Pending];
    pub fn index(self) -> usize {
        match self {
            Self::Sessions => 0,
            Self::Locks => 1,
            Self::Pending => 2,
        }
    }
    pub fn headings(self) -> &'static str {
        match self {
            Self::Sessions => "PID · User · State · Wait · Query",
            Self::Locks => "PID · Relation · Mode · Granted · Blockers · Query",
            Self::Pending => "PID · User · State · Transaction age · Query",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Sessions => "Sessions",
            Self::Locks => "Locks",
            Self::Pending => "Pending transactions",
        }
    }
}

#[derive(Default)]
pub struct ReadState {
    next: u64,
    pending: Option<(u64, bool)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Current,
    Cancelled,
    Stale,
}
impl ReadState {
    pub fn begin(&mut self) -> Result<u64, &'static str> {
        if self.pending.is_some() {
            return Err("An administration read is already pending");
        }
        let id = self
            .next
            .checked_add(1)
            .ok_or("Administration request identity exhausted; reopen the tab")?;
        self.next = id;
        self.pending = Some((id, false));
        Ok(id)
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancelling(&self) -> bool {
        self.pending.is_some_and(|(_, cancelled)| cancelled)
    }
    pub fn cancel(&mut self) {
        if let Some((_, cancelled)) = &mut self.pending {
            *cancelled = true;
        }
    }
    pub fn settle(&mut self, id: u64) -> Reply {
        match self.pending {
            Some((current, cancelled)) if id == current => {
                self.pending = None;
                if cancelled {
                    Reply::Cancelled
                } else {
                    Reply::Current
                }
            }
            _ => Reply::Stale,
        }
    }
    pub fn disconnected(&mut self) {
        self.pending = None;
    }
}

enum CaptureData {
    Owned(AdminCapture),
    #[cfg(test)]
    Detached(AdminSnapshot),
}
impl std::ops::Deref for CaptureData {
    type Target = AdminSnapshot;
    fn deref(&self) -> &AdminSnapshot {
        match self {
            Self::Owned(capture) => capture.snapshot(),
            #[cfg(test)]
            Self::Detached(data) => data,
        }
    }
}
pub struct Snapshot {
    data: CaptureData,
    budget: Rc<Cell<usize>>,
}
impl Snapshot {
    #[cfg(test)]
    fn new(data: AdminSnapshot, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        Self::admit(CaptureData::Detached(data), budget)
    }
    pub fn from_capture(
        capture: AdminCapture,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if capture.retained_bytes() > RETAINED_BYTES {
            return Err("Administration capture exceeds its retained allowance");
        }
        Self::admit(CaptureData::Owned(capture), budget)
    }
    pub fn review(
        &self,
        section: Section,
        index: usize,
        action: AdminControlAction,
    ) -> Result<AdminControlReview, AdminControlError> {
        match &self.data {
            CaptureData::Owned(capture) => capture.review(
                AdminRow {
                    section: match section {
                        Section::Sessions => AdminSection::Sessions,
                        Section::Locks => AdminSection::Locks,
                        Section::Pending => AdminSection::Pending,
                    },
                    index,
                },
                action,
            ),
            #[cfg(test)]
            CaptureData::Detached(_) => Err(AdminControlError::InvalidTarget),
        }
    }
    fn admit(data: CaptureData, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if data.sessions.len() > MAX_ADMIN_ROWS
            || data.locks.len() > MAX_ADMIN_ROWS
            || data.pending_transactions.len() > MAX_ADMIN_ROWS
            || data
                .locks
                .iter()
                .any(|lock| lock.blocked_by.len() > MAX_ADMIN_BLOCKERS)
            || heap_bytes(&data) > SNAPSHOT_BYTES
            || !valid_text(&data)
            || crate::results::encoded_size(&*data) > SNAPSHOT_BYTES
        {
            return Err("Administration snapshot exceeds 1 MiB; previous capture retained");
        }
        if RETAINED_BYTES > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Administration needs 2 MiB of shared allowance; clear a capture or close another tool",
            );
        }
        budget.set(budget.get() + RETAINED_BYTES);
        Ok(Self { data, budget })
    }
    pub fn count(&self, section: Section) -> usize {
        match section {
            Section::Sessions => self.data.sessions.len(),
            Section::Locks => self.data.locks.len(),
            Section::Pending => self.data.pending_transactions.len(),
        }
    }
    pub fn interval(&self) -> String {
        format!(
            "Database {} · reader PID {} · collected {} to {}. Readings may change during collection",
            self.data.database,
            self.data.reader_pid,
            self.data.collected_start,
            self.data.collected_end
        )
    }
    pub fn metrics(&self) -> String {
        let stats = &self.data.stats;
        format!(
            "Active sessions: {} · Idle in transaction: {} · Blocked locks (cluster): {} · Cache hit: {} · Database size: {}",
            metric(&stats.active_sessions, |v| v.to_string()),
            metric(&stats.idle_in_transaction, |v| v.to_string()),
            metric(&stats.blocked_locks, |v| v.to_string()),
            metric(&stats.cache_hit_ratio, |v| if v.is_finite() {
                format!("{:.1}%", v * 100.0)
            } else {
                "unavailable (invalid value)".into()
            }),
            metric(&stats.database_size_bytes, |v| format!("{v} bytes"))
        )
    }
    pub fn limits(&self) -> String {
        let mut parts = vec![self.data.scope_note.as_str()];
        if self.data.activity_restricted {
            parts.push("Activity visibility is restricted; session, lock and pending-transaction listings may be incomplete");
        }
        if self.data.sessions_truncated {
            parts.push("Sessions truncated at 200 rows");
        }
        if self.data.locks_truncated {
            parts.push("Locks truncated at 200 rows");
        }
        if self.data.pending_transactions_truncated {
            parts.push("Pending transactions truncated at 200 rows");
        }
        if self.data.locks.iter().any(|lock| lock.blocked_by_clipped) {
            parts.push("Some blocker lists are clipped to 64 PIDs; missing blockers are unknown");
        }
        parts.join(". ")
    }
    pub fn empty_label(&self) -> &'static str {
        if self.data.activity_restricted {
            "No visible rows in this section; activity visibility is restricted"
        } else {
            "No rows in this captured section"
        }
    }
    pub fn row_label(&self, section: Section, index: usize) -> Option<String> {
        match section {
            Section::Sessions | Section::Pending => {
                let row = if section == Section::Sessions {
                    self.data.sessions.get(index)?
                } else {
                    self.data.pending_transactions.get(index)?
                };
                Some(row_summary(&format!(
                    "{}{} · {} · {} · {} · {}",
                    flags(row.details_restricted, row.query_clipped),
                    row.pid,
                    text(&row.user),
                    text(&row.state),
                    if section == Section::Pending {
                        age(row.transaction_age_seconds)
                    } else {
                        format!("{} / {}", text(&row.wait_event_type), text(&row.wait_event))
                    },
                    summary(row.query.as_deref().unwrap_or("NULL"))
                )))
            }
            Section::Locks => {
                let row = self.data.locks.get(index)?;
                Some(row_summary(&format!(
                    "{}{} · {} · {} · {} · {} · {}",
                    flags(row.details_restricted, row.query_clipped),
                    pid(row.pid),
                    text(&row.relation),
                    row.mode,
                    if row.granted { "granted" } else { "waiting" },
                    blockers(row),
                    summary(row.query.as_deref().unwrap_or("NULL"))
                )))
            }
        }
    }
    pub fn details(&self, section: Section, index: usize) -> Option<String> {
        let details = match section {
            Section::Sessions => session_details(self.data.sessions.get(index)?),
            Section::Pending => session_details(self.data.pending_transactions.get(index)?),
            Section::Locks => {
                let row = self.data.locks.get(index)?;
                format!(
                    "PID: {}\nLock type: {}\nRelation: {}\nMode: {}\nGranted: {}\nBlocking PIDs: {}\nBackend start: {}\nQuery start: {}\nDetails restricted: {}\nQuery{}:\n{}",
                    pid(row.pid),
                    row.lock_type,
                    text(&row.relation),
                    row.mode,
                    row.granted,
                    blockers(row),
                    text(&row.backend_start),
                    text(&row.query_start),
                    row.details_restricted,
                    if row.query_clipped {
                        " (clipped to 500 characters by the server)"
                    } else {
                        ""
                    },
                    text(&row.query)
                )
            }
        };
        let result = format!("{}\n{details}", self.interval());
        Some(if result.len() > 64 * 1024 {
            "Selected details exceed the 64 KiB inspection limit; capture retained".into()
        } else {
            result
        })
    }
}
fn session_strings(row: &AdminSession) -> [&Option<String>; 11] {
    [
        &row.user,
        &row.database,
        &row.application_name,
        &row.client_addr,
        &row.state,
        &row.wait_event_type,
        &row.wait_event,
        &row.query,
        &row.backend_start,
        &row.query_start,
        &row.xact_start,
    ]
}
fn lock_strings(row: &AdminLock) -> [&Option<String>; 4] {
    [
        &row.relation,
        &row.query,
        &row.backend_start,
        &row.query_start,
    ]
}
fn heap_bytes(data: &AdminSnapshot) -> usize {
    let sessions = |rows: &Vec<AdminSession>| {
        rows.capacity()
            .saturating_mul(std::mem::size_of::<AdminSession>())
            .saturating_add(
                rows.iter()
                    .map(|row| {
                        session_strings(row)
                            .iter()
                            .map(|value| value.as_ref().map_or(0, String::capacity))
                            .sum::<usize>()
                    })
                    .sum::<usize>(),
            )
    };
    sessions(&data.sessions)
        .saturating_add(sessions(&data.pending_transactions))
        .saturating_add(
            data.locks
                .capacity()
                .saturating_mul(std::mem::size_of::<AdminLock>()),
        )
        .saturating_add(
            data.locks
                .iter()
                .map(|row| {
                    row.lock_type.capacity()
                        + row.mode.capacity()
                        + row.blocked_by.capacity() * std::mem::size_of::<i32>()
                        + lock_strings(row)
                            .iter()
                            .map(|value| value.as_ref().map_or(0, String::capacity))
                            .sum::<usize>()
                })
                .sum::<usize>(),
        )
        .saturating_add(data.database.capacity())
        .saturating_add(data.collected_start.capacity())
        .saturating_add(data.collected_end.capacity())
        .saturating_add(data.scope_note.capacity())
}
fn valid_text(data: &AdminSnapshot) -> bool {
    [
        &data.database,
        &data.collected_start,
        &data.collected_end,
        &data.scope_note,
    ]
    .iter()
    .all(|text| text.len() <= MAX_ADMIN_TEXT_BYTES)
        && data
            .sessions
            .iter()
            .chain(&data.pending_transactions)
            .all(|row| {
                session_strings(row).iter().all(|value| {
                    value
                        .as_ref()
                        .is_none_or(|value| value.len() <= MAX_ADMIN_TEXT_BYTES)
                })
            })
        && data.locks.iter().all(|row| {
            row.lock_type.len() <= MAX_ADMIN_TEXT_BYTES
                && row.mode.len() <= MAX_ADMIN_TEXT_BYTES
                && lock_strings(row).iter().all(|value| {
                    value
                        .as_ref()
                        .is_none_or(|value| value.len() <= MAX_ADMIN_TEXT_BYTES)
                })
        })
}
fn flags(restricted: bool, clipped: bool) -> &'static str {
    match (restricted, clipped) {
        (true, true) => "[restricted] [query clipped] ",
        (true, false) => "[restricted] ",
        (false, true) => "[query clipped] ",
        (false, false) => "",
    }
}
fn row_summary(value: &str) -> String {
    let mut chars = value.chars();
    let mut result: String = chars
        .by_ref()
        .take(256)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
fn metric<T>(value: &AdminMetric<T>, format: impl FnOnce(&T) -> String) -> String {
    match value {
        AdminMetric::Value(value) => format(value),
        AdminMetric::Null => "NULL".into(),
        AdminMetric::Restricted => "restricted".into(),
        AdminMetric::Unavailable => "unavailable".into(),
    }
}
fn text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("NULL")
}
fn pid(value: Option<i32>) -> String {
    value.map_or_else(|| "NULL (no process)".into(), |value| value.to_string())
}
fn age(value: Option<i64>) -> String {
    value.map_or_else(|| "NULL".into(), |value| format!("{value}s"))
}
fn summary(value: &str) -> String {
    let mut chars = value.chars();
    let mut result: String = chars
        .by_ref()
        .take(160)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
fn blockers(row: &AdminLock) -> String {
    if row.blocked_by_unavailable {
        return "unavailable; blocker identity was not returned".into();
    }
    let mut result = if row.blocked_by.is_empty() {
        "none reported".into()
    } else {
        row.blocked_by
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if row.blocked_by_clipped {
        result.push_str(" (clipped; additional blockers unknown)");
    }
    result
}
fn session_details(row: &AdminSession) -> String {
    format!(
        "PID: {}\nUser: {}\nDatabase: {}\nApplication: {}\nClient: {}\nState: {}\nWait: {} / {}\nQuery age: {}\nTransaction age: {}\nBackend start: {}\nQuery start: {}\nTransaction start: {}\nDetails restricted: {}\nQuery{}:\n{}",
        row.pid,
        text(&row.user),
        text(&row.database),
        text(&row.application_name),
        text(&row.client_addr),
        text(&row.state),
        text(&row.wait_event_type),
        text(&row.wait_event),
        age(row.query_age_seconds),
        age(row.transaction_age_seconds),
        text(&row.backend_start),
        text(&row.query_start),
        text(&row.xact_start),
        row.details_restricted,
        if row.query_clipped {
            " (clipped to 500 characters by the server)"
        } else {
            ""
        },
        text(&row.query)
    )
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(RETAINED_BYTES));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> AdminSnapshot {
        use dbunk_lib::backend::admin::AdminStats;
        AdminSnapshot {
            database: "fixture".into(),
            reader_pid: 99,
            activity_restricted: false,
            sessions: vec![],
            locks: vec![],
            pending_transactions: vec![],
            stats: AdminStats {
                database_size_bytes: AdminMetric::Value(0),
                cache_hit_ratio: AdminMetric::Null,
                active_sessions: AdminMetric::Value(0),
                idle_in_transaction: AdminMetric::Unavailable,
                blocked_locks: AdminMetric::Restricted,
            },
            collected_start: "2026-10-03T00:00:00Z".into(),
            collected_end: "2026-10-03T00:00:01Z".into(),
            scope_note: "Current database and NULL database sessions".into(),
            sessions_truncated: false,
            locks_truncated: false,
            pending_transactions_truncated: false,
        }
    }
    fn session() -> AdminSession {
        AdminSession {
            pid: 7,
            user: Some("reader".into()),
            database: None,
            application_name: Some(String::new()),
            client_addr: None,
            state: Some("active".into()),
            wait_event_type: None,
            wait_event: None,
            query_age_seconds: Some(0),
            transaction_age_seconds: None,
            query: Some("SELECT '雪'\nFROM t".into()),
            query_clipped: true,
            details_restricted: false,
            backend_start: None,
            query_start: None,
            xact_start: None,
        }
    }
    #[test]
    fn metric_absence_and_query_clipping_never_become_zero_or_complete_text() {
        let budget = Rc::new(Cell::new(0));
        let mut data = data();
        data.sessions.push(session());
        let snapshot = Snapshot::new(data, budget).unwrap();
        let metrics = snapshot.metrics();
        assert!(metrics.contains("Active sessions: 0"));
        assert!(metrics.contains("Idle in transaction: unavailable"));
        assert!(metrics.contains("Blocked locks (cluster): restricted"));
        assert!(metrics.contains("Cache hit: NULL"));
        let details = snapshot.details(Section::Sessions, 0).unwrap();
        assert!(
            details
                .contains("Query (clipped to 500 characters by the server):\nSELECT '雪'\nFROM t")
        );
        assert!(details.contains("Database: NULL"));
        assert!(
            snapshot
                .row_label(Section::Sessions, 0)
                .unwrap()
                .contains("[query clipped]")
        );
    }
    #[test]
    fn empty_restricted_listing_is_not_presented_as_an_unrestricted_empty_result() {
        let mut data = data();
        data.activity_restricted = true;
        let snapshot = Snapshot::new(data, Rc::new(Cell::new(0))).unwrap();
        assert!(snapshot.empty_label().contains("visibility is restricted"));
        assert!(snapshot.limits().contains("listings may be incomplete"));
    }
    #[test]
    fn candidate_refusal_preserves_old_lease_and_heap_capacity_is_counted() {
        let budget = Rc::new(Cell::new(WORKSPACE_BYTES - RETAINED_BYTES));
        let first = Snapshot::new(data(), budget.clone()).unwrap();
        assert_eq!(budget.get(), WORKSPACE_BYTES);
        assert!(Snapshot::new(data(), budget.clone()).is_err());
        assert_eq!(first.count(Section::Sessions), 0);
        drop(first);
        assert_eq!(budget.get(), WORKSPACE_BYTES - RETAINED_BYTES);
        let mut oversized = data();
        oversized.scope_note = String::with_capacity(SNAPSHOT_BYTES + 1);
        let before = budget.get();
        assert!(Snapshot::new(oversized, budget.clone()).is_err());
        assert_eq!(budget.get(), before);
        let mut too_many = data();
        too_many.sessions = vec![session(); MAX_ADMIN_ROWS + 1];
        assert!(Snapshot::new(too_many, budget).is_err());
    }
    #[test]
    fn missing_lock_pid_and_blocker_metadata_are_not_reported_as_empty() {
        let mut data = data();
        data.locks.push(AdminLock {
            pid: None,
            lock_type: "transactionid".into(),
            relation: None,
            mode: "ExclusiveLock".into(),
            granted: true,
            blocked_by: vec![],
            blocked_by_clipped: false,
            blocked_by_unavailable: true,
            query: None,
            query_clipped: false,
            details_restricted: true,
            backend_start: None,
            query_start: None,
        });
        let snapshot = Snapshot::new(data, Rc::new(Cell::new(0))).unwrap();
        let details = snapshot.details(Section::Locks, 0).unwrap();
        assert!(details.contains("PID: NULL (no process)"));
        assert!(details.contains("Blocking PIDs: unavailable"));
        assert!(!details.contains("none reported"));
    }

    #[test]
    fn cancellation_discards_late_success_without_settling_a_new_request() {
        let mut state = ReadState::default();
        let first = state.begin().unwrap();
        assert!(state.begin().is_err());
        state.cancel();
        assert_eq!(state.settle(first + 1), Reply::Stale);
        assert!(state.busy());
        assert_eq!(state.settle(first), Reply::Cancelled);
        let second = state.begin().unwrap();
        assert_eq!(state.settle(first), Reply::Stale);
        assert!(state.busy());
        assert_eq!(state.settle(second), Reply::Current);
        assert!(!state.busy());
    }
    #[test]
    fn reconnect_keeps_monotonic_identity_and_never_accepts_old_reply() {
        let mut state = ReadState::default();
        let old = state.begin().unwrap();
        state.disconnected();
        let current = state.begin().unwrap();
        assert!(current > old);
        assert_eq!(state.settle(old), Reply::Stale);
        assert_eq!(state.settle(current), Reply::Current);
    }
}
