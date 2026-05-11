# dbunk — Domain Context

dbunk is a desktop database client (Tauri + React) for exploring data, running
SQL, and managing connections across PostgreSQL, MySQL, SQLite, and ClickHouse.
This file is the canonical domain glossary. New work should reuse these terms
rather than coining synonyms.

## Top-level entities

- **Connection** — a saved database endpoint. Carries the engine, host, port,
  database name, user, role, and an optional credential. Persisted in the
  local SQLite database (`~/.config/dbunk/dbunk.sqlite`). Password storage is
  app-wide: encrypted SQLite is recommended, OS keychain is available, and
  unencrypted SQLite is available with warning (ADR-0007).
  Connections have a runtime **status** (`Connected` / `Read only` /
  `Disconnected`), a **latency** measurement from the last ping, a **last
  activity** timestamp from the most recent successful query/connect, and an
  optional **error message** from the last health check.
- **Stored Connection** — the backend wire shape (no returned password, no
  runtime status). The Rust backend hydrates credentials internally before DB
  operations; stored passwords are not returned to the frontend.
- **Active Connection** — the one currently selected in the sidebar; drives
  the schema explorer, overview, and any newly opened tab.
- **Engine** — `PostgreSQL`, `MySQL`, `SQLite`, or `ClickHouse`. PostgreSQL is
  the reference implementation (see ADR-0001); other engines have selective
  coverage and may return "not supported on \<engine\>" placeholders.

## Schema model

- **Schema** — a namespace inside a database (PostgreSQL, MySQL); SQLite
  collapses to a single schema, ClickHouse calls them databases. The UI
  treats them uniformly as schemas.
- **Schema Explorer** — the schemas + tables + views tree shown in the
  sidebar. Loaded per-connection on connect.
- **Schema Relationships** — the foreign-key graph used to render the schema
  map. Loaded lazily per-schema.
- **Table Structure** — columns, primary key, foreign keys, indexes, and
  constraints for one table. Includes a **capabilities** flag that tells the
  frontend which fields are populated for the active engine.
- **Row Identity** — the columns the frontend uses to address a single row
  for edit/delete. Picked from primary key first, falling back to non-null
  unique indexes. Tables without a usable identity render **read-only**.

## Workspace model

- **Workspace Tab** — an open tab in the main area. Two kinds: `table` (the
  data browser) and `query` (the SQL editor). The active tab is identified by
  its **active tab ID**.
- **Query History Entry** — a record of one executed query (sql, connection,
  status, runtime, optional error). Capped at 200 entries, persisted in
  SQLite.
- **Saved Query** — a named, persisted SQL snippet with optional connection
  pinning and a favorite flag. Stored in SQLite. The schema reserves an
  `ownerId` field so a future cloud-sync layer can plug in
  without migration (ADR-0003).
- **Cell Edit** — a pending change to one cell, keyed by row index inside
  the loaded page. Buffered in memory and committed in a transaction.
- **DDL Statement** — a schema-level change (CREATE/ALTER/DROP), executed via
  the `execute_ddl` command rather than the regular query path so the
  frontend can model the response shape distinctly.
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

## Live state

- **Health Check** — a `SELECT 1` ping that verifies a connection is alive.
  Fired every 30 seconds by a foreground tick (ADR-0002), and on demand from
  the **Test Connection** button before save.
- **Last Activity** — ISO-8601 timestamp on the connection record. Bumped
  whenever a query or connect succeeds (ADR-0004).
- **Database Overview Stats** — aggregate counts (tables, schemas, rows,
  indexes, connections) plus byte sizes (database / table / index). Fetched
  in a single round trip per connection. Row counts use planner estimates
  from `pg_class.reltuples`, not exact counts.

## Persistence layout

```
~/.config/dbunk/
└── dbunk.sqlite

dbunk.sqlite:
├── app_settings
├── connections
├── credentials
├── credential_verifier
├── query_history
└── saved_queries

Optional OS keychain backend (service: "dbunk", account: "connection-credentials"):
└── JSON blob: { connectionId: password }
```

ADR-0007 covers SQLite persistence and credential storage modes. ADR-0005's
single-blob keychain shape now applies only when keychain mode is active.

## Process model

- **Tauri command** — every backend operation is a `#[tauri::command]` async
  function in `src-tauri/src/lib.rs`. The frontend invokes them via
  `tauriInvoke<T>("name", payload)` from `src/lib/tauri.ts`. Payloads use
  camelCase via serde rename rules. The Tauri command itself owns payload
  validation and activity tracking; it delegates engine-aware work to
  **Engine Dispatch**.
- **Engine Dispatch** — the `dispatch` module
  (`src-tauri/src/dispatch.rs`) routes engine-aware operations
  (`run_query`, `execute_ddl`, mutations, introspection) to the right
  per-engine implementation (`postgres::`, `clickhouse::`, future
  `mysql::`, `redis::`). Every match is exhaustive over `DatabaseEngine`
  — no wildcards — so adding an engine forces every operation to make
  an explicit choice. Two error shapes: `not_implemented_yet` (will
  catch up — see ADR-0001) and `not_applicable` (the operation doesn't
  exist on this engine class, reserved for Redis etc.).
- **Engine UI Policy** — the frontend mirror to Engine Dispatch
  (`src/lib/engine-policy.ts`). A pure `Record<DatabaseEngine,
  EnginePolicy>` table owning UI-side engine-aware data: connection
  form shape (host/auth requirements, default port, CH HTTPS toggle),
  structure-view labels ("Primary key" vs "Sorting key", "Indexes" vs
  "Skip indices"), stats-card row-count semantics (`exact` vs
  `estimate`), foreign-key copy when not supported, schema-map empty
  banner. `TypeScript`'s `Record` enforces exhaustiveness — a new
  engine variant won't compile until every policy field is filled in.
  Scope is **engine-level only**: per-table mutation decisions stay on
  `TableStructure.capabilities` plus `pickRowIdentity`.
- **AppHandle** — Tauri's global handle, threaded through every command that
  needs to reach the config directory or keychain. Functions that don't need
  the keychain (e.g. `touch_connection_activity`) take a path-only path so
  they can stay off the prompt-prone keychain hot path.
- **App Shell** — the React root component (`src/components/app-shell.tsx`).
  Owns the top bar, sidebar, and the foreground health-check loop.
