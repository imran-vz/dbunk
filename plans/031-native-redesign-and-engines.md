# Plan 031: Native redesign and multi-engine workspace

- Status: IN PROGRESS through Step 3 (see [README.md](./README.md)); Step 4
  (engines) has ClickHouse (below), MySQL, SQLite and Redis remain; Step 5
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

## Step 4: ClickHouse (2026-10-04)

ClickHouse connections now work end to end in the native app. MySQL, SQLite
and Redis are separate Step 4 branches.

- **Session**: selecting a ClickHouse connection opens a session
  (`dbunk_lib::backend::clickhouse`): the stored record, the cached secret
  and, for tunnelled connections, an owned SSH route, proven by one bounded
  `SELECT 1` (10 s). Connect, disconnect, connecting and failed states use the
  existing sidebar row and status bar. Failures are classified (unreachable,
  timed out, authentication, missing database, or the ClickHouse error code),
  never server text. Nothing reconnects or retries on its own. A transport
  failure seen by any document or the tree marks the session failed; the user
  reconnects. Editing or deleting the connection, or changing credentials,
  ends its session.
- **Bounds**: native reads stream `JSONCompactEachRowWithNamesAndTypes` and
  stop at a row cap, a byte cap and a deadline (query 10,000 rows / 32 MiB /
  300 s; data page 100 rows / 16 MiB / 60 s; catalog 20,000 objects / 32 MiB /
  30 s; structure 30 s). Results that hit a cap say which one. No server
  setting is sent, so `readonly=1` profiles still work.
- **Policy**: every query document statement passes the connection's
  read-only and safe-mode policy first (shared classifier plus ClickHouse
  heads: `DESCRIBE`/`EXISTS`/`SHOW` read, `OPTIMIZE`/`SYSTEM`/`RENAME`/`KILL`
  DDL, `DETACH` destructive). Safe mode asks for confirmation in the document;
  a confirmed override is audited. One statement per run (ClickHouse HTTP
  accepts one). Stop aborts the request and sends a best-effort
  `KILL QUERY … ASYNC`.
- **Object tree**: all permitted databases from `system.databases` (user
  databases first, then `system`/`information_schema`), each with Tables (with
  engine), Views, Materialized Views (with target table, from the `TO` clause
  or the implicit `.inner`/`.inner_id` table) and Dictionaries (with
  `system.dictionaries` status) as separate groups. External-engine databases
  (MySQL, PostgreSQL, SQLite and their Materialized variants) are listed but
  not expanded, since listing them reads a remote server. An unreadable
  `system.dictionaries` (privileges) is shown as a note, not a failure;
  server-config dictionaries get their own group. Filter, keyboard navigation
  (arrows, Enter = data, Shift-Enter = structure), Refresh and New query.
  The legacy `fetch_schema_explorer` now uses the same catalog: every
  database, materialized views separate from views.
- **Documents** (session-scoped tabs, not persisted across launches): a query
  tab (Cmd-Enter runs the statement at the cursor or the selection, Cmd-.
  stops), table data (100-row pages with a look-ahead row, previous/next,
  header-click sort asc → desc → none) and structure (engine, rows, size,
  sorting key, partition and sample keys, columns with defaults and key
  marks, skip indexes, CHECK constraints, stored DDL). Results use the shared
  read-only grid. `+`/Cmd-T with a ClickHouse connection selected opens a
  ClickHouse query; Cmd-O (PostgreSQL open-table form) points to the tree.
- **Seams shared with the other engine branches**: `DocumentView` gained one
  `Content::ClickHouse` variant (no-op arms in the PostgreSQL-only methods),
  `from_clickhouse`/`clickhouse()` accessors and `is_transient()`, which the
  workspace snapshot uses to skip session-scoped tabs. Workspace hooks are
  one-liners into `workspace_clickhouse.rs` (select/disconnect guard arms,
  connection sync after reload, invalidation on form settle, close on quit,
  tree selection in render); the shell asks `clickhouse_phase`,
  `object_tree()` and `clickhouse_tab_icon`, and `connectable()` admits
  ClickHouse.
- **Verification**: backend unit tests for the bounded reader (split chunks,
  NULLs, row and byte caps, mid-stream exceptions, explicit `FORMAT`), the
  catalog (kinds, MV targets, external databases, truncation, unreadable
  dictionaries) and policy classification; an end-to-end backend test
  against a loopback fake ClickHouse HTTP server (connect, wrong password,
  catalog, browse SQL, structure, server errors, read-only refusal with no
  request sent, production confirmation and audit, closed session, unreachable
  endpoint). Native model tests for session phases (single attempt, stale
  results closed, loss scoped to the session used, invalidation rules), the
  tree (grouping, expansion, filter, bounds) and paging/summaries. Not
  verified: a real ClickHouse server and the running window (no AX or screen
  capture in this harness).
