# Plan 017: Table designer, routine editor, and table security activation

> **Executor instructions**: Do not start until Plan 016 is `DONE` in
> `plans/README.md`. Follow this plan step by step. Step 0 is a mock
> step: produce the three layout variants, stop, and wait for the
> operator to select one before writing component code. Run every
> verification command and confirm the expected result before moving
> on. Update this plan's README row after each step and mark `READY FOR
> REVIEW` after all gates. A reviewer/operator records `DONE:
> <completion SHA> (selected mock: X)` after an authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat 6b573f1..HEAD -- src src-tauri plans/README.md
> git status --short -- src src-tauri plans/README.md
> grep -n "generateGrant\|generateRls\|generateTrigger" src/components/table-editor/specialized-editors.tsx   # expect the three generate-only panels
> grep -n "type CreateKind" src/components/object-ddl/create-object-dialogs.tsx                                # expect schema|sequence|enum|view|materialized-view
> grep -n "op: \"createTable\"\|op: \"createFunction\"\|op: \"grantPrivileges\"" src/lib/store/types.ts       # expect the Plan 016 TS mirrors
> ```
>
> Expected on a fresh run: no `src` or `src-tauri` output from the first
> two commands. A load-bearing mismatch with the excerpts below is a STOP
> condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MEDIUM (every new surface feeds the existing reviewed gate,
  so the blast radius is a wrong op — visible in the preview — rather
  than wrong SQL; the residual risk is a stale structure or object
  description after apply, and a routine editor that silently drops a
  header attribute on round-trip)
- **Depends on**: Plan 016 complete
- **Category**: direction
- **Planned at**: commit `b82de63`, 2026-09-01
- **Gap**: `PAR-007` in `plans/parity-gap-register.md`

## Why this matters

Plan 016 makes the backend able to create tables, functions,
procedures, triggers, policies, and privileges through the typed
workflow, but nothing in the UI emits those operations. This plan
activates them at the three places users already look: the navigator's
create menu (tables, functions, procedures), the Object Viewer for a
routine (edit source), and the table Structure tab (triggers, row-level
security, privileges), and it retires the last three generate-only
panels in the Specialized tab. After this plan, every PostgreSQL
lifecycle action dbunk offers on a table or routine runs through
preview → review → gated apply, and the register's `ROADMAP.md` claims
about GRANT/RLS/trigger "editors" become true.

## Required Plan 016 contract

A mismatch is a STOP condition:

- TS mirrors in `src/lib/store/types.ts` for `createTable`,
  `createFunction`, `createProcedure`, `createTrigger`, `dropTrigger`,
  `setTriggerEnabled`, `setRowLevelSecurity`, `createPolicy`,
  `dropPolicy`, `grantPrivileges`, `revokePrivileges`; `PgIdentity`,
  `PgGrantee`, `PgPrivilege`, `PgVolatility`, `PgParallelSafety`,
  `PgTriggerTiming` / `PgTriggerEvent` / `PgTriggerLevel` /
  `PgTriggerMode`, `PgPolicyCommand`.
- `PgObjectFacts` routine arm with `body`, `strict`, `securityDefiner`,
  `parallel`; `TableStructure` with `triggers`, `policies`,
  `privileges`, `rowSecurity` and the three new capability flags.
- Plan 016's destructiveness rule (revoke / disable trigger / disable
  RLS / drop policy are destructive) — the preview carries it, the UI
  never re-derives it.

## Current frontend state (verified at `b82de63`; re-verify after 016)

- Navigator create menu: `database-navigator.tsx:377-425` renders a
  per-group "New …" item for `CreateKind` groups and `:683-684` the
  schema button; `object-ddl/create-object-dialogs.tsx:95-131` owns
  `CreateKind`, `TITLES`, and one `CreateObjectDialog` that builds ops
  and hands them to the `DdlReviewDialog` (`ddl-review-dialog.tsx:48-74`,
  `variant: "dialog" | "inline"`, `onApplied`, `onRefresh`).
- Object Viewer: `object-viewer/object-viewer.tsx` — comment editing
  (`:90-128`, `:409-475`), `ObjectActionKind` dialogs (`comment | rename
  | add-enum-value | rename-enum-value | alter-sequence`,
  `object-action-dialog.tsx:17-22`), Drop (`:664-666`), Refresh
  (`:648`); the definition tab shows `definitionSql` read-only.
- Structure tab: `table-structure/read-only-sections.tsx` renders
  PK/FK/index/constraint sections with Plan 015's inline add/drop
  forms over `PgStructureOps { schema, table, hasPrimaryKey, queueOp }`
  (`shared.tsx:12-17`); `use-structure.ts` owns the async preview,
  reviewed gate, and identity checks; `pending-changes-section.tsx`
  renders `DdlPlanPreviewGroups`; `relational-tables.ts:1643`
  `commitStructureChanges` applies with the DDL lock and epoch fence.
- Specialized tab: `specialized-editors.tsx` — `queueTypedOp` for the
  index/FK panels (`:96-113`, PostgreSQL only via `typedOpsAvailable`
  `:76`), `grantState` (`:114-119`), `rlsState` (`:121-129`),
  `triggerState` (`:155-163`), and the three generators
  (`generateGrant :197-201`, `generateRls :203-225`, `generateTrigger
  :310-329`); the trigger panel creates the function inline.
- Shared op builders live in `src/lib/structure-changes.ts`
  (`buildCreateIndexOp :103`, `buildAddForeignKeyOp :129`,
  `PG_REFERENTIAL_ACTIONS :77`).
- Catalog roles are list-only `PgCatalogEntry` names on the loaded
  catalog (`pg-objects` slice) — enough for a grantee/role picker.

## Decided architecture

### 0. Mock step (table designer only)

The designer is the one surface without an existing home. Produce
three static mocks in the scratchpad and wait for selection:

- **A — Designer tab**: a new `table-designer` Workspace Tab kind with
  a columns grid, constraint sub-sections, and a right-hand live
  preview (the `DdlPlanPreviewGroups` component) that reloads on every
  edit; "Create" opens the review dialog.
- **B — Large dialog**: a `CreateObjectDialog`-style `lg` dialog with
  a columns list, collapsible constraint groups, and the preview
  inline below (`variant: "inline"`), like the existing view dialog.
- **C — Empty structure editor**: "New table" opens a table tab in a
  `designing` state that reuses the Structure tab's column and
  constraint sections against an unsaved table; Commit emits
  `createTable` instead of `ALTER`s.

The routine editor, structure sections, and panel switch have
established patterns and get no mock.

### 1. Table designer

Whichever mock is selected, the designer is a pure function of its form
state to `PgObjectOp[]`: one `createTable` followed by `setComment` ops
for the table and any commented columns, then `createIndex` ops for
extra indexes (the user may tick `concurrently`; the preview shows the
resulting standalone group). `src/lib/table-designer.ts` owns
`buildTableDesignerOps` and the form-state validation that mirrors Plan
016's rules so the user sees a field error before a preview
`invalidOp` does; the preview remains authoritative. Column defaults
use the Plan 015 literal/expression selector; identity columns use a
`none | always | by default` selector that disables default and
nullable. On apply success: refresh the catalog, open the new table's
table tab, and close the designer. Entry points: the navigator Tables
group "New table" and the ⌘K command "New table in <schema>".

### 2. Routine editor

One `RoutineEditorDialog` (`object-ddl/routine-editor-dialog.tsx`)
serves create and edit. Fields: schema (fixed when editing), name,
arguments (text), returns (text; hidden for procedures), language
(select over the connection's `pg_language` names is **not** loaded —
plain text with `plpgsql` / `sql` suggestions), body (Monaco, SQL mode,
the same editor primitive the query editor uses), volatility, strict,
security definer, parallel. Editing prefills every field from the
routine facts and forces `orReplace: true`; the dialog shows a
one-line note that PostgreSQL refuses signature and return-type
changes under `OR REPLACE` and that those need drop + create (the Drop
action exists). C/internal routines open read-only. The dialog builds
one `createFunction` / `createProcedure` op and hands it to the
`DdlReviewDialog`. On apply success in edit mode: reload the
description; if the edited signature no longer resolves, the viewer
shows its existing not-found state rather than guessing the new ref.
Entry points: the Object Viewer "Edit source" action for functions and
procedures (aggregates: none), and the navigator Functions /
Procedures groups' "New …" items.

### 3. Structure tab sections (PostgreSQL)

Three sections join `read-only-sections.tsx` behind the new
capability flags, using the existing `Section` / `InlineForm` /
`InlineToggle` pattern and queueing into the shared pending list via
`queueOp`:

- **Triggers** — rows show name, timing, events, level, function, and
  enabled state; per-row Enable/Disable (queues `setTriggerEnabled`)
  and Drop (`dropTrigger`, cascade offered); "New trigger" inline form
  with a function picker over the catalog's functions returning
  `trigger` when known, else free text, plus a "create function
  inline" toggle that queues a `createFunction { returns: "trigger" }`
  op before the `createTrigger`.
- **Row-level security** — an enabled/forced toggle pair (queues one
  `setRowLevelSecurity`), policy rows (name, permissive, command,
  roles, using, with check) with Drop (`dropPolicy`) and Edit (queues
  `dropPolicy` + `createPolicy`), and "New policy" with a role picker
  over catalog roles plus `PUBLIC`.
- **Privileges** — rows grouped by grantee with per-privilege badges
  and grantable markers; per-row Revoke (queues `revokePrivileges`
  with a `cascade` toggle) and "Grant" with grantee picker, privilege
  checklist scoped to what a table may carry, and `WITH GRANT OPTION`.

Pending rows keep taking summary and destructiveness from the preview
(Plan 015 rule). The refresh fan-out after apply already reloads the
structure, so the new sections update without new plumbing.

### 4. Specialized panels

`generateGrant`, `generateRls`, and `generateTrigger` are replaced by
`queueTypedOp` calls building the same ops as §3 (grant →
`grantPrivileges` with `target` = this table's ref; RLS →
`setRowLevelSecurity` + `createPolicy`; trigger → `createFunction` +
`createTrigger`). Non-PostgreSQL engines keep the generate-SQL
behaviour verbatim as Plan 015 did for index/FK. The shared builders
move to `src/lib/structure-changes.ts` (`buildGrantOp`,
`buildRevokeOp`, `buildPolicyOps`, `buildTriggerOps`) so the panels and
the structure sections cannot drift. With the last string generator
gone, `pgQuoteIdent` / `quoteIdentOrPublic` / `quoteLiteral` imports
leave `specialized-editors.tsx` unless the cell editors still need
them; the header note on `src/lib/ddl/postgres.ts` is updated to say
what still calls it.

### 5. Documentation and truth pass

- `ROADMAP.md` §3: "Extensions (install/drop UI not wired)" stays;
  §8 lines 107, 108, 111 change from "generates DDL, opens in SQL
  editor" to the typed reviewed flow; add "Create table designer" and
  "Function / procedure editor" rows.
- `CONTEXT.md`: **Table Designer**, **Routine Editor** entries; the
  **Object Viewer** entry mentions Edit source.
- ADR-0014 pointer (specialized panels now all queue typed ops or edit
  cells).
- Register `PAR-007`: progress paragraph for Plans 016/017; remove
  "create-table designer" and "structured function, procedure …
  create/alter flows; triggers" and "row-level security policies" and
  "grants" from the missing list, leaving the deferred kinds named in
  Plan 016's Reconciliation section.

## Commands you will need

```sh
pnpm format && pnpm lint && pnpm typecheck
pnpm vitest run
pnpm run check:ui-gates && pnpm run check:slice-isolation
pnpm db:postgres && pnpm tauri dev        # Step 5 manual pass
grep -n "generateGrant\|generateRls\|generateTrigger" src/components/table-editor/specialized-editors.tsx   # empty after Step 4
```

## Scope

Expected files touched: `src/lib/table-designer.ts` (+test),
`src/lib/structure-changes.ts` (+test), `object-ddl/`
(`create-object-dialogs.tsx` or a new designer component per the
selected mock, `routine-editor-dialog.tsx`, `object-action-dialog.tsx`,
`index.ts`, tests), `object-viewer/object-viewer.tsx` (+test),
`workbench/database-navigator.tsx` (+test), `table-structure/`
(`read-only-sections.tsx`, `shared.tsx`, tests),
`table-editor/specialized-editors.tsx` (+test), `src/lib/open-anything.ts`
(the "New table" command only), `src/lib/store/workspace-tabs.ts` only
if mock A is selected (new tab kind + session persistence entry),
`ROADMAP.md`, `CONTEXT.md`, ADR-0014, the register, `plans/README.md`.

Out of scope: any backend change (Plan 016 amendment); aggregates;
database lifecycle; roles/ownership/default privileges; everything in
Plan 016's Reconciliation section; ClickHouse behaviour.

## Resume protocol

Each step ends with all gates green; re-run the step's verification on
resume. The selected mock is recorded in the README row and never
re-decided mid-plan.

## Git workflow

Working tree only; no commits/pushes/PRs without explicit operator
authorization.

## Steps

### Step 0: Designer mocks

Produce mocks A, B, C per §0 as static HTML in the scratchpad (one file
each, same fixture table), report their paths, and STOP for selection.

### Step 1: Shared builders and table designer

`table-designer.ts`, `structure-changes.ts` builders, the designer per
the selected mock, navigator/palette entry points. Tests: the
designer's form → ops mapping (comments and indexes appended in order,
identity disables default/nullable, concurrent index flagged); local
validation messages; apply success opens the table tab and refreshes
the catalog; a preview `invalidOp` is shown against its field when the
op index maps to one. Gates: standard three + vitest + both checks.

### Step 2: Routine editor

§2. Tests: create and edit modes build the expected op with every
header attribute round-tripped from facts; procedures hide `returns`;
C/internal routines are read-only; `orReplace` forced in edit mode;
apply success reloads the description; the not-found path when the
signature changed. Gates as Step 1.

### Step 3: Structure sections

§3. Tests: each section renders from the new structure fields and is
hidden when its capability flag is `false`; every affordance queues the
expected op(s) in order; the policy Edit queues drop + create; grant
checklist is scoped to relation privileges; pending rows still take
their label from the preview. Gates as Step 1.

### Step 4: Specialized panel switch

§4. Tests: the three panels queue ops on PostgreSQL and generate SQL on
ClickHouse (verbatim strings pinned); the trigger panel queues function
+ trigger; the grep in "Commands" is empty. Gates as Step 1.

### Step 5: End-to-end pass and truth pass

Manual pass with `pnpm db:postgres` and `pnpm tauri dev`: design and
create a table with an identity column, a commented column, an FK to
`lifecycle.orders`, and a concurrent index; confirm the preview shows
two groups and the table opens after apply. Edit
`lifecycle.order_total`'s body and confirm the viewer shows the new
source; attempt a return-type change and confirm the typed SQLSTATE
error. In the Structure tab: disable and re-enable `orders_touch`,
create a policy on `lifecycle.tenant_rows`, grant `SELECT` to
`lifecycle_reader` and revoke it, and confirm every destructive row
shows the badge and the production confirmation lists it. Confirm the
ClickHouse Specialized tab still generates SQL. Then §5 docs and the
full gate set. Mark `READY FOR REVIEW`.

## Execution record

- 2026-09-04: Step 0 mock A confirmed by the operator.
- 2026-09-04: Steps 1–4 implemented. The full automated suite passed: `pnpm
  format`, `pnpm lint`, `pnpm typecheck`, 1,352 Vitest tests across 109 files,
  `check:ui-gates`, `check:slice-isolation`, the production Vite build, and the
  removed-generator grep.
- 2026-09-04: The disposable PostgreSQL fixture became healthy on port 15432,
  `pnpm tauri dev` compiled and launched, and its Vite server listened on port
  3000. The native Computer Use walkthrough passed against the packaged debug
  app: `lifecycle.plan017_designer_check` was created with a `BY DEFAULT`
  identity, a commented column, an FK to `lifecycle.orders`, and a concurrent
  index; its preview split the table/comments into an atomic group and the
  index into a standalone group, and the table opened after apply.
- 2026-09-04: The routine walkthrough replaced `lifecycle.order_total` with a
  body that adds one and the refreshed viewer showed the new source. Attempting
  to change the return type to `text` failed with PostgreSQL SQLSTATE `42P13`,
  rendered against the failed statement.
- 2026-09-04: The Structure walkthrough disabled and re-enabled
  `orders_touch`, created `plan017_tenant_read` on `lifecycle.tenant_rows`,
  granted `SELECT` to `lifecycle_reader`, and revoked it. Destructive pending
  rows carried the backend-provided badge; on the disposable connection marked
  Production, the typed confirmation listed the revoke before apply. A local
  ClickHouse connection retained the Specialized generator and produced
  `GRANT SELECT ON TABLE \"dbunk_demo\".\"events\" TO \"plan017_reader\";`
  without executing it.
- 2026-09-04: Fresh review fixes made routine names and arguments editable,
  added local statement-boundary validation for designer index expressions,
  and replaced the `PUBLIC` string sentinel with an explicit typed grantee
  choice. The rebuilt native app showed separate `PUBLIC` and `Role` options;
  entering a real uppercase `PUBLIC` role previewed `TO \"PUBLIC\"`. The
  verification-only change was removed before apply, and the full gate set
  passed again.
- 2026-09-04: A second review pass removed the remaining unsafe grantee
  inference when the role catalog is unavailable, truncated, or collides with
  PostgreSQL's public pseudo-role; ambiguous edits now require an explicit
  target and privilege ops are deduplicated. Designer follow-ups added local
  index-predicate validation, SQL-aware top-level expression splitting, and an
  explicit default kind so an empty literal remains `DEFAULT ''`. The full
  gate set passed again with the final test count above.
- 2026-09-04: The final review fixes require every introspected `PUBLIC` or
  `public` sentinel to be resolved explicitly, disable PostgreSQL privilege and
  trigger operations when their typed selections are empty, and preserve
  in-progress trailing separators in designer comma-list inputs while keeping
  submit validation strict. The full gate set passed after these changes.
- 2026-09-04: The terminal review added field-level validation for blank SQL
  expression defaults while preserving empty literal defaults, closed the
  Specialized policy and trigger state over typed choices, rejected unknown
  policy commands and trigger events instead of broadening them, and removed a
  duplicate referential-action converter. All gates and 1,352 tests passed.

## Test plan

Steps 1–4 enumerate the automated coverage; none needs a live
database. Step 5 is the manual pass and must be recorded in the plan's
execution record before `READY FOR REVIEW`.

## Done criteria

- A PostgreSQL user can create a table, create or edit a function or
  procedure, and manage triggers, row-level security, and privileges on
  a table without leaving the reviewed workflow.
- The Specialized tab has no string generator left on PostgreSQL;
  ClickHouse is byte-for-byte unchanged.
- ROADMAP, CONTEXT, ADR-0014, and the register match reality.
- All gates green; the manual pass is recorded.

## STOP conditions

- Plan 016 contract mismatch.
- Any op the vocabulary lacks or any backend change — Plan 016
  amendment requiring operator authorization.
- Any ClickHouse test needs modification to stay green.
- A pending row or a designer preview would need a frontend
  classifier for destructiveness or a frontend-rendered statement.
- Mock selection missing when Step 1 starts.

## Maintenance notes

- Every table-scoped affordance is: form → op(s) via
  `structure-changes.ts` → `queueOp`; preview, review, gate, and
  refresh are inherited. Do not add a second path.
- The routine editor round-trips text fields, not parsed structures; if
  a structured argument builder is ever added it must produce the same
  `arguments` string, never a second op shape.
