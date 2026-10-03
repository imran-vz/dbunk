use super::*;
use dbunk_lib::backend::sequences::SequenceFailure;

#[test]
fn values_parse_strictly_within_signed_64_bit_range() {
    assert_eq!(parse_value("0"), Ok(0));
    assert_eq!(parse_value("-42"), Ok(-42));
    assert_eq!(parse_value("9223372036854775807"), Ok(i64::MAX));
    assert_eq!(parse_value("-9223372036854775808"), Ok(i64::MIN));
    for refused in [
        "",
        "-",
        "+5",
        " 5",
        "5 ",
        "1_000",
        "1,000",
        "1e3",
        "9223372036854775808",
        "-9223372036854775809",
        "٣",
        "--1",
    ] {
        assert!(parse_value(refused).is_err(), "{refused:?}");
    }
}

#[test]
fn review_actions_build_exact_typed_intents() {
    assert_eq!(
        intent_for(Action::ReviewAdvance, "junk", true, true, "junk"),
        Ok(SequenceIntent::Advance)
    );
    assert_eq!(
        intent_for(Action::ReviewSet, "17", false, false, ""),
        Ok(SequenceIntent::Set {
            value: 17,
            is_called: false
        })
    );
    assert!(intent_for(Action::ReviewSet, "", true, false, "").is_err());
    assert_eq!(
        intent_for(Action::ReviewRestart, "", true, false, "not used"),
        Ok(SequenceIntent::Restart { with: None })
    );
    assert_eq!(
        intent_for(Action::ReviewRestart, "", true, true, "-3"),
        Ok(SequenceIntent::Restart { with: Some(-3) })
    );
    assert!(intent_for(Action::ReviewRestart, "", true, true, "").is_err());
    assert!(intent_for(Action::Apply, "1", true, true, "1").is_err());
}

#[test]
fn unknown_outcomes_are_disclosed_as_unknown_and_never_retried() {
    let unknown = outcome_message(&SequenceOutcome::OutcomeUnknown {
        reason: SequenceFailure::Connection,
    });
    assert!(unknown.starts_with("Outcome unknown"));
    assert!(unknown.contains("will not be retried"));
    for outcome in [
        SequenceOutcome::TargetChanged,
        SequenceOutcome::NotDispatched {
            reason: SequenceFailure::Cancelled,
        },
        SequenceOutcome::Rejected {
            reason: SequenceFailure::Database {
                code: Some("22003".into()),
            },
        },
        SequenceOutcome::RolledBack {
            reason: SequenceFailure::Timeout,
        },
    ] {
        assert!(!outcome.unknown());
        assert!(!outcome_message(&outcome).contains("unknown"));
    }
    assert!(
        outcome_message(&SequenceOutcome::Completed { returned: Some(9) }).contains("returned 9")
    );
}

#[test]
fn lease_refuses_without_charge_and_releases_on_drop() {
    let limit = 128 * 1024 * 1024;
    let budget = Rc::new(Cell::new(limit - ALLOWANCE + 1));
    assert!(Lease::admit(budget.clone()).is_none());
    assert_eq!(budget.get(), limit - ALLOWANCE + 1);
    budget.set(limit - ALLOWANCE);
    let lease = Lease::admit(budget.clone()).unwrap();
    assert_eq!(budget.get(), limit);
    drop(lease);
    assert_eq!(budget.get(), limit - ALLOWANCE);
}
