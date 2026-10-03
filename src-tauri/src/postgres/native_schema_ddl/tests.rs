use super::*;
use crate::backend::schema_ddl::CreateSchemaStatement;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

struct Mock {
    commands: Arc<Mutex<Vec<String>>>,
    block_sql: &'static str,
    reached: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    failure: Option<&'static str>,
    cleanup_reached: Option<oneshot::Sender<()>>,
    cleanup_release: Option<oneshot::Receiver<()>>,
}
impl Transport for Mock {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.commands.lock().unwrap().push(sql.into());
            if sql == self.block_sql {
                if let Some(reached) = self.reached.take() {
                    let _ = reached.send(());
                }
                if let Some(release) = self.release.take() {
                    let _ = release.await;
                }
            }
            if self.failure == Some(sql) {
                Err(Failure::Connection)
            } else {
                Ok(())
            }
        })
    }
    fn cleanup(self, _: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            if let Some(reached) = self.cleanup_reached {
                let _ = reached.send(());
            }
            if let Some(release) = self.cleanup_release {
                let _ = release.await;
            }
        })
    }
}
fn mock(block: &'static str) -> (Mock, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (reached, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    (
        Mock {
            commands: Default::default(),
            block_sql: block,
            reached: Some(reached),
            release: Some(released),
            failure: None,
            cleanup_reached: None,
            cleanup_release: None,
        },
        ready,
        release,
    )
}
fn preview() -> CreateSchemaPreview {
    CreateSchemaPreview {
        statements: vec![CreateSchemaStatement {
            sql: "CREATE SCHEMA \"exact\";".into(),
            summary: "Create schema exact".into(),
        }],
    }
}

#[tokio::test]
async fn cancellation_before_commit_never_sends_commit_and_waits_cleanup() {
    let (mut socket, reached, release) = mock("CREATE SCHEMA \"exact\";");
    let commands = socket.commands.clone();
    let (cleaning, cleanup_started) = oneshot::channel();
    let (finish, finished) = oneshot::channel();
    socket.cleanup_reached = Some(cleaning);
    socket.cleanup_release = Some(finished);
    let permit = Arc::new(WritePermit::test_permit());
    let runner_permit = permit.clone();
    let mut runner = tokio::spawn(async move {
        run(
            socket,
            &runner_permit,
            runner_permit.test_cancellation(),
            &preview(),
            Instant::now() + Duration::from_secs(5),
        )
        .await
    });
    reached.await.unwrap();
    permit.test_cancel();
    let _ = release.send(());
    cleanup_started.await.unwrap();
    assert!(!runner.is_finished());
    assert!(!commands.lock().unwrap().iter().any(|sql| sql == "COMMIT"));
    finish.send(()).unwrap();
    assert!(matches!(
        (&mut runner).await.unwrap(),
        Outcome::NotApplied {
            reason: Failure::Cancelled
        }
    ));
}

#[tokio::test]
async fn retirement_after_commit_admission_preserves_received_success() {
    let (socket, reached, release) = mock("COMMIT");
    let commands = socket.commands.clone();
    let permit = Arc::new(WritePermit::test_permit());
    let runner_permit = permit.clone();
    let runner = tokio::spawn(async move {
        run(
            socket,
            &runner_permit,
            runner_permit.test_cancellation(),
            &preview(),
            Instant::now() + Duration::from_secs(5),
        )
        .await
    });
    reached.await.unwrap();
    permit.test_retire();
    release.send(()).unwrap();
    assert!(matches!(
        runner.await.unwrap(),
        Outcome::Applied { statements: 1, .. }
    ));
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|sql| sql.as_str() == "COMMIT")
            .count(),
        1
    );
}

#[tokio::test]
async fn lost_commit_reply_is_unknown_but_statement_failure_is_not_applied() {
    for failed in ["COMMIT", "CREATE SCHEMA \"exact\";"] {
        let (mut socket, _, release) = mock("");
        drop(release);
        socket.failure = Some(failed);
        let commands = socket.commands.clone();
        let permit = WritePermit::test_permit();
        let outcome = run(
            socket,
            &permit,
            permit.test_cancellation(),
            &preview(),
            Instant::now() + Duration::from_secs(5),
        )
        .await;
        if failed == "COMMIT" {
            assert!(matches!(
                outcome,
                Outcome::OutcomeUnknown {
                    reason: Failure::Connection
                }
            ));
        } else {
            assert!(matches!(
                outcome,
                Outcome::NotApplied {
                    reason: Failure::Connection
                }
            ));
            assert!(!commands.lock().unwrap().iter().any(|sql| sql == "COMMIT"));
        }
    }
}

#[tokio::test]
async fn expired_deadline_never_admits_commit() {
    let (socket, _, release) = mock("");
    drop(release);
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    let outcome = run(
        socket,
        &permit,
        permit.test_cancellation(),
        &preview(),
        Instant::now(),
    )
    .await;
    assert!(matches!(
        outcome,
        Outcome::NotApplied {
            reason: Failure::Timeout
        }
    ));
    assert!(commands.lock().unwrap().is_empty());
    assert!(permit.check_preparing());
}
