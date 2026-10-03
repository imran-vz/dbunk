use super::*;
use crate::backend::object_ddl::{
    render_preview,
    tests::{atomic_case, mixed_case},
    ObjectDdlStop,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
use tokio::sync::oneshot;

type Mutate = Box<dyn FnMut(usize, ObjectDdlClaim) -> Result<ObjectDdlClaim, Failure> + Send>;
struct Mock {
    description: ObjectDdlDescription,
    commands: Arc<Mutex<Vec<String>>>,
    captures: usize,
    mutate: Mutate,
    verified: bool,
    fail: Vec<(String, Failure)>,
    residue: Option<ObjectDdlResidue>,
    block: String,
    reached: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    cancels: Arc<AtomicUsize>,
    cleaned: Arc<Mutex<Option<bool>>>,
}
impl Transport for Mock {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.commands.lock().unwrap().push(sql.into());
            if sql == self.block {
                if let Some(reached) = self.reached.take() {
                    let _ = reached.send(());
                }
                if let Some(release) = self.release.take() {
                    let _ = release.await;
                }
            }
            match self.fail.iter().find(|(s, _)| s == sql) {
                Some((_, failure)) => Err(failure.clone()),
                None => Ok(()),
            }
        })
    }
    fn database_oid(&mut self) -> BoxFuture<'_, Result<u32, Failure>> {
        Box::pin(async move { Ok(self.description.database_oid) })
    }
    fn capture<'a>(
        &'a mut self,
        spec: &'a ClaimSpec,
    ) -> BoxFuture<'a, Result<ObjectDdlClaim, Failure>> {
        Box::pin(async move {
            self.captures += 1;
            let claim = self
                .description
                .claims
                .iter()
                .flatten()
                .find(|claim| claim.spec() == *spec)
                .cloned()
                .ok_or(Failure::TargetChanged)?;
            (self.mutate)(self.captures, claim)
        })
    }
    fn verify<'a>(
        &'a mut self,
        _: &'a ObjectDdlOperation,
        _: &'a [ObjectDdlClaim],
    ) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move { Ok(self.verified) })
    }
    fn residue<'a>(
        &'a mut self,
        _: &'a ObjectDdlOperation,
        _: &'a [ObjectDdlClaim],
    ) -> BoxFuture<'a, Option<ObjectDdlResidue>> {
        Box::pin(async move { self.residue.clone() })
    }
    fn canceller(&self) -> Canceller {
        let cancels = self.cancels.clone();
        Arc::new(move || {
            cancels.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        })
    }
    fn cleanup(self, cancel: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            *self.cleaned.lock().unwrap() = Some(cancel);
        })
    }
}
fn mock(description: &ObjectDdlDescription) -> Mock {
    Mock {
        description: description.clone(),
        commands: Default::default(),
        captures: 0,
        mutate: Box::new(|_, claim| Ok(claim)),
        verified: true,
        fail: Vec::new(),
        residue: None,
        block: String::new(),
        reached: None,
        release: None,
        cancels: Default::default(),
        cleaned: Default::default(),
    }
}
fn case(
    mixed: bool,
) -> (
    ObjectDdlDescription,
    Vec<ObjectDdlOperation>,
    ObjectDdlPreview,
) {
    let (description, operations) = if mixed { mixed_case() } else { atomic_case() };
    let preview = render_preview(&description, &operations, None, false).unwrap();
    (description, operations, preview)
}
async fn run_mock(
    mock: Mock,
    permit: &WritePermit,
    description: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    preview: &ObjectDdlPreview,
) -> Outcome {
    run(
        mock,
        permit,
        permit.test_cancellation(),
        description,
        operations,
        preview,
        Instant::now() + Duration::from_secs(5),
    )
    .await
}
fn sent(commands: &Arc<Mutex<Vec<String>>>, sql: &str) -> usize {
    commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| *command == sql)
        .count()
}

#[tokio::test]
async fn atomic_group_rechecks_inside_one_transaction_then_commits_once() {
    let (description, operations, preview) = case(false);
    let mock = mock(&description);
    let commands = mock.commands.clone();
    let cleaned = mock.cleaned.clone();
    let permit = WritePermit::test_permit();
    let outcome = run_mock(mock, &permit, &description, &operations, &preview).await;
    assert!(matches!(outcome, Outcome::Applied { .. }), "{outcome:?}");
    let commands = commands.lock().unwrap().clone();
    assert_eq!(
        commands,
        vec![
            LOCK_TIMEOUT.to_owned(),
            "BEGIN ISOLATION LEVEL READ COMMITTED".into(),
            preview.statements[0].sql.clone(),
            preview.statements[1].sql.clone(),
            "COMMIT".into(),
        ]
    );
    assert_eq!(*cleaned.lock().unwrap(), Some(false));
}

#[tokio::test]
async fn stale_or_recreated_identity_refuses_before_any_statement() {
    let (description, operations, preview) = case(false);
    for stale in 0..3 {
        let mut mock = mock(&description);
        mock.mutate = Box::new(move |_, claim| match (stale, claim) {
            // Same name, recreated object: new OID.
            (
                0,
                ObjectDdlClaim::Existing {
                    reference,
                    mut address,
                },
            ) => {
                address.object_oid += 1;
                Ok(ObjectDdlClaim::Existing { reference, address })
            }
            // Same OID, altered since review: new catalog row version.
            (
                1,
                ObjectDdlClaim::Existing {
                    reference,
                    mut address,
                },
            ) => {
                address.row_version = "78".into();
                Ok(ObjectDdlClaim::Existing { reference, address })
            }
            // Target vanished or the create target now exists.
            (2, ObjectDdlClaim::Absent { .. }) => Err(Failure::TargetChanged),
            (_, claim) => Ok(claim),
        });
        let commands = mock.commands.clone();
        let permit = WritePermit::test_permit();
        assert_eq!(
            run_mock(mock, &permit, &description, &operations, &preview).await,
            Outcome::NotDispatched {
                reason: Failure::TargetChanged
            },
            "stale {stale}"
        );
        for statement in &preview.statements {
            assert_eq!(sent(&commands, &statement.sql), 0);
        }
        assert_eq!(sent(&commands, "COMMIT"), 0);
    }
}

#[tokio::test]
async fn failed_post_statement_identity_rolls_back_the_whole_group() {
    let (description, operations, preview) = case(false);
    let mut mock = mock(&description);
    mock.verified = false;
    let commands = mock.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        run_mock(mock, &permit, &description, &operations, &preview).await,
        Outcome::Stopped {
            committed: 0,
            stopped_at: 0,
            stop: ObjectDdlStop::RolledBack,
            reason: Failure::TargetChanged,
            residue: None,
        }
    );
    assert_eq!(sent(&commands, "ROLLBACK"), 1);
    assert_eq!(sent(&commands, "COMMIT"), 0);
}

#[tokio::test]
async fn lost_commit_or_rollback_reply_is_unknown_and_never_retried() {
    let (description, operations, preview) = case(false);
    for lost in ["COMMIT", "ROLLBACK"] {
        let mut mock = mock(&description);
        mock.fail.push((lost.into(), Failure::Connection));
        if lost == "ROLLBACK" {
            mock.verified = false;
        }
        let commands = mock.commands.clone();
        let permit = WritePermit::test_permit();
        let outcome = run_mock(mock, &permit, &description, &operations, &preview).await;
        assert_eq!(
            outcome,
            Outcome::OutcomeUnknown {
                committed: 0,
                uncertain_end: 2,
                reason: if lost == "COMMIT" {
                    Failure::Connection
                } else {
                    Failure::RollbackUnconfirmed
                },
            }
        );
        assert_eq!(sent(&commands, lost), 1);
        assert_eq!(sent(&commands, &preview.statements[0].sql), 1);
    }
}

#[tokio::test]
async fn standalone_rejection_reports_committed_prefix_and_residue() {
    let (description, operations, preview) = case(true);
    let mut mock = mock(&description);
    mock.fail.push((
        preview.statements[2].sql.clone(),
        Failure::Database {
            code: Some("42710".into()),
        },
    ));
    let commands = mock.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        run_mock(mock, &permit, &description, &operations, &preview).await,
        Outcome::Stopped {
            committed: 2,
            stopped_at: 2,
            stop: ObjectDdlStop::Rejected,
            reason: Failure::Database {
                code: Some("42710".into())
            },
            residue: None,
        }
    );
    // The atomic suffix after the rejected statement is never sent.
    assert_eq!(sent(&commands, &preview.statements[3].sql), 0);
    assert_eq!(sent(&commands, "COMMIT"), 1);

    let mut mock = self::mock(&description);
    mock.fail.push((
        preview.statements[1].sql.clone(),
        Failure::Database {
            code: Some("23505".into()),
        },
    ));
    mock.residue = Some(ObjectDdlResidue::InvalidIndex {
        schema: "app".into(),
        name: "orders_state_idx".into(),
    });
    let permit = WritePermit::test_permit();
    let outcome = run_mock(mock, &permit, &description, &operations, &preview).await;
    assert!(matches!(
        outcome,
        Outcome::Stopped {
            committed: 1,
            stopped_at: 1,
            stop: ObjectDdlStop::Rejected,
            residue: Some(ObjectDdlResidue::InvalidIndex { .. }),
            ..
        }
    ));
    assert!(outcome.may_have_changed());
}

#[tokio::test]
async fn lost_standalone_reply_is_unknown_and_stops_without_retry() {
    let (description, operations, preview) = case(true);
    let mut mock = mock(&description);
    mock.fail
        .push((preview.statements[1].sql.clone(), Failure::Connection));
    let commands = mock.commands.clone();
    let cleaned = mock.cleaned.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        run_mock(mock, &permit, &description, &operations, &preview).await,
        Outcome::OutcomeUnknown {
            committed: 1,
            uncertain_end: 2,
            reason: Failure::Connection,
        }
    );
    assert_eq!(sent(&commands, &preview.statements[1].sql), 1);
    assert_eq!(sent(&commands, &preview.statements[2].sql), 0);
    assert_eq!(*cleaned.lock().unwrap(), Some(true));
}

#[tokio::test]
async fn standalone_target_changed_after_committed_prefix_is_not_dispatched() {
    let (description, operations, preview) = case(true);
    let mut mock = mock(&description);
    // Captures 1-3 recheck group 0 (database, schema, absent) inside its
    // transaction; the table changes before the standalone recheck.
    mock.mutate = Box::new(|_, claim| match claim {
        ObjectDdlClaim::Existing {
            reference,
            mut address,
        } if address.object_oid == 600 => {
            address.row_version = "99".into();
            Ok(ObjectDdlClaim::Existing { reference, address })
        }
        claim => Ok(claim),
    });
    let commands = mock.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        run_mock(mock, &permit, &description, &operations, &preview).await,
        Outcome::Stopped {
            committed: 1,
            stopped_at: 1,
            stop: ObjectDdlStop::NotDispatched,
            reason: Failure::TargetChanged,
            residue: None,
        }
    );
    assert_eq!(sent(&commands, &preview.statements[1].sql), 0);
}

#[tokio::test]
async fn cancellation_before_dispatch_sends_nothing() {
    let (description, operations, preview) = case(true);
    let mock = mock(&description);
    let commands = mock.commands.clone();
    let permit = WritePermit::test_permit();
    permit.test_cancel();
    assert_eq!(
        run_mock(mock, &permit, &description, &operations, &preview).await,
        Outcome::NotDispatched {
            reason: Failure::Cancelled
        }
    );
    for statement in &preview.statements {
        assert_eq!(sent(&commands, &statement.sql), 0);
    }
}

#[tokio::test]
async fn cancel_during_commit_keeps_the_known_commit_and_stops_later_groups() {
    let (description, operations, preview) = case(true);
    let mut mock = mock(&description);
    let (ready, reached) = oneshot::channel();
    let (release, released) = oneshot::channel();
    mock.block = "COMMIT".into();
    mock.reached = Some(ready);
    mock.release = Some(released);
    let commands = mock.commands.clone();
    let permit = Arc::new(WritePermit::test_permit());
    let running = permit.clone();
    let job = tokio::spawn(async move {
        run(
            mock,
            &running,
            running.test_cancellation(),
            &description,
            &operations,
            &preview,
            Instant::now() + Duration::from_secs(5),
        )
        .await
    });
    reached.await.unwrap();
    permit.test_cancel();
    let _ = release.send(());
    assert_eq!(
        job.await.unwrap(),
        Outcome::Stopped {
            committed: 1,
            stopped_at: 1,
            stop: ObjectDdlStop::NotDispatched,
            reason: Failure::Cancelled,
            residue: None,
        }
    );
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("CREATE INDEX"))
            .count(),
        0
    );
}

#[tokio::test]
async fn cancelled_standalone_statement_asks_the_server_and_reports_its_reply() {
    let (description, operations, preview) = case(true);
    let mut mock = mock(&description);
    let (ready, reached) = oneshot::channel();
    let (release, released) = oneshot::channel();
    mock.block = preview.statements[1].sql.clone();
    mock.reached = Some(ready);
    mock.release = Some(released);
    mock.fail.push((
        preview.statements[1].sql.clone(),
        Failure::Database {
            code: Some("57014".into()),
        },
    ));
    let cancels = mock.cancels.clone();
    let permit = Arc::new(WritePermit::test_permit());
    let running = permit.clone();
    let job = tokio::spawn(async move {
        run(
            mock,
            &running,
            running.test_cancellation(),
            &description,
            &operations,
            &preview,
            Instant::now() + Duration::from_secs(5),
        )
        .await
    });
    reached.await.unwrap();
    permit.test_cancel();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let _ = release.send(());
    let outcome = job.await.unwrap();
    assert_eq!(
        outcome,
        Outcome::Stopped {
            committed: 1,
            stopped_at: 1,
            stop: ObjectDdlStop::Rejected,
            reason: Failure::Cancelled,
            residue: None,
        }
    );
    assert!(cancels.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn inconsistent_plan_never_dispatches() {
    let (description, operations, mut preview) = case(true);
    preview.groups.swap(1, 2);
    let mock = mock(&description);
    let commands = mock.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        run_mock(mock, &permit, &description, &operations, &preview).await,
        Outcome::NotDispatched {
            reason: Failure::Limit
        }
    );
    assert!(commands.lock().unwrap().is_empty());
}
