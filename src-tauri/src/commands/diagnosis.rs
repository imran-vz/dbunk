//! Staged connection diagnosis command (ADR-0025).

use tauri::State;

use crate::diagnosis::{self, ConnectionDiagnosis, DiagnoseConnectionPayload};
use crate::{credentials, storage, tunnel, AppState};

use super::current_credential_mode;

/// Reusing a stored password is safe only while the caller-controlled
/// connection still targets the same authentication boundary. Cosmetic,
/// policy, and database-name edits may vary; endpoint and transport edits
/// require the caller to provide the password explicitly.
fn credential_destination_matches(
    supplied: &crate::StoredConnection,
    stored: &crate::StoredConnection,
) -> bool {
    if supplied.id() != stored.id()
        || supplied.host() != stored.host()
        || supplied.port() != stored.port()
        || supplied.user() != stored.user()
        || supplied
            .ssh_tunnel()
            .map(crate::SshTunnelConfig::normalized)
            != stored.ssh_tunnel().map(crate::SshTunnelConfig::normalized)
    {
        return false;
    }

    match (supplied, stored) {
        (crate::StoredConnection::PostgreSQL(a), crate::StoredConnection::PostgreSQL(b)) => {
            crate::postgres::tls::ResolvedTls::from_postgres(a)
                == crate::postgres::tls::ResolvedTls::from_postgres(b)
        }
        (crate::StoredConnection::MySQL(a), crate::StoredConnection::MySQL(b)) => a.ssl == b.ssl,
        (crate::StoredConnection::ClickHouse(a), crate::StoredConnection::ClickHouse(b)) => {
            a.use_https == b.use_https && a.url_path == b.url_path
        }
        (crate::StoredConnection::Redis(a), crate::StoredConnection::Redis(b)) => {
            a.use_tls == b.use_tls && a.verify_tls_cert == b.verify_tls_cert
        }
        (crate::StoredConnection::SQLite(_), crate::StoredConnection::SQLite(_)) => true,
        _ => false,
    }
}

/// Run the connect ladder for a (possibly unsaved) connection. The outer
/// `Err` covers validation and credential-store failures only; probe
/// failures are inside the report.
#[tauri::command]
pub async fn diagnose_connection(
    state: State<'_, AppState>,
    payload: DiagnoseConnectionPayload,
) -> Result<ConnectionDiagnosis, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    tunnel::validate_connection_tunnel(&payload.connection)?;
    let mut connection = payload.connection;
    if let Some(id) = payload.hydrate_credential_from {
        if connection.password().is_empty() {
            let mut stored = storage::read_connection_by_id(&state.pool, &id)
                .await?
                .ok_or_else(|| "Connection not found".to_string())?;
            if !credential_destination_matches(&connection, &stored) {
                return Err(
                    "Enter the password again after changing the connection endpoint or transport security settings."
                        .to_string(),
                );
            }
            credentials::hydrate(&state.pool, mode, &mut stored).await?;
            connection.set_password(stored.password().to_string());
        }
    }
    diagnosis::run(&state.pool, mode, &connection).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn postgres() -> crate::StoredConnection {
        serde_json::from_value(serde_json::json!({
            "engine": "PostgreSQL",
            "id": "connection-1",
            "name": "Postgres",
            "database": "app",
            "host": "db.internal",
            "port": 5432,
            "user": "app",
            "password": "",
            "role": "read/write",
            "ssl": true,
            "tlsOptions": { "mode": "verify-full" }
        }))
        .expect("postgres fixture")
    }

    #[test]
    fn hydration_is_bound_to_endpoint_identity_and_transport() {
        let stored = postgres();
        let mut supplied = stored.clone();
        assert!(credential_destination_matches(&supplied, &stored));

        let crate::StoredConnection::PostgreSQL(pg) = &mut supplied else {
            unreachable!()
        };
        pg.database = "another_database".into();
        assert!(credential_destination_matches(&supplied, &stored));
        let crate::StoredConnection::PostgreSQL(pg) = &mut supplied else {
            unreachable!()
        };
        pg.host = "attacker.example".into();
        assert!(!credential_destination_matches(&supplied, &stored));
        let crate::StoredConnection::PostgreSQL(pg) = &mut supplied else {
            unreachable!()
        };
        pg.host = "db.internal".into();
        pg.tls_options.as_mut().unwrap().mode = crate::PgTlsMode::Disable;
        assert!(!credential_destination_matches(&supplied, &stored));
    }
}
