//! Connection CRUD and health-check commands.

use tauri::State;

use crate::dispatch;
use crate::managed;
use crate::socket_lifecycle;
use crate::storage;
use crate::tunnel;
use crate::{
    AppState, ConnectResult, ConnectionPayload, HealthCheckResult, StoredConnection,
    TestConnectionPayload,
};

use super::{current_credential_mode, find_connection, public_connections, with_active_connection};

#[tauri::command]
pub async fn load_connections(state: State<'_, AppState>) -> Result<Vec<StoredConnection>, String> {
    public_connections(state.inner()).await
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    save_connection_inner(state.inner(), connection).await
}

pub(crate) async fn save_connection_inner(
    state: &AppState,
    connection: StoredConnection,
) -> Result<Vec<StoredConnection>, String> {
    let mode = current_credential_mode(state).await?;
    tunnel::validate_connection_tunnel(&connection)?;
    let engine = connection.engine();
    let connection_id = connection.id().to_string();
    let save_result = socket_lifecycle::with_connection_fence(state, &connection_id, async {
        storage::upsert_connection(&state.pool, &connection).await?;
        crate::credentials::upsert(&state.pool, mode, &connection).await?;
        socket_lifecycle::invalidate_connection_caches(&connection_id, Some(engine));
        Ok::<_, String>(())
    })
    .await;
    save_result?;
    public_connections(state).await
}

#[tauri::command]
pub async fn delete_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    let delete_result =
        socket_lifecycle::with_connection_fence(state, &payload.connection_id, async {
            if !storage::delete_connection(&state.pool, &payload.connection_id).await? {
                return Err(format!("Connection '{}' not found", payload.connection_id));
            }
            if let Err(error) =
                crate::credentials::delete(&state.pool, mode, &payload.connection_id).await
            {
                log::warn!(
                    "Failed to delete credential for {}: {error}",
                    payload.connection_id
                );
            }
            socket_lifecycle::invalidate_connection_caches(&payload.connection_id, None);
            Ok(())
        })
        .await;
    delete_result?;
    public_connections(state).await
}

/// Pick the first non-colliding "<name> copy" / "<name> copy N" label.
fn next_copy_name(source_name: &str, existing: &[String]) -> String {
    let base = format!("{source_name} copy");
    if !existing.iter().any(|name| name == &base) {
        return base;
    }
    let mut counter = 2u32;
    loop {
        let candidate = format!("{base} {counter}");
        if !existing.iter().any(|name| name == &candidate) {
            return candidate;
        }
        counter += 1;
    }
}

/// Duplicate a stored connection under a fresh id and a collision-safe
/// name, copying the credential through the credential backend so the
/// secret never crosses IPC. Favorite flag and activity reset; folder,
/// color, policy, tunnel, and driver options carry over. On a
/// credential-write failure the freshly inserted row is rolled back so
/// a half-duplicated record can't linger. Plan 009 (PAR-005).
#[tauri::command]
pub async fn duplicate_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<Vec<StoredConnection>, String> {
    duplicate_connection_inner(state.inner(), &payload.connection_id).await
}

pub(crate) async fn duplicate_connection_inner(
    state: &AppState,
    connection_id: &str,
) -> Result<Vec<StoredConnection>, String> {
    let mode = current_credential_mode(state).await?;
    // One list read serves both the source lookup and the name
    // collision check.
    let all = storage::read_connections(&state.pool).await?;
    let mut source = all
        .iter()
        .find(|connection| connection.id() == connection_id)
        .cloned()
        .ok_or_else(|| format!("Connection '{connection_id}' not found"))?;

    // Hydrate the secret up front so a failing credential store stops
    // the operation before any row is written. `hydrate` already
    // skips SQLite (no credential — a locked encrypted store must not
    // block duplicating them); the OS keychain backend never errors —
    // read failures degrade to an empty map per its documented
    // failure policy, so a locked keychain duplicates the record
    // without a credential rather than failing.
    crate::credentials::hydrate(&state.pool, mode, &mut source).await?;
    let secret = (!source.password().is_empty()).then(|| source.password().to_string());

    let existing_names: Vec<String> = all
        .iter()
        .map(|connection| connection.name().to_string())
        .collect();
    let new_id = uuid::Uuid::new_v4().to_string();
    let mut copy = source.duplicated_as(
        new_id.clone(),
        next_copy_name(source.name(), &existing_names),
    );

    storage::upsert_connection(&state.pool, &copy).await?;
    if let Some(secret) = secret {
        copy.set_password(secret);
        if let Err(error) = crate::credentials::upsert(&state.pool, mode, &copy).await {
            // Roll back the row so we never keep a copy that silently
            // lost its credential.
            let _ = storage::delete_connection(&state.pool, &new_id).await;
            return Err(error);
        }
    }
    public_connections(state).await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionOrganizationPayload {
    pub connection_id: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub is_favorite: bool,
    #[serde(default)]
    pub color: String,
}

/// Organization-only update (Plan 009 amendment): folder / favorite /
/// color, never credentials — safe for one-click toggles from list
/// rows where the frontend holds no password to re-send.
#[tauri::command]
pub async fn update_connection_organization(
    state: State<'_, AppState>,
    payload: ConnectionOrganizationPayload,
) -> Result<Vec<StoredConnection>, String> {
    update_connection_organization_inner(state.inner(), payload).await
}

pub(crate) async fn update_connection_organization_inner(
    state: &AppState,
    payload: ConnectionOrganizationPayload,
) -> Result<Vec<StoredConnection>, String> {
    let organization = crate::ConnectionOrganization {
        folder: payload.folder.trim().to_string(),
        is_favorite: payload.is_favorite,
        color: payload.color.trim().to_string(),
    };
    let updated =
        storage::update_connection_organization(&state.pool, &payload.connection_id, &organization)
            .await?;
    if !updated {
        return Err(format!("Connection '{}' not found", payload.connection_id));
    }
    public_connections(state).await
}

#[tauri::command]
pub async fn disconnect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<(), String> {
    socket_lifecycle::with_connection_fence(state.inner(), &payload.connection_id, async {
        socket_lifecycle::invalidate_connection_caches(&payload.connection_id, None);
    })
    .await;
    Ok(())
}

#[tauri::command]
pub async fn connect_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<ConnectResult, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    with_active_connection(state, &payload.connection_id, |connection| async move {
        managed::ensure_running_for_connection(&state.pool, mode, &connection).await?;
        dispatch::ping_connection(&connection).await
    })
    .await
}

/// Validate credentials without saving them. Used by the New Connection
/// side panel's `Test Connection` button — connects, runs `SELECT 1`,
/// disconnects, returns latency or surfaces the underlying driver error.
#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    payload: TestConnectionPayload,
) -> Result<ConnectResult, String> {
    let state = state.inner();
    let mode = current_credential_mode(state).await?;
    tunnel::validate_connection_tunnel(&payload.connection)?;
    let route_key = format!("test-{}", uuid::Uuid::new_v4());
    let connection =
        tunnel::resolve_connection(&state.pool, mode, &route_key, &payload.connection).await?;
    let result = dispatch::ping_connection(&connection).await;
    tunnel::drop_connection(&route_key);
    result
}

/// Periodic poll: returns "healthy" + latency or "error" + message. Designed
/// for a frontend interval that fans out across all stored connections — the
/// caller decides cadence and concurrency.
/// Health check is a probe, not a use of the connection — we deliberately do
/// NOT route through `with_active_connection` so a 30 s tick doesn't keep
/// `lastActivityAt` artificially fresh.
#[tauri::command]
pub async fn health_check_connection(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<HealthCheckResult, String> {
    let connection = find_connection(state.inner(), &payload.connection_id).await?;
    match dispatch::ping_connection(&connection).await {
        Ok(result) => Ok(HealthCheckResult::Healthy {
            latency_ms: result.latency_ms,
        }),
        Err(error) => Ok(HealthCheckResult::Error { error }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ConnectionOrganization, CredentialStorageMode, Environment, PgDriverOptions,
        PgStoredConnection, SafeMode, SshTunnelConfig,
    };

    fn pg_connection(id: &str, name: &str, password: &str) -> StoredConnection {
        StoredConnection::PostgreSQL(PgStoredConnection {
            id: id.into(),
            name: name.into(),
            database: "postgres".into(),
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            password: password.into(),
            role: "read/write".into(),
            environment: Environment::Staging,
            safe_mode: SafeMode::Protected,
            read_only: true,
            last_activity_at: Some("2026-08-24T00:00:00Z".into()),
            organization: ConnectionOrganization {
                folder: "Fleet".into(),
                is_favorite: true,
                color: "teal".into(),
            },
            ssl: true,
            driver_options: Some(PgDriverOptions {
                statement_timeout_ms: Some(30_000),
                ..PgDriverOptions::default()
            }),
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    #[test]
    fn next_copy_name_suffixes_until_free() {
        let existing = vec![
            "Primary".to_string(),
            "Primary copy".to_string(),
            "Primary copy 2".to_string(),
        ];
        assert_eq!(next_copy_name("Primary", &existing), "Primary copy 3");
        assert_eq!(next_copy_name("Other", &existing), "Other copy");
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn duplicate_copies_record_and_credential_without_favorite_or_activity() {
        let (_directory, state) = crate::test_app_state().await;
        save_connection_inner(&state, pg_connection("source", "Primary", "s3cret"))
            .await
            .expect("save source");

        let connections = duplicate_connection_inner(&state, "source")
            .await
            .expect("duplicate");
        let copy = connections
            .iter()
            .find(|connection| connection.name() == "Primary copy")
            .expect("copy present");
        assert_ne!(copy.id(), "source");
        assert_eq!(copy.folder(), "Fleet");
        assert_eq!(copy.color(), "teal");
        assert!(!copy.is_favorite());
        assert!(copy.last_activity_at().is_none());
        assert_eq!(copy.policy().environment, Environment::Staging);
        assert_eq!(copy.policy().safe_mode, SafeMode::Protected);
        assert!(copy.policy().read_only);
        // The wire never carries a secret.
        assert!(copy.password().is_empty());

        // The credential backend carries it, keyed to the new id.
        let secrets = crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read credentials");
        assert_eq!(secrets.get(copy.id()).map(String::as_str), Some("s3cret"));
        assert_eq!(secrets.get("source").map(String::as_str), Some("s3cret"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn duplicate_twice_suffixes_the_name() {
        let (_directory, state) = crate::test_app_state().await;
        save_connection_inner(&state, pg_connection("source", "Primary", ""))
            .await
            .expect("save source");
        duplicate_connection_inner(&state, "source")
            .await
            .expect("first duplicate");
        let connections = duplicate_connection_inner(&state, "source")
            .await
            .expect("second duplicate");
        let names: Vec<&str> = connections.iter().map(StoredConnection::name).collect();
        assert!(names.contains(&"Primary copy"));
        assert!(names.contains(&"Primary copy 2"));
    }

    fn sqlite_connection(id: &str, name: &str) -> StoredConnection {
        StoredConnection::SQLite(crate::SqliteStoredConnection {
            id: id.into(),
            name: name.into(),
            database: "/tmp/local.db".into(),
            host: String::new(),
            port: 0,
            user: String::new(),
            password: String::new(),
            role: "read/write".into(),
            environment: Environment::Development,
            safe_mode: SafeMode::Inherit,
            read_only: false,
            last_activity_at: None,
            organization: Default::default(),
        })
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn duplicate_copies_credential_in_encrypted_mode() {
        let (_directory, state) = crate::test_app_state().await;
        crate::credentials::configure(
            &state.pool,
            CredentialStorageMode::EncryptedSqlite,
            Some("test passphrase"),
        )
        .await
        .expect("configure encrypted storage");
        save_connection_inner(&state, pg_connection("source", "Primary", "s3cret"))
            .await
            .expect("save source");

        let connections = duplicate_connection_inner(&state, "source")
            .await
            .expect("duplicate");
        let copy = connections
            .iter()
            .find(|connection| connection.name() == "Primary copy")
            .expect("copy present");
        let secrets =
            crate::credentials::read_all(&state.pool, CredentialStorageMode::EncryptedSqlite)
                .await
                .expect("read credentials");
        assert_eq!(secrets.get(copy.id()).map(String::as_str), Some("s3cret"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn locked_encrypted_store_blocks_pg_duplicate_but_not_sqlite() {
        let (_directory, state) = crate::test_app_state().await;
        crate::credentials::configure(
            &state.pool,
            CredentialStorageMode::EncryptedSqlite,
            Some("test passphrase"),
        )
        .await
        .expect("configure encrypted storage");
        save_connection_inner(&state, pg_connection("pg-src", "Primary", "s3cret"))
            .await
            .expect("save pg source");
        save_connection_inner(&state, sqlite_connection("lite-src", "Local file"))
            .await
            .expect("save sqlite source");
        crate::credentials::lock_for_tests();

        // A network-backed source fails up front — before any row is
        // written — because its credential cannot be read.
        let error = duplicate_connection_inner(&state, "pg-src")
            .await
            .expect_err("locked store must refuse pg duplicate");
        assert!(!error.is_empty());
        let after = storage::read_connections(&state.pool)
            .await
            .expect("read connections");
        assert!(!after.iter().any(|c| c.name() == "Primary copy"));

        // SQLite connections carry no credential; a locked store must
        // not block duplicating them.
        let connections = duplicate_connection_inner(&state, "lite-src")
            .await
            .expect("sqlite duplicate under locked store");
        assert!(connections.iter().any(|c| c.name() == "Local file copy"));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn organization_update_changes_columns_and_never_touches_credentials() {
        let (_directory, state) = crate::test_app_state().await;
        save_connection_inner(&state, pg_connection("source", "Primary", "s3cret"))
            .await
            .expect("save source");

        let connections = update_connection_organization_inner(
            &state,
            ConnectionOrganizationPayload {
                connection_id: "source".into(),
                folder: "  Ops  ".into(),
                is_favorite: false,
                color: "blue".into(),
            },
        )
        .await
        .expect("organization update");
        let updated = connections
            .iter()
            .find(|connection| connection.id() == "source")
            .expect("source present");
        assert_eq!(updated.folder(), "Ops");
        assert!(!updated.is_favorite());
        assert_eq!(updated.color(), "blue");

        // The credential is untouched — this path must be safe for a
        // one-click toggle with no password in frontend memory.
        let secrets = crate::credentials::read_all(&state.pool, CredentialStorageMode::PlainSqlite)
            .await
            .expect("read credentials");
        assert_eq!(secrets.get("source").map(String::as_str), Some("s3cret"));

        let missing = update_connection_organization_inner(
            &state,
            ConnectionOrganizationPayload {
                connection_id: "ghost".into(),
                folder: String::new(),
                is_favorite: true,
                color: String::new(),
            },
        )
        .await;
        assert!(missing.is_err());
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn duplicate_missing_source_refuses() {
        let (_directory, state) = crate::test_app_state().await;
        let error = duplicate_connection_inner(&state, "ghost")
            .await
            .expect_err("missing source must refuse");
        assert!(error.contains("not found"), "unexpected error: {error}");
    }
}
