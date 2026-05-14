# Code Review — Tier 2 Redis + cross-cutting UX (2026-05-14)

**Scope**: `git diff origin/main..HEAD` covering commits `b2c00c5` (Group C — Redis Tier 2: inline editors, server cards, CLI autocomplete, pub/sub discovery) and `0567311` (Group D — command palette, Kbd, prefs menu, sidebar gear, state shells). 32 files, +4188/−167.

**Review profile**: chill. Gates green at review time: bun format / lint / typecheck, just fmt / lint / test, vitest 575/575.

---

## Walkthrough

This range ships two big commits: Group C (Redis Tier 2 — inline editors for list/set/zset/stream/json, stream consumer groups panel, four new Server tab admin cards with global auto-refresh, CLI autocomplete catalogue, Pub/Sub channel discovery) and Group D (cross-cutting UX — global command palette, platform-aware `<Kbd>`, avatar preferences menu, sidebar gear per-connection actions, reusable state shells). ~4188 insertions / ~167 deletions across 32 files. Most weight is in new Tauri write commands (Rust) and per-type editor components (TSX); a smaller polish pass adds shared primitives.

## Changes

| File | Summary |
|------|---------|
| `src-tauri/src/redis/key_ops.rs` | New write Tauri commands for list / set / zset / stream / JSON; tag-and-LREM list delete |
| `src-tauri/src/redis/key_inspector.rs` | `fetch_stream_groups` runs XINFO GROUPS + per-group XINFO CONSUMERS |
| `src-tauri/src/redis/server_info.rs` | New admin endpoints: CLIENT LIST / ACL LIST / CONFIG GET / LATENCY LATEST |
| `src-tauri/src/redis/pubsub.rs` | `discover_channels` via PUBSUB CHANNELS + pipelined NUMSUB |
| `src-tauri/src/dispatch/keyvalue.rs` & `src-tauri/src/lib.rs` | Dispatch + Tauri command registration for the new commands |
| `src-tauri/Cargo.toml` | Adds `uuid` `v4` feature for the delete-sentinel |
| `src/components/keyvalue/viewers/{List,Set,SortedSet,Stream,Json}ValueView.tsx` | Add inline edit-mode with Save / Cancel pending-state pattern |
| `src/components/keyvalue/ServerTab.tsx` | Auto-refresh picker; four admin cards with per-card fetch |
| `src/components/keyvalue/CliTab.tsx` | Autocomplete dropdown driven by `cli-catalog` |
| `src/components/keyvalue/pubsub-tab/pubsub-toolbar.tsx` | "Discover" button + channel list popover |
| `src/lib/redis/api.ts` | TS bindings for all new Tauri commands |
| `src/lib/redis/cli-catalog.ts` (+test) | Static 60-command Redis catalogue with arity hints |
| `src/components/command-palette/command-palette.tsx` | Global Cmd/Ctrl+K palette |
| `src/components/ui/kbd.tsx` (+test) | Platform-aware shortcut renderer |
| `src/components/ui/state-panel.tsx` (+test) | Reusable Loading / Error / Empty / Skeleton shells |
| `src/components/app-shell/app-preferences-menu.tsx` | Avatar dropdown with theme picker (localStorage) |
| `src/components/app-shell/app-shell-header.tsx` | Mac traffic-light gutter gated; header chip uses Kbd |
| `src/components/app-shell.tsx` (+test) | Mounts CommandPalette; platform stub in test |
| `src/components/sidebar.tsx` | Footer gear becomes per-connection dropdown |
| `src/components/workspace-overview/admin-tab.tsx` | Error banner gets Retry |

## Estimated Review Effort

**4 / 5** — large surface, two layers (Rust + TSX), multiple new contracts, and a couple of correctness bugs hiding under a clean test run.

---

## Findings

### P0 — critical (2)

#### P0-1: theme picker is dead UI

**Location**: `src/components/app-shell/app-preferences-menu.tsx:35-43`

`applyTheme` writes `data-theme="dark"`/`"light"` on `<html>`, but `src/styles.css` only defines `.dark { … }` (with `@custom-variant dark (&:is(.dark *))`) — there is no `[data-theme="dark"]` selector anywhere. On top of that, `src/components/app-shell.tsx` unconditionally adds `classList.add("dark")` on mount. As shipped, every theme choice renders identically, so this is user-visible feature theatre.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src/components/app-shell/app-preferences-menu.tsx`, change `applyTheme` to toggle `document.documentElement.classList` (add `"dark"` for dark, remove for light, resolve "system" via `window.matchMedia("(prefers-color-scheme: dark)")` and subscribe to the `change` event so the choice stays in sync). Also remove the unconditional `classList.add("dark")` in `/Users/imran/projects/Code/dbunk/src/components/app-shell.tsx` (its mount effect) so the new picker actually drives styles.

**Status**: Being addressed in the theme-support follow-up (see plan below).

#### P0-2: list edits target wrong elements in `reverse=true` mode

**Location**: `src/components/keyvalue/viewers/ListValueView.tsx:97-105`

`fetch_list` does `LRANGE start stop` head-indexed and reverses the result in memory; it never translates head-indexed ranges into tail-indexed for the `reverse=true` case. The UI's `absIndex` is therefore `page * pageSize + offset` against the *reversed* page (row 0 = original index `stop`), but `toRedisIndex(absIndex)` returns `-(absIndex + 1)`. For a 500-element list in reverse mode, clicking delete on the topmost visible row issues `LSET -1 sentinel; LREM` — which deletes element 499 (tail of the list), not the entry shown. Edits silently target the wrong rows.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src-tauri/src/redis/key_inspector.rs::fetch_list`, when `payload.reverse` is true, translate `start`/`stop` to tail-relative indices before issuing LRANGE (e.g. compute `len = LLEN key`, then `LRANGE key (len - 1 - stop) (len - 1 - start)`, then return without reversing in memory). After that, the UI's `toRedisIndex(absIndex) = -(absIndex + 1)` formula aligns with the displayed row. Add an integration test that round-trips a list-edit through `reverse=true` and asserts the right element was mutated.

---

### P1 — major (6)

#### P1-1: `apply_list_edits` leaks sentinel strings on partial failure

**Location**: `src-tauri/src/redis/key_ops.rs:166-209`

The `LSET ... sentinel` loop awaits each call sequentially; if any LSET fails (e.g. the list shrank under us), prior iterations have already overwritten cells with `__dbunk_del_<uuid>__` and the final `LREM ... sentinel` never runs. The user is then left with a poisoned value visible in the list. The comment claims "Per-edit `LSET` calls are pipelined" — they aren't.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src-tauri/src/redis/key_ops.rs::apply_list_edits`, either (a) wrap the whole sets + sentinel-plant + LREM + appends sequence in `MULTI`/`EXEC` so the deletes commit or roll back atomically, or (b) wrap the sentinel-plant loop in a try/?-style block that always issues the final LREM (even on a planting failure) before propagating the error, so the sentinels get scrubbed. Fix the misleading "Per-edit LSET calls are pipelined" comment. Add a happy-path integration test.

#### P1-2: `apply_stream_edits` returns an error mid-loop after partial XADD commits

**Location**: `src-tauri/src/redis/key_ops.rs:288-309`

Inside the serial `for entry in &payload.appends` loop, `entry.fields.is_empty()` returns `Err(...)` after preceding entries' XADD calls have already committed. The frontend filters fields with `k.length > 0` before send, so a staged draft with all-blank keys produces `fields: []` and surfaces this partial-commit failure mode in practice.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src-tauri/src/redis/key_ops.rs::apply_stream_edits`, validate every `entry.fields` is non-empty before issuing any XADD (one-pass check, then iterate to commit). Alternatively, batch appends + deletes + XTRIM into a single `MULTI`/`EXEC`.

#### P1-3: Enter swallows submit when typed command matches a suggestion

**Location**: `src/components/keyvalue/CliTab.tsx:170-189`

The keydown branch fires `acceptSuggestion` when `suggestionsOpen && suggestions.length > 0 && suggestions[suggestionIndex] && !input.endsWith(" ")`. Typing `GET` produces a suggestion list that starts with `GET`, so the first Enter rewrites input to `"GET "` instead of submitting. The user has to press Enter twice for any command that exactly matches a catalogue head.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src/components/keyvalue/CliTab.tsx`, tighten the suggestion-accept branch so Enter only "accepts" when the user has typed less than the suggested name. Compute `const exactMatch = suggestions[suggestionIndex]?.name.toUpperCase() === input.trim().toUpperCase();` and route through `handleSubmit()` when `input.endsWith(" ") || exactMatch`. Or bind acceptance to Tab only.

#### P1-4: `fetch_stream_groups` is N+1 with no pipelining

**Location**: `src-tauri/src/redis/key_inspector.rs:617-650`

For each group returned by `XINFO GROUPS`, a separate awaited `XINFO CONSUMERS` call runs on the same multiplexed connection. At 10 groups on a remote Redis (50 ms RTT) that's ~500 ms vs ~50 ms pipelined.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src-tauri/src/redis/key_inspector.rs::fetch_stream_groups`, replace the per-group serial `XINFO CONSUMERS` await loop with a single `redis::pipe()` that queues one `XINFO CONSUMERS <key> <group>` per group, then zips the responses with the group names.

#### P1-5: serial-await LSET / XADD loops should be pipelined

**Location**: `src-tauri/src/redis/key_ops.rs:182-209 / 366-381`

Both `apply_list_edits` (per-edit LSET) and `apply_stream_edits` (per-entry XADD) `await` each command individually. For a 50-row Save on a remote Redis, that's 50× RTT serial.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src-tauri/src/redis/key_ops.rs`, replace the serial-await loops with `redis::pipe()` batches that issue all commands in one round trip. Pair with the partial-failure fix.

#### P1-6: `fetch_overview` issues 6 INFO + MODULE LIST + SLOWLOG GET sequentially

**Location**: `src-tauri/src/redis/server_info.rs:100-201`

Eight serial round trips per refresh, run every 5 s when auto-refresh is set to the shortest interval. The header comment advertises "pipelined INFO sections" — the implementation does not match.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src-tauri/src/redis/server_info.rs::fetch_overview`, batch the six `INFO <section>` calls + `MODULE LIST` + `SLOWLOG GET 25` into a single `redis::pipe()` and parse the `Vec<redis::Value>` per arm.

---

### P2 — minor (8)

#### P2-1: `useFetched` duplicates `useRedisFetch` and is missing a request-seq guard

**Location**: `src/components/keyvalue/ServerTab.tsx:144-172`

`src/components/keyvalue/viewers/use-redis-fetch.ts` already exists with cancellation and a request-seq guard. The new local `useFetched` only checks `cancelled`, so a stale slow fetch can race a new one. Also it ignores changes to its `fetcher` arg (suppressed via biome-ignore) and relies on every caller folding all deps into `cacheKey`.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src/components/keyvalue/ServerTab.tsx`, delete the local `useFetched` helper and use `useRedisFetch`. If a `{ data, error, loading }` variant is wanted, extract it once alongside `useRedisFetch` as `useRedisFetchState`.

#### P2-2: ServerTab forks `CardError`/`CardSkeleton`/`CardEmpty` instead of using `state-panel.tsx` shipped in the same commit

**Location**: `src/components/keyvalue/ServerTab.tsx:419-432`

Commit `0567311` adds `src/components/ui/state-panel.tsx` with the explicit purpose "All app panels with a fetch lifecycle should pick one of these". The next file in the same diff defines local `CardError`/`CardSkeleton`/`CardEmpty` and uses those throughout the new cards.

**Fix prompt**:
> In `/Users/imran/projects/Code/dbunk/src/components/keyvalue/ServerTab.tsx`, replace the local helpers with `ErrorState`, `Skeleton`, and `EmptyState` from `@/components/ui/state-panel`. If those shells don't fit visually inside a card body, extend them with a `compact` / `variant="card"` prop.

#### P2-3: theme preference duplicates the existing app-settings plumbing

**Location**: `src/components/app-shell/app-preferences-menu.tsx:23-31`

`AppSettingsSnapshot` already round-trips through `loadAppSettings` / `saveAppSettings`. The new menu hand-rolls a parallel `localStorage["dbunk.theme"]`.

**Fix prompt**:
> Move theme onto the existing app-settings slice: extend `AppSettingsSnapshot` in `src-tauri/src/types.rs` with `theme`, plumb it through `load_app_settings` / `save_app_settings`, then read/write via `useAppStore`.

**Status**: Being addressed in the theme-support follow-up (see plan below) — partially. The follow-up keeps localStorage for fast boot but documents the future SQLite migration.

#### P2-4: bypass `setActiveConnectionId` action with raw `setState`

**Location**: `src/components/command-palette/command-palette.tsx:195-199`

**Fix prompt**:
> Select `setActiveConnectionId` alongside the other actions and replace the inline `useAppStore.setState({ activeConnectionId: item.connectionId })` call with `setActiveConnectionId(item.connectionId)`.

#### P2-5: `loadMore` always re-fetches the boundary entry

**Location**: `src/components/keyvalue/viewers/StreamValueView.tsx:120-135`

**Fix prompt**:
> Prefix the boundary id with `(` when sending to the backend (`(lastId` for XRANGE / XREVRANGE exclusive bounds, supported on Redis 6.2+). Removes the trailing dedupe step.

#### P2-6: page state can desync after Save (SortedSetValueView)

**Location**: `src/components/keyvalue/viewers/SortedSetValueView.tsx:99-115`

**Fix prompt**:
> In `handleSave`, call `setPage(0)` on success so users don't end up viewing an empty rank window when their delete-batch emptied the page.

#### P2-7: 300-table cap prioritises insertion order over the active connection

**Location**: `src/components/command-palette/command-palette.tsx:68-94`

**Fix prompt**:
> Iterate the active connection first so its tables can't get starved by the 300-item cap.

#### P2-8: cli-catalog 12-result cap test is vacuous

**Location**: `src/lib/redis/cli-catalog.test.ts:43-45`

`suggestCommands("X")` produces only 9 entries; no single-letter head in the catalogue produces more than 11. The `.length <= 12` assertion would pass even if `.slice(0, 12)` were removed.

**Fix prompt**:
> Refactor `suggestCommands` to accept an optional catalog argument and inject 20 synthetic entries with a common prefix, then assert `expect(...).length).toBe(12)`.

---

### P3 — trivial (5)

#### P3-1: Windows/Linux render branch untested in `kbd.test.tsx`

**Fix prompt**:
> Stub `navigator.platform` per render: `"MacIntel"` asserting `⌘` and `"Win32"` asserting `Ctrl+K`. Restore in `afterEach`.

#### P3-2: retry callback test should assert exactly-once

**Location**: `src/components/ui/state-panel.test.tsx:24-36`

**Fix prompt**:
> Change `expect(onRetry).toHaveBeenCalled()` to `expect(onRetry).toHaveBeenCalledTimes(1)`. Drop the redundant `.toBeTruthy()` after `getByText`.

#### P3-3: catalog overlaps with engine-policy destructive lists with no cross-reference

**Location**: `src/lib/redis/cli-catalog.ts`

**Fix prompt**:
> Add an optional `destructive?: "hard" | "soft"` field to `CommandSpec` and derive it by intersecting names with `KEYVALUE_REDIS_DESTRUCTIVE_HARD` / `_SOFT` from `src/lib/engine-policy.ts`. Render a destructive badge in the CliTab autocomplete row.

#### P3-4: `redis_value_to_string` duplicates `server_info::scalar`

**Location**: `src-tauri/src/redis/key_inspector.rs:671-687`

**Fix prompt**:
> In `src-tauri/src/redis/value.rs`, add `pub fn scalar_string(value: &redis::Value) -> Option<String>` and `pub fn scalar_u64(value: &redis::Value) -> Option<u64>`. Update `server_info::scalar` and `key_inspector::{redis_value_to_string,redis_value_to_u64}` to call the shared helpers.

#### P3-5: `DraftAppend.fields` index keys hide a missing identity

**Location**: `src/components/keyvalue/viewers/StreamValueView.tsx`

**Fix prompt**:
> Change `DraftAppend.fields` from `Array<[string, string]>` to `Array<{ id: string; key: string; value: string }>` with `crypto.randomUUID()` for new rows. Remove the three `biome-ignore noArrayIndexKey` suppressions in `DraftEditor`.

---

## Security

No findings.

Audited: every new write op in `key_ops.rs` correctly calls `assert_writable`. UUID-based delete sentinel is non-colliding. Pub/Sub and `CONFIG GET` glob patterns are not injection vectors. `parse_client_list_line` is panic-safe. UI renders untrusted Redis values as text content — no XSS. Adding the `v4` feature to an already-bundled `uuid` crate does not expand supply-chain surface.

Pre-existing concern noted but not raised: Server tab's Config card renders `requirepass` / `masterauth` values in plain text — Redis-ACL-governed, any CLI user with `CONFIG GET` permission already sees those.

## Test Coverage

**Added**: 8 catalog tests, 3 Kbd tests, 4 state-panel tests, plus `navigator.platform` stub in `app-shell.test.tsx`.

**Gaps**:
- No integration tests for any new Redis write command (`apply_list_edits`, `apply_set_edits`, `apply_sorted_set_edits`, `apply_stream_edits`, `set_json_path`, `delete_json_path`, `discover_channels`, the four `fetch_*` admin endpoints) — P1-1 and P1-2 would have been caught.
- No UI tests for the new editors (~1000 lines across five viewers). P3 finding flags double-submit / race as the priority case.
- No CliTab interaction test — the Enter-swallow-submit bug (P1-3) would have surfaced.

## Related

- ADR-0009 (Redis writes by default with server-signal read-only) — every new write path conforms.
- ADR-0014 (specialized cell editors) — separately covers the data-grid editors, not these Redis ones.

Suggested labels: `feat:redis-tier2`, `feat:ux-palette`, `perf` (for the P1 pipelining work), `bug:p0` (for the two correctness fixes).
