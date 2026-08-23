# UI Recreation — Follow-ups

Tracker for TODOs, static fixtures, and deferred decisions surfaced during the
pixel-perfect recreation of `DESIGN.md`. Address in a separate session once the
visual pass (Phases 1–8) lands.

Each item: **Where it lives** (file/component) · **What's faked** · **What real
behavior should look like**.

---

## Shipped (closed since 2026-05-13)

Items below ship-checked against `main` on 2026-05-16. Bodies kept for archaeology — search for "✅ Done" markers inline.

- Overview page-level tabs (Tables / Schemas / Query History / Details / Settings) — all routed.
- Table editor `Indexes` and `Relations` sub-tabs — real panels at `indexes-sub-tab.tsx` / `relations-sub-tab.tsx`.
- Query editor `Explain` tab — `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` plan tree.
- Query editor Run dropdown — Run selection / current / all all wired.
- Query editor `Format` button — pipes through `sql-formatter` with engine-aware dialect.
- Query editor connection switcher — `retargetQueryTab` + dropQueryStateForTab cascade (2026-05-16).
- ⌘K command palette, top-bar global search, avatar prefs menu, sidebar footer gear — landed in commit `0567311`.
- `<Kbd>` component with `isMacPlatform` switch — `src/components/ui/kbd.tsx`.
- Toast system — `sonner` `Toaster` in `src/routes/__root.tsx`.
- Redis Tier 2 value editors — List, Set, Sorted Set, Stream, JSON all editable.
- Redis Stream consumer-groups read-only panel — `ConsumerGroupsPanel` driven by `fetchStreamGroups`.
- Redis CLI autocomplete — `cli-catalog.ts` with `suggestCommands`.
- Pub/Sub channel discovery — `PUBSUB CHANNELS` panel in `pubsub-toolbar.tsx`.
- `lastSync` removed; `lastActivityAt` is the canonical activity timestamp.
- State shells (empty/loading/error) — initial pass shipped; surface-specific polish remains.

Items still on the tracker: cross-platform window chrome (Windows/Linux conditional spacer), `Auto-commit` footer transaction-status, Redis Server-tab cards, Sentinel/Cluster, advanced Redis tabs (saved-command, bulk-edit, transaction-builder, scripting, monitor, keyspace notifications), per-key ACL gating, doc panel in CLI, static-map / bit-grid editors, CLI history persistence, destructive-commands TOML generator, pub/sub Tauri event channel, CLI MULTI/EXEC session, replica-role caching, `verifyTlsCert=false` honouring, production log-file target, and Settings tab SSH-tunnel / TCP-keepalive / connect-timeout fields.

---

## Phase 4 — Overview dashboard

### Page-level tabs (Overview / Tables / Schemas / Query History / Details / Settings) ✅
- **Where**: workspace overview header, new tab strip component.
- **Done**: every tab routes to its own sub-view —
  - `Tables` → `workspace-overview/tables-tab.tsx`
  - `Schemas` → `workspace-overview/schemas-tab.tsx` (Schema Map tab also shipped)
  - `Query History` → `workspace-overview/query-history-tab.tsx`
  - `Details` → `workspace-overview/details-tab.tsx`
  - `Settings` → `workspace-overview/settings-tab.tsx`

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

### `Indexes` sub-tab ✅
- **Where**: `src/components/table-editor/indexes-sub-tab.tsx`.
- **Done**: real panel rendering `IndexesSection` from `table-structure/read-only-sections`; opens the index builder via the specialized-editors panel.

### `Relations` sub-tab ✅
- **Where**: `src/components/table-editor/relations-sub-tab.tsx`.
- **Done**: outbound FKs via `ForeignKeysSection` plus an inbound "Referenced by" section sourced from `schemaRelationships`; opens the FK builder via specialized editors.

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

### `Explain` tab ✅
- **Where**: `query-editor-panel.tsx` (`handleExplain`) + `query-editor/results-view.tsx`.
- **Done**: `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` runs against PG with tree-cost-rows-time-loops rendering plus a text fallback.

### Run dropdown (Run selection / current statement / all) ✅
- **Where**: `query-editor/toolbar.tsx` (`onRunSelection` / `onRunCurrent` / `onRunAll`) wired through `use-monaco-query-editor.ts`.
- **Done**: respects Monaco selection range and statement-under-cursor detection via `pickSqlToRun`.

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

### `⌘K` command palette ✅
- **Where**: command-palette component invoked from the top-bar search.
- **Done**: cmdk-style palette over tables, connections, saved queries, and actions — landed in commit `0567311`.

### `Search tables` global search ✅
- **Where**: top-bar search.
- **Done**: opens the command palette which fuzzy-searches the active connection's tables — landed in commit `0567311`.

### Avatar `AD` button ✅
- **Where**: top-bar right (`app-shell-header.tsx`).
- **Done**: prefs dropdown with theme toggle / settings entry — landed in commit `0567311`.

---

## Phase 3 — Sidebar

### Sidebar footer settings gear ✅
- **Where**: bottom of sidebar.
- **Done**: per-connection quick actions menu — landed in commit `0567311`.

---

## Cross-cutting / Phase 8

### Cross-platform window chrome ✅
- **Where**: `AppShell` top bar (`app-shell-header.tsx`).
- **Done**: macOS uses native traffic lights via Tauri's
  `titleBarStyle: "Overlay"` (see `src-tauri/tauri.conf.json`); the React
  placeholder dots are gone. The left spacer that reserves room for the
  traffic lights is now gated on `isMacPlatform() && !isWindowFullscreen`
  (`pl-22` on macOS, `pl-2.5` elsewhere), so Windows/Linux don't pay the
  macOS padding tax.

### Shortcut labels ✅
- **Where**: anywhere a chord is shown.
- **Done**: `src/components/ui/kbd.tsx` exports `<Kbd keys={…} />` plus an `isMacPlatform()` helper; consumers in `app-shell-header.tsx` render `Ctrl+...` on Windows/Linux.

### Empty / loading / error states 🟡
- **Where**: every data-bound surface.
- **Done**: shared state shells (`state-panel.tsx` etc.) plus skeleton loaders + retry shells landed in commit `0567311`.
- **Remaining**: surface-specific polish — bespoke empty illustrations and CTA copy for each data-bound view.

### Toasts / notifications ✅
- **Where**: app-wide.
- **Done**: `sonner` `<Toaster />` mounted in `src/routes/__root.tsx`; consumers (`query-editor-panel.tsx`, `table-editor-panel.tsx`, `settings-tab.tsx`) call `toast.error` / `toast.info` / `toast.success`.

---

## Items added during the visual pass

### `Format` button ✅
- **Where**: `query-editor-panel.tsx::handleFormat` + `src/lib/sql-format.ts`.
- **Done**: pipes the buffer through `sql-formatter` (engine-aware dialect from `activeConnection.engine`), writes back via `updateQuery`, surfaces failure/unchanged states through `toast`.

### Editor connection switcher (DB selector) ✅
- **Where**: `query-editor/toolbar.tsx` dropdown + `query-editor-panel.tsx::handleRetargetConnection`.
- **Done**: filters to relational engines via `storageClassFor`, prompts a themed confirm (the `@/lib/confirm` service, UI refresh P6) when pending grid edits exist, calls `WorkspaceTabsSlice.retargetQueryTab(tabId, connectionId)` which flips `tab.connectionId`, syncs `activeConnectionId`, and cascades through `RelationalQueriesSlice.dropQueryStateForTab` to clear stale queryStatus/edits/previews (2026-05-16).

### Status bar — `Auto-commit ON` ✅
- **Where**: `query-editor/status-items.ts` (`buildQueryStatusItems`).
- **Done (UI refresh P4/P6)**: the status bar shows the live session
  state per tab (`open · autocommit · idle`, etc.) from the persistent
  query session, plus the clickable staged-mutations badge; transaction
  controls live in the query toolbar (`transaction-controls.tsx`).

---

## Redis — Tier 2 + cross-cutting deferred

Captured during the Redis grilling session (May 2026). Tier 1 ships
according to `designs/REDIS_PLAN.md`; the items below are either Tier 2
(earmarked, one PR each, no enforced order) or cross-cutting deferrals
with no Tier promise. Each item has enough context to pick up cold.

### Tier 2 editors (one PR per type)

#### List editor ✅
- **Where**: `src/components/keyvalue/viewers/ListValueView.tsx`.
- **Done**: `applyRedisListEdits` queues `LPUSH`/`RPUSH`/`LSET` and tag-then-`LREM` deletes; offset-paged with direction toggle.

#### Set editor ✅
- **Where**: `src/components/keyvalue/viewers/SetValueView.tsx`.
- **Done**: `applyRedisSetEdits` queues `SADD`/`SREM` with bulk paste.

#### Sorted-set editor ✅
- **Where**: `src/components/keyvalue/viewers/SortedSetValueView.tsx`.
- **Done**: `applyRedisSortedSetEdits` covers `ZADD` (with flag handling), score-in-place edit, and `ZREM`. Geo secondary panel still renders the lat/lng table — static-map rendering remains deferred.

#### Stream editor 🟡
- **Where**: `src/components/keyvalue/viewers/StreamValueView.tsx`.
- **Done**: `applyRedisStreamEdits` queues `XADD` (auto/explicit ID), `XDEL`, and `XTRIM MAXLEN ~`; consumer-group panel renders `XINFO GROUPS` / `XINFO CONSUMERS` read-only.
- **Remaining**: consumer-group write actions — `XGROUP CREATE`/`DESTROY`/`DELCONSUMER`, `XACK`, `XCLAIM`, `XGROUP SETID`.

#### JSON editor ✅
- **Where**: `src/components/keyvalue/viewers/JsonValueView.tsx`.
- **Done**: `JSON.SET` per-path edit, `JSON.DEL` per-path delete; JSONPath expression builder UI sits alongside the read-only viewer.

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

#### Persist CLI history across app restart ✅
- **Where**: `src/components/keyvalue/CliTab.tsx` + `redis_cli_history` SQLite table (migration 6) + `load_redis_cli_history` / `append_redis_cli_history` Tauri commands.
- **Done**: SQLite-backed history with a global 1000-entry cap trimmed on every insert. `CliTab` hydrates a `recallHistory` array on mount (per connection) and prepends each submission, so arrow-up recall survives tab close. Scrollback (commands + their results) is still session-scoped — old commands don't backfill the pane without their original output.

#### Generate destructive-command lists from TOML at build time
- **Where**: `src-tauri/src/redis/destructive-commands.toml` (source) → `destructive_commands.rs` + `src/lib/redis/destructive-commands.ts`.
- **Hook**: Both lists are hand-mirrored today. Add a node script to read the TOML and emit both files; CI asserts they match the TOML.

#### Switch pub/sub to Tauri event channel
- **Where**: `src-tauri/src/redis/pubsub.rs` + `src/components/keyvalue/PubsubTab.tsx`.
- **Hook**: Today the backend buffers and the frontend polls every 750 ms via `redis_pubsub_drain`. Replace with Tauri's `app.emit_to("pubsub-msg", ...)` and a frontend `listen()` for push-based delivery.

#### CLI MULTI/EXEC session state
- **Where**: `src-tauri/src/redis/cli.rs`.
- **Hook**: `MULTI` then `EXEC` works only if the same physical connection is held. Add per-CLI-tab session state with a dedicated connection that's kept alive between `run_command` calls, plus a `QUEUED`/`EXEC`-pending visual indicator in the CLI input.

#### Stream consumer groups sub-panel ✅
- **Where**: `src/components/keyvalue/viewers/StreamValueView.tsx` (`ConsumerGroupsPanel`) + `fetchStreamGroups` on the Rust side.
- **Done**: read-only `XINFO GROUPS` / `XINFO CONSUMERS` panel toggled below the stream table. Write actions remain in the Stream editor remaining-work list above.

#### Cache replica role per connection session ✅
- **Where**: `src-tauri/src/redis/key_ops.rs::assert_writable` + `connection::cached_replica_role` / `cache_replica_role` / `drop_cached`.
- **Done**: writes consult a per-`connection_id` cache (`REPLICA_ROLE_CACHE`) before falling back to a focused `INFO replication` probe; the cache is cleared on disconnect / delete so a reconnect re-probes. A failure or missing `role:` field falls through to "allow write" (matching the prior behaviour for managed Redis instances that block `INFO`).

#### Honor verifyTlsCert=false on Redis connections ✅
- **Where**: `src-tauri/Cargo.toml` + `src-tauri/src/redis/url.rs`.
- **Done**: redis-rs's `tls-rustls-insecure` feature is enabled in Cargo.toml; `url::build` now appends `#insecure` to the URL when `use_tls && !verify_tls_cert`, so redis-rs's parser sets `ConnectionAddr::TcpTls { insecure: true }` and the rustls handshake skips cert verification. Three url-builder tests cover the on/off/plain-scheme cases.

#### Production-mode logging to file + frontend `@tauri-apps/plugin-log` access 🟡
- **Where**: `src-tauri/src/lib.rs::build_log_plugin()` + Tauri capabilities + `package.json`.
- **Done**: dev logging via `tauri-plugin-log` writes to stdout + webview; level is `Debug` for `dbunk_lib`, `Warn` elsewhere. Visible in `pnpm tauri dev` terminal and the browser DevTools console. Release builds additionally rotate into `TargetKind::LogDir` (default file name) under `app.path().app_log_dir()` — `~/Library/Logs/codes.imran.dbunk/` on macOS, `%APPDATA%/codes.imran.dbunk/logs/` on Windows, `~/.local/share/codes.imran.dbunk/logs/` on Linux.
- **Remaining**: add the frontend `@tauri-apps/plugin-log` package + capability so JS code can call `info()`/`error()` against the same logger. Settings UI to surface the log path + level toggle would be a polish item on top.

---

## Connection-record cleanups (added 2026-05-12)

### `lastSync` overlaps with `lastActivityAt` ✅
- **Where**: previously across `connections.ts`, `types.ts`, sidebar/overview render sites.
- **Done**: `lastSync` removed from `Connection`; `formatLastChecked` is a pure formatter over `lastActivityAt` ISO timestamps. One source of truth, no drift.
