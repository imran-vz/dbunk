# Pending Tasks Inventory

Last reconciled 2026-05-16 against the working tree. Items that shipped
since the prior sweep (2026-05-13) are summarised in the "Recently shipped"
section at the bottom so the change is auditable.

## Visible In-App Placeholders

### Query editor connection selector is disabled
- Source: `src/components/query-editor/toolbar.tsx`
- Current state: Dropdown opens a "Retargeting the editor's connection
  isn't supported yet — open a new query tab from the target connection's
  sidebar." message and lists connections as disabled items.
- Expected work: Selecting a connection should retarget the editor tab,
  persist the new `connectionId` into `workspaceTabs`, update completion
  context, and re-key any per-connection query state.

### New Redis key dialog warns about read-only types it now edits
- Source: `src/components/keyvalue/NewKeyDialog.tsx`
- Current state: Description still says "only string and hash have inline
  editors after creation — others are read-only until Tier 2," but list,
  set, sorted set, stream, and JSON all have editors today (see the seven
  `*ValueView.tsx` siblings under `src/components/keyvalue/viewers/`).
- Expected work: Drop the outdated warning. If a per-type caveat remains,
  state it accurately (e.g., per-type capability flags) instead of
  blanket "read-only".

### `workspace-view.tsx` claims overview sub-tabs are placeholders
- Source: `src/components/workspace-view.tsx:432`
- Current state: Comment says "the other Phase 1 sub-tabs render
  placeholders today and get filled in by their respective Phase 1
  steps." All Phase 1 sub-tabs are now implemented.
- Expected work: Remove or rewrite the stale comment.

## Source TODOs And No-Op Actions

### Default-value expression model still lacks a true literal/expression split
- Source: `src/lib/ddl/shared.ts`
- Current state: A SQL-keyword bareword whitelist
  (`CURRENT_TIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME`, `LOCALTIMESTAMP`,
  `LOCALTIME`, `CURRENT_USER`, `SESSION_USER`, `USER`, `NULL`, `TRUE`,
  `FALSE`) is emitted raw alongside numeric literals and `()` function
  calls. Arbitrary SQL expressions still require the function form
  (e.g., `now()`, `gen_random_uuid()`) — anything else is quoted as a
  string literal.
- Expected work (deferred): A tagged-union model on the column record
  (`{ kind: "literal", text } | { kind: "expression", sql }`) plus a
  Literal/Expression toggle in the column form. Lets users write
  arbitrary defaults (`(some_function(args))::text`) without relying
  on the whitelist or `()` heuristic. Cross-cutting — would touch
  the column type, introspection round-trip, every form, and all
  callers of `formatDefault`.

## Placeholder Or Reserved Store Slices

### KeyValue workspace client cache
- Source: `src/lib/store/keyvalue-workspace.ts`
- Current state: Slice is a documented no-op reserved for keyspace
  browser, key inspector, watched keys, per-session DB switching, and
  per-key cache cleanup.
- Expected work: Move Redis keyspace/key inspector session state into
  the slice when watched keys or DB switching lands.

### KeyValue Pub/Sub client metadata
- Source: `src/lib/store/keyvalue-pubsub.ts`
- Current state: Slice is a documented no-op reserved for Pub/Sub
  auto-reconnect and per-session client metadata.
- Expected work: Track Pub/Sub sessions client-side and clean them up
  during connection deletion or tab closure.

## Postgres And Relational Follow-Ups

### Connection Settings tab is still read-mostly
- Sources: `src/components/workspace-overview/settings-tab.tsx`,
  `docs/design/PHASES.md`, `ROADMAP.md`
- Current state: Settings mirrors existing connection fields and
  launches edit. Phase 1 intentionally did not introduce SSH tunnel,
  keepalive, statement timeout, default schema/search path, default
  role, or other driver/session knobs.
- Expected work: Add the connection-record fields and backend wiring
  for SSH tunnel, keepalive, statement timeout, default schema/search
  path, default role, and any other driver-level/session options.

### Schema map deferred items
- Sources: `docs/design/PHASES.md`, `docs/design/PLAN.md`,
  GitHub issue #17, `ROADMAP.md`
- Current state: Core schema map shipped, but these items remain
  deferred (still ❌ in `ROADMAP.md`).
- Expected work:
  - Notes / annotations on canvas.
  - Virtual / user-drawn relationships.
  - MySQL / SQLite FK introspection.
  - 1:1 cardinality detection.
  - Multi-schema canvases and custom-pick diagrams.

### PL/pgSQL debugger
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add debugger support for breakpoints, stepping,
  variable inspection, and server-extension capability detection.

### Visual query builder
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add a visual relational query builder that can
  generate SQL and round-trip with the text editor where practical.

### Parquet export
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add Parquet export if the app adopts a suitable
  JS/Rust writer and streaming path.

### XML import/export
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add XML export and XML-to-table import if still part
  of the target parity scope.

## Redis Tier 2 Work

The list/set/zset/stream/JSON editors and the Stream consumer-groups
read-only panel shipped. Remaining Tier 2 / cross-cutting work:

### Redis connection-form refinements
- Source: `designs/FOLLOWUPS.md`, ADR-0009
- Expected work:
  - Per-session DB picker in the keyspace sidebar.
  - Explicit read-only toggle persisted on the connection record.
  - Dismissible warning for masters with replicas.

### Redis Server tab cards
- Source: `designs/FOLLOWUPS.md`
- Expected work:
  - Auto-refresh interval.
  - Client list card from `CLIENT LIST`.
  - ACL list card from `ACL LIST`.
  - Config viewer/editor from `CONFIG GET` / `CONFIG SET`.
  - Latency stats from `LATENCY LATEST` / `LATENCY HISTORY`.

### Redis deployment and module support
- Source: `designs/FOLLOWUPS.md`, `designs/REDIS_PLAN.md`
- Expected work:
  - Sentinel discovery.
  - Cluster-aware keyspace browsing and slot-aware command routing.
  - Module viewers for RediSearch, RedisTimeSeries, RedisBloom, and
    possibly RedisGraph.

### Redis advanced tabs and workflows
- Source: `designs/FOLLOWUPS.md`
- Expected work:
  - Saved Redis commands tab with parameter substitution.
  - Bulk edit tab for multi-key rename/delete/expire with preview.
  - Transaction builder for visual `MULTI`/`EXEC` composition.
  - Lua / Redis Functions scripting tab.
  - Monitor tab for rate-limited `MONITOR` capture.
  - Keyspace notifications opt-in for live refresh.
  - Publish-from-UI in Pub/Sub.
  - Cancel long-running SCANs via `CLIENT KILL`.
  - Multi-key compare and watched-keys UX.
  - Per-key ACL gating hints.
  - Client-side CLI argument validation.
  - CLI command documentation side panel.
  - Static map rendering for Geo keys.
  - Bit-grid editing for bitmap strings.
  - Stream consumer-group write actions
    (`XGROUP CREATE`, `XACK`, `XCLAIM`) on top of the read-only panel.

### Redis stream editor write actions on consumer groups
- Source: `src/components/keyvalue/viewers/StreamValueView.tsx`
- Current state: `XINFO GROUPS` / `XINFO CONSUMERS` render in a
  read-only panel.
- Expected work: Layer editing actions for groups and consumers —
  `XGROUP CREATE`/`DESTROY`, `XACK`, `XCLAIM`, `XGROUP SETID`.

### Redis implementation deltas
- Source: `designs/FOLLOWUPS.md`
- Expected work:
  - Persist CLI history in SQLite with a cap.
  - Generate destructive-command lists from TOML and assert
    frontend/backend parity in CI.
  - Replace Pub/Sub polling drain with a Tauri event channel.
  - Preserve Redis CLI `MULTI`/`EXEC` state on a dedicated per-tab
    connection.
  - Add frontend `@tauri-apps/plugin-log` access so JS code can
    `info()`/`error()` into the same logger as the backend.

## Cross-Cutting UX Follow-Ups

### Empty, loading, and error state polish
- Source: `designs/FOLLOWUPS.md`
- Current state: The cross-cutting UX pass shipped state shells;
  surface-specific polish remains.
- Expected work: Replace generic empty states with surface-specific
  empty, skeleton loading, inline error, and retry states for the
  remaining surfaces.

### Query editor transaction status
- Source: `designs/FOLLOWUPS.md`,
  `src/components/query-editor/status-items.ts`
- Current state: The literal "Auto-commit ON" copy is gone — status
  bar shows connection / tab / cursor / diagnostics. There is no
  transaction-state indicator at all.
- Expected work (feature, not placeholder): Track per-connection
  transaction state on the Rust side, surface it in the editor footer
  as `Auto-commit ON` / `In transaction` / `Failed transaction`, and
  wire commit/rollback controls.

## Search Notes

- Included: `Coming soon`, `TODO`, `deferred`, `follow-up`, `stub`,
  `placeholder for`, `reserved for future`, and design trackers under
  `docs/`, `designs/`, and `ROADMAP.md`.
- Excluded from this tracker: normal form input `placeholder` props,
  test-only stubs, generated lockfiles, Cargo dependency names
  containing `future`, and already-implemented runtime concepts named
  "pending" such as pending edits or pending mutations.

## Recently Shipped (closed since 2026-05-13)

Tracked here for the next person reading this file — these used to be
listed as pending and are now done.

- **Table editor Indexes sub-tab** — real implementation at
  `src/components/table-editor/indexes-sub-tab.tsx` (uses
  `IndexesSection` from `table-structure/read-only-sections`).
- **Table editor Relations sub-tab** —
  `src/components/table-editor/relations-sub-tab.tsx`, including the
  inbound "Referenced by" section.
- **Redis Stream consumer groups read-only panel** — `ConsumerGroupsPanel`
  in `StreamValueView.tsx` driven by `fetchStreamGroups` (`XINFO GROUPS`
  / `XINFO CONSUMERS`).
- **Redis value editors** — list, set, sorted set, stream, and JSON all
  have inline editors (`{List,Set,SortedSet,Stream,Json}ValueView.tsx`).
- **Redis CLI autocomplete** — `src/lib/redis/cli-catalog.ts` powers
  `suggestCommands` in `CliTab.tsx`, with Tab/Enter accept and
  ArrowUp/Down navigation.
- **Pub/Sub discover-channels flow** — `PUBSUB CHANNELS` panel in
  `src/components/keyvalue/pubsub-tab/pubsub-toolbar.tsx`.
- **Query history cross-slice write** —
  `src/lib/store/relational-queries.ts` now routes through
  `get().applyConnectionActivity(connectionId)`.
- **Favorite tables row counts** —
  `favorite-tables-card.tsx` shows `≈n` planner estimates from
  `pg_class.reltuples` (with `—` fallback for unsupported engines).
- **Command palette, Kbd, avatar prefs menu, sidebar gear, state
  shells** — landed in commit `0567311`.
- **Theme persistence, presets, and animated switch** — landed in
  commits `06f09e5`, `212d243`, `0de28c0`.
- **Toast system** — `sonner` `Toaster` in `src/routes/__root.tsx`,
  with `toast()` calls in query/table editors and settings.
- **`lastSync` cleanup** — field removed from `Connection`; all
  consumers read `lastActivityAt`.
- **Stale cascade docstrings** — `workspace-tabs.ts`,
  `keyvalue-workspace.ts`, `keyvalue-pubsub.ts`, and
  `src/lib/store/README.md` updated to reflect that
  `deleteConnection` / `disconnectConnection` invoke the full cascade.
- **NewKeyDialog warning text** — outdated "read-only until Tier 2"
  copy replaced; all seven types have editors.
- **OverviewTabBody stale placeholder comment** — removed.
- **Query editor connection selector** — disabled dropdown replaced
  with a working retarget action backed by
  `WorkspaceTabsSlice.retargetQueryTab` and
  `RelationalQueriesSlice.dropQueryStateForTab`.
- **ROADMAP.md / `designs/FOLLOWUPS.md` reconciled** — ROADMAP top-line
  flipped Connection settings ❌→🟡 to match § 1.5; FOLLOWUPS gained a
  "Shipped" summary and ✅ markers on the sections that landed (page-level
  tabs, Indexes/Relations sub-tabs, Explain tab, Run dropdown, Format
  button, connection switcher, ⌘K palette, top-bar search, avatar prefs,
  sidebar gear, Kbd component, toasts, all Redis Tier 2 value editors,
  stream consumer-groups read-only panel, `lastSync` removal).
- **Cross-platform window chrome** — `app-shell-header.tsx:83`
  already gates the 88px left spacer on `isMacPlatform() && !isWindowFullscreen`,
  so Windows / Linux don't reserve the macOS traffic-light padding.
- **`verifyTlsCert=false` honoured on Redis** — `src-tauri/Cargo.toml`
  enables the redis-rs `tls-rustls-insecure` feature; `redis/url.rs`
  appends `#insecure` to the URL when `use_tls && !verify_tls_cert`
  so the rustls handshake actually skips cert verification instead of
  the toggle being persisted but ignored. Three new url.rs tests.
- **Production log-file target** — `build_log_plugin()` in
  `src-tauri/src/lib.rs` now adds `TargetKind::LogDir` to the targets
  list in release builds, so shipped binaries rotate logs into the
  per-OS app log directory (`~/Library/Logs/codes.imran.dbunk/` on
  macOS, equivalents elsewhere) without the user re-running from a
  terminal. Dev builds keep stdout + DevTools only.
- **Replica-role caching for Redis writes** —
  `src-tauri/src/redis/key_ops.rs::assert_writable` now consults a
  per-`connection_id` cache (`REPLICA_ROLE_CACHE` in
  `connection.rs`) before falling back to `INFO replication`. Cache
  is cleared by `drop_cached` on disconnect/delete so reconnect
  re-probes. One round trip per session instead of per write; four
  new `parse_role` tests.
- **DDL default-value bareword whitelist** —
  `src/lib/ddl/shared.ts::formatDefault` now emits
  `CURRENT_TIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME`, `LOCALTIMESTAMP`,
  `LOCALTIME`, `CURRENT_USER`, `SESSION_USER`, `USER`, `NULL`, `TRUE`,
  `FALSE` raw (case-insensitive). Three new tests in
  `postgres.test.ts`. The full literal-vs-expression tagged-union
  model remains deferred above.
