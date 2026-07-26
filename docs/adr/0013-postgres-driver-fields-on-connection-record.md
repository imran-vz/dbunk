# ADR-0013 — Driver-level fields on the Postgres connection record

**Status**: Accepted (2026-05-14) — Phase 2 landed 2026-07-27

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

- Calls `SET statement_timeout`, `SET idle_in_transaction_session_timeout`, `SET search_path`, `SET ROLE` after every successful handshake (shipped).
- Bounds the initial handshake with `connect_timeout_ms` by wrapping
  `connect_with` in `tokio::time::timeout` (shipped, Phase 2). It is
  deliberately *not* mapped onto sqlx's `acquire_timeout`: that clock
  also covers waiting for a free slot on a saturated pool, so a short
  connect deadline would start failing healthy queries. The cost of
  the narrower mapping is that connections the pool opens later, on an
  endpoint already proven reachable, are unbounded.
- Reserves `keepalive_seconds` on the struct but does not apply it —
  sqlx 0.8's `PgConnectOptions` exposes no socket keepalive setter. It
  round-trips through the form so a save can't wipe it, but ships **no
  control**: a knob that silently does nothing is worse than an absent
  one.
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

**Form**: extends `ConnectionFormPolicy::host-auth` with an
`showDriverOptions` flag, read inside the existing "Advanced Options"
expander. The form continues to read the policy union (ADR-0012); no
new `selectedEngine ===` checks. PG sets it `true`, MySQL `false` —
they share the form `kind` but only PG carries the field.

`default_search_path` is typed as free comma-separated text and split
by `parseSearchPath`; the backend already double-quotes each entry, so
validation only rejects blank entries and embedded `"`.

## Consequences

- One Phase 1 commit covers struct + migration + plumbing into
  `connect()`. The form expander is a Phase 2 commit. Per-field UI
  polish (e.g., `search_path` token picker) lives in follow-ups.
- Settings tab renders the configured knobs alongside the other
  connection fields, and the "Phase 1 introduces no new fields"
  comment is gone. **Deviation from the original decision**: it stays
  a read-only mirror rather than becoming a second inline editor.
  `<ConnectionForm>` is the single construction site for the
  `StoredConnection` wire shape (ADR-0012) and the tab's Edit button
  already routes saves through it into `updateConnection` — a parallel
  editor would duplicate the builder the union depends on. The knobs
  block is omitted entirely when nothing is set, so the grid doesn't
  grow five permanent dashes for every Postgres connection.
- Because the form now carries every knob in its own state, the
  post-hoc `driverOptions` merge in `use-connection-form.ts` is gone;
  the blob survives an edit save because the form round-trips it, not
  because a special case re-attaches it.
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
- ADR-0018 — first-class Bastion Servers and SSH tunnels. The
  follow-up this ADR reserved the `ssh_tunnel` slot for; it landed as
  its own field on the connection record rather than inside
  `driver_options`.
- `ROADMAP.md §1 "Settings" tab` — the tracked debt this resolves.
  (The `PENDING_TASKS.md` referenced above no longer exists; its
  contents folded into ROADMAP.)
