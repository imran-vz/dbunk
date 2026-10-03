//! Non-blocking, byte- and count-bounded delivery with an independent failure wake.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::results::encoded_size;
use dbunk_lib::backend::{
    QueryEventEnvelope, QuerySessionError, QueryTransactionSnapshot, SinkClosed,
    StatementClassSummary,
};
use serde::Serialize;

pub const QUEUE_CAPACITY: usize = 64;
pub const QUEUE_BYTES: usize = 8 * 1024 * 1024;
pub const WORKSPACE_QUEUE_BYTES: usize = 16 * 1024 * 1024;

/// Review displays exactly the service-bound value; diagnostics never do.
#[derive(Serialize)]
pub struct ReviewParameter {
    pub name: String,
    pub value: Option<String>,
}
impl std::fmt::Debug for ReviewParameter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReviewParameter")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Shared encoded-byte admission. Permits own accounting until consumed data
/// is released, including queued messages dropped during teardown.
#[derive(Clone)]
pub struct ByteBudget(Arc<ByteBudgetState>);
struct ByteBudgetState {
    used: AtomicUsize,
    high_water: AtomicUsize,
    limit: usize,
}
pub struct BytePermit {
    budget: ByteBudget,
    bytes: usize,
}
impl ByteBudget {
    pub fn new(limit: usize) -> Self {
        Self(Arc::new(ByteBudgetState {
            used: AtomicUsize::new(0),
            high_water: AtomicUsize::new(0),
            limit,
        }))
    }
    pub fn reserve(&self, bytes: usize) -> Option<BytePermit> {
        let previous = self
            .0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .ok()?;
        self.0
            .high_water
            .fetch_max(previous + bytes, Ordering::Relaxed);
        Some(BytePermit {
            budget: self.clone(),
            bytes,
        })
    }
    pub fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }
    pub fn high_water(&self) -> usize {
        self.0.high_water.load(Ordering::Relaxed)
    }
}
impl BytePermit {
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
impl Drop for BytePermit {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Debug, Serialize)]
// The fixed 64-slot queue bounds enum storage. Boxing would allocate for each
// delivered event without reducing the much larger, separately bounded payload.
#[allow(clippy::large_enum_variant)]
pub enum Message {
    Event(QueryEventEnvelope),
    Ready,
    HistoryFailed(String),
    Review {
        execution: String,
        sql: String,
        statements: Vec<StatementClassSummary>,
        parameters: Option<Vec<ReviewParameter>>,
        row_limit: Option<i64>,
    },
    Transaction {
        session: String,
        result: Result<QueryTransactionSnapshot, QuerySessionError>,
    },
    Rejected {
        execution: String,
        message: String,
    },
    Acked {
        execution: String,
        sequence: u64,
    },
    CancelFailed {
        execution: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Full,
    Closed,
    Oversize,
    Backend(String),
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => f.write_str("Result delivery stopped: queue full"),
            Self::Closed => f.write_str("Result delivery stopped: consumer closed"),
            Self::Oversize => f.write_str("Result delivery stopped: event exceeds queue budget"),
            Self::Backend(message) => f.write_str(message),
        }
    }
}

#[derive(Clone)]
pub struct Sender {
    data: async_channel::Sender<Queued>,
    wake: async_channel::Sender<()>,
    shared: Arc<Shared>,
}
pub struct Receiver {
    data: async_channel::Receiver<Queued>,
    pub wake: async_channel::Receiver<()>,
    shared: Arc<Shared>,
}
struct Shared {
    bytes: AtomicUsize,
    high_water: AtomicUsize,
    budget: usize,
    workspace_budget: ByteBudget,
    failure_wake: tokio::sync::Notify,
    failed: Mutex<Option<Failure>>,
    closed: AtomicBool,
}
struct Queued {
    message: Option<Message>,
    bytes: usize,
    shared: Arc<Shared>,
    _workspace_permit: BytePermit,
}
impl Drop for Queued {
    fn drop(&mut self) {
        self.shared.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub fn channel(capacity: usize, budget: usize) -> (Sender, Receiver) {
    channel_with_budget(capacity, budget, ByteBudget::new(budget))
}

pub fn channel_with_budget(
    capacity: usize,
    budget: usize,
    workspace_budget: ByteBudget,
) -> (Sender, Receiver) {
    let (data, receive) = async_channel::bounded(capacity);
    let (wake, awakened) = async_channel::bounded(1);
    let shared = Arc::new(Shared {
        bytes: AtomicUsize::new(0),
        high_water: AtomicUsize::new(0),
        budget,
        workspace_budget,
        failure_wake: tokio::sync::Notify::new(),
        failed: Mutex::new(None),
        closed: AtomicBool::new(false),
    });
    (
        Sender {
            data,
            wake,
            shared: shared.clone(),
        },
        Receiver {
            data: receive,
            wake: awakened,
            shared,
        },
    )
}

impl Sender {
    pub(crate) fn shares_budget(&self, budget: &ByteBudget) -> bool {
        Arc::ptr_eq(&self.shared.workspace_budget.0, &budget.0)
    }
    pub(crate) fn reserve_history(&self, bytes: usize) -> Option<BytePermit> {
        self.shared.workspace_budget.reserve(bytes)
    }
    pub fn send(&self, message: Message) -> Result<(), SinkClosed> {
        let bytes = encoded_size(&message);
        // Linearize admission and first failure across event and command-reply
        // producers. No serialization, await or callback runs under this lock.
        let mut failed = self.shared.failed.lock().unwrap();
        if self.shared.closed.load(Ordering::Acquire) || failed.is_some() {
            self.shared.failure_wake.notify_one();
            return Err(SinkClosed);
        }
        if bytes > self.shared.budget {
            failed.get_or_insert(Failure::Oversize);
            let _ = self.wake.try_send(());
            self.shared.failure_wake.notify_one();
            return Err(SinkClosed);
        }
        let used = self
            .shared
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= self.shared.budget)
            });
        let Ok(used) = used else {
            failed.get_or_insert(Failure::Full);
            let _ = self.wake.try_send(());
            self.shared.failure_wake.notify_one();
            return Err(SinkClosed);
        };
        self.shared
            .high_water
            .fetch_max(used + bytes, Ordering::Relaxed);
        let Some(workspace_permit) = self.shared.workspace_budget.reserve(bytes) else {
            self.shared.bytes.fetch_sub(bytes, Ordering::AcqRel);
            failed.get_or_insert(Failure::Full);
            let _ = self.wake.try_send(());
            self.shared.failure_wake.notify_one();
            return Err(SinkClosed);
        };
        let queued = Queued {
            message: Some(message),
            bytes,
            shared: self.shared.clone(),
            _workspace_permit: workspace_permit,
        };
        #[cfg(feature = "fixture-verification")]
        crate::verification::offered(queued.message.as_ref().unwrap());
        if let Err(error) = self.data.try_send(queued) {
            failed.get_or_insert(if error.is_full() {
                Failure::Full
            } else {
                Failure::Closed
            });
            #[cfg(feature = "fixture-verification")]
            eprintln!("VERIFY queue-rejected {failed:?}");
            let _ = self.wake.try_send(());
            self.shared.failure_wake.notify_one();
            return Err(SinkClosed);
        }
        let _ = self.wake.try_send(());
        Ok(())
    }
    pub fn fail(&self, failure: Failure) {
        self.shared.failed.lock().unwrap().get_or_insert(failure);
        self.shared.failure_wake.notify_one();
        let _ = self.wake.try_send(());
    }
    pub async fn failed(&self) {
        loop {
            let changed = self.shared.failure_wake.notified();
            if self.is_failed() {
                return;
            }
            changed.await;
        }
    }
    pub fn is_failed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire) || self.shared.failed.lock().unwrap().is_some()
    }
}
impl Receiver {
    pub fn receive(&self) -> Option<Message> {
        self.data
            .try_recv()
            .ok()
            .and_then(|mut queued| queued.message.take())
    }
    pub fn failure(&self) -> Option<Failure> {
        self.shared.failed.lock().unwrap().clone()
    }
    pub fn pending(&self) -> bool {
        !self.data.is_empty()
    }
    pub fn high_water(&self) -> usize {
        self.shared.high_water.load(Ordering::Relaxed)
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        let mut failed = self.shared.failed.lock().unwrap();
        self.shared.closed.store(true, Ordering::Release);
        self.shared.failure_wake.notify_one();
        failed.get_or_insert(Failure::Closed);
        self.data.close();
        while self.data.try_recv().is_ok() {}
        self.wake.close();
        eprintln!(
            "Native queue released: high_water_bytes={} remaining_bytes={}",
            self.shared.high_water.load(Ordering::Relaxed),
            self.shared.bytes.load(Ordering::Acquire),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::{QueryEvent, QueryTransactionSnapshot};

    fn event(sequence: u64, event: QueryEvent) -> Message {
        Message::Event(QueryEventEnvelope {
            session_id: "session".into(),
            tab_id: "query".into(),
            connection_id: "fixture".into(),
            generation: 1,
            sequence,
            execution_id: Some("execution".into()),
            requires_ack: true,
            event,
        })
    }

    fn row(sequence: u64) -> Message {
        event(
            sequence,
            QueryEvent::RowBatch {
                result_set_index: 0,
                rows: vec![vec![Some(format!("row{sequence}"))]],
            },
        )
    }
    #[test]
    fn workspace_budget_spans_mailboxes_and_releases_on_consume_or_drop() {
        let bytes = encoded_size(&Message::Ready);
        let budget = ByteBudget::new(2 * bytes);
        let (a, first) = channel_with_budget(64, QUEUE_BYTES, budget.clone());
        let (b, second) = channel_with_budget(64, QUEUE_BYTES, budget.clone());
        let (c, third) = channel_with_budget(64, QUEUE_BYTES, budget.clone());
        a.send(Message::Ready).unwrap();
        b.send(Message::Ready).unwrap();
        assert!(c.send(Message::Ready).is_err());
        assert_eq!(third.failure(), Some(Failure::Full));
        assert!(first.failure().is_none());
        assert!(second.failure().is_none());
        assert_eq!(budget.used(), 2 * bytes);
        first.receive().unwrap();
        assert_eq!(budget.used(), bytes);
        drop(second);
        assert_eq!(budget.used(), 0);
        assert_eq!(budget.high_water(), 2 * bytes);
        assert_eq!(c.shared.bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn failure_wakes_controller_even_with_no_data_or_heartbeat() {
        let (sender, receiver) = channel(64, QUEUE_BYTES);
        let waiting = sender.clone();
        let waiter = tokio::spawn(async move { waiting.failed().await });
        sender.fail(Failure::Backend("fixture failure".into()));
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(!receiver.pending());
        drop(receiver);
        sender.failed().await;
    }

    #[test]
    fn saturation_has_an_independent_failure_and_releases_permits() {
        let (tx, rx) = channel(1, 1024);
        tx.send(Message::Ready).unwrap();
        assert!(tx.send(Message::Ready).is_err());
        assert_eq!(rx.failure(), Some(Failure::Full));
        assert!(rx.wake.try_recv().is_ok());
        assert_eq!(
            tx.shared.bytes.load(Ordering::Relaxed),
            encoded_size(&Message::Ready)
        );
        drop(rx);
        assert_eq!(tx.shared.bytes.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn byte_budget_applies_even_before_count_capacity() {
        let (tx, rx) = channel(64, encoded_size(&Message::Ready));
        tx.send(Message::Ready).unwrap();
        assert!(tx.send(Message::Ready).is_err());
        assert_eq!(rx.failure(), Some(Failure::Full));
        assert!(rx.receive().is_some());
        assert_eq!(tx.shared.bytes.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn oversize_and_closed_consumer_fail_without_retry() {
        let (tx, rx) = channel(64, 1);
        assert!(tx.send(Message::Ready).is_err());
        assert_eq!(rx.failure(), Some(Failure::Oversize));
        let (tx, rx) = channel(64, 1024);
        drop(rx);
        assert!(tx.send(Message::Ready).is_err());
    }

    #[test]
    fn full_queue_reports_initial_row_and_terminal_failures_out_of_band() {
        let terminal = QueryEvent::ExecutionCompleted {
            status: "completed".into(),
            transaction: QueryTransactionSnapshot::default(),
            omitted_rows: 0,
            omitted_result_sets: 0,
            omitted_notices: 0,
            omitted_metadata_bytes: 0,
            truncation_reasons: Vec::new(),
            error: None,
            refusal: None,
            context: None,
        };
        for message in [
            event(1, QueryEvent::ExecutionStarted),
            row(2),
            event(3, terminal),
        ] {
            let (tx, rx) = channel(1, QUEUE_BYTES);
            tx.send(Message::Ready).unwrap();
            // Consume the earlier wake while deliberately leaving data queued.
            rx.wake.try_recv().unwrap();
            assert!(tx.send(message).is_err());
            assert_eq!(rx.failure(), Some(Failure::Full));
            rx.wake.try_recv().unwrap();
            assert!(matches!(rx.receive(), Some(Message::Ready)));
            assert!(rx.receive().is_none());
            assert_eq!(tx.shared.bytes.load(Ordering::Relaxed), 0);
            assert!(tx.send(Message::Ready).is_err());
        }
    }

    #[test]
    fn slow_consumer_preserves_order_and_coalesces_wakes_with_bounded_bytes() {
        use std::sync::Barrier;
        let budget = (1..=4).map(|sequence| encoded_size(&row(sequence))).sum();
        let (tx, rx) = channel(4, budget);
        let filled = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let producer = {
            let tx = tx.clone();
            let filled = filled.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                for sequence in 1..=4 {
                    tx.send(row(sequence)).unwrap();
                }
                filled.wait();
                resume.wait();
                for sequence in 5..=6 {
                    tx.send(row(sequence)).unwrap();
                }
            })
        };
        filled.wait();
        assert_eq!(rx.data.len(), 4);
        assert_eq!(rx.wake.len(), 1);
        assert_eq!(tx.shared.bytes.load(Ordering::Relaxed), budget);
        let mut sequences = Vec::new();
        for _ in 0..2 {
            let Some(Message::Event(envelope)) = rx.receive() else {
                panic!("missing accepted row")
            };
            sequences.push(envelope.sequence);
        }
        resume.wait();
        producer.join().unwrap();
        while let Some(Message::Event(envelope)) = rx.receive() {
            sequences.push(envelope.sequence);
        }
        assert_eq!(sequences, [1, 2, 3, 4, 5, 6]);
        assert!(rx.failure().is_none());
        assert!(rx.high_water() <= budget);
        assert_eq!(tx.shared.bytes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn retired_consumer_releases_bytes_and_cannot_affect_a_new_mailbox() {
        let (old_sender, old_receiver) = channel(4, QUEUE_BYTES);
        old_sender.send(row(1)).unwrap();
        drop(old_receiver);
        let (sender, receiver) = channel(4, QUEUE_BYTES);
        sender.send(row(1)).unwrap();
        assert!(old_sender.send(row(2)).is_err());
        old_sender.fail(Failure::Backend("late old-session failure".into()));
        assert_eq!(old_sender.shared.bytes.load(Ordering::Relaxed), 0);
        assert!(receiver.failure().is_none());
        assert!(
            matches!(receiver.receive(), Some(Message::Event(envelope)) if envelope.sequence == 1)
        );
    }

    #[test]
    fn first_failure_is_sticky_and_fences_other_producers() {
        let (tx, rx) = channel(4, QUEUE_BYTES);
        let other_producer = tx.clone();
        tx.fail(Failure::Backend("first failure".into()));
        let producer = std::thread::spawn(move || {
            assert!(other_producer.send(row(1)).is_err());
            other_producer.fail(Failure::Full);
        });
        producer.join().unwrap();
        assert_eq!(rx.failure(), Some(Failure::Backend("first failure".into())));
        assert_eq!(rx.wake.len(), 1);
        assert!(!rx.pending());
    }

    #[test]
    fn escaped_notice_and_command_errors_use_encoded_byte_budget() {
        for message in [
            event(
                1,
                QueryEvent::Notice {
                    severity: "NOTICE".into(),
                    message: "\0".repeat(100),
                },
            ),
            Message::Review {
                execution: "execution".into(),
                sql: "\0".repeat(100),
                statements: Vec::new(),
                parameters: Some(vec![ReviewParameter {
                    name: "bound".into(),
                    value: Some("\0".repeat(100)),
                }]),
                row_limit: Some(100),
            },
            Message::Rejected {
                execution: "execution".into(),
                message: "\0".repeat(100),
            },
        ] {
            let bytes = encoded_size(&message);
            let (tx, rx) = channel(64, bytes - 1);
            assert!(tx.send(message).is_err());
            assert_eq!(rx.failure(), Some(Failure::Oversize));
            assert_eq!(tx.shared.bytes.load(Ordering::Relaxed), 0);
        }
    }
}
