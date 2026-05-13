# Pending Tasks Inventory

Generated on 2026-05-13 from a repo sweep for visible "Coming soon" surfaces,
TODO markers, deferred comments, placeholder slices, and roadmap/design
follow-ups. This file is the single consolidation point for unfinished work
found in the application and design notes.

## Visible In-App Placeholders

### Table editor Indexes sub-tab
- Source: `src/components/table-editor/sub-tab-placeholder.tsx`
- Current state: `Indexes` is a table-editor sub-tab but still shows
  `Coming soon`.
- Expected work: Render real per-table indexes with name, columns, method/type,
  uniqueness, primary flag, size, and usage stats. For PostgreSQL, source from
  table structure plus `pg_indexes` / `pg_stat_user_indexes`.

### Table editor Relations sub-tab
- Source: `src/components/table-editor/sub-tab-placeholder.tsx`
- Current state: `Relations` is a table-editor sub-tab but still shows
  `Coming soon`.
- Expected work: Render inbound and outbound foreign-key relationships for the
  active table. A focused mini-graph or table-plus-actions view can reuse schema
  relationship-map data.

### Redis Stream consumer groups note
- Source: `src/components/keyvalue/viewers/StreamValueView.tsx`
- Current state: Stream viewer displays `Stream consumer groups (XINFO GROUPS)
  — deferred to Tier 2.`
- Expected work: Add a read-only consumer-group and consumer panel using
  `XINFO GROUPS` / `XINFO CONSUMERS`; later layer editing actions such as
  `XGROUP CREATE`, `XACK`, and `XCLAIM`.

### New Redis key read-only type warning
- Source: `src/components/keyvalue/NewKeyDialog.tsx`
- Current state: Dialog tells users that lists, sets, sorted sets, streams, and
  JSON can be created but only strings and hashes have inline editors.
- Expected work: Add inline editors for list, set, sorted set, stream, and JSON
  Redis key types.

## Source TODOs And No-Op Actions

### Query editor connection selector does nothing
- Source: `src/components/query-editor/toolbar.tsx`
- Current state: Connection dropdown items have `onClick={() => {}}` and a
  `TODO(FOLLOWUPS)` comment.
- Expected work: Selecting a connection should retarget the editor tab,
  persist the new `connectionId` into `workspaceTabs`, update completion
  context, and re-key any per-connection query state.

### Favorite tables card row counts are fake
- Source: `src/components/workspace-overview/favorite-tables-card.tsx`
- Current state: Row count column renders `—` with `TODO(Phase 4 follow-up):
  real row counts`.
- Expected work: Use relation stats or table-data metadata to show approximate
  or exact row counts, with a clear fallback when unavailable.

### Redis CLI autocomplete is deferred
- Source: `src/components/keyvalue/CliTab.tsx`
- Current state: CLI supports command history and execution, but command
  autocomplete/catalog help is explicitly deferred.
- Expected work: Wire a command catalog into suggestions, arity hints, and
  completion for command names and common subcommands.

### Pub/Sub discover-channels flow is deferred
- Source: `src/components/keyvalue/PubsubTab.tsx`
- Current state: Pub/Sub watches user-entered patterns only.
- Expected work: Add a discovery/sample flow for channels so users can browse
  or select active channels before subscribing.

### Query default-value expression model is limited
- Source: `src/lib/ddl/shared.ts`
- Current state: Default value formatting uses a safe simple rule. Bareword SQL
  expressions without `()` are not represented well.
- Expected work: Introduce a richer default-expression model that distinguishes
  string literals from SQL expressions.

### Query history activity update is a cross-slice write
- Source: `src/lib/store/relational-queries.ts`
- Current state: Query completion directly mutates `connections` to bump
  `lastActivityAt`.
- Expected work: Add a `connections` slice helper such as
  `applyConnectionActivity()` and route the update through that API.

### Orphan workspace tabs survive connection delete
- Source: `src/lib/store/workspace-tabs.ts`
- Current state: `closeTabsForConnection(connectionId)` exists, but
  `Connections.deleteConnection` does not call it; orphan tabs intentionally
  preserve pre-refactor behavior.
- Expected work: Wire the delete-connection cleanup cascade end-to-end and close
  relation/key/query tabs for deleted connections.

## Placeholder Or Reserved Store Slices

### KeyValue workspace client cache
- Source: `src/lib/store/keyvalue-workspace.ts`
- Current state: Slice is a documented no-op reserved for keyspace browser,
  key inspector, watched keys, per-session DB switching, and per-key cache
  cleanup.
- Expected work: Move Redis keyspace/key inspector session state into the slice
  when watched keys or DB switching lands.

### KeyValue Pub/Sub client metadata
- Source: `src/lib/store/keyvalue-pubsub.ts`
- Current state: Slice is a documented no-op reserved for Pub/Sub auto-reconnect
  and per-session client metadata.
- Expected work: Track Pub/Sub sessions client-side and clean them up during
  connection deletion or tab closure.

## Postgres And Relational Follow-Ups

### Connection Settings tab is still read-mostly
- Sources: `docs/design/PHASES.md`, `ROADMAP.md`
- Current state: Settings mirrors existing connection fields and launches edit.
  Driver/session fields are not modeled.
- Expected work: Add connection-record fields and backend wiring for SSH tunnel,
  keepalive, statement timeout, default schema/search path, default role, and
  other driver-level/session options.

### Schema map deferred items
- Sources: `docs/design/PHASES.md`, `docs/design/PLAN.md`, GitHub issue #17
- Current state: Core schema map shipped, but these items remain deferred.
- Expected work:
  - Notes / annotations on canvas.
  - Virtual / user-drawn relationships.
  - MySQL / SQLite FK introspection.
  - 1:1 cardinality detection.
  - Multi-schema canvases and custom-pick diagrams.

### PL/pgSQL debugger
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add debugger support for breakpoints, stepping, variable
  inspection, and server-extension capability detection.

### Visual query builder
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add a visual relational query builder that can generate SQL
  and round-trip with the text editor where practical.

### Parquet export
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add Parquet export if the app adopts a suitable JS/Rust
  writer and streaming path.

### XML import/export
- Source: `ROADMAP.md`
- Current state: Not implemented.
- Expected work: Add XML export and XML-to-table import if still part of the
  target parity scope.

## Redis Tier 2 Work

### Redis value editors
- Source: `designs/FOLLOWUPS.md`
- Expected work:
  - List editor: `LPUSH`, `RPUSH`, `LSET`, `LREM`, `LINSERT`, possible reorder.
  - Set editor: `SADD`, `SREM`, bulk add.
  - Sorted-set editor: `ZADD` flag matrix, score editing, `ZREM`.
  - Stream editor: `XADD`, `XDEL`, `XTRIM`, consumer-group operations.
  - JSON editor: `JSON.SET`, `JSON.DEL`, JSON array operations, JSONPath UI.

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
  - Module viewers for RediSearch, RedisTimeSeries, RedisBloom, and possibly
    RedisGraph.

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

### Redis implementation deltas
- Source: `designs/FOLLOWUPS.md`
- Expected work:
  - Persist CLI history in SQLite with a cap.
  - Generate destructive-command lists from TOML and assert frontend/backend
    parity in CI.
  - Replace Pub/Sub polling drain with a Tauri event channel.
  - Preserve Redis CLI `MULTI`/`EXEC` state on a dedicated per-tab connection.
  - Cache replica role per Redis connection session for write checks.
  - Honor `verifyTlsCert=false` in Redis TLS connection construction.
  - Add production log-file target and frontend `@tauri-apps/plugin-log`
    access.

## Cross-Cutting UX Follow-Ups

### Command palette and global search
- Source: `designs/FOLLOWUPS.md`
- Current state: Top-bar search and keybinding hints exist, but full command
  palette/global table search remains incomplete.
- Expected work: Add command palette over tables, connections, saved queries,
  and actions; make global table search open selected tables.

### Avatar / app preferences menu
- Source: `designs/FOLLOWUPS.md`
- Current state: Avatar initials are static.
- Expected work: Add dropdown for app preferences, theme toggle, future cloud
  sign-in/logout hooks.

### Sidebar footer settings gear
- Source: `designs/FOLLOWUPS.md`
- Expected work: Add per-connection quick actions such as reconnect,
  disconnect, edit, and view in connections screen.

### Cross-platform window chrome
- Source: `designs/FOLLOWUPS.md`
- Current state: macOS titlebar spacing is reserved unconditionally.
- Expected work: Detect platform and apply spacer/window-control layout only
  where needed.

### Shortcut labels
- Source: `designs/FOLLOWUPS.md`
- Current state: Some shortcut labels use hardcoded macOS glyphs.
- Expected work: Add a platform-aware `<Kbd>` component and render `Ctrl+...`
  on Windows/Linux.

### Empty, loading, and error states
- Source: `designs/FOLLOWUPS.md`
- Expected work: Replace generic empty states with surface-specific empty,
  skeleton loading, inline error, and retry states.

### Toasts and notifications
- Source: `designs/FOLLOWUPS.md`
- Current state: App-wide toast system is not present.
- Expected work: Add notifications for connect/disconnect success, query
  failures, save confirmations, import/export completion, and backup/restore
  results.

### Query editor Format button
- Source: `designs/FOLLOWUPS.md`
- Current state: Previously documented as a no-op. Recheck current behavior
  before implementation because the toolbar has changed since the visual pass.
- Expected work: If still missing, add SQL formatting with dialect selection and
  write the formatted SQL back into the editor.

### Query editor transaction status
- Source: `designs/FOLLOWUPS.md`
- Current state: Previously documented as literal `Auto-commit ON`. Recheck
  current footer before implementation.
- Expected work: Reflect live transaction state per editor tab and add future
  transaction controls.

### Connection record timestamp cleanup
- Source: `designs/FOLLOWUPS.md`
- Current state: `lastSync` and `lastActivityAt` overlap.
- Expected work: Remove `lastSync`, format `lastActivityAt` at render sites,
  and use one source of truth for sidebar/overview/card activity display.

## Documentation Reconciliation

### ROADMAP.md is stale after the phase implementation pass
- Source: `ROADMAP.md`
- Current state: Several items still show ❌ even though Phases 3-10 now landed
  in code and `docs/design/PHASES.md`, including object navigator depth, admin
  tools, snippets, bind variables, import/export, dump/restore, compare, mock
  data, and specialized editors.
- Expected work: Update `ROADMAP.md` to match the actual current app. Keep only
  genuinely missing items such as PL/pgSQL debugger, visual query builder,
  Parquet, XML import/export, schema-map deferrals, and Settings driver/session
  fields.

### Stale implementation comments
- Sources: `src/components/workspace-view.tsx`, older design files
- Current state: Some comments still mention overview sub-tabs rendering
  placeholders even though many tabs are now implemented.
- Expected work: Clean up stale comments during the next pass touching those
  files.

## Search Notes

- Included: `Coming soon`, `TODO`, `deferred`, `follow-up`, `stub`,
  `placeholder for`, `reserved for future`, and design trackers under `docs/`,
  `designs/`, and `ROADMAP.md`.
- Excluded from this tracker: normal form input `placeholder` props, test-only
  stubs, generated lockfiles, Cargo dependency names containing `future`, and
  already-implemented runtime concepts named "pending" such as pending edits or
  pending mutations.
