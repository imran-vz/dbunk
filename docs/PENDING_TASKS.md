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

### Stale "cascade is not wired" comments
- Sources: `src/lib/store/workspace-tabs.ts`,
  `src/lib/store/keyvalue-workspace.ts`,
  `src/lib/store/keyvalue-pubsub.ts`, `src/lib/store/README.md`
- Current state: `Connections.deleteConnection` already calls the full
  cascade (`closeTabsForConnection`, `closeKeyTabsForConnection`,
  `closePubSubSessionsForConnection`,
  `dropOpenQueryStateForConnection`, `dropRelationalCachesForConnection`),
  but the slice docstrings and the README still claim "today this isn't
  wired up." That misleads new contributors.
- Expected work: Update the four docstrings/README to reflect that the
  cascade is live; reword the no-op key-value cleanup methods as
  "reserved for future per-key cache" without the "today the workspace
  tabs slice already cleans..." preamble.

### Default-value expression model is limited
- Source: `src/lib/ddl/shared.ts`
- Current state: Default-value formatting uses a safe simple rule —
  bareword SQL expressions without `()` are not represented well.
- Expected work: Introduce a richer default-expression model that
  distinguishes string literals from SQL expressions.

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
  - Cache replica role per Redis connection session for write checks.
  - Honor `verifyTlsCert=false` in Redis TLS connection construction.
  - Add production log-file target and frontend
    `@tauri-apps/plugin-log` access.

## Cross-Cutting UX Follow-Ups

### Cross-platform window chrome
- Source: `designs/FOLLOWUPS.md`
- Current state: macOS titlebar spacing is reserved unconditionally;
  no platform detection in the chrome layer.
- Expected work: Detect platform and apply spacer/window-control
  layout only where needed.

### Empty, loading, and error state polish
- Source: `designs/FOLLOWUPS.md`
- Current state: The cross-cutting UX pass shipped state shells;
  surface-specific polish remains.
- Expected work: Replace generic empty states with surface-specific
  empty, skeleton loading, inline error, and retry states for the
  remaining surfaces.

### Query editor transaction status
- Source: `designs/FOLLOWUPS.md`
- Current state: Recheck — toolbar has changed since the visual pass.
- Expected work: Reflect live transaction state per editor tab and add
  future transaction controls.

## Documentation Reconciliation

### ROADMAP.md still shows shipped items as ❌
- Source: `ROADMAP.md`
- Current state: Object navigator depth, admin tools, snippets, bind
  variables, import/export, dump/restore, compare, mock data, and
  specialized editors all landed but several items still show ❌.
- Expected work: Update `ROADMAP.md` to match the actual current app.
  Keep only genuinely missing items: Settings driver/session fields,
  schema-map deferrals (annotations, virtual FKs, MySQL/SQLite FK
  introspection, 1:1 detection, multi-schema canvases), PL/pgSQL
  debugger, visual query builder, Parquet, XML import/export.

### `designs/FOLLOWUPS.md` still tracks shipped items
- Source: `designs/FOLLOWUPS.md`
- Current state: The tracker mixes shipped and pending work — e.g.,
  list/set/zset/stream/JSON editors, command palette, Kbd component,
  avatar prefs menu, sidebar gear, toast system, `lastSync` cleanup
  and the Pub/Sub channel discovery flow all landed.
- Expected work: Move shipped items to a "Done" section or strike them
  through so the tracker reflects ground truth.

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
