use super::*;
use crate::backend::schema_alter::SchemaIdentity;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
struct Mock {
    commands: Arc<Mutex<Vec<String>>>,
    captures: VecDeque<SchemaAlterDescription>,
    requests: Arc<Mutex<Vec<SchemaAlterRequest>>>,
    owned: bool,
    fail: Option<&'static str>,
    block: &'static str,
    reached: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
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
            if self.fail == Some(sql) {
                Err(Failure::Connection)
            } else {
                Ok(())
            }
        })
    }
    fn capture<'a>(
        &'a mut self,
        request: &'a SchemaAlterRequest,
    ) -> BoxFuture<'a, Result<SchemaAlterDescription, Failure>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request.clone());
            self.captures.pop_front().ok_or(Failure::TargetChanged)
        })
    }
    fn owned_version(&mut self, _: u32) -> BoxFuture<'_, Result<bool, Failure>> {
        Box::pin(async move { Ok(self.owned) })
    }
    fn cleanup(self, cancel: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            *self.cleaned.lock().unwrap() = Some(cancel);
        })
    }
}
fn description() -> SchemaAlterDescription {
    SchemaAlterDescription {
        identity: SchemaIdentity {
            database_oid: 1,
            schema_oid: 2,
        },
        schema: "a".into(),
        namespace_xmin: "9".into(),
        namespace_ctid: "(0,1)".into(),
        comment: Some("old".into()),
    }
}
fn setup(
    intent: SchemaAlterIntent,
) -> (
    Mock,
    SchemaAlterDescription,
    SchemaAlterIntent,
    SchemaAlterPreview,
) {
    let target = description();
    let mut after = post_target(&target, &intent);
    if matches!(intent, SchemaAlterIntent::Rename { .. }) {
        // The rewritten row has this transaction's version, not the observed one.
        after.namespace_xmin = "77".into();
        after.namespace_ctid = "(0,9)".into();
    }
    let mock = Mock {
        commands: Default::default(),
        captures: VecDeque::from([target.clone(), after]),
        requests: Default::default(),
        owned: true,
        fail: None,
        block: "",
        reached: None,
        release: None,
        cleaned: Default::default(),
    };
    let preview = SchemaAlterPreview {
        sql: "ALTER SCHEMA a RENAME TO c".into(),
        summary: "rename".into(),
        statement_timeout_ms: None,
        operation_timeout_ms: SCHEMA_ALTER_OPERATION_TIMEOUT_MS,
    };
    (mock, target, intent, preview)
}
fn rename() -> SchemaAlterIntent {
    SchemaAlterIntent::Rename {
        new_name: "c".into(),
    }
}
fn comment() -> SchemaAlterIntent {
    SchemaAlterIntent::SetComment {
        comment: Some("new".into()),
    }
}
async fn execute_mock(
    mock: Mock,
    target: &SchemaAlterDescription,
    intent: &SchemaAlterIntent,
    preview: &SchemaAlterPreview,
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
async fn schema_alter_commits_only_after_exact_pre_and_post_identity_guards() {
    for intent in [rename(), comment()] {
        let (mock, target, intent, preview) = setup(intent);
        let commands = mock.commands.clone();
        let requests = mock.requests.clone();
        let cleaned = mock.cleaned.clone();
        assert!(matches!(
            execute_mock(mock, &target, &intent, &preview).await,
            Outcome::Applied { .. }
        ));
        let commands = commands.lock().unwrap();
        assert_eq!(
            commands.first().unwrap(),
            "BEGIN ISOLATION LEVEL READ COMMITTED"
        );
        assert_eq!(&commands[2], &preview.sql);
        assert_eq!(commands.last().unwrap(), "COMMIT");
        // Both captures pin the observed OID; the post capture uses the new name.
        let requests = requests.lock().unwrap();
        assert!(requests.iter().all(|r| r.expected == Some(target.identity)));
        assert_eq!(requests[0].schema, "a");
        let after = if matches!(intent, SchemaAlterIntent::Rename { .. }) {
            "c"
        } else {
            "a"
        };
        assert_eq!(requests[1].schema, after);
        assert_eq!(*cleaned.lock().unwrap(), Some(false));
    }
}
#[tokio::test]
async fn stale_observation_refuses_before_dispatch_with_no_statement_sent() {
    for defect in 0..4 {
        let (mut mock, target, intent, preview) = setup(rename());
        let changed = &mut mock.captures[0];
        match defect {
            0 => changed.namespace_xmin = "10".into(),
            1 => changed.namespace_ctid = "(0,2)".into(),
            2 => changed.comment = None,
            _ => changed.identity.schema_oid = 3,
        }
        let commands = mock.commands.clone();
        let cleaned = mock.cleaned.clone();
        assert!(matches!(
            execute_mock(mock, &target, &intent, &preview).await,
            Outcome::NotDispatched {
                reason: Failure::TargetChanged
            }
        ));
        let commands = commands.lock().unwrap();
        assert!(!commands.contains(&preview.sql), "defect {defect}");
        assert!(!commands.contains(&"COMMIT".into()));
        assert!(cleaned.lock().unwrap().is_some());
    }
    // A missing schema (dropped or renamed away) is also a stale identity.
    let (mut mock, target, intent, preview) = setup(comment());
    mock.captures.clear();
    let commands = mock.commands.clone();
    assert!(matches!(
        execute_mock(mock, &target, &intent, &preview).await,
        Outcome::NotDispatched {
            reason: Failure::TargetChanged
        }
    ));
    assert!(!commands.lock().unwrap().contains(&preview.sql));
}
#[tokio::test]
async fn concurrent_change_after_statement_rolls_back_instead_of_committing() {
    // Rename: a row version written by another transaction, a different OID now
    // carrying the name, or a changed comment all refuse COMMIT.
    for defect in 0..3 {
        let (mut mock, target, intent, preview) = setup(rename());
        match defect {
            0 => mock.owned = false,
            1 => mock.captures[1].identity.schema_oid = 3,
            _ => mock.captures[1].comment = None,
        }
        let commands = mock.commands.clone();
        assert!(matches!(
            execute_mock(mock, &target, &intent, &preview).await,
            Outcome::RolledBack {
                reason: Failure::TargetChanged
            }
        ));
        let commands = commands.lock().unwrap();
        assert!(commands.contains(&"ROLLBACK".into()), "defect {defect}");
        assert!(!commands.contains(&"COMMIT".into()));
    }
    // Comment: the namespace row must remain the observed version, with the
    // intended comment. Names returning to old values still differ by ctid.
    for defect in 0..2 {
        let (mut mock, target, intent, preview) = setup(comment());
        match defect {
            0 => mock.captures[1].namespace_ctid = "(0,2)".into(),
            _ => mock.captures[1].comment = Some("other".into()),
        }
        let commands = mock.commands.clone();
        assert!(matches!(
            execute_mock(mock, &target, &intent, &preview).await,
            Outcome::RolledBack {
                reason: Failure::TargetChanged
            }
        ));
        assert!(!commands.lock().unwrap().contains(&"COMMIT".into()));
    }
}
#[tokio::test]
async fn lost_rollback_and_commit_replies_are_unknown_never_retried() {
    for command in ["ROLLBACK", "COMMIT"] {
        let (mut mock, target, intent, preview) = setup(rename());
        mock.fail = Some(command);
        if command == "ROLLBACK" {
            mock.owned = false;
        }
        let commands = mock.commands.clone();
        let cleaned = mock.cleaned.clone();
        assert!(matches!(
            execute_mock(mock, &target, &intent, &preview).await,
            Outcome::OutcomeUnknown { .. }
        ));
        let sent = commands.lock().unwrap();
        assert_eq!(sent.iter().filter(|c| **c == preview.sql).count(), 1);
        assert_eq!(sent.iter().filter(|c| *c == command).count(), 1);
        assert_eq!(*cleaned.lock().unwrap(), Some(true));
    }
}
#[tokio::test]
async fn cancellation_before_commit_rolls_back_and_known_commit_wins_retirement() {
    for block in ["ALTER SCHEMA a RENAME TO c", "COMMIT"] {
        let (mut mock, target, intent, preview) = setup(rename());
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
        assert!(cleaned.lock().unwrap().is_some());
    }
}
#[tokio::test]
async fn retired_permit_before_dispatch_never_sends_the_statement() {
    let (mock, target, intent, preview) = setup(comment());
    let commands = mock.commands.clone();
    let permit = WritePermit::test_permit();
    permit.test_retire();
    let result = run(
        mock,
        &permit,
        permit.test_cancellation(),
        &target,
        &intent,
        &preview,
        Instant::now() + Duration::from_secs(3),
    )
    .await;
    assert!(matches!(
        result,
        Outcome::NotDispatched {
            reason: Failure::Cancelled
        }
    ));
    assert!(!commands.lock().unwrap().contains(&preview.sql));
}
#[test]
fn empty_comment_postcondition_is_server_removal_and_rename_keeps_identity() {
    let target = description();
    let empty = SchemaAlterIntent::SetComment {
        comment: Some(String::new()),
    };
    assert_eq!(post_target(&target, &empty).comment, None);
    let whitespace = SchemaAlterIntent::SetComment {
        comment: Some(" ".into()),
    };
    assert_eq!(post_target(&target, &whitespace).comment, Some(" ".into()));
    let renamed = post_target(&target, &rename());
    assert_eq!(renamed.identity, target.identity);
    assert_eq!(renamed.schema, "c");
    assert_eq!(renamed.comment, target.comment);
}
