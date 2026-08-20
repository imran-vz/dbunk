# Plan 004: Activate server-backed browsing in PostgreSQL table tabs

> **Executor instructions**: Do not start until Plan 003 is `DONE` in
> `plans/README.md` with its completion SHA recorded. Follow each step and gate
> exactly. Step 1 is a human design checkpoint and must finish before real TSX
> changes. Update this plan's README status after each step. Stop on every STOP
> condition; do not improvise. Mark `READY FOR REVIEW` after all gates; a
> reviewer/operator records `DONE: <completion SHA>` after an authorized
> commit.
>
> **Prerequisite and drift check**:
>
> ```sh
> rg -n '^\| \[003\].*DONE: [0-9a-f]+' plans/README.md
> git diff --stat <PLAN_003_COMPLETION_SHA>..HEAD -- src plans/README.md plans/mocks/table-browse
> git status --short -- src plans/README.md plans/mocks/table-browse
> ```
>
> Replace the placeholder with the SHA recorded by Plan 003. Expected on a
> fresh run: the prerequisite matches and no in-scope frontend files changed
> after that SHA. If resuming, follow "Resume protocol".

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: MEDIUM
- **Depends on**: Plan 003 at its recorded completion SHA
- **Category**: direction
- **Planned at**: commit `26268ca`, 2026-08-19
- **Completed at**: commit `ecefce8`, 2026-08-21
- **Gap**: `PAR-002` in `plans/parity-gap-register.md`

## Why this matters

Plan 003 lands a dark, typed, cancelable PostgreSQL browse backend. Today's
grid silently discards filter operators and filters only the loaded page,
has no sorting at all, stacks an invisible client-side 50-row pagination on
the server's 100-row page, ships a page-size selector and Sort and Expand
buttons with no handlers, leaks filters across tables, and races rapid page
navigation with no stale-response rejection. This plan moves PostgreSQL table
tabs onto the browse contract with per-tab state, wires every dead control,
adds count and query-inspection UX, restores durable per-table grid
preferences, and keeps every other engine and the query-results grid on their
existing behavior.

## Required Plan 003 contract

Before work, confirm the implemented DTOs and ADR-0022 match these facts. A
mismatch is a STOP condition, not permission to change backend behavior here.

- Commands: `browse_table_data`, `cancel_table_browse`,
  `count_table_browse_rows`, `close_table_browse_for_tab`,
  `load_table_grid_prefs`, `save_table_grid_prefs`.
- Requests carry `tabId` and a frontend-monotonic `requestId`; a newer request
  for the same tab supersedes the older, which resolves with the typed error
  `superseded`.
- Typed filters AND-combine; raw SQL filter mode is a WHERE fragment; the
  operator set includes `isNull`/`isNotNull` and `inList`.
- Cells are `string | null`, not the `"NULL"` sentinel.
- `pageInfo` reports the mode that actually ran (`keyset` or `offset`),
  `hasMore`, and an opaque `nextCursor`; `invalidCursor` means re-request the
  first page.
- `count` is labeled `exact | estimated | unknown`; `browse_table_data` never
  counts implicitly.
- Responses carry `identity { kind, columns }` and `rowIdentity`, plus
  `inspection { sql, params }` and truncation counters.
- Errors are the `TableBrowseError` tagged union; never parse message strings.
- Backend commands remain unused until this plan enables PostgreSQL through
  one local capability switch.

## Current frontend state

- `src/lib/store/relational-tables.ts:775-851` invokes `load_table_data`,
  keyed by `connectionId::schema::table` (`types.ts:504-517`), with no request
  generation, no stale-response rejection, and no cancellation. Two tabs on
  the same table share one entry; `openTableSession` and
  `use-table-editor-data.ts:38-44` routinely fire duplicate loads.
- `src/components/data-grid.tsx:543-602` holds filters in component state,
  maps them into TanStack `columnFilters` at `:583-586` dropping the operator
  entirely, and prunes them only when a column name disappears (`:576-581`),
  so filters leak across tables with overlapping columns.
- `src/components/data-grid.tsx:672-690` stacks `getPaginationRowModel` with
  `initialState.pagination.pageSize = 50` on top of the server page. No
  `SortingState` or `getSortedRowModel` exists anywhere in `src/`.
- `src/components/data-grid/toolbar.tsx:98-111,155` defines twelve filter
  operators whose null variants are unreachable because apply requires a
  non-empty value. `:240-250` (Sort), `:506-517` (page-size Select), and
  `:518-525` (Expand grid) have no handlers.
- `src/components/table-editor/use-table-pagination.ts:41` derives page size
  from the last response with no setter; `:61-72` fires `loadTableData` and
  forgets it.
- `src/lib/store/query-sessions.ts` plus `src/lib/query-session-channel.ts`
  and `src/lib/query-session-budget.ts` are the house pattern: a tab-keyed
  slice whose I/O and pure reduction live in dedicated lib modules, with
  generation-stamped stale rejection (`query-session-budget.ts:341-348`).
- `src/lib/store/edit-strategies.ts:95-273` and
  `src/lib/table-session.ts:23-42` gate PostgreSQL table editing on
  `pickRowIdentity(structure)` and positional row indexes into the loaded
  page. PostgreSQL table editing works today and must not regress.
- Store slices cannot import one another; cross-slice cleanup goes through
  named owner actions via Zustand `get()` per `src/lib/store/README.md`.
- UI must remain true black, white primary text, dense, minimal, and free of
  decorative chrome and continuously repainting animation.

## Decided frontend architecture

### Browse state ownership

1. `src/lib/table-browse-client.ts` is the only owner of browse command
   invocation. It assigns the monotonic `requestId` per tab, tracks the
   in-flight request, exposes `browse`, `cancel`, `countExact`, and
   `closeForTab`, and resolves every call into a discriminated union result —
   never a thrown string. `superseded` results are swallowed silently; they
   are the expected outcome of rapid navigation.
2. A new store slice `src/lib/store/table-browse.ts` keys browse state by
   Workspace Tab `id`, mirroring `query-sessions.ts` structure: slice type
   first, a `patchBrowse(set, tabId, patch)` helper, all I/O through the lib
   module. State per tab: target (`connectionId`, `schema`, `table`), grid
   state (typed filters, raw filter text, filter mode, sort keys, page size,
   page/cursor), the last successful result, load status, count state,
   inspection payload, and a `generation` incremented on every
   connection-level invalidation.
3. Stale-response rejection is double-layered: the lib module drops responses
   whose `requestId` is older than the newest issued for that tab, and the
   slice drops applies whose `generation` no longer matches. Only the newest
   request may write result state. Every grid-state change (filter, sort,
   page size, page, refresh) issues exactly one new request and relies on
   backend supersession plus `cancel_table_browse` on tab close.
4. Reset semantics: changing filters or sort returns to the first page.
   Retargeting a tab to another table, schema change, and connection change
   clear filters, sort, cursor, selection, and pending edits for that tab and
   bump `generation`. Closing a tab calls `closeForTab` and drops its state.
   `teardownConnectionWorkspace` and `closeTab` invoke named owner actions on
   this slice exactly as they do for query sessions.
5. Retained browse results are bounded by construction (Plan 003 caps a
   response at 32 MiB and 1000 rows): the slice retains exactly one result
   per tab, replacing the previous on every successful response. No LRU or
   global budget is needed; do not build one.

### Compatibility and editing

6. `supportsServerTableBrowse(engine)` is the only activation switch,
   PostgreSQL-only. Rollback means disabling it. MySQL, SQLite, and
   ClickHouse table tabs, the query-results grid, and every non-browse
   `DataGrid` consumer keep current behavior unchanged.
7. `TableEditorPanel` branches once on the switch: browse mode reads from the
   new slice; legacy mode keeps `useTableSession`/`relational-tables` paths
   untouched. Null cells convert from `string | null` to display and to the
   legacy `"NULL"` sentinel exactly at the boundary into `DataGrid` and the
   edit payload builders, so `edit-strategies.ts` payload semantics are
   unchanged.
8. Editing in browse mode: `resolveEditContext` gains an explicit data-source
   input (columns, rows, identity columns) instead of reading only the legacy
   `tableData` record, fed from the browse slice with the
   backend-authoritative `identity.columns`. Positional selection, expansion,
   and pending edits are cleared whenever a new result replaces the rows;
   if pending edits exist, applying a new filter/sort/page prompts to discard
   or cancel first. Editing stays disabled when `identity.kind` is `virtual`
   or `none` (mutation through `ctid` is `PAR-003` work).

### Grid controls

9. Sorting: header click cycles asc → desc → none; shift-click appends a
   multi-sort key; the toolbar Sort button opens the sort list for reorder,
   direction, nulls placement, and removal. Sort chips show position for
   multi-sort. All sorting round-trips through the backend; TanStack sorting
   models stay unused in browse mode.
10. Typed filters: the existing filter bar becomes typed — column, operator
    from the Plan 003 set (null operators enabled with no value input,
    `inList` with a multi-value input), value. Chips show real semantics. A
    raw-mode toggle swaps the bar for a single WHERE-fragment input, visually
    distinct, with typed database errors (including `position`) surfaced
    inline. Typed filters and the raw fragment can coexist; the bar states
    they AND-combine.
11. In browse mode, TanStack `getFilteredRowModel` and `getPaginationRowModel`
    are disabled; the server page renders whole. Export in browse mode
    labels itself as exporting the current page (whole-relation export stays
    with the existing export tasks feature).
12. Page size: the Select wires to browse state with options
    10/25/50/100/250/500/1000, persists per table, and re-requests the first
    page. The footer and toolbar can no longer disagree because both read the
    same state.
13. Pagination: next/prev/first are always available; next uses keyset when
    the backend reports it, transparently. Last and numbered jumps are
    offered only when the count kind allows a page count and are labeled
    approximate when the count is estimated. `hasMore` drives next-button
    enablement when no count exists. `invalidCursor` silently reloads the
    first page.
14. Counts: the footer shows `~N rows (estimated)`, `N rows`, or `unknown`,
    with a `Count rows` action that invokes the exact count command, shows a
    cancelable pending state, and replaces the label on completion. No browse
    interaction ever triggers an exact count implicitly.
15. Query inspection: a toolbar action opens a read-only popover with the
    executed SQL and ordered parameters from `inspection`, with copy actions.
    Truncation counters render an honest partial-result notice.
16. Expand grid: the button toggles a maximized grid state that hides the
    row-details panel and subtab chrome, following the
    `use-row-details-visibility.ts` pattern, with Escape to restore and the
    existing fullscreen-overlay test conventions.

### Durable preferences, history, and presets

17. On first browse of a table, load `table_grid_prefs`; apply persisted page
    size, last sort, and last filters. Persist through `save_table_grid_prefs`
    debounced on grid-state changes. Prefs are per `(connection, schema,
    table)` and shared across tabs; live grid state stays per tab.
18. Filter/sort history (last 20 applied combinations per table) and named
    presets live inside the same prefs document. The filter bar offers
    history recall and save-as-preset; presets apply atomically (filters +
    sort + mode).

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Focused store tests | `pnpm test -- src/lib/store/table-browse.test.ts src/lib/store.test.ts` | pass |
| Focused UI tests | `pnpm test -- src/components/table-editor-panel.test.tsx src/components/data-grid.test.tsx` | pass |
| Slice boundary | `pnpm check:slice-isolation` | exit 0 |
| Format | `pnpm format` | exit 0, no changes |
| Lint | `pnpm lint` | exit 0, no warnings |
| Types | `pnpm typecheck` | exit 0 |
| Frontend suite | `pnpm test` | all pass |
| Rust regression | `just test` | all non-ignored pass |
| Diff hygiene | `git diff --check` | no output |

## Scope

**In scope**:

- `plans/mocks/table-browse/variant-a.html` (create)
- `plans/mocks/table-browse/variant-b.html` (create)
- `plans/mocks/table-browse/variant-c.html` (create)
- `plans/README.md` status and selected-mock note only
- `src/lib/table-browse-client.ts` (create)
- `src/lib/store/table-browse.ts` (create), `table-browse.test.ts` (create)
- `src/lib/store/types.ts`, `index.ts`, `README.md`
- `src/lib/store/relational-tables.ts` (browse-mode branch only)
- `src/lib/store/workspace-tabs.ts`, `connections.ts` (named cleanup wiring)
- `src/lib/store/edit-strategies.ts`, `edit-strategies.test.ts`
- `src/lib/table-session.ts`
- `src/lib/store.test.ts`
- `src/components/table-editor-panel.tsx`, `table-editor-panel.test.tsx`
- `src/components/table-editor/body.tsx`, `use-table-pagination.ts`,
  `use-table-editor-data.ts`, `use-row-selection.ts`,
  `use-row-details-visibility.ts`
- `src/components/data-grid.tsx`, `data-grid.test.tsx`
- `src/components/data-grid/toolbar.tsx`
- `src/components/table-editor/pagination.tsx` (or the footer's home in
  `body.tsx`)
- `src/components/workbench/relational-workbench.tsx`

**Out of scope**:

- Any `src-tauri` change; return to Plan 003 review if its contract is
  incomplete
- MySQL, SQLite, ClickHouse, Redis, or query-results-grid behavior changes
- Mutation through virtual identity, updatability analysis, DML preview
  (`PAR-003`)
- Whole-relation streaming export changes
- OR filter groups, cross-tab shared filter state, saved-query integration
- External mock hosting, commits, pushes, PRs without authorization

## Resume protocol

1. Read Plan 004 status and selected-mock note in `plans/README.md`.
2. Inspect scoped status/diff against the Plan 003 completion SHA.
3. Accept changes only through the last recorded completed step.
4. Unexplained or out-of-order changes are a STOP. Never discard user work.
5. Continue at the first incomplete step and update status after its gate.

## Git workflow

- Suggested branch: `feat/table-browse-grid`, only if the operator asks.
- Do not commit unless authorized. If authorized, use a logical message such
  as `Activate server-backed browsing in PostgreSQL table tabs`.
- Never publish mock files externally, push, or open a PR without instruction.

## Steps

### Step 1: Produce and select local static UI mocks

Before real TSX edits, create three materially distinct self-contained
variants under `plans/mocks/table-browse/`. Each must show:

- the typed filter bar with operator chips, null operators, and in-list input
- the raw WHERE mode toggle and an inline typed database error with position
- multi-column sort chips and the sort editor
- the wired page-size select and a footer whose counts show
  exact/estimated/unknown states plus the `Count rows` action
- keyset next-paging with no page count versus offset numbered paging
- the query-inspection popover with SQL, parameters, and truncation notice
- the pending-edits discard prompt on filter/sort change
- the expanded (maximized) grid state
- a superseded/loading state during rapid navigation that never flashes stale
  rows

Serve locally from the repository, report localhost URLs, and verify wide and
narrow viewports, keyboard access, zero document overflow, and no console
errors.

STOP and wait for the operator to choose A, B, or C. Record the selection in
`plans/README.md`, stop the local server, and only then continue.

### Step 2: Build the browse client and store slice

Implement `table-browse-client.ts` (request ids, in-flight tracking, typed
result unions, silent supersession) and the `table-browse.ts` slice
(tab-keyed state, `patchBrowse`, generation fencing, reset semantics, prefs
load/apply/save debounce, history and preset bookkeeping, named cleanup
actions). Wire `closeTab`, tab retarget, and `teardownConnectionWorkspace` to
the named actions via `get()`.

Slice tests follow `query-sessions.test.ts` conventions (module-scope mocks,
snapshot-and-restore store reset): newest-request-wins under interleaved
resolutions, superseded silence, generation rejection after
retarget/disconnect, reset-to-first-page on filter/sort/page-size change,
state cleared on retarget, prefs applied on first browse and persisted
debounced, history capped at 20, presets applying atomically, and
`invalidCursor` recovering to the first page.

**Verify**: focused store tests, slice check, typecheck. Expected: all pass.

### Step 3: Integrate the panel, editing gates, and data-source seam

Branch `TableEditorPanel` on `supportsServerTableBrowse` with the switch still
returning false. Browse mode reads rows, columns, identity, counts, and
inspection from the slice; legacy mode is untouched. Convert `string | null`
cells at the `DataGrid` and edit-payload boundaries. Give
`resolveEditContext` its explicit data-source input and feed
backend-authoritative identity in browse mode; editing disabled for `virtual`
and `none` identity kinds with honest copy. Clear selection, expansion, and
pending edits when rows are replaced; prompt before discarding pending edits
on filter/sort/page changes.

**Verify**: focused store and edit-strategy tests, slice check, typecheck.
Tests cover both panel modes, boundary null conversion, identity-kind gating,
edit-discard prompting, and no legacy-engine behavior change.

### Step 4: Implement the selected grid controls

Follow the selected mock exactly: typed filter bar, raw mode, sort
interactions and editor, wired page-size select, keyset/offset pagination
presentation, count states and `Count rows`, inspection popover, truncation
notice, expand-grid toggle with Escape, and history/preset menus. In browse
mode disable TanStack filtering and pagination models; keep client behavior
for every other `DataGrid` consumer. Status changes announce through a live
region; loading never flashes stale rows.

**Verify**: focused UI tests and typecheck. Expected: control wiring,
accessible names and keyboard paths, operator semantics including null
operators, raw-mode error display, count labeling, inspection copy actions,
and expand/restore all pass; `data-grid.test.tsx` legacy-mode tests still
pass unchanged.

### Step 5: Activate PostgreSQL and exercise the complete flow

Enable `supportsServerTableBrowse("postgres")`, then run the full gates and
exercise manually against the fixture (`pnpm db:postgres`):

- filters match the full relation, not the loaded page; operators behave per
  their labels; null operators work
- raw WHERE mode executes, surfaces a typed error with position for a bad
  fragment, and cannot write (verify a writing fragment fails)
- multi-column sort round-trips and pages correctly under keyset and offset
- rapid next-next-next never shows out-of-order pages; superseded requests
  are silent; cancel interrupts a slow raw filter
- page-size change persists per table across app restart
- a keyless fixture table browses stably, shows virtual identity, and is
  read-only with honest copy
- estimated count shows `~`, `Count rows` produces the exact value, and no
  interaction triggers an implicit exact count
- inspection shows the executed SQL and parameters matching the applied grid
  state
- pending edits prompt before a filter change discards them; legacy editing
  still works end to end on a keyed table
- MySQL/SQLite table tabs and the query-results grid behave exactly as
  before

**Verify**: `pnpm format`, `pnpm lint`, `pnpm typecheck`, slice check, full
frontend tests, `just test`, and `git diff --check`. Expected: all pass and
only Scope files plus README status are modified.

## Test plan

- Slice tests carry request fencing, reset, prefs, history, and preset logic;
  do not duplicate them through React.
- Component tests cover visible controls, disabled reasons, accessible
  names, count labels, inspection content, discard prompts, and expand state.
- Edit-strategy tests prove payload semantics are byte-identical for legacy
  mode and correct for browse-mode identity input.
- A console spy proves SQL, filter text, parameter values, and row values
  never reach frontend logging.
- The manual fixture flow validates the Plan 003/004 boundary.

## Done criteria

- [ ] PostgreSQL table tabs browse through the typed contract; every other
      engine and grid consumer is behaviorally unchanged.
- [ ] Browse state is tab-keyed with newest-request-wins and generation
      fencing; rapid navigation never renders stale rows.
- [ ] Filtering and sorting are server-side with true operator semantics,
      null operators, in-list, and a raw WHERE mode with typed errors.
- [ ] The client-side double-pagination layer and operator-dropping filter
      path are inert in browse mode.
- [ ] Page size, sort, filters, history, and presets persist per table and
      restore on next open.
- [ ] Counts are labeled exact/estimated/unknown; exact counts are explicit
      and cancelable.
- [ ] Inspection shows the executed SQL and parameters; truncation renders an
      honest partial notice.
- [ ] Keyless tables browse stably with virtual identity and read-only copy.
- [ ] Sort, page-size, and Expand-grid controls all function; expand restores
      with Escape.
- [ ] Editing on keyed tables is not regressed; pending edits are never
      silently discarded.
- [ ] Tab close, retarget, and disconnect cleanup is awaited and stale-safe.
- [ ] No sensitive frontend logging exists.
- [ ] The selected mock is recorded and implemented.
- [ ] All frontend, Rust regression, slice, format, lint, type, and diff
      gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; after an authorized commit,
      the reviewer/operator records `DONE: <completion SHA>`.

## STOP conditions

Stop and report if:

- Plan 003 is not DONE or its implemented contract differs from "Required
  Plan 003 contract".
- No mock is explicitly selected before TSX work.
- Newest-request-wins cannot be guaranteed for interleaved resolutions, or a
  stale response can overwrite newer grid state.
- Browse mode cannot preserve legacy editing semantics behind the data-source
  seam without changing legacy behavior.
- Any change would alter MySQL/SQLite/ClickHouse table tabs or the
  query-results grid.
- A backend change is required; return to Plan 003 review.
- Any required verification fails twice or a change reaches outside Scope.

## Maintenance notes

- The capability switch is the rollback seam. Do not add automatic fallback
  to `load_table_data` after a browse request has been accepted.
- Per-tab grid state versus per-table durable prefs is intentional: two tabs
  on one table may hold different live filters while sharing persisted
  defaults. `PAR-005` workspace restoration should persist the per-tab grid
  state as descriptors.
- `PAR-003` consumes `identity`/`rowIdentity` to re-key edits off positional
  indexes and to extend editing to query results and virtual identity; do not
  extend positional identity further in the meantime.
- Once `PAR-014` considers other engines, the browse contract — not the
  legacy `load_table_data` shape — is the surface to adapt per engine.
