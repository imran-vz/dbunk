# Plan 010: Open Anything activation, connection organization, and reconnect-safe restoration

> **Executor instructions**: Do not start until Plan 009 is `DONE` in
> `plans/README.md`. Follow this plan step by step. Step 1 ends in a STOP
> for operator mock selection — do not write any TSX before a mock is
> selected and recorded. Run every verification command and confirm the
> expected result before moving on. Update this plan's README row after
> each step and mark `READY FOR REVIEW` after all gates. A
> reviewer/operator records `DONE: <completion SHA>` after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat <PLAN_009_COMPLETION_SHA>..HEAD -- src plans/README.md plans/mocks/open-anything
> git status --short -- src plans/README.md plans/mocks/open-anything
> ```
>
> Expected on a fresh run: no `src` output. A load-bearing mismatch with
> the excerpts below is a STOP condition.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: MEDIUM
- **Depends on**: Plan 009 complete
- **Category**: direction
- **Planned at**: commit `9570f11`, 2026-08-24
- **Gap**: `PAR-005` in `plans/parity-gap-register.md`

## Why this matters

Plan 009 ships contracts nobody can reach: an Open Anything index no
surface renders, organization fields no form exposes, a duplicate command
no menu calls, and a caret field no editor writes. This plan activates all
of it — the palette becomes a real Open Anything over every navigable
target the app has, connections gain folders/favorites/colors/recency plus
Duplicate / Copy URI / Import-from-URI, restored tabs stop erroring
against disconnected connections, and the editor round-trips the caret.
It closes the `PAR-005` selected scope and ends with the documentation
truth pass the register (`PAR-017`) demands.

## Required Plan 009 contract

A mismatch is a STOP condition, not permission to re-implement here:

- `ConnectionCommon` carries optional `folder`, `isFavorite`,
  `color: ConnectionColor`; zod schema, `EMPTY_NEW_DEFAULTS`,
  `buildStoredConnectionFromForm`, `defaultValuesFromConnection` already
  thread them. `CONNECTION_COLORS` + `isConnectionColor` live in
  `src/lib/connection-colors.ts`.
- `duplicate_connection(connectionId) -> Vec<StoredConnection>` — the
  full public list, matching `save_connection` / `delete_connection`.
  Backend-side it copies the record + credential (sequenced with
  rollback; secret never crosses IPC), suffixes the name
  collision-safely, and resets favorite/activity. The frontend
  replaces its list from the return value (the same flow
  `saveConnection` uses) — it does NOT insert a single record.
- `src/lib/connection-uri.ts` — `buildConnectionUri` (secret-free,
  refuses sqlite/clickhouse) and `parseConnectionUri` (pg/mysql/redis,
  `ignoredParams` reporting).
- `src/lib/open-anything.ts` — `buildOpenAnythingIndex`,
  `rankOpenAnythingItems` (per-kind caps after ranking + `truncated`
  report; the frecency boost inside a non-empty query is capped at 50
  so it reorders within a relevance tier but never overrides one),
  `resolveSavedQueryTarget`, typed `OpenAnythingTarget` whose
  `open-relation` carries `relationKind` — the dispatch below must not
  discard it.
- `QueryTab.caret` + `updateQueryCaret(tabId, caret)`; the session blob
  round-trips it.

## Current frontend state (verified at `9570f11`)

- `src/components/command-palette/command-palette.tsx` — cmdk dialog,
  mounted once at `src/components/app-shell.tsx:290`; ⌘K toggle
  (`:99-110`); dead `dbunk:open-command-palette` listener (`:118-124`);
  `>` command-mode prefix (`:136-140`); frecency (`:41-79`, top 6
  "Recent"); item building `:142-263`; grouped render `:313-431`. Tests:
  `command-palette.test.tsx` (commands, `>` prefix, frecency only).
- `src/components/connections-view.tsx` — flat filtered list/grid
  (matcher `:34-42`, order-preserving filter `:104-107`, rows `:180-243`,
  `lastActivityAt` display `:381`, `:463`); rail variant
  `<ConnectionsView variant="rail">` used by the workbench.
- `src/components/connection-actions.tsx:36-49` — Connect / Disconnect /
  Edit… / Delete… menu; callbacks arrive as props.
- `src/components/connection-form.tsx` + `connection-form/*` — identity
  region and Advanced sections; safety fields precedent in
  `connection-form/safety-fields.tsx` (Plan 008) is the row pattern for a
  color picker / folder input / favorite toggle.
- `src/components/workbench/relational-workbench.tsx` — activity-rail
  workbench; rail persisted at `dbunk.workbench.rail` (`:66-73`); rails:
  connections, tables, queries, history, schema-map, admin, overview
  (`src/components/app-shell/activity-rail.tsx:45-53`).
- `src/components/workbench/database-navigator.tsx` — tables-only tree,
  local "Filter tables" input (`:225-238`), opens tabs via
  `openTableTab`.
- Reconnect gap: `src/components/table-editor/use-table-editor-data.ts:37-43`
  and `use-table-session.ts:342-356` fire loads on mount regardless of
  connection status; restored tabs against `Disconnected` connections
  error. `DisconnectedConnectionCard` /
  `NoConnectionCard`
  (`src/components/workspace-overview/disconnected-card.tsx`) are the
  existing "connect first" shells to reuse.
- Query editor: Monaco mount in `src/components/query-editor-panel.tsx`;
  hot-exit SQL already flows through `updateQuery`
  (`src/lib/store/relational-queries.ts:159-163`) — caret wiring follows
  the same debounced pattern.
- `src/lib/store/connections.ts` — `connectConnection` populates
  `schemaExplorer` on success (`:478-497`); `loadConnections` returns
  `boolean` (`:195-226`); delete GCs `ui.v1.grid.layout.<id>.` prefixes
  (`:330`).

## Decided frontend architecture

### Open Anything palette

1. The palette's item building (`:142-263`) is replaced by
   `buildOpenAnythingIndex` + `rankOpenAnythingItems` fed from a store
   snapshot; rendering stays cmdk but with **manual filtering disabled**
   (`shouldFilter={false}`) since ranking now happens in the library.
   **Render order for a non-empty query is the library's flat ranked
   order — no fixed group buckets.** With manual filtering, cmdk
   renders in DOM order and defaults selection to the first item, so
   grouping ranked results into fixed-order buckets would hand Enter
   to a weak match in an early group over an exact match in a late one.
   Kind identity renders inline per row (icon + kind badge) instead of
   group headers. The empty query keeps its shaped view (Recent, then
   open tabs and commands — the order the library already returns).
2. Target dispatch is one `runTarget(target)` switch (exhaustive
   type-narrowing, no default): `command` → existing shortcut runner;
   `connect` → `connectConnection`; `activate-tab` → `setActiveTab`
   (+ reveal); `open-relation` → **branch on `relationKind`**: `table`
   opens through the table-browse path, while `view` /
   `materialized-view` / `foreign-table` open through the existing
   `openViewTab` SELECT-query path — the app has never browsed a view
   through the table-tab contract, and this plan must not quietly start
   (promoting the view family into table browse is follow-on work with
   its own verification). Both relation branches and `reveal-schema`
   must first switch the active connection when the target's
   `connectionId` differs (the current palette's
   `useAppStore.setState({ activeConnectionId })` step — `openTableTab`
   / `openViewTab` take no connection id); `open-saved-query` →
   `resolveSavedQueryTarget` then `openWorkspaceTab` (typed refusal →
   toast, no broken tab); `open-history-entry` → `reopenHistoryEntry`.
3. `reveal-schema` needs plumbing that does not exist yet — the rail is
   local state in `relational-workbench.tsx:66-73` and the navigator
   filter is a local tables-only input, so "filter to the schema" is
   not implementable as stated anywhere today. Ship it as: set the
   active connection, add the schema to the store's `expandedSchemas`
   (existing mechanism), and reveal the tables rail via the existing
   `tabRevealRequest`-style signal — add a sibling store counter
   (`railRevealRequest` or equivalent) that `relational-workbench`
   subscribes to, mirroring `workspace-tabs.ts:44-51`. Do NOT write
   into the navigator's text filter.
4. Frecency: the storage key and stored shape (`{count, lastUsedAt}`
   map) are unchanged; item keys switch to the index's stable `key`
   field (old entries that no longer match simply age out). The
   palette converts the stored shape to the ranker's
   `ReadonlyMap<string, number>` with the existing decayed score
   (`count / (1 + ageDays)` as computed today at
   `command-palette.tsx:46,76-79`); the ranker caps the boost at 50
   internally, so the conversion needs no additional bounding.
5. Truncation disclosure: when `truncated` is non-empty, a muted footer
   row states what was cut ("214 more objects — keep typing"). No silent
   caps (register requirement).
6. The dead `dbunk:open-command-palette` listener gets a dispatcher or
   dies: the selected mock decides whether a header search affordance
   ships; if none does, delete the listener.
7. `>` command mode is preserved as-is.

### Connection organization

1. `connections-view.tsx`: group rows by `folder` (ungrouped last),
   favorites pinned first within groups, then most-recent
   `lastActivityAt` (the recents ordering the register asks for), then
   name. Color renders as a left stripe/chip via a token map in
   `connection-colors.ts` (CSS variables, no raw hex — UI-gate rule).
   The same treatment applies to the rail variant and the navigator's
   connection header where applicable per the selected mock.
2. `connection-form.tsx`: folder text input (datalist of existing
   folders), favorite toggle, color swatch row — placed per the selected
   mock; validation already exists from Plan 009.
3. `connection-actions.tsx` menu gains: **Duplicate** (invoke
   `duplicate_connection`, insert returned record into the store, toast),
   **Copy URI** (`buildConnectionUri`; hidden on `{ok:false}` engines;
   clipboard + toast noting the password is not included), and the
   existing items unchanged.
4. Connection form gains **Import from URI**: a paste field that runs
   `parseConnectionUri` and prefills engine/host/port/user/database
   (+password when present), listing `ignoredParams` when non-empty.
   Exact placement (new-connection entry point vs form header) per the
   selected mock.

### Reconnect-safe restoration + caret

1. `use-table-editor-data.ts` / `use-table-session.ts`: gate initial
   loads on the connection being `Connected`/`Read only`; a restored tab
   whose connection is disconnected renders the existing
   `DisconnectedConnectionCard` shell with a Connect action instead of
   firing a doomed fetch; a successful connect triggers the normal load
   path (effect keyed on connection status — no polling).
2. Query tabs: Monaco `onDidChangeCursorPosition`/selection listener,
   debounced ≥500 ms, calls `updateQueryCaret`; on mount, a stored caret
   restores via `setPosition`/`setSelection` + `revealPositionInCenter`
   guarded against out-of-range positions (clamp to model bounds).
3. No auto-connect anywhere — restoration stays descriptor-only
   (repo rule; `connections.ts:360-370` precedent).

### Documentation truth pass (PAR-017 discipline)

Update `plans/parity-gap-register.md` (PAR-005 progress block + stale
evidence paths), `plans/README.md`, and `ROADMAP.md` so no claim outruns
the implementation; record what Plans 009/010 deliberately deferred (list
in Plan 009's Reconciliation section).

## Commands you will need

```sh
pnpm format && pnpm lint && pnpm typecheck
pnpm vitest run
pnpm run check:ui-gates   # if wired; the UI-refresh grep gates (no raw hex, primitives)
just fmt && just lint && just test   # only if Rust files change (none expected)
```

## Scope

Expected files touched: `command-palette.tsx` (+test),
`connections-view.tsx` (+test if present), `connection-actions.tsx`,
`connection-form.tsx` + `connection-form/*` (organization fields, URI
import), `connection-colors.ts` (token map only), workbench navigator
+ `relational-workbench.tsx` + a store slice (the `reveal-schema`
rail-reveal signal + color/folder treatment per mock),
`use-table-editor-data.ts` / `use-table-session.ts` (+tests),
`query-editor-panel.tsx` (caret wiring), `plans/*`, `ROADMAP.md`.

Out of scope: new backend commands (Plan 009 shipped them all), SQL
files, split editors, deep links, keyvalue restore, object viewers
(`PAR-007`).

## Resume protocol

Each step ends with all gates green; re-run the step's verification on
resume. The selected mock is recorded in `plans/README.md` — if no
selection is recorded, you are still in Step 1 regardless of tree state.

## Git workflow

Working tree only; no commits/pushes/PRs without explicit operator
authorization.

## Steps

### Step 1: Produce and select local static UI mocks — STOP

Build three self-contained static HTML mocks under
`plans/mocks/open-anything/` (`mock-a.html`, `mock-b.html`,
`mock-c.html`), each showing: the Open Anything palette with mixed-kind
ranked results + truncation footer, the connections list with
folders/favorites/colors/recency, the actions menu with
Duplicate/Copy URI, the URI-import placement, and the disconnected
restored-tab shell. Every mock's palette shows a **flat ranked result
list with inline kind badges** (fixed group buckets are ruled out by
the architecture — selection must follow rank); differentiate along
the remaining real axes (e.g. A: folder headers in the connections
list + badge-forward palette rows; B: sidebar folder tree + two-line
palette rows; C: kind filter chips above the palette + color-forward
connection rows).
**STOP: operator selects a mock; record the selection in
`plans/README.md` before any TSX.**

### Step 2: Palette → Open Anything

Rewire per architecture; delete or wire the dead listener per mock;
extend `command-palette.test.tsx`: mixed-kind results from a fixture
store rendered in flat ranked order with the top-ranked item as the
default selection (this is what the flat-list decision exists to
guarantee — assert Enter targets the best match, not the first group),
view/matview rows open through `openViewTab` while tables open through
the browse path, cross-connection opens switch the active connection
first, truncation footer, saved-query refusal → toast (no tab),
disconnected connection row connects, `>` mode unchanged, frecency
still ranks Recent. Gates: the standard three + vitest.

### Step 3: Connection organization UI

Grouping/pinning/recency/color + form fields per mock; update/extend
connections-view tests (ordering: favorites → recency → name inside
folder groups; ungrouped last). Gates as above.

### Step 4: Duplicate, Copy URI, Import from URI

Menu action invokes `duplicate_connection` and replaces the store's
connection list from the returned `Vec<StoredConnection>` (the same
list-replacement flow `saveConnection` uses — no single-record
insert), then toasts naming the new copy; Copy URI via
`buildConnectionUri` with the secret-free note in the toast; URI
import prefill incl. `ignoredParams` disclosure (an imported password
lands in the form's password field only — normal save flow stores
it); tests for the list-replacement path and the parse-prefill
mapping. Gates as above.

### Step 5: Reconnect-safe tabs + caret

Load gating + disconnected shell + connect-triggers-load effect; caret
capture/restore with clamping; tests: restored table tab with
disconnected connection renders shell and fires zero fetches; connect →
single load; caret round-trip and out-of-range clamp. Gates as above.

### Step 6: End-to-end pass, mocks removal, documentation truth pass

Manual pass with the dev app (`pnpm tauri dev`) across: relaunch-restore
with a disconnected connection, palette reach into every kind, duplicate
+ copy URI + import URI, folder/favorite/color round-trip. Delete
`plans/mocks/open-anything/`. Execute the documentation truth pass.
Full gates: `pnpm format && pnpm lint && pnpm typecheck && pnpm vitest
run` (+ `just` suite if any Rust file changed). Mark `READY FOR REVIEW`.

## Test plan

Steps 2–5 enumerate the automated coverage; Step 6 is the manual pass.
No live database is required for the automated suite.

## Done criteria

- Every navigable target kind is reachable from ⌘K with ranked results
  and disclosed truncation; no silent caps remain.
- Connections support folder grouping, favorites, colors, recency
  ordering, duplicate, secret-free copy-URI, and URI import.
- Restored tabs never fire loads against disconnected connections and
  self-load on connect; caret survives relaunch.
- The saved-query empty-connectionId defect and the dead-listener defect
  are gone.
- Register/README/ROADMAP match reality; deferred items are recorded.
- All gates green.

## STOP conditions

- No recorded mock selection.
- Plan 009 contract mismatch.
- Any need for a new Tauri command or migration (belongs in a 009
  amendment, not here).
- The UI gates (`check-ui-gates`) require raw hex or non-token colors to
  implement the selected mock — re-cut the mock instead.

## Maintenance notes

- `runTarget` is the single dispatch point — new index kinds must extend
  it exhaustively (type-narrowing switch, no default).
- Folder names are free text; a rename affordance is a follow-on — until
  then renames are per-connection edits.
- Caret restore clamps rather than validates against stale SQL — hot-exit
  SQL and caret are captured by the same debounce so drift is bounded.
