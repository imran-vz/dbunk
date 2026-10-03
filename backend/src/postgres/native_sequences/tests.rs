use super::*;
use crate::backend::sequences::SequenceTarget;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;

type Reply = Result<Option<i64>, Failure>;
struct Mock {
    commands: Arc<Mutex<Vec<String>>>,
    parameters: Arc<Mutex<Vec<usize>>>,
    guarded: VecDeque<Reply>,
    fail: Option<(String, Failure)>,
    block: Option<(String, oneshot::Sender<()>, oneshot::Receiver<()>)>,
}
impl Mock {
    fn new(guarded: Vec<Reply>) -> Self {
        Self {
            commands: Default::default(),
            parameters: Default::default(),
            guarded: guarded.into(),
            fail: None,
            block: None,
        }
    }
    async fn record(&mut self, sql: &str) {
        self.commands.lock().unwrap().push(sql.into());
        if self.block.as_ref().is_some_and(|(block, ..)| block == sql) {
            let (_, reached, release) = self.block.take().unwrap();
            let _ = reached.send(());
            let _ = release.await;
        }
    }
}
impl Transport for Mock {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.record(sql).await;
            match &self.fail {
                Some((command, reason)) if command == sql => Err(reason.clone()),
                _ => Ok(()),
            }
        })
    }
    fn guarded<'a>(
        &'a mut self,
        sql: &'static str,
        parameters: &'a Parameters,
    ) -> BoxFuture<'a, Result<Option<i64>, Failure>> {
        Box::pin(async move {
            self.parameters
                .lock()
                .unwrap()
                .push(parameters.values().len());
            self.record(sql).await;
            self.guarded.pop_front().expect("unexpected guarded call")
        })
    }
    fn cleanup(self, _: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}
fn observation() -> Observation {
    Observation {
        target: SequenceTarget {
            database_oid: 1,
            database: "db".into(),
            namespace_oid: 2,
            schema: "owned\"字".into(),
            sequence_oid: 3,
            name: "ids".into(),
        },
        definition: SequenceDefinition {
            data_type: SequenceDataType::Bigint,
            start: 1,
            increment: 1,
            min_value: 1,
            max_value: 1000,
            cache: 1,
            cycle: false,
        },
        value: SequenceValue::Read {
            last_value: 10,
            is_called: true,
        },
        owned_by: None,
        identity: false,
    }
}
async fn run_mock(socket: Mock, permit: &WritePermit, intent: Intent) -> Execution {
    run(
        socket,
        permit,
        permit.test_cancellation(),
        intent,
        &observation(),
        Instant::now() + Duration::from_secs(5),
    )
    .await
}
fn database(code: &str) -> Failure {
    Failure::Database {
        code: Some(code.into()),
    }
}

#[test]
fn inspection_sql_never_advances_and_writes_are_oid_guarded() {
    for sql in [OBSERVE_SQL, GUARD_SQL] {
        assert!(!sql.contains("nextval") && !sql.contains("setval"));
    }
    assert!(ADVANCE_SQL.contains("pg_catalog.nextval(c.oid::pg_catalog.regclass)"));
    assert!(SET_SQL.contains(
        "setval(c.oid::pg_catalog.regclass, $13::pg_catalog.int8, $14::pg_catalog.bool)"
    ));
    for sql in [ADVANCE_SQL, SET_SQL, GUARD_SQL] {
        assert!(sql.contains("c.oid = $1") && sql.contains("s.seqcycle = $12"));
    }
    assert!(!ADVANCE_SQL.contains("$13"));
    assert_eq!(Parameters::new(&observation(), None).values().len(), 12);
    assert_eq!(
        Parameters::new(&observation(), Some((5, false)))
            .values()
            .len(),
        14
    );
}

#[test]
fn only_pre_effect_errors_are_reported_as_rejected() {
    for code in ["22003", "2200H", "25006", "42501", "42P01", "55P03"] {
        assert!(rejected_before_effect(&database(code)), "{code}");
    }
    for reason in [
        database("57014"),
        database("57P01"),
        database("XX000"),
        Failure::Database { code: None },
        Failure::Connection,
        Failure::Timeout,
        Failure::Cancelled,
    ] {
        assert!(!rejected_before_effect(&reason), "{reason:?}");
    }
}

#[tokio::test]
async fn advance_and_set_send_one_guarded_statement_and_report_returned_value() {
    for (intent, sql, parameters) in [
        (Intent::Advance, ADVANCE_SQL, 12),
        (
            Intent::Set {
                value: 5,
                is_called: false,
            },
            SET_SQL,
            14,
        ),
    ] {
        let socket = Mock::new(vec![Ok(Some(11))]);
        let commands = socket.commands.clone();
        let sizes = socket.parameters.clone();
        let permit = WritePermit::test_permit();
        let result = run_mock(socket, &permit, intent).await;
        assert_eq!(result.outcome, Outcome::Completed { returned: Some(11) });
        assert_eq!(*commands.lock().unwrap(), vec![LOCK_TIMEOUT_SQL, sql]);
        assert_eq!(*sizes.lock().unwrap(), vec![parameters]);
    }
}

#[tokio::test]
async fn stale_guard_reports_target_changed_without_effect() {
    let permit = WritePermit::test_permit();
    let result = run_mock(Mock::new(vec![Ok(None)]), &permit, Intent::Advance).await;
    assert_eq!(result.outcome, Outcome::TargetChanged);
}

#[tokio::test]
async fn dispatched_failures_are_rejected_or_unknown_and_never_retried() {
    for (reply, expected_unknown) in [
        (Err(database("22003")), false),
        (Err(database("57014")), true),
        (Err(Failure::Connection), true),
    ] {
        let socket = Mock::new(vec![reply]);
        let commands = socket.commands.clone();
        let permit = WritePermit::test_permit();
        let result = run_mock(
            socket,
            &permit,
            Intent::Set {
                value: 7,
                is_called: true,
            },
        )
        .await;
        assert_eq!(result.outcome.unknown(), expected_unknown);
        if !expected_unknown {
            assert!(matches!(result.outcome, Outcome::Rejected { .. }));
        }
        assert_eq!(
            commands
                .lock()
                .unwrap()
                .iter()
                .filter(|sql| *sql == SET_SQL)
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn cancelled_or_expired_before_dispatch_never_sends_the_effect() {
    for cancelled in [false, true] {
        let socket = Mock::new(vec![]);
        let commands = socket.commands.clone();
        let permit = WritePermit::test_permit();
        if cancelled {
            permit.test_cancel();
        }
        let result = run(
            socket,
            &permit,
            permit.test_cancellation(),
            Intent::Advance,
            &observation(),
            if cancelled {
                Instant::now() + Duration::from_secs(5)
            } else {
                Instant::now()
            },
        )
        .await;
        assert!(matches!(result.outcome, Outcome::NotDispatched { .. }));
        assert!(!commands
            .lock()
            .unwrap()
            .iter()
            .any(|sql| sql == ADVANCE_SQL));
    }
}

#[tokio::test]
async fn restart_rechecks_identity_around_alter_and_commits_once() {
    let alter = restart_sql(&observation().target, Some(500));
    assert_eq!(
        alter,
        "ALTER SEQUENCE \"owned\"\"字\".\"ids\" RESTART WITH 500"
    );
    let socket = Mock::new(vec![Ok(Some(1)), Ok(Some(1))]);
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    let result = run_mock(socket, &permit, Intent::Restart { with: Some(500) }).await;
    assert_eq!(result.outcome, Outcome::Completed { returned: None });
    assert_eq!(
        *commands.lock().unwrap(),
        vec![
            RESTART_BEGIN_SQL.to_owned(),
            GUARD_SQL.into(),
            alter,
            GUARD_SQL.into(),
            "COMMIT".into()
        ]
    );
}

#[tokio::test]
async fn restart_stale_or_failed_never_commits() {
    let alter = restart_sql(&observation().target, None);
    assert_eq!(alter, "ALTER SEQUENCE \"owned\"\"字\".\"ids\" RESTART");
    // Stale before ALTER: no ALTER sent.
    let socket = Mock::new(vec![Ok(None)]);
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    let result = run_mock(socket, &permit, Intent::Restart { with: None }).await;
    assert_eq!(result.outcome, Outcome::TargetChanged);
    assert!(!commands.lock().unwrap().contains(&alter));
    // Stale after ALTER: rolled back by closing without COMMIT.
    let socket = Mock::new(vec![Ok(Some(1)), Ok(None)]);
    let commands = socket.commands.clone();
    let result = run_mock(socket, &permit, Intent::Restart { with: None }).await;
    assert_eq!(result.outcome, Outcome::TargetChanged);
    assert!(!commands.lock().unwrap().iter().any(|sql| sql == "COMMIT"));
    // ALTER rejected: rolled back.
    let mut socket = Mock::new(vec![Ok(Some(1))]);
    socket.fail = Some((alter.clone(), database("22023")));
    let commands = socket.commands.clone();
    let result = run_mock(socket, &permit, Intent::Restart { with: None }).await;
    assert!(matches!(result.outcome, Outcome::RolledBack { .. }));
    assert!(!commands.lock().unwrap().iter().any(|sql| sql == "COMMIT"));
}

#[tokio::test]
async fn lost_commit_reply_is_unknown_and_not_retried() {
    let mut socket = Mock::new(vec![Ok(Some(1)), Ok(Some(1))]);
    socket.fail = Some(("COMMIT".into(), Failure::Connection));
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    let result = run_mock(socket, &permit, Intent::Restart { with: Some(3) }).await;
    assert_eq!(
        result.outcome,
        Outcome::OutcomeUnknown {
            reason: Failure::Connection
        }
    );
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|sql| *sql == "COMMIT")
            .count(),
        1
    );
}

#[tokio::test]
async fn admitted_call_wins_retirement_and_is_sent_once() {
    let (reached, ready) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let mut socket = Mock::new(vec![Ok(Some(42))]);
    socket.block = Some((ADVANCE_SQL.into(), reached, released));
    let commands = socket.commands.clone();
    let permit = Arc::new(WritePermit::test_permit());
    let owner = permit.clone();
    let task = tokio::spawn(async move { run_mock(socket, &owner, Intent::Advance).await });
    ready.await.unwrap();
    permit.test_retire();
    release.send(()).unwrap();
    assert_eq!(
        task.await.unwrap().outcome,
        Outcome::Completed { returned: Some(42) }
    );
    assert_eq!(
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|sql| *sql == ADVANCE_SQL)
            .count(),
        1
    );
}
