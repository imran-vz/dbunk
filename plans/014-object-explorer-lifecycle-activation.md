# Plan 014: Object explorer, viewers, and lifecycle activation

> **Executor instructions**: Do not start until Plan 013 is `DONE` in
> `plans/README.md`. Follow this plan step by step. Step 1 ends in a
> STOP for operator mock selection — do not write any TSX before a mock
> is selected and recorded. Run every verification command and confirm
> the expected result before moving on. Update this plan's README row
> after each step and mark `READY FOR REVIEW` after all gates. A
> reviewer/operator records `DONE: <completion SHA>` after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat <PLAN_013_COMPLETION_SHA>..HEAD -- src src-tauri plans/README.md plans/mocks/object-lifecycle
> git status --short -- src src-tauri plans/README.md plans/mocks/object-lifecycle
> ```
>
> Expected on a fresh run: no `src` or `src-tauri` output. A
> load-bearing mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MEDIUM-HIGH (the backend semantics are Plan 013's; this
  plan's risk is a destructive apply presented as routine — a drop
  without its impact, a cascade without its reach, a preview the user
  never actually reviewed)
- **Depends on**: Plan 013 complete
- **Category**: direction
- **Planned at**: commit `b45e294`, 2026-08-29
- **Amended**: 2026-08-29 (pre-execution review) — contract excerpt
  aligned with the amended Plan 013; structure-editor switchover split
  out as Plan 015; see `## Review correction record` at the end.
- **Gap**: `PAR-007` in `plans/parity-gap-register.md`

## Why this matters

Plan 013 ships contracts nobody can reach: an eleven-kind catalog no
tree renders, descriptions no tab shows, drop impact no dialog reads,
and a preview/apply workflow with no entry point. The navigator lists
tables only; the palette's own header comment says
functions/sequences/types "wait on PAR-007 object viewers"
(`src/lib/open-anything.ts:9-12`); and the workbench consolidation
deleted the old sidebar that at least *listed* every kind. This plan is
the visible half of `PAR-007` for *schema-level* objects: routine
PostgreSQL administration stops requiring hand-written catalog queries,
while every generated statement stays inspectable before it runs. The
table structure editor's switch onto the same workflow is Plan 015 — it
is independent of everything here, rewrites a working commit path, and
deserves its own gates.

## Required Plan 013 contract

A mismatch is a STOP condition, not permission to re-implement here:

- `load_pg_object_catalog(connectionId) -> PgObjectCatalog` — per-schema
  entries for eleven kinds plus database-scoped `eventTriggers` /
  `roles` / `tablespaces` (**plain entry lists, not kinds** — they
  cannot form a `PgObjectRef`); entries carry `name`, optional
  `identityArgs`, optional `comment`, optional `typeClass`; a
  `truncated: PgCatalogTruncation[]` list names any (schema, kind)
  group cut at the cap. Extension members are already excluded.
- `PgObjectKind` has twelve arms (schema + eleven schema-scoped kinds).
  `PgObjectRef.schema` is `null` only for `schema`; extensions carry
  their placement schema.
- `describe_pg_object(connectionId, reference) -> PgObjectDescription`
  — `owner`, `comment`, `definitionSql` (nullable; null for
  aggregates), tagged `facts`; `objectNotFound` is a typed arm. There
  is no `describeUnsupported`.
- `load_pg_drop_impact(connectionId, reference) -> PgDropImpact` —
  the **transitive** closure, each dependent with `depth`, capped at
  200, `truncated` disclosed.
- `preview_object_ddl(connectionId, ops) -> DdlPlanPreview` —
  `statements` with `sql`/`summary`/`destructive`/`transactional`,
  `groups` of `atomic`/`standalone`. `renameObject` is valid only for
  schema/table/view/matview/sequence; `createMaterializedView` has no
  `orReplace` (PostgreSQL has none).
- `apply_object_ddl({ connectionId, ops, confirmed }) -> DdlApplyResult
  { appliedStatements, runtimeMs }` with the typed `PgObjectError` union:
  `policyBlocked`, `policyNeedsConfirmation { statements:
  DdlStatementSummary[] }` where `DdlStatementSummary = { index,
  summary, destructive, transactional }` (**not** the query-session
  `StatementClassSummary`), `lockTimeout { statementIndex,
  appliedStatements }`, `database { statementIndex?, code, message,
  position, appliedStatements, residue? }` with `residue` =
  `{ kind: "invalidIndex", schema, name }`, `invalidOp`, `connection`,
  `unsupportedEngine`, `objectNotFound`. Apply accepts ops only — never
  a whole statement.
- TS mirrors for all of the above exist in `src/lib/store/types.ts`,
  exported through the barrel, referenced by nothing.
- The live fixture has a `lifecycle` schema with one of every in-scope
  kind, including the `add_nums` overload pair and the
  `orders → orders_view → orders_mat` chain.
- No frontend caller of `refresh_materialized_view` exists — only the
  backend command (`src-tauri/src/commands/relational.rs:359-380`) and
  the legacy allowlist entry (`src/lib/invoke-with-safety-confirmation.ts:11`).
  This plan builds the caller (§3).

## Current frontend state (verified at `b45e294`)

- Navigator: `src/components/workbench/database-navigator.tsx` — a
  two-level schema → tables tree (`NavigatorRow` `:35-37`) with roving
  tabindex, arrows, Home/End, type-ahead (`:173-197`) and a pinned
  filter input; `onOpenTable` is the only activation (`:115-124`).
  Wired in `workbench/relational-workbench.tsx:347-364`.
- Tabs: `WorkspaceTabKind = "table" | "query" | "key" | "cli" |
  "pubsub" | "server"` (`src/lib/store/types.ts:1042-1083`). All
  creation funnels through `openWorkspaceTab`
  (`workspace-tabs.ts:241-269`, dedupe: table tabs by schema+table,
  everything else by label). `openViewTab` (`:297-305`) opens views as
  `SELECT * … LIMIT 100` query tabs — the sole non-table object path.
  Session persistence: `session-persistence.ts:30-32` serializes only
  `query`/`table` kinds; restore whitelists the same
  (`workspace-tabs.ts:372`) and rejects tabs whose `schema` is not a
  string (`:371`). Tab strip icon map is a binary `kind === "query"`
  ternary (`object-tab-row.tsx:347,389`), and the MRU switcher has the
  same ternary (`workbench/tab-shortcuts.tsx:188-192`);
  `relational-workbench.tsx:132` routes every non-query tab to the
  "tables" rail. Restored table tabs against a disconnected connection
  render a "Connect & load" shell and fire nothing until connected
  (`table-editor-panel.tsx:739-770`, gated by `awaitingConnection` at
  `use-table-session.ts:268` — Plan 010's contract).
- Connect flow: `connectConnection` awaits `load_schema_explorer` and
  then unconditionally `set`s Connected + explorer
  (`store/connections.ts:575-607`) with no check that
  `teardownConnectionWorkspace` (`:117-132`, which also drops the
  backend pool via `socket_lifecycle.rs:86-96`) ran meanwhile; an
  explorer failure is `console.error`ed and the navigator shows
  "Connect to load schemas" while status is Connected
  (`connections.ts:593-595`, `database-navigator.tsx:252-260`). The
  legacy PG explorer emits a synthetic `"Database"` pseudo-schema
  (`src-tauri/src/dispatch/relational.rs:634`) that today leaks into
  palette schema items, `sql-completions.ts:163-170`, and
  `createNewQueryTab`'s `explorerSchemas[0]` (`workspace-tabs.ts:322-324`).
- Navigator UI state: `expandedSchemas` is a flat `string[]` of
  `${connectionId}:${schema}` ids (`relational-tables.ts:190,423-427`,
  persisted at `session-persistence.ts:68`, restored at
  `workspace-tabs.ts:398-402`). The tree is not virtualized;
  `rows`/`filteredSchemas` recompute over everything per keystroke
  (`database-navigator.tsx:61-95`) and every row is in the DOM
  (`:262-328`).
- Palette: `open-anything.ts` — `OpenAnythingTarget` (`:34-47`),
  `RELATION_SOURCES` maps four `SchemaExplorer` fields (`:117-137`),
  `KIND_CAPS` (`:92-98`), frecency keys stability-critical (`:15-16`),
  key-migration precedent (`command-palette.tsx:63-93`); `runTarget`
  ends in a `never` exhaustiveness guard (`:340-345`).
- Structure editing and the specialized editors are **Plan 015's**
  surface; this plan touches neither `table-structure/*` nor
  `table-editor/specialized-editors.tsx` except to reuse the
  "open in SQL editor" handoff at `:138-147` (read, not modify).
- Review pattern to mirror: `mutation-review/` — scoped draft +
  generation counter, three-state preview machine with a `reviewed`
  flag that resets when regenerated DML differs
  (`mutation-review-panel.tsx:358-400,619-632`), per-statement
  `dml-preview.tsx` with Copy SQL/params, `applyWithSafetyConfirmation`
  retry on `policyNeedsConfirmation` (`:65-87`),
  `MutationReviewAside` shell (`:669-679`).
- Error plumbing: `decode-transport-error.ts` helpers;
  `formatSharedTransportError` (`safety-policy.ts:162`);
  `requestConfirm` (`confirm.ts`). Safety confirmation:
  `SafetyConfirmationRequest.subject` is `{ kind: "statements";
  statements: StatementClassSummary[] } | { kind: "command"; … }`
  (`safety-confirmation.ts:11-16`); the dialog renders statements as
  `CLASS_LABEL[class]` + Unbounded/Destructive flags only
  (`safety-confirm-dialog.tsx:79-93`) — it cannot show statement text.
- Store: one zustand store, slices listed in `store/index.ts:157-171`;
  `check:slice-isolation` enumerates slice files; every external import
  goes through the `@/lib/store` barrel; cross-slice writes go through
  an owner-side setter (`store/README.md:54-63`).
- UI gates: `scripts/check-ui-gates.mjs` scans `src/components/**/*.tsx`
  only — HTML mocks are not checked.
- Stale docs this plan must fix: `open-anything.ts:9-12` header,
  `plans/README.md:94` ("fifteen write-capable surfaces" — now
  sixteen), the register's `PAR-007` block and its stale
  `workspace-overview/compare-tab.tsx` evidence citation under
  `PAR-008`, `CONTEXT.md` glossary (no object vocabulary). `ROADMAP.md`
  and `docs/PENDING_TASKS.md` do **not** mention object viewers or
  `PAR-007` (verified by grep) — nothing to update there.

## Decided frontend architecture

### 1. Catalog store slice and explorer adapter

New slice `src/lib/store/pg-objects.ts` (add to the barrel, the slice
list, and `check:slice-isolation`): `pgObjectCatalog` keyed by
connection id with `{ status: "idle" | "loading" | "ready" | "error";
catalog?; error?: PgObjectError; generation }`,
`loadPgObjectCatalog(connectionId)`, description cache keyed by a
canonical ref key, and `dropPgObjectCachesForConnection` wired into the
existing disconnect/delete cascades.

- **Generation guard.** Every async write into the slice (catalog load,
  describe, refresh-after-apply) captures the connection's `generation`
  before the await and discards the result if it changed.
  `dropPgObjectCachesForConnection` bumps it. This is the mutation-
  review counter pattern applied to the connect → disconnect →
  reconnect race that `connectConnection` currently ignores; a dropped
  catalog must not be resurrected by a late response.
- On connect, PostgreSQL connections fetch the catalog **instead of**
  `load_schema_explorer`, and a pure, memoised-at-load adapter
  (`catalogToSchemaExplorer`) derives the legacy `SchemaExplorer` shape
  — including the `name(identityArgs)` concatenation for routines — so
  `sql-completions.ts` and every other explorer consumer keep working
  with one fetch. The derived explorer is written through an
  owner-side setter on `relational-tables.ts` (it owns
  `schemaExplorer`), not a cross-slice `set`. The catalog is a strict
  superset of what consumers read (`name`, `tables`, `views`, plus the
  palette's four fields), so the adapter is total.
- The legacy `"Database"` pseudo-schema disappears from the derived
  shape for PostgreSQL (the three list-only groups live on the catalog
  root). This is a deliberate behaviour change: no more `Database`
  schema in the palette, completions, or `createNewQueryTab`'s default
  schema. Test it.
- **Load failure is visible.** `status: "error"` renders in the
  navigator as an inline error row with the formatted `PgObjectError`
  and a Retry affordance — not `console.error` + "Connect to load
  schemas". Connection status still becomes Connected (query tabs work
  without a catalog).
- Non-PostgreSQL engines are untouched.

### 2. Navigator breadth

`database-navigator.tsx` grows grouped sections per schema — Tables,
Views, Materialized Views, Foreign Tables, Sequences, Functions,
Procedures, Aggregates, Types, Domains, Extensions — plus a
database-scoped group (Event Triggers, Roles, Tablespaces, list-only)
placed per the selected mock. Empty groups are hidden.

- **Expand state.** Tables stays expanded by default, other groups
  collapsed. `expandedSchemas` is an *expanded* list of
  `${connectionId}:${schema}` ids and stays exactly as it is. Add a
  second persisted list, `expandedNavigatorGroups: string[]` of
  `${connectionId}:${schema}:${group}` ids, with the rule "a group is
  expanded iff its id is present, except `tables`, which is expanded
  iff its id is *absent*" so the default state serializes to an empty
  list. Write the rule in the slice, restore it beside
  `expandedSchemas` (`workspace-tabs.ts:398-402`), and never
  reinterpret legacy `expandedSchemas` entries.
- **Bounded rendering.** Each group renders at most 200 rows followed
  by a "Show N more" row that expands that group only; the catalog's
  `truncated` list renders a per-group "list cut at 2000 on the
  server" note. The filter searches all kinds but the same per-group
  cap applies to matches. Row/filter recomputation stays memoised per
  connection and filter string, not per keystroke over every group
  when the filter is unchanged. Virtualization is *not* required —
  the cap bounds the DOM.
- The roving-focus/type-ahead keyboard model extends to the new rows
  and group headers.
- **Activation is consistent with the palette (§5).** Tables →
  `openTableTab`. Views, materialized views, and foreign tables keep
  their existing primary activation (`openViewTab` / browse) in *both*
  the navigator and the palette, and gain "Open object viewer" as a
  secondary row action. Sequences, functions, procedures, aggregates,
  types, domains, and extensions → object viewer as the primary
  activation. List-only rows are inert with a tooltip saying why.
- Each creatable group gets a "New …" affordance (schema, view,
  materialized view, sequence, enum) opening the Step-4 dialogs.

### 3. `object` workspace tab kind and viewer

- `WorkspaceTabKind` gains `"object"`; `WorkspaceTab` gains
  `objectRef?: PgObjectRef`. `WorkspaceTab.schema` (required string)
  holds `objectRef.schema ?? ""` for schema-kind tabs; the canonical
  ref key normalises `null` and `""` identically so the two never
  produce distinct keys. Touch every enumerated site: dedupe (by
  connection + canonical ref key), tab-strip icon map
  (`object-tab-row.tsx`, third branch) **and** the MRU switcher
  (`tab-shortcuts.tsx:188-192`), `renderMainPane`,
  `relational-workbench.tsx:132` rail routing (object tabs go to the
  "tables" rail — state it in a comment), `closeTab` (no draft cascade
  needed), `serializeTabs` whitelist + restore validator (validate
  `objectRef` field-by-field, drop invalid), palette `activate-tab`
  filtering.
- New `src/components/object-viewer/` tab component. **It follows the
  Plan 010 disconnected-restore contract**: while the connection is not
  Connected it renders the same "Connect & load" shell as a restored
  table tab and invokes nothing; describe fires when status becomes
  Connected (the `awaitingConnection` pattern of
  `use-table-session.ts:268`). Once connected it loads
  `describe_pg_object` (status/error/not-found states via
  `state-panel.tsx`, guarded by the slice generation); header with kind
  badge, qualified name, owner, and comment (inline edit →
  `setComment` through the DDL review flow); a definition section
  rendering `definitionSql` in a read-only block with **Copy** and
  **Open in SQL editor** (the `specialized-editors.tsx:138-147`
  handoff, reused for routines as the edit path — `pg_get_functiondef`
  output is a runnable `CREATE OR REPLACE`); a facts section per kind
  (exhaustive switch over `PgObjectFacts`, no default arm); and an
  actions row, **rendered in Step 4, not Step 3**:
  - Drop (all kinds) and Comment (all kinds).
  - Rename — **only** for schema, table, view, materialized view,
    sequence (the kinds `renameObject` renders); hidden otherwise.
  - Enum: add value / rename value. Sequence: alter
    (restart/increment/…). View: Edit definition (`createView` with
    `orReplace: true`).
  - Materialized view: **Edit definition is drop + create** — there is
    no `CREATE OR REPLACE MATERIALIZED VIEW`. The action opens the drop
    flow (impact first, §4) and appends `createMaterializedView` to the
    same op list, so the review shows one atomic group with the
    destructive drop marked and the user sees that indexes and grants
    on the matview are lost. Refresh routes to the backend
    `refresh_materialized_view` through `invokeWithSafetyConfirmation`
    — a **declared exception** to the "every lifecycle action goes
    through the review dialog" rule, because it is not DDL and the
    legacy command already exists; note the exception in the component
    and in `CONTEXT.md`.
  - Aggregates render facts with honest "definition rendering not
    supported" copy.
- A refresh affordance re-describes after any apply that touched the
  object — on success **and** on a `database`/`lockTimeout` error with
  `appliedStatements > 0` (something ran). `objectNotFound` after an
  external drop renders a clean empty state with a "close tab" action,
  never a crash.

### 4. DDL review flow (shared) and drop dialog

- New `src/lib/object-ddl.ts`: typed invoke wrappers for
  preview/apply/impact returning the `result-mutation-client`
  `{ ok | cancelled | error }` shape, a decoder for `PgObjectError`
  (the `decode-transport-error.ts` conventions), and
  `formatObjectDdlError` — an exhaustive formatter, no default arm.
  `database` and `lockTimeout` arms name the failing statement summary
  and, when `appliedStatements > 0`, say plainly which earlier
  statements were applied; a `residue` renders "an invalid index
  `<schema>.<name>` was left behind — drop it before retrying".
- **Confirmation subject.** Add `{ kind: "ddl"; statements:
  DdlStatementSummary[] }` to `SafetyConfirmationRequest.subject`
  (`safety-confirmation.ts:11-16`) and a `confirmDdlStatements`
  helper; `safety-confirm-dialog.tsx` renders the summary text with a
  danger-toned Destructive flag and a "runs outside a transaction"
  flag for non-transactional statements. The existing `statements`
  subject cannot show statement text and must not be reused.
- New `src/components/object-ddl/ddl-review-dialog.tsx`: given ops, it
  calls `preview_object_ddl` and renders per-statement articles
  (summary, SQL in `<pre>`, Copy — the `dml-preview.tsx` shape),
  visible group boundaries, a danger-toned marker on destructive
  statements, and a distinct "runs outside a transaction; earlier
  statements stay applied if it fails" callout on `standalone` groups.
  Apply is disabled until the preview has loaded **and while an apply
  is in flight** (the reviewed-gate: ops are frozen at dialog open;
  editing reopens preview; a double-click cannot fire apply twice).
  Apply calls `apply_object_ddl` unconfirmed and on
  `policyNeedsConfirmation` routes through `confirmDdlStatements` with
  the returned summaries, then retries `confirmed: true` (the
  `applyWithSafetyConfirmation` pattern — typed retry, not the legacy
  string-tag path). Success reports `runtimeMs` and triggers the
  caller's refresh callback.
- **Refresh scope is per op kind, not blanket.** The dialog computes
  from the ops what changed: catalog-affecting ops (create/drop/rename/
  comment of a catalog entry) reload the catalog; ops on a described
  object re-describe it; nothing here touches structure/browse caches
  (that is Plan 015's concern). A `database`/`lockTimeout` error with
  `appliedStatements > 0` runs the same refresh as success — the
  caches are stale either way.
- Drop flow: the Drop action first calls `load_pg_drop_impact` and
  shows the transitive dependents grouped by depth (capped list,
  `truncated` disclosed, empty → "no dependents found"). Default emits
  `cascade: false`; a separate, explicit opt-in ("Also drop these N
  dependent objects — CASCADE") flips it and re-previews. The review
  dialog follows.

### 5. Palette reach

`open-anything.ts`: new target `{ type: "open-object"; connectionId;
reference: PgObjectRef }` and kind `"object"`; index sources extend to
the catalog's non-relation kinds with per-kind badges (fn, proc, agg,
seq, type, domain, ext) under a shared `object` cap in `KIND_CAPS`;
frecency keys `object:<connectionId>:<canonical ref key>` (the
`command-palette.tsx:63-93` migration precedent applies if any key
changes shape later). `runTarget` gains the branch (the `never` guard
forces it); schemas keep `reveal-schema`; views/matviews/foreign
tables keep `open-relation` (consistent with the navigator's primary
activation, §2) and are **not** double-indexed as objects. Delete the
`:9-12` "wait on PAR-007" comment. List-only kinds (event triggers,
roles, tablespaces) are **not** indexed — no open surface, no palette
entry.

### 6. Create dialogs

One thin dialog family (per the selected mock): New schema (name);
New sequence (name + optional params); New enum (name + ordered
labels); New view (name + SQL body textarea; `orReplace: true` when
reached from a view's Edit definition); New materialized view (name +
SQL body, `withData`; when reached from a matview's Edit definition it
is preceded by the drop flow per §3). Each ends in the shared DDL
review dialog — no dialog applies directly.

### 7. Documentation truth pass (`PAR-017` discipline)

`CONTEXT.md` glossary: **Object Ref** (kind + schema + name +
overload-safe `identityArgs`), **Object Catalog**, **Object Viewer**
(the `object` tab kind), **DDL Plan** (typed operations), **DDL
Preview** (per-statement, grouped, distinct from **DML Preview**),
**Drop Impact** (transitive); note the matview-refresh legacy-path
exception under **DDL Statement** / **DDL Outcome** together with the
non-transactional exception. `plans/README.md:94` fifteen → sixteen
surfaces. Register: `PAR-007` progress block (delivered scope, the
Plan 013 deferral list, Plan 015 as the pending structure-editor
half, corrected evidence paths), fix the stale
`workspace-overview/compare-tab.tsx` citation under `PAR-008`, and
update the `PAR-005` "palette reach … blocked on PAR-007" line. No
claim may outrun the implementation — roles, RLS, databases, triggers,
the create-table designer, and the structure-editor switchover (Plan
015) are listed as deferred/pending, not delivered.

**Pre-authorized backend deletion.** Once every PostgreSQL explorer
consumer reads the derived shape (Step 2), the sqlx-Any PostgreSQL arm
of `fetch_schema_explorer_sqlx` (`src-tauri/src/dispatch/relational.rs:521-652`)
is dead code. Delete it in this step (the PostgreSQL match arm returns
`UnsupportedEngine`-style error text pointing at
`load_pg_object_catalog`), run the `just` trio, and record it in the
README row. This is the one backend edit this plan authorizes; STOP if
anything else in `src-tauri` needs to change.

## Commands you will need

```sh
pnpm format && pnpm lint && pnpm typecheck
pnpm vitest run
pnpm run check:ui-gates && pnpm run check:slice-isolation
just fmt && just lint && just test        # Step 6 (explorer-arm deletion) and any authorized Plan 013 amendment
pnpm db:postgres && pnpm tauri dev        # Step 6 manual pass (separate terminals; compose runs attached)
grep -rn "wait on PAR-007" src            # empty after Step 5
```

## Scope

Expected files touched (creation marked ＋): ＋ `store/pg-objects.ts`
(+test), `store/index.ts`, `store/types.ts` (store-state types only),
`store/connections.ts` (catalog fetch + generation guard),
`store/relational-tables.ts` (owner-side `schemaExplorer` setter,
`expandedNavigatorGroups`), `package.json` (slice-isolation list),
`database-navigator.tsx` (+test), `relational-workbench.tsx`,
`workspace-tabs.ts` (+test), `session-persistence.ts` (+test),
`object-tab-row.tsx`, `tab-shortcuts.tsx`,
＋ `components/object-viewer/*` (+tests), ＋ `lib/object-ddl.ts`
(+test), `lib/safety-confirmation.ts` (+test),
`components/safety-confirm-dialog.tsx` (+test),
＋ `components/object-ddl/ddl-review-dialog.tsx` (+test),
drop-impact dialog, create dialogs, `open-anything.ts` (+test),
`command-palette/command-palette.tsx` (+test),
`src-tauri/src/dispatch/relational.rs` (Step 6 deletion only), docs
listed in §7, `plans/*`.

Out of scope (Plan 015): `table-structure/*`,
`table-editor/specialized-editors.tsx`, `src/lib/ddl/*`,
`commitStructureChanges`, the structure/browse refresh fan-out. Out of
scope entirely: any new Tauri command, backend behaviour change beyond
the Step 6 deletion, or migration (Plan 013 amendment, not here);
MySQL/SQLite/ClickHouse lifecycle; roles/grants/RLS/trigger/partition/
rule/tablespace/database lifecycle; extension install/remove; the
create-table designer; `execute_ddl` removal (ClickHouse still needs
it).

## Resume protocol

Each step ends with all gates green; re-run the step's verification on
resume. The selected mock is recorded in `plans/README.md` — if no
selection is recorded, you are still in Step 1 regardless of tree
state.

## Git workflow

Working tree only; no commits/pushes/PRs without explicit operator
authorization.

## Steps

### Step 1: Produce and select local static UI mocks — STOP

Three self-contained static HTML mocks under
`plans/mocks/object-lifecycle/` (`mock-a.html` … `mock-c.html`), each
showing: the navigator with grouped object sections (populated +
collapsed states, a "Show N more" capped group, a server-truncated
note, the catalog error row with Retry, list-only database group); an
object viewer tab for a view, a sequence, and an enum (header,
definition, facts, actions — Rename only where applicable) plus the
disconnected "Connect & load" shell; the DDL review dialog in three
states (multi-statement atomic plan; plan containing a `CONCURRENTLY`
standalone group with its outside-a-transaction callout; destructive
drop with the depth-grouped impact list and the explicit CASCADE
opt-in); and the `ddl` safety-confirmation dialog showing statement
summaries. Differentiate along real axes (e.g. A: viewer as a full tab
with stacked sections, dialogs centered; B: viewer with a right facts
rail, review as a side panel reusing the aside shell; C: compact viewer
with tabbed definition/facts, review inline above the actions). Every
mock renders destructive statements and the CASCADE opt-in in danger
tone using the design tokens — the UI-gate script does not scan HTML,
so this is a review criterion, not an automated one. **STOP: operator
selects a mock; record the selection in `plans/README.md` before any
TSX.**

### Step 2: Catalog slice, explorer adapter, navigator breadth

Per §1/§2. Tests: adapter derives every legacy explorer field from a
catalog fixture (routines concatenated, type classes preserved, no
`Database` pseudo-schema); PG connect fetches catalog once and explorer
consumers see the derived shape; **a catalog response arriving after
`dropPgObjectCachesForConnection` is discarded** (generation guard);
catalog error renders the navigator error row and Retry re-fetches;
navigator renders groups, hides empty ones, caps a 250-entry group at
200 with "Show 50 more", renders the server-truncated note, filter
matches a function by name, keyboard model traverses group headers;
group expand state round-trips through serialize/restore without
touching legacy `expandedSchemas` entries; caches drop on disconnect.
Gates: standard three + vitest + `check:ui-gates` +
`check:slice-isolation`.

### Step 3: `object` tab kind and viewer (read-only)

Per §3 **without the actions row** — the review dialog it needs is
Step 4, so Step 3 ships a viewer that can only read, copy, and hand
off to the SQL editor. Tests: open-object dedupes by ref (including
`null` vs `""` schema); serialize/restore round-trips an object tab
and drops a malformed `objectRef`; both icon branches
(`object-tab-row`, `tab-shortcuts`); viewer renders the "Connect &
load" shell while disconnected and invokes nothing, then describes on
Connected; viewer renders each `PgObjectFacts` arm from fixtures
(exhaustive — a new arm fails typecheck); `objectNotFound` renders the
empty state; a describe response from a stale generation is discarded;
definition Copy / Open-in-SQL-editor handoff. Gates as Step 2.

### Step 4: DDL review flow, confirmation subject, drop impact, create dialogs, viewer actions

Per §4/§6 plus the viewer's actions row from §3. Tests: `ddl`
confirmation subject renders summaries with destructive and
non-transactional flags; preview renders statements, groups,
destructive tone, and the standalone callout; apply disabled until
preview and while in flight (double-click fires once); typed
confirmation retry on `policyNeedsConfirmation`; `database` /
`lockTimeout` error copy names the statement, applied count, and
residue; refresh runs on success and on error with
`appliedStatements > 0`, and only reloads the catalog for
catalog-affecting ops; drop dialog shows depth-grouped
dependents/truncation/empty states and CASCADE requires the explicit
opt-in (asserting the op payload flips); Rename is absent for enum/
domain/routine/extension viewers; matview Edit definition produces
`[dropObject, createMaterializedView]` after the impact step; matview
Refresh routes to `invokeWithSafetyConfirmation`; each create dialog
produces the expected ops. Gates as Step 2.

### Step 5: Palette reach

Per §5. Tests: index emits object items with badges and caps;
frecency keys stable; `runTarget` opens the viewer; views stay
`open-relation` and are not double-indexed; list-only kinds absent;
the header comment is gone. Gates as Step 2.

### Step 6: End-to-end pass, explorer-arm deletion, truth pass, mocks removal

Manual pass with `pnpm db:postgres` up and `pnpm tauri dev` against the
`lifecycle` fixture schema: browse every navigator group; open viewers
for a view, matview, sequence, enum, domain, extension, and both
`add_nums` overloads (distinct tabs); create schema → sequence → enum →
view and drop them through the impact dialog; attempt a drop of
`orders` and confirm **both** `orders_view` (depth 1) and `orders_mat`
(depth 2) appear as impact, then cancel; edit `orders_mat`'s definition
and confirm the review shows the destructive drop before the create;
set environment to production and confirm the `ddl` confirmation lists
the statement summaries; disconnect while a catalog load is in flight
and confirm nothing resurrects; restart the app and confirm object
tabs restore as "Connect & load" shells and describe after connect.
Then delete the sqlx-Any PostgreSQL explorer arm (§7), the §7
documentation truth pass, delete `plans/mocks/object-lifecycle/`, and
run the full gate set (standard three, vitest, both checks, the `just`
trio). Mark `READY FOR REVIEW`.

## Test plan

Steps 2–5 enumerate the automated coverage; no automated test needs a
live database. Step 6 is the manual pass against the plain fixture
plus the `just` trio for the backend deletion.

## Done criteria

- The navigator shows every catalog kind, bounded per group, with
  server truncation and load errors visible; the palette reaches every
  kind with an open surface; both honest about list-only kinds; views
  activate the same way from both.
- Object viewers show owner, comment, reconstructed DDL, and typed
  facts; survive external drops, disconnected restore, and the
  connect/disconnect race gracefully; object tabs persist and restore.
- Every lifecycle action — create/rename/drop/comment, enum and
  sequence edits, view definition edits, matview drop + recreate —
  flows through one review dialog showing exact statements, groups,
  destructive tone, and non-transactional disclosure; the production
  confirmation shows statement summaries; drops show transitive impact
  first and CASCADE is an explicit opt-in; partial applies refresh.
  Matview refresh is the one declared legacy-path exception.
- The sqlx-Any PostgreSQL explorer arm is deleted; MySQL/SQLite/
  ClickHouse explorers are untouched.
- The stale palette comment is gone; CONTEXT, the register, and README
  match reality, deferrals and Plan 015 included.
- All gates green.

## STOP conditions

- No recorded mock selection.
- Plan 013 contract mismatch.
- Any need for a new Tauri command, changed backend semantics, an op
  the vocabulary lacks, or a migration — that is a Plan 013 amendment
  requiring operator authorization, not an inline edit. The Step 6
  explorer-arm deletion is the only pre-authorized backend edit.
- The review dialog would need to apply SQL it did not obtain from
  `preview_object_ddl`, or apply without a loaded preview.
- Any file under `table-structure/` or `specialized-editors.tsx` needs
  a behavioural change — that is Plan 015.
- The UI gates require non-token colours for the selected mock —
  re-cut the mock instead.

## Maintenance notes

- `PgObjectFacts` and `PgObjectError` rendering are exhaustive
  switches with no default arm — a Plan 013 amendment adding an arm
  fails typecheck here until rendered.
- The canonical ref key (dedupe, frecency, description cache) lives in
  one helper in `pg-objects.ts`; every keyed surface uses it, and it
  normalises `null`/`""` schema identically.
- Every async write into `pg-objects.ts` checks the connection
  generation captured before its await; a new async path that skips
  this reintroduces the resurrection race.
- The reviewed-gate invariant: ops mutated ⇒ preview invalidated ⇒
  apply disabled ⇒ in-flight apply blocks a second apply. Any new
  lifecycle entry point must route through the review dialog to
  inherit it; matview refresh is the recorded exception.
- List-only kinds (event triggers, roles, tablespaces) are the marker
  for the deferred `PAR-011`-adjacent work; do not grow ad-hoc actions
  on them here.

## Review correction record

Pre-execution review, 2026-08-29 (no code landed; the sections above
are already corrected):

1. The "Required Plan 013 contract" excerpt was wrong in three places
   that would have STOPped Step 4: the confirmation summary shape
   (`StatementClassSummary` cannot carry statement text — a `ddl`
   subject is now in scope), `createMaterializedView { orReplace }`
   (does not exist in PostgreSQL — matview edit is drop + create), and
   Rename on every kind (`renameObject` covers five kinds). Also
   `DdlApplyResult` was undefined, `describeUnsupported` no longer
   exists, and drop impact is now transitive with `depth`.
2. "Refresh (existing `refresh_materialized_view` path)" assumed a
   frontend caller; none exists. This plan builds it as a declared
   legacy-path exception.
3. Restored object tabs regressed Plan 010's disconnected-restore
   contract; the viewer now follows the `awaitingConnection` pattern.
4. The connect/disconnect race in `connectConnection` was inherited
   unaddressed; the slice now carries a generation guard.
5. Catalog load failure was invisible; the navigator now renders it
   with Retry.
6. Refresh ran only on success and reloaded the whole catalog for
   every apply; now per-op-kind and also on partial failure.
7. Navigator breadth was unbounded; per-group cap + server truncation
   note. Group expand state now has an explicit encoding beside
   `expandedSchemas`.
8. Views activated differently from navigator vs palette; now
   consistent (`open-relation` primary, viewer secondary).
9. Step 3 shipped actions with no dialog to route to; now read-only.
   `tab-shortcuts.tsx` icon ternary, `schema: ""` for schema-kind
   tabs, and apply double-submit were unaddressed; now covered.
10. §6 structure-editor switchover moved to Plan 015 with its store
    type decision made there. The legacy PG explorer arm deletion is
    scheduled here (Step 6) instead of living on indefinitely.
11. Stale refs: `plans/README.md:55` → `:94`; ROADMAP/PENDING_TASKS
    contain nothing to update; `command-palette.tsx` path given.
