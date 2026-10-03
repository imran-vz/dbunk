//! Frozen copy of the baseline Tauri storage migrations at `102568b`
//! (`backend/src/storage.rs`, versions 1-18). The importer validates a legacy
//! snapshot against this schema independently of later native migrations, and
//! test corpora are created from it rather than from current storage code.

pub(super) const BASELINE_SCHEMA_VERSION: i64 = 18;

pub(super) const BASELINE_MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
CREATE TABLE app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE connections (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  database_name TEXT NOT NULL,
  engine TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  user_name TEXT NOT NULL,
  role TEXT NOT NULL,
  last_activity_at TEXT,
  use_https INTEGER NOT NULL DEFAULT 0,
  url_path TEXT NOT NULL DEFAULT ''
);

CREATE TABLE credentials (
  connection_id TEXT PRIMARY KEY REFERENCES connections(id) ON DELETE CASCADE,
  storage_mode TEXT NOT NULL,
  nonce TEXT,
  password_value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE credential_verifier (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  kdf TEXT NOT NULL,
  salt TEXT NOT NULL,
  nonce TEXT NOT NULL,
  ciphertext TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE query_history (
  id TEXT PRIMARY KEY,
  sql TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  connection_name TEXT NOT NULL,
  database_name TEXT NOT NULL,
  engine TEXT NOT NULL,
  status TEXT NOT NULL,
  error_message TEXT,
  runtime_ms INTEGER NOT NULL,
  row_count INTEGER,
  started_at TEXT NOT NULL
);

CREATE INDEX idx_query_history_started_at ON query_history(started_at DESC);

CREATE TABLE saved_queries (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  body TEXT NOT NULL,
  connection_id TEXT,
  is_favorite INTEGER NOT NULL DEFAULT 0,
  owner_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_saved_queries_updated_at ON saved_queries(updated_at DESC);
"#,
    ),
    (
        2,
        // Redis-specific columns on the shared `connections` table. Mirror the
        // pattern ClickHouse already uses for `use_https`/`url_path`: nullable
        // / default-valued, ignored by engines that don't need them.
        r#"
ALTER TABLE connections ADD COLUMN db_number INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN use_tls INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN verify_tls_cert INTEGER NOT NULL DEFAULT 1;
"#,
    ),
    (
        3,
        // PG/MySQL TLS-on-the-wire toggle. Default 1 matches the
        // previously-hidden form default; existing connections continue
        // to negotiate TLS the way they did before. The column lives on
        // the shared `connections` table because the SQLite schema stays
        // flat (per ADR-0010); engines other than PG/MySQL simply ignore
        // the column when their variant is constructed.
        r#"
ALTER TABLE connections ADD COLUMN ssl INTEGER NOT NULL DEFAULT 1;
"#,
    ),
    (
        4,
        r#"
CREATE TABLE schema_map_positions (
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema        TEXT NOT NULL,
  table_id      TEXT NOT NULL,
  x             REAL NOT NULL,
  y             REAL NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_id)
);

CREATE TABLE schema_map_prefs (
  connection_id  TEXT    NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema         TEXT    NOT NULL,
  routing        TEXT    NOT NULL DEFAULT 'bezier',
  attr_mode      TEXT    NOT NULL DEFAULT 'all',
  show_types     INTEGER NOT NULL DEFAULT 1,
  show_nulls     INTEGER NOT NULL DEFAULT 0,
  show_comments  INTEGER NOT NULL DEFAULT 0,
  updated_at     TEXT    NOT NULL,
  PRIMARY KEY (connection_id, schema)
);
"#,
    ),
    (
        5,
        // ADR-0013: optional Postgres driver/session knobs serialised
        // as one JSON blob. Adding a new knob is a struct field, not
        // a schema migration. Engines other than PG ignore the
        // column (it stays NULL on their rows).
        r#"
ALTER TABLE connections ADD COLUMN driver_options TEXT;
"#,
    ),
    (
        6,
        // Redis CLI command history. One row per submitted command,
        // scoped to the connection. Results are NOT persisted — only
        // the command text + when it was submitted, mirroring the
        // shell-history idea. Capped globally at 1000 rows via
        // `REDIS_CLI_HISTORY_CAP` (trimmed on every insert).
        r#"
CREATE TABLE redis_cli_history (
  id            TEXT NOT NULL PRIMARY KEY,
  connection_id TEXT NOT NULL,
  command       TEXT NOT NULL,
  submitted_at  TEXT NOT NULL
);

CREATE INDEX idx_redis_cli_history_connection_submitted_at
  ON redis_cli_history(connection_id, submitted_at DESC);
"#,
    ),
    (
        7,
        // Belt-and-braces Redis read-only toggle (ADR-0009). Default
        // 0 (not read-only). When 1, `assert_writable` rejects writes
        // without even consulting the replica-role cache.
        r#"
ALTER TABLE connections ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0;
"#,
    ),
    (
        8,
        // Saved Redis CLI commands — analogous to `saved_queries` for
        // SQL, but the connection ref is optional so users can save
        // engine-portable commands. Parameter substitution is
        // deferred — `body` is treated as-is by the CLI at load time.
        r#"
CREATE TABLE saved_redis_commands (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  body          TEXT NOT NULL,
  connection_id TEXT,
  is_favorite   INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE INDEX idx_saved_redis_commands_updated_at
  ON saved_redis_commands(updated_at DESC);
"#,
    ),
    (
        9,
        // ADR-0018: the credentials backend now stores database
        // passwords and bastion secrets. Existing rows are database
        // password credentials keyed by connection id; moving to a
        // generic credential_id preserves them while removing the
        // connection FK that would block bastion-secret keys.
        r#"
CREATE TABLE credentials_new (
  credential_id TEXT PRIMARY KEY,
  storage_mode TEXT NOT NULL,
  nonce TEXT,
  password_value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

INSERT INTO credentials_new (credential_id, storage_mode, nonce, password_value, updated_at)
SELECT connection_id, storage_mode, nonce, password_value, updated_at FROM credentials;

DROP TABLE credentials;

ALTER TABLE credentials_new RENAME TO credentials;
"#,
    ),
    (
        10,
        // ADR-0018 first slice: first-class bastion servers plus
        // per-connection SSH tunnel config on network-backed engines.
        // SQLite ignores these shared-table columns because it has no
        // network transport and no SshTunnelConfig field.
        r#"
CREATE TABLE bastion_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  user_name TEXT NOT NULL,
  auth_method TEXT NOT NULL,
  private_key_path TEXT,
  host_key_fingerprint TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_bastion_servers_name ON bastion_servers(name COLLATE NOCASE);

ALTER TABLE connections ADD COLUMN ssh_tunnel_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN ssh_tunnel_bastion_server_id TEXT;
ALTER TABLE connections ADD COLUMN ssh_tunnel_local_bind_host TEXT;
ALTER TABLE connections ADD COLUMN ssh_tunnel_local_port INTEGER;
"#,
    ),
    (
        11,
        // ADR-0018 deferred polish: advanced per-Connection SSH
        // Tunnel options. Jump chains store Bastion Server IDs as JSON
        // so the first-class Bastion records and their separate secret
        // namespace remain the source of truth for every hop.
        r#"
ALTER TABLE connections ADD COLUMN ssh_tunnel_compression INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN ssh_tunnel_keepalive_interval_seconds INTEGER;
ALTER TABLE connections ADD COLUMN ssh_tunnel_keepalive_want_reply INTEGER NOT NULL DEFAULT 1;
ALTER TABLE connections ADD COLUMN ssh_tunnel_jump_chain TEXT;
ALTER TABLE connections ADD COLUMN ssh_tunnel_proxy_command TEXT;
"#,
    ),
    (
        12,
        // ADR-0019: Managed Servers — Docker-provisioned local
        // databases. The Connection link is one-way (managed server →
        // connection_id); status is never stored, it is derived live
        // from Docker.
        r#"
CREATE TABLE managed_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  engine TEXT NOT NULL,
  version TEXT NOT NULL,
  port INTEGER NOT NULL,
  container_name TEXT NOT NULL,
  volume_name TEXT NOT NULL,
  database_name TEXT NOT NULL,
  user_name TEXT NOT NULL,
  connection_id TEXT,
  created_at TEXT NOT NULL
);

CREATE INDEX idx_managed_servers_name ON managed_servers(name COLLATE NOCASE);
"#,
    ),
    (
        13,
        // ADR-0022: opaque per-table Grid Preferences JSON for Table Browse.
        r#"
CREATE TABLE table_grid_prefs (
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema        TEXT NOT NULL,
  table_name    TEXT NOT NULL,
  prefs         TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_name)
);
"#,
    ),
    (
        14,
        // ADR-0023: user-selected ordered column sets for relations whose
        // catalog identity cannot safely identify the projected result.
        r#"
CREATE TABLE virtual_keys (
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  schema        TEXT NOT NULL,
  table_name    TEXT NOT NULL,
  virtual_key   TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  PRIMARY KEY (connection_id, schema, table_name)
);
"#,
    ),
    (
        15,
        // ADR-0024: shared Connection policy fields plus the bounded,
        // class-labels-only audit of deliberate safety overrides.
        r#"
ALTER TABLE connections ADD COLUMN environment TEXT NOT NULL DEFAULT 'development';
ALTER TABLE connections ADD COLUMN safe_mode TEXT NOT NULL DEFAULT 'inherit';

CREATE TABLE safety_overrides (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  connection_id TEXT NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
  command       TEXT NOT NULL,
  classes       TEXT NOT NULL,
  occurred_at   TEXT NOT NULL
);

CREATE INDEX idx_safety_overrides_connection_occurred_at
  ON safety_overrides(connection_id, occurred_at DESC, id DESC);
"#,
    ),
    (
        16,
        // UI refresh P8: namespaced, versioned `ui.v1.*` key/value store
        // for layout state and per-connection content state (session
        // restore, hot-exit SQL, grid layouts). Values are opaque JSON
        // owned by the frontend; corrupt values fall back to defaults
        // there.
        r#"
CREATE TABLE ui_state (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
"#,
    ),
    (
        17,
        // Plan 009 (PAR-005): connection organization. `folder` is a
        // single-level group name (empty = ungrouped); `color` is an
        // opaque presentation token validated frontend-side.
        r#"
ALTER TABLE connections ADD COLUMN folder TEXT NOT NULL DEFAULT '';
ALTER TABLE connections ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN color TEXT NOT NULL DEFAULT '';
"#,
    ),
    (
        18,
        // ADR-0025 (Plan 011, PAR-006): PostgreSQL TLS mode and
        // certificate paths as one JSON blob, mirroring migration 5.
        // NULL on legacy rows and on every non-PostgreSQL engine; legacy
        // rows keep resolving through `ssl`.
        r#"
ALTER TABLE connections ADD COLUMN tls_options TEXT;
"#,
    ),
];
