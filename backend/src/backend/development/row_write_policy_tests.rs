use super::*;
use DevelopmentEnvironment::{Development, Production, Staging, Test};
use DevelopmentSafeMode::{Disabled, Inherit, Protected, Strict};

#[test]
fn inherit_follows_the_environment_and_gates_protected_and_strict_applies() {
    let cases = [
        (
            Development,
            Inherit,
            DevelopmentSafetyLevel::Disabled,
            false,
        ),
        (Test, Inherit, DevelopmentSafetyLevel::Disabled, false),
        (Staging, Inherit, DevelopmentSafetyLevel::Protected, true),
        (Production, Inherit, DevelopmentSafetyLevel::Strict, true),
        (
            Production,
            Disabled,
            DevelopmentSafetyLevel::Disabled,
            false,
        ),
        (
            Development,
            Protected,
            DevelopmentSafetyLevel::Protected,
            true,
        ),
        (Development, Strict, DevelopmentSafetyLevel::Strict, true),
    ];
    for (environment, safe_mode, level, apply_needs_confirmation) in cases {
        assert_eq!(
            row_write_policy(environment, safe_mode, false),
            DevelopmentRowWritePolicy {
                level,
                apply_needs_confirmation,
            },
            "{environment:?} {safe_mode:?}"
        );
    }
}

#[test]
fn read_only_connections_refuse_instead_of_asking_for_confirmation() {
    let policy = row_write_policy(Production, Strict, true);
    assert_eq!(policy.level, DevelopmentSafetyLevel::Strict);
    assert!(!policy.apply_needs_confirmation);
}
