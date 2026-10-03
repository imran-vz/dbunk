//! Synthetic baseline-format profile corpus. Created from the frozen baseline
//! migrations with synthetic values only; never derived from a real profile.
//!
//! What the baseline actually persisted in SQLite: connections (organization,
//! TLS/driver/policy/SSH columns), bastions, managed servers, credentials and
//! verifier, history, saved SQL/Redis commands, schema-map positions/prefs,
//! table grid prefs, virtual keys, safety overrides, `app_settings` (theme,
//! credential mode) and frontend `ui.v1.*` keys (session tabs with hot-exit SQL,
//! carets and designer drafts, export tasks, grid layouts, panels, palette
//! frecency). Theme/density boot caches stayed in browser localStorage and are
//! not part of this corpus (Plan 030 R02). Connection order was derived from
//! names, not stored.

use sqlx::{Connection, SqliteConnection};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CredentialFixture {
    /// Default corpus: synthetic secrets stored in plain SQLite.
    PlainSqlite,
    /// Metadata-only: opaque synthetic verifier and ciphertext, never decrypted.
    EncryptedMetadata,
    /// Metadata-only: the mode setting with no secrets; no Keychain is touched.
    KeychainMetadata,
    NotOnboarded,
}

pub(super) const PG_PRIMARY: &str = "c0ffee00-0000-4000-8000-000000000001";
pub(super) const PG_BASTION: &str = "c0ffee00-0000-4000-8000-000000000002";
pub(super) const PG_UNKNOWN_FIELD: &str = "c0ffee00-0000-4000-8000-000000000003";
pub(super) const MYSQL: &str = "c0ffee00-0000-4000-8000-000000000004";
pub(super) const REDIS: &str = "c0ffee00-0000-4000-8000-000000000005";
pub(super) const BASTION: &str = "b0570000-0000-4000-8000-000000000001";
pub(super) const PRIMARY_SECRET: &str = "synthetic-primary-secret";
pub(super) const BASTION_SECRET: &str = "synthetic-bastion-passphrase";
pub(super) const UNKNOWN_DRIVER_OPTIONS: &str =
    r#"{"connectTimeoutMs":4000,"futureKnob":{"nested":[1,2,3]}}"#;
pub(super) const QUERY_SQL: &str = "select 'ünïcode 😀' as greeting;\nselect 2;";
pub(super) const UNKNOWN_UI_KEY: &str = "ui.v1.futurePanel.layout";
pub(super) const UNKNOWN_UI_VALUE: &str = r#"{"version":7,"unknown":["kept"]}"#;
pub(super) const TIMESTAMP: &str = "2026-05-01T10:00:00+00:00";

/// Opens a baseline-style writer: WAL mode, as the baseline host used, with
/// automatic checkpoints disabled so committed content can stay in the WAL.
pub(super) async fn open_writer(path: &Path) -> SqliteConnection {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true)
        .pragma("wal_autocheckpoint", "0");
    SqliteConnection::connect_with(&options).await.unwrap()
}

/// Builds the complete corpus through `connection`.
pub(super) async fn build(connection: &mut SqliteConnection, credentials: CredentialFixture) {
    super::apply_baseline(connection).await.unwrap();
    let pg_tls = r#"{"mode":"verify-full","rootCertPath":"/synthetic/ca.pem","serverName":"db.synthetic.invalid"}"#;
    let pg_driver = r#"{"connectTimeoutMs":5000,"statementTimeoutMs":30000,"defaultSearchPath":["app","public"]}"#;
    /// ID, name, engine, environment, TLS options and driver options.
    type Row<'a> = (
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        Option<&'a str>,
        Option<&'a str>,
    );
    let connections: [Row; 5] = [
        (
            PG_PRIMARY,
            "Alpha primary",
            "PostgreSQL",
            "production",
            Some(pg_tls),
            Some(pg_driver),
        ),
        (
            PG_BASTION,
            "Bravo via bastion",
            "PostgreSQL",
            "staging",
            None,
            None,
        ),
        (
            PG_UNKNOWN_FIELD,
            "Charlie future knob",
            "PostgreSQL",
            "development",
            None,
            Some(UNKNOWN_DRIVER_OPTIONS),
        ),
        (MYSQL, "Delta MySQL", "MySQL", "test", None, None),
        (REDIS, "Echo Redis", "Redis", "development", None, None),
    ];
    for (index, (id, name, engine, environment, tls, driver)) in connections.into_iter().enumerate()
    {
        sqlx::query(
            "INSERT INTO connections (id, name, database_name, engine, host, port, user_name, role,
               last_activity_at, ssl, tls_options, driver_options, read_only, environment, safe_mode,
               folder, is_favorite, color, db_number, use_tls)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'read/write', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(format!("synthetic_db_{index}"))
        .bind(engine)
        .bind(format!("host{index}.synthetic.invalid"))
        .bind(5432 + index as i64)
        .bind(format!("synthetic_user_{index}"))
        .bind((index % 2 == 0).then_some(TIMESTAMP))
        .bind(i64::from(index != 2))
        .bind(tls)
        .bind(driver)
        .bind(i64::from(id == PG_PRIMARY))
        .bind(environment)
        .bind(if id == PG_PRIMARY { "strict" } else { "inherit" })
        .bind(if index < 2 { "Production" } else { "" })
        .bind(i64::from(index == 0 || index == 3))
        .bind(["red", "", "green", "blue", ""][index])
        .bind(if engine == "Redis" { 3 } else { 0 })
        .bind(i64::from(engine == "Redis"))
        .execute(&mut *connection)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO bastion_servers (id, name, host, port, user_name, auth_method, private_key_path,
           host_key_fingerprint, created_at, updated_at)
         VALUES (?, 'Synthetic jump', 'jump.synthetic.invalid', 22, 'synthetic_jump', 'privateKeyPath',
           '/synthetic/id_ed25519', 'SHA256:c3ludGhldGljLWZpbmdlcnByaW50', ?, ?)",
    )
    .bind(BASTION)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(&mut *connection)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE connections SET ssh_tunnel_enabled = 1, ssh_tunnel_bastion_server_id = ?,
           ssh_tunnel_local_bind_host = '127.0.0.1', ssh_tunnel_local_port = 15433,
           ssh_tunnel_compression = 1, ssh_tunnel_keepalive_interval_seconds = 30,
           ssh_tunnel_jump_chain = ?, ssh_tunnel_proxy_command = NULL
         WHERE id = ?",
    )
    .bind(BASTION)
    .bind(format!(r#"["{BASTION}"]"#))
    .bind(PG_BASTION)
    .execute(&mut *connection)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO managed_servers (id, name, engine, version, port, container_name, volume_name,
           database_name, user_name, connection_id, created_at)
         VALUES ('managed-synthetic-1', 'Synthetic managed', 'PostgreSQL', '16', 25432,
           'dbunk-synthetic-pg', 'dbunk-synthetic-volume', 'synthetic_db_0', 'synthetic_user_0', ?, ?)",
    )
    .bind(PG_PRIMARY)
    .bind(TIMESTAMP)
    .execute(&mut *connection)
    .await
    .unwrap();
    for (index, status) in ["success", "error", "success"].into_iter().enumerate() {
        sqlx::query(
            "INSERT INTO query_history (id, sql, connection_id, connection_name, database_name, engine,
               status, error_message, runtime_ms, row_count, started_at)
             VALUES (?, ?, ?, 'Alpha primary', 'synthetic_db_0', 'PostgreSQL', ?, ?, ?, ?, ?)",
        )
        .bind(format!("history-{index}"))
        .bind(format!("select {index};"))
        .bind(PG_PRIMARY)
        .bind(status)
        .bind((status == "error").then_some("synthetic failure"))
        .bind(10 + index as i64)
        .bind((status == "success").then_some(index as i64))
        .bind(format!("2026-05-0{}T10:00:00+00:00", index + 1))
        .execute(&mut *connection)
        .await
        .unwrap();
    }
    for (id, connection_id, favorite) in [
        ("saved-1", Some(PG_PRIMARY), 1),
        ("saved-2", None::<&str>, 0),
    ] {
        sqlx::query(
            "INSERT INTO saved_queries (id, name, body, connection_id, is_favorite, owner_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, NULL, ?, ?)",
        )
        .bind(id)
        .bind(format!("Synthetic {id}"))
        .bind(format!("select '{id}';"))
        .bind(connection_id)
        .bind(favorite)
        .bind(TIMESTAMP)
        .bind(TIMESTAMP)
        .execute(&mut *connection)
        .await
        .unwrap();
    }
    let statements = [
        format!(
            "INSERT INTO schema_map_positions VALUES ('{PG_PRIMARY}', 'public', 'users', 12.5, -40.25, '{TIMESTAMP}'),
               ('{PG_PRIMARY}', 'public', 'orders', 0.1, 1e-7, '{TIMESTAMP}')"
        ),
        format!(
            "INSERT INTO schema_map_prefs VALUES ('{PG_PRIMARY}', 'public', 'orthogonal', 'keys', 0, 1, 1, '{TIMESTAMP}')"
        ),
        format!(
            r#"INSERT INTO table_grid_prefs VALUES ('{PG_PRIMARY}', 'public', 'users', '{{"version":1,"columns":{{"email":{{"width":220}}}},"futurePref":true}}', '{TIMESTAMP}')"#
        ),
        format!(
            r#"INSERT INTO virtual_keys VALUES ('{PG_PRIMARY}', 'public', 'events', '{{"version":1,"columns":["tenant_id","event_id"]}}', '{TIMESTAMP}')"#
        ),
        format!(
            "INSERT INTO safety_overrides (id, connection_id, command, classes, occurred_at) VALUES
               (7, '{PG_PRIMARY}', 'query', '[\"write\"]', '{TIMESTAMP}'),
               (9, '{PG_PRIMARY}', 'ddl', '[\"ddl\",\"destructive\"]', '{TIMESTAMP}')"
        ),
        format!(
            "INSERT INTO redis_cli_history VALUES ('redis-history-1', '{REDIS}', 'GET synthetic:key', '{TIMESTAMP}')"
        ),
        format!(
            "INSERT INTO saved_redis_commands VALUES ('redis-saved-1', 'Synthetic ping', 'PING', '{REDIS}', 1, '{TIMESTAMP}', '{TIMESTAMP}')"
        ),
    ];
    for statement in statements {
        sqlx::query(&statement)
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    let session = serde_json::json!({
        "tabs": [
            {"id": "tab-query-1", "kind": "query", "label": "Greeting", "connectionId": PG_PRIMARY,
             "schema": "public", "query": QUERY_SQL, "pinned": true, "isDirty": true,
             "caret": {"line": 1, "column": 21, "anchorLine": 1, "anchorColumn": 8},
             "futureTabField": {"kept": true}},
            {"id": "tab-query-2", "kind": "query", "label": "Deleted binding",
             "connectionId": "c0ffee00-0000-4000-8000-0000000000ff", "schema": "", "query": "select 'orphan';"},
            {"id": "tab-query-mysql", "kind": "query", "label": "MySQL draft", "connectionId": MYSQL,
             "schema": "", "query": "select 'mysql';"},
            {"id": "tab-table-1", "kind": "table", "label": "users", "connectionId": PG_PRIMARY,
             "schema": "public", "table": "users"},
            {"id": "tab-designer-1", "kind": "table-designer", "label": "New table", "connectionId": PG_PRIMARY,
             "schema": "public", "tableDesignerDraft": {"name": "synthetic_new", "columns": []}},
        ],
        "activeTabId": "tab-query-1",
        "expandedSchemas": [format!("{PG_PRIMARY}:public")],
        "expandedNavigatorGroups": ["Production"],
    })
    .to_string();
    let export_tasks = serde_json::json!([{
        "id": format!("{PG_PRIMARY}:public:users:1714557600000"), "name": "public.users CSV export",
        "scope": {"connectionId": PG_PRIMARY, "schema": "public", "table": "users"},
        "format": "csv", "encoding": "utf-8", "compression": "none", "nullAs": "", "createdAt": TIMESTAMP
    }])
    .to_string();
    let ui_state = [
        ("ui.v1.migrated".to_string(), "1".to_string()),
        ("ui.v1.session".into(), session),
        ("ui.v1.exportTasks.v1".into(), export_tasks),
        (
            format!("ui.v1.grid.layout.{PG_PRIMARY}.public.users"),
            r#"{"widths":{"email":220.5},"pinned":["id"]}"#.into(),
        ),
        ("ui.v1.workbench.navigator".into(), r#"{"size":280}"#.into()),
        (
            "ui.v1.palette.frecency".into(),
            r#"{"connection:alpha":3}"#.into(),
        ),
        (UNKNOWN_UI_KEY.into(), UNKNOWN_UI_VALUE.into()),
    ];
    for (key, value) in ui_state {
        sqlx::query("INSERT INTO ui_state (key, value, updated_at) VALUES (?, ?, ?)")
            .bind(key)
            .bind(value)
            .bind(TIMESTAMP)
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    let mut settings = vec![
        ("theme", "dark".to_string()),
        ("themePreset", "dracula".into()),
        ("futureSetting.v9", r#"{"opaque":true}"#.into()),
    ];
    let bastion_secret = crate::credentials::bastion_secret_id(BASTION, "passphrase");
    match credentials {
        CredentialFixture::PlainSqlite => {
            settings.push(("onboardingCompleted", "true".into()));
            settings.push(("credentialStorageMode", "plain-sqlite".into()));
            for (id, secret) in [
                (PG_PRIMARY, PRIMARY_SECRET),
                (&bastion_secret, BASTION_SECRET),
            ] {
                credential(connection, id, "plain-sqlite", None, secret).await;
            }
            // A stale row from an earlier mode, which the baseline ignores.
            credential(
                connection,
                MYSQL,
                "encrypted-sqlite",
                Some("bm9uY2U="),
                "c3RhbGU=",
            )
            .await;
        }
        CredentialFixture::EncryptedMetadata => {
            settings.push(("onboardingCompleted", "true".into()));
            settings.push(("credentialStorageMode", "encrypted-sqlite".into()));
            sqlx::query(
                "INSERT INTO credential_verifier VALUES (1, 'argon2id-v1', 'c3ludGhldGljLXNhbHQ=',
                   'c3ludGhldGljLW5vbmNl', 'b3BhcXVlLXZlcmlmaWVy', ?)",
            )
            .bind(TIMESTAMP)
            .execute(&mut *connection)
            .await
            .unwrap();
            for id in [PG_PRIMARY, bastion_secret.as_str()] {
                credential(
                    connection,
                    id,
                    "encrypted-sqlite",
                    Some("b3BhcXVlLW5vbmNl"),
                    "b3BhcXVl",
                )
                .await;
            }
        }
        CredentialFixture::KeychainMetadata => {
            settings.push(("onboardingCompleted", "true".into()));
            settings.push(("credentialStorageMode", "keychain".into()));
        }
        CredentialFixture::NotOnboarded => {}
    }
    for (key, value) in settings {
        sqlx::query("INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, ?)")
            .bind(key)
            .bind(value)
            .bind(TIMESTAMP)
            .execute(&mut *connection)
            .await
            .unwrap();
    }
}

async fn credential(
    connection: &mut SqliteConnection,
    id: &str,
    mode: &str,
    nonce: Option<&str>,
    value: &str,
) {
    sqlx::query(
        "INSERT INTO credentials (credential_id, storage_mode, nonce, password_value, updated_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(mode)
    .bind(nonce)
    .bind(value)
    .bind(TIMESTAMP)
    .execute(&mut *connection)
    .await
    .unwrap();
}

/// A complete corpus file, closed cleanly (WAL checkpointed) at `path`.
pub(super) async fn create(path: &Path, credentials: CredentialFixture) {
    let mut connection = open_writer(path).await;
    build(&mut connection, credentials).await;
    connection.close().await.unwrap();
}

/// Marks a corpus as written by a newer, unknown host version.
pub(super) async fn add_future_version(path: &Path) {
    let mut connection = open_writer(path).await;
    sqlx::query("INSERT INTO schema_migrations (version, applied_at) VALUES (19, ?)")
        .bind(TIMESTAMP)
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
}
