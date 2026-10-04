//! Inline form validation. Pure: callers pass field values and render the
//! returned `(field key, message)` pairs under the matching inputs. The
//! backend still validates everything; this only catches mistakes before a
//! round trip and says which field to fix.
use super::engine::Engine;
use dbunk_lib::backend::{DevelopmentCredentialState, DevelopmentStorageMode};

pub(super) type Errors = Vec<(&'static str, String)>;

pub(super) fn error_for<'a>(errors: &'a Errors, key: &str) -> Option<&'a str> {
    errors
        .iter()
        .find(|(field, _)| *field == key)
        .map(|(_, message)| message.as_str())
}

/// Connection fields the selected engine shows. Tunnel fields are checked by
/// the tunnel section when it is enabled.
pub(super) fn connection(engine: Engine, value: impl Fn(&str) -> String) -> Errors {
    let mut errors = Errors::new();
    let trimmed = |key| value(key).trim().to_owned();
    if trimmed("name").is_empty() {
        errors.push(("name", "Name the connection".into()));
    }
    if engine.shows("host") {
        let host = trimmed("host");
        if host.is_empty() {
            errors.push(("host", "Enter a host".into()));
        } else if host.chars().any(char::is_whitespace) {
            errors.push(("host", "Host cannot contain spaces".into()));
        }
    }
    if engine.shows("port") && !matches!(trimmed("port").parse::<u16>(), Ok(1..)) {
        errors.push(("port", "1–65535".into()));
    }
    if engine == Engine::Postgres {
        if trimmed("database").is_empty() {
            errors.push(("database", "Enter a database".into()));
        }
        if trimmed("user").is_empty() {
            errors.push(("user", "Enter a user".into()));
        }
        for key in [
            "statement-timeout",
            "idle-timeout",
            "connect-timeout",
            "keepalive",
        ] {
            let text = trimmed(key);
            if !text.is_empty() && text.parse::<u32>().is_err() {
                errors.push((key, "Whole number, or blank for the default".into()));
            }
        }
    }
    if engine.shows("path") && trimmed("path").is_empty() {
        errors.push(("path", "Choose an existing database file".into()));
    }
    if engine.shows("db-number") && trimmed("db-number").parse::<u8>().is_err() {
        errors.push(("db-number", "0–255".into()));
    }
    errors
}

/// Credential storage. Only Encrypted SQLite takes a password; it must be
/// typed twice when it is being set. Unencrypted storage and a new password
/// both need an explicit acknowledgement, as they did in the previous app.
pub(super) fn credentials(
    state: DevelopmentCredentialState,
    mode: DevelopmentStorageMode,
    password: &str,
    confirm: &str,
    acknowledged: bool,
) -> Errors {
    let mut errors = Errors::new();
    match state {
        DevelopmentCredentialState::NeedsRecovery => {}
        DevelopmentCredentialState::NeedsUnlock => {
            if password.is_empty() {
                errors.push(("password", "Enter your credential password".into()));
            }
        }
        DevelopmentCredentialState::NeedsOnboarding | DevelopmentCredentialState::Ready => {
            if mode == DevelopmentStorageMode::EncryptedSqlite {
                if password.is_empty() {
                    errors.push(("password", "Choose a credential password".into()));
                } else if confirm != password {
                    errors.push(("confirm", "Passwords do not match".into()));
                }
            }
            if needs_acknowledgement(mode) && !acknowledged {
                errors.push(("acknowledge", "Confirm you understand to continue".into()));
            }
        }
    }
    errors
}

pub(super) fn needs_acknowledgement(mode: DevelopmentStorageMode) -> bool {
    mode != DevelopmentStorageMode::Keychain
}

#[cfg(test)]
mod tests {
    use super::*;
    use DevelopmentCredentialState as State;
    use DevelopmentStorageMode as Mode;
    use std::collections::HashMap;

    fn values(pairs: &[(&str, &str)]) -> impl Fn(&str) -> String {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned().unwrap_or_default()
    }

    fn keys(errors: &Errors) -> Vec<&'static str> {
        errors.iter().map(|(key, _)| *key).collect()
    }

    #[test]
    fn postgres_requires_endpoint_identity_and_numeric_options() {
        let complete = [
            ("name", "Local"),
            ("host", "127.0.0.1"),
            ("port", "5432"),
            ("database", "postgres"),
            ("user", "postgres"),
        ];
        assert!(connection(Engine::Postgres, values(&complete)).is_empty());
        let errors = connection(
            Engine::Postgres,
            values(&[
                ("name", "  "),
                ("host", "db host"),
                ("port", "0"),
                ("statement-timeout", "5s"),
            ]),
        );
        assert_eq!(
            keys(&errors),
            [
                "name",
                "host",
                "port",
                "database",
                "user",
                "statement-timeout"
            ]
        );
        assert_eq!(error_for(&errors, "port"), Some("1–65535"));
        assert_eq!(error_for(&errors, "path"), None);
    }

    #[test]
    fn other_engines_check_only_the_fields_they_show() {
        assert_eq!(
            keys(&connection(Engine::Sqlite, values(&[("name", "x")]))),
            ["path"]
        );
        assert_eq!(
            keys(&connection(
                Engine::Redis,
                values(&[
                    ("name", "x"),
                    ("host", "h"),
                    ("port", "6379"),
                    ("db-number", "256")
                ])
            )),
            ["db-number"]
        );
        // MySQL leaves database and user optional.
        assert!(
            connection(
                Engine::MySql,
                values(&[("name", "x"), ("host", "h"), ("port", "3306")])
            )
            .is_empty()
        );
    }

    #[test]
    fn credential_password_applies_only_to_encrypted_storage() {
        assert!(credentials(State::NeedsOnboarding, Mode::Keychain, "", "", false).is_empty());
        assert_eq!(
            keys(&credentials(
                State::NeedsOnboarding,
                Mode::PlainSqlite,
                "",
                "",
                false
            )),
            ["acknowledge"]
        );
        assert_eq!(
            keys(&credentials(
                State::NeedsOnboarding,
                Mode::EncryptedSqlite,
                "secret",
                "secrte",
                true
            )),
            ["confirm"]
        );
        assert_eq!(
            keys(&credentials(
                State::Ready,
                Mode::EncryptedSqlite,
                "",
                "",
                false
            )),
            ["password", "acknowledge"]
        );
        assert!(
            credentials(
                State::NeedsOnboarding,
                Mode::EncryptedSqlite,
                "secret",
                "secret",
                true
            )
            .is_empty()
        );
    }

    #[test]
    fn unlock_needs_a_password_and_nothing_else() {
        assert_eq!(
            keys(&credentials(
                State::NeedsUnlock,
                Mode::EncryptedSqlite,
                "",
                "",
                false
            )),
            ["password"]
        );
        assert!(credentials(State::NeedsUnlock, Mode::EncryptedSqlite, "x", "", false).is_empty());
        assert!(credentials(State::NeedsRecovery, Mode::PlainSqlite, "", "", false).is_empty());
    }
}
