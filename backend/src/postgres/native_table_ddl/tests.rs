use super::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
struct Mock {
    commands: Arc<Mutex<Vec<String>>>,
    captures: VecDeque<TableDdlDescription>,
    locked: bool,
    fail: Option<&'static str>,
    block: &'static str,
    reached: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    cleaned: Arc<Mutex<bool>>,
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
            if self.fail == Some(sql) {
                Err(Failure::Connection)
            } else {
                Ok(())
            }
        })
    }
    fn capture<'a>(
        &'a mut self,
        _: &'a TableDdlRequest,
    ) -> BoxFuture<'a, Result<TableDdlDescription, Failure>> {
        Box::pin(async move { self.captures.pop_front().ok_or(Failure::TargetChanged) })
    }
    fn locked(&mut self, _: TableIdentity) -> BoxFuture<'_, Result<bool, Failure>> {
        Box::pin(async move { Ok(self.locked) })
    }
    fn cleanup(self, _: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            *self.cleaned.lock().unwrap() = true;
        })
    }
}
fn setup() -> (Mock, TableDdlDescription, TableDdlIntent, TableDdlPreview) {
    let target = TableDdlDescription {
        identity: TableIdentity {
            database_oid: 1,
            relation_oid: 2,
        },
        schema_oid: 3,
        schema: "a".into(),
        table: "b".into(),
        namespace_xmin: "9".into(),
        namespace_ctid: "(0,1)".into(),
        column: None,
        comment: None,
    };
    let intent = TableDdlIntent::Rename {
        new_name: "c".into(),
    };
    let mock = Mock {
        commands: Default::default(),
        captures: VecDeque::from([
            target.clone(),
            target.clone(),
            post_target(&target, &intent),
        ]),
        locked: true,
        fail: None,
        block: "",
        reached: None,
        release: None,
        cleaned: Default::default(),
    };
    let preview = TableDdlPreview {
        sql: "ALTER TABLE a.b RENAME TO c".into(),
        summary: "rename".into(),
        statement_timeout_ms: None,
        operation_timeout_ms: TABLE_DDL_OPERATION_TIMEOUT_MS,
    };
    (mock, target, intent, preview)
}
async fn execute_mock(
    mock: Mock,
    target: &TableDdlDescription,
    intent: &TableDdlIntent,
    preview: &TableDdlPreview,
) -> Outcome {
    let permit = WritePermit::test_permit();
    run(
        mock,
        &permit,
        permit.test_cancellation(),
        target,
        intent,
        preview,
        Instant::now() + Duration::from_secs(3),
    )
    .await
}
#[tokio::test]
async fn wrong_lock_refuses_dispatch_and_namespace_aba_rolls_back_after_statement() {
    let (mut mock, target, intent, preview) = setup();
    mock.locked = false;
    let commands = mock.commands.clone();
    let cleaned = mock.cleaned.clone();
    assert!(matches!(
        execute_mock(mock, &target, &intent, &preview).await,
        Outcome::NotDispatched {
            reason: Failure::TargetChanged
        }
    ));
    assert!(!commands.lock().unwrap().contains(&preview.sql));
    assert!(*cleaned.lock().unwrap());
    let (mut mock, target, intent, preview) = setup();
    // Names return to their original values but physical namespace version differs.
    mock.captures[2].namespace_ctid = "(0,2)".into();
    let commands = mock.commands.clone();
    assert!(matches!(
        execute_mock(mock, &target, &intent, &preview).await,
        Outcome::RolledBack {
            reason: Failure::TargetChanged
        }
    ));
    assert!(commands.lock().unwrap().contains(&"ROLLBACK".into()));
    assert!(!commands.lock().unwrap().contains(&"COMMIT".into()));
}
#[tokio::test]
async fn lost_rollback_and_commit_replies_are_unknown() {
    for command in ["ROLLBACK", "COMMIT"] {
        let (mut mock, target, intent, preview) = setup();
        mock.fail = Some(command);
        if command == "ROLLBACK" {
            mock.captures[2].namespace_xmin = "10".into();
        }
        assert!(matches!(
            execute_mock(mock, &target, &intent, &preview).await,
            Outcome::OutcomeUnknown { .. }
        ));
    }
}
#[tokio::test]
async fn cancellation_before_commit_rolls_back_and_known_commit_wins_retirement() {
    for block in ["ALTER TABLE a.b RENAME TO c", "COMMIT"] {
        let (mut mock, target, intent, preview) = setup();
        let (ready, reached) = oneshot::channel();
        let (release, released) = oneshot::channel();
        mock.block = block;
        mock.reached = Some(ready);
        mock.release = Some(released);
        let commands = mock.commands.clone();
        let cleaned = mock.cleaned.clone();
        let permit = Arc::new(WritePermit::test_permit());
        let running = permit.clone();
        let job = tokio::spawn(async move {
            run(
                mock,
                &running,
                running.test_cancellation(),
                &target,
                &intent,
                &preview,
                Instant::now() + Duration::from_secs(3),
            )
            .await
        });
        reached.await.unwrap();
        permit.test_retire();
        let _ = release.send(());
        let result = job.await.unwrap();
        if block == "COMMIT" {
            assert!(matches!(result, Outcome::Applied { .. }));
        } else {
            assert!(matches!(
                result,
                Outcome::RolledBack {
                    reason: Failure::Cancelled
                }
            ));
            assert!(!commands.lock().unwrap().contains(&"COMMIT".into()));
        }
        assert!(*cleaned.lock().unwrap());
    }
}

#[test]
fn empty_comment_postcondition_is_server_removal_without_rewriting_intent() {
    let (_, mut target, _, _) = setup();
    target.comment = Some("before".into());
    let empty = TableDdlIntent::SetComment {
        comment: Some(String::new()),
    };
    assert_eq!(post_target(&target, &empty).comment, None);
    assert_eq!(
        empty,
        TableDdlIntent::SetComment {
            comment: Some(String::new())
        }
    );
    let whitespace = TableDdlIntent::SetComment {
        comment: Some(" ".into()),
    };
    assert_eq!(post_target(&target, &whitespace).comment, Some(" ".into()));
}
