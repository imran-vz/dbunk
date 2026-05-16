# UI Recreation — Follow-ups

Tracker for TODOs, static fixtures, and deferred decisions surfaced during the
pixel-perfect recreation of `DESIGN.md`. Address in a separate session once the
visual pass (Phases 1–8) lands.

Each item: **Where it lives** (file/component) · **What's faked** · **What real
behavior should look like**.

---

## Phase 4 — Overview dashboard

### Page-level tabs (Overview / Tables / Schemas / Query History / Details / Settings)
- **Where**: workspace overview header, new tab strip component.
- **Faked**: only the `Overview` panel renders. Other tabs are visual-only chips
  with no route binding.
- **Real**: each tab routes to its own sub-view.
  - `Tables` → list of tables for the active connection (separate from sidebar
    tree, more like a sortable index).
  - `Schemas` → relocate the existing `SchemaRelationshipMap` here.
  - `Query History` → full history view (currently only a "Recent Queries" card).
  - `Details` → expanded connection metadata (server version, uptime, parameters).
  - `Settings` → per-connection settings (timeouts, default schema, role).

### Connection Details card
- **Where**: `WorkspaceDatabaseOverview`.
- **Faked**: `Region: us-east-1` and `SSL: Enabled` are hardcoded.
- **Real**: derive from the stored connection record. Region only applies to
  managed services — show `—` or hide the row when unknown. SSL should reflect
  the actual TLS state of the live connection, not the saved preference.

### Health banner — `View health checks` action
- **Where**: bottom of overview.
- **Done**: a 30 s foreground tick now drives healthy / unreachable variants
  (Group A3). Latency is rendered when present.
- **Remaining**: the `View health checks` button has no destination yet —
  ideally a panel listing per-check status (TCP reachability, `SELECT 1`,
  replication lag, max-connections headroom) with timestamps and history.

---

## Phase 5 — Table data browser

### `Indexes` sub-tab
- **Where**: new sub-tab on `TableEditorPanel`.
- **Faked**: placeholder panel.
- **Real**: list indexes with name, columns, type (btree/gin/etc.), unique flag,
  size. Source from `pg_indexes` + `pg_stat_user_indexes`.

### `Relations` sub-tab
- **Where**: new sub-tab on `TableEditorPanel`.
- **Faked**: placeholder panel.
- **Real**: list FK relationships in/out of this table; eventually a focused
  graph view (subset of `SchemaRelationshipMap`).

### Status pills in the data grid
- **Where**: `DataGrid` cells.
- **Faked**: pills are visual-only — any column literally named `status` and
  containing `active`/`inactive`/`pending` will render colored.
- **Real**: needs a column-type/enum-aware rendering layer. PG enums and
  `CHECK ... IN (...)` constraints should drive which columns get pill
  treatment, with a per-column override.

### Summary card metadata
- **Where**: bottom-right of table browser.
- **Faked**: `Last vacuum` and `Last analyze` will start empty/`—`.
- **Real**: query `pg_stat_user_tables.last_vacuum` /
  `last_autovacuum` / `last_analyze` for PG; per-engine fallbacks.

### Pagination
- **Where**: table browser footer.
- **Faked**: client-side pagination over already-loaded rows.
- **Real**: server-side paging (LIMIT/OFFSET or keyset) so `Showing 1 to 10 of
  6,921 rows` is accurate for huge tables. Wire `10 rows` selector to the page
  size sent over Tauri.

---

## Phase 6 — Query editor

### `Explain` tab
- **Where**: results area below editor.
- **Faked**: placeholder panel.
- **Real**: run `EXPLAIN (ANALYZE, FORMAT JSON) ...` against PG, render the plan
  tree (cost, rows, actual time, loops). Per-engine variants later.

### Run dropdown (Run selection / current statement / all)
- **Where**: green `Run` split button.
- **Faked**: the dropdown options exist but the only path executed is the
  whole-buffer run.
- **Real**: respect Monaco selection range; for "current statement" detect the
  statement under the cursor by parsing `;` boundaries (avoiding string/comment
  contents).

### Bottom status `Auto-commit ON`
- **Where**: editor footer.
- **Faked**: literal text.
- **Real**: reflect the live transaction state — `Auto-commit ON`,
  `In transaction`, `Failed transaction`. Toggleable from a future Tx panel.

---

## Phase 7 — Connections screen

### Card kebab menu
- **Where**: each card.
- **Faked**: only Edit / Delete planned.
- **Real**: also `Duplicate`, `Test connection`, `Disconnect`, `Copy connection
  string`. Need confirmation flow for Delete (already exists as
  `DeleteConnectionDialog` — reuse).

---

## Phase 2 — Top bar / shell

### `⌘K` command palette
- **Where**: top-bar search.
- **Faked**: keybinding hint shown; pressing `⌘K` focuses the input but no
  palette opens.
- **Real**: command palette over tables, connections, saved queries, and
  actions. Probably `cmdk` library.

### `Search tables` global search
- **Where**: top-bar search.
- **Faked**: input is decorative.
- **Real**: fuzzy search across the active connection's schemas/tables;
  selecting a result opens the table browser tab.

### Avatar `AD` button
- **Where**: top-bar right.
- **Faked**: static initials, no menu.
- **Real**: dropdown for app preferences, theme toggle, sign-in (when cloud
  sync arrives), logout.

---

## Phase 3 — Sidebar

### Sidebar footer settings gear
- **Where**: bottom of sidebar.
- **Faked**: opens nothing or routes to existing connections view.
- **Real**: per-connection quick actions (Reconnect, Disconnect, Edit, View in
  connections screen).

---

## Cross-cutting / Phase 8

### Cross-platform window chrome
- **Where**: `AppShell` top bar.
- **Done**: macOS uses native traffic lights via Tauri's
  `titleBarStyle: "Overlay"` (see `src-tauri/tauri.conf.json`); the React
  placeholder dots are gone.
- **Remaining**: the 78 px left padding is reserved unconditionally — looks
  off on Windows/Linux where the OS draws controls above the header. Detect
  platform and conditionally apply the spacer (or wrap behind a
  `<WindowControls />` component). Also keep title-bar height variable so
  Windows/Linux can shrink it.

### Shortcut labels
- **Where**: anywhere a chord is shown (`⌘K`, `⌘N`, `⌘Enter`, `⌘S`, `⌘F`).
- **Faked**: macOS glyphs hardcoded.
- **Real**: detect platform and render `Ctrl+...` on Windows/Linux. Single
  `<Kbd>` component.

### Empty / loading / error states
- **Where**: every data-bound surface (data grid, history, saved queries,
  cards).
- **Faked**: most surfaces only render the happy path or a generic "No data".
- **Real**: bespoke empty states (illustration + CTA), skeleton loaders for
  initial fetch, inline error surfaces with retry.

### Toasts / notifications
- **Where**: app-wide.
- **Faked**: not present.
- **Real**: connect/disconnect success, query failures, save-confirmations.
  Likely `sonner`.

---

## Items added during the visual pass

### `Format` button
- **Where**: query editor toolbar (`query-editor-panel.tsx`).
- **Faked**: button does nothing — `handleFormat` is a no-op stub.
- **Real**: pipe the buffer through a SQL formatter (`sql-formatter` package or
  similar) and write back via `updateQuery`. Engine-aware dialect selection.

### Editor connection switcher (DB selector)
- **Where**: query editor toolbar dropdown.
- **Faked**: dropdown lists every connection but `onClick` is empty, so picking
  a different connection does not retarget the editor.
- **Real**: thread the selection through `tab.connectionId`, persist back
  into `workspaceTabs`, and re-key `tableStructure` lookups for completions.

### Status bar — `Auto-commit ON`
- **Where**: `query-editor-panel.tsx`.
- **Faked**: literal text.
- **Real**: live transaction state per editor tab —
  `Auto-commit ON / In transaction / Failed transaction`. Toggleable from a
  future Tx panel. (Connection-status portion of the status bar already
  reflects the live health-check tick added in Group A3.)

---

## Redis — Tier 2 + cross-cutting deferred

Captured during the Redis grilling session (May 2026). Tier 1 ships
according to `designs/REDIS_PLAN.md`; the items below are either Tier 2
(earmarked, one PR each, no enforced order) or cross-cutting deferrals
with no Tier promise. Each item has enough context to pick up cold.

### Tier 2 editors (one PR per type)

#### List editor
- **Where**: `src/components/keyvalue/viewers/ListValueView.tsx` (read-only
  in Tier 1.2; grows edit affordances here).
- **Hook**: `LPUSH` / `RPUSH` (head/tail insert), `LSET` (replace by index),
  `LREM` (delete by value), `LINSERT BEFORE | AFTER` (relative insert).
  Drag-to-reorder via remove-and-reinsert.

#### Set editor
- **Where**: `src/components/keyvalue/viewers/SetValueView.tsx`.
- **Hook**: `SADD` / `SREM`. Bulk-add via paste-list.

#### Sorted-set editor
- **Where**: `src/components/keyvalue/viewers/SortedSetValueView.tsx`.
- **Hook**: `ZADD` with the full flag matrix (`NX`/`XX`/`GT`/`LT`/`CH`/
  `INCR`). Edit-score-in-place vs add-new-member. `ZREM` by member.
  Care: score can include `+inf` / `-inf`. Each flag combination needs its
  own test row; this editor warrants its own grilling pass.

#### Stream editor
- **Where**: `src/components/keyvalue/viewers/StreamValueView.tsx`.
- **Hook**: `XADD` with explicit ID / auto-ID. `XDEL` by ID. `XTRIM` by
  MAXLEN or MINID. Consumer-group ops: `XGROUP CREATE`, `XGROUP DESTROY`,
  `XGROUP DELCONSUMER`, `XACK`, `XCLAIM`. ID semantics (the `*` auto-ID,
  explicit `ms-seq` format) deserves its own grilling pass.

#### JSON editor
- **Where**: `src/components/keyvalue/viewers/JsonValueView.tsx` (read-only
  + JSONPath query in Tier 1.2).
- **Hook**: `JSON.SET` per-path edit, `JSON.DEL` per-path delete,
  `JSON.ARRAPPEND` / `JSON.ARRINSERT` / `JSON.ARRPOP` for arrays.
  JSONPath expression builder UI.

### Tier 2 connection-form refinements

#### Sidebar DB switcher
- **Where**: `src/components/keyvalue/KeyspaceBrowser.tsx`.
- **Hook**: Per-session DB picker. Switching invalidates open `key` tabs
  (or scopes them to the prior DB with a banner), re-runs INFO/keyspace
  stats, re-roots SCAN. Tier-1 leaves DB selection fixed per connection
  record (users create multiple connections for multiple DBs).

#### Explicit read-only toggle
- **Where**: `new-connection-form.tsx` / `edit-connection-dialog.tsx`,
  under Advanced.
- **Hook**: A `readOnly?: boolean` connection field. Belt-and-braces for
  users on masters who want guaranteed safety. ADR-0009 documents why
  this is deferred (the server-signal `role:replica` heuristic plus
  destructive-command guards cover the most-common safety case; the
  friction tax on local-dev is not worth the marginal win).

#### Soft warning on multi-replica masters
- **Where**: workspace shell, after Q9-shaped capabilities fetch.
- **Hook**: A dismissable notice when `role:master` with
  `connected_slaves > 0`. Dismiss-state persisted on the connection
  record (`dismissed_replica_warning_at`). Could ship in Tier 1.4 if
  cheap; flagged as Tier 2 to keep Tier 1.4 scope tight.

### Tier 2 Server-tab cards

#### Auto-refresh interval
- **Where**: `src/components/keyvalue/ServerTab.tsx`.
- **Hook**: Per-card or whole-tab interval (Off / 5s / 15s / 30s / 1m /
  5m). Tier-1 has manual refresh only — avoids surprise traffic on
  managed Redis.

#### Client list card
- **Where**: `src/components/keyvalue/ServerTab.tsx`.
- **Hook**: `CLIENT LIST` parsed into a table. Privacy-sensitive — the
  design pass needs to settle redaction for `addr` / `cmd` on shared
  servers.

#### ACL list card
- **Where**: `src/components/keyvalue/ServerTab.tsx`.
- **Hook**: `ACL LIST` parsed; password hashes redacted; per-user
  permission summary.

#### Config viewer / editor card
- **Where**: `src/components/keyvalue/ServerTab.tsx`.
- **Hook**: `CONFIG GET` filtered table; `CONFIG SET` per-row edit
  routed through the destructive-command guard. Persistent
  (`CONFIG REWRITE`) vs in-memory distinction surfaced.

#### Latency stats card
- **Where**: `src/components/keyvalue/ServerTab.tsx`.
- **Hook**: `LATENCY LATEST` + `LATENCY HISTORY` table or sparkline.
  Niche; users hitting latency problems usually have other tools.

### Cross-cutting deferred (no tier promise)

#### Redis Sentinel + Cluster deployments
- **Where**: connection form + `dispatch/keyvalue.rs` + keyspace browser.
- **Hook**: Sentinel discovery (form fields for sentinel addresses +
  service name; backend resolves master). Cluster awareness (multi-node
  SCAN, slot-aware command routing, no DB selection beyond 0). Big
  enough to warrant its own ADR. Tier-1 rejects cluster/Sentinel URIs
  with a clear "not supported yet" error at the connection-test step.

#### Module viewers — RediSearch / RedisTimeSeries / RedisBloom / RedisGraph
- **Where**: new tab kinds or new viewers under
  `src/components/keyvalue/modules/`.
- **Hook**: RediSearch indexes become a sidebar entity (alongside keys);
  `FT.SEARCH` editor. RedisTimeSeries needs a charting library decision.
  RedisBloom (`BF.ADD` / `BF.EXISTS`) is a thin viewer. RedisGraph is
  deprecated upstream — likely never. Tier 1 detects all four via
  `MODULE LIST` and surfaces them on the Server tab's Modules card, but
  ships no per-module viewer.

#### `saved-command` tab kind
- **Where**: `src/components/keyvalue/SavedCommandsTab.tsx` (new).
- **Hook**: Saved CLI snippets with parameter substitution; `EVAL` /
  `FUNCTION` distinction. Sits alongside `saved_queries` conceptually
  but the shape differs enough to want its own table
  (`saved_redis_commands`).

#### `bulk-edit` tab kind
- **Where**: `src/components/keyvalue/BulkEditTab.tsx` (new).
- **Hook**: Multi-key rename-pattern, bulk DEL with confirmation,
  mass-EXPIRE. Power-user feature; needs guard rails (preview-before-
  apply, hard typed-confirmation per N keys).

#### `transaction-builder` UI
- **Where**: `src/components/keyvalue/TransactionBuilderTab.tsx` (new).
- **Hook**: Compose `MULTI`/`EXEC` blocks visually. CLI handles this
  textually in Tier 1; the visual builder is a Tier-2+ nice-to-have.

#### `scripting` tab — Lua / Redis Functions
- **Where**: `src/components/keyvalue/ScriptingTab.tsx` (new).
- **Hook**: Lua editor with `EVAL` / `EVALSHA` execution, Redis Functions
  (`FUNCTION LOAD` + `FCALL`). Code-editor surface; CodeMirror with Lua
  mode. Specialist need; defer.

#### `monitor` tab kind
- **Where**: `src/components/keyvalue/MonitorTab.tsx` (new).
- **Hook**: Rate-limited `MONITOR` capture (entire-server command
  firehose, not channel-scoped). Distinct from `pubsub` because it
  covers every command. Tier 1's CLI explicitly routes `MONITOR` here
  with a directive message; the tab does not yet exist.

#### Keyspace notifications as live-refresh mechanism
- **Where**: replaces polling-based watch toggles in Key Inspector
  + the keyspace browser.
- **Hook**: `PSUBSCRIBE __keyspace@<db>__:*` instead of polling.
  Requires a connection-level opt-in because enabling
  `notify-keyspace-events` is a server-side `CONFIG SET`. Tier 1 polls
  on demand only (no auto-refresh).

#### Publish-from-UI
- **Where**: `src/components/keyvalue/PubsubTab.tsx`.
- **Hook**: A "Publish to channel" surface in the pubsub tab. Trivial
  to implement; deferred because publish-from-CLI is fine for Tier 1.

#### `KILL CLIENT` for long-running SCANs
- **Where**: `KeyspaceBrowser.tsx` cancel button.
- **Hook**: A "cancel" affordance on in-progress SCANs that issues
  `CLIENT KILL ID ...` against the SCANning connection. Today, cancel
  drops the cursor client-side but the server may still be working on
  the in-flight COUNT batch.

#### Multi-key compare / watched-keys UX
- **Where**: new `compare` tab kind or per-key "Watch" sidebar.
- **Hook**: Side-by-side diff of two keys; persistent "watched keys"
  list with auto-refresh. Mirrors the relational "saved tables" idea.

#### Per-key Redis ACL gating
- **Where**: dispatch + Key Inspector.
- **Hook**: When the connection's user has key-pattern ACL rules
  (`~user:*`), the inspector surfaces "this user cannot access keys
  outside the matching patterns" and gates accordingly. Tier 1 trusts
  the server to reject ACL-violating commands; this would surface that
  proactively.

#### Client-side argument validation in CLI
- **Where**: `src/components/keyvalue/CliTab.tsx` + command catalog.
- **Hook**: Pre-flight arity / type check against `command_catalog.rs`
  before sending; saves a server round trip on malformed commands.
  Tier 1 lets the server reject and surfaces the error verbatim.

#### Doc panel in CLI
- **Where**: `src/components/keyvalue/CliTab.tsx`.
- **Hook**: Full command documentation (examples, related commands,
  since-version) sourced from `COMMAND DOCS` (Redis 7+) with a fallback
  to the static catalog. Tier 1 has the inline arg-signature hint
  only; the `?` icon is wired but the side panel is empty.

#### Static-map rendering for Geo keys
- **Where**: `src/components/keyvalue/viewers/SortedSetValueView.tsx`'s
  Geo secondary panel.
- **Hook**: Replace the lat/lng table with an actual map view. Defers
  a static-map dependency choice (Leaflet? mapbox-gl? raster tiles?).

#### Bit-grid editing for bitmaps
- **Where**: `src/components/keyvalue/viewers/StringValueView.tsx`'s
  Bitmap secondary panel.
- **Hook**: Flip individual bits visually (`SETBIT key offset value`).
  Tier 1 bitmap edits go through the string editor (raw byte) or CLI.

### Reality-vs-plan deltas from Tier 1 implementation (added 2026-05-12)

#### Persist CLI history across app restart
- **Where**: `src/components/keyvalue/CliTab.tsx` + new `redis_command_history` SQLite table.
- **Hook**: Tier 1 keeps CLI history in component state only — closing the tab drops it. Add the table from the original plan plus read/write helpers and a 1000-entry cap.

#### Generate destructive-command lists from TOML at build time
- **Where**: `src-tauri/src/redis/destructive-commands.toml` (source) → `destructive_commands.rs` + `src/lib/redis/destructive-commands.ts`.
- **Hook**: Both lists are hand-mirrored today. Add a node script to read the TOML and emit both files; CI asserts they match the TOML.

#### Switch pub/sub to Tauri event channel
- **Where**: `src-tauri/src/redis/pubsub.rs` + `src/components/keyvalue/PubsubTab.tsx`.
- **Hook**: Today the backend buffers and the frontend polls every 750 ms via `redis_pubsub_drain`. Replace with Tauri's `app.emit_to("pubsub-msg", ...)` and a frontend `listen()` for push-based delivery.

#### CLI MULTI/EXEC session state
- **Where**: `src-tauri/src/redis/cli.rs`.
- **Hook**: `MULTI` then `EXEC` works only if the same physical connection is held. Add per-CLI-tab session state with a dedicated connection that's kept alive between `run_command` calls, plus a `QUEUED`/`EXEC`-pending visual indicator in the CLI input.

#### Stream consumer groups sub-panel
- **Where**: `src/components/keyvalue/viewers/StreamValueView.tsx` + backend `XINFO GROUPS` / `XINFO CONSUMERS`.
- **Hook**: Read-only consumer-group + consumer panel below the stream entries. Tier 2 stream editor will add `XGROUP CREATE` / `XACK` / `XCLAIM` on top.

#### Cache replica role per connection session
- **Where**: `src-tauri/src/redis/key_ops.rs::assert_writable`.
- **Hook**: Every write runs `INFO replication` to check role. Cache the result for the connection's lifetime (invalidate on reconnect / replication topology change). One round trip per write becomes one per session.

#### Honor verifyTlsCert=false on Redis connections
- **Where**: `src-tauri/src/redis/connection.rs` + `redis::ConnectionInfo` construction.
- **Hook**: redis-rs's URL parser ignores `verify=false`-style query params. Construct `ConnectionInfo` manually with `rustls`-skip-verification config when `verifyTlsCert == false`. Form already captures + persists the toggle.

#### Production-mode logging to file + frontend `@tauri-apps/plugin-log` access
- **Where**: `src-tauri/src/lib.rs::build_log_plugin()` + Tauri capabilities + `package.json`.
- **Done**: dev logging via `tauri-plugin-log` writes to stdout + webview; level is `Debug` for `dbunk_lib`, `Warn` elsewhere. Visible in `pnpm tauri dev` terminal and the browser DevTools console.
- **Remaining**: add `Target::new(TargetKind::LogDir { file_name: None })` so production builds rotate logs into `app.path().app_log_dir()` (per-OS: `~/Library/Logs/dbunk/` on macOS, `%APPDATA%/dbunk/logs/` on Windows, `~/.local/share/dbunk/logs/` on Linux). Also add the frontend `@tauri-apps/plugin-log` package + capability so JS code can call `info()`/`error()` against the same logger. Settings UI to surface the log path + level toggle would be a polish item on top.

---

## Connection-record cleanups (added 2026-05-12)

### `lastSync` overlaps with `lastActivityAt`
- **Where**: `Connection.lastSync` declared in `src/lib/store/types.ts:125`; written in `src/lib/store/connections.ts:38,183,331,354,372`; read by `src/components/workspace-view.tsx:402` via `formatLastChecked`. Parallel field `lastActivityAt` is the canonical activity timestamp per ADR-0004 (`CONTEXT.md` "Last Activity" entry).
- **Faked**: two fields track the same concept. `lastSync` is a frozen display string ("Never" / "Just now" / ISO timestamp snapshotted at write time); `lastActivityAt` is the ISO-8601 source of truth bumped on successful query/connect. The sidebar/overview reads `lastSync`, but the store also writes `lastActivityAt` from the same paths — so the two fields drift whenever one write path forgets the other.
- **Real**: kill `lastSync`. Read `lastActivityAt` and format at the render site (`formatLastChecked` becomes a pure function over the ISO timestamp). One source of truth; sidebar/overview/cards all derive display strings the same way. Surfaced during the connection-form architecture review (Group A/B/C plan) and explicitly deferred to keep that work shape-focused.
