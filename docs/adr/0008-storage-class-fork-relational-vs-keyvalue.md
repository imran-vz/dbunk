# ADR-0008 — Engines fork at a `StorageClass` layer above `DatabaseEngine`

**Status**: Accepted (2026-05-11)

## Context

ADR-0001 anticipated this question: adding a non-SQL engine (Redis was
named explicitly) would not slot under the existing `Connection` /
`Schema` / `Table` / `Cell Edit` model the way ClickHouse did. That ADR
deferred the decision until a real Redis integration was in scope; it now
is.

The relational engines (PG, MySQL, SQLite, CH) share a load-bearing spine:
schemas contain tables, tables contain rows, rows have columns, the user
edits cells, the workspace renders tabs of kind `table` or `query`, the
sidebar renders Schemas → Tables/Views. ClickHouse stretched this spine
(no row uniqueness, async mutations, sorting keys instead of PKs) but did
not break it — every CH concept maps to a relational analogue with a
relabel.

Redis breaks the spine in four places at once:

1. **No tables, no schemas in the relational sense.** A Redis "database"
   is a numbered keyspace (DB 0–15 on standalone); there are no tables;
   keys are flat. The hierarchical entity model (`Schema` → `Table` →
   `Row`) has no Redis equivalent.
2. **Type-per-key, not type-per-column.** Each key has one of seven
   types (string, hash, list, set, sorted set, stream, JSON); the type
   determines which read/write commands apply. There is no "columns of
   a table" abstraction to gate on.
3. **The relational tab kinds (`table` data grid, `query` SQL editor)
   are wrong shape.** A Redis user wants a per-type viewer, a REPL, a
   pub/sub firehose, a server-health panel — not a data grid and a SQL
   editor.
4. **Most relational backend operations are not just "not yet implemented"
   on Redis — they are conceptually inapplicable.** `fetch_table_structure`,
   `fetch_schema_relationships`, `execute_ddl`, `commit_cell_edits`,
   `insert_row`, `delete_rows`, `poll_mutation_status` — none of these
   map to anything in Redis. Forcing them through the existing dispatch
   as `not_implemented_yet` would lie about the surface; routing each as
   `not_applicable` per-engine in 20 separate arms would be honest but
   noisy.

The naive answer — extend `DatabaseEngine` with a `Redis` variant, fill in
every existing dispatch arm with `Redis => not_applicable(...)`, fill
`EnginePolicy` fields with nonsensical placeholders — preserves the type
system's exhaustiveness check at the cost of structural honesty. Reviewers
four months later would see `policy.labels.primaryKey: ""` and not know
whether the engine doesn't have PKs or the engine record forgot to fill
the field.

## Decision

**Engines fork at a `StorageClass` layer above `DatabaseEngine`**.
`StorageClass` is `Relational | KeyValue` today, with room for
`DocumentStore`, `WideColumn`, `Graph` as future arms when real engines
demand them.

The fork is **structurally honest**: relational-only concepts (tables,
foreign keys, indexes, cell edits, DDL) live behind the `Relational`
arm of every place they were defined; keyvalue-only concepts (keys, key
metadata, pub/sub sessions, command history) live behind the `KeyValue`
arm. Cross-cutting concepts (connections, credentials, the workspace tab
chrome, the connection-form's host/port/auth shape) stay shared.

Specifically:

- **`StorageClass` is derived, not stored.** Single source of truth:
  `DatabaseEngine::storage_class()` (Rust) and
  `enginePolicy(engine).storageClass` (TypeScript). Both classify every
  variant; a snapshot test runs both and asserts they agree, so drift
  breaks CI.
- **Backend dispatch splits by class.** `src-tauri/src/dispatch.rs`
  becomes `src-tauri/src/dispatch/mod.rs` plus
  `dispatch/relational.rs` (PG/MySQL/SQLite/CH — all existing arms move
  verbatim) and `dispatch/keyvalue.rs` (Redis). The public functions in
  `dispatch/mod.rs` open with one match on `engine.storage_class()` and
  route. Relational-only operations return `not_applicable` from the
  `KeyValue` arm at the routing layer (one place per op, not 20).
  Redis-specific operations live entirely under `dispatch/keyvalue.rs`
  and return `not_applicable` from the `Relational` arm.
- **The `DatabaseEngine` enum stays flat.** No `Relational(...)` /
  `KeyValue(...)` wrapping. The class lookup is one function, not a
  type-level partition. This decision is deliberate — the alternative
  (nested enum) would have a much larger blast radius across every
  serde-derived DTO and every existing `match engine` site for net-zero
  type-safety gain.
- **`DatabaseOverview` becomes a tagged union**, not a stretched record.
  `RelationalOverviewStats` keeps its current shape; `KeyValueOverviewStats`
  is its sibling with Redis-shaped fields (identity, keyspace, memory,
  clients, replication, modules, slow log, persistence). The Tauri
  command's return type narrows on `kind: "relational" | "keyvalue"`.
- **`EnginePolicy` becomes a discriminated union with a shared base.**
  `{ engine, connectionForm }` stays flat; per-class fields
  (`RelationalEnginePolicy` vs `KeyValueEnginePolicy`) live under a
  `storageClass` discriminant. Two narrowing helpers
  (`relationalPolicy()` / `keyvaluePolicy()`) are the entry points for
  storage-class-specific component trees; deep call sites do not need
  to re-narrow.
- **The workspace shell forks at the top level.** When the active
  connection's storage class is `KeyValue`, the workspace renders
  `<KeyValueWorkspace />` (keyspace browser sidebar; tab kinds `key`,
  `cli`, `pubsub`, `server`) instead of `<RelationalWorkspace />`
  (schema explorer sidebar; tab kinds `table`, `query`). The app shell
  (top bar, connections list, settings, credential onboarding) stays
  shared.
- **`TableStructure` stays relational-only.** Redis's per-key inspection
  uses a separate `KeyMetadata` DTO and a separate Tauri command
  (`fetch_key_metadata`), not a stretched `TableStructure`. The dispatch
  split makes this routing explicit.
- **Connection records do not store `storageClass`.** It is always
  derived from `engine`. Storing it would invite drift; deriving it
  forces the engine variant to remain the single discriminator.
- **The `connections`, `credentials`, `credential_verifier`,
  `query_history`, `saved_queries`, `app_settings` SQLite tables are
  shared across classes.** Redis-specific persistence
  (`redis_command_history`) is its own table. Class-specific connection
  columns (`db_number`, `use_tls`, `verify_tls_cert`) are nullable on
  the shared `connections` table — same pattern ClickHouse already uses
  for `use_https` and `url_path`.

## Consequences

- **Adding a third storage class** (e.g. `DocumentStore` for MongoDB) is
  a localized change: a new `StorageClass` arm, a new dispatcher module,
  a new policy union arm, a new workspace component. The relational and
  keyvalue paths don't move.
- **Adding a fourth relational engine** (e.g. CockroachDB) is unchanged
  from today: add a `DatabaseEngine` variant, fill in `dispatch/relational.rs`,
  add an `EnginePolicy` entry. The class layer is invisible to that work.
- **Adding a Redis-class concept to a relational engine** (or vice versa)
  is genuinely awkward, and that is the correct outcome. The classes
  exist precisely because those concepts don't transfer cleanly. If a
  relational engine grows pub/sub-like fan-out (Postgres `LISTEN/NOTIFY`,
  for example), the right move is to model it inside the relational
  shell — not to drag the keyvalue shell over it.
- **`not_applicable` is a routing-layer error**, not a per-arm error.
  Pre-fork, every relational dispatch function had a single arm; post-
  fork, the cross-class case is "this Tauri command's operation does
  not exist on the active engine's class" — exactly one branch per
  command, in `dispatch/mod.rs`.
- **Two classifiers stay synchronized via test.** The Rust
  `storage_class()` and the TS `enginePolicy().storageClass` classify
  every variant. The snapshot test reads both, compares them, fails on
  drift. Engineers adding a `DatabaseEngine` variant who only update
  one side will see CI fail before merge.
- **The frontend's storage-class-aware code is concentrated.** Components
  under `src/components/keyvalue/` are Redis-shaped; components under
  `src/components/` that touch tables (e.g. `table-editor-panel.tsx`,
  `table-structure-view.tsx`, `data-grid.tsx`) are relational-shaped.
  They share connection-form, settings, top bar, status bar, credential
  onboarding — and nothing else. A reviewer who opens
  `src/components/keyvalue/` knows immediately what world they are in.
- **Per-table capability flags (`StructureCapabilities`) stay
  relational-only.** Redis has no analogue at the per-table granularity;
  per-key write/delete gating is computed at the editor render site from
  the connection's auto-read-only flag plus the value type. If a future
  Redis ACL design demands per-key capabilities, that introduces a
  `KeyCapabilities` sibling type — it does not retrofit
  `StructureCapabilities`.
- **The CONTEXT.md domain glossary gains `StorageClass`** as a top-level
  concept, plus entries for `Key`, `KeyMetadata`, `KeyValueOverviewStats`,
  `CommandHistoryEntry`. Existing entries (`Schema`, `Table Structure`,
  `Row Identity`, `Cell Edit`, `DDL Statement`, `Pending Mutation`) are
  annotated as relational-only.
- **A future proposal to unify the workspace shell** ("let's render
  Redis under the schema-explorer with key-types-as-tables and the SQL
  editor as a CLI") should be rejected on the grounds laid out here. The
  shells are different because the concepts behind them are different;
  unification has been considered and explicitly declined.
- **ADR-0001's promise is honored.** That ADR named this decision as
  "a deliberate v2 question — we'll define the KV/non-SQL contract when
  a real Redis (or similar) integration is in scope, not before." This
  is that contract.
