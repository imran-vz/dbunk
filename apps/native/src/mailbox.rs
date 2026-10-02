//! Non-blocking, byte- and count-bounded delivery with an independent failure wake.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::results::encoded_size;
use dbunk_lib::backend::{QueryEventEnvelope, SinkClosed};
use serde::Serialize;

pub const QUEUE_CAPACITY: usize = 64;
pub const QUEUE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
// The fixed 64-slot queue bounds enum storage. Boxing would allocate for each
// delivered event without reducing the much larger, separately bounded payload.
#[allow(clippy::large_enum_variant)]
pub enum Message {
    Event(QueryEventEnvelope),
    Ready,
    Rejected { execution: String, message: String },
    Acked { execution: String, sequence: u64 },
    CancelFailed { execution: String, message: String },
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
    failed: Mutex<Option<Failure>>,
    closed: AtomicBool,
}
struct Queued {
    message: Option<Message>,
    bytes: usize,
    shared: Arc<Shared>,
}
impl Drop for Queued {
    fn drop(&mut self) {
        self.shared.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub fn channel(capacity: usize, budget: usize) -> (Sender, Receiver) {
    let (data, receive) = async_channel::bounded(capacity);
    let (wake, awakened) = async_channel::bounded(1);
    let shared = Arc::new(Shared {
        bytes: AtomicUsize::new(0),
        high_water: AtomicUsize::new(0),
        budget,
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
    pub fn send(&self, message: Message) -> Result<(), SinkClosed> {
        let bytes = encoded_size(&message);
        // Linearize admission and first failure across event and command-reply
        // producers. No serialization, await or callback runs under this lock.
        let mut failed = self.shared.failed.lock().unwrap();
        if self.shared.closed.load(Ordering::Acquire) || failed.is_some() {
            return Err(SinkClosed);
        }
        if bytes > self.shared.budget {
            failed.get_or_insert(Failure::Oversize);
            let _ = self.wake.try_send(());
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
            return Err(SinkClosed);
        };
        self.shared
            .high_water
            .fetch_max(used + bytes, Ordering::Relaxed);
        let queued = Queued {
            message: Some(message),
            bytes,
            shared: self.shared.clone(),
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
            return Err(SinkClosed);
        }
        let _ = self.wake.try_send(());
        Ok(())
    }
    pub fn fail(&self, failure: Failure) {
        self.shared.failed.lock().unwrap().get_or_insert(failure);
        let _ = self.wake.try_send(());
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
