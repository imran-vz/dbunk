//! Host seam: what the backend needs from the desktop shell that runs it.
//!
//! Nothing below the command layer names a UI framework. A host supplies a
//! Tokio runtime handle to each manager's monitor, an event sink per stream,
//! and lifecycle inputs (window identity, focus, document replacement and
//! teardown) through the managers' own methods. See ADR-0032.

use std::sync::Arc;

/// The host's consumer is gone or cannot take the event. Core code treats it
/// as a lost stream: the event is not retried and the owner is torn down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SinkClosed;

/// Delivers one stream's events to the host, in call order.
///
/// `send` runs on async worker threads and must return without blocking. A
/// host that has to cross a thread or process boundary queues there; bounded
/// retention is the caller's job (see the Query Session credit window), not
/// the sink's. Serialization, if the host needs any, happens inside the sink.
pub trait EventSink<T>: Send + Sync + 'static {
    fn send(&self, event: T) -> Result<(), SinkClosed>;
}

impl<T, F> EventSink<T> for F
where
    F: Fn(T) -> Result<(), SinkClosed> + Send + Sync + 'static,
{
    fn send(&self, event: T) -> Result<(), SinkClosed> {
        self(event)
    }
}

pub(crate) type SharedSink<T> = Arc<dyn EventSink<T>>;

/// A host document replacing the one that owned earlier replies. A WebView
/// host reports its page loads; a host whose views cannot reload reports
/// `Started` once per window and never again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentLoad {
    /// The previous document is gone: its pending replies are retired.
    Started,
    /// Loading ended, possibly by failing with the old document still alive.
    Finished,
}

/// A host runtime for tests that start a monitor from a thread with no
/// ambient Tokio context, as a host's setup does.
#[cfg(test)]
pub(crate) fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("host runtime")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn a_closure_is_a_sink_and_keeps_call_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let sink: SharedSink<u32> = Arc::new(move |event: u32| {
            recorded.lock().unwrap().push(event);
            Ok(())
        });
        for event in [3, 1, 2] {
            sink.send(event).expect("open sink");
        }
        assert_eq!(*seen.lock().unwrap(), [3, 1, 2]);
    }

    #[test]
    fn a_closed_sink_reports_it_to_the_sender() {
        let sink: SharedSink<u32> = Arc::new(|_: u32| Err(SinkClosed));
        assert_eq!(sink.send(1), Err(SinkClosed));
    }
}
