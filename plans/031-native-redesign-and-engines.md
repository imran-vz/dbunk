# Plan 031: Native redesign and multi-engine workspace

- Status: IN PROGRESS through Step 3, Step 4 SQLite done (see
  [README.md](./README.md)); Step 4 for MySQL, ClickHouse and Redis and Step 5
  (window acceptance) remain. Requested by Imran on
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

## Step 4: SQLite (2026-10-04)

Branch `plan-031-step4-sqlite`. Selecting a SQLite connection opens a native
SQLite workspace end to end; the sidebar row's connect, disconnect, connecting
and failed states and the status bar now apply to SQLite too.

- **Session** (`backend/src/backend/sqlite_session.rs`,
  `Backend::open_sqlite_session`): one owned worker task per connection holds
  one `SqliteConnection`; requests run in order on a bounded queue (8, refused
  with "busy" when full). The file must already exist (the existing
  `check_sqlite_file` probe, then `create_if_missing(false)`; `ATTACH` cannot
  create files either). Read-only connections open with
  `SQLITE_OPEN_READONLY`. The tree, table pages, structure and the query tab
  share the session, so `ATTACH`, temp tables and an open transaction are
  visible across documents, like a SQLite shell.
- **Bounds**: 2,000 rows per result set, 16 result sets, 16 MiB retained per
  run, 64 KiB per text cell, 256-byte BLOB hex previews, 1,000-row table
  pages, 5,000 tree objects per kind per database, 1 MiB SQL. Rows past a
  bound are still read (every statement runs) and reported as omitted. The
  native grid also charges results to the workspace retention allowance.
- **Safety**: user SQL passes the existing policy (`classify_script` +
  `assert_permitted`): read-only blocks writes, protected/strict (including
  production by default) ask for confirmation of the exact reviewed text, and
  confirmed overrides are audited after success. PRAGMA, ATTACH and REPLACE
  classify as unknown, so they count as writes.
- **Cancel and close**: Stop interrupts only its own request through SQLite's
  progress handler. Disconnect refuses queued work, interrupts the running
  request and joins the worker within 5 s (rolling back an open transaction);
  a worker that misses the deadline is aborted. Quit closes every SQLite
  session before the host shuts down. Nothing reconnects or retries by
  itself; a reconnect drops late results from the previous attempt, and a
  saved change to the path, read-only flag or policy closes the session.
- **Tree**: `main`, `temp` once used, and attached databases (`PRAGMA
  database_list`) → tables, views, indexes, triggers from `sqlite_master`,
  with counts, filter and keyboard navigation (arrows, Enter opens data,
  Shift-Enter opens structure). Indexes and triggers open their table's
  structure. The tree refreshes after a run that may create, drop or attach.
- **Documents**: query tab (editor, ⌘↵ run, ⌘. stop, multiple result sets in
  the shared result grid, row/latency summary, confirmation banner); table
  data (200-row pages in natural order, previous/next, refresh); structure
  (columns with type, NOT NULL, default, key position and generated/hidden
  markers; indexes with origin, partial flag and expression members; foreign
  keys; triggers with definitions; CREATE statement). Tab titles hide `main.`
  and name attached schemas.

Not yet: SQLite tabs are not persisted in the workspace snapshot or listed in
the command palette; no sorting, filtering or cell editing in table data; no
schema tools (DDL editor, export, compare) for SQLite. Checks: backend and
native unit tests against real SQLite files; no window, keyboard/AX or IME
acceptance.
