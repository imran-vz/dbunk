use tauri::State;

use crate::safety::policy::{assert_permitted, resolve_policy, SafetyAuthorization, WriteIntent};
use crate::safety::{CONFIRM_TAG, READ_ONLY_TAG};
use crate::{storage, AppState, SafetyOverrideRecord};
use crate::{SqlitePool, StoredConnection};

pub(crate) fn resolved_policy(
    connection: &StoredConnection,
) -> crate::safety::policy::ResolvedSafetyPolicy {
    resolve_policy(connection.policy())
}

pub(crate) fn assert_legacy_permitted(
    connection: &StoredConnection,
    intent: &WriteIntent,
    confirmed: bool,
) -> Result<SafetyAuthorization, String> {
    let policy = resolved_policy(connection);
    assert_permitted(&policy, intent, confirmed).map_err(|refusal| {
        refusal.fold(
            |reason, _| format!("{READ_ONLY_TAG} {reason}"),
            |_| format!("{CONFIRM_TAG} This operation requires confirmation"),
        )
    })
}

pub(crate) async fn record_override(
    pool: &SqlitePool,
    connection_id: &str,
    command: &str,
    intent: &WriteIntent,
) {
    let classes = audit_class_labels(intent);
    if let Err(error) =
        storage::insert_safety_override(pool, connection_id, command, &classes).await
    {
        log::warn!("Failed to record safety override: {error}");
    }
}

fn audit_class_labels(intent: &WriteIntent) -> Vec<String> {
    match intent {
        WriteIntent::Statement { classes } | WriteIntent::ApplyMutations { classes } => classes
            .iter()
            .map(|class| class.label().to_string())
            .collect(),
        WriteIntent::RowMutation
        | WriteIntent::Import
        | WriteIntent::Seed
        | WriteIntent::CopyDestination => vec!["dml".into()],
        WriteIntent::Ddl
        | WriteIntent::Maintenance
        | WriteIntent::RefreshMatView
        | WriteIntent::Restore => vec!["ddl".into()],
        WriteIntent::TerminateBackend | WriteIntent::CancelBackend => Vec::new(),
    }
}

#[tauri::command]
pub(crate) async fn load_safety_overrides(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<SafetyOverrideRecord>, String> {
    storage::read_safety_overrides(&state.inner().pool, &connection_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postgres::sql_class::StatementClass;
    use crate::safety::policy::AuditDisposition;
    use crate::{Environment, PgStoredConnection, SafeMode, SshTunnelConfig};

    fn connection(
        environment: Environment,
        safe_mode: SafeMode,
        read_only: bool,
    ) -> StoredConnection {
        StoredConnection::PostgreSQL(PgStoredConnection {
            organization: Default::default(),
            id: "policy".into(),
            name: "Policy".into(),
            database: "postgres".into(),
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            password: String::new(),
            role: "read/write".into(),
            environment,
            safe_mode,
            read_only,
            last_activity_at: None,
            ssl: false,
            tls_options: None,
            driver_options: None,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    fn write_intents() -> Vec<(&'static str, WriteIntent)> {
        vec![
            (
                "run_query",
                WriteIntent::Statement {
                    classes: vec![StatementClass::Dml {
                        unbounded: false,
                        destructive: false,
                    }],
                },
            ),
            ("execute_ddl", WriteIntent::Ddl),
            ("run_pg_restore", WriteIntent::Restore),
            ("refresh_materialized_view", WriteIntent::RefreshMatView),
            ("run_pg_maintenance", WriteIntent::Maintenance),
            ("commit_cell_edits", WriteIntent::RowMutation),
            ("insert_row", WriteIntent::RowMutation),
            ("seed_table", WriteIntent::Seed),
            ("import_rows", WriteIntent::Import),
            ("copy_table_rows", WriteIntent::CopyDestination),
            ("delete_rows", WriteIntent::RowMutation),
            ("terminate_pg_backend", WriteIntent::TerminateBackend),
        ]
    }

    #[test]
    fn every_legacy_write_gate_refuses_strict_until_confirmed() {
        let connection = connection(Environment::Production, SafeMode::Inherit, false);
        for (command, intent) in write_intents() {
            let refusal = assert_legacy_permitted(&connection, &intent, false).expect_err(command);
            assert!(
                refusal.starts_with(CONFIRM_TAG),
                "{command} returned an unstable policy prefix"
            );
            assert!(matches!(
                assert_legacy_permitted(&connection, &intent, true)
                    .expect("confirmed override")
                    .audit_disposition(),
                AuditDisposition::RequiredAfterSuccess
            ));
        }
        assert!(matches!(
            assert_legacy_permitted(&connection, &WriteIntent::CancelBackend, false)
                .expect("cancel is always safe")
                .audit_disposition(),
            AuditDisposition::NotRequired
        ));
    }

    #[test]
    fn default_policy_is_dark_and_confirmed_does_not_create_an_audit() {
        let connection = connection(Environment::Development, SafeMode::Inherit, false);
        for (command, intent) in write_intents() {
            assert!(matches!(
                assert_legacy_permitted(&connection, &intent, true)
                    .unwrap_or_else(|error| panic!("{command}: {error}"))
                    .audit_disposition(),
                AuditDisposition::NotRequired
            ));
        }
    }

    #[test]
    fn read_only_tag_is_exact_and_confirmation_cannot_unlock() {
        let connection = connection(Environment::Development, SafeMode::Disabled, true);
        let refusal = assert_legacy_permitted(&connection, &WriteIntent::RowMutation, true)
            .expect_err("read-only refusal");
        assert!(refusal.starts_with(READ_ONLY_TAG));
        assert!(!refusal.starts_with(CONFIRM_TAG));

        assert!(matches!(
            assert_legacy_permitted(
                &connection,
                &WriteIntent::Statement {
                    classes: vec![StatementClass::Read]
                },
                false
            )
            .expect("read-only select")
            .audit_disposition(),
            AuditDisposition::NotRequired
        ));
    }

    #[test]
    fn audit_values_are_class_labels_only() {
        assert_eq!(
            audit_class_labels(&WriteIntent::Statement {
                classes: vec![
                    StatementClass::Read,
                    StatementClass::Ddl { destructive: true },
                    StatementClass::Unknown,
                ],
            }),
            vec!["read", "ddl", "unknown"]
        );
        assert_eq!(
            audit_class_labels(&WriteIntent::TerminateBackend),
            Vec::<String>::new()
        );
    }
}
