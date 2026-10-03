use super::*;
use crate::postgres::backup::{
    manager::PgToolJobManager,
    protocol::*,
    runner::{self, Ready, Request},
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

#[tokio::test]
async fn native_reap_does_not_freeze_cancellation_before_late_observed_success() {
    let owner = Ownership::default();
    let manager = PgToolJobManager::new().with_native_ownership(owner.clone());
    let mut payload = crate::postgres::backup::tests::backup(Path::new("/not-created/archive"));
    payload.connection_id = "native-reap".into();
    let mut snapshot = Request::Backup(payload).snapshot();
    snapshot.kind = PgToolJobKind::Restore;
    let admission = manager.admission("native-reap").unwrap();
    let (send, receive) = tokio::sync::oneshot::channel();
    let (started, ready) = tokio::sync::oneshot::channel();
    let effects = Arc::new(AtomicUsize::new(0));
    let completed = effects.clone();
    let job = manager
        .start(
            admission,
            snapshot,
            move |context| async move {
                context.phase(PgToolJobPhase::Preflight)?;
                context.phase(PgToolJobPhase::Running)?;
                context.mark_restore_dispatch();
                let reap_context = context.clone();
                let result = runner::reap(
                    &context,
                    async move {
                        let _ = started.send(());
                        let _ = receive.await;
                        reap_context.irreversible_success();
                    },
                    Duration::from_millis(1),
                )
                .await;
                assert!(result.is_ok());
                context.phase_after_irreversible_success(PgToolJobPhase::Finalizing)?;
                Ok(Ready::Restore)
            },
            Box::pin(async move {
                completed.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
    ready.await.unwrap();
    manager.cancel(&job.job_id).unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        manager.get(&job.job_id).unwrap().phase,
        PgToolJobPhase::Cancelling
    );
    assert!(manager.admission("native-reap").is_err());
    assert!(owner
        .drain_until(tokio::time::Instant::now())
        .await
        .is_err());
    send.send(()).unwrap();
    owner
        .drain_until(tokio::time::Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(
        manager.get(&job.job_id).unwrap().phase,
        PgToolJobPhase::Completed
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}
