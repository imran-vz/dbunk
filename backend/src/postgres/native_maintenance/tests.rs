use super::*;
use crate::backend::maintenance::{preview, MaintenanceIntent as Intent};
use std::sync::Mutex;
use tokio::sync::oneshot;
struct Mock {
    commands: Arc<Mutex<Vec<String>>>,
    current: bool,
    matches_seen: usize,
    stale_after_lock: bool,
    cancel_at: Option<(String, Arc<WritePermit>)>,
    block: String,
    reached: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    fail: Option<(String, Failure)>,
    cleanup: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
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
            if let Some((command, permit)) = &self.cancel_at {
                if command == sql {
                    permit.test_cancel();
                }
            }
            match &self.fail {
                Some((command, reason)) if sql == command => Err(reason.clone()),
                _ => Ok(()),
            }
        })
    }
    fn matches<'a>(&'a mut self, _: &'a Target) -> BoxFuture<'a, Result<bool, Failure>> {
        Box::pin(async move {
            self.matches_seen += 1;
            if let Some((command, permit)) = &self.cancel_at {
                if command == "match" {
                    permit.test_cancel();
                }
            }
            Ok(self.current && !(self.stale_after_lock && self.matches_seen > 1))
        })
    }
    fn cleanup(self, _: bool, _: Instant) -> BoxFuture<'static, Diagnostics> {
        Box::pin(async move {
            if let Some((started, release)) = self.cleanup {
                let _ = started.send(());
                let _ = release.await;
            }
            Diagnostics::default()
        })
    }
}
fn mock(block: &str) -> (Mock, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (reached, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    (
        Mock {
            commands: Default::default(),
            current: true,
            matches_seen: 0,
            stale_after_lock: false,
            cancel_at: None,
            block: block.into(),
            reached: Some(reached),
            release: Some(released),
            fail: None,
            cleanup: None,
        },
        ready,
        release,
    )
}
fn target(kind: Kind) -> Target {
    Target {
        database_oid: 1,
        database: "db".into(),
        namespace_oid: 2,
        schema: "owned".into(),
        relation_oid: 3,
        name: "target".into(),
        kind,
    }
}
fn reviewed(intent: Intent, kind: Kind) -> Preview {
    preview(&target(kind), intent, Some(1234)).unwrap()
}
async fn execute_mock(
    socket: Mock,
    permit: &WritePermit,
    preview: &Preview,
    kind: Kind,
) -> Execution {
    run(
        socket,
        permit,
        permit.test_cancellation(),
        &target(kind),
        preview,
        Instant::now() + Duration::from_secs(5),
    )
    .await
}
#[tokio::test]
async fn cancelled_or_expired_before_dispatch_never_sends_maintenance() {
    for cancelled in [false, true] {
        let (socket, _, _) = mock("");
        let commands = socket.commands.clone();
        let permit = WritePermit::test_permit();
        if cancelled {
            permit.test_cancel();
        }
        let result = run(
            socket,
            &permit,
            permit.test_cancellation(),
            &target(Kind::Table),
            &reviewed(Intent::Vacuum, Kind::Table),
            if cancelled {
                Instant::now() + Duration::from_secs(5)
            } else {
                Instant::now()
            },
        )
        .await;
        assert!(matches!(result.outcome, Outcome::NotDispatched { .. }));
        assert!(commands.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn stale_target_never_executes_the_utility() {
    for intent in [Intent::Vacuum, Intent::ReindexTable] {
        let (mut socket, _, _) = mock("");
        socket.current = false;
        let commands = socket.commands.clone();
        let permit = WritePermit::test_permit();
        let result =
            execute_mock(socket, &permit, &reviewed(intent, Kind::Table), Kind::Table).await;
        assert_eq!(result.outcome, Outcome::TargetChanged);
        assert_eq!(commands.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn cancellation_during_transaction_rolls_back_and_joins_cleanup_without_commit() {
    let preview = reviewed(Intent::ReindexTable, Kind::Table);
    let (mut socket, ready, release) = mock(&preview.sql);
    let commands = socket.commands.clone();
    let (cleaning, cleanup_started) = oneshot::channel();
    let (finish, finished) = oneshot::channel();
    socket.cleanup = Some((cleaning, finished));
    let permit = Arc::new(WritePermit::test_permit());
    let owner = permit.clone();
    let task =
        tokio::spawn(async move { execute_mock(socket, &owner, &preview, Kind::Table).await });
    ready.await.unwrap();
    permit.test_cancel();
    cleanup_started.await.unwrap();
    assert!(!task.is_finished());
    assert!(!commands.lock().unwrap().iter().any(|sql| sql == "COMMIT"));
    let _ = release.send(());
    finish.send(()).unwrap();
    assert_eq!(
        task.await.unwrap().outcome,
        Outcome::RolledBack {
            reason: Failure::Cancelled
        }
    );
}
#[tokio::test]
async fn known_commit_or_standalone_success_wins_retirement_and_is_sent_only_once() {
    for intent in [Intent::Vacuum, Intent::ReindexTable] {
        let preview = reviewed(intent, Kind::Table);
        let command = if intent == Intent::Vacuum {
            preview.sql.clone()
        } else {
            "COMMIT".into()
        };
        let (socket, ready, release) = mock(&command);
        let commands = socket.commands.clone();
        let permit = Arc::new(WritePermit::test_permit());
        let owner = permit.clone();
        let task =
            tokio::spawn(async move { execute_mock(socket, &owner, &preview, Kind::Table).await });
        ready.await.unwrap();
        permit.test_retire();
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().outcome, Outcome::Completed);
        assert_eq!(
            commands
                .lock()
                .unwrap()
                .iter()
                .filter(|sql| **sql == command)
                .count(),
            1
        );
    }
}
#[tokio::test]
async fn errors_distinguish_transaction_rollback_partial_effects_and_unknown_commit() {
    for (intent, kind, failed_commit, expected) in [
        (Intent::Vacuum, Kind::Table, false, 0),
        (Intent::Analyze, Kind::Table, false, 0),
        (Intent::ReindexTable, Kind::PartitionedTable, false, 0),
        (Intent::ReindexTable, Kind::Table, false, 1),
        (Intent::ReindexTable, Kind::Table, true, 2),
    ] {
        let preview = reviewed(intent, kind);
        let (mut socket, _, _) = mock("");
        socket.fail = Some((
            if failed_commit {
                "COMMIT".into()
            } else {
                preview.sql.clone()
            },
            Failure::Database {
                code: Some("57014".into()),
            },
        ));
        let permit = WritePermit::test_permit();
        let result = execute_mock(socket, &permit, &preview, kind).await;
        assert!(matches!(
            (&result.outcome, expected),
            (Outcome::InterruptedEffectsPossible { .. }, 0)
                | (Outcome::RolledBack { .. }, 1)
                | (Outcome::OutcomeUnknown { .. }, 2)
        ));
    }
}
#[tokio::test]
async fn lost_standalone_reply_is_unknown_and_partitioned_reindex_never_begins_transaction() {
    let preview = reviewed(Intent::ReindexTable, Kind::PartitionedTable);
    let (mut socket, _, _) = mock("");
    let commands = socket.commands.clone();
    socket.fail = Some((preview.sql.clone(), Failure::Connection));
    let permit = WritePermit::test_permit();
    assert_eq!(
        execute_mock(socket, &permit, &preview, Kind::PartitionedTable)
            .await
            .outcome,
        Outcome::OutcomeUnknown {
            reason: Failure::Connection
        }
    );
    assert!(!commands
        .lock()
        .unwrap()
        .iter()
        .any(|sql| sql.contains("BEGIN") || sql == "COMMIT" || sql.starts_with("LOCK")));
}
#[tokio::test]
async fn both_refresh_modes_use_transaction_without_invalid_lock_table_on_matview() {
    for concurrently in [false, true] {
        let preview = reviewed(
            Intent::RefreshMaterializedView { concurrently },
            Kind::MaterializedView,
        );
        let (socket, _, _) = mock("");
        let commands = socket.commands.clone();
        let permit = WritePermit::test_permit();
        assert_eq!(
            execute_mock(socket, &permit, &preview, Kind::MaterializedView)
                .await
                .outcome,
            Outcome::Completed
        );
        let commands = commands.lock().unwrap();
        assert!(commands[0].starts_with("BEGIN"));
        assert_eq!(commands.last().unwrap(), "COMMIT");
        assert!(!commands.iter().any(|sql| sql.starts_with("LOCK")));
    }
}
#[tokio::test]
async fn unsettled_standalone_cancellation_is_unknown_and_cleanup_is_joined() {
    let preview = reviewed(Intent::Vacuum, Kind::Table);
    let (socket, ready, _release) = mock(&preview.sql);
    let permit = Arc::new(WritePermit::test_permit());
    let owner = permit.clone();
    let task =
        tokio::spawn(async move { execute_mock(socket, &owner, &preview, Kind::Table).await });
    ready.await.unwrap();
    permit.test_cancel();
    assert_eq!(
        task.await.unwrap().outcome,
        Outcome::OutcomeUnknown {
            reason: Failure::Cancelled
        }
    );
}

#[tokio::test]
async fn revalidation_after_relation_lock_refuses_replacement() {
    let preview = reviewed(Intent::ReindexTable, Kind::Table);
    let (mut socket, _, _) = mock("");
    socket.stale_after_lock = true;
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        execute_mock(socket, &permit, &preview, Kind::Table)
            .await
            .outcome,
        Outcome::TargetChanged
    );
    assert!(!commands.lock().unwrap().contains(&preview.sql));
}
#[tokio::test]
async fn cancellation_after_preflight_or_last_statement_wins_atomic_effect_fence() {
    for intent in [Intent::Vacuum, Intent::ReindexTable] {
        let preview = reviewed(intent, Kind::Table);
        let (mut socket, _, _) = mock("");
        let permit = Arc::new(WritePermit::test_permit());
        socket.cancel_at = Some((
            if intent == Intent::Vacuum {
                "match".into()
            } else {
                preview.sql.clone()
            },
            permit.clone(),
        ));
        let commands = socket.commands.clone();
        let outcome = execute_mock(socket, &permit, &preview, Kind::Table)
            .await
            .outcome;
        if intent == Intent::Vacuum {
            assert_eq!(
                outcome,
                Outcome::NotDispatched {
                    reason: Failure::Cancelled
                }
            );
            assert!(!commands.lock().unwrap().contains(&preview.sql));
        } else {
            assert_eq!(
                outcome,
                Outcome::RolledBack {
                    reason: Failure::Cancelled
                }
            );
            assert!(!commands.lock().unwrap().iter().any(|s| s == "COMMIT"));
        }
    }
}
