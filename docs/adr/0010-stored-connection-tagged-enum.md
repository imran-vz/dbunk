# ADR-0010 — `StoredConnection` is a serde-tagged enum; `ssl` lives on PG/MySQL only

**Status**: Accepted (2026-05-12)

## Context

`StoredConnection` was a flat `struct` carrying every engine-specific
field as an `Option<>`/`#[serde(default)]`: `use_https`/`url_path`
applied only to ClickHouse, `db_number`/`use_tls`/`verify_tls_cert`
applied only to Redis, and every other engine left them at their
neutral defaults. Reading any of those fields on the wrong engine
silently returned the default — a class of bug that the type system
should be catching at compile time.

A separate but related problem: PostgreSQL and MySQL drive their TLS
posture from a hidden form default (`ssl: true` in
`new-connection-form.tsx`) that was never persisted or threaded into
the driver. The toggle was dead UI; users who needed to disable TLS
had no surface to do it.

This ADR settles two questions together because they share a
structural answer: per-engine variants on the record, threading
engine-specific knobs into typed slots.

## Decision

`StoredConnection` becomes a tagged enum with one variant per engine:

```rust
#[serde(tag = "engine")]
pub enum StoredConnection {
    PostgreSQL(PgStoredConnection),
    MySQL(MySqlStoredConnection),
    SQLite(SqliteStoredConnection),
    ClickHouse(ClickHouseStoredConnection),
    Redis(RedisStoredConnection),
}
```

Each variant carries only the fields its engine actually uses:

- **PG / MySQL**: shared host-auth fields + `ssl: bool` (new — TLS
  upgrade on the wire protocol). The hidden form default landing as a
  real field is the lever for ADR-0010's second half.
- **SQLite**: file-only fields. Sentinel host/port/user/password slots
  preserved at zero values for wire compatibility with the frontend's
  flat `Connection` type (Slice 2 narrows the frontend; until then
  every connection still carries these slots).
- **ClickHouse**: host-auth + `use_https` + `url_path`.
- **Redis**: host-auth + `db_number` + `use_tls` + `verify_tls_cert`.

Serialization is **internally tagged on `engine`**
(`#[serde(tag = "engine")]`). The wire JSON shape is byte-identical
to the previous flat record except that engine-irrelevant fields are
now absent for engines that don't have them. The frontend's optional
field types (`useHttps?`, `dbNumber?`, …) tolerate the absence, and
the new `ssl` field appears only on PG/MySQL responses.

The SQLite schema stays **flat** — one row per connection, all
columns on the same table. The row-to-variant construction in
`storage::read_connections` matches on the `engine` column and reads
only the columns that apply to the chosen variant. A new migration
(`ALTER TABLE connections ADD COLUMN ssl INTEGER NOT NULL DEFAULT 1`)
backfills `ssl = true` for existing rows, matching the previous
hidden form default; existing connections continue to negotiate TLS
the way they did before.

PG `connect` threads `ssl` into `PgConnectOptions::ssl_mode` —
`Prefer` (negotiate-if-supported, historical implicit default) when
on, `Disable` when off. MySQL goes through the sqlx-Any DSN with
`?ssl-mode=preferred|disabled`.

`StoredConnection` exposes accessor methods (`host()`, `port()`,
`user()`, `password()`, `database()`, `id()`, `name()`, `role()`,
`last_activity_at()`, `set_password()`, `engine()`) for the common
fields. Engine-specific access goes through pattern matching: the
dispatch layer (`dispatch/relational.rs`, `dispatch/keyvalue.rs`)
matches on the variant and passes a typed reference into the engine
module. ClickHouse's `as_ch` helper and the keyvalue dispatcher's
`as_redis` helper localize the runtime narrowing once per call so
deeper code paths get a typed `&ClickHouseStoredConnection` /
`&RedisStoredConnection` without re-asserting the contract.

## Alternatives considered

1. **Keep the flat struct**. Cheapest but preserves the silent-default
   bug class. Rejected: the whole point of this work is to make the
   type system enforce engine-applicability.
2. **Two-level enum: `StoredConnection` discriminates on storage
   class (Relational | KeyValue), then on engine inside**. Aligns with
   ADR-0008's storage-class fork at the dispatcher level. Rejected as
   over-structured: the per-engine fork at the record level is finer
   than the storage-class fork at the dispatcher level, and that's
   the right asymmetry — workspace shells and dispatch routers care
   about the class; records carry per-engine fields.
3. **Adjacently-tagged enum** (`{ "engine": "PostgreSQL", "data":
   {...} }`). Cleaner enum representation but breaks wire
   compatibility with the existing frontend. Rejected: we want
   Slices B and C to be independently shippable, which requires the
   wire shape to stay stable across the boundary.
4. **Wire SSL only for PG, defer MySQL**. Saves one driver path.
   Rejected: the form-policy union (Slice 4) groups PG + MySQL under
   the `host-auth` form kind precisely because they have the same
   form shape and the same TLS concept; threading SSL through both
   keeps the symmetry honest.

## Implications

- The Credential Backend pattern (`credentials.rs`) was previously
  the lone exemplar of "closed variant set + exhaustive dispatch + per-
  variant logic concentrated" in this codebase. `StoredConnection` is
  the second instance. The two together establish the pattern as
  load-bearing rather than situational; future engine-class
  additions should follow it.
- The dispatch layer's `match connection { ... }` replaces
  `match connection.engine { ... }` everywhere it matters; the
  variants bring typed references for free, and the compiler will
  refuse to compile a dispatch site that forgets an engine.
- `dispatch::relational::sqlx_dsn` is the new home for "what does the
  sqlx connection URL look like per engine" — including `?sslmode=...`
  / `?ssl-mode=...` for PG and MySQL. The native PG path
  (`postgres.rs::connect`) builds `PgConnectOptions` directly with
  the same toggle; the two stay in sync because they read the same
  field on the same variant.
- The frontend stays on the flat `Connection` type for now. Slice 2
  (ADR-0011) narrows it to a mirror per-engine union; the wire
  contract is unchanged across that transition.
- `CONTEXT.md`'s `Stored Connection` entry will need a rewrite to
  describe the tagged-enum shape; deferring that to Slice 4 keeps the
  doc in sync with what the frontend actually sees end-to-end.

## Related

- ADR-0007 — SQLite primary persistence (the connections table this
  ADR adds a column to).
- ADR-0008 — storage-class fork at the dispatcher layer (the layer
  *above* the per-engine record fork introduced here).
- ADR-0011 (forthcoming) — frontend `Connection` mirrors this shape.
- ADR-0012 (forthcoming) — unified `ConnectionForm` consumes the
  per-engine policy + records.
