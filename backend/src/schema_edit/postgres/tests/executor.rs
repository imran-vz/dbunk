use super::super::capture::drift_token;
use super::super::executor::{
    annotate, run, Canceller, Transport, BEGIN, BEGIN_VERIFY, LOCK_TIMEOUT_LOCAL,
};
use super::{review, setup};
use crate::backend::table_ddl::WritePermit;
use crate::schema_edit::fixtures::existing;
use crate::schema_edit::*;
use futures_util::future::BoxFuture;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

const OID: u32 = 16384;
const LOCK: &str = r#"LOCK TABLE ONLY "public"."users" IN ACCESS EXCLUSIVE MODE"#;

struct Fake {
    commands: Arc<Mutex<Vec<String>>>,
    drift: Option<DriftToken>,
    relation: Option<u32>,
    /// What the in-transaction capture sees.
    observed: TableModel,
    /// What later captures see, after standalone statements ran.
    finished: TableModel,
    observations: usize,
    fail: Vec<(String, SchemaEditFailure)>,
    cleaned: Arc<Mutex<Option<bool>>>,
    /// What the invalid-index probe answers.
    invalid: Result<bool, SchemaEditFailure>,
}

impl Transport for Fake {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), SchemaEditFailure>> {
        Box::pin(async move {
            self.commands.lock().unwrap().push(sql.into());
            match self.fail.iter().find(|(s, _)| s == sql) {
                Some((_, failure)) => Err(failure.clone()),
                None => Ok(()),
            }
        })
    }
    fn drift<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<DriftToken>, SchemaEditFailure>> {
        Box::pin(async move { Ok(self.drift.clone()) })
    }
    fn relation<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<u32>, SchemaEditFailure>> {
        Box::pin(async move { Ok(self.relation) })
    }
    fn invalid_index<'a>(
        &'a mut self,
        schema: &'a str,
        index: &'a str,
        table: u32,
    ) -> BoxFuture<'a, Result<bool, SchemaEditFailure>> {
        Box::pin(async move {
            self.commands
                .lock()
                .unwrap()
                .push(format!("probe {schema}.{index} on {table}"));
            self.invalid.clone()
        })
    }
    fn observe<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a str,
        types: &'a [String],
    ) -> BoxFuture<'a, Result<(TableModel, Vec<String>), SchemaEditFailure>> {
        // The fake server keeps every type exactly as written.
        Box::pin(async move {
            self.observations += 1;
            let table = if self.observations == 1 {
                &self.observed
            } else {
                &self.finished
            };
            Ok((table.clone(), types.to_vec()))
        })
    }
    fn canceller(&self) -> Canceller {
        Arc::new(|| Box::pin(async {}))
    }
    fn cleanup(self, cancel: bool, _: Instant) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            *self.cleaned.lock().unwrap() = Some(cancel);
        })
    }
}

struct Case {
    baseline: SchemaBaseline,
    draft: SchemaDraft,
    preview: SchemaEditPreview,
}

fn case(edit: impl FnOnce(&mut SchemaDraft) -> PlanOptions) -> Case {
    let (mut baseline, _) = setup();
    baseline.drift = drift_token(OID, "abc");
    let mut draft = SchemaDraft::from_baseline(&baseline);
    let options = edit(&mut draft);
    let preview = review(&baseline, &draft, &options).expect("plans");
    Case {
        baseline,
        draft,
        preview,
    }
}

fn add_column() -> Case {
    case(|draft| {
        draft.add_column("nick", "text");
        PlanOptions::default()
    })
}

fn fake(case: &Case) -> Fake {
    Fake {
        commands: Default::default(),
        drift: Some(case.baseline.drift.clone()),
        relation: Some(OID),
        observed: case.draft.table.clone(),
        finished: case.draft.table.clone(),
        observations: 0,
        fail: Vec::new(),
        cleaned: Default::default(),
        invalid: Ok(false),
    }
}

async fn apply(fake: Fake, permit: &WritePermit, case: &Case) -> SchemaEditOutcome {
    run(
        fake,
        permit,
        permit.test_cancellation(),
        &case.baseline,
        &case.draft,
        &case.preview,
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
async fn locks_rechecks_drift_runs_verifies_and_commits_once() {
    let case = add_column();
    let fake = fake(&case);
    let commands = fake.commands.clone();
    let cleaned = fake.cleaned.clone();
    let permit = WritePermit::test_permit();
    let outcome = apply(fake, &permit, &case).await;
    assert!(
        matches!(outcome, SchemaEditOutcome::Applied { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        *commands.lock().unwrap(),
        vec![
            BEGIN.to_owned(),
            LOCK_TIMEOUT_LOCAL.to_owned(),
            LOCK.to_owned(),
            case.preview.statements[0].sql.clone(),
            "COMMIT".to_owned(),
        ]
    );
    assert_eq!(*cleaned.lock().unwrap(), Some(false));
}

#[tokio::test]
async fn a_drifted_or_missing_table_refuses_before_any_statement() {
    let case = add_column();
    for drift in [Some(drift_token(OID, "changed")), None] {
        let mut fake = fake(&case);
        fake.drift = drift;
        let commands = fake.commands.clone();
        let permit = WritePermit::test_permit();
        assert_eq!(
            apply(fake, &permit, &case).await,
            SchemaEditOutcome::NotDispatched {
                reason: SchemaEditFailure::BaselineChanged
            }
        );
        assert_eq!(sent(&commands, &case.preview.statements[0].sql), 0);
        assert_eq!(sent(&commands, "COMMIT"), 0);
    }
}

#[tokio::test]
async fn lock_timeout_and_a_vanished_table_are_not_dispatched() {
    let case = add_column();
    for (failure, reason) in [
        (
            SchemaEditFailure::LockTimeout,
            SchemaEditFailure::LockTimeout,
        ),
        (
            SchemaEditFailure::Database {
                code: Some("42P01".into()),
                message: "relation does not exist".into(),
            },
            SchemaEditFailure::BaselineChanged,
        ),
    ] {
        let mut fake = fake(&case);
        fake.fail.push((LOCK.into(), failure));
        let commands = fake.commands.clone();
        let permit = WritePermit::test_permit();
        assert_eq!(
            apply(fake, &permit, &case).await,
            SchemaEditOutcome::NotDispatched { reason }
        );
        assert_eq!(sent(&commands, &case.preview.statements[0].sql), 0);
    }
}

#[tokio::test]
async fn a_rejected_statement_rolls_back() {
    let case = add_column();
    let mut fake = fake(&case);
    let failure = SchemaEditFailure::Database {
        code: Some("42701".into()),
        message: "column \"nick\" already exists".into(),
    };
    fake.fail
        .push((case.preview.statements[0].sql.clone(), failure.clone()));
    let commands = fake.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(fake, &permit, &case).await,
        SchemaEditOutcome::RolledBack { reason: failure }
    );
    assert_eq!(sent(&commands, "ROLLBACK"), 1);
    assert_eq!(sent(&commands, "COMMIT"), 0);
}

#[tokio::test]
async fn a_structure_that_does_not_match_the_draft_rolls_back() {
    let case = add_column();
    let mut fake = fake(&case);
    fake.observed.columns.pop();
    let commands = fake.commands.clone();
    let permit = WritePermit::test_permit();
    let outcome = apply(fake, &permit, &case).await;
    assert!(
        matches!(
            outcome,
            SchemaEditOutcome::RolledBack {
                reason: SchemaEditFailure::VerificationFailed { .. }
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(sent(&commands, "ROLLBACK"), 1);
    assert_eq!(sent(&commands, "COMMIT"), 0);
}

#[tokio::test]
async fn a_lost_commit_or_rollback_reply_is_unknown() {
    let case = add_column();
    let mut lost_commit = fake(&case);
    lost_commit
        .fail
        .push(("COMMIT".into(), SchemaEditFailure::Connection));
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(lost_commit, &permit, &case).await,
        SchemaEditOutcome::OutcomeUnknown {
            committed: 0,
            reason: SchemaEditFailure::Connection
        }
    );

    let mut lost_rollback = fake(&case);
    lost_rollback.observed.columns.pop();
    lost_rollback
        .fail
        .push(("ROLLBACK".into(), SchemaEditFailure::Connection));
    let cleaned = lost_rollback.cleaned.clone();
    let permit = WritePermit::test_permit();
    let outcome = apply(lost_rollback, &permit, &case).await;
    assert!(
        matches!(outcome, SchemaEditOutcome::OutcomeUnknown { .. }),
        "{outcome:?}"
    );
    assert_eq!(*cleaned.lock().unwrap(), Some(true));
}

fn with_concurrent_index() -> Case {
    case(|draft| {
        draft.add_column("nick", "text");
        let origin = draft.new_origin();
        draft.table.indexes.push(IndexModel {
            origin,
            name: "users_email_idx".into(),
            unique: true,
            method: None,
            members: vec![IndexMember::column(existing(2))],
            include: Vec::new(),
            predicate: None,
        });
        PlanOptions {
            concurrent_indexes: vec![origin],
            ..PlanOptions::default()
        }
    })
}

#[tokio::test]
async fn concurrent_builds_run_after_commit_and_report_the_committed_prefix() {
    let case = with_concurrent_index();
    assert_eq!(case.preview.statements.len(), 2);
    let index_sql = case.preview.statements[1].sql.clone();
    assert!(index_sql.starts_with("CREATE UNIQUE INDEX CONCURRENTLY"));

    let mut observed = case.draft.table.clone();
    observed.indexes.pop();

    let mut ok = fake(&case);
    ok.observed = observed.clone();
    let commands = ok.commands.clone();
    let permit = WritePermit::test_permit();
    let outcome = apply(ok, &permit, &case).await;
    assert!(
        matches!(outcome, SchemaEditOutcome::Applied { .. }),
        "{outcome:?}"
    );
    let commands = commands.lock().unwrap().clone();
    let commit = commands.iter().position(|c| c == "COMMIT").unwrap();
    assert_eq!(
        commands[commit + 1],
        index_sql,
        "the build runs after COMMIT"
    );
    assert_eq!(&commands[commit + 2..], [BEGIN_VERIFY, "ROLLBACK"]);

    let failure = SchemaEditFailure::Database {
        code: Some("23505".into()),
        message: "could not create unique index".into(),
    };
    let mut rejected = fake(&case);
    rejected.observed = observed.clone();
    rejected.fail.push((index_sql.clone(), failure.clone()));
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(rejected, &permit, &case).await,
        SchemaEditOutcome::Stopped {
            committed: 1,
            failed_statement: 1,
            reason: failure.clone(),
        }
    );

    let probe = format!("probe public.users_email_idx on {OID}");
    let drop = r#"DROP INDEX CONCURRENTLY IF EXISTS "public"."users_email_idx""#;

    // A failed build that left no index reports the server error as is.
    let mut clean = fake(&case);
    clean.observed = observed.clone();
    clean.fail.push((index_sql.clone(), failure.clone()));
    let commands = clean.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(clean, &permit, &case).await,
        SchemaEditOutcome::Stopped {
            committed: 1,
            failed_statement: 1,
            reason: failure.clone(),
        }
    );
    assert_eq!(sent(&commands, &probe), 1);
    assert_eq!(sent(&commands, drop), 0);

    // A leftover invalid index is dropped, and the reason says so.
    let mut leftover = fake(&case);
    leftover.observed = observed.clone();
    leftover.fail.push((index_sql.clone(), failure.clone()));
    leftover.invalid = Ok(true);
    let commands = leftover.commands.clone();
    let permit = WritePermit::test_permit();
    let outcome = apply(leftover, &permit, &case).await;
    assert_eq!(sent(&commands, drop), 1);
    match outcome {
        SchemaEditOutcome::Stopped {
            committed: 1,
            failed_statement: 1,
            reason: SchemaEditFailure::Database { code, message },
        } => {
            assert_eq!(code.as_deref(), Some("23505"));
            assert!(
                message.starts_with("could not create unique index"),
                "{message}"
            );
            assert!(message.contains("was dropped"), "{message}");
        }
        other => panic!("{other:?}"),
    }

    // A cleanup that fails, or a probe that cannot answer, stays Stopped
    // and says the invalid index may remain.
    for (invalid, drop_fails) in [
        (Ok(true), true),
        (Err(SchemaEditFailure::Connection), false),
    ] {
        let mut stuck = fake(&case);
        stuck.observed = observed.clone();
        stuck
            .fail
            .push((index_sql.clone(), SchemaEditFailure::LockTimeout));
        if drop_fails {
            stuck
                .fail
                .push((drop.into(), SchemaEditFailure::LockTimeout));
        }
        stuck.invalid = invalid;
        let permit = WritePermit::test_permit();
        match apply(stuck, &permit, &case).await {
            SchemaEditOutcome::Stopped {
                committed: 1,
                failed_statement: 1,
                reason: SchemaEditFailure::Database { code, message },
            } => {
                assert_eq!(code.as_deref(), Some("55P03"));
                assert!(
                    message.contains("may have left the invalid index"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    let mut moved = fake(&case);
    moved.observed = observed.clone();
    moved.relation = Some(OID + 1);
    let commands = moved.commands.clone();
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(moved, &permit, &case).await,
        SchemaEditOutcome::Stopped {
            committed: 1,
            failed_statement: 1,
            reason: SchemaEditFailure::BaselineChanged,
        }
    );
    assert_eq!(sent(&commands, &index_sql), 0);

    let mut lost = fake(&case);
    lost.observed = observed;
    lost.fail.push((index_sql, SchemaEditFailure::Connection));
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(lost, &permit, &case).await,
        SchemaEditOutcome::OutcomeUnknown {
            committed: 1,
            reason: SchemaEditFailure::Connection
        }
    );
}

#[tokio::test]
async fn a_tampered_preview_or_cancelled_permit_sends_nothing() {
    let mut case = add_column();
    case.preview.statements[0].sql =
        "ALTER TABLE \"public\".\"users\" DROP COLUMN \"email\";".into();
    let fake_socket = fake(&case);
    let commands = fake_socket.commands.clone();
    let permit = WritePermit::test_permit();
    assert!(matches!(
        apply(fake_socket, &permit, &case).await,
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Unsupported { .. }
        }
    ));
    assert!(commands.lock().unwrap().is_empty());

    let case = add_column();
    let fake_socket = fake(&case);
    let commands = fake_socket.commands.clone();
    let permit = WritePermit::test_permit();
    permit.test_cancel();
    assert_eq!(
        apply(fake_socket, &permit, &case).await,
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::Cancelled
        }
    );
    assert_eq!(sent(&commands, &case.preview.statements[0].sql), 0);
}

#[tokio::test]
async fn the_lock_timeout_is_transaction_local_and_absent_without_an_atomic_group() {
    // Only the atomic group lowers lock_timeout, inside its transaction.
    let grouped = with_concurrent_index();
    let mut socket = fake(&grouped);
    let mut observed = grouped.draft.table.clone();
    observed.indexes.pop();
    socket.observed = observed;
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    assert!(apply(socket, &permit, &grouped).await.applied());
    let commands = commands.lock().unwrap().clone();
    let begin = commands.iter().position(|c| c == BEGIN).unwrap();
    assert_eq!(commands[begin + 1], LOCK_TIMEOUT_LOCAL);
    assert_eq!(sent_in(&commands, LOCK_TIMEOUT_LOCAL), 1);
    assert!(LOCK_TIMEOUT_LOCAL.ends_with(", true)"), "transaction-local");
    assert!(
        LOCK_TIMEOUT_LOCAL.contains("GREATEST("),
        "a sub-millisecond inherited timeout never rounds to 0"
    );

    // A concurrent build alone runs under the inherited setting.
    let only = case(|draft| {
        let origin = draft.new_origin();
        draft.table.indexes.push(IndexModel {
            origin,
            name: "users_email_idx".into(),
            unique: false,
            method: None,
            members: vec![IndexMember::column(existing(2))],
            include: Vec::new(),
            predicate: None,
        });
        PlanOptions {
            concurrent_indexes: vec![origin],
            ..PlanOptions::default()
        }
    });
    // No atomic group, so the only capture is the final verification.
    let socket = fake(&only);
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    assert!(apply(socket, &permit, &only).await.applied());
    let commands = commands.lock().unwrap().clone();
    assert_eq!(
        commands,
        vec![
            only.preview.statements[0].sql.clone(),
            BEGIN_VERIFY.to_owned(),
            "ROLLBACK".to_owned()
        ]
    );
}

#[tokio::test]
async fn standalone_statements_are_verified_after_they_commit() {
    let case = with_concurrent_index();
    let mut observed = case.draft.table.clone();
    observed.indexes.pop();

    // The build reports success, but the index is not there.
    let mut missing = fake(&case);
    missing.observed = observed.clone();
    missing.finished = observed.clone();
    let commands = missing.commands.clone();
    let permit = WritePermit::test_permit();
    match apply(missing, &permit, &case).await {
        SchemaEditOutcome::OutcomeUnknown {
            committed: 2,
            reason: SchemaEditFailure::VerificationFailed { .. },
        } => {}
        other => panic!("expected an unverified outcome, got {other:?}"),
    }
    let commands = commands.lock().unwrap().clone();
    let index = commands
        .iter()
        .position(|c| *c == case.preview.statements[1].sql)
        .unwrap();
    assert_eq!(commands[index + 1], BEGIN_VERIFY);
    assert_eq!(commands.last().map(String::as_str), Some("ROLLBACK"));

    // A verification read that fails leaves the outcome unknown too.
    let mut unread = fake(&case);
    unread.observed = observed;
    unread
        .fail
        .push((BEGIN_VERIFY.into(), SchemaEditFailure::Connection));
    let permit = WritePermit::test_permit();
    assert_eq!(
        apply(unread, &permit, &case).await,
        SchemaEditOutcome::OutcomeUnknown {
            committed: 2,
            reason: SchemaEditFailure::Connection
        }
    );

    // An atomic-only plan is verified once, inside its transaction.
    let plain = add_column();
    let socket = fake(&plain);
    let commands = socket.commands.clone();
    let permit = WritePermit::test_permit();
    assert!(apply(socket, &permit, &plain).await.applied());
    assert_eq!(sent(&commands, BEGIN_VERIFY), 0);
}

fn sent_in(commands: &[String], sql: &str) -> usize {
    commands.iter().filter(|command| *command == sql).count()
}

#[test]
fn annotating_a_reason_keeps_its_code() {
    assert_eq!(
        annotate(SchemaEditFailure::LockTimeout, "note"),
        SchemaEditFailure::Database {
            code: Some("55P03".into()),
            message: "Timed out waiting for a lock on the table; note".into(),
        }
    );
    assert_eq!(
        annotate(
            SchemaEditFailure::Database {
                code: Some("23505".into()),
                message: "duplicate".into(),
            },
            "note"
        ),
        SchemaEditFailure::Database {
            code: Some("23505".into()),
            message: "duplicate; note".into(),
        }
    );
    assert_eq!(
        annotate(SchemaEditFailure::Cancelled, "note"),
        SchemaEditFailure::Database {
            code: None,
            message: "Cancelled; note".into(),
        }
    );
}
