//! Opt-in barriers for the owned fixture's actual-window tests.
//! Nothing here is compiled into an ordinary native build.
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::task::Poll;
use tokio::sync::watch;

struct Hooks {
    scenario: String,
    opens: AtomicUsize,
    armed: AtomicBool,
    drain: watch::Sender<bool>,
}
static HOOKS: OnceLock<Hooks> = OnceLock::new();
gpui::actions!(verification, [ResumeDrain, ReplaceView, Reconnect]);

pub fn initialize() -> anyhow::Result<()> {
    let Ok(scenario) = std::env::var("DBUNK_NATIVE_VERIFY") else {
        return Ok(());
    };
    anyhow::ensure!(
        matches!(
            scenario.as_str(),
            "connect" | "reconnect" | "credit" | "saturated" | "streaming"
        ),
        "Unknown native verification scenario"
    );
    let (drain, _) = watch::channel(false);
    HOOKS
        .set(Hooks {
            scenario,
            opens: AtomicUsize::new(0),
            armed: AtomicBool::new(true),
            drain,
        })
        .map_err(|_| anyhow::anyhow!("Verification already initialized"))?;
    Ok(())
}

pub fn enabled() -> bool {
    HOOKS.get().is_some()
}

pub fn capacity() -> usize {
    if HOOKS
        .get()
        .is_some_and(|hooks| hooks.scenario == "saturated" && hooks.armed.load(Ordering::Acquire))
    {
        3
    } else {
        crate::mailbox::QUEUE_CAPACITY
    }
}

/// Poll the real open once, then hold its pending future until window shutdown
/// cancels the caller. Backend work stays owned by the facade and must be joined.
pub async fn open<F: std::future::Future>(future: F) -> F::Output {
    if let Some(hooks) = HOOKS.get() {
        let number = hooks.opens.fetch_add(1, Ordering::AcqRel) + 1;
        if (hooks.scenario == "connect" && number == 1)
            || (hooks.scenario == "reconnect" && number == 2)
        {
            let mut future = Box::pin(future);
            match futures_util::poll!(&mut future) {
                Poll::Ready(value) => return value,
                Poll::Pending => {
                    eprintln!("VERIFY open-pending {number}");
                    std::future::pending::<()>().await;
                }
            }
            return future.await;
        }
    }
    future.await
}

pub fn before_run() {
    if let Some(hooks) = HOOKS.get()
        && matches!(hooks.scenario.as_str(), "credit" | "saturated")
        && hooks.armed.swap(false, Ordering::AcqRel)
    {
        hooks.drain.send_replace(true);
        eprintln!("VERIFY drain-held");
    }
}

pub async fn drain() {
    if let Some(hooks) = HOOKS.get() {
        let mut drain = hooks.drain.subscribe();
        while *drain.borrow_and_update() {
            if drain.changed().await.is_err() {
                break;
            }
        }
    }
}

pub fn release() {
    if let Some(hooks) = HOOKS.get() {
        hooks.drain.send_replace(false);
        eprintln!("VERIFY drain-released");
    }
}

pub fn offered(message: &crate::mailbox::Message) {
    if !enabled() {
        return;
    }
    if let crate::mailbox::Message::Event(envelope) = message {
        let kind = match envelope.event {
            dbunk_lib::backend::QueryEvent::RowBatch { .. } => "rows",
            dbunk_lib::backend::QueryEvent::ExecutionCompleted { .. } => "terminal",
            dbunk_lib::backend::QueryEvent::SessionState { .. } => "session",
            _ => return,
        };
        eprintln!(
            "VERIFY offered {kind} session={} sequence={}",
            envelope.session_id, envelope.sequence
        );
    }
}
