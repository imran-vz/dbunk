# Redis — Phased Coverage Plan

Anchored on ADR-0008 (engines fork at a `StorageClass` layer; relational and
keyvalue share the connections + credentials + tab-workspace chrome but have
separate dispatch, overview envelopes, policy unions, and workspace shells)
and ADR-0009 (Redis writes are allowed by default; server-signal drives
auto-read-only; destructive commands are guarded). Tier 1 ships a complete
Redis browser plus editors for the two highest-leverage types; Tier 2
fills in the remaining editors and the connection-form refinements. Each
phase is sized to ship as one PR.

## Shipped status (2026-05-12)

**Tier 1 — all five phases landed in a single development pass.**

- **Phase 1.1 — shipped.** `StorageClass` enum + dispatch split (`dispatch/relational.rs` + `dispatch/keyvalue.rs`) + redis-rs 0.27 wiring + connection-form Redis branch (new + edit). SQLite migration v2 adds `db_number` / `use_tls` / `verify_tls_cert` columns. Cross-language snapshot test asserts `engine → storage_class` agreement.
- **Phase 1.2 — shipped.** `KeyspaceBrowser` (SCAN-driven flat list with type-filter chips, search input, "Load more" cursor pagination) + `KeyInspectorTab` (three-region: header / value / metadata drawer) + all seven type viewers (`StringValueView`, `HashValueView`, `ListValueView`, `SetValueView`, `SortedSetValueView`, `StreamValueView`, `JsonValueView`). Tagged-union `WorkspaceTab` adds `key`/`cli`/`pubsub`/`server` kinds.
- **Phase 1.3 — shipped.** `CliTab` (REPL + history navigation + type-aware result rendering + destructive-command typed-confirmation modal), `ServerTab` (six v1 cards — Identity, Keyspace, Memory, Clients, Replication, Modules; plus stretch Slow log + Persistence), `PubsubTab` (pattern input, split-view channel summary + filtered message log, polling drain). Singletons enforced in `KeyValueWorkspace`.
- **Phase 1.4 — shipped.** `StringValueView` + `HashValueView` gain edit modes (HashValueView stages pending sets / deletes and commits both atomically on Save). `KeyInspectorTab` header gains Delete (typed confirmation), Rename, Expire/Persist modals. `NewKeyDialog` creates keys of any of the seven types with type-specific initial-value parsing.
- **Phase 1.5 — this section.**

### Reality-vs-plan deltas

Documented here for honesty; follow-ups in `designs/FOLLOWUPS.md`.

- **`redis_command_history` SQLite table not added.** The CLI keeps history in component state for the open session only — closing the tab drops it. Added to deferrals as "persist CLI history across app restart". Rationale: the in-memory history works for the day-to-day case, and shipping the table required schema + cap + read/write helpers we'd rather defer until a user actually asks for persistent history.
- **Destructive-commands codegen not wired.** The TOML at `src-tauri/src/redis/destructive-commands.toml` is documentation today; both `destructive_commands.rs` and `src/lib/redis/destructive-commands.ts` are hand-mirrored. Risk of drift is real but small (the list rarely changes); added to deferrals as "generate Rust + TS destructive-command lists from TOML at build time".
- **Pub/sub uses polling drain instead of Tauri events.** The backend buffers messages per session in a `Mutex<VecDeque>`; the frontend polls `redis_pubsub_drain` every 750 ms. Same UX, slightly less efficient than a push-based Tauri event stream. Added to deferrals as "switch pub/sub to Tauri event channel".
- **`MULTI`/`EXEC` not tracked in the CLI yet.** Each `run_command` is an independent execution against the connection manager; sending `MULTI` then `EXEC` on the same session works only if the manager keeps the same underlying connection (which it doesn't guarantee). Added to deferrals as "CLI MULTI/EXEC session state".
- **Stream consumer groups (XINFO GROUPS / CONSUMERS) not rendered.** The stream viewer ships entry browsing only; the consumer-group sub-panel is read-only-deferred. Added to deferrals.
- **Auto-read-only check fires `INFO replication` on every write.** No cache today. Cheap (one round trip) but wasteful. Added to deferrals as "cache replica role per connection session".
- **TLS verify-cert toggle is wired into the form but not yet honored.** redis-rs's URL parser takes `rediss://` for TLS but doesn't expose a "skip verification" knob via URL; honoring `verify_tls_cert: false` requires constructing an explicit `ConnectionInfo` with `rustls`-skip-verify config. Form field is captured + persisted; the backend uses cert-verifying defaults. Added to deferrals as "honor verifyTlsCert=false on Redis connections".

References:
- Backend dispatch (current, flat): `src-tauri/src/dispatch.rs`
- Backend types: `src-tauri/src/types.rs`
- Frontend engine type: `src/lib/store.ts:34` (`DatabaseEngine`)
- Frontend policy: `src/lib/engine-policy.ts`
- PG reference module: `src-tauri/src/postgres.rs`
- CH reference module: `src-tauri/src/clickhouse.rs`
- Existing connection form: `src/components/new-connection-form.tsx`
- Existing edit form: `src/components/edit-connection-dialog.tsx`
- App shell + workspace: `src/components/app-shell.tsx`, `workspace-view.tsx`
- Engine-name literal sites that will need migration:
  `table-structure-view.tsx:822`, `edit-connection-dialog.tsx:40,125,126`,
  `new-connection-form.tsx:40,158,159,480,484`.

---

## Tier 1 — A complete Redis browser, plus the two highest-leverage editors

Goal: a user can save a Redis connection, browse its keyspace, inspect any
of the seven supported types, run arbitrary commands via the CLI, watch
pub/sub traffic, see server-level health, edit string and hash keys, and
delete / rename / expire / create any key. The shell is its own (Redis-
specific) workspace; the relational shell is untouched.

Locked-in scope (from the grilling session — full record in commit
history; ADR-0008 and ADR-0009 carry the load-bearing decisions):

- **Standalone Redis only.** Sentinel and Cluster deferred.
- **Core types + RedisJSON.** Module detection at connect time. Bitmap /
  HLL / Geo render as their underlying type with a secondary panel.
- **Read viewers for all seven types** (string, hash, list, set, sorted
  set, stream, JSON). **Edit only for string and hash** in Tier 1.
- **Four tab kinds**: `key` (multi), `cli` (singleton), `pubsub`
  (singleton), `server` (singleton).
- **Hybrid keyspace browser** — prefix tree (lazy SCAN MATCH), search
  input that swaps the panel into flat-result mode, type-filter chips.
- **Three-region key inspector** — sticky header, type-specific value
  region, collapsible right-side metadata drawer.
- **CLI with curated command catalog**, type-aware result rendering
  (`nil`/`int`/`status`/`string`/`error`/`array`), MULTI/EXEC support,
  SUBSCRIBE/MONITOR routed out, hard destructive-command guards.
- **Server tab** — six v1 cards (Identity, Keyspace, Memory, Clients,
  Replication, Modules); slow-log + persistence as stretch.
- **Pub/Sub tab** — hybrid sampling subscription flow, split-view
  display (channel rail + filtered message log), 10k ring buffer,
  capture-to-file escape valve, 64 KB payload cap, backpressure
  sampling with honest indicator.
- **Connection form** — host, port, optional ACL username, optional
  password, DB number (0–15), TLS toggle + verify-cert toggle. No
  read-only toggle in v1; server-signal auto-read-only when
  `role:replica`.

### Phase 1.1 — Engine class + dispatch split + Redis connection skeleton

The smallest meaningful PR. Lands all the structural plumbing as a near-
zero-behavior-change for relational engines, plus enough Redis backend to
"save and test a Redis connection." No Redis workspace UI yet.

**Backend — types & dispatch:**

- `src-tauri/src/types.rs`:
  - Add `Redis` variant to `DatabaseEngine`.
  - Add `StorageClass` enum (`Relational | KeyValue`). One impl method on
    `DatabaseEngine`:
    ```rust
    impl DatabaseEngine {
        pub fn storage_class(&self) -> StorageClass { /* exhaustive match */ }
    }
    ```
    PG/MySQL/SQLite/CH → `Relational`; Redis → `KeyValue`.
  - Extend `StoredConnection` with three optional fields:
    `db_number: u8` (default 0), `use_tls: bool` (default false),
    `verify_tls_cert: bool` (default true). `#[serde(default)]` so
    pre-Redis records still load.
- `src-tauri/src/dispatch.rs` → `src-tauri/src/dispatch/mod.rs`. Split:
  - `dispatch/relational.rs` — every existing function moves here verbatim
    (no behavior change). Internally still matches on `DatabaseEngine`
    over PG/MySQL/SQLite/CH only.
  - `dispatch/keyvalue.rs` — new module. Tier 1.1 surface: just
    `ping_connection_redis`, `fetch_redis_capabilities` (PING + HELLO +
    INFO replication + INFO server + MODULE LIST + DBSIZE as one pipe).
  - `dispatch/mod.rs` — keeps the public function signatures the Tauri
    layer calls, routes by `engine.storage_class()`:
    ```rust
    pub async fn ping_connection(...) -> Result<ConnectResult, String> {
        match connection.engine.storage_class() {
            StorageClass::Relational => relational::ping_connection(...).await,
            StorageClass::KeyValue   => keyvalue::ping_connection(...).await,
        }
    }
    ```
    For relational-only ops (`load_schema_explorer`, `fetch_table_structure`,
    `fetch_schema_relationships`, `execute_ddl`, `commit_cell_edits`,
    `insert_row`, `delete_rows`, `poll_mutation_status`,
    `fetch_database_overview_stats`): a single `KeyValue =>
    Err(not_applicable(...))` arm — one place, not 20.
  - `not_applicable` was reserved in dispatch.rs for this moment (the
    `#[allow(dead_code)]` comes off).
- `src-tauri/src/redis/mod.rs` — new module tree:
  - `redis/mod.rs` — re-exports
  - `redis/connection.rs` — wraps `redis-rs`'s
    `aio::ConnectionManager`. One manager per `connection_id`,
    process-cached behind a `OnceLock<DashMap<String, ConnectionManager>>`
    so reconnects amortize across the schema-explorer-equivalent fan-out.
    `tls-rustls` feature wired here.
  - `redis/url.rs` — DSN builder: `redis[s]://[user[:pass]@]host:port/db`.
  - `redis/capabilities.rs` — the connect-time pipeline (PING / HELLO /
    INFO replication / INFO server / MODULE LIST / DBSIZE in one
    `redis::pipe()`, tolerating per-command `NOPERM` errors via
    `Option<>`).
- Tauri commands in `lib.rs`:
  - `test_connection` already exists — Redis branch flows through the new
    dispatch split. Returns `ConnectResult` plus (new) the
    `RedisCapabilities` blob the form's post-test banner needs.
- Cargo:
  - Add `redis = { version = "0.27", features = ["tokio-comp",
    "connection-manager", "tls-rustls"] }` to `src-tauri/Cargo.toml`.

**Backend — persistence:**

- `src-tauri/src/storage.rs`:
  - SQLite migration adds three nullable columns to `connections`:
    `db_number INTEGER`, `use_tls INTEGER`, `verify_tls_cert INTEGER`.
  - `INSERT`/`UPDATE` paths populate them; `SELECT` paths hydrate with
    defaults (`0`, `0`, `1`) when null.
  - **Defer** the `redis_command_history` migration to Phase 1.3 (lands
    with the CLI tab that produces history rows).

**Frontend — types & policy:**

- `src/lib/store.ts:34`: `DatabaseEngine` gains `"Redis"`.
- `StoredConnection` (line 55) and `Connection` (line 73) gain
  `dbNumber?`, `useTls?`, `verifyTlsCert?`.
- `src/lib/engine-policy.ts`: refactor per Q13. Shared base
  `{ engine, connectionForm }` + storage-class-specific union
  (`RelationalEnginePolicy` | `KeyValueEnginePolicy`). Add
  `relationalPolicy(engine)` and `keyvaluePolicy(engine)` narrowing
  helpers. Add Redis entry with `storageClass: "keyvalue"`,
  `keyvaluePolicy` fields (`defaultDbNumber: 0`, `maxDbNumber: 15`,
  `keyTypeIcons`, `pubSubSupported: true`, `transactionsSupported: true`,
  `destructiveCommands` — pulled from the generated module below).
- Connection form's `ConnectionFormPolicy` gains `showRedisTls: boolean`
  and `showRedisDbNumber: boolean`. The existing
  `showClickHouseHttp: boolean` pattern is the template.

**Frontend — connection form:**

- `new-connection-form.tsx` and `edit-connection-dialog.tsx`: when
  `selectedEngine === "Redis"`, render:
  - `Username` (optional, placeholder "default")
  - `Password` (optional)
  - `DB number` (numeric input, 0–15, default 0)
  - Under **Advanced Options**: `Use TLS` switch → `Verify TLS
    certificate` switch (shown only when TLS is on).
- Default port placeholder switches to 6379 when engine is Redis.
- The existing engine-literal checks in these files (lines listed in the
  References block) gain a Redis arm. Migration to policy-driven flags
  happens incrementally — Phase 1.1 only adds the Redis branch.
- Post-test-connection: render the modules-detected banner.
  ("Detected Redis 7.2.4. Modules: ReJSON v2.6.10, search v2.10.0." or
  "No modules detected." or "Module detection unavailable — your user
  may lack the `MODULE LIST` permission.")

**Frontend — workspace fork (stub):**

- `app-shell.tsx` / `workspace-view.tsx`: when
  `enginePolicy(activeConnection.engine).storageClass === "keyvalue"`,
  render a placeholder workspace ("Redis workspace landing — sidebar
  and tabs coming in Phase 1.2"). This is the seam everything in 1.2
  hangs off; landing the stub here keeps the merge surface contained.

**Health check:**

- `src/components/app-shell.tsx` foreground tick (ADR-0002): the existing
  ping path already dispatches through `ping_connection`, which now
  routes Redis via the new keyvalue dispatcher (uses `PING` instead of
  `SELECT 1`). No frontend changes; the tick is already engine-agnostic.

**Codegen:**

- `scripts/generate-redis-destructive-commands.ts`: reads
  `src-tauri/src/redis/destructive-commands.toml`, emits
  `src-tauri/src/redis/destructive_commands.rs` (a `pub const
  DESTRUCTIVE_COMMANDS: &[&str]`) and `src/lib/redis/destructive-commands.ts`
  (the same list as a TS const).
- `package.json`: add `"generate:redis-commands"` script + run it from
  `prepare`. A test in both Rust and TS asserts the file is in sync with
  the TOML (fails CI when someone edits the generated file directly).
- Initial TOML contents: `FLUSHDB`, `FLUSHALL`, `DEBUG`, `SHUTDOWN`,
  `CONFIG SET`, `CONFIG RESETSTAT`, `SCRIPT FLUSH`, `SCRIPT KILL`,
  `CLIENT KILL`, `KEYS`.

**Tests:**

- `redis/url.rs` unit tests: DSN building covers TLS / no-TLS,
  user-only / user+pass / no-auth, db 0 / db 15.
- `redis/capabilities.rs` unit test: parser for `INFO replication`
  + `INFO server` against captured fixtures (`tests/fixtures/redis/info-replica.txt`
  etc.).
- `engine-policy.test.ts`: snapshot test that every `DatabaseEngine`
  variant returns the expected `storageClass`, matching the Rust
  classification. The same fixture (`tests/fixtures/storage-class.json`)
  is read by a Rust test in `types.rs` — drift breaks CI.

**Risks / decisions:**

- The dispatch split is the largest blast-radius change. Mitigation:
  zero-behavior-change is the bar for the relational extraction. Reviewers
  can verify by running the relational test suite before/after and seeing
  identical output. The Redis-specific code added in 1.1 is small enough
  to be reviewed as a separate logical chunk inside the same PR.
- `storage_class()` and the TS classifier are two sources of truth.
  The cross-language snapshot test prevents drift.

### Phase 1.2 — Keyspace browser + key inspector (read-only, all seven types)

The biggest UI PR of Tier 1. Lands the workspace shell fork plus every
type viewer. No editors yet (Phase 1.4) and no CLI yet (Phase 1.3) —
this PR is "you can browse and inspect."

**Backend — type-specific fetch:**

- `redis/keyspace.rs`:
  - `scan_keys(connection, db, pattern, type_filter, cursor, count)` —
    wraps `redis::AsyncIter` for `SCAN MATCH ... TYPE ... COUNT ...`.
    Returns `{ keys: [{ name, type }], cursor: Option<String> }`. Uses
    server-side `TYPE` filter (Redis 6.0+); on older servers, post-filters.
  - Tree-build helpers — `tree_node_for_prefix(prefix, separator)` — used
    by the prefix-tree mode in the sidebar; each tree node has a `count`
    estimate from the latest SCAN sample.
- `redis/key_inspector.rs`:
  - `fetch_key_metadata(connection, db, key)` — pipelines `TYPE` + `TTL`
    + `OBJECT ENCODING` + `OBJECT REFCOUNT` + `OBJECT IDLETIME` +
    `OBJECT FREQ` (last one only if `maxmemory-policy` is LFU; the
    policy is cached at connect time in `capabilities`).
  - `fetch_string(connection, db, key, max_bytes)` — GET with explicit
    byte cap; returns `{ value, total_bytes, truncated }`. Default cap
    1 MiB; configurable per connection.
  - `fetch_hash(connection, db, key, mode, cursor, count, pattern)` —
    `mode: "full"` runs HGETALL; `"scan"` runs HSCAN. Mode picker uses
    HLEN (taken from the metadata pipeline) against the threshold
    (default 500, configurable per connection).
  - `fetch_set(connection, db, key, mode, cursor, count, pattern)` — same
    pattern with SCARD / SSCAN / SMEMBERS.
  - `fetch_list(connection, db, key, start, stop, direction)` — LRANGE
    with offset semantics. Direction is `head_to_tail` or `tail_to_head`
    (LRANGE with negative indices).
  - `fetch_sorted_set(connection, db, key, mode, ...)` — `mode: "rank"`
    runs `ZRANGE WITHSCORES`; `"score"` runs `ZRANGEBYSCORE`; `"lex"`
    runs `ZRANGEBYLEX`. Direction toggle (`ascending`/`descending`)
    maps to ZRANGE vs ZREVRANGE.
  - `fetch_stream(connection, db, key, direction, last_seen_id, count)` —
    XRANGE or XREVRANGE with cursor. Also returns `XINFO GROUPS` +
    `XINFO CONSUMERS` for the consumer-group sub-panel.
  - `fetch_json(connection, db, key, path)` — `JSON.GET` against the
    given JSONPath; path defaults to `$` for full document.
- `redis/value.rs`:
  - `serialize_value(v: redis::Value) -> SerializedValue` — the
    discriminated JSON shape from Q11: `{ kind: "nil" }`,
    `{ kind: "int", value }`, `{ kind: "status", value }`,
    `{ kind: "string", value, encoding: "utf8" | "hex" | "base64" }`,
    `{ kind: "error", value }`, `{ kind: "array", value: [...] }`.
  - String encoding is auto-detected: try UTF-8 → if valid and no
    control chars, return `utf8`; else return `hex`. Configurable
    override per viewer.
- Dispatch:
  - `dispatch/keyvalue.rs` exposes one Tauri-facing function per shape
    above (`scan_keys`, `fetch_key_metadata`, `fetch_string`,
    `fetch_hash`, `fetch_set`, `fetch_list`, `fetch_sorted_set`,
    `fetch_stream`, `fetch_json`). The `dispatch/mod.rs` routes
    `engine.storage_class() == KeyValue` to them; `Relational` returns
    `not_applicable` for every Redis-specific op.

**Frontend — workspace shell fork:**

- `src/components/workspace-view.tsx`: top-level conditional on
  `enginePolicy(activeConnection.engine).storageClass`. Render
  `<RelationalWorkspace />` (existing content factored out) or
  `<KeyValueWorkspace />` (new).
- New components under `src/components/keyvalue/`:
  - `KeyValueWorkspace.tsx` — sidebar + workspace-tabs orchestration,
    parallel to the existing workspace.
  - `KeyspaceBrowser.tsx` — the hybrid sidebar from Q2:
    - Prefix-tree mode (default): each tree node lazily fires
      `scan_keys` with `MATCH prefix:*`; expand triggers fetch + cache.
    - Search mode (active when input > 2 chars): flat result list,
      `scan_keys` with `MATCH *foo*`, paged via cursor.
    - Type-filter chips below the search: clicking adds `TYPE hash`
      etc. to the SCAN. Multi-select: post-filter (server-side TYPE
      filter applies to one type at a time on Redis 6.0+; for the
      multi-select case we run parallel SCANs and merge).
    - Configurable separator (default `:`), per-connection in the
      KeyspaceBrowser settings popover.
  - `KeyInspectorTab.tsx` — the three-region layout from Q10:
    - `KeyHeader.tsx` — sticky top: name + copy, type badge,
      live-decrementing TTL chip, encoding badge, MEMORY USAGE chip
      (gated for large keys), actions row (Refresh, Copy as CLI,
      Rename, Delete).
    - `KeyValueView.tsx` — routes by `type` to one of seven
      type-specific viewers (read-only for all in Phase 1.2):
      `StringValueView`, `HashValueView`, `ListValueView`,
      `SetValueView`, `SortedSetValueView`, `StreamValueView`,
      `JsonValueView`. Each is its own component file under
      `src/components/keyvalue/viewers/`.
    - `KeyMetadataDrawer.tsx` — collapsible right rail: object info,
      lifecycle, size breakdown, watch toggle (off by default), raw
      RESP panel (collapsed by default).
- Per-type viewers carry pagination governed by Q15:
  - `StringValueView` — single textarea, 1 MiB cap, "load full" CTA.
  - `HashValueView` — two-mode (full vs SCAN) driven by HLEN +
    threshold. Filter input labeled honestly ("filtering all M
    fields" vs "filtering loaded page only").
  - `SetValueView` — same two-mode pattern.
  - `ListValueView` — offset/limit + page-size selector + direction
    toggle.
  - `SortedSetValueView` — mode picker (rank / score / lex) + direction
    toggle.
  - `StreamValueView` — direction toggle (newest-first default) +
    cursor pagination + consumer-group sub-panel (read-only).
  - `JsonValueView` — tree with lazy expand, JSONPath bar, client-side
    text filter.
- Bitmap / HLL / Geo render under their underlying viewer plus a
  `KeySecondaryPanel.tsx` that surfaces type-specific operations:
  - Bitmap → bit grid (first 8 KB), `BITCOUNT`, `BITPOS 0`/`BITPOS 1`.
  - HLL → `PFCOUNT` result + ad-hoc union form.
  - Geo → `GEOPOS` lat/lng table, `GEOSEARCH` form. No map in Tier 1.

**Workspace-tab integration:**

- `store.ts`: the `WorkspaceTab` type becomes a tagged union. Today's
  `kind: "table" | "query"` gains `kind: "key" | "cli" | "pubsub" |
  "server"`. The `key` tab carries `(connectionId, dbNumber, keyName)`
  identity. The relational types stay untouched.
- Singletons (`cli`/`pubsub`/`server`) are enforced in the tab-open
  flow: opening one focuses the existing instance instead of creating a
  duplicate.
- Default tab on Redis-connection activation: `server` (the workspace
  fork opens a server tab; Phase 1.3 lands the content).

**Tests:**

- Component tests for each viewer (`StringValueView.test.tsx`, etc.):
  loading state, error state, paginated state, filter-scope label
  accuracy.
- Unit tests for `tree-node-from-keys.ts` (the helper that turns
  `["user:1:profile", "user:1:sessions", "user:2:profile"]` into a tree).
- Backend unit tests on `scan_keys` against an embedded Redis fake
  (use `redis-test-server` or a docker-compose fixture in CI; if cost
  is a concern, mock at the `redis::cmd` layer instead).

**Risks / decisions:**

- The seven viewers are seven distinct UIs; biggest correctness risk is
  shipping a viewer that looks fine on small data and chokes on real
  scale. Each viewer's test suite must cover a "large key" case
  (10k+ elements) using mocked fetch responses.
- The hybrid tree-vs-flat sidebar mode swap can flicker on quick typing.
  Debounce the search input at 200 ms; pre-empt in-flight SCANs on
  input change (track a cancel token per scan).
- Memory: keeping the prefix-tree-cache and the search-mode-cache and
  each open `key` tab's value in memory is real. Cap the per-tab value
  cache at 4 MiB; evict prefix-tree branches not touched in 5 min.

### Phase 1.3 — CLI tab + Server tab + Pub/Sub tab

Implements the three singleton tab kinds, the command-history persistence,
and the destructive-command guards. After 1.3, a user can do every
read/observe thing they'd want to do.

**Backend — CLI:**

- `redis/cli.rs`:
  - `run_command(connection, db, command_tokens)` — accepts already-
    tokenized command (the frontend tokenizes for autocomplete; the
    backend trusts the tokens). Routes through `redis::cmd(name).arg(...)`.
  - **Destructive-command guard at the backend**: before issuing,
    consult `DESTRUCTIVE_COMMANDS` (generated). For matching commands,
    require the request payload's `confirmed: true` flag; otherwise
    return `{ kind: "needs-confirmation", command, reason }`. The
    frontend renders the typed-confirmation modal, then re-fires with
    `confirmed: true`. Auto-read-only mode (`role: "replica"`) rejects
    all destructive commands regardless of `confirmed`.
  - **SUBSCRIBE/PSUBSCRIBE/UNSUBSCRIBE/MONITOR** rejected at the
    backend with a directive error: "Use the Pub/Sub tab for
    subscriptions." (Not in `DESTRUCTIVE_COMMANDS` — separate rejected-
    commands list, frontend renders a softer message.)
  - **MULTI/EXEC/DISCARD**: tracked in a per-CLI-session state
    (`HashMap<session_id, MultiState>`) on the dispatcher. Backend
    queues commands when in MULTI; EXEC flushes; DISCARD resets.
  - Result serialized via `redis/value.rs::serialize_value` from 1.2.
- `redis/command_catalog.rs`:
  - Static catalog: ~250 core commands + RedisJSON's commands. Source:
    Redis's `commands.json` (BSD-3, checked in at
    `src-tauri/src/redis/commands.snapshot.json` plus a small Rust
    parser that turns it into a `CommandSpec` struct at compile time
    via `include_str!` + lazy parse).
  - Exposed via `fetch_command_catalog()` Tauri command — frontend
    caches the result for autocomplete and inline hints.

**Backend — persistence:**

- `src-tauri/src/storage.rs` — SQLite migration adds a new table:
  ```sql
  CREATE TABLE redis_command_history (
      id TEXT PRIMARY KEY,
      connection_id TEXT NOT NULL,
      db_number INTEGER NOT NULL,
      command TEXT NOT NULL,
      status TEXT NOT NULL,        -- "ok" | "error" | "rejected"
      runtime_ms INTEGER NOT NULL,
      executed_at TEXT NOT NULL,
      FOREIGN KEY (connection_id) REFERENCES connections(id)
  );
  CREATE INDEX redis_command_history_by_connection
      ON redis_command_history(connection_id, executed_at DESC);
  ```
- Cap at 1000 entries per `connection_id` (trim on insert). Separate
  from `query_history` (which stays SQL-only).

**Backend — server tab:**

- `redis/server_info.rs`:
  - `fetch_keyvalue_overview(connection)` — pipelines INFO server, INFO
    keyspace, INFO memory, INFO clients, INFO replication, INFO
    persistence, MODULE LIST, SLOWLOG GET 25. Per-command failures
    (e.g. `NOPERM` on `INFO replication`) parsed into `Option<>` so the
    frontend renders empty-state per card instead of failing the whole
    fetch.
  - Returns `KeyValueOverviewStats { identity, keyspace, memory,
    clients, replication, modules, slow_log: Option<>,
    persistence: Option<> }`.

**Backend — pub/sub:**

- `redis/pubsub.rs`:
  - `start_pubsub_session(connection_id, tab_id, patterns)` — opens a
    dedicated `aio::PubSub` connection. Returns a session handle.
  - `pubsub_subscribe(session_id, pattern)` — issues PSUBSCRIBE on the
    session's dedicated connection.
  - `pubsub_unsubscribe(session_id, pattern)` — PUNSUBSCRIBE.
  - `pubsub_drain(session_id, since)` — non-blocking pull; backend
    forwards buffered messages to the frontend. Backpressure: per-
    pattern token bucket at 1000 msg/sec; excess sampled (every Nth)
    with a `sampling_factor` returned in the payload.
  - `pubsub_close_session(session_id)` — drops the dedicated connection.
  - The discover-channels sampling flow: `start_pubsub_sample(connection,
    duration_ms, max_messages)` — runs PSUBSCRIBE * for the window,
    aggregates seen channels with msg counts and rate, returns the
    summary. Then the dedicated connection is dropped automatically.

**Backend — Tauri command surface (new):**

- `run_redis_command(connection_id, db, tokens, confirmed)`
- `fetch_redis_command_catalog()`
- `list_redis_command_history(connection_id, limit)`
- `clear_redis_command_history(connection_id)`
- `fetch_keyvalue_overview(connection_id)`
- `redis_pubsub_start_session(connection_id, tab_id)`
- `redis_pubsub_subscribe(session_id, pattern)`
- `redis_pubsub_unsubscribe(session_id, pattern)`
- `redis_pubsub_drain(session_id, since_ms)`
- `redis_pubsub_close(session_id)`
- `redis_pubsub_sample(connection_id, duration_ms, max_messages)`

Each command goes through dispatch routing the same way Phase 1.2 ones do.

**Frontend — CLI tab:**

- `src/components/keyvalue/CliTab.tsx`:
  - Top: scrolling result pane (terminal-style, cap 500 blocks; older
    blocks scroll into a "Full session log" drawer).
  - Bottom: CodeMirror input, single-line by default, Shift+Enter for
    newline, Enter to execute (Cmd/Ctrl+Enter optional).
  - Up/Down at start-of-empty-input cycles history.
  - TAB completes command name; dropdown sourced from the cached
    command catalog.
  - Below the input: inline arg-signature hint when the cursor is
    after a completed command name. `?` icon opens a side panel with
    the catalog entry's full doc.
  - Result blocks rendered by `kind`: nil (italicized `(nil)`), int
    (blue pill), status (green pill), string (monospace box, "show
    more" expander, encoding toggle), error (red pill), array
    (recursive — list or key/value table, hinted by the catalog's
    `resultShape` field).
  - "Copy as JSON" / "Copy as RESP" buttons per block.
  - Destructive-command typed-confirmation modal: triggered on the
    `kind: "needs-confirmation"` response. User types the command's
    canonical name (e.g. `FLUSHDB`) to enable the Confirm button.

**Frontend — Server tab:**

- `src/components/keyvalue/ServerTab.tsx`:
  - Header: refresh button (re-runs the pipelined fetch), last-refreshed
    time, server identity quick-view.
  - Six cards in a responsive grid (CSS grid `auto-fit minmax(280px,
    1fr)`):
    1. **Identity** — version, mode, uptime, modules count.
    2. **Keyspace** — per-DB key count, active DB highlighted.
    3. **Memory** — used / peak / rss / fragmentation / maxmemory /
       policy. Progress bar if `maxmemory > 0`.
    4. **Clients** — connected / max / blocked.
    5. **Replication** — role + replica count or master link status.
       Coloured chip echoed on identity hero.
    6. **Modules** — name + version table. Empty state.
  - Stretch cards (ship if runway allows; flag explicitly in PR description):
    7. **Slow log** — last 25 entries, table, refresh button.
    8. **Persistence** — rdb last save + aof status.
  - Per-card empty-state when its source command was rejected.

**Frontend — Pub/Sub tab:**

- `src/components/keyvalue/PubsubTab.tsx`:
  - Empty state with two CTAs: "Discover channels" (opens sampling
    modal) and "Subscribe by pattern" (input).
  - Sampling modal: countdown + live count of channels seen + sampled
    msg/sec. On finish: multi-select of discovered channels, "Subscribe
    to selection" button.
  - Active state: top bar (subscription chips with ×, Pause/Resume,
    Discover, Add subscription input, Search/filter, Buffer indicator,
    Clear buffer, Record-to-file toggle).
  - Split-view body:
    - Left rail (resizable, default 35%): `ChannelSummaryTable.tsx` —
      Channel / Count / Rate / Last seen, multi-select.
    - Right pane: `MessageLog.tsx` — `[HH:MM:SS.mmm] channel: payload`
      rows, click-to-expand, auto-scroll-to-bottom with smart
      manual-scroll detection.
  - Backpressure indicator surfaces when `sampling_factor > 1` per
    pattern.
- `MessageLog.tsx`'s ring buffer is 10k by default, configurable to
  50k. Persisted setting: per-tab.
- Record-to-file: writes JSONL to
  `${configDir}/pubsub-captures/${connectionId}-${ISO timestamp}.jsonl`.
  Bypasses ring-buffer caps; survives app restart.

**Status bar / cross-cutting:**

- `status-bar.tsx` gets a new "Pub/Sub active on N tabs" indicator when
  any pubsub session is live on any connection. Click → focuses the
  newest pubsub tab.

**Tests:**

- `CliTab.test.tsx`: history navigation, multi-line input, destructive-
  command modal flow (the modal fires from the response payload, not from
  a frontend pattern match).
- `ServerTab.test.tsx`: per-card render of empty/populated states;
  per-card error state when the source command was rejected.
- `PubsubTab.test.tsx`: sampling-modal countdown, subscription chips
  add/remove, backpressure indicator visibility, ring-buffer FIFO cap.
- Backend: command-catalog parser test against the snapshot JSON;
  destructive-command-guard tests against a mocked connection.

**Risks / decisions:**

- Pub/sub backpressure under real load is the hardest thing to test in
  CI. We pick a 1000 msg/sec per-pattern limit as a defensible default;
  it's tunable in code (not in UI) for v1. If users hit it, we promote a
  setting in Tier 2.
- The MULTI state lives in the backend (per-session HashMap). If a Tauri
  command from the same session arrives during a MULTI, we queue it. If
  the user closes the CLI tab mid-MULTI, the session is dropped and the
  server-side MULTI is implicitly discarded (the connection drops; we
  rely on Redis's behavior).
- Command-catalog snapshot drift: we pin to a specific Redis version's
  `commands.json`. A small CI check verifies the snapshot file is
  unchanged; updating it is a deliberate PR.

### Phase 1.4 — Editors for string + hash, key-level ops, new-key wizard

The polish tier. After 1.4, a Redis-savvy user with no need for list/
set/zset/stream/JSON editing can use dbunk as their daily driver.

**Backend — editors:**

- `redis/key_inspector.rs` gains write functions:
  - `set_string(connection, db, key, value, ttl: Option<Duration>,
    if_not_exists: bool, keep_ttl: bool)` — wraps `SET key value [EX |
    PX] [NX | XX] [KEEPTTL]`.
  - `set_hash_fields(connection, db, key, fields: HashMap<String, String>)`
    — wraps `HSET key f1 v1 f2 v2 ...` (atomic across fields).
  - `delete_hash_fields(connection, db, key, fields: Vec<String>)` —
    wraps `HDEL key f1 f2 ...`.
- `redis/key_ops.rs`:
  - `del_keys(connection, db, keys: Vec<String>)` — DEL with the
    soft-guard from Q11 (warning when keys.len() > 5).
  - `set_expire(connection, db, key, ttl: Option<Duration>)` — EXPIRE
    or PERSIST.
  - `rename_key(connection, db, old, new)` — RENAME. Soft guard: when
    the source key's MEMORY USAGE > 1 GiB, requires confirmation.
  - `create_key(connection, db, key, type, initial_value)` — calls the
    right command per type: SET / HSET / LPUSH / SADD / ZADD / XADD /
    JSON.SET. v1 supports all seven types at creation time even though
    only string/hash get full editors after creation.
  - `copy_as_cli(key_metadata, value)` — pure backend helper that
    builds a `redis-cli`-compatible command string for the current key's
    state. Used by the "Copy as CLI" action in the header. Returns the
    SET / HSET... / RPUSH... / etc. command(s) needed to recreate the
    value.

**Backend — write-gating:**

- All write functions consult the connection's auto-read-only state
  (cached from connect-time `INFO replication`). If `role == "replica"`,
  return `Err("This connection is read-only — server reports role=replica.")`
  before issuing the command. Plus the destructive-command list applies
  here as elsewhere (no FLUSHDB through the editor surface, even if
  someone wires it).

**Frontend — string editor:**

- `StringValueView.tsx` gains edit affordances:
  - Textarea bound to local state; "Save" button when dirty.
  - TTL picker (presets: Never, 1m, 1h, 24h, 7d, Custom).
  - Encoding toggle for non-UTF8 strings (Auto / Hex / Base64 / Raw)
    — applies on read; on write, the encoding determines how the
    textarea contents are serialized.
  - Save commits via `set_string`; on success, the metadata-drawer
    encoding and size refresh.

**Frontend — hash editor:**

- `HashValueView.tsx` gains edit affordances:
  - Field/value grid with inline edit (click cell → input).
  - "Add field" row.
  - Per-row delete button (lazy commit — staged in the
    `pending-edits-for-key` map until Save).
  - Save commits via `set_hash_fields` + `delete_hash_fields` in
    parallel; on success, refetch (in SCAN mode) or replace local
    state (in full-fetch mode).

**Frontend — key-level ops:**

- `KeyHeader.tsx`:
  - **Delete** — typed-confirmation modal (type the key name).
  - **Expire / Persist** — modal with TTL picker (same as string
    editor's).
  - **Rename** — input modal; live preview ("Will rename `old` →
    `new`"). Soft warning when MEMORY USAGE > 1 GiB.
  - **Copy as CLI** — generates the CLI sequence on the backend (it
    knows the current value); copies to clipboard.
- `FlushDbDialog.tsx` — accessible from the Server tab's Keyspace card
  per-DB row. Hard typed-confirmation modal (type the DB number).
  Disabled entirely when auto-read-only.

**Frontend — new-key wizard:**

- `NewKeyDialog.tsx` — invoked from the keyspace browser sidebar's "+"
  button and from the deleted-key empty state in `KeyInspectorTab`.
- Two-step flow:
  - Step 1: name + type picker (string default; dropdown for the
    other six types).
  - Step 2: type-specific initial value form.
    - string → textarea + TTL.
    - hash → 1+ field/value pairs.
    - list → 1+ initial elements + push direction.
    - set → 1+ initial members.
    - zset → 1+ score/member pairs.
    - stream → 1+ initial entries (field/value pairs).
    - JSON → JSON editor seeded with `{}`.
  - On confirm: calls `create_key`. On success: closes the modal,
    opens the new key in a `key` tab.

**Status surfaces:**

- `KeyInspectorTab.tsx` shows a "Connection is read-only — server
  reports role=replica. Editors disabled." banner when applicable. The
  editor components render in read-only mode under this banner.

**Tests:**

- `StringValueView.test.tsx`: dirty/save flow; TTL change persists; the
  optimistic UI rolls back on backend error.
- `HashValueView.test.tsx`: stage multiple edits + deletes, single
  save commits all atomically (via HSET + HDEL in parallel); rollback
  on partial failure.
- `NewKeyDialog.test.tsx`: type-picker swap clears form state correctly;
  stream wizard accepts heterogeneous entry shapes.
- Backend: `set_string`/`set_hash_fields` against a Redis fixture;
  auto-read-only rejection paths.

**Risks / decisions:**

- HSCAN-mode hash edits can have stale visible-page state after a save
  (per Q15). The editor's "reload to see all changes" warning is the
  honest mitigation; we don't try to splice the saved fields into the
  visible page.
- The "Copy as CLI" path serializes the *current* value. For large
  values (the textarea content can be a multi-MB string), we generate
  the CLI command on the backend and stream-write it to the clipboard;
  no intermediate render. Test: a 4 MiB string round-trips through
  copy-as-CLI without crashing the UI.

### Phase 1.5 — Documentation, ADRs, deferral capture

The docs pass. Lands after 1.4 so the docs reflect what we built, not
what we planned. No code changes (or only trivial ones discovered while
writing the docs).

**Artifacts:**

- `docs/adr/0008-storage-class-fork-relational-vs-keyvalue.md` — written
  in this batch (Tier 1.5 just lands it on `main` once the Phase 1.1–1.4
  PRs are merged; reality may have shifted a paragraph).
- `docs/adr/0009-redis-writes-by-default-with-server-signal-readonly.md`
  — same.
- `CONTEXT.md` — adds the `StorageClass` glossary entry, the
  `Key` / `KeyMetadata` / `KeyValueOverviewStats` / `CommandHistoryEntry`
  entries, the policy-union shape, the new persistence rows and tables
  (db_number/use_tls/verify_tls_cert columns; redis_command_history
  table), updates the Process model to cite the dispatch split.
- `designs/FOLLOWUPS.md` — append the deferral list with one-line
  context per item (so future-you can pick it up cold).
- `designs/REDIS_PLAN.md` (this doc) — annotated with "shipped" markers
  on each Tier-1 phase and any reality-vs-plan deltas.
- `README.md` — Redis added to the supported-engines list. Limitations
  (Sentinel/Cluster, list/set/zset/stream/JSON read-only) called out.

**Risks / decisions:**

- The two ADRs must not contradict ADR-0006 (CH read-only-by-default)
  silently. ADR-0009 cites 0006 and explains why Redis takes a
  different default. Reviewer attention should land squarely on that
  paragraph.

---

## Tier 2 — Editors + connection-form refinements

Each Tier 2 item is its own PR. No enforced order — pick by user demand.
Each PR earns its own design pass (the editors in particular —
sorted-set ZADD-flag combinations and stream XADD-ID semantics each
deserve their own grilling session).

### 2.A — List editor
- `LPUSH` / `RPUSH` (insert at head/tail), `LSET` (replace by index),
  `LREM` (delete by value), `LINSERT BEFORE | AFTER` (insert relative
  to a pivot value). Drag-to-reorder via remove-and-reinsert.

### 2.B — Set editor
- `SADD` (add member), `SREM` (remove member). Bulk add via paste-list.

### 2.C — Sorted-set editor
- `ZADD` with the full flag matrix (`NX`/`XX`/`GT`/`LT`/`CH`/`INCR`).
  Edit-score-in-place (`ZADD CH`) vs add-new-member. `ZREM` (remove by
  member). Care: the "score" cell type is a float that can include
  `+inf` / `-inf`.

### 2.D — Stream editor
- `XADD` with explicit ID / auto-ID. `XDEL` (delete by ID). `XTRIM`
  (truncate by MAXLEN or MINID). Consumer-group management: `XGROUP
  CREATE`, `XGROUP DESTROY`, `XGROUP DELCONSUMER`, `XACK`, `XCLAIM`.

### 2.E — JSON editor
- `JSON.SET` per-path edit. `JSON.DEL` per-path delete. `JSON.ARRAPPEND`
  / `JSON.ARRINSERT` / `JSON.ARRPOP` for arrays. JSONPath expression
  builder UI.

### 2.F — Sidebar DB switcher
- Per-session DB picker in the keyspace browser. Switching invalidates
  open `key` tabs (or scopes them to the prior DB with a banner), re-
  runs INFO/keyspace stats, re-roots SCAN.

### 2.G — Explicit read-only toggle on the connection form
- Adds a `readOnly?: boolean` field on the connection record. When set,
  the editor gating is the OR of `readOnly` and the server-signal
  auto-read-only. Form UX: a toggle under Advanced.

### 2.H — Soft warning surface on multi-replica masters
- Implements the "this server has N replicas — may be production" soft
  notice from Q9. Dismissable per-connection.

### 2.I — Auto-refresh interval on Server tab cards
- Per-card or whole-tab interval picker (Off / 5s / 15s / 30s / 1m / 5m).

### 2.J — Client list card on Server tab
- `CLIENT LIST` parsed into a table. Privacy-sensitive: design pass needs
  to settle the redaction story for `cmd` and `addr` columns on shared
  servers.

### 2.K — ACL list card on Server tab
- `ACL LIST` parsed; password hashes redacted; per-user permission
  summary.

### 2.L — Config viewer / editor card
- `CONFIG GET` filtered table; `CONFIG SET` per-row edit. Edits go
  through the destructive-command guard (CONFIG SET is on the hard
  list). Persistent vs in-memory distinction surfaced.

### 2.M — Latency stats card
- `LATENCY LATEST` + `LATENCY HISTORY` table or sparkline.

---

## Cross-cutting deferred items

Things that will surface during implementation but should not gate Tier 1
or Tier 2. Each has a one-line hook in `designs/FOLLOWUPS.md` so they
can be picked up cold.

- **Redis Sentinel + Cluster deployments.** Connection form gains
  Sentinel discovery; cluster awareness drives multi-node SCAN and
  slot-aware routing. Big enough to warrant its own ADR.
- **Module viewers.** RediSearch (indexes as a sidebar entity,
  FT.SEARCH editor), RedisTimeSeries (charting, TS.RANGE viewer),
  RedisBloom (BF.ADD/BF.EXISTS surfaces), RedisGraph (deprecated
  upstream — likely never).
- **`saved-command` tab kind.** Saved CLI snippets with parameter
  substitution; `EVAL` / `FUNCTION` distinction.
- **`bulk-edit` tab kind.** Multi-key rename-pattern, bulk DEL with
  confirmation, mass-EXPIRE.
- **`transaction-builder` UI.** Compose `MULTI`/`EXEC` blocks visually.
- **`scripting` tab.** Lua editor with `EVAL` / `EVALSHA` execution,
  Redis Functions (`FUNCTION LOAD` + `FCALL`).
- **`monitor` tab kind.** Rate-limited `MONITOR` capture with explicit
  user consent (distinct from `pubsub` because it's full-server, not
  channel-scoped).
- **Keyspace notifications as live-refresh mechanism.** Replaces
  polling with `PSUBSCRIBE __keyspace@0__:*`; needs a connection-level
  opt-in (changes `CONFIG SET notify-keyspace-events`).
- **Publish-from-UI.** A "Publish to channel" surface in the pubsub
  tab; trivial to implement, deferred because publish-from-CLI is
  fine for v1.
- **`KILL CLIENT` for long-running SCANs.** A "cancel" button on the
  keyspace browser that issues `CLIENT KILL ID ...` on the SCANning
  connection.
- **Multi-key compare / watched-keys UX.** Side-by-side diff of two
  keys; persistent "watched keys" with auto-refresh.
- **Per-key Redis ACL gating.** When the connection's user has ACL
  rules with key-pattern restrictions (`~user:*`), the inspector
  surfaces "this user cannot access keys outside the matching
  patterns" and gates accordingly.
- **Client-side argument validation in CLI.** Pre-flight arity / type
  check against the command catalog before sending; saves a server
  round trip on malformed commands.
- **Doc panel in CLI.** Full command documentation (examples, related
  commands, since-version) — sourced from `COMMAND DOCS` (Redis 7+)
  with a fallback to the static catalog.
- **Sidebar DB switcher with open-tab reconciliation.** Listed under
  Tier 2 above; called out here for completeness.
- **Explicit read-only toggle on connection form.** Same.
- **Auto-refresh interval on Server tab cards.** Same.
- **Static-map rendering for Geo keys.** Replace the lat/lng table
  with an actual map view; defers a new dependency choice.
- **Bit-grid editing for bitmaps.** Flip individual bits; today
  bitmap edits go through the string editor (raw byte edit) or CLI
  (`SETBIT`).
