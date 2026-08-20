# Plan 006: Staged mutation review in table and query results

> **Executor instructions**: Do not start until Plan 005 is `DONE` in
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
> rg -n '^\| \[005\].*DONE: [0-9a-f]+' plans/README.md
> git diff --stat <PLAN_005_COMPLETION_SHA>..HEAD -- src plans/README.md plans/mocks/result-mutations
> git status --short -- src plans/README.md plans/mocks/result-mutations
> ```
>
> Replace the placeholder with the SHA recorded by Plan 005. Expected on a
> fresh run: the prerequisite matches and no in-scope frontend files changed
> after that SHA. If resuming, follow "Resume protocol".

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: Plan 005 at its recorded completion SHA
- **Category**: direction
- **Planned at**: commit `ecefce8`, 2026-08-21
- **Gap**: `PAR-003` in `plans/parity-gap-register.md`

## Why this matters

Plan 005 lands a dark analysis/preview/apply backend. Today's frontend still
edits through two disconnected positional buffers: table tabs commit
row/column-index edits with no stored originals, no preview, and no per-change
control, while query tabs buffer edits that can never be committed at all —
the Save button has no handler, nulls are flattened to empty strings before
the grid, and nothing knows whether a result set is updatable. Inserts and
deletes are immediate one-shot commands that can never be reviewed. This plan
replaces both buffers with one typed Mutation Draft model, adds the staged
review surface — generated DML preview, per-change include/exclude/revert,
typed conflict attribution — wires query-result editing end to end behind
updatability analysis, extends editing to keyless tables through virtual keys
and guarded `ctid`, and keeps every legacy engine on its existing path.

## Required Plan 005 contract

Before work, confirm the implemented DTOs and ADR-0023 match these facts. A
mismatch is a STOP condition, not permission to change backend behavior here.

- Commands: `analyze_result_set`, `preview_result_mutations`,
  `apply_result_mutations`, `cancel_result_mutation`,
  `close_result_mutation_for_connection`, `load_virtual_key`,
  `save_virtual_key`, `clear_virtual_key`.
- Analysis sources are tagged `statement { sql }` and
  `relation { schema, table }`; results carry per-column origin and
  writability (`writable | generated | identityAlways | systemColumn`),
  per-table identity
  (`primaryKey | uniqueIndex | virtualKey | ctidFallback | none`),
  projection indexes, per-capability verdicts with typed reasons, an
  `analysisId`, and `statement: analyzed | notAnalyzable { reason }` —
  reasons include `possibleTempShadowing`, which renders honest copy about
  temp-table name collision, never a generic failure.
- `MutationPlan` ops are tagged `update | delete | insert` with identity,
  guards, and set/values as `{column, value}` pairs; guard strength by
  identity kind (edited-column guards for keyed updates, full
  projected-row guards for keyed deletes, full-row guards for `virtualKey`
  and `ctidFallback`) is enforced by the backend. Keyed updates do not
  detect concurrent changes to unedited columns; user-facing copy must not
  overclaim.
- Preview returns per-op `{ opIndex, sql, params }`; apply is one
  all-or-nothing transaction returning per-op `rowsAffected`, with typed
  errors `conflict { opIndex }`, `identityNotUnique { opIndex }`,
  `lockTimeout { opIndex }`, `analysisExpired`, `busy`, and the rest of the
  `ResultMutationError` union. Never parse message strings.
- Analysis requests supersede per tab; apply is exclusive, never queued,
  and never superseded; `cancel_result_mutation` cancels a tab's analysis,
  and a running apply is cancelable only by the tab that initiated it.
- Backend commands remain unused until this plan enables PostgreSQL through
  one local capability switch.

## Current frontend state

- `src/lib/store/relational-tables.ts:198` holds table edits as
  `Record<string, Record<number, Record<number, string>>>` — positional,
  original values never stored, read back from the loaded page at commit
  (`src/lib/store/edit-strategies.ts:248-301`).
- `src/lib/store/relational-queries.ts:61` holds `queryEdits` in the same
  positional shape keyed by tab id, with `setQueryEdit` and
  `discardQueryEdits` only — no commit action exists.
- `src/components/query-editor/toolbar.tsx:124-134` renders Discard wired to
  `onDiscardEdits` and a Save button with no `onClick`, no handler prop, and
  no disabled state.
- `src/components/query-editor/results-view.tsx:94-108` converts result sets
  to the legacy preview shape, flattening `null` cells to `""` — NULL is
  unrecoverable on the query-result editing path. `:254-264` renders
  `DataGrid` with editing enabled and no eligibility check at all.
- `src/lib/store/edit-strategies.ts:110-238` (`resolveEditContext`) gates
  editing: browse-backed sources use backend identity and reject `virtual`
  and `none` kinds via `identityIsEditable`
  (`src/lib/table-browse.ts:378-380`); legacy sources use the
  `pickRowIdentity` heuristic. `EditDataSource` (`edit-strategies.ts:100-108`)
  is the data-source seam Plan 004 introduced, built by
  `browseDataSourceFor` (`relational-tables.ts:120-147`).
- Inserts and deletes are immediate: `insertTableRow`
  (`relational-tables.ts:639-723`) and `deleteTableRows` (`:739`) invoke
  backend commands directly from the form and the selection; nothing is
  staged.
- `commitTableCellEdits` (`relational-tables.ts:513-624`) invokes the legacy
  `commit_cell_edits`, with ClickHouse queue polling and
  `refreshAfterWrite` (`:96-118`).
- `src/lib/store/table-browse.ts:288-297` refuses to apply a new browse
  result while `tableEdits` is non-empty because positional edits would
  reattach to different rows.
- The browse slice already exposes backend-authoritative
  `result.identity { kind, columns }` and `result.rowIdentity` per row
  (`src/lib/table-browse.ts:102-114`), consumed by
  `src/components/table-editor/use-table-session.ts:261, 374-395`.
- The `GRID_NULL_SENTINEL = "NULL"` bridge lives in
  `src/lib/table-browse.ts:10, 204-212`.
- House patterns: slice isolation with named owner actions via `get()`
  (`src/lib/store/README.md`, enforced by `pnpm check:slice-isolation`);
  I/O in dedicated lib modules with per-tab monotonic request ids
  (`src/lib/table-browse-client.ts`); typed error decoding
  (`src/lib/table-browse-error.ts`).
- UI must remain true black, white primary text, dense, minimal, and free of
  decorative chrome and continuously repainting animation.

## Decided frontend architecture

### Mutation Draft model

1. A new store slice `src/lib/store/mutation-drafts.ts` owns every staged
   change. Drafts are keyed by a **draft scope**: `table:<tabId>` for a
   browse-mode table tab, `query:<tabId>:<executionId>:<resultSetIndex>` for
   a query result set. One draft holds: the source target (`connectionId`
   plus relation or statement source), the current `analysisId` and analysis
   snapshot, an ordered map of **Draft Changes** keyed by a stable
   `changeId`, apply status, and a `generation` fenced exactly like the
   browse slice.
2. A Draft Change is typed, identity-keyed, and self-contained — never
   positional: `updateRow { table, identity: [{column, value}], cells:
   { column: { original, value } }, rowIndex }` (rowIndex is display-only),
   `deleteRow { table, identity, originals }`, `insertRow { table, values }`
   (duplicate-row is an `insertRow` prefilled from a source row minus
   generated and identity-always columns). Originals are captured at staging
   time from the grid row, including true NULLs. Each change carries
   `included: boolean` (default true); building the `MutationPlan` maps
   included changes in order to ops, so `opIndex` maps back to a `changeId`
   for typed error attribution.
3. `src/lib/result-mutation-client.ts` is the only owner of command
   invocation: per-tab monotonic request ids for analysis, silent
   supersession, typed result unions for analyze/preview/apply/cancel and
   virtual-key commands, mirroring `table-browse-client.ts`. `busy` and
   `analysisExpired` surface as typed states the slice handles. Expired
   analysis before review triggers exactly one transparent
   re-analyze-and-retry; expiry at apply time follows the review-integrity
   rule in item 11.
4. Draft lifecycle is explicit about every loss path. A draft is
   identity-keyed and self-contained, so a **budget release of its
   execution's rows never drops it**: the draft and the review panel keep
   functioning from staged originals after the result tombstones. Paths
   that do clear a draft with staged changes always confirm first:
   re-running the query (prompt before execution, following the
   connection-retarget precedent at `query-editor-panel.tsx:233-238`),
   closing the tab, tab retarget, and connection disconnect where
   interactive. `teardownConnectionWorkspace` and generation-fenced
   invalidation clear silently only when the connection itself is gone.
   Cross-slice cleanup follows the house rules: named owner actions on this
   slice via `get()`.

### Editability and analysis

5. `supportsResultMutations(engine)` is the only activation switch,
   PostgreSQL-only. Rollback means disabling it. Legacy engines keep the
   positional path and the legacy commands untouched, including their
   immediate insert/delete behavior.
6. Query results: the grid is read-only until analysis says otherwise.
   The first edit gesture on a result set (or an explicit Edit toggle,
   per the selected mock) triggers `analyze_result_set` with the executed
   single-statement SQL; `notAnalyzable` and per-table verdicts render
   honest copy on the grid and toolbar. Editing is enabled per column:
   a cell is editable when its origin table is updatable, its column is
   `writable`, and the table's identity is fully projected. Multi-statement
   executions and result sets past the first are read-only with the typed
   reason surfaced. Analysis runs at most once per result set and is
   invalidated with the draft on re-execution.
7. Table tabs (browse mode): `analyze_result_set` with the `relation`
   source runs lazily alongside the first edit gesture and enriches the
   existing browse identity with column writability — generated and
   identity-always cells render read-only with per-cell copy. Identity for
   staging comes from the browse result's `rowIdentity`, which maps grid
   rows to identity tuples without positional coupling.
8. Keyless editing: when analysis reports identity `none` or an identity
   that is not fully projected, the grid offers virtual-key selection
   (choose projected columns, persisted via `save_virtual_key`, with clear).
   Tables with `ctidFallback` identity become editable for update and
   delete — the slice stages full-row originals as guards exactly as the
   backend requires. The Plan 004 read-only copy for `virtual` identity is
   replaced by this flow.

### Staged review surface

9. The **review panel** is the single commit surface for both scopes,
   opened from the grid toolbar Save/Review control (exact layout per the
   selected mock). It lists every Draft Change grouped by kind and table
   with: changed-cell before/after values, include/exclude toggles,
   per-change revert, revert-all, and the generated DML from
   `preview_result_mutations` for the currently included set — statements
   and ordered parameters, display-only, with copy actions. Preview
   refreshes when inclusion changes. The panel **always displays the fully
   qualified resolved target relations** (`schema.table` per group) — this
   is the user-facing mitigation for runtime `search_path` drift the Plan
   005 contract documents — and its conflict copy states what the guards
   actually detect per identity kind, never a blanket "conflicts are
   detected" claim.
10. Commit invokes `apply_result_mutations` with the included changes.
    While an apply is in flight the draft is locked (no staging, no
    inclusion changes) and the panel shows a cancelable running state
    driving `cancel_result_mutation`. On success: report per-op affected
    counts, clear the draft, and refresh — browse tabs through
    `refreshTableBrowsesForRelation`, query tabs by marking the result
    stale with a re-run affordance (results are a point-in-time snapshot;
    they are not silently re-executed).
11. Typed failures attribute to changes: `conflict`, `identityNotUnique`,
    `lockTimeout`, and `database { opIndex }` highlight the offending
    change in the panel with its reason; the transaction rolled back, so
    every change stays staged and the user excludes, revises, or refreshes.
    `analysisExpired` at apply time re-analyzes once, rebuilds the plan,
    and proceeds **only if the regenerated statements and parameters are
    identical to the last previewed set**; any difference refreshes the
    panel and requires the user to re-review before apply — "every commit
    is reviewable first" survives cache expiry. `busy` reports the
    concurrent apply. No failure path ever silently drops a staged change.
12. Grid presentation of staged state: edited cells show a pending marker
    with original-on-hover, staged deletes render struck-through, staged
    inserts render as appended placeholder rows, and excluded changes
    render dimmed. A toolbar badge counts staged changes and opens the
    panel. Bulk staging: multi-row delete stages one `deleteRow` per
    selected row; bulk edit applies one value to a column across selected
    rows as individual `updateRow` changes; duplicate row stages a
    prefilled `insertRow` (opened in the add-row form per the selected
    mock).

### Migration off the positional buffers

13. Browse-mode table tabs move wholly onto the draft slice: cell edits,
    inserts (the add-row form stages instead of invoking `insert_row`),
    deletes, and duplicates all stage; `tableEdits` and
    `commitTableCellEdits` no longer run for browse mode. The
    `table-browse.ts:288-297` refresh guard is **removed, not converted to
    a prompt**: it existed because positional edits reattach to whatever
    rows load next, and identity-keyed changes don't. Every browse load —
    pagination, sort, filter, and explicit refresh — applies silently;
    staged changes rebind to loaded rows by identity, and changes whose
    identity no longer matches a loaded row render in the review panel as
    off-page, still committable. Rebinding for `ctidFallback` and
    `virtualKey` drafts additionally requires the loaded row's values to
    match the staged full-row originals — a vacuum-reused ctid must never
    visually attach a pending marker to an unrelated row; on mismatch the
    change renders off-page. Retargeting the tab to another relation
    prompts before clearing, per item 4.
14. Query tabs: `queryEdits`, `setQueryEdit`, and `discardQueryEdits` are
    replaced by the draft slice; the results-view boundary preserves
    `string | null` through the `GRID_NULL_SENTINEL` bridge instead of
    flattening to `""`. The toolbar Save control gains a real handler
    opening the review panel; Discard clears the draft with confirmation.
15. Legacy engines and legacy (non-browse) PostgreSQL paths keep
    `tableEdits`, `queryEdits` buffering (still uncommittable), and the
    immediate insert/delete commands, all behaviorally unchanged.
    `edit-strategies.ts` keeps serving them; browse-mode branches that
    become dead for PostgreSQL are removed only where provably unreachable,
    otherwise left untouched.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Focused store tests | `pnpm test -- src/lib/store/mutation-drafts.test.ts src/lib/store.test.ts` | pass |
| Focused UI tests | `pnpm test -- src/components/query-editor-panel.test.tsx src/components/table-editor-panel.test.tsx src/components/data-grid.test.tsx` | pass |
| Slice boundary | `pnpm check:slice-isolation` | exit 0 |
| Format | `pnpm format` | exit 0, no changes |
| Lint | `pnpm lint` | exit 0, no warnings |
| Types | `pnpm typecheck` | exit 0 |
| Frontend suite | `pnpm test` | all pass |
| Rust regression | `just test` | all non-ignored pass |
| Diff hygiene | `git diff --check` | no output |

## Scope

**In scope**:

- `plans/mocks/result-mutations/variant-a.html` (create)
- `plans/mocks/result-mutations/variant-b.html` (create)
- `plans/mocks/result-mutations/variant-c.html` (create)
- `plans/README.md` status and selected-mock note only
- `src/lib/result-mutation-client.ts` (create), tests
- `src/lib/result-mutation.ts` (create: TS protocol mirrors + error
  decoder), tests
- `src/lib/store/mutation-drafts.ts` (create), `mutation-drafts.test.ts`
  (create)
- `src/lib/store/types.ts`, `index.ts`, `README.md`
- `src/lib/store/relational-tables.ts`, `relational-queries.ts`
  (browse/query draft branches and cleanup wiring only)
- `src/lib/store/table-browse.ts` (refresh-guard change only),
  `table-browse.test.ts`
- `src/lib/store/workspace-tabs.ts`, `connections.ts`,
  `query-sessions.ts` (named cleanup wiring)
- `src/lib/query-session-budget.ts` (only if surfacing release events to
  the draft slice requires it; drafts must not block or pin the budget)
- `src/lib/store/edit-strategies.ts`, `edit-strategies.test.ts`
- `src/lib/table-browse.ts` (sentinel bridge reuse only)
- `src/lib/store.test.ts`
- `src/components/query-editor/results-view.tsx`, `toolbar.tsx`
- `src/components/query-editor-panel.tsx`, `query-editor-panel.test.tsx`
- `src/components/table-editor-panel.tsx`, `table-editor-panel.test.tsx`
- `src/components/table-editor/use-table-session.ts`, `body.tsx`,
  `use-row-selection.ts`, add-row form components
- `src/components/data-grid.tsx`, `data-grid.test.tsx`,
  `src/components/data-grid/toolbar.tsx`, `cell-editors.tsx`
- `src/components/mutation-review/` (create: panel + tests)

**Out of scope**:

- Any `src-tauri` change; return to Plan 005 review if its contract is
  incomplete
- MySQL, SQLite, ClickHouse, Redis, or legacy-path behavior changes
- Batch paste, deep value editors (JSON/XML/BLOB/image/spatial beyond the
  existing three), Quick Look, configurable copy formats
- Safe Mode, confirmation policy, environment classification (`PAR-004`)
- Re-executing queries automatically after apply; savepoints; applying
  inside an open manual transaction
- External mock hosting, commits, pushes, PRs without authorization

## Resume protocol

1. Read Plan 006 status and selected-mock note in `plans/README.md`.
2. Inspect scoped status/diff against the Plan 005 completion SHA.
3. Accept changes only through the last recorded completed step.
4. Unexplained or out-of-order changes are a STOP. Never discard user work.
5. Continue at the first incomplete step and update status after its gate.

## Git workflow

- Suggested branch: `feat/staged-mutation-review`, only if the operator
  asks.
- Do not commit unless authorized. If authorized, use a logical message
  such as `Stage and review result mutations in table and query grids`.
- Never publish mock files externally, push, or open a PR without
  instruction.

## Steps

### Step 1: Produce and select local static UI mocks

Before real TSX edits, create three materially distinct self-contained
variants under `plans/mocks/result-mutations/`. Each must show:

- a query result set entering edit mode after analysis, with per-column
  editability (expression and generated columns visibly read-only with
  reasons) and a `notAnalyzable` result set's honest copy
- staged cell edits with pending markers and original values, a staged
  delete, a staged insert, and a staged duplicate row
- the review panel: changes grouped by table, before/after values,
  include/exclude, per-change revert, generated DML with parameters and
  copy actions
- a conflict outcome attributed to one change after rollback, with
  exclude/revise/refresh recourse
- the virtual-key selection flow for a keyless table and the guarded
  `ctid` editing state
- the apply running state with cancel, and the success state with per-op
  counts and the stale-result re-run affordance on a query tab
- staged changes surviving browse pagination silently, with an off-page
  change shown in the panel, and the confirm-discard prompt on query re-run
  and tab close
- the toolbar staged-changes badge and the wired query-editor Save control

Serve locally from the repository, report localhost URLs, and verify wide
and narrow viewports, keyboard access, zero document overflow, and no
console errors.

STOP and wait for the operator to choose A, B, or C. Record the selection
in `plans/README.md`, stop the local server, and only then continue.

### Step 2: Build the protocol mirrors, client, and draft slice

Implement `result-mutation.ts` (TS mirrors of the Plan 005 protocol plus
typed error decoding), `result-mutation-client.ts` (request ids, analysis
supersession, typed unions, one-shot `analysisExpired` retry), and the
`mutation-drafts.ts` slice (scope-keyed drafts, identity-keyed changes with
originals, inclusion, plan building with `opIndex`→`changeId` mapping,
apply lifecycle lock, generation fencing, named cleanup actions). Wire
`closeTab`, retarget, disconnect, teardown, execution replacement, and
budget release to the named actions via `get()`.

Slice tests follow `table-browse.test.ts` conventions: staging captures
originals including NULL, inclusion mapping produces correct plans and
attribution, apply lock blocks staging, conflict keeps every change staged
with the offender marked, `analysisExpired` retries once pre-review and
enforces the identical-DML rule at apply, generation fencing rejects stale
applies, cleanup on every owner action, drafts surviving tombstone/budget
release, and the re-run confirmation path.

**Verify**: focused store tests, slice check, typecheck. Expected: all
pass.

### Step 3: Wire query-result editing end to end

Preserve `string | null` through the results-view boundary via the
sentinel bridge; gate editing behind lazy analysis with per-column
enablement and honest read-only copy; stage edits into the draft; wire the
toolbar Save control to the review panel and Discard to draft clearing
with confirmation. Multi-statement and non-first result sets stay
read-only with typed reasons. The capability switch still returns false
for rendering the new controls outside tests.

**Verify**: focused query-editor tests and typecheck. Tests cover null
round-trip (NULL never becomes `""` or the string `"NULL"` in a payload),
analysis-gated enablement per column, notAnalyzable copy, Save opening the
panel, and discard confirmation.

### Step 4: Migrate browse-mode table tabs onto drafts

Move browse-mode cell edits, add-row, delete, duplicate, and bulk edit
onto the draft slice keyed by `rowIdentity`; enrich writability via
`relation` analysis; implement virtual-key selection and guarded `ctid`
editing; remove the browse refresh guard in favor of silent identity
rebinding with off-page handling and full-row-match rebinding for
`ctidFallback`/`virtualKey` drafts. Legacy engines and non-browse paths
are untouched; `edit-strategies.ts` behavior for them is proven unchanged.

**Verify**: focused table-editor, browse-slice, and edit-strategy tests,
slice check, typecheck. Tests cover identity-keyed staging from
`rowIdentity`, staged insert/delete/duplicate, writability gating,
virtual-key persistence flow, full-row guard capture for `ctid` and
virtual keys, silent rebinding across page loads with off-page fallback,
full-row-match rebinding for ctid drafts, and byte-identical legacy
payloads.

### Step 5: Implement the review panel per the selected mock

Build `mutation-review/` exactly per the selected mock: grouped changes,
before/after, include/exclude with preview refresh, per-change and global
revert, DML display with copy, apply with running/cancel state, per-op
success reporting, typed failure attribution, and the query-tab
stale-result affordance. Status changes announce through a live region; a
console spy proves SQL, parameters, and row values never reach frontend
logging.

**Verify**: focused panel tests and typecheck. Expected: rendering,
inclusion/preview interaction, attribution highlighting, apply lifecycle,
and accessibility paths all pass.

### Step 6: Activate PostgreSQL and exercise the complete flow

Enable `supportsResultMutations` for the `"PostgreSQL"` engine literal
(matching `supportsServerTableBrowse`), then run the full gates and
exercise manually against the fixture (`pnpm db:postgres`):

- edit a single-table query result, review the DML, commit, and see the
  change in the table; NULL edits round-trip
- a join result edits only the identity-projected table; expression and
  generated columns are read-only with reasons
- a concurrent change between staging and commit produces an attributed
  conflict, full rollback, and preserved staging — for a keyed update on
  an edited column and for a keyed delete whose row changed
- a keyless table becomes editable after virtual-key selection; a
  non-unique virtual key aborts with `identityNotUnique`; `ctid` editing
  survives a row move (guards fail closed)
- staged insert, duplicate, multi-row delete, and bulk edit commit in one
  reviewed transaction; excluded changes do not run
- apply cancel rolls back; `busy` reports a concurrent apply; browse
  pagination and refresh silently rebind staged changes, off-page changes
  render honestly, and the panel shows fully qualified target relations
- re-running a query with staged changes prompts before executing; a
  budget release of another tab's results never drops a draft
- a temp table shadowing the queried name yields the
  `possibleTempShadowing` copy, not editing against the wrong relation
- multi-statement executions and second result sets are read-only with
  honest copy; legacy engines behave exactly as before

**Verify**: `pnpm format`, `pnpm lint`, `pnpm typecheck`, slice check,
full frontend tests, `just test`, and `git diff --check`. Expected: all
pass and only Scope files plus README status are modified.

## Test plan

- Slice tests carry staging, inclusion, plan mapping, attribution, locks,
  fencing, and cleanup; do not duplicate them through React.
- Component tests cover gating copy, staged-state rendering, panel
  interactions, virtual-key flow, prompts, and accessibility.
- Edit-strategy tests prove legacy payload semantics are byte-identical.
- A console spy proves no sensitive frontend logging.
- The manual fixture flow validates the Plan 005/006 boundary.

## Done criteria

- [ ] One identity-keyed Mutation Draft model serves browse-mode table tabs
      and query results; positional buffers no longer run for either on
      PostgreSQL.
- [ ] Query-result editing works end to end behind analysis: per-column
      enablement, honest read-only reasons, wired Save, committed DML.
- [ ] NULL survives every boundary; no payload ever carries `""` or the
      sentinel string for a true NULL.
- [ ] Every commit is reviewable first: generated DML with parameters,
      include/exclude, revert, and per-change conflict attribution after
      all-or-nothing rollback.
- [ ] Inserts, duplicates, deletes, and bulk edits stage and commit through
      the reviewed transaction; nothing mutates immediately in browse mode.
- [ ] Keyless tables edit through persisted virtual keys or guarded `ctid`;
      guard capture matches the backend contract.
- [ ] Drafts survive pagination, refresh, and result-budget release by
      identity rebinding; every path that clears staged changes on a live
      connection confirms first, and apply after analysis expiry never runs
      DML that differs from the reviewed preview.
- [ ] Legacy engines and non-browse paths are behaviorally unchanged.
- [ ] No sensitive frontend logging exists.
- [ ] The selected mock is recorded and implemented.
- [ ] All frontend, Rust regression, slice, format, lint, type, and diff
      gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; after an authorized
      commit, the reviewer/operator records `DONE: <completion SHA>`.

## STOP conditions

Stop and report if:

- Plan 005 is not DONE or its implemented contract differs from "Required
  Plan 005 contract".
- No mock is explicitly selected before TSX work.
- Originals (including NULL) cannot be captured faithfully at staging time,
  or identity-keyed staging cannot be mapped back from `opIndex`
  attribution.
- The draft model cannot preserve legacy positional behavior for
  non-activated paths without changing it.
- Any change would alter MySQL/SQLite/ClickHouse tabs or legacy PostgreSQL
  paths.
- A staged change can be silently dropped, or an apply can proceed with
  stale generation or a locked draft.
- A backend change is required; return to Plan 005 review.
- Any required verification fails twice or a change reaches outside Scope.

## Maintenance notes

- The capability switch is the rollback seam. Do not add automatic
  fallback to the positional buffers after a draft has staged changes.
- Batch paste, deep value editors, Quick Look, and configurable copy
  formats are the remaining `PAR-003` register items after this plan; they
  compose on top of the draft model (paste stages `updateRow`/`insertRow`
  changes) and should be planned as a follow-on once this surface is
  stable.
- `PAR-004` plugs in at two seams: the review panel is the generated-SQL
  review surface, and `apply_result_mutations` is the enforcement point —
  design nothing here that assumes UI-side enforcement.
- `PAR-005` should persist draft descriptors (scope, changes, analysis
  target) but never `analysisId`s, which are executor-lifetime handles.
- Once `PAR-001` follow-ons deliver per-statement script splitting,
  per-statement analysis lifts the single-statement editing restriction
  without frontend model changes.
