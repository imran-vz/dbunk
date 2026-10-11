use super::super::capture::drift_token;
use super::super::executor::{run, Canceller, Transport, BEGIN, LOCK_TIMEOUT, LOCK_TIMEOUT_LOCAL};
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
    observed: TableModel,
    fail: Vec<(String, SchemaEditFailure)>,
    cleaned: Arc<Mutex<Option<bool>>>,
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
    fn observe<'a>(
        &'a mut self,
        _: &'a str,
        _: &'a str,
        types: &'a [String],
    ) -> BoxFuture<'a, Result<(TableModel, Vec<String>), SchemaEditFailure>> {
        // The fake server keeps every type exactly as written.
        Box::pin(async move { Ok((self.observed.clone(), types.to_vec())) })
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
        fail: Vec::new(),
        cleaned: Default::default(),
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
            LOCK_TIMEOUT.to_owned(),
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
    assert_eq!(commands.last(), Some(&index_sql));
    let commit = commands.iter().position(|c| c == "COMMIT").unwrap();
    assert_eq!(
        commit + 1,
        commands.len() - 1,
        "the build runs after COMMIT"
    );

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
            reason: failure,
        }
    );

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
