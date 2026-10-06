//! Plan 032 write-safety rules for the table grid. This mirrors the backend's
//! `resolve_policy` only to choose the review dialog; the backend stays the
//! enforcement boundary (ADR-0024) and its refusal always wins.
use dbunk_lib::backend::{DevelopmentConnection, DevelopmentEnvironment, DevelopmentSafeMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectiveSafeMode {
    Disabled,
    Protected,
    Strict,
}

/// How the review dialog collects intent before apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmStyle {
    /// One Confirm click.
    Plain,
    /// The user types `confirm` first.
    Typed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TablePolicy {
    pub environment: Option<DevelopmentEnvironment>,
    pub safe_mode: EffectiveSafeMode,
    pub read_only: bool,
}

impl TablePolicy {
    /// Before connection metadata arrives the tab assumes the strictest
    /// writable policy.
    pub const UNKNOWN: Self = Self {
        environment: None,
        safe_mode: EffectiveSafeMode::Strict,
        read_only: false,
    };

    /// Inherit resolves development and test to disabled, staging to
    /// protected and production to strict, as the backend does.
    pub fn resolve(
        environment: DevelopmentEnvironment,
        safe_mode: DevelopmentSafeMode,
        read_only: bool,
    ) -> Self {
        let safe_mode = match safe_mode {
            DevelopmentSafeMode::Disabled => EffectiveSafeMode::Disabled,
            DevelopmentSafeMode::Protected => EffectiveSafeMode::Protected,
            DevelopmentSafeMode::Strict => EffectiveSafeMode::Strict,
            DevelopmentSafeMode::Inherit => match environment {
                DevelopmentEnvironment::Development | DevelopmentEnvironment::Test => {
                    EffectiveSafeMode::Disabled
                }
                DevelopmentEnvironment::Staging => EffectiveSafeMode::Protected,
                DevelopmentEnvironment::Production => EffectiveSafeMode::Strict,
            },
        };
        Self {
            environment: Some(environment),
            safe_mode,
            read_only,
        }
    }

    /// PostgreSQL records resolve their policy; anything else is `UNKNOWN`.
    pub fn from_connection(connection: &DevelopmentConnection) -> Self {
        connection.postgres.as_ref().map_or(Self::UNKNOWN, |postgres| {
            Self::resolve(postgres.environment, postgres.safe_mode, postgres.read_only)
        })
    }

    /// `None` when the review cannot be opened at all (read-only).
    pub fn confirm_style(&self) -> Option<ConfirmStyle> {
        if self.read_only {
            None
        } else if self.environment == Some(DevelopmentEnvironment::Production)
            || self.safe_mode == EffectiveSafeMode::Strict
        {
            Some(ConfirmStyle::Typed)
        } else {
            Some(ConfirmStyle::Plain)
        }
    }

    /// Whether the backend will refuse an unconfirmed apply with
    /// `PolicyNeedsConfirmation` (protected and strict both gate row
    /// mutations since Plan 032).
    pub fn expects_backend_confirmation(&self) -> bool {
        !self.read_only && self.safe_mode != EffectiveSafeMode::Disabled
    }

    pub fn read_only_reason(&self) -> Option<&'static str> {
        self.read_only.then_some(
            "Read-only connection: editing is disabled. Change it in connection settings.",
        )
    }

    /// `"Production · Strict safe mode"`, plus `" · Read-only"` when set.
    pub fn describe(&self) -> String {
        let environment = match self.environment {
            Some(DevelopmentEnvironment::Development) => "Development",
            Some(DevelopmentEnvironment::Test) => "Test",
            Some(DevelopmentEnvironment::Staging) => "Staging",
            Some(DevelopmentEnvironment::Production) => "Production",
            None => "Unknown environment",
        };
        let safe_mode = match self.safe_mode {
            EffectiveSafeMode::Disabled => "Disabled",
            EffectiveSafeMode::Protected => "Protected",
            EffectiveSafeMode::Strict => "Strict",
        };
        let mut text = format!("{environment} · {safe_mode} safe mode");
        if self.read_only {
            text.push_str(" · Read-only");
        }
        text
    }
}

/// Whether the user's single dialog action already counts as the backend
/// confirmation, so a `PolicyNeedsConfirmation` reply can be confirmed
/// without asking again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preconfirmation {
    Granted,
    NotGranted,
}

pub fn preconfirmation(policy: &TablePolicy, typed_matched: bool) -> Preconfirmation {
    match policy.confirm_style() {
        Some(ConfirmStyle::Typed) if typed_matched => Preconfirmation::Granted,
        Some(ConfirmStyle::Plain) if policy.expects_backend_confirmation() => {
            Preconfirmation::Granted
        }
        _ => Preconfirmation::NotGranted,
    }
}

/// What to do when the backend asks for confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmationStep {
    /// Send the confirmation without another prompt.
    AutoConfirm,
    /// The UI's view of the policy was stale, or a confirmation was already
    /// sent: require the typed confirmation.
    AskTyped,
}

/// At most one automatic confirmation per apply, and only when granted.
pub fn on_needs_confirmation(pre: Preconfirmation, auto_confirms_sent: u32) -> ConfirmationStep {
    if pre == Preconfirmation::Granted && auto_confirms_sent == 0 {
        ConfirmationStep::AutoConfirm
    } else {
        ConfirmationStep::AskTyped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::DevelopmentEnvironment::{Development, Production, Staging, Test};
    use dbunk_lib::backend::DevelopmentSafeMode::{Disabled, Inherit, Protected, Strict};

    const ENVIRONMENTS: [DevelopmentEnvironment; 4] = [Development, Test, Staging, Production];

    #[test]
    fn resolution_matches_the_backend_matrix() {
        // Rows: environment. Columns: Inherit, Disabled, Protected, Strict.
        // Inherit follows `inherit_resolution_follows_environment`
        // (backend/src/safety/policy.rs); explicit modes always win.
        let expected = [
            [
                EffectiveSafeMode::Disabled,
                EffectiveSafeMode::Disabled,
                EffectiveSafeMode::Protected,
                EffectiveSafeMode::Strict,
            ],
            [
                EffectiveSafeMode::Disabled,
                EffectiveSafeMode::Disabled,
                EffectiveSafeMode::Protected,
                EffectiveSafeMode::Strict,
            ],
            [
                EffectiveSafeMode::Protected,
                EffectiveSafeMode::Disabled,
                EffectiveSafeMode::Protected,
                EffectiveSafeMode::Strict,
            ],
            [
                EffectiveSafeMode::Strict,
                EffectiveSafeMode::Disabled,
                EffectiveSafeMode::Protected,
                EffectiveSafeMode::Strict,
            ],
        ];
        for (environment, row) in ENVIRONMENTS.into_iter().zip(expected) {
            for (safe_mode, level) in [Inherit, Disabled, Protected, Strict].into_iter().zip(row) {
                let policy = TablePolicy::resolve(environment, safe_mode, false);
                assert_eq!(policy.safe_mode, level, "{environment:?} {safe_mode:?}");
                assert_eq!(policy.environment, Some(environment));
                assert!(!policy.read_only);
            }
        }
    }

    #[test]
    fn confirm_style_follows_environment_and_safe_mode() {
        let production_disabled = TablePolicy::resolve(Production, Disabled, false);
        assert_eq!(
            production_disabled.confirm_style(),
            Some(ConfirmStyle::Typed)
        );
        assert!(!production_disabled.expects_backend_confirmation());

        let staging = TablePolicy::resolve(Staging, Inherit, false);
        assert_eq!(staging.confirm_style(), Some(ConfirmStyle::Plain));
        assert!(staging.expects_backend_confirmation());

        let development = TablePolicy::resolve(Development, Inherit, false);
        assert_eq!(development.confirm_style(), Some(ConfirmStyle::Plain));
        assert!(!development.expects_backend_confirmation());

        let development_strict = TablePolicy::resolve(Development, Strict, false);
        assert_eq!(
            development_strict.confirm_style(),
            Some(ConfirmStyle::Typed)
        );
        assert!(development_strict.expects_backend_confirmation());

        assert_eq!(
            TablePolicy::resolve(Production, Inherit, false).confirm_style(),
            Some(ConfirmStyle::Typed)
        );
        assert_eq!(TablePolicy::UNKNOWN.confirm_style(), Some(ConfirmStyle::Typed));
        assert!(TablePolicy::UNKNOWN.expects_backend_confirmation());
        assert_eq!(TablePolicy::UNKNOWN.read_only_reason(), None);
    }

    #[test]
    fn read_only_refuses_review_with_a_reason_in_every_environment() {
        for environment in ENVIRONMENTS {
            for safe_mode in [Inherit, Disabled, Protected, Strict] {
                let policy = TablePolicy::resolve(environment, safe_mode, true);
                assert_eq!(policy.confirm_style(), None);
                assert!(!policy.expects_backend_confirmation());
                assert_eq!(
                    policy.read_only_reason(),
                    Some("Read-only connection: editing is disabled. Change it in connection settings.")
                );
                assert_eq!(
                    preconfirmation(&policy, true),
                    Preconfirmation::NotGranted
                );
            }
        }
        assert_eq!(
            TablePolicy::resolve(Development, Inherit, false).read_only_reason(),
            None
        );
    }

    #[test]
    fn describe_names_environment_mode_and_read_only() {
        assert_eq!(
            TablePolicy::resolve(Production, Inherit, false).describe(),
            "Production · Strict safe mode"
        );
        assert_eq!(
            TablePolicy::resolve(Staging, Disabled, true).describe(),
            "Staging · Disabled safe mode · Read-only"
        );
        assert_eq!(
            TablePolicy::UNKNOWN.describe(),
            "Unknown environment · Strict safe mode"
        );
    }

    fn record(postgres: Option<(DevelopmentEnvironment, DevelopmentSafeMode, bool)>) -> DevelopmentConnection {
        DevelopmentConnection {
            id: "c".into(),
            name: "c".into(),
            engine: (if postgres.is_some() { "PostgreSQL" } else { "MySQL" }).into(),
            organization: Default::default(),
            unsupported_reason: None,
            postgres: postgres.map(|(environment, safe_mode, read_only)| {
                dbunk_lib::backend::DevelopmentPostgresConnection {
                    name: "c".into(),
                    host: "localhost".into(),
                    port: 5432,
                    database: "postgres".into(),
                    user: "postgres".into(),
                    environment,
                    safe_mode,
                    read_only,
                    tls: Default::default(),
                    driver_options: Default::default(),
                    ssh_tunnel: None,
                }
            }),
            environment: Production,
            settings: None,
            last_activity_at: None,
        }
    }

    #[test]
    fn connection_records_resolve_postgres_policy_or_unknown() {
        assert_eq!(
            TablePolicy::from_connection(&record(Some((Staging, Inherit, false)))),
            TablePolicy {
                environment: Some(Staging),
                safe_mode: EffectiveSafeMode::Protected,
                read_only: false,
            }
        );
        assert!(TablePolicy::from_connection(&record(Some((Development, Disabled, true)))).read_only);
        assert_eq!(TablePolicy::from_connection(&record(None)), TablePolicy::UNKNOWN);
    }

    #[test]
    fn preconfirmation_truth_table() {
        let typed = TablePolicy::resolve(Production, Inherit, false);
        assert_eq!(preconfirmation(&typed, true), Preconfirmation::Granted);
        assert_eq!(preconfirmation(&typed, false), Preconfirmation::NotGranted);
        let protected = TablePolicy::resolve(Staging, Inherit, false);
        assert_eq!(preconfirmation(&protected, false), Preconfirmation::Granted);
        assert_eq!(preconfirmation(&protected, true), Preconfirmation::Granted);
        let disabled = TablePolicy::resolve(Development, Inherit, false);
        assert_eq!(preconfirmation(&disabled, false), Preconfirmation::NotGranted);
        assert_eq!(preconfirmation(&disabled, true), Preconfirmation::NotGranted);
        assert_eq!(
            preconfirmation(&TablePolicy::UNKNOWN, true),
            Preconfirmation::Granted
        );
        assert_eq!(
            preconfirmation(&TablePolicy::UNKNOWN, false),
            Preconfirmation::NotGranted
        );
    }

    #[test]
    fn needs_confirmation_auto_confirms_once_and_only_when_granted() {
        assert_eq!(
            on_needs_confirmation(Preconfirmation::Granted, 0),
            ConfirmationStep::AutoConfirm
        );
        // A second refusal for the same apply escalates.
        assert_eq!(
            on_needs_confirmation(Preconfirmation::Granted, 1),
            ConfirmationStep::AskTyped
        );
        // The UI believed the policy was Disabled: stale, so never auto-confirm.
        let stale = TablePolicy::resolve(Development, Inherit, false);
        assert_eq!(
            on_needs_confirmation(preconfirmation(&stale, false), 0),
            ConfirmationStep::AskTyped
        );
        assert_eq!(
            on_needs_confirmation(Preconfirmation::NotGranted, 0),
            ConfirmationStep::AskTyped
        );
    }
}
