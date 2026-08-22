use crate::postgres::sql_class::{StatementClass, StatementClassSummary};
use crate::{Environment, SafeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SafetyLevel {
    Disabled,
    Protected,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedSafetyPolicy {
    pub(crate) environment: Environment,
    pub(crate) level: SafetyLevel,
    pub(crate) read_only: bool,
}

impl Default for ResolvedSafetyPolicy {
    fn default() -> Self {
        resolve_policy(Environment::default(), SafeMode::default(), false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteIntent {
    Statement { classes: Vec<StatementClass> },
    RowMutation,
    Ddl,
    Import,
    Seed,
    CopyDestination,
    Maintenance,
    RefreshMatView,
    Restore,
    ApplyMutations { classes: Vec<StatementClass> },
    TerminateBackend,
    CancelBackend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SafetyRefusal {
    Blocked {
        reason: &'static str,
        statements: Vec<StatementClassSummary>,
    },
    NeedsConfirmation {
        statements: Vec<StatementClassSummary>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditDisposition {
    NotRequired,
    RequiredAfterSuccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SafetyAuthorization {
    audit: AuditDisposition,
}

impl SafetyAuthorization {
    pub(crate) fn audit_disposition(self) -> AuditDisposition {
        self.audit
    }
}

pub(crate) fn resolve_policy(
    environment: Environment,
    safe_mode: SafeMode,
    read_only: bool,
) -> ResolvedSafetyPolicy {
    let level = match safe_mode {
        SafeMode::Disabled => SafetyLevel::Disabled,
        SafeMode::Protected => SafetyLevel::Protected,
        SafeMode::Strict => SafetyLevel::Strict,
        SafeMode::Inherit => match environment {
            Environment::Development | Environment::Test => SafetyLevel::Disabled,
            Environment::Staging => SafetyLevel::Protected,
            Environment::Production => SafetyLevel::Strict,
        },
    };
    ResolvedSafetyPolicy {
        environment,
        level,
        read_only,
    }
}

pub(crate) fn assert_permitted(
    policy: &ResolvedSafetyPolicy,
    intent: &WriteIntent,
    confirmed: bool,
) -> Result<SafetyAuthorization, SafetyRefusal> {
    if policy.read_only && !read_only_permits(intent) {
        return Err(SafetyRefusal::Blocked {
            reason: "This connection is read-only",
            statements: statement_summaries(intent),
        });
    }
    let confirmation_required = requires_confirmation(policy, intent);
    if confirmation_required && !confirmed {
        return Err(SafetyRefusal::NeedsConfirmation {
            statements: statement_summaries(intent),
        });
    }
    Ok(SafetyAuthorization {
        audit: if confirmation_required {
            AuditDisposition::RequiredAfterSuccess
        } else {
            AuditDisposition::NotRequired
        },
    })
}

pub(crate) fn requires_confirmation(policy: &ResolvedSafetyPolicy, intent: &WriteIntent) -> bool {
    if policy.read_only && !read_only_permits(intent) {
        return false;
    }
    match policy.level {
        SafetyLevel::Disabled => false,
        SafetyLevel::Protected => protected_requires_confirmation(intent),
        SafetyLevel::Strict => strict_requires_confirmation(intent),
    }
}

pub(crate) fn statement_summaries(intent: &WriteIntent) -> Vec<StatementClassSummary> {
    match intent {
        WriteIntent::Statement { classes } | WriteIntent::ApplyMutations { classes } => classes
            .iter()
            .enumerate()
            .map(|(index, class)| class.summary(index))
            .collect(),
        _ => Vec::new(),
    }
}

fn read_only_permits(intent: &WriteIntent) -> bool {
    match intent {
        WriteIntent::Statement { classes } => {
            !classes.is_empty()
                && classes
                    .iter()
                    .all(|class| matches!(class, StatementClass::Read))
        }
        WriteIntent::CancelBackend => true,
        _ => false,
    }
}

fn protected_requires_confirmation(intent: &WriteIntent) -> bool {
    match intent {
        WriteIntent::Statement { classes } => classes.iter().any(statement_is_destructive),
        WriteIntent::Ddl | WriteIntent::Restore | WriteIntent::TerminateBackend => true,
        WriteIntent::RowMutation
        | WriteIntent::Import
        | WriteIntent::Seed
        | WriteIntent::CopyDestination
        | WriteIntent::Maintenance
        | WriteIntent::RefreshMatView
        | WriteIntent::ApplyMutations { .. }
        | WriteIntent::CancelBackend => false,
    }
}

fn strict_requires_confirmation(intent: &WriteIntent) -> bool {
    match intent {
        WriteIntent::Statement { classes } => classes.iter().any(statement_is_write),
        WriteIntent::CancelBackend => false,
        WriteIntent::RowMutation
        | WriteIntent::Ddl
        | WriteIntent::Import
        | WriteIntent::Seed
        | WriteIntent::CopyDestination
        | WriteIntent::Maintenance
        | WriteIntent::RefreshMatView
        | WriteIntent::Restore
        | WriteIntent::ApplyMutations { .. }
        | WriteIntent::TerminateBackend => true,
    }
}

fn statement_is_destructive(class: &StatementClass) -> bool {
    match class {
        StatementClass::Dml {
            unbounded,
            destructive,
        } => *unbounded || *destructive,
        StatementClass::Ddl { destructive } => *destructive,
        StatementClass::Unknown => true,
        StatementClass::Read | StatementClass::Transaction | StatementClass::Session => false,
    }
}

fn statement_is_write(class: &StatementClass) -> bool {
    matches!(
        class,
        StatementClass::Dml { .. } | StatementClass::Ddl { .. } | StatementClass::Unknown
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENVIRONMENTS: [Environment; 4] = [
        Environment::Development,
        Environment::Test,
        Environment::Staging,
        Environment::Production,
    ];

    fn policy(level: SafetyLevel, read_only: bool) -> ResolvedSafetyPolicy {
        ResolvedSafetyPolicy {
            environment: Environment::Development,
            level,
            read_only,
        }
    }

    fn statement(class: StatementClass) -> WriteIntent {
        WriteIntent::Statement {
            classes: vec![class],
        }
    }

    #[test]
    fn inherit_resolution_follows_environment() {
        let expected = [
            SafetyLevel::Disabled,
            SafetyLevel::Disabled,
            SafetyLevel::Protected,
            SafetyLevel::Strict,
        ];
        for (environment, level) in ENVIRONMENTS.into_iter().zip(expected) {
            assert_eq!(
                resolve_policy(environment, SafeMode::Inherit, false).level,
                level
            );
        }
        for environment in ENVIRONMENTS {
            assert_eq!(
                resolve_policy(environment, SafeMode::Disabled, false).level,
                SafetyLevel::Disabled
            );
            assert_eq!(
                resolve_policy(environment, SafeMode::Protected, false).level,
                SafetyLevel::Protected
            );
            assert_eq!(
                resolve_policy(environment, SafeMode::Strict, false).level,
                SafetyLevel::Strict
            );
        }
    }

    #[test]
    fn read_only_only_admits_proven_reads_and_cancel() {
        let all_classes = [
            StatementClass::Read,
            StatementClass::Dml {
                unbounded: false,
                destructive: false,
            },
            StatementClass::Ddl { destructive: false },
            StatementClass::Transaction,
            StatementClass::Session,
            StatementClass::Unknown,
        ];
        for level in [
            SafetyLevel::Disabled,
            SafetyLevel::Protected,
            SafetyLevel::Strict,
        ] {
            for class in all_classes.clone() {
                let result =
                    assert_permitted(&policy(level, true), &statement(class.clone()), true);
                assert_eq!(result.is_ok(), matches!(class, StatementClass::Read));
            }
            assert!(
                assert_permitted(&policy(level, true), &WriteIntent::CancelBackend, false).is_ok()
            );
            assert!(matches!(
                assert_permitted(&policy(level, true), &WriteIntent::RowMutation, true),
                Err(SafetyRefusal::Blocked { .. })
            ));
        }
    }

    #[test]
    fn protected_requires_only_destructive_overrides() {
        let policy = policy(SafetyLevel::Protected, false);
        let ordinary = [
            statement(StatementClass::Read),
            statement(StatementClass::Dml {
                unbounded: false,
                destructive: false,
            }),
            statement(StatementClass::Ddl { destructive: false }),
            statement(StatementClass::Transaction),
            statement(StatementClass::Session),
            WriteIntent::RowMutation,
            WriteIntent::Import,
            WriteIntent::Seed,
            WriteIntent::CopyDestination,
            WriteIntent::Maintenance,
            WriteIntent::RefreshMatView,
            WriteIntent::ApplyMutations { classes: vec![] },
            WriteIntent::CancelBackend,
        ];
        for intent in ordinary {
            assert!(
                assert_permitted(&policy, &intent, false).is_ok(),
                "{intent:?}"
            );
        }
        let destructive = [
            statement(StatementClass::Dml {
                unbounded: true,
                destructive: false,
            }),
            statement(StatementClass::Ddl { destructive: true }),
            statement(StatementClass::Unknown),
            WriteIntent::Ddl,
            WriteIntent::Restore,
            WriteIntent::TerminateBackend,
        ];
        for intent in destructive {
            assert!(matches!(
                assert_permitted(&policy, &intent, false),
                Err(SafetyRefusal::NeedsConfirmation { .. })
            ));
            assert!(assert_permitted(&policy, &intent, true).is_ok());
        }
    }

    #[test]
    fn strict_requires_every_write_but_not_read_transaction_session_or_cancel() {
        let policy = policy(SafetyLevel::Strict, false);
        for intent in [
            statement(StatementClass::Read),
            statement(StatementClass::Transaction),
            statement(StatementClass::Session),
            WriteIntent::CancelBackend,
        ] {
            assert!(assert_permitted(&policy, &intent, false).is_ok());
        }
        for intent in [
            statement(StatementClass::Dml {
                unbounded: false,
                destructive: false,
            }),
            statement(StatementClass::Ddl { destructive: false }),
            statement(StatementClass::Unknown),
            WriteIntent::RowMutation,
            WriteIntent::Ddl,
            WriteIntent::Import,
            WriteIntent::Seed,
            WriteIntent::CopyDestination,
            WriteIntent::Maintenance,
            WriteIntent::RefreshMatView,
            WriteIntent::Restore,
            WriteIntent::ApplyMutations { classes: vec![] },
            WriteIntent::TerminateBackend,
        ] {
            assert!(
                matches!(
                    assert_permitted(&policy, &intent, false),
                    Err(SafetyRefusal::NeedsConfirmation { .. })
                ),
                "{intent:?}"
            );
            assert!(assert_permitted(&policy, &intent, true).is_ok());
        }
    }

    #[test]
    fn disabled_is_the_dark_launch_default() {
        let policy = resolve_policy(Environment::Development, SafeMode::Inherit, false);
        for intent in [
            statement(StatementClass::Unknown),
            WriteIntent::Ddl,
            WriteIntent::Restore,
            WriteIntent::TerminateBackend,
        ] {
            assert!(assert_permitted(&policy, &intent, false).is_ok());
        }
    }

    #[test]
    fn authorization_only_audits_a_required_confirmed_override() {
        let strict = policy(SafetyLevel::Strict, false);
        let write = statement(StatementClass::Dml {
            unbounded: false,
            destructive: false,
        });
        assert_eq!(
            assert_permitted(&strict, &write, true)
                .expect("confirmed strict write")
                .audit_disposition(),
            AuditDisposition::RequiredAfterSuccess
        );
        assert_eq!(
            assert_permitted(&strict, &statement(StatementClass::Read), true)
                .expect("strict read")
                .audit_disposition(),
            AuditDisposition::NotRequired
        );
        assert_eq!(
            assert_permitted(&policy(SafetyLevel::Disabled, false), &write, true,)
                .expect("disabled write")
                .audit_disposition(),
            AuditDisposition::NotRequired
        );
    }

    #[test]
    fn scripts_use_the_strictest_statement_and_refusals_expose_classes_only() {
        let intent = WriteIntent::Statement {
            classes: vec![
                StatementClass::Read,
                StatementClass::Dml {
                    unbounded: true,
                    destructive: false,
                },
            ],
        };
        let Err(SafetyRefusal::NeedsConfirmation { statements }) =
            assert_permitted(&policy(SafetyLevel::Protected, false), &intent, false)
        else {
            panic!("expected confirmation refusal");
        };
        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0].class,
            crate::postgres::sql_class::StatementClassKind::Read
        );
        assert_eq!(
            statements[1].class,
            crate::postgres::sql_class::StatementClassKind::Dml
        );
        assert!(statements[1].unbounded);
        assert_eq!(
            serde_json::to_string(&statements).unwrap(),
            r#"[{"index":0,"class":"read","unbounded":false,"destructive":false},{"index":1,"class":"dml","unbounded":true,"destructive":false}]"#
        );
    }
}
