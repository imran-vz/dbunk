//! Managed Server orchestration (ADR-0019).
//!
//! dbunk provisions local dev databases as Docker containers it
//! orchestrates but never supervises: the data directory lives on a
//! named volume with a lifetime independent of the container, status
//! is always derived live from Docker, and provisioning ends by
//! creating a normal Connection (plus credential) pointing at the new
//! server.

use std::net::TcpListener;
use std::time::Duration;

use rand::distributions::Alphanumeric;
use rand::Rng;
use sqlx::sqlite::SqlitePool;
use sqlx::ConnectOptions;

use crate::{
    credentials, docker, storage, CredentialStorageMode, DatabaseEngine, Environment,
    ManagedServer, ManagedServerWithStatus, MySqlStoredConnection, PgStoredConnection,
    ProvisionManagedServerPayload, ProvisionManagedServerResult, SafeMode, SshTunnelConfig,
    StoredConnection,
};

/// Image major versions dbunk will provision, newest first. Doubles as
/// an allowlist: the tag is interpolated into `docker run`.
pub const SUPPORTED_POSTGRES_VERSIONS: &[&str] = &["18", "17", "16", "15", "14"];
pub const SUPPORTED_MYSQL_VERSIONS: &[&str] = &["9", "8.4", "8.0"];

/// Non-default port bases (ADR-0019): never contend with a Homebrew
/// Postgres on 5432 / system MySQL on 3306.
const PG_PORT_BASE: u16 = 5433;
const MYSQL_PORT_BASE: u16 = 3307;
const PORT_SCAN_SPAN: u16 = 500;

const READY_TIMEOUT: Duration = Duration::from_secs(120);
const READY_POLL_INTERVAL: Duration = Duration::from_millis(1000);

struct EngineSpec {
    image_repo: &'static str,
    container_port: u16,
    volume_mount: &'static str,
    port_base: u16,
    supported_versions: &'static [&'static str],
}

fn engine_spec(engine: DatabaseEngine) -> Result<EngineSpec, String> {
    match engine {
        DatabaseEngine::PostgreSQL => Ok(EngineSpec {
            image_repo: "postgres",
            container_port: 5432,
            volume_mount: "/var/lib/postgresql/data",
            port_base: PG_PORT_BASE,
            supported_versions: SUPPORTED_POSTGRES_VERSIONS,
        }),
        DatabaseEngine::MySQL => Ok(EngineSpec {
            image_repo: "mysql",
            container_port: 3306,
            volume_mount: "/var/lib/mysql",
            port_base: MYSQL_PORT_BASE,
            supported_versions: SUPPORTED_MYSQL_VERSIONS,
        }),
        other => Err(format!(
            "Managed servers are not supported for {} yet",
            other.as_str()
        )),
    }
}

fn provision_env(
    engine: DatabaseEngine,
    database: &str,
    user: &str,
    password: &str,
) -> Vec<(String, String)> {
    match engine {
        DatabaseEngine::MySQL => vec![
            ("MYSQL_DATABASE".into(), database.into()),
            ("MYSQL_USER".into(), user.into()),
            ("MYSQL_PASSWORD".into(), password.into()),
            ("MYSQL_ROOT_PASSWORD".into(), password.into()),
        ],
        // PostgreSQL (the only other engine_spec arm).
        _ => vec![
            ("POSTGRES_DB".into(), database.into()),
            ("POSTGRES_USER".into(), user.into()),
            ("POSTGRES_PASSWORD".into(), password.into()),
        ],
    }
}

/// Lowercased `[a-z0-9_]` identifier derived from the server name,
/// used for default database/user names.
fn slugify(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        let mut result = trimmed.to_string();
        if result.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            result.insert(0, '_');
        }
        result.truncate(48);
        result
    }
}

fn generate_password() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

fn pick_port(claimed: &[u16], base: u16) -> Result<u16, String> {
    for port in base..base.saturating_add(PORT_SCAN_SPAN) {
        if claimed.contains(&port) {
            continue;
        }
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(format!(
        "no free port found in {base}..{}",
        base + PORT_SCAN_SPAN
    ))
}

/// Poll until the server accepts authenticated connections. Covers
/// both the TCP listener coming up and the image's first-boot
/// initialization (initdb / mysql bootstrap).
async fn wait_until_ready(
    engine: DatabaseEngine,
    port: u16,
    database: &str,
    user: &str,
    password: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    let mut last_error = String::from("server did not become ready");
    while tokio::time::Instant::now() < deadline {
        let attempt: Result<(), String> = match engine {
            DatabaseEngine::PostgreSQL => {
                let options = sqlx::postgres::PgConnectOptions::new()
                    .host("127.0.0.1")
                    .port(port)
                    .database(database)
                    .username(user)
                    .password(password)
                    .ssl_mode(sqlx::postgres::PgSslMode::Disable);
                match options.connect().await {
                    Ok(conn) => {
                        let _ = sqlx::Connection::close(conn).await;
                        Ok(())
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            DatabaseEngine::MySQL => {
                let options = sqlx::mysql::MySqlConnectOptions::new()
                    .host("127.0.0.1")
                    .port(port)
                    .database(database)
                    .username(user)
                    .password(password)
                    .ssl_mode(sqlx::mysql::MySqlSslMode::Disabled);
                match options.connect().await {
                    Ok(conn) => {
                        let _ = sqlx::Connection::close(conn).await;
                        Ok(())
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            _ => return Err("unsupported engine".to_string()),
        };
        match attempt {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
        tokio::time::sleep(READY_POLL_INTERVAL).await;
    }
    Err(format!(
        "server did not become ready within {}s: {last_error}",
        READY_TIMEOUT.as_secs()
    ))
}

fn build_connection(
    server: &ManagedServer,
    connection_id: &str,
    password: &str,
) -> Result<StoredConnection, String> {
    Ok(match server.engine {
        DatabaseEngine::PostgreSQL => StoredConnection::PostgreSQL(PgStoredConnection {
            id: connection_id.to_string(),
            name: server.name.clone(),
            database: server.database.clone(),
            host: "127.0.0.1".to_string(),
            port: server.port,
            user: server.user.clone(),
            password: password.to_string(),
            role: "read/write".to_string(),
            environment: Environment::default(),
            safe_mode: SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            folder: String::new(),
            is_favorite: false,
            color: String::new(),
            ssl: false,
            driver_options: None,
            ssh_tunnel: SshTunnelConfig::default(),
        }),
        DatabaseEngine::MySQL => StoredConnection::MySQL(MySqlStoredConnection {
            id: connection_id.to_string(),
            name: server.name.clone(),
            database: server.database.clone(),
            host: "127.0.0.1".to_string(),
            port: server.port,
            user: server.user.clone(),
            password: password.to_string(),
            role: "read/write".to_string(),
            environment: Environment::default(),
            safe_mode: SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            folder: String::new(),
            is_favorite: false,
            color: String::new(),
            ssl: false,
            ssh_tunnel: SshTunnelConfig::default(),
        }),
        other => {
            return Err(format!(
                "Managed servers are not supported for {} yet",
                other.as_str()
            ))
        }
    })
}

fn connection_string(server: &ManagedServer, password: &str) -> String {
    let scheme = match server.engine {
        DatabaseEngine::MySQL => "mysql",
        _ => "postgres",
    };
    format!(
        "{scheme}://{}:{password}@127.0.0.1:{}/{}",
        server.user, server.port, server.database
    )
}

async fn clean_up_failed_provision(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    credentials_before: &std::collections::HashMap<String, String>,
    server: &ManagedServer,
    connection_id: &str,
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = credentials::write_all(pool, mode, credentials_before).await {
        errors.push(format!("credential rollback failed: {error}"));
    }
    if let Err(error) = storage::delete_connection(pool, connection_id).await {
        errors.push(format!("connection cleanup failed: {error}"));
    }
    if let Err(error) = storage::managed::delete_managed_server(pool, &server.id).await {
        errors.push(format!("managed-server cleanup failed: {error}"));
    }
    if docker::container_state(&server.container_name)
        .await
        .is_some()
    {
        if let Err(error) = docker::remove_container(&server.container_name).await {
            errors.push(format!("container cleanup failed: {error}"));
        }
    }
    if docker::volume_exists(&server.volume_name).await {
        if let Err(error) = docker::remove_volume(&server.volume_name).await {
            errors.push(format!("volume cleanup failed: {error}"));
        }
    }
    errors
}

pub async fn provision(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    payload: ProvisionManagedServerPayload,
) -> Result<ProvisionManagedServerResult, String> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err("server name is required".to_string());
    }
    let spec = engine_spec(payload.engine)?;
    if !spec.supported_versions.contains(&payload.version.as_str()) {
        return Err(format!(
            "unsupported {} version '{}'",
            payload.engine.as_str(),
            payload.version
        ));
    }

    let docker_status = docker::status().await;
    if !docker_status.available {
        return Err(format!(
            "Docker is not available: {}",
            docker_status.error.unwrap_or_default()
        ));
    }
    // Capture the credential backend before creating external resources.
    // A partial credential write can otherwise evade delete-by-ID when
    // the in-process credential cache was not updated.
    let credentials_before = credentials::read_all(pool, mode).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let short_id = &id[..8];
    let slug = slugify(&name);
    let database = payload
        .database
        .map(|d| slugify(&d))
        .unwrap_or_else(|| slug.clone());
    let user = payload
        .user
        .map(|u| slugify(&u))
        .unwrap_or_else(|| slug.clone());
    let password = generate_password();

    let port = match payload.port {
        Some(port) => port,
        None => {
            let claimed = storage::managed::claimed_ports(pool).await?;
            pick_port(&claimed, spec.port_base)?
        }
    };

    let server = ManagedServer {
        id: id.clone(),
        name,
        engine: payload.engine,
        version: payload.version.clone(),
        port,
        container_name: format!("dbunk-{slug}-{short_id}"),
        volume_name: format!("dbunk-{slug}-{short_id}-data"),
        database,
        user,
        connection_id: None,
        created_at: storage::now(),
    };

    docker::create_volume(&server.volume_name).await?;
    let run_result = docker::run_container(&docker::RunContainerSpec {
        name: &server.container_name,
        image: &format!("{}:{}", spec.image_repo, server.version),
        host_port: server.port,
        container_port: spec.container_port,
        volume_name: &server.volume_name,
        volume_mount: spec.volume_mount,
        env: provision_env(server.engine, &server.database, &server.user, &password),
    })
    .await;

    let ready = match run_result {
        Ok(()) => {
            wait_until_ready(
                server.engine,
                server.port,
                &server.database,
                &server.user,
                &password,
            )
            .await
        }
        Err(error) => Err(error),
    };
    if let Err(error) = ready {
        // Roll the provision back completely: a failed create must not
        // leave a half-made container or volume behind.
        let _ = docker::remove_container(&server.container_name).await;
        let _ = docker::remove_volume(&server.volume_name).await;
        return Err(error);
    }

    // Server is live — persist the record, then the Connection +
    // credential through the same path `save_connection` uses.
    let connection_id = uuid::Uuid::new_v4().to_string();
    let mut server = server;
    server.connection_id = Some(connection_id.clone());

    let persist_result = async {
        storage::managed::upsert_managed_server(pool, &server).await?;
        let connection = build_connection(&server, &connection_id, &password)?;
        storage::upsert_connection(pool, &connection).await?;
        credentials::upsert(pool, mode, &connection).await
    }
    .await;
    if let Err(error) = persist_result {
        let cleanup_errors =
            clean_up_failed_provision(pool, mode, &credentials_before, &server, &connection_id)
                .await;
        if cleanup_errors.is_empty() {
            return Err(format!(
                "failed to save the managed server; provisioning was rolled back: {error}"
            ));
        }
        return Err(format!(
            "failed to save the managed server: {error}; rollback was incomplete: {}",
            cleanup_errors.join("; ")
        ));
    }

    let connection_string = connection_string(&server, &password);
    Ok(ProvisionManagedServerResult {
        server,
        connection_id,
        connection_string,
    })
}

/// All managed servers with status derived live from Docker.
pub async fn list(pool: &SqlitePool) -> Result<Vec<ManagedServerWithStatus>, String> {
    let servers = storage::managed::read_managed_servers(pool).await?;
    let mut result = Vec::with_capacity(servers.len());
    for server in servers {
        let (status, volume_exists) = match docker::container_state(&server.container_name).await {
            Some(state) if state == "running" => ("running".to_string(), true),
            Some(_) => ("stopped".to_string(), true),
            None => (
                "orphaned".to_string(),
                docker::volume_exists(&server.volume_name).await,
            ),
        };
        result.push(ManagedServerWithStatus {
            server,
            status,
            volume_exists,
        });
    }
    Ok(result)
}

async fn require_server(pool: &SqlitePool, id: &str) -> Result<ManagedServer, String> {
    storage::managed::read_managed_server_by_id(pool, id)
        .await?
        .ok_or_else(|| format!("Managed server '{id}' not found"))
}

async fn start_and_wait(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    server: &ManagedServer,
) -> Result<(), String> {
    let connection_id = server
        .connection_id
        .as_deref()
        .ok_or_else(|| "managed server has no linked connection".to_string())?;
    let mut connection = storage::read_connection_by_id(pool, connection_id)
        .await?
        .ok_or_else(|| "the linked connection was deleted".to_string())?;
    credentials::hydrate(pool, mode, &mut connection).await?;
    docker::start_container(&server.container_name).await?;
    wait_until_ready(
        server.engine,
        server.port,
        &server.database,
        &server.user,
        connection.password(),
    )
    .await
}

pub async fn start(pool: &SqlitePool, mode: CredentialStorageMode, id: &str) -> Result<(), String> {
    let server = require_server(pool, id).await?;
    start_and_wait(pool, mode, &server).await
}

pub async fn stop(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let server = require_server(pool, id).await?;
    docker::stop_container(&server.container_name).await
}

/// The one deliberately destructive action (ADR-0019): removes the
/// container AND the data volume, then forgets the record. The linked
/// Connection is left in place — deleting it stays a separate,
/// non-destructive act.
pub async fn destroy(pool: &SqlitePool, id: &str) -> Result<(), String> {
    let server = require_server(pool, id).await?;
    if docker::container_state(&server.container_name)
        .await
        .is_some()
    {
        docker::remove_container(&server.container_name).await?;
    }
    if docker::volume_exists(&server.volume_name).await {
        docker::remove_volume(&server.volume_name).await?;
    }
    storage::managed::delete_managed_server(pool, id).await?;
    Ok(())
}

/// Recover an orphaned server whose volume survived: recreate the
/// container and reattach the volume. Credentials are read back from
/// the linked Connection — the database files in the volume already
/// hold the original users, so the env values only matter on a fresh
/// volume and reusing them keeps the run idempotent.
pub async fn recreate(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    id: &str,
) -> Result<(), String> {
    let server = require_server(pool, id).await?;
    if docker::container_state(&server.container_name)
        .await
        .is_some()
    {
        return Err("container already exists — start it instead".to_string());
    }
    let connection_id = server
        .connection_id
        .clone()
        .ok_or_else(|| "managed server has no linked connection".to_string())?;
    let mut connection = storage::read_connection_by_id(pool, &connection_id)
        .await?
        .ok_or_else(|| {
            "the linked connection was deleted; destroy this record and provision a new server"
                .to_string()
        })?;
    credentials::hydrate(pool, mode, &mut connection).await?;
    let password = connection.password().to_string();

    let spec = engine_spec(server.engine)?;
    if !docker::volume_exists(&server.volume_name).await {
        docker::create_volume(&server.volume_name).await?;
    }
    docker::run_container(&docker::RunContainerSpec {
        name: &server.container_name,
        image: &format!("{}:{}", spec.image_repo, server.version),
        host_port: server.port,
        container_port: spec.container_port,
        volume_name: &server.volume_name,
        volume_mount: spec.volume_mount,
        env: provision_env(server.engine, &server.database, &server.user, &password),
    })
    .await?;
    wait_until_ready(
        server.engine,
        server.port,
        &server.database,
        &server.user,
        &password,
    )
    .await
}

/// Start a stopped Managed Server before its linked Connection is used.
///
/// Returns `true` when a start was required so callers can distinguish
/// a cold boot from an already-running server. Missing containers remain
/// an explicit Orphaned error; connect intent never recreates resources.
pub async fn ensure_running_for_connection(
    pool: &SqlitePool,
    mode: CredentialStorageMode,
    connection: &StoredConnection,
) -> Result<bool, String> {
    let Some(server) =
        storage::managed::read_managed_server_by_connection_id(pool, connection.id()).await?
    else {
        return Ok(false);
    };
    match docker::container_state(&server.container_name).await {
        Some(state) if state == "running" => Ok(false),
        Some(_) => {
            start_and_wait(pool, mode, &server).await?;
            Ok(true)
        }
        None => Err(format!(
            "Managed server '{}' is orphaned; recreate it under Settings → Local Databases",
            server.name
        )),
    }
}

#[cfg(test)]
mod live_tests {
    //! End-to-end provisioning against the real Docker daemon.
    //! Ignored by default; run with:
    //! `cargo test --manifest-path src-tauri/Cargo.toml managed_live -- --ignored`

    use super::*;
    use crate::storage::{open_pool, Paths};

    #[tokio::test]
    #[ignore = "requires a running Docker daemon; pulls postgres:16 if absent"]
    async fn managed_live_provision_lifecycle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = open_pool(&Paths::from_dir(dir.path().to_path_buf()))
            .await
            .expect("open_pool");
        let mode = CredentialStorageMode::PlainSqlite;

        let result = provision(
            &pool,
            mode,
            ProvisionManagedServerPayload {
                name: "dbunk live test".into(),
                engine: DatabaseEngine::PostgreSQL,
                version: "16".into(),
                port: None,
                database: None,
                user: None,
            },
        )
        .await
        .expect("provision");

        let server_id = result.server.id.clone();
        assert!(result.connection_string.starts_with("postgres://"));
        assert!(result.server.port >= PG_PORT_BASE);

        // The auto-created Connection exists and its credential
        // round-trips through the credential backend.
        let mut connection = storage::read_connection_by_id(&pool, &result.connection_id)
            .await
            .expect("read connection")
            .expect("connection exists");
        credentials::hydrate(&pool, mode, &mut connection)
            .await
            .expect("hydrate");
        assert!(!connection.password().is_empty());

        // Status derives live from Docker: running → stopped → running.
        let status_of = |servers: Vec<ManagedServerWithStatus>| {
            servers
                .into_iter()
                .find(|s| s.server.id == server_id)
                .expect("server listed")
        };
        assert_eq!(
            status_of(list(&pool).await.expect("list")).status,
            "running"
        );

        stop(&pool, &server_id).await.expect("stop");
        assert_eq!(
            status_of(list(&pool).await.expect("list")).status,
            "stopped"
        );

        start(&pool, mode, &server_id).await.expect("start");
        assert_eq!(
            status_of(list(&pool).await.expect("list")).status,
            "running"
        );

        // External removal → orphaned with surviving volume → recreate.
        docker::remove_container(&result.server.container_name)
            .await
            .expect("external rm");
        let orphaned = status_of(list(&pool).await.expect("list"));
        assert_eq!(orphaned.status, "orphaned");
        assert!(orphaned.volume_exists);

        recreate(&pool, mode, &server_id).await.expect("recreate");
        assert_eq!(
            status_of(list(&pool).await.expect("list")).status,
            "running"
        );

        // Destroy removes container, volume, and record.
        destroy(&pool, &server_id).await.expect("destroy");
        assert!(docker::container_state(&result.server.container_name)
            .await
            .is_none());
        assert!(!docker::volume_exists(&result.server.volume_name).await);
        assert!(list(&pool)
            .await
            .expect("list")
            .iter()
            .all(|s| s.server.id != server_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_produces_safe_identifiers() {
        assert_eq!(slugify("My Cool Project!"), "my_cool_project");
        assert_eq!(slugify("  "), "app");
        assert_eq!(slugify("123db"), "_123db");
        assert_eq!(slugify("payments-api"), "payments_api");
    }

    #[test]
    fn pick_port_skips_claimed_ports() {
        // Bind a port ourselves so the scan must skip past it.
        let holder = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let held = holder.local_addr().expect("addr").port();
        let picked = pick_port(&[], held).expect("pick");
        assert_ne!(picked, held);
        let picked = pick_port(&[held + 1], held).expect("pick");
        assert_ne!(picked, held);
        assert_ne!(picked, held + 1);
    }

    #[test]
    fn connection_string_shapes_per_engine() {
        let server = ManagedServer {
            id: "x".into(),
            name: "demo".into(),
            engine: DatabaseEngine::PostgreSQL,
            version: "17".into(),
            port: 5433,
            container_name: "dbunk-demo".into(),
            volume_name: "dbunk-demo-data".into(),
            database: "demo".into(),
            user: "demo".into(),
            connection_id: None,
            created_at: "now".into(),
        };
        assert_eq!(
            connection_string(&server, "pw"),
            "postgres://demo:pw@127.0.0.1:5433/demo"
        );
    }
}
