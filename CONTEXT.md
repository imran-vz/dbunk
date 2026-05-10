# dbunk — Domain Context

dbunk is a desktop database client (Tauri + React) for exploring data, running
SQL, and managing connections across PostgreSQL, MySQL, SQLite, and ClickHouse.
This file is the canonical domain glossary. New work should reuse these terms
rather than coining synonyms.

## Top-level entities

- **Connection** — a saved database endpoint. Carries the engine, host, port,
  database name, user, role, and an optional credential. Persisted in
  `connections.json` (no password) plus the OS keychain (passwords).
  Connections have a runtime **status** (`Connected` / `Read only` /
  `Disconnected`), a **latency** measurement from the last ping, a **last
  activity** timestamp from the most recent successful query/connect, and an
  optional **error message** from the last health check.
- **Stored Connection** — the on-disk JSON shape (no password, no runtime
  status). The Rust backend hydrates and dehydrates between this shape and
  the runtime `Connection` shape on every read/write.
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
  `query_history.json`.
- **Saved Query** — a named, persisted SQL snippet with optional connection
  pinning and a favorite flag. Stored in `saved_queries.json`. The schema
  reserves an `ownerId` field so a future cloud-sync layer can plug in
  without migration (ADR-0003).
- **Cell Edit** — a pending change to one cell, keyed by row index inside
  the loaded page. Buffered in memory and committed in a transaction.
- **DDL Statement** — a schema-level change (CREATE/ALTER/DROP), executed via
  the `execute_ddl` command rather than the regular query path so the
  frontend can model the response shape distinctly.

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
├── connections.json        # connection metadata, no passwords
├── query_history.json      # last 200 entries
└── saved_queries.json

OS keychain (service: "dbunk", account: "connection-credentials"):
└── JSON blob: { connectionId: password }
```

ADR-0005 covers the single-blob keychain shape.

## Process model

- **Tauri command** — every backend operation is a `#[tauri::command]` async
  function in `src-tauri/src/lib.rs`. The frontend invokes them via
  `tauriInvoke<T>("name", payload)` from `src/lib/tauri.ts`. Payloads use
  camelCase via serde rename rules.
- **AppHandle** — Tauri's global handle, threaded through every command that
  needs to reach the config directory or keychain. Functions that don't need
  the keychain (e.g. `touch_connection_activity`) take a path-only path so
  they can stay off the prompt-prone keychain hot path.
- **App Shell** — the React root component (`src/components/app-shell.tsx`).
  Owns the top bar, sidebar, and the foreground health-check loop.
