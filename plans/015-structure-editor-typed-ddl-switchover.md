# Plan 015: PostgreSQL structure editor switchover to the typed DDL workflow

> **Executor instructions**: Do not start until Plans 013 and 014 are
> `DONE` in `plans/README.md`. Follow this plan step by step. There is
> no mock step — the pending-changes surface already exists and the
> per-statement preview component ships in Plan 014; this plan changes
> what feeds it. Run every verification command and confirm the
> expected result before moving on. Update this plan's README row after
> each step and mark `READY FOR REVIEW` after all gates. A
> reviewer/operator records `DONE: <completion SHA>` after an
> authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat <PLAN_014_COMPLETION_SHA>..HEAD -- src src-tauri plans/README.md
> git status --short -- src src-tauri plans/README.md
> grep -rn "quoteIdent" src/components/table-editor/specialized-editors.tsx   # expect the private helper still present
> grep -rn "generateDdlForEngine" src | wc -l                                  # expect the Plan 014-era call sites
> ```
>
> Expected on a fresh run: no `src` or `src-tauri` output. A
> load-bearing mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MEDIUM-HIGH (this rewrites a working commit path for the
  most-used DDL surface in the app; the danger is a column change that
  previewed one way and applied another, or a ClickHouse regression
  from a shared type that grew a PostgreSQL-only arm)
- **Depends on**: Plans 013 and 014 complete
- **Category**: direction
- **Planned at**: commit `b45e294`, 2026-08-29 (split out of Plan 014
  during pre-execution review)
- **Gap**: `PAR-007` in `plans/parity-gap-register.md`

## Why this matters

After Plan 014 every *schema-level* lifecycle action runs through the
typed preview → review → gated apply workflow, but the table structure
editor — the surface people actually use daily — still generates its
SQL in the frontend (`src/lib/ddl/postgres.ts`), guesses whether a
default is a literal or an expression (`src/lib/ddl/shared.ts:24-57`),
cannot emit a `USING` clause (`postgres.ts:71`), commits through the
blanket-transaction `execute_ddl` that cannot run `CREATE INDEX
CONCURRENTLY`, and shows one opaque SQL blob. The read-only
PK/FK/index/constraint sections have no affordances, and the
specialized index and cross-table FK panels end at "open in SQL
editor". This plan closes that half of `PAR-007` for PostgreSQL while
leaving ClickHouse's frontend generator untouched.

This was Plan 014 §6. It was split out because it is independent of
the navigator/viewer/palette work, rewrites a working path, and carries
a store-type decision that deserved to be made explicitly rather than
discovered mid-step.

## Required Plan 013/014 contract

A mismatch is a STOP condition:

- Plan 013's `PgObjectOp` union with `addColumn { column: NewColumnSpec
  { default: PgDefaultValue } }`, `dropColumn`, `renameColumn`,
  `alterColumnType { using }`, `setColumnNullable`, `setColumnDefault`,
  `addPrimaryKey`, `addUnique`, `addForeignKey`, `addCheck`,
  `dropConstraint`, `createIndex` (explicit-name rendering,
  `concurrently`), `dropIndex`; `preview_object_ddl` and
  `apply_object_ddl` with `DdlApplyResult { appliedStatements,
  runtimeMs }` and the typed `PgObjectError` union.
- Plan 014's `src/lib/object-ddl.ts` (invoke wrappers,
  `formatObjectDdlError`), the per-statement grouped preview component
  inside `components/object-ddl/`, the `ddl` safety-confirmation
  subject, and the reviewed-gate pattern.

## Current frontend state (verified at `b45e294`; re-verify after 014)

- `table-structure/use-structure.ts` — queue + `previewSql` via
  `generateDdlForEngine` (`:151-161`), destructive confirm
  (`:205-227`), `describeChange`/`classifyDestructive` at `:193-194`.
- `pending-changes-section.tsx` — single-blob `SqlPreview` (`:133`);
  Commit disabled while `isRunning` (`:85-93`).
- Store: `pendingStructureChanges: Record<string, PendingChange[]>`
  (`relational-tables.ts:197`), `PendingChange.change: ColumnChangeKind`
  shared with the ClickHouse generator (`src/lib/ddl/index.ts:29-59`),
  `classifyDestructive`, `describeChange` (`table-structure/shared.tsx:53`),
  and the tests. Commit: `relational-tables.ts:1535-1628` →
  `invokeWithSafetyConfirmation` → `execute_ddl`, refresh fan-out
  `:1602-1620`, `DDLOutcome` back.
- Read-only PK/FK/index/constraint sections have no affordances
  (`read-only-sections.tsx`). `AddColumnForm` default is an untagged
  string (`columns-section.tsx:76-150`).
- Specialized editors: `table-editor/specialized-editors.tsx` — eight
  generate-only panels; index (`:180-194`, `concurrently` default
  true) and cross-table FK (`:196-213`) are the two whose ops Plan 013
  models; a private `quoteIdent` at `:1127`.
- ClickHouse structure commits use the same `PendingChange[]` →
  `generateDdlForEngine` → `execute_ddl` path and must keep doing so.

## Decided architecture

### 1. Pending-change store type

`PendingChange.change` becomes a discriminated union:

```ts
type StructureChange =
  | { kind: "column"; change: ColumnChangeKind }   // ClickHouse (and the MySQL/SQLite dead end)
  | { kind: "pg-op"; op: PgObjectOp };             // PostgreSQL
```

- ClickHouse forms keep producing `{ kind: "column" }` and nothing
  downstream of them changes; `generateDdlForEngine`,
  `classifyDestructive`, and `describeChange` narrow on `kind ===
  "column"` and are otherwise untouched.
- PostgreSQL forms produce `{ kind: "pg-op" }` **directly** — there is
  no `ColumnChangeKind → PgObjectOp` mapper. A mapper would have to
  carry the tagged default and `USING` through a type designed
  without them; producing the op at the form is simpler and the
  preview is the single source of truth for description and
  destructiveness (`PlannedStatement.summary` / `.destructive` replace
  `describeChange` / `classifyDestructive` for PG rows).
- The slot stays `Record<string, PendingChange[]>`; one table's list
  is homogeneous by engine. A test pins that a PG list never contains
  a `column` entry and a ClickHouse list never contains a `pg-op`.

### 2. Column forms (PostgreSQL)

- `AddColumnForm` / `ColumnEditPanel`: default gains a
  literal-vs-expression selector producing `PgDefaultValue`; the
  `formatDefault` heuristic is not used for PG. `AlterColumnType` gains
  an optional **USING** field. The six column ops map 1:1 onto
  `addColumn`/`dropColumn`/`renameColumn`/`alterColumnType`/
  `setColumnNullable`/`setColumnDefault`.
- Pending rows for PG display the preview's `summary` once loaded and
  a neutral "pending preview" label before; they never call
  `describeChange`.

### 3. Lifecycle affordances on the read-only sections

- Indexes — per-row Drop (queues `dropIndex`; `concurrently` offered)
  and "New index" (queues `createIndex`).
- Foreign keys / Constraints — per-row Drop (queues `dropConstraint`)
  and "Add foreign key" / "Add check" / "Add unique" / "Add primary
  key" (hidden when a PK exists).
- The specialized **index** and **cross-table FK** panels switch from
  generate-string to queueing these same typed ops into the shared
  pending list (their forms already collect the right fields). The
  GRANT / RLS / trigger panels stay generate-only (deferred kinds) and
  keep the SQL handoff. Retire the private `quoteIdent` (`:1127`) with
  the panels that used it.

### 4. Preview and commit

- `use-structure.ts`: for PostgreSQL, `previewSql` is replaced by an
  async `preview` from `preview_object_ddl` (per-statement, grouped),
  rendered by Plan 014's preview component in place of the single-blob
  `SqlPreview`. A `reviewed` gate mirrors the mutation-review rule:
  any pending-change edit invalidates the loaded preview and Commit
  disables until it reloads; Commit also stays disabled while in
  flight (the existing `isRunning` guard).
- `relational-tables.ts` `commitStructureChanges`: PostgreSQL routes
  to `apply_object_ddl` through `object-ddl.ts` with the typed
  `policyNeedsConfirmation` retry via the `ddl` confirmation subject;
  the `DDLOutcome` return shape is kept; the refresh fan-out
  (`:1602-1620`) runs on success **and** on a `database`/`lockTimeout`
  error with `appliedStatements > 0`. Typed errors render via
  `formatObjectDdlError` instead of `errorToMessage`.
- ClickHouse keeps the frontend generator + `execute_ddl` exactly as
  today. `src/lib/ddl/postgres.ts` stays only as the MySQL/SQLite
  dead-end fallback and gains a header note saying so.

### 5. Documentation

`CONTEXT.md` **DDL Statement** / **DDL Outcome** entries: the
PostgreSQL structure path is now typed-op preview/apply; ClickHouse is
the remaining `execute_ddl` caller. ADR-0014 pointer (index/FK panels
now queue typed ops). Register `PAR-007`: structure-editor half
delivered; remaining deferrals unchanged.

## Commands you will need

```sh
pnpm format && pnpm lint && pnpm typecheck
pnpm vitest run
pnpm run check:ui-gates && pnpm run check:slice-isolation
pnpm db:postgres && pnpm tauri dev        # Step 4 manual pass
grep -rn "quoteIdent" src/components/table-editor   # empty after Step 3
```

## Scope

Expected files touched: `store/types.ts` (`StructureChange`),
`store/relational-tables.ts` (+test), `table-structure/*`
(`use-structure.ts`, `columns-section.tsx`, `column-row.tsx`,
`pending-changes-section.tsx`, `read-only-sections.tsx`, `shared.tsx`,
tests), `table-editor/specialized-editors.tsx` (+test),
`src/lib/ddl/index.ts` and `postgres.ts` (narrowing + header note),
`CONTEXT.md`, ADR-0014, the register, `plans/README.md`.

Out of scope: any new Tauri command or backend change (Plan 013
amendment); ClickHouse behaviour; the GRANT/RLS/trigger panels;
`execute_ddl` removal; the create-table designer.

## Resume protocol

Each step ends with all gates green; re-run the step's verification on
resume.

## Git workflow

Working tree only; no commits/pushes/PRs without explicit operator
authorization.

## Steps

### Step 1: Store type split

Per §1. Introduce `StructureChange`, narrow every existing consumer on
`kind === "column"`, and add the homogeneity test. **No PG form
produces `pg-op` yet** — this step is a pure type refactor and every
existing test keeps passing unchanged. Gates: standard three + vitest +
both checks.

### Step 2: Column forms and async preview

Per §2/§4 for the six column ops only. Tests: each PG form produces
the expected `pg-op` (tagged default both ways, USING threaded);
pending edits invalidate the loaded preview and Commit re-disables;
Commit disabled while in flight; PG commit calls `apply_object_ddl`
and renders typed errors including `appliedStatements > 0` refresh;
ClickHouse path unchanged (existing tests keep passing). Gates as
Step 1.

### Step 3: Constraint/index affordances and specialized panels

Per §3. Tests: index/constraint rows queue drops; "Add …" forms queue
the right ops; PK add hidden when a PK exists; the specialized
index/FK panels queue ops into the shared pending list; `quoteIdent`
is gone; GRANT/RLS/trigger panels still generate strings. Gates as
Step 1.

### Step 4: End-to-end pass and truth pass

Manual pass with `pnpm db:postgres` up and `pnpm tauri dev` against
`lifecycle.orders`: add a column with a literal default and one with
an expression default; change a type with `USING`; add a check
constraint and a `CONCURRENTLY` index in one pending batch and confirm
the preview shows two groups with the callout; break a statement
(duplicate constraint name) and confirm the typed error names it; set
environment to production and confirm the `ddl` confirmation lists the
summaries; verify a ClickHouse structure commit still works. Then §5
docs and the full gate set. Mark `READY FOR REVIEW`.

## Test plan

Steps 1–3 enumerate the automated coverage; no automated test needs a
live database. Step 4 is the manual pass.

## Done criteria

- PostgreSQL structure commits run through the typed backend workflow
  with tagged defaults and `USING`; every pending row's description
  and destructiveness come from the preview.
- Constraint and index add/drop are queueable from the structure
  editor; the index and cross-table FK panels queue typed ops.
- ClickHouse is byte-for-byte unchanged in behaviour; its tests are
  untouched.
- CONTEXT, ADR-0014, and the register match reality.
- All gates green.

## STOP conditions

- Plan 013/014 contract mismatch.
- An op the vocabulary lacks or any backend change — Plan 013
  amendment requiring operator authorization.
- Any ClickHouse test needs modification to stay green.
- A PG pending row would need `describeChange` or
  `classifyDestructive` (the preview must be the source).

## Maintenance notes

- `StructureChange` is engine-homogeneous per table; a mixed list is a
  bug, and the Step 1 test pins it.
- For PostgreSQL, description and destructiveness of a pending row
  come from `preview_object_ddl`; do not reintroduce a frontend
  classifier for PG.
- Adding a new PG structure affordance is: form produces a `pg-op`,
  nothing else — preview, review, gate, and refresh are inherited.
