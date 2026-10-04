# Plan 031: Native redesign and multi-engine workspace

- Status: IN PROGRESS through Step 3 (see [README.md](./README.md)); Step 4
  has Redis (below); MySQL, SQLite and ClickHouse and Step 5 (window
  acceptance) remain. Requested by Imran on
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

## Step 4: Redis (2026-10-04)

Selecting a saved Redis connection opens a native session and shows the Redis
surface: the sidebar keyspace tree replaces the PostgreSQL navigator, the tab
bar shows the Redis tabs, and the workspace opens on the console.

- **Session** (`backend/src/backend/development/redis_session.rs`):
  `Backend::open_redis_session` resolves the saved record, its secret and SSH
  route under the development gate, then opens two dedicated connections
  (console lane, browse lane) within 10 s. No reconnect and no retry: a
  transport error or a command unanswered in 15 s latches the session as lost;
  the row and status bar show the failure and the user retries by selecting
  the connection again. Server error replies (`WRONGTYPE`, …) stay console
  replies.
- **Keyspace tree**: `db0–dbN` from `CONFIG GET databases` (capped at 16;
  without it, databases holding keys plus the default) with exact totals from
  `INFO keyspace`. Expanding a database loads one bounded page (up to 8 `SCAN`
  calls of `COUNT 200`, at most 1,000 names, one pipelined `TYPE`), grouped
  into string, hash, list, set, zset, stream and other. Group counts are
  labelled `~N` (sample share of the total) until a scan completes. "Load
  more" continues the cursor on request; at most 5,000 keys are retained per
  database and 500 listed per group. A failed page keeps its samples and
  cursor.
- **Console** (first tab, not closable): `redis-cli` quoting, Up/Down history
  (100), a 200-entry transcript rendered virtually, `SELECT` tracked in the
  prompt. Guards: the existing ADR-0009 destructive list and Pub/Sub refusal
  (`redis::cli::guard`, now shared with the old CLI path), plus
  `redis::console_policy`: blocking commands (`BLPOP`, `XREAD … BLOCK`,
  `WAIT`, …) and connection-changing ones (`HELLO`, `QUIT`, `CLIENT REPLY`, …)
  are refused; anything not on the read allowlist counts as a write. Read-only
  connections refuse writes (unknown commands included); connections whose
  resolved safety level is not Disabled (staging/production by default)
  confirm writes with an explicit "Run COMMAND" button. Replies are bounded to
  2,000 nodes and 256 KiB, and 500 lines each.
- **Key inspector** (one tab per key, at most 8, oldest replaced): type, TTL,
  encoding and length, then the first 200 elements (or 64 KiB of a string)
  through the existing `key_inspector` fetchers, now generic over a
  caller-owned connection (`fetch_*_on`). Non-UTF-8 key names are listed
  escaped and inspected from the console.
- **Ownership**: each Redis surface owns its session and aborts its Tokio jobs
  on disconnect, loss, connection edit/delete, credential change and window
  close; late results are dropped by generation.
- **Shared seam** (for the other engine branches):
  `apps/native/src/workspace_engines.rs` holds `EngineSurface` (one variant per
  engine; Redis today) and the per-connection surfaces. The shell asks it for
  the tree, tab strip, body and phase of the selected connection;
  `Workspace::activate` lets it intercept connection selection/disconnect and
  tab shortcuts first. No PostgreSQL document changed.
- **Verification**: backend unit tests (keyspace parsing, database list,
  console admission, reply bounds), a profile test for the saved-record open
  path (refused port fails once, wrong engine refused), and a live test run
  against a disposable Valkey 9.1 container (`DBUNK_REDIS_TEST_PORT`): browse,
  run, inspect, lane isolation, stored read-only/staging policy and loss
  latching. Native model tests cover tokenizing, grouping, estimates, bounds
  and the failed-page cursor. Not verified: the real window, keyboard and AX
  (no screen-recording or AX trust in this harness).
