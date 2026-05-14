# ADR-0013 — Driver-level fields on the Postgres connection record

**Status**: Proposed (2026-05-14)

## Context

The Settings tab is read-mostly because `PgStoredConnection` only
models the bare credential set (host/port/user/password/database/ssl).
Power users expect a connection to remember knobs that DBeaver and
JetBrains DataGrip surface in the connection dialog:

- SSH tunnel (host + port + user + key/password)
- TCP keepalive interval
- `statement_timeout` and `idle_in_transaction_session_timeout`
- Default `search_path`
- Default role (`SET ROLE` on connect)
- TCP connect timeout

Today these are absent from the type, the SQLite schema, the form,
and the runtime — adding any one of them touches all four layers
plus the engine-policy table. `PENDING_TASKS.md §"Connection Settings
tab"` and `ROADMAP.md §1` both flag this.

## Decision

Add an optional `driver_options` sub-struct on `PgStoredConnection`
(`src-tauri/src/types.rs`) holding the canonical knob list:

```rust
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct PgDriverOptions {
    pub statement_timeout_ms: Option<u32>,
    pub idle_in_transaction_timeout_ms: Option<u32>,
    pub connect_timeout_ms: Option<u32>,
    pub keepalive_seconds: Option<u32>,
    pub default_search_path: Option<Vec<String>>,
    pub default_role: Option<String>,
    pub ssh_tunnel: Option<SshTunnel>,
}
```

The connection-pool builder (`postgres::connect`) reads these and:

- Calls `SET statement_timeout`, `SET idle_in_transaction_session_timeout`, `SET search_path`, `SET ROLE` after every `connect()`.
- Threads `keepalive_seconds` into `PgConnectOptions::keepalives_idle`.
- Threads `connect_timeout_ms` into the existing connect deadline.
- Routes through the SSH tunnel when configured (see §SSH below).

**SSH tunnel is its own follow-up ADR.** It introduces a credential
type (passphrase / key path / key contents), a tunnel lifecycle, and
process-cache concerns that don't compose into the same SQLite blob
strategy as today's password (ADR-0005). This ADR reserves the field
on the struct so the migration is one-shot; the runtime path stays
unreachable until that follow-up lands.

**SQLite migration**: add a single nullable `driver_options TEXT` column
that holds the serialized blob. Existing rows backfill to `NULL` →
treated as `PgDriverOptions::default()`. No data rewrite needed.

**Form**: extends `ConnectionFormPolicy::host-auth` with an "Advanced"
expander. The form continues to read the policy union (ADR-0012); no
new `selectedEngine ===` checks.

## Consequences

- One Phase 1 commit covers struct + migration + plumbing into
  `connect()`. The form expander is a Phase 2 commit. Per-field UI
  polish (e.g., `search_path` token picker) lives in follow-ups.
- Settings tab becomes editable: drop the "read-only mirror" comment
  in `settings-tab.tsx`, render the advanced fields, route saves
  through `updateConnection`.
- `connectionFormPolicy` types grow optional driver-options keys; the
  ClickHouse / SQLite / Redis variants ignore them. MySQL gets its
  own ADR if/when it adopts the same shape.
- The four `SET ...` statements run on every new connection. Cost is
  one round-trip; cached pool connections inherit by virtue of the
  pool's `after_connect` hook.

## Alternatives considered

1. **Per-field flat columns.** Six new SQLite columns instead of one
   JSON blob. Rejected — every future knob is a migration; harder to
   keep tests, forms, and Rust types in sync.
2. **Connection-scoped global config file.** Live in a TOML next to
   the keychain. Rejected — divorces the knobs from the connection
   record they belong to and complicates the cascade in
   `deleteConnection`.
3. **Run-time-only flags applied per query.** Pass `statement_timeout`
   on the `runQuery` payload. Rejected — works for timeout but not
   for `search_path` (which the editor's autocomplete reads at
   connect time) or SSH (which has to be established before the
   first byte goes over the wire).

## Related

- ADR-0010 — backend `StoredConnection` tagged enum. This ADR adds an
  optional field to one variant.
- ADR-0012 — unified `ConnectionForm` + policy union. This ADR
  extends the `host-auth` policy with optional driver fields.
- `PENDING_TASKS.md §Connection Settings tab` — the tracked debt
  this resolves.
