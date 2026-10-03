//! Bounded native document leases. A retired handle never becomes valid again,
//! even if a new document reuses its visible tab ID. Admission is owned by the
//! backend task, so dropping a UI waiter cannot release in-flight work early.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{watch, OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

const DOCUMENT_LIMIT: usize = 16;

#[derive(Clone)]
pub struct DataDocument(pub(super) Arc<Lease>);

pub(super) struct Lease {
    id: Uuid,
    owner: Arc<()>,
    pub(super) window: String,
    pub(super) tab: String,
    pub(super) manager_tab: String,
    pub(super) connection: String,
    closed: AtomicBool,
    reads_cancelled: watch::Sender<u64>,
    admission: Arc<RwLock<()>>,
    write: Mutex<WriteState>,
}

#[derive(Default)]
pub(super) struct Documents {
    owner: Arc<()>,
    leases: Mutex<HashMap<Uuid, Arc<Lease>>>,
}

impl Documents {
    pub(super) fn register(
        &self,
        window: String,
        tab: String,
        connection: String,
    ) -> Result<DataDocument, &'static str> {
        if [&window, &tab, &connection]
            .iter()
            .any(|value| value.is_empty() || value.len() > 256)
        {
            return Err("Document identities must contain 1 to 256 bytes");
        }
        let mut leases = self.leases.lock().unwrap();
        if leases.len() >= DOCUMENT_LIMIT {
            return Err("Sixteen data documents are open. Close one before opening another.");
        }
        if leases
            .values()
            .any(|lease| lease.window == window && lease.tab == tab)
        {
            return Err("This document is already registered or still closing");
        }
        let lease = Arc::new(Lease {
            id: Uuid::new_v4(),
            owner: self.owner.clone(),
            window,
            tab,
            manager_tab: Uuid::new_v4().to_string(),
            connection,
            closed: AtomicBool::new(false),
            reads_cancelled: watch::channel(0).0,
            admission: Arc::new(RwLock::new(())),
            write: Mutex::new(WriteState::default()),
        });
        leases.insert(lease.id, lease.clone());
        Ok(DataDocument(lease))
    }

    pub(super) fn owns(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.owner, &document.0.owner)
    }

    /// Refuse overlapping schema and maintenance writes on the same connection. The registry
    /// has at most sixteen leases; no second connection map or waiter is needed.
    /// Lock order is registry then lease. A write permit never takes the registry.
    pub(super) fn begin_write(&self, document: &DataDocument) -> Result<WritePermit, &'static str> {
        if !self.owns(document) {
            return Err("Document belongs to a different workspace");
        }
        let leases = self.leases.lock().unwrap();
        if !leases.contains_key(&document.0.id) {
            return Err("Document is closed");
        }
        for lease in leases.values() {
            if lease.connection == document.0.connection
                && lease.write.lock().unwrap().active.is_some()
            {
                return Err("A schema or maintenance write is already active on this connection");
            }
        }
        let mut write = document.0.write.lock().unwrap();
        document.0.check_open()?;
        let id = Uuid::new_v4();
        write.active = Some(ActiveWrite {
            id,
            phase: WritePhase::Preparing,
            interrupted: false,
        });
        Ok(WritePermit {
            lease: document.0.clone(),
            id,
        })
    }

    /// One control per document, independent of schema-write exclusivity: a
    /// session signal must remain usable while another document is writing.
    pub(super) fn begin_control(
        &self,
        document: &DataDocument,
    ) -> Result<ControlPermit, &'static str> {
        if !self.owns(document) {
            return Err("Document belongs to a different workspace");
        }
        let leases = self.leases.lock().unwrap();
        if !leases.contains_key(&document.0.id) {
            return Err("Document is closed");
        }
        let mut state = document.0.write.lock().unwrap();
        document.0.check_open()?;
        if state.control.is_some() {
            return Err("An administration control is still settling in this document");
        }
        let id = Uuid::new_v4();
        state.control = Some(ActiveWrite {
            id,
            phase: WritePhase::Preparing,
            interrupted: false,
        });
        Ok(ControlPermit {
            lease: document.0.clone(),
            id,
        })
    }

    pub(super) async fn enter(
        &self,
        document: &DataDocument,
    ) -> Result<OwnedRwLockReadGuard<()>, &'static str> {
        if !self.owns(document) {
            return Err("Document belongs to a different workspace");
        }
        document.0.check_open()?;
        let permit = document.0.admission.clone().read_owned().await;
        document.0.check_open()?;
        Ok(permit)
    }

    pub(super) fn retire(&self, document: &DataDocument) -> Result<(), &'static str> {
        if !self.owns(document) {
            return Err("Document belongs to a different workspace");
        }
        document.0.retire();
        Ok(())
    }

    /// Fence a connection before its service invalidation; active tasks retain
    /// their read permits until the manager's cancellation/cleanup settles.
    pub(super) fn retire_matching(&self, connection: Option<&str>) {
        for lease in self.leases.lock().unwrap().values() {
            if connection.is_none_or(|id| lease.connection == id) {
                lease.retire();
            }
        }
    }

    pub(super) async fn finish_retired(&self, connection: Option<&str>) {
        let retired = self
            .leases
            .lock()
            .unwrap()
            .values()
            .filter(|lease| {
                lease.closed.load(Ordering::Acquire)
                    && connection.is_none_or(|id| lease.connection == id)
            })
            .cloned()
            .map(DataDocument)
            .collect::<Vec<_>>();
        for document in retired {
            let _ = self.finish(&document).await;
        }
    }

    /// Call only after the managers have been told to cancel this tab. Keeping
    /// the registry entry until every permit returns prevents ID reuse races.
    pub(super) async fn finish(&self, document: &DataDocument) -> Result<(), &'static str> {
        self.retire(document)?;
        let _joined = document.0.admission.write().await;
        self.leases.lock().unwrap().remove(&document.0.id);
        Ok(())
    }
}

impl Lease {
    /// Epochs cancel current reads without poisoning subsequent requests. A retired
    /// document additionally fails check_open, including after admission waits.
    pub(super) fn cancel_reads(&self) {
        self.interrupt(false);
    }

    fn retire(&self) {
        self.interrupt(true);
    }

    // The same short lock decides cancel/retire versus COMMIT or signal admission.
    // Watch only wakes the owner; its independently observed epoch is not authority.
    fn interrupt(&self, retire: bool) {
        let mut write = self.write.lock().unwrap();
        if retire {
            self.closed.store(true, Ordering::Release);
        }
        if let Some(active) = &mut write.active {
            if active.phase == WritePhase::Preparing {
                active.phase = WritePhase::Interrupted;
            }
            active.interrupted = true;
        }
        if let Some(active) = &mut write.control {
            if active.phase == WritePhase::Preparing {
                active.phase = WritePhase::Interrupted;
            }
        }
        self.reads_cancelled
            .send_modify(|epoch| *epoch = epoch.wrapping_add(1));
    }

    pub(super) fn read_cancellation(&self) -> watch::Receiver<u64> {
        self.reads_cancelled.subscribe()
    }

    /// Recheck after waiting for startup admission: close may have retired a
    /// document while an already admitted operation waited for that gate.
    pub(super) fn check_open(&self) -> Result<(), &'static str> {
        if self.closed.load(Ordering::Acquire) {
            Err("Document is closed")
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct WriteState {
    active: Option<ActiveWrite>,
    control: Option<ActiveWrite>,
}
struct ActiveWrite {
    id: Uuid,
    phase: WritePhase,
    /// Any cancel/retire since admission. Only grouped DDL consults it, to
    /// refuse returning to Preparing after an already admitted COMMIT.
    interrupted: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum WritePhase {
    Preparing,
    Interrupted,
    CommitAdmitted,
}

/// Held through signal acknowledgement and joined socket cleanup. Its dispatch
/// fence shares the lease lock with cancellation and retirement, not watch epochs.
pub(crate) struct ControlPermit {
    lease: Arc<Lease>,
    id: Uuid,
}
impl ControlPermit {
    pub(crate) fn check_preparing(&self) -> bool {
        self.lease
            .write
            .lock()
            .unwrap()
            .control
            .as_ref()
            .is_some_and(|active| active.id == self.id && active.phase == WritePhase::Preparing)
    }
    pub(crate) fn admit_dispatch(&self) -> bool {
        let mut state = self.lease.write.lock().unwrap();
        match state.control.as_mut() {
            Some(active) if active.id == self.id && active.phase == WritePhase::Preparing => {
                active.phase = WritePhase::CommitAdmitted;
                true
            }
            _ => false,
        }
    }
    #[cfg(test)]
    pub(crate) fn test_permit() -> Self {
        let documents = Documents::default();
        let document = documents
            .register("test".into(), "control".into(), "connection".into())
            .unwrap();
        documents.begin_control(&document).unwrap()
    }
    #[cfg(test)]
    pub(crate) fn test_cancel(&self) {
        self.lease.cancel_reads();
    }
    #[cfg(test)]
    pub(crate) fn test_retire(&self) {
        self.lease.retire();
    }
    #[cfg(test)]
    pub(crate) fn test_cancellation(&self) -> watch::Receiver<u64> {
        self.lease.read_cancellation()
    }
}
impl Drop for ControlPermit {
    fn drop(&mut self) {
        let mut state = self.lease.write.lock().unwrap();
        if state
            .control
            .as_ref()
            .is_some_and(|active| active.id == self.id)
        {
            state.control = None;
        }
    }
}

/// Held by the admitted backend task through socket cleanup, not by its waiter.
pub(crate) struct WritePermit {
    lease: Arc<Lease>,
    id: Uuid,
}
impl WritePermit {
    #[cfg(test)]
    pub(crate) fn test_permit() -> Self {
        let documents = Documents::default();
        let document = documents
            .register("test".into(), "schema".into(), "connection".into())
            .unwrap();
        documents.begin_write(&document).unwrap()
    }
    #[cfg(test)]
    pub(crate) fn test_cancel(&self) {
        self.lease.cancel_reads();
    }
    #[cfg(test)]
    pub(crate) fn test_retire(&self) {
        self.lease.retire();
    }
    #[cfg(test)]
    pub(crate) fn test_cancellation(&self) -> watch::Receiver<u64> {
        self.lease.read_cancellation()
    }
    pub(crate) fn check_preparing(&self) -> bool {
        let state = self.lease.write.lock().unwrap();
        state
            .active
            .as_ref()
            .is_some_and(|active| active.id == self.id && active.phase == WritePhase::Preparing)
    }
    /// Standalone maintenance has its effect boundary at dispatch, not COMMIT.
    /// It uses the same atomic cancellation/retirement fence and write slot.
    pub(crate) fn admit_dispatch(&self) -> bool {
        self.admit_commit()
    }
    pub(crate) fn admit_commit(&self) -> bool {
        let mut state = self.lease.write.lock().unwrap();
        match state.active.as_mut() {
            Some(active) if active.id == self.id && active.phase == WritePhase::Preparing => {
                active.phase = WritePhase::CommitAdmitted;
                true
            }
            _ => false,
        }
    }
    /// Grouped DDL: after one admitted group settles, return to Preparing for
    /// the next group only when no cancel/retire arrived since admission. The
    /// same lease lock orders this against cancellation, so a cancel during an
    /// earlier COMMIT can never be lost by a later group's dispatch.
    pub(crate) fn readmit(&self) -> bool {
        let mut state = self.lease.write.lock().unwrap();
        match state.active.as_mut() {
            Some(active)
                if active.id == self.id
                    && active.phase == WritePhase::CommitAdmitted
                    && !active.interrupted =>
            {
                active.phase = WritePhase::Preparing;
                true
            }
            _ => false,
        }
    }
}
impl Drop for WritePermit {
    fn drop(&mut self) {
        let mut state = self.lease.write.lock().unwrap();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.id == self.id)
        {
            state.active = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn control_slot_is_independent_bounded_and_retained_until_owner_drop() {
        let documents = Documents::default();
        let document = documents
            .register("w".into(), "a".into(), "connection".into())
            .unwrap();
        let schema = documents.begin_write(&document).unwrap();
        let first = documents.begin_control(&document).unwrap();
        assert!(documents.begin_control(&document).is_err());
        document.0.cancel_reads();
        assert!(!first.admit_dispatch());
        assert!(!schema.admit_commit());
        assert!(documents.begin_control(&document).is_err());
        drop(first);
        let next = documents.begin_control(&document).unwrap();
        assert!(next.admit_dispatch());
        documents.retire_matching(Some("connection"));
        assert!(!next.check_preparing());
        assert!(matches!(
            document
                .0
                .write
                .lock()
                .unwrap()
                .control
                .as_ref()
                .unwrap()
                .phase,
            WritePhase::CommitAdmitted
        ));
        drop(next);
        assert!(documents.begin_control(&document).is_err());
    }

    #[tokio::test]
    async fn close_waits_for_owned_work_and_never_revives_a_retired_handle() {
        let documents = Arc::new(Documents::default());
        let first = documents
            .register("window".into(), "one".into(), "connection".into())
            .unwrap();
        let other = documents
            .register("window".into(), "two".into(), "connection".into())
            .unwrap();
        let permit = documents.enter(&first).await.unwrap();
        documents.retire(&first).unwrap();
        assert!(documents.enter(&first).await.is_err());
        assert!(documents.enter(&other).await.is_ok());
        let closing = documents.clone();
        let closed = first.clone();
        let mut joined = tokio::spawn(async move { closing.finish(&closed).await });
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut joined)
            .await
            .is_err());
        assert!(documents
            .register("window".into(), "one".into(), "connection".into())
            .is_err());
        drop(permit);
        joined.await.unwrap().unwrap();
        let replacement = documents
            .register("window".into(), "one".into(), "connection".into())
            .unwrap();
        assert!(documents.enter(&replacement).await.is_ok());
        assert!(documents.enter(&first).await.is_err());
        assert!(Documents::default().enter(&replacement).await.is_err());
        documents.retire_matching(Some("connection"));
        assert!(documents.enter(&other).await.is_err());
        assert!(documents.enter(&replacement).await.is_err());
    }

    #[tokio::test]
    async fn abandoned_close_waiter_keeps_admission_reserved_until_joined() {
        let documents = Arc::new(Documents::default());
        let mut handles = Vec::new();
        for index in 0..DOCUMENT_LIMIT {
            handles.push(
                documents
                    .register("window".into(), index.to_string(), "connection".into())
                    .unwrap(),
            );
        }
        let active = handles.remove(0);
        let permit = documents.enter(&active).await.unwrap();
        documents.retire(&active).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), documents.finish(&active))
                .await
                .is_err()
        );
        assert!(documents
            .register("window".into(), "extra".into(), "connection".into())
            .is_err());
        drop(permit);
        documents.finish(&active).await.unwrap();
        assert!(documents
            .register("window".into(), "extra".into(), "connection".into())
            .is_ok());
    }
    #[test]
    fn schema_write_admission_is_connection_scoped_bounded_and_released() {
        let documents = Documents::default();
        let a = documents
            .register("w".into(), "a".into(), "first".into())
            .unwrap();
        let b = documents
            .register("w".into(), "b".into(), "first".into())
            .unwrap();
        let c = documents
            .register("w".into(), "c".into(), "second".into())
            .unwrap();
        let first = documents.begin_write(&a).unwrap();
        assert!(documents.begin_write(&b).is_err());
        let other = documents.begin_write(&c).unwrap();
        a.0.cancel_reads();
        assert!(!first.admit_commit());
        assert!(documents.begin_write(&b).is_err()); // cleanup still owns it
        drop(first);
        let next = documents.begin_write(&b).unwrap();
        assert!(next.admit_commit());
        b.0.cancel_reads();
        documents.retire(&b).unwrap();
        assert!(!next.check_preparing());
        assert!(documents.begin_write(&a).is_err());
        drop(next);
        assert!(documents.begin_write(&a).is_ok());
        assert!(documents.begin_write(&b).is_err());
        assert!(other.admit_commit());
    }

    #[test]
    fn retirement_and_commit_admission_share_a_single_fence() {
        let documents = Documents::default();
        let before = documents
            .register("w".into(), "before".into(), "a".into())
            .unwrap();
        let permit = documents.begin_write(&before).unwrap();
        documents.retire_matching(Some("a"));
        assert!(!permit.admit_commit());
        drop(permit);
        let after = documents
            .register("w".into(), "after".into(), "b".into())
            .unwrap();
        let permit = documents.begin_write(&after).unwrap();
        assert!(permit.admit_commit());
        documents.retire_matching(Some("b"));
        let state = after.0.write.lock().unwrap();
        assert!(matches!(
            state.active.as_ref().unwrap().phase,
            WritePhase::CommitAdmitted
        ));
    }
}
