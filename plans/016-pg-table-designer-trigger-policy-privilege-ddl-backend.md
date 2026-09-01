# Plan 016: PostgreSQL table designer, routine, trigger, policy, and privilege DDL backend (dark)

> **Executor instructions**: Do not start until Plan 015 is `DONE` in
> `plans/README.md`. Follow this plan step by step. This plan is dark:
> it adds operations, facts, fixtures, and tests behind the existing
> `preview_object_ddl` / `apply_object_ddl` / `describe_pg_object` /
> `fetch_table_structure` commands and changes no user-visible surface.
> Run every verification command and confirm the expected result before
> moving on. Update this plan's README row after each step and mark
> `READY FOR REVIEW` after all gates. A reviewer/operator records
> `DONE: <completion SHA>` after an authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat 84112dc..HEAD -- src src-tauri plans/README.md
> git status --short -- src src-tauri plans/README.md
> awk '/^pub\(crate\) enum PgObjectOp/,/^}/' src-tauri/src/postgres/object_ddl.rs | grep -cE '^    [A-Z][A-Za-z]* \{'   # expect 24 at b82de63
> grep -n "PgObjectFacts::Table," src-tauri/src/postgres/objects.rs        # expect the unit-variant table facts
> grep -n "fn lex_dollar" src-tauri/src/postgres/sql_lex.rs               # expect the tagged dollar-quote lexer
> ```
>
> Expected on a fresh run: no `src` or `src-tauri` output from the first
> two commands. A load-bearing mismatch with the excerpts below is a STOP
> condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MEDIUM-HIGH (this adds the first typed operations that change
  *who can read data* — grants, revokes, row-level security, and policy
  drops — and the first operation that carries an opaque routine body;
  a rendering or classification mistake here is a privilege or
  code-injection defect, not a cosmetic one)
- **Depends on**: Plans 013, 014, and 015 complete
- **Category**: direction
- **Planned at**: commit `b82de63`, 2026-09-01
- **Gap**: `PAR-007` in `plans/parity-gap-register.md`

## Why this matters

After Plan 015 every `ALTER TABLE` a user can express from the structure
editor runs through the typed preview → review → gated apply workflow,
but the three table-scoped surfaces users touch next still stop at
"generate SQL, open in the editor": the GRANT, row-level-security, and
trigger panels in `src/components/table-editor/specialized-editors.tsx`
(`:196-225`, `:310-329`). dbunk also cannot create a table at all — the
navigator's create menu covers schemas, sequences, enums, views, and
materialized views (`object-ddl/create-object-dialogs.tsx:95`) and
nothing else — and cannot create or edit a function or procedure without
hand-writing the whole `CREATE OR REPLACE`. These are the register's
first three `PAR-007` missing pieces, and they are all the same shape:
a table-scoped or routine-scoped `CREATE`/`ALTER` that the existing
operation union can express one statement at a time.

This plan extends the backend vocabulary and the table/routine facts so
Plan 017 can activate a create-table designer, a routine editor, and
typed trigger/policy/privilege sections without moving SQL generation
into a rendering path. Everything remains PostgreSQL-only per ADR-0001
and ADR-0026.

## Required Plan 013/014/015 contract

A mismatch is a STOP condition:

- `src-tauri/src/postgres/object_ddl.rs`: `PgObjectOp` is a
  `#[serde(tag = "op")]` union of 24 variants (`:86-247`);
  `generate_object_ddl` validates each op, renders one `RenderedOp`
  (`sql`, `summary`, `destructive`, `transactional`), ORs in
  `fragment_is_destructive` over `fragments(op)`, and rejects any
  rendering whose `classify_script` is not exactly one
  `StatementClass::Ddl` (`:367-397`); `group_statements` folds
  contiguous transactional statements into `Atomic` groups
  (`:413-436`); `validate_fragment` has `Embedded`,
  `IdentityArguments`, and `StatementBody` contexts (`:501-505`);
  `NewColumnSpec { name, data_type, nullable, default:
  Option<PgDefaultValue> }` (`:19-26`); `PgReferentialAction`
  (`:28-36`); `render_constraint_add` (`:1669`); `qualified`,
  `render_ident_list`, `quote_double`, `quote_literal` helpers.
- `src-tauri/src/postgres/objects.rs`: `PgObjectKind` (`:11-27`),
  `PgObjectRef` (`:39-46`), `PgObjectCatalog` with database-scoped
  list-only `roles` (`:104-110`, `:217-229`), `describe_routine`
  (`:1197-1251`) returning `PgObjectFacts::Routine { language,
  returns, volatility, arguments }` (`:2028-2033`), and
  `describe_relation` returning unit `PgObjectFacts::Table` (`:832`).
- `src-tauri/src/postgres/sql_lex.rs`: `lex_dollar` (`:146`) lexes a
  tagged dollar-quoted string as one token.
- `src-tauri/src/postgres/sql_class.rs`: `GRANT`, `REVOKE`, `CREATE`,
  `ALTER`, and `SECURITY` classify as non-destructive DDL and `DROP` as
  destructive DDL (`:191-193`).
- `src-tauri/src/postgres/schema.rs::fetch_table_structure` (`:42`)
  builds the PostgreSQL `TableStructure` on the SQLx pool and already
  reads user triggers for the Trigger Indicator (`:491-505`, filtering
  `tgisinternal` and `tgparentid`).
- `src-tauri/src/commands/pg_objects.rs`: `preview_object_ddl_inner`
  reads only the persisted record (`:93-108`); `apply_object_ddl_inner`
  regenerates from ops and authorizes through `WriteIntent::Ddl`
  (`:118-146`).
- Live tests in `objects.rs` use `test_connection` on
  `DBUNK_OBJECT_TEST_PORT` (default 15432), `#[serial_test::serial]`,
  and `#[ignore = "requires pnpm db:postgres"]` (`:2085-2110`,
  `:2194-2197`).
- Fixture `infrastructure/test-db/postgres/004_lifecycle_objects.sql`
  provides `lifecycle.orders`, `lifecycle.add_nums` overloads,
  `lifecycle.bump_orders`, and `lifecycle.order_status`; the only
  fixture role is `dbunk_cert` (`003_tls_roles.sql:5`).

## Decided architecture

### 1. Operation vocabulary

Eleven variants join `PgObjectOp`. Every variant renders exactly one
statement; multi-statement intents (a table with comments and indexes,
a trigger with its function) are expressed as several ops that the
existing grouping makes atomic.

**Relations**

- `createTable { schema, name, columns: Vec<NewColumnSpec>,
  primary_key: Option<PgKeySpec>, uniques: Vec<PgKeySpec>, checks:
  Vec<PgCheckSpec>, foreign_keys: Vec<PgForeignKeySpec>, unlogged:
  bool, if_not_exists: bool }` with `PgKeySpec { name: Option<String>,
  columns: Vec<String> }`, `PgCheckSpec { name: Option<String>,
  expression: String }`, and `PgForeignKeySpec` carrying the
  `addForeignKey` fields minus `schema`/`table`/`not_valid`.
  `NewColumnSpec` gains `identity: Option<PgIdentity>` (`Always |
  ByDefault`, `#[serde(default)]`, so `addColumn` accepts it too and
  every existing payload still decodes). Validation: at least one
  column; column names unique; every constraint column names a declared
  column; identity columns are non-nullable and default-free; FK
  local/referenced column counts match. Comments, indexes, and
  partitioning are **not** fields — the designer emits `setComment`
  and `createIndex` ops after the `createTable`, and partitioning is
  deferred (§Reconciliation). Summary: `Create table lifecycle.orders
  (5 columns, 3 constraints)`.

**Routines**

- `createFunction { schema, name, or_replace: bool, arguments: String,
  returns: String, language: String, body: String, volatility:
  PgVolatility, strict: bool, security_definer: bool, parallel:
  Option<PgParallelSafety> }` and `createProcedure { schema, name,
  or_replace, arguments, language, body, security_definer }`.
  `arguments` and `returns` are fragments validated in a new
  `FragmentContext::RoutineSignature` (paren-balanced, no statement
  boundary, no `AS`/`LANGUAGE`/`$` tokens; `returns` additionally
  accepts a leading `SETOF` or a `TABLE (…)` form). `language` must lex
  as one identifier. `body` is **opaque**: it is rendered inside a
  renderer-chosen dollar quote whose tag is the first of `$dbunk$`,
  `$dbunk1$`, `$dbunk2$`, … that does not occur in the body, so no
  body content can close the string; the finished statement still has
  to pass the existing exactly-one-DDL classification, which is what
  proves the lexer saw one token. "Alter" is `or_replace: true`; a
  signature change PostgreSQL refuses under `OR REPLACE` surfaces as
  the existing typed `database` error with its SQLSTATE. Summary:
  `Create or replace function lifecycle.order_total(integer)`.

**Triggers**

- `createTrigger { schema, table, name, timing: PgTriggerTiming
  (Before | After | InsteadOf), events: Vec<PgTriggerEvent>
  (Insert | Update { columns: Vec<String> } | Delete | Truncate),
  for_each: PgTriggerLevel (Row | Statement), when: Option<String>,
  function_schema, function_name, arguments: Vec<String>, or_replace:
  bool }`. Validation: at least one event, each event at most once,
  `InsteadOf` requires `Row` and forbids `when` and `Truncate`,
  `Truncate` requires `Statement`. `when` is an `Embedded` fragment;
  `arguments` render as literals. The trigger's function is a separate
  `createFunction` op with `returns: "trigger"` when the user is
  creating it inline.
- `dropTrigger { schema, table, name, cascade: bool }`.
- `setTriggerEnabled { schema, table, name, mode: PgTriggerMode
  (Enable | Disable | EnableReplica | EnableAlways) }` →
  `ALTER TABLE … {ENABLE|DISABLE|ENABLE REPLICA|ENABLE ALWAYS} TRIGGER …`.

**Row-level security**

- `setRowLevelSecurity { schema, table, enabled: bool, force:
  Option<bool> }` → one `ALTER TABLE` with comma-joined actions
  (`ENABLE ROW LEVEL SECURITY, FORCE ROW LEVEL SECURITY`).
- `createPolicy { schema, table, name, permissive: bool, command:
  PgPolicyCommand (All | Select | Insert | Update | Delete), roles:
  Vec<PgGrantee>, using: Option<String>, with_check: Option<String> }`
  with `PgGrantee = Public | Role { name }`. Validation: `Insert`
  forbids `using`; `Select` and `Delete` forbid `with_check`; at least
  one role (`Public` counts). `using` / `with_check` are `Embedded`
  fragments.
- `dropPolicy { schema, table, name }`. Editing a policy is
  `dropPolicy` + `createPolicy` in one atomic group; `ALTER POLICY` is
  deferred.

**Privileges**

- `grantPrivileges { target: PgObjectRef, privileges:
  Vec<PgPrivilege>, all_privileges: bool, grantee: PgGrantee,
  with_grant_option: bool }` and `revokePrivileges { target,
  privileges, all_privileges, grantee, grant_option_for: bool,
  cascade: bool }`. `target.kind` may be `table`, `view`,
  `materialized-view`, `foreign-table` (rendered `ON TABLE`),
  `sequence`, `schema`, `function`, or `procedure` (rendered with the
  identity arguments, reusing `render_object_identity`). `PgPrivilege`
  is the closed set `Select | Insert | Update | Delete | Truncate |
  References | Trigger | Usage | Create | Execute | Maintain`, and
  validation rejects a privilege the target kind cannot carry
  (`Execute` only on routines, `Usage` on sequences and schemas,
  `Create` on schemas, `Maintain` on relations). `all_privileges`
  requires an empty `privileges` list and vice versa.

### 2. Destructiveness beyond the classifier

`sql_class` marks `REVOKE` and `ALTER` as non-destructive, which is
correct for the query editor's mistakes-not-adversaries model but wrong
for review: a revoke or a disabled trigger can break a running
application as surely as a drop. The renderer sets `destructive: true`
for `revokePrivileges`, `setTriggerEnabled { Disable }`,
`setRowLevelSecurity { enabled: false }`, and `dropPolicy`, so the
safety gate's typed-confirm path and the preview's destructive badge
both engage. `setRowLevelSecurity { force: Some(false) }` is not
destructive on its own. This rule is recorded in ADR-0027.

### 3. Facts: routines and table security

- `PgObjectFacts::Routine` gains `body: Option<String>` (`prosrc`),
  `strict: bool`, `security_definer: bool`, and `parallel:
  Option<String>` (`safe | restricted | unsafe`, `None` for
  aggregates). `arguments` stays the `pg_get_function_arguments` text
  and `returns` the `pg_get_function_result` text, which is exactly
  what `createFunction` accepts back, so Plan 017's editor round-trips
  without parsing. For C or internal-language routines `body` is the
  symbol name and the frontend shows it read-only.
- `TableStructure` (PostgreSQL only) gains `triggers:
  Vec<TriggerInfo>` (name, timing, events with update columns, level,
  enabled state, function schema/name, `when` expression via
  `pg_get_triggerdef` parsing is **not** attempted — `when` ships as
  the raw definition string for display), `policies: Vec<PolicyInfo>`
  (name, permissive, command, roles, `using`, `with_check` from
  `pg_policies`), `privileges: Vec<PrivilegeInfo>` (grantee,
  privilege, grantable from `aclexplode(relacl)`, `PUBLIC` for grantee
  oid 0), and `row_security: Option<RowSecurityInfo { enabled, forced
  }>` from `relrowsecurity` / `relforcerowsecurity`. `Structure
  Capabilities` gains `triggers`, `policies`, and `privileges` flags,
  `true` only on PostgreSQL. The trigger query reuses the existing
  `tgisinternal` / `tgparentid = 0` filter from the Trigger Indicator
  loader and stays on the SQLx pool like the rest of
  `fetch_table_structure`.

### 4. Apply, grouping, safety

Nothing changes in `apply_object_ddl_inner`, `group_statements`, or
`authorize_object_ddl`. All eleven operations are transactional; a
designer batch of `createTable` + `setComment`… + `createIndex {
concurrently: false }` forms one atomic group, and a trailing
concurrent index forms its own standalone group exactly as today. The
`ddl` confirmation subject lists the new summaries unchanged.
`preview_object_ddl` stays pure: routine and privilege validation never
consult the catalog, so a grant to a role that does not exist, or a
trigger on a function that does not exist, is a typed `database` error
at apply time, not a preview-time lookup.

### 5. Fixture and live coverage

`infrastructure/test-db/postgres/005_table_security.sql` adds: role
`lifecycle_reader` (`NOLOGIN`) with `SELECT` on `lifecycle.orders`;
`lifecycle.touch_orders()` (`RETURNS trigger`, plpgsql) and trigger
`orders_touch` (`BEFORE UPDATE … FOR EACH ROW`) on `lifecycle.orders`;
table `lifecycle.tenant_rows (id serial primary key, tenant text not
null, note text)` with row-level security enabled and policy
`tenant_isolation` (`FOR SELECT TO PUBLIC USING (tenant =
current_setting('app.tenant', true))`); and `lifecycle.order_total
(integer)` (plpgsql, `STABLE`) for source round-trip tests. Fixture
objects the existing live tests assert against are not modified.

### 6. Documentation

- ADR-0027 "Table-scoped security and routine DDL": opaque
  dollar-quoted routine bodies and the tag-selection rule; the
  renderer-owned destructiveness rule from §2; triggers and policies as
  table-scoped structure entries (not Object Refs) with the promotion
  path if a viewer is ever needed; the closed privilege set and
  per-kind validity; the deferrals in §Reconciliation.
- ADR-0026: one "Related" pointer to ADR-0027.
- `CONTEXT.md`: **Table Structure** lists triggers, policies,
  privileges, and row-security state for PostgreSQL; new **Routine
  Source** entry (the `body` / `arguments` / `returns` facts that the
  editor round-trips). The **Object Operation** vocabulary note points
  at ADR-0027.
- Register `PAR-007`: "Progress (Plan 016)" paragraph naming the dark
  vocabulary; no missing-piece bullet is removed until Plan 017 lands.

## Reconciliation with the register's missing pieces

Deferred from this pair, with rationale, so the register stays honest:

- **Database lifecycle** (`CREATE`/`DROP`/`ALTER DATABASE`). dbunk
  connections are per-database; a created database is not reachable
  from the connection that created it, `CREATE DATABASE` cannot run in
  a transaction, and the navigator has no server-level node to hang the
  action on. It needs its own small plan with a server-scope decision.
- **Aggregates** create/alter, `ALTER FUNCTION` attribute-only changes,
  and `ALTER POLICY`: `OR REPLACE` and drop+create cover the daily
  cases; these are additive ops later.
- **Roles, memberships, ownership, default privileges, tablespaces,
  event triggers, rules, partitions, extensions, non-enum types and
  domains**: each needs catalog and describe work before DDL (ADR-0026
  promotion rule) and none is table-scoped; they follow as their own
  slices.
- **DDL cancellation**: unchanged deferral from ADR-0026.

## Commands you will need

```sh
just fmt && just lint && just test
pnpm db:postgres
DBUNK_OBJECT_TEST_PORT=15432 cargo test --manifest-path src-tauri/Cargo.toml -- --ignored postgres::object_ddl postgres::objects postgres::schema
pnpm format && pnpm lint && pnpm typecheck && pnpm vitest run   # TS mirrors only
```

## Scope

Expected files touched: `src-tauri/src/postgres/object_ddl.rs`
(variants, validation, rendering, summaries, unit tests, live tests),
`src-tauri/src/postgres/objects.rs` (routine facts + live test),
`src-tauri/src/postgres/schema.rs` (structure sections + live test),
`src-tauri/src/types.rs` (`StructureCapabilities` `:1862`, `TableStructure`
`:1885`) and the non-PostgreSQL constructors (empty vectors, `false` flags),
`src/lib/store/types.ts` (TS mirrors of every new op, fact, and
structure field — dark, no consumer), `docs/adr/0027-…`, ADR-0026,
`CONTEXT.md`, the fixture, the register, `plans/README.md`.

Out of scope: any frontend component, store slice, or command surface
(Plan 017); `execute_ddl`; ClickHouse/MySQL/SQLite structure loaders
beyond the empty-field constructors; everything in §Reconciliation.

## Resume protocol

Each step ends with all Rust gates green; re-run the step's
verification on resume. Live tests are `#[ignore]` and run explicitly
against `pnpm db:postgres`.

## Git workflow

Working tree only; no commits/pushes/PRs without explicit operator
authorization.

## Steps

### Step 1: Relation and routine operations

`createTable`, `createFunction`, `createProcedure`; `PgIdentity` on
`NewColumnSpec`; `FragmentContext::RoutineSignature`; the dollar-tag
chooser. Unit tests: rendering of a full designer batch (columns with
literal/expression defaults and an identity column, PK, unique, FK,
check, `UNLOGGED`, `IF NOT EXISTS`); every validation rule in §1
returns `invalidOp` naming the op index; a body containing `$dbunk$`
gets `$dbunk1$`; a body containing a statement boundary or a closing
tag attempt still yields exactly one DDL statement; `returns`
`SETOF`/`TABLE (…)` forms pass and `RETURNS int AS $$x$$` fails; an
`arguments` fragment with `LANGUAGE` fails; grouping of `createTable`
+ `setComment` + `createIndex { concurrently: true }` yields one atomic
group then one standalone. TS mirrors added. Gates: `just fmt`, `just
lint`, `just test`, `pnpm typecheck`.

### Step 2: Trigger, policy, and privilege operations

The remaining eight variants, `PgGrantee`, `PgPrivilege`, per-kind
privilege validity, and the §2 destructiveness rule. Unit tests: every
rendering in §1 including `ON TABLE` for the four relation kinds and
identity-argument rendering for routines; every validation rule; the
destructive flag on revoke / disable trigger / disable RLS / drop
policy and its absence on grant / enable / create policy; `PUBLIC` is
never quoted and a role named `public` is quoted; policy roles list
rendering. TS mirrors added. Gates as Step 1.

### Step 3: Facts and structure sections

§3 in `objects.rs` and `schema.rs`, capabilities flags, empty
constructors for the other engines, TS mirrors. Unit tests: decoding
of the extended facts; capability flags `false` off PostgreSQL. Gates
as Step 1 plus `pnpm vitest run` (existing structure tests must pass
unchanged with the new optional fields).

### Step 4: Fixture, live tests, docs

§5 fixture; live tests (ignored, serial): designer batch creates
`lifecycle.designer_demo` with comments and a concurrent index and
`describe_pg_object` reconstructs it; `createFunction { or_replace }`
changes `lifecycle.order_total` and the described `body` reflects it;
trigger create → disable → structure shows `disabled` → drop; RLS
enable + policy → structure shows both → drop policy; grant to
`lifecycle_reader` then revoke round-trips through `privileges`; a
grant to a nonexistent role is a typed `database` error with SQLSTATE
`42704`; a policy `Insert` with `using` is rejected at preview. Each
live test cleans up in a `finally`-style block so reruns are
idempotent. Then §6 docs and the full gate set. Mark `READY FOR
REVIEW`.

## Test plan

Steps 1–3 are pure unit coverage with no database. Step 4 runs against
`pnpm db:postgres`. No frontend behaviour changes, so no UI test
changes are expected beyond type-level additions.

## Done criteria

- The eleven operations preview and apply through the unchanged
  commands with one statement each, honest grouping, and the §2
  destructiveness rule.
- Routine facts round-trip `arguments` / `returns` / `body` into
  `createFunction` without parsing.
- PostgreSQL `TableStructure` reports triggers, policies, privileges,
  and row-security state; other engines report empty sections with
  `false` capabilities.
- ADR-0027, ADR-0026 pointer, CONTEXT, register, and fixture match
  reality.
- All gates green; live tests pass against the fixture.

## STOP conditions

- Plan 013/014/015 contract mismatch, including `lex_dollar` not
  lexing a tagged dollar string as one token.
- Any new operation that would need more than one statement, a
  frontend-supplied statement, or a preview-time catalog lookup.
- A change to `apply_object_ddl_inner`, `group_statements`, or the
  safety authorization path.
- Any existing Plan 013–015 test needing modification to stay green.

## Maintenance notes

- Adding a privilege or target kind is a `PgPrivilege` / validity-table
  edit plus a rendering test; never widen `target.kind` past what
  `render_object_identity` can name.
- The dollar-tag chooser must stay deterministic and body-derived; a
  random tag would make preview and apply render different SQL, which
  breaks the review-integrity rule.
- Triggers and policies are structure entries, not Object Refs. If a
  viewer or Open Anything reach is ever wanted, promote them through
  the ADR-0026 catalog/describe/DDL rule rather than special-casing.

## Execution record (2026-09-02)

Steps 1–4 implemented in the working tree on top of `b82de63`; all gates
green (`just fmt` / `lint` / `test`, the eight ignored live tests against
`pnpm db:postgres`, `pnpm format` / `lint` / `typecheck`, vitest,
`check:ui-gates`, `check:slice-isolation`). Deviations from the plan text,
none contract-changing:

- **Policy role lists collapse.** PostgreSQL stores a policy whose role list
  names `PUBLIC` as `{public}` alone, discarding the other roles. The live
  test and ADR-0027 record this; the frontend must not expect the extra
  roles back and should treat `public` as the pseudo-role when re-creating.
- **Routine identity arguments carry parameter names.**
  `pg_get_function_identity_arguments` reports `order_id integer`, not
  `integer`; the live tests use the former.
- **Existing `NewColumnSpec` test literals gained `identity: None`.** The
  field is additive and `#[serde(default)]`, so no behaviour or wire change
  is involved; Rust struct literals are exhaustive, so the thirteen existing
  Plan 013–015 test literals and one command-test literal had to name the
  new field. This is the only edit to pre-existing tests and is recorded
  here as the STOP-condition deviation it technically is.
- **Signature validation is stricter than the plan's minimum.** Besides
  `AS` / `LANGUAGE` and the dollar-sign rule, top-level `RETURNS`, `BEGIN`,
  `ATOMIC`, `WINDOW`, volatility words, `LEAKPROOF`, `CALLED`, `STRICT`,
  `SECURITY`, `PARALLEL`, `COST`, `ROWS`, `SUPPORT`, `SET`, `TRANSFORM`, and
  `WITH` are refused in `arguments` / `returns`; `returns` additionally
  refuses literals and top-level commas.
- **Privilege duplicates are refused** in addition to the per-kind validity
  check, so a preview cannot render `SELECT, SELECT`.
- **Fixture trigger is on `lifecycle.orders` as planned**; the existing
  drop-impact live test still passes because a trigger is an auto
  dependency of its table and is not reported as a CASCADE dependent.

## Review correction record (2026-09-02, second pass)

An adversarial review of the first delivery found five plan gaps and four
structural blockers; all are addressed in the working tree:

- **Predicate-free policies were rejected.** An undocumented rule required
  `USING` or `WITH CHECK`; PostgreSQL's grammar makes both optional. The
  rule and its test are gone; a test now pins `CREATE POLICY … FOR ALL TO
  PUBLIC;`.
- **Live-test cleanup was not failure-safe.** The single 470-line test is
  replaced by five domain tests under `commands/pg_objects_live_tests/`
  (table, routine, trigger, policy, privilege, plus the Plan 013 apply test
  moved verbatim). Each owns disposable objects, resets before running, runs
  its body under `catch_unwind`, resets again, and only then re-raises.
  The routine test creates its own `lifecycle.live_total` instead of
  replacing the fixture routine.
- **Signature boundary.** The plan's "no `$` tokens" is now literal: any
  dollar sign in `arguments` / `returns` is refused. Quoted `LANGUAGE`
  identifiers are accepted and rendered as typed, matching the plan's "one
  identifier".
- **Privilege introspection** reads `aclexplode(relacl)` only; a `NULL` ACL
  reports an empty list. Plan, ADR-0027, and CONTEXT agree on "explicit".
- **Every validation test pins the operation index** through
  `sole_invalid`, and one multi-op case pins index 2.
- **Generator split into domain modules.** `PgObjectOp` variants are newtypes
  over payload structs; each domain module owns `validate`, `fragments`, and
  `render` through the `ObjectOperation` trait, dispatched by an exhaustive
  match with no wildcard. The wire format is pinned by a serde round-trip
  test. The largest production module is under 500 lines; unit tests are
  split per domain under `object_ddl/tests/`.
- **`pg_objects.rs` is back to 564 lines**; live tests live in their own
  directory module.
- **Security metadata decoding fails closed.** Trigger, policy, privilege,
  and row-security rows decode through `sqlx::FromRow` records; `tgenabled`,
  policy command, and permissive/restrictive codes decode into closed enums
  (`TriggerEnabledState`, `PolicyCommand`) and unknown values are errors.
- **TypeScript keeps the contract required.** The structure and routine
  fields are required in `types.ts`; `src/lib/table-structure-contract.ts`
  normalizes a payload once at the two invoke boundaries, and the
  twenty-five test fixtures were updated to the required shape.
