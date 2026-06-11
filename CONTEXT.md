# dbunk — Domain Context

dbunk is a desktop database client (Tauri + React) for exploring data, running
SQL, and managing connections across PostgreSQL, MySQL, SQLite, ClickHouse,
and Redis. This file is the canonical domain glossary. New work should reuse
these terms rather than coining synonyms.

## Top-level entities

- **Connection** — a saved database endpoint. A **per-engine tagged
  union** (ADR-0011) — `PgConnection | MySqlConnection |
  SqliteConnection | ClickHouseConnection | RedisConnection` — each
  variant discriminated by `engine`. Common fields (`id`, `name`,
  `host`, `port`, `user`, `password`, `database`, `role`,
  `lastActivityAt`) plus runtime fields (`status` =
  `Connected` / `Read only` / `Disconnected`, `latency`, `lastSync`,
  optional `errorMessage`) live on every variant. Persisted in the
  local SQLite database (`~/.config/dbunk/dbunk.sqlite`). Database
  credential storage is app-wide: encrypted SQLite is recommended, OS
  keychain is available, unencrypted SQLite is available with warning
  (ADR-0007).
  Engine-class-specific fields live on their variants and **only
  their variants** — TypeScript narrows on `connection.engine` to
  reach them. `ssl: boolean` lives on `PgConnection` /
  `MySqlConnection` (TLS upgrade on the wire protocol); `useHttps`
  and `urlPath` live on `ClickHouseConnection` (TLS for the HTTP
  transport — a distinct concept from `ssl`); `dbNumber`, `useTls`,
  `verifyTlsCert` live on `RedisConnection` (TLS via `rediss://`).
  SQLite has no transport-encryption field.
- **Stored Connection** — the wire shape, identical to `Connection`
  minus the runtime fields. A **per-engine serde-tagged enum** on
  the Rust side (`#[serde(tag = "engine")]` over
  `PgStoredConnection` / `MySqlStoredConnection` /
  `SqliteStoredConnection` / `ClickHouseStoredConnection` /
  `RedisStoredConnection`; ADR-0010); the wire JSON is flat
  (variant fields next to the `engine` tag). The Rust backend
  hydrates credentials internally before DB operations; stored
  passwords are not returned to the frontend.
- **Bastion Server** — a reusable saved SSH endpoint used to reach
  database endpoints that are not directly reachable from the user's
  machine. It owns one active SSH credential, separate from any
  database credential on a Connection.
- **SSH Tunnel** — a Connection's routing choice to forward its
  database traffic through a Bastion Server. It routes the Connection's
  database endpoint rather than replacing it, and may carry
  connection-specific forwarding options; applies to network-backed
  Connections, while SQLite has no tunnel because it has no network
  transport.
- **Active Connection** — the one currently selected in the sidebar; drives
  the schema explorer (relational engines) or keyspace browser (Redis), the
  overview / server tab, and any newly opened tab.
- **Engine** — `PostgreSQL`, `MySQL`, `SQLite`, `ClickHouse`, or `Redis`.
  PostgreSQL is the reference implementation for the relational class (see
  ADR-0001); other relational engines have selective coverage and may return
  "not supported on \<engine\>" placeholders. Redis is the first keyvalue
  engine (see ADR-0008).
- **Storage Class** — `Relational` or `KeyValue`. Engines fork at this layer
  above `DatabaseEngine` (ADR-0008): relational engines share schemas/tables/
  rows/SQL; keyvalue engines share a keyspace of typed keys, no schemas, no
  rows. The class is **derived** from the engine via
  `DatabaseEngine::storage_class()` (Rust) and `enginePolicy(engine).storageClass`
  (TS); it is never stored. A snapshot test asserts both classifiers agree.
  Adding a future class (`DocumentStore`, `WideColumn`, …) means adding an
  arm to the enum and a matching dispatcher / workspace / policy union.

## Schema model (relational class only)

The following entities apply only to engines whose `storage_class()` is
`Relational`. Keyvalue engines (Redis) have no schemas, no tables, no row
identity; see the **Keyspace model** section below.

- **Schema** — a namespace inside a database (PostgreSQL, MySQL); SQLite
  collapses to a single schema, ClickHouse calls them databases. The UI
  treats them uniformly as schemas.
- **Schema Explorer** — the schemas + tables + views tree shown in the
  sidebar. Loaded per-connection on connect.
- **Schema Relationships** — the foreign-key graph used to render the schema
  map. Loaded lazily per-schema.
- **Schema Map** — the visual graph of relational tables and their
  foreign-key relationships.
- **Table-Level Schema Map** — a Schema Map scoped to one table workspace
  subtab, showing the current Table Card, directly referenced Table Cards,
  directly referencing Table Cards, and the Relationship Edges connecting
  those direct neighbors.
- **Table Card** — one table's visual node in the Schema Map.
- **Junction Table Card** — a Table Card identified as representing a
  many-to-many association through its real foreign-key relationships; the
  Schema Map still shows the real Relationship Edges rather than replacing
  them with a synthetic direct edge.
- **Column Row** — one column entry inside a Table Card.
- **Trigger Indicator** — a Schema Map signal on a Table Card or Column Row
  that database trigger metadata exists for that table or explicitly targets
  that column.
- **Relationship Edge** — one visual line for one foreign-key constraint
  between Table Cards.
- **Relationship Cardinality** — the backend-provided classification for a
  Relationship Edge, derived from database constraints where possible and
  accompanied by a reason when the classification needs explanation.
- **Relationship Detail Popover** — the popup shown when a Relationship Edge
  is selected.
- **Focused Table** — the selected Table Card whose directly referencing and
  directly referenced Table Cards, plus connected Relationship Edges, remain
  emphasized while unrelated graph elements dim.
- **Focused Relationship Edge** — the selected Relationship Edge whose two
  endpoint Table Cards remain emphasized while unrelated graph elements dim.
- **Table Structure** — columns, primary key, foreign keys, indexes, and
  constraints for one table. Includes a **capabilities** flag that tells the
  frontend which fields are populated for the active engine. Relational-only;
  Redis uses **Key Metadata** instead.
- **Row Identity** — the columns the frontend uses to address a single row
  for edit/delete. Picked from primary key first, falling back to non-null
  unique indexes. Tables without a usable identity render **read-only**.

## Keyspace model (keyvalue class only)

The following entities apply only to engines whose `storage_class()` is
`KeyValue` (Redis today). See ADR-0008 for the storage-class fork and
ADR-0009 for the writes-by-default posture.

- **Database (Redis)** — a numbered keyspace, 0–15 on standalone. Selected
  per connection record (`dbNumber` field); the per-session DB switcher is
  a Tier-2 deferral. Conceptually parallel to a relational **Schema**, but
  flat (no nested tables).
- **Key** — a single record in the keyspace. Has a name (bytes; usually
  UTF-8), a **type** (`string` | `hash` | `list` | `set` | `sorted set` |
  `stream` | `JSON`), a **TTL** (live-decrementing seconds remaining; `-1`
  for never expires), an **encoding** (Redis's internal representation —
  `embstr`/`raw`/`listpack`/`ziplist`/`quicklist`/`intset`/`hashtable`/
  `skiplist`/`ReJSON-RL`/…), and an opaque **value** whose shape depends on
  the type. Bitmap, HyperLogLog, and Geo keys render as their underlying
  type (`string`, `string`, `sorted set`) with a secondary type-aware panel.
- **Key Metadata** — the analogue of **Table Structure** for the keyvalue
  class. Carries type, TTL, encoding, `OBJECT REFCOUNT` / `IDLETIME` /
  `FREQ`, size summary (element count for collections, byte length for
  strings), and a watch/refresh toggle. Returned by the `fetch_key_metadata`
  Tauri command; rendered in the **Key Inspector**'s right-hand metadata
  drawer.
- **Keyspace Browser** — the sidebar for keyvalue connections (parallel to
  the relational **Schema Explorer**). Hybrid shape: a lazy prefix tree
  (splits keys on a configurable separator, default `:`), a search input
  that swaps the panel into flat-result mode, and a type-filter chip row.
  Driven by `SCAN MATCH` with cursor pagination; the full keyspace is never
  materialised in memory.
- **Pub/Sub Session** — a dedicated `redis-rs` connection backing one open
  `pubsub` tab. Holds active `PSUBSCRIBE` patterns and a 10k-message ring
  buffer (configurable to 50k). Lives for the lifetime of the tab; survives
  app restart in paused state. Distinct from the multiplexed connection
  used for every other operation.
- **Command History Entry** — a record of one CLI-tab command (command
  text, status, runtime, executed_at, db number). Cap 1000 entries per
  connection. Persisted in the `redis_command_history` SQLite table —
  separate from `query_history` because the shapes differ (no SQL/DDL
  split, no row count).

## Workspace model

- **Workspace Shell** — the contents of the main area when a connection is
  active. Forks on the active connection's storage class (ADR-0008): the
  **Relational Workspace** renders the schema explorer + `table`/`query`
  tab kinds; the **KeyValue Workspace** renders the keyspace browser +
  `key`/`cli`/`pubsub`/`server` tab kinds. The app shell (top bar,
  connections list, settings, credential onboarding) is shared.
- **Workspace Tab** — an open tab in the main area. Relational kinds:
  `table` (the data browser) and `query` (the SQL editor). Keyvalue kinds:
  `key` (multi-instance, one per inspected key), `cli` (singleton REPL),
  `pubsub` (singleton subscription monitor), `server` (singleton INFO /
  health view; also the default-opened tab when a Redis connection becomes
  active). The active tab is identified by its **active tab ID**.
- **Table Session** — the per-table client-side session for a relational
  `table` Workspace Tab. Identified by `(connectionId, schema, table)`, not
  by table name alone. Owns the loaded **Table Data**, loaded **Table
  Structure**, pending **Cell Edit** buffer, write lifecycle status, and the
  caller-facing **Edit Outcome** actions for cell-edit commit, row insert, and
  row delete. Export, copy-table, schema-map, DDL, and maintenance workflows
  are table-adjacent but remain outside the Table Session.
- **Query History Entry** — a record of one executed query (sql, connection,
  status, runtime, optional error). Capped at 200 entries, persisted in
  SQLite.
- **Saved Query** — a named, persisted SQL snippet with optional connection
  pinning and a favorite flag. Stored in SQLite. The schema reserves an
  `ownerId` field so a future cloud-sync layer can plug in
  without migration (ADR-0003).
- **Cell Edit** — a pending change to one cell, keyed by row index inside
  the loaded page. Buffered in memory and committed in a transaction.
  Relational-only.
- **DDL Statement** — a schema-level change (CREATE/ALTER/DROP), executed via
  the `execute_ddl` command rather than the regular query path so the
  frontend can model the response shape distinctly. Relational-only — Redis
  has no schema-shaped surface; its equivalents (key creation, rename,
  expire) flow through dedicated Tauri commands, not `execute_ddl`.
- **Pending Mutation** — a single ClickHouse `ALTER TABLE … UPDATE` or
  `ALTER TABLE … DELETE` statement that has been accepted by the server
  and is applying asynchronously across MergeTree parts. Identified by
  `mutation_id` from `system.mutations`, scoped to a specific
  `(connection, database, table)`. The `pending-mutations` module
  (`src/lib/pending-mutations.ts`) drives a batch to a `MutationOutcome`
  (`completed` / `failed` / `timeout`); consumers — `commitTableEdits`,
  `deleteSelectedTableRows`, future async DDL — translate that outcome
  into their own status surface and decide what to refresh.
  Distinct from a **Cell Edit**: a Cell Edit is the user's intent
  (buffered in memory); a Pending Mutation is the server-side work
  produced when that intent commits on an async engine.
- **Edit Outcome** — the caller-facing terminal result of one
  store-action-driven write (cell-edit commit, row insert, row delete).
  A tagged union on `kind`: `"completed" | "failed" | "timeout" |
  "noop"`, returned by the action that initiated the write so the
  caller can react to its own operation's result without subscribing
  to shared status state. Synchronous engines (PG/MySQL/SQLite)
  resolve to `completed` or `failed`; `timeout` is reachable only on
  async engines — ClickHouse, where the action drives a **Pending
  Mutation** batch to terminal state before resolving. The `noop`
  variant is returned when the action found nothing to commit (no
  edits, no rows after filtering); the UI does not render a banner
  for it. Distinct from the lifecycle status the store keeps in
  `tableEditsCommitStatus` (`running` / `queued` intermediate states
  for the UI badge); the Edit Outcome is the truth a caller awaits,
  the lifecycle status is a best-effort view for badge rendering and
  concurrent-op gating.
- **DDL Outcome** — the caller-facing terminal result of one
  `commitStructureChanges` invocation — i.e. the outcome of executing
  one batch of **DDL Statement**s. A tagged union on `kind`:
  `"completed" | "failed" | "noop"`, returned by the store action so
  the caller can await its own operation's result. Always synchronous
  today: `execute_ddl` returns a final status from the engine
  immediately and DDL never enters a **Pending Mutation** flow on any
  current engine — hence no `timeout` variant (unlike Edit Outcome).
  The `noop` variant is returned when there were no pending changes;
  the UI does not render a banner for it. Distinct from the lifecycle
  status the store keeps in `structureCommitStatus` (`running` only),
  which exists solely to keep the Commit button disabled across tab
  unmounts.
- **Query Outcome** — the caller-facing terminal result of one
  `runQuery` invocation — i.e. the outcome of executing one SQL
  statement on a `query`-kind **Workspace Tab**. A tagged union on
  `kind`: `"completed" | "failed" | "noop"`, returned by the store
  action so the panel can await its own operation's result. `completed`
  carries `runtimeMs` and `rowCount`; the row data itself is written
  to the store's `queryPreviews` slot and persists across tab unmount
  (the **Query History Entry** also writes there, regardless of the
  outcome that flows back to the caller). `noop` covers the five
  short-circuit paths: no tab, wrong tab kind, already running,
  empty query after trim, no Tauri backend. The UI does not render
  a banner for `noop`. Distinct from the lifecycle status the store
  keeps in `queryStatus` (`running` only), which exists to keep the
  Run button disabled and the panel labelled "Running…" across tab
  unmounts.

## Live state

- **Health Check** — a `SELECT 1` ping that verifies a connection is alive.
  Fired every 30 seconds by a foreground tick (ADR-0002), and on demand from
  the **Test Connection** button before save.
- **Last Activity** — ISO-8601 timestamp on the connection record. Bumped
  whenever a query or connect succeeds (ADR-0004).
- **Database Overview** — the storage-class-shaped landing payload for an
  active connection. Per ADR-0008 it is a **tagged union**:
  - **Relational Overview Stats** (`kind: "relational"`) — aggregate counts
    (tables, schemas, rows, indexes, connections) plus byte sizes (database
    / table / index). Fetched in a single round trip per connection. Row
    counts use planner estimates from `pg_class.reltuples` (PG) or exact
    counts from `system.parts.rows` (CH), not exact counts on PG.
  - **KeyValue Overview Stats** (`kind: "keyvalue"`) — Redis-shaped: server
    identity (version, mode, uptime), per-DB keyspace counts, memory
    (used/peak/rss/fragmentation/maxmemory/policy), clients
    (connected/max/blocked), replication (role + replica count or master
    link), modules (name + version), optional slow log, optional persistence
    summary. Per-section degradation when source commands are restricted
    (e.g. managed Redis blocking `INFO replication`).

## Persistence layout

```
~/.config/dbunk/
├── dbunk.sqlite
└── pubsub-captures/         (created on first capture-to-file from a pubsub tab)
    └── <connectionId>-<ISO>.jsonl

dbunk.sqlite:
├── app_settings
├── connections                   (relational + keyvalue; Redis-only columns
│                                  dbNumber/useTls/verifyTlsCert nullable)
├── credentials
├── credential_verifier
├── query_history                 (relational only — SQL queries)
├── redis_command_history         (keyvalue only — CLI commands, cap 1000/connection)
└── saved_queries

Optional OS keychain backend (service: "dbunk", account: "connection-credentials"):
└── JSON blob: { connectionId: password }
```

ADR-0007 covers SQLite persistence and credential storage modes. ADR-0005's
single-blob keychain shape now applies only when keychain mode is active.
The `pubsub-captures` directory is created lazily and stores raw JSONL
streams from any pub/sub tab that has the **Record to file** toggle on.
Captures are never auto-pruned; users manage them manually.

## Process model

- **Tauri command** — every backend operation is a `#[tauri::command]` async
  function in `src-tauri/src/lib.rs`. The frontend invokes them via
  `tauriInvoke<T>("name", payload)` from `src/lib/tauri.ts`. Payloads use
  camelCase via serde rename rules. The Tauri command itself owns payload
  validation and activity tracking; it delegates engine-aware work to
  **Engine Dispatch**.
- **Engine Dispatch** — the `dispatch` module
  (`src-tauri/src/dispatch/mod.rs`) routes engine-aware operations to
  the right per-engine implementation. Public functions open with one
  match on `engine.storage_class()`, then delegate to either
  `dispatch::relational::*` (PG/MySQL/SQLite/CH; module
  `dispatch/relational.rs`) or `dispatch::keyvalue::*` (Redis; module
  `dispatch/keyvalue.rs`) — see ADR-0008. Within each class, dispatch
  matches exhaustively over the `DatabaseEngine` variants of that class —
  no wildcards — so adding an engine forces every operation to make an
  explicit choice. Cross-class operations return one of two error shapes
  at the routing layer: `not_implemented_yet` (in scope for this engine
  but not built yet — see ADR-0001) or `not_applicable` (the operation
  does not exist on this engine's class; e.g. `fetch_table_structure`
  returns this for Redis). Relational-only operations live entirely
  under `dispatch::relational::*`; keyvalue-only operations (`scan_keys`,
  `fetch_key_metadata`, `run_redis_command`, `redis_pubsub_*`,
  `fetch_keyvalue_overview`) live entirely under `dispatch::keyvalue::*`.
- **Engine UI Policy** — the frontend mirror to Engine Dispatch
  (`src/lib/engine-policy.ts`). A pure `Record<DatabaseEngine,
  EnginePolicy>` where `EnginePolicy` is a discriminated union on
  `storageClass` (ADR-0008): shared base `{ engine, connectionForm }`
  plus either a `RelationalEnginePolicy` arm (structure-view labels —
  "Primary key" vs "Sorting key", "Indexes" vs "Skip indices" — plus
  `rowCountKind` exact/estimate, `hasForeignKeys`, `foreignKeysUnsupportedCopy`,
  `schemaMapNoForeignKeysCopy`) or a `KeyValueEnginePolicy` arm
  (`defaultDbNumber`, `maxDbNumber`, `keyTypeIcons`, `pubSubSupported`,
  `transactionsSupported`, `destructiveCommands` — the last sourced via
  codegen from `redis-destructive-commands.toml`, shared with Rust).
  **Connection-form policy** (`ConnectionFormPolicy`) is itself a
  tagged union on `kind` (ADR-0012): `host-auth` (PG/MySQL — carries
  `defaultPort`, `showSslToggle`), `clickhouse-http` (CH —
  `defaultPortHttp`, `defaultPortHttps`), `redis` (`defaultPort`,
  `defaultDbNumber`, `maxDbNumber`), `file` (SQLite). The `kind`
  discriminator names the **form shape**, not the engine; multiple
  engines (PG + MySQL) can share a `kind`. The unified
  `ConnectionForm` component switches on `policy.kind` to decide
  which engine-specific fields to render, and validates via
  `validateConnection(policy, value, mode)` — one shared function,
  mode-aware password rule.
  Storage-class-specific components narrow via the
  `relationalPolicy(engine)` / `keyvaluePolicy(engine)` helpers; deep
  call sites do not re-narrow. `TypeScript`'s `Record` enforces
  exhaustiveness — a new engine variant won't compile until its policy
  is filled in. Scope is **engine-level only**: per-table mutation
  decisions stay on `TableStructure.capabilities` plus `pickRowIdentity`
  (relational); per-key write gating is computed at the editor render
  site from the connection's auto-read-only state plus the value type
  (keyvalue; see ADR-0009).
- **Credential Backend** — the `CredentialBackend` enum in
  `src-tauri/src/credentials.rs` is the single point of dispatch over
  the three storage modes (`Keychain`, `PlainSqlite`,
  `EncryptedSqlite`). Each variant owns its per-mode I/O in its own
  struct (`KeychainBackend`, `PlainSqliteBackend`,
  `EncryptedSqliteBackend`); the enum's methods are a 3-arm match.
  Public functions (`read_all`, `write_all`, `upsert`, etc.)
  construct a backend via `backend_for(mode, pool)` and delegate.
  Two physical storage areas back the three modes — the OS keychain
  is independent, while Plain and Encrypted SQLite share the
  `credentials` table — and `clear_inactive_storage` is the single
  helper that encodes this topology when the active mode changes.
  Mirrors the **Engine Dispatch** pattern: closed variant set,
  exhaustive dispatch, per-variant logic concentrated, cross-variant
  invariants in one helper.
- **AppHandle** — Tauri's global handle, threaded through every command that
  needs to reach the config directory or keychain. Functions that don't need
  the keychain (e.g. `touch_connection_activity`) take a path-only path so
  they can stay off the prompt-prone keychain hot path.
- **App Shell** — the React root component (`src/components/app-shell.tsx`).
  Owns the top bar, sidebar, and the foreground health-check loop.
