use super::*;
use crate::backend::profile;

fn observation() -> SequenceObservation {
    SequenceObservation {
        target: SequenceTarget {
            database_oid: 1,
            database: "owned".into(),
            namespace_oid: 2,
            schema: "owned\"schema".into(),
            sequence_oid: 3,
            name: "multi字\"seq".into(),
        },
        definition: SequenceDefinition {
            data_type: SequenceDataType::Integer,
            start: 1,
            increment: 1,
            min_value: 1,
            max_value: 100,
            cache: 1,
            cycle: false,
        },
        value: SequenceValue::Read {
            last_value: 50,
            is_called: true,
        },
        owned_by: Some("\"owned\"\"schema\".t.id".into()),
        identity: false,
    }
}
fn observed(document: &DataDocument) -> ObservedSequence {
    ObservedSequence {
        document: document.clone(),
        observation: observation(),
        statement_timeout_ms: None,
    }
}
async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("sequence-test", "one", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
async fn policy(backend: &Backend, readonly: bool, mode: crate::SafeMode) {
    let mut connection =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut connection else {
        panic!("postgres")
    };
    pg.read_only = readonly;
    pg.safe_mode = mode;
    crate::storage::upsert_connection(&backend.0.state.pool, &connection)
        .await
        .unwrap();
}
fn outcome(outcome: SequenceOutcome) -> native_sequences::Execution {
    native_sequences::Execution {
        outcome,
        runtime_ms: 1,
    }
}
const INTENTS: [SequenceIntent; 3] = [
    SequenceIntent::Advance,
    SequenceIntent::Set {
        value: 10,
        is_called: true,
    },
    SequenceIntent::Restart { with: Some(5) },
];

#[test]
fn previews_are_exact_quoted_and_bind_the_observed_identity() {
    let observation = observation();
    let advance = preview(&observation, SequenceIntent::Advance, Some(1234)).unwrap();
    assert_eq!(
        advance.summary,
        "SELECT pg_catalog.nextval(E'\"owned\"\"schema\".\"multi字\"\"seq\"'::pg_catalog.regclass)"
    );
    assert_eq!(advance.sql, format!("{LOCK_TIMEOUT_SQL}\n{ADVANCE_SQL}"));
    assert_eq!(advance.parameters.len(), 12);
    assert_eq!(advance.parameters[0], "3 (sequence OID)");
    assert!(advance.effect.contains("expected to return 51"));
    assert!(!advance.transactional);
    assert_eq!(advance.statement_timeout_ms, Some(1234));

    let set = preview(
        &observation,
        SequenceIntent::Set {
            value: 7,
            is_called: false,
        },
        None,
    )
    .unwrap();
    assert_eq!(set.parameters.len(), 14);
    assert_eq!(set.parameters[12], "7 (new last_value)");
    assert_eq!(set.parameters[13], "false (is_called)");
    assert!(set.effect.contains("expected to return 7"));
    assert!(set.effect.contains("backwards"));

    let restart = preview(&observation, SequenceIntent::Restart { with: None }, None).unwrap();
    assert_eq!(
        restart.summary,
        "ALTER SEQUENCE \"owned\"\"schema\".\"multi字\"\"seq\" RESTART"
    );
    assert!(restart.transactional);
    assert!(restart.sql.starts_with(RESTART_BEGIN_SQL) && restart.sql.ends_with("COMMIT"));
    assert!(restart
        .effect
        .contains("returns 1 (the observed START value)"));

    let forward = preview(
        &observation,
        SequenceIntent::Restart { with: Some(90) },
        None,
    )
    .unwrap();
    assert!(!forward.effect.contains("backwards"));
    assert!(!forward.text().contains("$13") && forward.text().contains("$12"));
}

#[test]
fn out_of_range_and_inconsistent_definitions_refuse_before_review() {
    let observation = observation();
    for intent in [
        SequenceIntent::Set {
            value: 0,
            is_called: true,
        },
        SequenceIntent::Set {
            value: 101,
            is_called: false,
        },
        SequenceIntent::Restart { with: Some(0) },
        SequenceIntent::Restart {
            with: Some(i64::MAX),
        },
    ] {
        assert_eq!(
            preview(&observation, intent, None),
            Err(SequenceError::OutOfRange)
        );
    }
    let mut zero_increment = observation.clone();
    zero_increment.definition.increment = 0;
    let mut inverted = observation.clone();
    inverted.definition.min_value = 200;
    let mut beyond_type = observation.clone();
    beyond_type.definition.max_value = i64::from(i32::MAX) + 1;
    let mut unnamed = observation.clone();
    unnamed.target.name = "字".repeat(22);
    for invalid in [zero_increment, inverted, beyond_type, unnamed] {
        assert_eq!(
            preview(&invalid, SequenceIntent::Advance, None),
            Err(SequenceError::InvalidTarget)
        );
    }
}

#[test]
fn next_value_respects_direction_limits_and_is_called() {
    let mut definition = observation().definition;
    assert_eq!(definition.next_after(100, true), None);
    assert_eq!(definition.next_after(100, false), Some(100));
    definition.increment = -3;
    assert_eq!(definition.next_after(50, true), Some(47));
    assert_eq!(definition.next_after(2, true), None);
    definition.max_value = i64::MAX;
    definition.data_type = SequenceDataType::Bigint;
    definition.increment = 1;
    assert_eq!(definition.next_after(i64::MAX, true), None);
}

#[test]
fn reviews_are_single_document_and_refuse_after_retirement() {
    let documents = super::super::data_documents::Documents::default();
    let document = documents
        .register("w".into(), "t".into(), "connection".into())
        .unwrap();
    let capture = observed(&document);
    let review = capture.review(SequenceIntent::Advance).unwrap();
    assert!(review.belongs_to(&document));
    assert!(review.retained_bytes() < MAX_SEQUENCE_REVIEW_BYTES);
    assert_ne!(
        review.attempt_id(),
        capture
            .review(SequenceIntent::Advance)
            .unwrap()
            .attempt_id()
    );
    documents.retire(&document).unwrap();
    assert!(matches!(
        capture.review(SequenceIntent::Advance),
        Err(SequenceError::Unavailable)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readonly_refuses_every_sequence_write_before_execution() {
    let (_directory, backend, document) = fixture().await;
    for mode in [
        crate::SafeMode::Disabled,
        crate::SafeMode::Protected,
        crate::SafeMode::Strict,
    ] {
        policy(&backend, true, mode).await;
        for intent in INTENTS {
            let review = observed(&document).review(intent).unwrap();
            assert!(matches!(
                backend
                    .submit_sequence(review, true, |_, _, _, _, _, _, _| async {
                        panic!("read-only precedes executor")
                    })
                    .await,
                Err(SequenceError::PolicyBlocked)
            ));
        }
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn protected_confirms_set_and_restart_but_not_advance() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Protected).await;
    for intent in INTENTS {
        let review = observed(&document).review(intent).unwrap();
        let attempt = review.attempt_id().to_owned();
        let result = backend
            .submit_sequence(review, false, |_, _, permit, _, _, _, _| async move {
                assert!(permit.admit_dispatch());
                outcome(SequenceOutcome::Completed { returned: Some(1) })
            })
            .await
            .unwrap();
        match (intent, result) {
            (SequenceIntent::Advance, SequenceSubmission::Finished(receipt)) => {
                assert_eq!(receipt.attempt_id, attempt);
                assert_eq!(
                    receipt.outcome,
                    SequenceOutcome::Completed { returned: Some(1) }
                );
            }
            (
                SequenceIntent::Set { .. } | SequenceIntent::Restart { .. },
                SequenceSubmission::NeedsConfirmation(confirmation),
            ) => {
                assert_eq!(confirmation.review().attempt_id(), attempt);
                assert_eq!(confirmation.review().intent(), intent);
            }
            (intent, _) => panic!("unexpected protected policy for {intent:?}"),
        }
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_confirmation_rechecks_policy_and_audits_only_completed_writes() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Strict).await;
    let confirm = |review| {
        let backend = backend.clone();
        async move {
            match backend
                .submit_sequence(review, false, |_, _, _, _, _, _, _| async {
                    panic!("no execution before confirmation")
                })
                .await
                .unwrap()
            {
                SequenceSubmission::NeedsConfirmation(confirmation) => confirmation,
                _ => panic!("strict confirmation"),
            }
        }
    };
    // Unknown outcome is reported verbatim and not audited as a success.
    let confirmation = confirm(observed(&document).review(SequenceIntent::Advance).unwrap()).await;
    let receipt = match backend
        .submit_sequence(confirmation.review, true, |_, _, _, _, _, _, _| async {
            outcome(SequenceOutcome::OutcomeUnknown {
                reason: SequenceFailure::Connection,
            })
        })
        .await
        .unwrap()
    {
        SequenceSubmission::Finished(receipt) => receipt,
        _ => panic!("finished"),
    };
    assert!(receipt.outcome.unknown());
    assert!(
        crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .is_empty()
    );
    // A completed confirmed write records exactly one override.
    let confirmation = confirm(
        observed(&document)
            .review(SequenceIntent::Restart { with: None })
            .unwrap(),
    )
    .await;
    backend
        .submit_sequence(confirmation.review, true, |_, _, _, _, _, _, _| async {
            outcome(SequenceOutcome::Completed { returned: None })
        })
        .await
        .unwrap();
    assert_eq!(
        crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .len(),
        1
    );
    // Policy is re-read at confirmation time.
    let confirmation = confirm(observed(&document).review(SequenceIntent::Advance).unwrap()).await;
    policy(&backend, true, crate::SafeMode::Strict).await;
    assert!(matches!(
        backend.confirm_sequence(*confirmation).await,
        Err(SequenceError::PolicyBlocked)
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tampered_or_foreign_reviews_never_reach_credentials_or_execution() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Disabled).await;
    let mut review = observed(&document)
        .review(SequenceIntent::Set {
            value: 10,
            is_called: true,
        })
        .unwrap();
    review.preview.sql.push_str("; DROP TABLE t");
    assert!(matches!(
        backend
            .submit_sequence_with_loader(
                review,
                true,
                |_, _, _, _, _, _, _| async { panic!("tampered review executed") },
                |_, _| panic!("tampered review hydrated credentials"),
            )
            .await,
        Err(SequenceError::InvalidTarget)
    ));
    let mut review = observed(&document)
        .review(SequenceIntent::Restart { with: Some(5) })
        .unwrap();
    review.intent = SequenceIntent::Restart { with: Some(500) };
    assert!(matches!(
        backend
            .submit_sequence_with_loader(
                review,
                true,
                |_, _, _, _, _, _, _| async { panic!("out-of-range review executed") },
                |_, _| panic!("out-of-range review hydrated credentials"),
            )
            .await,
        Err(SequenceError::OutOfRange)
    ));
    let documents = super::super::data_documents::Documents::default();
    let foreign = documents
        .register("foreign".into(), "t".into(), backend.fixture().id.clone())
        .unwrap();
    assert!(matches!(
        backend
            .apply_sequence(observed(&foreign).review(SequenceIntent::Advance).unwrap())
            .await,
        Err(SequenceError::ForeignDocument)
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observation_refuses_non_sequence_references_without_reading() {
    let (_directory, backend, document) = fixture().await;
    for reference in [
        PgObjectRef {
            kind: PgObjectKind::Table,
            schema: Some("public".into()),
            name: "ids".into(),
            identity_args: None,
        },
        PgObjectRef {
            kind: PgObjectKind::Sequence,
            schema: None,
            name: "ids".into(),
            identity_args: None,
        },
        PgObjectRef {
            kind: PgObjectKind::Sequence,
            schema: Some("public".into()),
            name: "x".repeat(64),
            identity_args: None,
        },
    ] {
        assert!(matches!(
            backend.observe_sequence(&document, reference).await,
            Err(SequenceError::InvalidTarget)
        ));
    }
    backend.shutdown().await.unwrap();
}
