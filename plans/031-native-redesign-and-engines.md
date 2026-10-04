# Plan 031: Native redesign and multi-engine workspace

- Status: IN PROGRESS through Step 3 (see [README.md](./README.md)); Step 4
  (engines) is in progress (MySQL done on its branch, see below) and Step 5
  (window acceptance) remains. Requested by Imran on
  2026-10-03 together with the hard migration (ADR-0033). Supersedes the visual direction of Plans 027–029; their
  behavioral contracts (bounded resources, exact-save recovery, owned workers,
  stored policy, no automatic retry) still apply.
- Design: [interactive mock](./mocks/native-redesign/index.html). Decisions:
  environment signal = **frame + tint**; projects are a **new connection field**
  (folders stay as optional sub-grouping).

## Requirements (from the request)

- Developer-tool density: UI text and icons around 11 px, monospace data.
- Sidebar, top third: macOS traffic lights, collapse button and logo on one
  row; connection search; connections grouped by project and environment, with
  fast switching between projects and environments.
- Sidebar, below: collapsible object tree for the selected connection, shaped
  by engine (PostgreSQL/MySQL schemas with tables, views and other objects;
  ClickHouse databases with tables, views, materialized views and dictionaries;
  Redis logical databases with keys grouped by data type).
- Separate top bar with tabs for the current connection (table data, query,
  structure, tools, …); the workspace fills the rest.
- Environment indicator across the whole window (frame + tint), production
  warning strip.
- Collapsible bottom status bar: connected database, last query latency,
  connection active state.
- Redis workspace starts as a console view.

## Steps

1. **Shell** – theme tokens and a small widget kit at 11 px; transparent
   titlebar with traffic lights in the sidebar; new sidebar, tab bar, status bar,
   environment frame/tint and production strip. Existing documents render inside.
2. **Projects** – `project` on connection organization (storage migration,
   native connection API, form field), connection list grouped by project and
   environment, project/environment switcher and shortcuts.
3. **Restyle documents** – query, table, structure, Objects and the Tool tabs
   adopt the kit; tool launchers move into the tab bar menu and Cmd-K.
4. **Engines** – native connections, object trees and documents for MySQL,
   SQLite and ClickHouse; Redis keyspace tree and console.
5. **Verification** – focused tests per step; window/keyboard/IME acceptance
   when automation is available.

## Engine object models (for the sidebar tree)

- **PostgreSQL**: database → schemas → tables, views, materialized views,
  foreign tables, functions/procedures/aggregates, sequences, types/domains,
  extensions; database-wide event triggers, roles, tablespaces (list-only).
- **MySQL**: a "database" is the schema → tables, views, routines, events,
  triggers.
- **SQLite**: one `main` schema (plus attached databases) → tables, views,
  indexes, triggers.
- **ClickHouse**: no schemas. The server holds databases (`system.databases`);
  each contains tables (with their engine, e.g. MergeTree, Distributed),
  views, materialized views (with their target table) and dictionaries
  (`system.dictionaries`). The current backend explorer lists only the
  connection's database and groups materialized views with views; the native
  tree should list all permitted databases and separate the kinds.
- **Redis**: logical databases (db0–dbN) → keys grouped by type (string, hash,
  list, set, zset, stream) from bounded SCAN pages; counts come from sampling
  and must be labelled as estimates. The workspace is a console first.

## Whole-app theme and interaction pass (2026-10-04)

Requested by Imran after reviewing the redesign: the theme must cover every
surface, not only the workspace. [`DESIGN.md`](../DESIGN.md) is now the design
reference, started from the mock.

- Credential storage restores the three previous options with the previous
  rules: Encrypted SQLite (recommended default, password typed twice,
  acknowledgement), OS keychain, Unencrypted SQLite (acknowledgement). Setup,
  unlock and recovery are non-dismissable full-window gates; unlock has
  "Forgot password?" → confirmed reset. The backend now refuses a password sent
  with a non-encrypted mode and an empty unlock password; previously plain mode
  silently ignored the typed password, which read as "any password works".
- Every form page (connection, credentials, bastions, rename, open table,
  delete, discard) uses the kit: header with a working close button, sections,
  labelled inputs with inline validation, tone-aware messages, footer actions.
  Command palette, managed servers and query library buttons follow the kit.
- Tabs close from their own close button; the separate top-right close-tab
  icon, which silently did nothing without an active closable tab, is removed.
  Overlays now block pointer input to the workspace beneath them.
- Motion: press depth on controls, 160 ms fade/rise for pages, documents and
  popovers, error shake, opacity-only tooltips on icon buttons, and a spring
  for sidebar hide/show. All respect Reduce motion.
- Not yet verified in a real window: this harness cannot capture or drive the
  app window (no screen-recording or AX trust).

## Step 4: MySQL (2026-10-04)

Branch `plan-031-step4-mysql`. A MySQL connection now works end to end in the
native workspace.

- **Session** (`backend/src/backend/mysql_sessions.rs`): `Backend::
  open_mysql_session` resolves the record, its cached secret and its SSH route
  under the development gate and credential guard (as the health probe does),
  then opens one dedicated `sqlx::MySqlConnection` with a 10 s budget. Server
  session defaults (time zone, `sql_mode`) are left alone; a read-only record
  also sets `SESSION TRANSACTION READ ONLY`. A tracked worker owns the
  connection and route and runs one request at a time from a bounded queue
  (8; more is refused as busy). A lost connection, a 30 s metadata timeout, a
  retirement (disconnect, connection or credential change, bastion change, via
  `retire_data`) or shutdown closes it for good; nothing reconnects. The host
  learns why through the session status (`Closed(None)` normal,
  `Closed(Some)` failure).
- **Tree**: databases (`information_schema.SCHEMATA`) → Tables, Views,
  Routines (procedure/function), Events, Triggers (with their table), loaded
  per database on first expansion; at most 5000 names per kind, with a visible
  truncation note. The connection's default database opens with its tables.
- **Documents** (tabs in the shell's tab bar): query (editor, database
  picker, ⌘↵ / Run, Stop sends `KILL QUERY` on a short side connection),
  table data (200-row pages ordered by the primary key, one row read ahead for
  Next), structure (shared MySQL introspection: columns, indexes, foreign keys,
  checks) plus `SHOW CREATE` text, and definitions for views, routines, events
  and triggers. Statements go through the shared SQL classifier and safety
  policy: read-only refuses writes, protected/strict ask for "Run anyway" and
  audit the override. Results use the text protocol (server formatting kept,
  NULL distinct, binary as hex, BIT as an integer) and keep the last result
  set within 1000 rows, 16 MiB and 64 KiB per cell, counting dropped rows.
- **Shell seam** (`apps/native/src/engine_lane.rs`, `workspace_engine.rs`):
  an `EngineLane` enum (one variant per engine) owns a connection's session,
  tree and tabs. The workspace keeps one lane per selected connection and
  asks it for the sidebar tree, tab list, content and connection phase; tab
  and connection commands route to it. PostgreSQL documents are unchanged.
- **Not yet**: MySQL tabs and query text are not persisted across restarts;
  no row editing, export or EXPLAIN; the query runs the whole editor text
  (no statement-under-cursor); no SSH-tunnelled live check. Verified by unit
  tests and a live backend test against a disposable `mysql:8.4` container
  (`DBUNK_MYSQL_LIVE=host:port:password`); no real-window or AX check.

