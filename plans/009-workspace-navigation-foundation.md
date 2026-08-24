# Plan 009: Workspace navigation foundation — connection organization, URI portability, and the Open Anything index

> **Executor instructions**: Plans 001–008 must be `DONE` in
> `plans/README.md` before starting. Follow this plan step by step. This is
> a **dark** plan: no user-visible UI change may land here — new fields,
> commands, and libraries ship unreferenced by any rendering path (adding
> optional serde/zod fields with defaults is invisible and allowed). Run
> every verification command and confirm the expected result before moving
> on. Update this plan's README row after each step and mark
> `READY FOR REVIEW` after all gates. A reviewer/operator records
> `DONE: <completion SHA>` after an authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git status --short -- src src-tauri plans/README.md
> git log --oneline -1
> ```
>
> Expected on a fresh run: clean tree at or after `9570f11`. A load-bearing
> mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: MEDIUM
- **Depends on**: Plans 001–008 complete
- **Category**: foundation (dark)
- **Planned at**: commit `9570f11`, 2026-08-24
- **Gap**: `PAR-005` in `plans/parity-gap-register.md`

## Review correction record

- **Step 3 amendment (2026-08-24):** credentials are not per-connection
  rows in all modes — they live in a mode-switched backend (OS keychain
  blob / encrypted SQLite / plain) keyed `connection_id → password`
  (`src-tauri/src/credentials.rs`). The duplicate therefore copies the
  secret through `credentials::read_all` + `credentials::upsert`
  backend-side (still never crossing IPC) instead of a raw row copy, and
  atomicity is sequenced (credential read up front; row insert; credential
  write; row rollback on credential-write failure) instead of a single SQL
  transaction.
- **Step 3 amendment (2026-08-24):** `duplicate_connection` returns the
  full `Vec<StoredConnection>` public list (matching `save_connection` /
  `delete_connection`) rather than the single new record, so the frontend
  keeps its existing list-replacement flow and backend name-ordering.
- **Adversarial review applied (2026-08-24):** an independent review of
  the delivered implementation produced fixes folded in before commit:
  the architecture in §2 below was rewritten to describe the delivered
  sequenced-with-rollback + credential-backend design (the original
  "row copy in one transaction" text predated the first amendment);
  the credential read-modify-write paths gained a serializing mutex
  (Duplicate racing Delete could previously erase the copy's secret via
  last-writer-wins whole-map rewrites); the false "locked store fails
  up front" comment was corrected for keychain mode and SQLite sources
  now skip the credential read; budget shedding now drops `caret` and
  `isDirty` alongside the shed SQL (an orphaned caret and a false
  unsaved-changes claim otherwise restore against an empty editor); the
  caret persistence test now actually asserts the isDirty invariant it
  documented; the ranker's frecency boost is capped below one scoring
  tier so frecency reorders within a tier but never overrides
  relevance; IPv6 hosts round-trip through brackets in the URI library;
  and every document stamp moved from the rewritten-away `2f4fd84` to
  main's `9570f11`. The review also corrected this plan's Current state
  section: the deleted-connection-drop and counter-bump restore tests
  already existed at the stamped commit, so Step 6 correctly added
  nothing for them.

## Why this matters

`PAR-005` asks for two things: durable workspace restoration and global
navigation. The restoration half was substantially delivered *after* the
register's audit by the UI-refresh series (Phase 8, commits `5d401c4` →
`9570f11`): the app SQLite carries a namespaced `ui.v1.*` store
(migration 16), and query/table tabs, tab order/pinning, the active tab,
hot-exit SQL, and expanded navigator nodes all restore across relaunch with
corrupt-blob fallback and connection-scoped GC. What the register still
correctly identifies as missing is the navigation half and a short tail of
restoration follow-ons:

- The command palette is the only global finder and it searches a hard-capped
  truncation (`MAX_TABLES = 300`) of *connected* connections' tables plus
  saved queries — no schemas, no views, no history, no open-tab switching,
  and caps are applied before relevance, so a matching table past position
  300 simply cannot be found.
- Connections have no folders, favorites, colors, duplicate, copy-URI, or
  URI import. The entire actions menu is Connect / Disconnect / Edit / Delete.
- Restored table tabs fire fetches against `Disconnected` connections and
  error; caret/selection is not part of the session blob.

This plan lands every non-visual foundation those features need — schema
migration and storage round-trip for organization fields, a backend
`duplicate_connection` command that never moves a secret across IPC, a pure
URI build/parse library, a pure Open Anything index/ranking library, the
session-blob caret extension, and the missing restore-orchestration tests —
so Plan 010 can activate the UI against tested contracts.

## Reconciliation against the register (read before scoping questions)

The register's `PAR-005` evidence lines predate the workbench consolidation
(commit `8411dbf` deleted `src/components/sidebar.tsx` and several
`workspace-overview/*` tabs; settings moved to
`src/components/settings-view.tsx`). Verified current state:

**Already delivered (do not re-plan):** `ui.v1.*` SQLite store with
debounce/retry/close-flush/512 KiB caps (`src/lib/ui-state.ts`,
`src-tauri/src/storage.rs:395-410`, migration 16); session blob with tabs +
hot-exit SQL + pins + active tab + expanded nodes
(`src/lib/session-persistence.ts:19-45`, budget shedding `:69-85`);
restore that validates field-by-field, drops tabs of deleted connections,
and bumps id/label counters (`src/lib/store/workspace-tabs.ts:302-380`);
skip-restore-and-persist when connections fail to load
(`src/components/app-shell.tsx:161-200`); no auto-connect by design
(`src/lib/store/connections.ts:360-370`).

**Deferred out of Plans 009/010, staying in the register:** SQL files /
recent files / external-change watch (needs fs+dialog plugins); split
editors, split results, multiple windows; OS-level deep links (single
`/` route today, no `tauri-plugin-deep-link`); encrypted profile exchange;
cross-connection metadata search of *disconnected* connections (needs a
persisted catalog snapshot) and data search; keyvalue tab restore; per-tab
(vs per-table) browse-state persistence; non-table object viewers — the
workbench consolidation removed the Phase 6 object-navigator UI, so
functions/sequences/types have no open surface; resurrecting object
viewers is `PAR-007` scope, not navigation scope.

## Current state (verified at `9570f11`)

### Backend

- `src-tauri/src/storage.rs:87-99` — `CREATE TABLE connections (id, name,
  database_name, engine, host, port, user_name, role, last_activity_at,
  use_https, url_path)`; later migrations added `read_only` (7) and
  `environment` / `safe_mode` (15, `:380-381`). Migration list
  `MIGRATIONS: &[(i64, &str)]` at `:77`; **the next free slot is 17.**
- Column list sites that must all change together: read at `:701`,
  row-decode at `:779-780` (the `Environment::from_str` pattern),
  insert at `:988`, upsert at `:1013-1014`, plus test literals
  (`:2155` and neighbors) — `grep -n "safe_mode" src-tauri/src` is the
  authoritative site list; every hit is a candidate.
- `credentials` table `:101-107` — secrets are keyed by `connection_id`
  and never leave the backend unencrypted except through the existing
  connect path. Duplicate-connection must copy this row backend-side.
- `ui_state` migration 16 at `:395-410`; namespace guard `:604-614`.

### Frontend

- `src/lib/store/types.ts:249-263` — `ConnectionCommon { id, name,
  database, host, port, user, password, role, environment?, safeMode?,
  lastActivityAt? }`. New optional fields land here once.
- `src/lib/store/types.ts:850-866` — `SchemaExplorer { name, tables,
  views?, materializedViews?, sequences?, foreignTables?, functions?,
  procedures?, aggregateFunctions?, types?, domains?, extensions?,
  eventTriggers?, roles?, tablespaces? }`. Populated per connection on
  successful connect only (`src/lib/store/connections.ts:478-497`).
- `src/lib/store/types.ts:868-879` — `SavedQuery { id, name, body,
  connectionId (nullable), isFavorite, … }`.
- `src/components/connection-form/form-utils.ts` — zod `connectionSchema`
  (`:24-60`), `EMPTY_NEW_DEFAULTS` (`:64-95`),
  `buildStoredConnectionFromForm` (`:276-293`, single construction site),
  `defaultValuesFromConnection` (`:308-392`).
- `src/components/command-palette/command-palette.tsx` — `MAX_TABLES =
  300` / `MAX_SAVED_QUERIES = 100` (`:41-42`); tables loop early-returns
  at the cap (`:153-179`); saved-query open hardcodes `schema: "public"`
  and falls back to `activeConnectionId ?? ""` (`:238-252`); dead
  `dbunk:open-command-palette` listener with no dispatcher (`:118-124`);
  frecency map persisted at `dbunk.palette.frecency`, capped 200, recent
  limit 6 (`:41-79`).
- `src/lib/shortcuts.ts:26-146` — `SHORTCUTS` registry;
  `hasShortcutHandler(id)` gates palette command rows to mounted surfaces.
- `src/lib/session-persistence.ts:19-45` — serialized tab fields are
  exactly `id, kind, label, connectionId, schema, table?, query?,
  pinned?, isDirty?`; budget shedding `:69-85`; pre-close hook `:107-112`.
- `src/lib/store/workspace-tabs.ts:302-380` — `restoreSession()`;
  `:359-368` counter bumps.
- `src/components/app-shell.tsx:161-200` — restore orchestration
  (`loadConnections` retried 3×, restore + persistence both skipped on
  failure). `src/components/app-shell.test.tsx` mocks `@/lib/ui-state` and
  stubs `loadConnections` but has **no test** for the skip-on-failure
  invariant or ordering.
- `src/components/connection-actions.tsx:36-49` — menu is Connect /
  Disconnect / Edit… / Delete… only.
- Existing test files to extend: `src/lib/session-persistence.test.ts`
  (restore rebuild, corrupt fallback, shedding, deleted-connection
  drop, and counter bumps are all already covered — the caret cases
  are the only genuinely new restore coverage), `src/lib/ui-state.test.ts`,
  `src-tauri/src/storage.rs:2502-2560` (ui_state round-trip tests — the
  in-file test pattern to copy for migration 17).

## Decided architecture

### 1. Connection organization fields (migration 17)

Three columns on `connections`, mirroring the migration-15 pattern exactly:

```sql
ALTER TABLE connections ADD COLUMN folder TEXT NOT NULL DEFAULT '';
ALTER TABLE connections ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN color TEXT NOT NULL DEFAULT '';
```

- `folder`: free single-level group name; empty string = ungrouped. No
  nesting in v1 (the register's "folders/groups" collapses to one level;
  nesting is a follow-on).
- `is_favorite`: plain bool.
- `color`: opaque TEXT backend-side; the frontend validates against a
  closed set so themes stay token-driven:
  `CONNECTION_COLORS = ["red","orange","amber","green","teal","blue","violet","pink","gray"]`
  (new `src/lib/connection-colors.ts`, exporting the list, a type, and an
  `isConnectionColor` guard; the token→CSS mapping is Plan 010's concern).
- Rust `StoredConnection` gains `folder: String`, `is_favorite: bool`,
  `color: String` with `#[serde(default)]` so legacy frontend payloads
  round-trip. Unknown/invalid color strings are stored as-is and treated
  as "no color" by the frontend guard — the backend does not validate
  presentation vocabulary.
- Frontend `ConnectionCommon` gains `folder?: string`,
  `isFavorite?: boolean`, `color?: ConnectionColor`. `connectionSchema`
  gains `folder: z.string().max(120).optional()`,
  `isFavorite: z.boolean().optional()`,
  `color: z.enum(CONNECTION_COLORS).optional()`;
  `EMPTY_NEW_DEFAULTS`, `buildStoredConnectionFromForm`, and
  `defaultValuesFromConnection` thread them through. No form UI here.

### 2. `duplicate_connection` (backend command, dark)

`duplicate_connection(connection_id) -> Vec<StoredConnection>` (the full
public list, matching `save_connection` / `delete_connection`, so the
frontend keeps its list-replacement flow and backend name-ordering):

- Copies the connection under a fresh UUID with name `"<name> copy"`
  (`"<name> copy 2"`, `"copy 3"`, … on collision against existing
  names), preserving every field including `folder` and `color` but
  resetting `is_favorite` and `last_activity_at` and blanking the
  in-memory password.
- The secret travels through the credential backend
  (`credentials::read_all` → `credentials::upsert`), never IPC.
  Credentials are a mode-switched whole-map store (OS keychain blob /
  encrypted SQLite / plain), not per-connection rows, so there is no
  single SQL transaction; instead the operation is **sequenced with
  rollback**: credential read up front (fails a locked encrypted store
  before any row is written; SQLite sources skip the read — they carry
  no credential), then row upsert, then credential upsert, and on a
  credential-write failure the fresh row is deleted so a copy never
  silently loses its secret. Keychain caveat: keychain read errors
  degrade to an empty map per that backend's documented failure
  policy, so a locked keychain duplicates the record without a
  credential rather than failing.
- The credential read-modify-write paths (`upsert` / `delete` /
  `delete_bastion_secrets`) are serialized by a module-level
  `tokio::sync::Mutex` — without it, two concurrent whole-map writers
  (e.g. Duplicate racing Delete of the source) could snapshot the map
  independently and the last `write_all` would erase the other
  caller's change.
- Does not copy `safety_overrides`, `virtual_keys`, `table_grid_prefs`,
  `schema_map_*`, or history — those are per-target state, and grid/ui
  state is keyed by connection id on the frontend.
- Refuses with a plain string error when the source id does not exist.

### 3. `src/lib/connection-uri.ts` (pure, tested)

- `buildConnectionUri(connection): { ok: true; uri: string } | { ok: false; reason: string }`
  - `postgres://user@host:port/database` for engine `postgres`;
    `mysql://…` for `mysql`; `redis://` (or `rediss://` when the
    connection's TLS flag is set) for `redis`.
  - **Never emits a password** — there is no include-secret option; the
    copy affordance is secret-free by contract.
  - Percent-encodes user, database, and path segments.
  - `sqlite` and `clickhouse` return `{ ok: false }` with a typed reason
    (no canonical URI / HTTP endpoint ambiguity); Plan 010 hides the
    action on refusal.
- `parseConnectionUri(uri): { ok: true; values: ParsedConnectionUri } | { ok: false; reason: string }`
  - Accepts `postgres://`, `postgresql://`, `mysql://`, `redis://`,
    `rediss://`. Extracts engine, user, optional password, host, port
    (engine default when absent: 5432 / 3306 / 6379), database (first
    path segment, percent-decoded; optional).
  - Query parameters are ignored in v1 and reported back as
    `ignoredParams: string[]` so the form can say so.
  - Anything else → `{ ok: false }` with a reason naming the accepted
    schemes. Parsing is hand-rolled on `URL` with scheme normalization —
    no new dependency.

### 4. `src/lib/open-anything.ts` (pure, tested)

A pure index + ranking module the palette consumes in Plan 010. No store
imports — everything arrives via an injected snapshot so tests are plain
fixtures.

```ts
type OpenAnythingTarget =
  | { type: "command"; commandId: string }
  | { type: "connect"; connectionId: string }
  | { type: "activate-tab"; tabId: string }
  | { type: "open-relation"; connectionId: string; schema: string;
      name: string; relationKind: "table" | "view" | "materialized-view" | "foreign-table" }
  | { type: "reveal-schema"; connectionId: string; schema: string }
  | { type: "open-saved-query"; savedQueryId: string }
  | { type: "open-history-entry"; historyId: string };

type OpenAnythingItem = {
  key: string;            // stable, doubles as the frecency key
  kind: "command" | "tab" | "connection" | "schema" | "relation"
      | "saved-query" | "history";
  label: string;
  description?: string;
  keywords: string;       // lowercase haystack the ranker scores
  target: OpenAnythingTarget;
};
```

- `buildOpenAnythingIndex(snapshot)` consumes `{ connections,
  activeConnectionId, schemaExplorer, savedQueries, queryHistory,
  workspaceTabs, commands }` and emits items for: every connection
  (including `Disconnected` — target `connect`), every schema of every
  *connected* connection (`reveal-schema`), every table / view /
  materialized view / foreign table (`open-relation`, kind-labelled),
  saved queries, the most recent 50 history entries, open query/table
  tabs (`activate-tab`), and available commands. Functions, sequences,
  and other `SchemaExplorer` kinds are deliberately not emitted — no
  open surface exists post-consolidation (see Reconciliation).
- `rankOpenAnythingItems(items, query, frecency)` — lowercase tokenized
  scoring per item: exact 400 / label-prefix 300 / word-boundary 200 /
  substring 100 / subsequence 40, summed across query tokens (every
  token must hit or the item is out), plus a frecency boost.
  **Caps are applied per kind after ranking** (connections 20,
  schemas 20, relations 200, saved queries 50, history 25, tabs and
  commands uncapped) and the function reports
  `truncated: Record<kind, number>` so the UI can disclose what was cut.
  Empty query → frecency-recent items (limit 6) + tabs + commands.
- `resolveSavedQueryTarget(saved, connections, activeConnectionId)` fixes
  the current defect: prefers `saved.connectionId` *only when that id
  still exists*, else falls back to `activeConnectionId` when set and
  extant, else returns a typed refusal (`{ ok: false; reason }`) instead
  of fabricating a tab with an empty connection id; the schema falls back
  to `"public"` explicitly at this single site.

### 5. Session blob v2: caret

- `QueryTab` (workspace-tabs types) gains optional
  `caret?: { line: number; column: number; anchorLine?: number; anchorColumn?: number }`
  plus a store action `updateQueryCaret(tabId, caret)` that no-ops when
  the tab is missing or non-query. The existing debounced session
  subscription picks the field up with **no version bump**: the
  serializer emits `caret` for query tabs and the restore validator
  accepts it only when all present members are finite positive integers
  (else the field is dropped, never the tab). Editor wiring is Plan 010.

## Commands you will need

```sh
pnpm format && pnpm lint && pnpm typecheck
pnpm vitest run src/lib/connection-uri.test.ts src/lib/open-anything.test.ts src/lib/session-persistence.test.ts src/components/app-shell.test.tsx
just fmt && just lint && just test
grep -n "safe_mode" src-tauri/src/storage.rs        # full column-site list
grep -rn "environment" src-tauri/src/types.rs        # StoredConnection field pattern
```

## Scope

Expected files touched (creation marked ＋):

- `src-tauri/src/storage.rs` — migration 17, column threading, duplicate
  helper, tests.
- `src-tauri/src/types.rs` — `StoredConnection` fields.
- `src-tauri/src/commands/connections.rs` — `duplicate_connection`
  (where `save_connection` lives; registered in `lib.rs` handler list).
- `src-tauri/src/credentials.rs` — credential-mutation serialization.
- ＋ `src/lib/connection-colors.ts`
- ＋ `src/lib/connection-uri.ts`, ＋ `src/lib/connection-uri.test.ts`
- ＋ `src/lib/open-anything.ts`, ＋ `src/lib/open-anything.test.ts`
- `src/lib/store/types.ts` — `ConnectionCommon` fields.
- `src/lib/engine-policy.ts` — `ConnectionFormValues` fields (type-only).
- `src/components/connection-form/form-utils.ts` — schema/defaults/build
  (+ its test's exact-equality fixtures gain the new common fields).
- `src/lib/store/workspace-tabs.ts` — `caret` field + action + restore
  validation.
- `src/lib/session-persistence.ts` — serialize `caret`.
- `src/lib/session-persistence.test.ts`, `src/components/app-shell.test.tsx`
  — new cases.
- `plans/README.md` — status row updates.

Out of scope (STOP if you find yourself editing them): any rendering
component, `command-palette.tsx`, `connection-actions.tsx`,
`connections-view.tsx`, navigator/workbench files, Monaco wiring.

## Resume protocol

Each step ends with all gates green. If interrupted, re-run the step's
verification commands; a red gate means the step is not done regardless of
what the README row says. Never mark a later step done while an earlier
step's gate is red.

## Git workflow

Work on the current branch in the working tree. **No commits, pushes, or
PRs without explicit operator authorization** (repo rule). The completion
SHA is recorded by the operator after review.

## Steps

### Step 1: Record contract decisions

Confirm against the working tree: migration slot 17 is free
(`grep -n "(\s*17," src-tauri/src/storage.rs` — expect no migration hit),
the `safe_mode` site list matches the excerpts, and
`src/lib/session-persistence.ts` serializes exactly the fields listed
above. Record deviations in this file under a `### Review correction
record` heading before proceeding. A migration numbered 17 already
existing is a STOP.

### Step 2: Migration 17 + StoredConnection round-trip

Land the SQL, the Rust struct fields (serde-defaulted), and thread every
column site found via the `safe_mode` grep. Add a storage test copying the
migration-15 round-trip pattern: insert legacy-shaped row → defaults come
back; save with folder/favorite/color → identical values return; upsert
preserves them. Gate: `just fmt && just lint && just test`.

### Step 3: `duplicate_connection`

Implement per the architecture (single transaction, credential row copied
backend-side, collision-suffixed name, favorites reset). Tests: duplicate
with credential (both rows copied, secret identical ciphertext), without
credential, name collision suffixing, missing source id refusal. Register
the command in the invoke handler list. Gate: `just fmt && just lint &&
just test`.

### Step 4: Frontend fields + URI library

`ConnectionCommon`/zod/defaults/build threading; `connection-colors.ts`;
`connection-uri.ts` + tests per the architecture (round-trips for all
three engines, percent-encoding both directions, engine default ports,
password captured on parse but never emitted on build, sqlite/clickhouse
refusals, `ignoredParams` reporting, junk-input refusals). Gate:
`pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run
src/lib/connection-uri.test.ts`.

### Step 5: Open Anything index

`open-anything.ts` + tests: fixture snapshot covering every emitted kind;
ranking order (exact > prefix > word-boundary > substring > subsequence);
multi-token AND semantics; caps applied after ranking (construct 250
relations where the best match sorts past the 200th input position and
assert it survives); truncation reporting; disconnected connection →
`connect` target; empty-query recents honoring frecency; all
`resolveSavedQueryTarget` branches including both defect cases (stale
`saved.connectionId`, no active connection → typed refusal). Gate:
`pnpm vitest run src/lib/open-anything.test.ts` plus the standard three.

### Step 6: Caret field + restoration tests

Store action, serializer, restore validation; new session-persistence
cases (caret round-trip; caret dropped when malformed while the tab
survives; **deleted-connection tab drop**; **counter bump past restored
ids** — add these two if truly absent, per the current-state note); new
app-shell case asserting `loadConnections → false` leaves both
`restoreSession` and `startSessionPersistence` uncalled. Gate: full suite
— `pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run` and
`just fmt && just lint && just test`.

## Test plan

Everything enumerated in Steps 2–6; nothing here requires a live database
or a running app. The storage tests use the in-file sqlite test harness
(`storage.rs:1964+` pattern).

## Done criteria

- Migration 17 applied cleanly on a legacy-shaped database in tests;
  organization fields round-trip through save/load/upsert.
- `duplicate_connection` copies record + credential transactionally with
  no secret over IPC.
- `connection-uri.ts` and `open-anything.ts` are pure, fully tested, and
  imported by nothing outside their tests (dark).
- Session blob round-trips `caret`; malformed caret degrades to no caret.
- The skip-restore-on-failure invariant and restore edge cases have tests.
- All six gates green; zero rendering diffs (`git diff --stat` shows no
  component file except the two test files listed in Scope).

## STOP conditions

- Migration slot 17 is taken, or the `connections` schema differs from the
  excerpt.
- Any step requires editing a rendering component to keep gates green.
- `StoredConnection` turns out not to be the wire type for
  `save_connection` (check `src-tauri/src/types.rs` first).
- The session serializer already carries a `caret` or conflicting field.
- Anything forces a secret across IPC to implement duplicate.

## Maintenance notes

- `open-anything.ts` deliberately excludes non-relation object kinds;
  when `PAR-007` restores object viewers, extend `OpenAnythingTarget`
  rather than overloading `open-relation`.
- The frecency store stays at `dbunk.palette.frecency` — item `key`
  values must remain stable across releases or recents silently reset.
- When SQL-file support lands (register follow-on), file entries join the
  index as a new kind; the ranking contract already tolerates new kinds.
- URI library v1 limitations (documented, deliberate): Redis path
  segments outside 0–15, extra path segments, and `#fragment` parts are
  dropped without an `ignoredParams`-style disclosure; the suite
  exercises WHATWG-URL non-special-scheme parsing under Node — the
  shipping runtime is WKWebView, so a smoke check belongs in Plan 010's
  manual pass.
- Concurrent duplicates of the same source can both mint the same
  "copy" name — names carry no unique constraint; harmless beyond
  cosmetics.
- The bastion-server save path (`commands/bastions.rs`) still does its
  credential read-modify-write outside the new serialization mutex —
  same pre-existing last-writer-wins exposure, out of this plan's blast
  radius; file with `PAR-006`.
