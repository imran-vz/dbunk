# Pending Tasks Inventory

Last reconciled 2026-05-16 against the working tree.

This file used to be a mix of "small TODOs that should be polished
off" and "future features the size of a multi-week project". The
2026-05-16 session resolved every item in the first bucket
(commit-by-commit history in "Recently Shipped" below). What remains
is roadmap-level feature work that doesn't fit single-PR tasks — kept
here as a hand-off list with one-line summaries; the deep detail
lives in `designs/FOLLOWUPS.md`, `docs/design/PHASES.md`, and
`ROADMAP.md`.

When picking up roadmap items, treat each bullet below as the seed
for its own design pass — most will spawn an ADR or design doc
before code lands.

## Roadmap (feature-scope, queued)

### Postgres / relational

- **Default-value tagged-union** (`src/lib/ddl/shared.ts`) — replace
  the current "bareword whitelist + `()` heuristic" with a tagged
  `{ kind: "literal", text } | { kind: "expression", sql }` model on
  the column record + Literal/Expression toggle in the column form.
  Cross-cutting: column type, introspection round-trip, every form,
  all `formatDefault` callers.
- **Connection Settings driver/session fields** —
  `workspace-overview/settings-tab.tsx` is still a read-mostly mirror
  even though `PgDriverOptions` (ADR-0013) ships statement-timeout /
  search-path / role plumbing on the backend and the connection form
  already exposes those knobs, SSH tunnel, connect timeout, keepalive,
  and the TLS mode / certificate paths (ADR-0025). Expand the Settings
  UI to mirror them.
- **Schema map deferred items** (issue #17) — notes / annotations on
  canvas, virtual / user-drawn relationships, MySQL / SQLite FK
  introspection, 1:1 cardinality detection, multi-schema canvases.
- **PL/pgSQL debugger** — breakpoints, stepping, variable inspection,
  server-extension capability detection.
- **Visual query builder** — relational query builder that generates
  SQL and round-trips with the text editor.
- **Parquet export** — depends on adopting a suitable JS/Rust writer
  with a streaming path.
- **XML import/export** — only if XML is still in the parity scope.

### Redis Tier 2 + cross-cutting (designs/FOLLOWUPS.md)

- **Cross-tab DB switching cascade** — the keyspace-browser picker
  ships, but opening a key tab continues to use the connection's
  default DB. A future pass can either re-key open key tabs or
  reuse the scan session's connection for the key inspector.
- **Sentinel discovery + Cluster awareness** — connection form
  changes, dispatch-layer routing, slot-aware command routing.
- **Module viewers** — RediSearch, RedisTimeSeries, RedisBloom (each
  warrants its own viewer / new tab kind).
- **Advanced Redis tab kinds** — Transaction Builder (visual
  `MULTI`/`EXEC`), Lua / Redis Functions scripting, MONITOR capture.
  (Saved Redis Commands shipped as a CLI-tab integration; parameter
  substitution remains deferred. Bulk DEL / EXPIRE / RENAME-prefix
  shipped as keyspace-browser affordances with dry-run preview +
  typed confirm. Multi-key compare shipped as a side-by-side string
  viewer; hash/list/set/zset/stream comparison is a follow-up.
  Keyspace-notifications "Live" toggle shipped in the keyspace
  browser — relies on the server-side `notify-keyspace-events`
  config being non-empty.)
- **Operational affordances** — static-map rendering for Geo keys.

### Cross-cutting UX

- **Empty / loading / error state polish** — generic state shells
  shipped; replace with surface-specific empty illustrations + CTAs,
  skeleton loaders, inline error + retry on every data-bound view.
- **Query editor transaction status footer** — track per-connection
  transaction state on the Rust side and surface
  `Auto-commit ON / In transaction / Failed transaction` in the
  editor footer with commit/rollback controls.

### Reserved store slices (waiting on a feature)

- `src/lib/store/keyvalue-pubsub.ts` is still a documented no-op —
  meant to track Pub/Sub session client-side state when the auto-
  reconnect feature lands.
- `src/lib/store/keyvalue-workspace.ts` now caches
  `redisCapabilitiesByConnection`; the rest of its surface
  (watched keys, per-session DB switcher state) is reserved for the
  features above.

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
- **Redis CLI history persisted in SQLite** — migration 6 adds
  `redis_cli_history` (id / connection_id / command / submitted_at)
  with a global 1000-entry cap trimmed on each insert. New
  `load_redis_cli_history` / `append_redis_cli_history` Tauri
  commands; `CliTab.tsx` hydrates a `recallHistory` array on mount
  and prepends each submission, so arrow-up recall survives tab
  close. Two new storage tests cover the connection filter and the
  global trim.
- **DDL default-value bareword whitelist** —
  `src/lib/ddl/shared.ts::formatDefault` now emits
  `CURRENT_TIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME`, `LOCALTIMESTAMP`,
  `LOCALTIME`, `CURRENT_USER`, `SESSION_USER`, `USER`, `NULL`, `TRUE`,
  `FALSE` raw (case-insensitive). Three new tests in
  `postgres.test.ts`. The full literal-vs-expression tagged-union
  model is in the roadmap section above.
- **Frontend `@tauri-apps/plugin-log` access** — package installed +
  `log:default` capability granted; `src/lib/log.ts` exports a tiny
  `log.{debug,info,warn,error}` helper that routes to the backend
  logger when in Tauri and falls back to `console.*` otherwise.
- **Publish-from-UI in Pub/Sub** — `redis_pubsub_publish` Tauri
  command issues `PUBLISH channel message`; the toolbar grows a
  collapsible Publish row (channel + message + Send), Enter submits,
  toast surfaces the subscriber count from the integer reply.
- **Client-side CLI argument validation** — `findCommand`,
  `requiredArgCount`, and `validateArgs` in
  `src/lib/redis/cli-catalog.ts` provide bracket-aware arity counting
  with two-word command awareness. CliTab calls `validateArgs` before
  round-tripping; arity shortfalls become inline rejected entries
  with the args hint, saving a server round trip on malformed
  commands. 11 new tests.
- **CLI command documentation strip** — `CliTab` renders a one-line
  strip above the input showing the matched command's signature +
  description (sourced from the static catalog) while the user
  types. Persists after accepting a suggestion as long as the typed
  prefix still resolves to a known command.
- **Explicit Redis read-only toggle** — `RedisStoredConnection.read_only`
  field + migration 7; `assert_writable` refuses writes when the flag
  is set (independent of replica role). New `Read-only` switch in
  `redis-fields.tsx` Advanced section; threaded through
  `connectionSchema`, defaults, `buildRedis`, and
  `defaultValuesFromConnection`.
- **CLI MULTI/EXEC session state** — `cli::run_command` now accepts
  an optional `session_id`; when present, the command routes through
  a per-session dedicated `MultiplexedConnection` (held in a
  `tokio::sync::Mutex` inside a module-static map) so `MULTI ...
  EXEC` queues against one physical request stream instead of being
  fan-shared with other tabs. `CliTab` passes its `tabId` as the
  session ID and fires `closeRedisCliSession` on unmount.
- **Pub/Sub Tauri event channel** — `pubsub::start_session` now takes
  an `AppHandle`; the worker emits each message as a
  `pubsub-message` Tauri event (with `sessionId` for routing) instead
  of relying on the frontend to poll `drain`. `use-pubsub-subscription`
  switched from a 750ms `setInterval` to a single `listen(...)` that
  filters by session ID. The backend buffer + drain endpoint remain
  as a catch-up path the frontend hits once on session-start to
  recover messages received before the listener attached.
- **Stream consumer-group write actions** —
  `redis_create_stream_group` / `redis_destroy_stream_group` Tauri
  commands wrap `XGROUP CREATE [MKSTREAM]` / `XGROUP DESTROY`.
  `ConsumerGroupsPanel` grows an inline "New group" form (name +
  start-ID + MKSTREAM checkbox) and a per-group destroy button with
  typed-confirm. `XACK`/`XCLAIM` are intentionally not surfaced —
  they're consumer-code operations, not interactive UI actions.
- **Replica warning on Redis masters with attached replicas** —
  KeyValue Workspace slice now stores
  `redisCapabilitiesByConnection`; `connectConnection` populates it
  from the capability probe; `closeKeyTabsForConnection` clears it on
  disconnect/delete. KeyspaceBrowser renders a dismissable advisory
  when `role === "master" && connected_slaves > 0`. Dismiss state is
  session-local (a future migration can persist
  `dismissed_replica_warning_at` on the connection record).
- **Deferred SSH tunnel polish** — ADR-0018 follow-up items shipped:
  Bastion Server search/filter and Settings polish, guided stored
  private-key capture, typed host-key trust reset, and advanced
  per-Connection SSH Tunnel options for compression, keepalive
  interval/reply handling, Bastion Server jump chains, and proxy
  commands.
- **Server tab `CONFIG SET` inline editor** — `redis_set_config`
  Tauri command + per-row edit/save/cancel in `ConfigCard`. Guarded
  by `window.confirm` since `CONFIG SET` is a destructive command.
- **Destructive-commands TOML generator** —
  `destructive-commands.toml` is now the single source of truth;
  `scripts/generate-redis-commands.mjs` rewrites the
  `<generated:destructive-commands>` block in
  `destructive_commands.rs` and the whole
  `src/lib/redis/destructive-commands.ts` file from it.
  `engine-policy.ts` imports the generated TS constants instead of
  hand-mirroring. `pnpm run generate:redis-commands` regenerates;
  `pnpm run check:redis-commands` fails if the files drift (wire
  into CI alongside `check:slice-isolation`).
