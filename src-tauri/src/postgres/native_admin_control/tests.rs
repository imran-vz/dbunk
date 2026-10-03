use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::oneshot;

struct Fake {
    reply: BoxFuture<'static, Result<Option<bool>, Failure>>,
    calls: Arc<AtomicUsize>,
    cleaned: Arc<AtomicBool>,
    cancel_cleanup: Arc<AtomicBool>,
    cleanup_wait: Option<oneshot::Receiver<()>>,
    cleanup_started: Option<oneshot::Sender<()>>,
}
impl Fake {
    fn new(reply: BoxFuture<'static, Result<Option<bool>, Failure>>) -> Self {
        Self {
            reply,
            calls: Arc::new(AtomicUsize::new(0)),
            cleaned: Arc::new(AtomicBool::new(false)),
            cancel_cleanup: Arc::new(AtomicBool::new(false)),
            cleanup_wait: None,
            cleanup_started: None,
        }
    }
}
impl Transport for Fake {
    fn signal<'a>(
        &'a mut self,
        _: &'a Target,
        _: Action,
    ) -> BoxFuture<'a, Result<Option<bool>, Failure>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::mem::replace(
            &mut self.reply,
            Box::pin(async { panic!("signal retried") }),
        )
    }
    fn cleanup(self, cancel: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            self.cancel_cleanup.store(cancel, Ordering::SeqCst);
            if let Some(started) = self.cleanup_started {
                let _ = started.send(());
            }
            if let Some(wait) = self.cleanup_wait {
                let _ = wait.await;
            }
            self.cleaned.store(true, Ordering::SeqCst);
        })
    }
}
#[tokio::test]
async fn cancellation_and_retirement_before_dispatch_never_signal() {
    for retire in [false, true] {
        let permit = ControlPermit::test_permit();
        let cancelled = permit.test_cancellation();
        if retire {
            permit.test_retire();
        } else {
            permit.test_cancel();
        }
        let fake = Fake::new(Box::pin(async { panic!("must not signal") }));
        let calls = fake.calls.clone();
        let cleaned = fake.cleaned.clone();
        assert_eq!(
            run(
                fake,
                &permit,
                cancelled,
                &Target::test_target(),
                Action::CancelQuery,
                Instant::now() + Duration::from_secs(1)
            )
            .await,
            Outcome::NotDispatched {
                reason: Failure::Cancelled
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(cleaned.load(Ordering::SeqCst));
    }
}
#[tokio::test]
async fn exact_server_results_and_transport_loss_keep_distinct_outcomes() {
    for (reply, expected) in [
        (Ok(None), Outcome::TargetChanged),
        (Ok(Some(false)), Outcome::SignalNotSent),
        (Ok(Some(true)), Outcome::SignalSent),
        (
            Err(Failure::Connection),
            Outcome::OutcomeUnknown {
                reason: Failure::Connection,
            },
        ),
    ] {
        let permit = ControlPermit::test_permit();
        let fake = Fake::new(Box::pin(async move { reply }));
        let calls = fake.calls.clone();
        let cancel = fake.cancel_cleanup.clone();
        assert_eq!(
            run(
                fake,
                &permit,
                permit.test_cancellation(),
                &Target::test_target(),
                Action::TerminateSession,
                Instant::now() + Duration::from_secs(1)
            )
            .await,
            expected
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            cancel.load(Ordering::SeqCst),
            matches!(expected, Outcome::OutcomeUnknown { .. })
        );
    }
}
#[tokio::test]
async fn late_cancel_settles_the_same_future_and_preserves_success() {
    let permit = Arc::new(ControlPermit::test_permit());
    let cancelled = permit.test_cancellation();
    let interruption = permit.clone();
    let fake = Fake::new(Box::pin(async move {
        interruption.test_cancel();
        tokio::task::yield_now().await;
        Ok(Some(true))
    }));
    let calls = fake.calls.clone();
    assert_eq!(
        run(
            fake,
            &permit,
            cancelled,
            &Target::test_target(),
            Action::CancelQuery,
            Instant::now() + Duration::from_secs(1)
        )
        .await,
        Outcome::SignalSent
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unresolved_postdispatch_cancel_is_unknown_and_never_retried() {
    let permit = Arc::new(ControlPermit::test_permit());
    let cancelled = permit.test_cancellation();
    let interruption = permit.clone();
    let fake = Fake::new(Box::pin(async move {
        interruption.test_retire();
        std::future::pending().await
    }));
    let calls = fake.calls.clone();
    assert_eq!(
        run(
            fake,
            &permit,
            cancelled,
            &Target::test_target(),
            Action::TerminateSession,
            Instant::now() + Duration::from_millis(20)
        )
        .await,
        Outcome::OutcomeUnknown {
            reason: Failure::Cancelled
        }
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn receipt_waits_for_joined_cleanup_even_after_acknowledgement() {
    let permit = ControlPermit::test_permit();
    let (started, ready) = oneshot::channel();
    let (release, wait) = oneshot::channel();
    let mut fake = Fake::new(Box::pin(async { Ok(Some(true)) }));
    fake.cleanup_started = Some(started);
    fake.cleanup_wait = Some(wait);
    let task = tokio::spawn(async move {
        run(
            fake,
            &permit,
            permit.test_cancellation(),
            &Target::test_target(),
            Action::CancelQuery,
            Instant::now() + Duration::from_secs(1),
        )
        .await
    });
    ready.await.unwrap();
    assert!(!task.is_finished());
    release.send(()).unwrap();
    assert_eq!(task.await.unwrap(), Outcome::SignalSent);
}
