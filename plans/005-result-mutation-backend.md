# Plan 005: Add the PostgreSQL Result Mutation backend

> **Executor instructions**: Do not start until Plans 003 and 004 are `DONE` in
> `plans/README.md`. Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report without improvising. Keep the feature dark: Plan 006
> owns frontend activation, and `commit_cell_edits`, `insert_row`, and
> `delete_rows` remain the live mutation path for every engine until then.
> Update this plan's row in `plans/README.md` to `IN PROGRESS: through Step N`
> after each completed step and to `READY FOR REVIEW` after all gates pass. A
> reviewer/operator records `DONE: <completion SHA>` only after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat ecefce8..HEAD -- CONTEXT.md docs/adr src-tauri plans/README.md
> git status --short -- CONTEXT.md docs/adr src-tauri plans/README.md
> ```
>
> Expected on a fresh run: no source/config output; advisor-authored artifacts
> under `plans/` may be untracked or modified. If resuming, follow "Resume
> protocol" instead. A load-bearing mismatch with the excerpts below is a STOP
> condition.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: Plan 003 (`202f756`) and Plan 004 (`ecefce8`) complete
- **Category**: direction
- **Planned at**: commit `ecefce8`, 2026-08-21
- **Gap**: `PAR-003` in `plans/parity-gap-register.md`

## Why this matters

Query results render an editable grid and buffer edits, but nothing can commit
them: `queryEdits` has no commit action, no identity source, and the toolbar
Save button has no handler. Table-tab edits commit through positional row/column
indexes with no stored originals, so the only staleness signal is a formatted
`"row not found"` string that rolls back the whole batch with no per-change
attribution. Nothing anywhere previews the DML a commit will run, nothing knows
which columns are generated or identity columns, keyless tables are read-only
with no recourse, and inserts and deletes are immediate one-shot commands that
can never be reviewed before they run.

This plan lands a dark, typed, PostgreSQL-only Result Mutation backend: an
updatability analysis that recovers column origins for arbitrary result sets
through the extended query protocol, a catalog descriptor that knows generated
and identity columns, persisted user-selected virtual keys for keyless
relations, a pure parameterized DML builder with per-operation preview, and a
transactional apply with per-operation conflict detection and an all-or-nothing
failure policy. Plan 006 activates it as one staged mutation model shared by
table tabs and query results.

## Current state

### Repository constraints

- PostgreSQL is the reference engine per
  `docs/adr/0001-postgres-first-engine-coverage.md`. This plan must not broaden
  into MySQL, SQLite, or ClickHouse; those engines stay on the legacy mutation
  commands.
- `docs/adr/0004-last-activity-on-connection-record.md`: record activity after
  a successful operation only.
- `docs/adr/0013-postgres-driver-fields-on-connection-record.md`: effective
  timeouts, search path, and role apply to every PostgreSQL connection. The
  mutation connection must apply the same resolved driver options.
- `docs/adr/0021-dedicated-postgres-query-session-driver.md` and
  `docs/adr/0022-server-backed-table-browse-contract.md`: dedicated
  tokio-postgres sockets exist outside the five-slot SQLx pool, built through
  `postgres::dedicated` with TLS parity, `CancelToken` cancellation, and
  teardown fencing through `socket_lifecycle.rs`. Reuse that infrastructure;
  do not fork it.
- Never log SQL, row values, parameter values, or structured database error
  detail. `tokio_postgres = Warn` filtering is a security invariant.
- CI runs `cargo test` with no database service. Every generated-SQL behavior
  must be proven by exact-string unit tests on pure builders; live behavior is
  covered by `--ignored` tests against the local fixture.

### Backend evidence

`src-tauri/src/postgres/mutations.rs:167-217` is today's cell-edit commit: one
SQLx transaction over `build_update` per edit, with this as the only
staleness signal (`mutations.rs:194-202`):

```rust
let affected = result.rows_affected();
if affected == 0 {
    let identity_desc = edit.identity.iter()
        .map(|kv| format!("{}={}", kv.column, kv.value.as_deref().unwrap_or("NULL")))
        .collect::<Vec<_>>().join(", ");
    return Err(format!("row not found: {}", identity_desc));
}
```

It is a `Result<_, String>` with no per-edit index, it embeds identity values
in an error string, and it cannot distinguish a concurrent change from a wrong
key. `build_update`/`build_insert`/`build_delete` (`mutations.rs:18-161`) are
the parameterized-builder precedent: `(String, Vec<Option<String>>)`, `$N`
placeholders, `quote_double` identifiers, `IS NULL` identity rendered without
a parameter.

The query session executes through the simple query protocol
(`src-tauri/src/query_session/postgres.rs:137`), whose `SimpleColumn` exposes
only `name()`. `ResultSetStarted` therefore carries
`columns: Vec<Option<String>>` and nothing else
(`src-tauri/src/query_session/protocol.rs:183-186`) — no types, no table OIDs,
no attnums. The extended protocol's `tokio_postgres::Column` (tokio-postgres
`0.7.18`, pinned in `src-tauri/Cargo.toml`) exposes `table_oid() ->
Option<u32>` and `column_id() -> Option<i16>`, so column origins are
recoverable by preparing the statement — without executing it — on an
extended-protocol socket.

The Table Browse descriptor (`src-tauri/src/table_browse/postgres.rs:75-99`)
reads only `attname`, `format_type`, and `attnotnull`. It never reads
`attgenerated`, `attidentity`, or `atthasdef`, so the browse contract cannot
tell a writable column from a stored generated column. Identity resolution
(`src-tauri/src/table_browse/builder.rs:41-77`) is the authoritative rule from
ADR-0022: primary key → smallest qualifying unique index → virtual
`ctid`/`(tableoid, ctid)` → `none`.

`src-tauri/src/socket_lifecycle.rs:11-129` is the teardown coordinator: fences
`QuerySessionManager` and `TableBrowseManager` together at every invalidation
site. `src-tauri/src/postgres/dedicated.rs:47-142` owns dedicated-socket
connect, cancel, and error mapping. `src-tauri/src/postgres/row_budget.rs`
owns the shared 1 MiB cell / 32 MiB response caps.

`src-tauri/src/table_browse/protocol.rs:262-276` already delivers
backend-authoritative `identity { kind, columns }` and per-row `rowIdentity`
aligned with rows — the metadata this plan's apply path consumes.

### Frontend evidence (context only; Plan 006 owns all frontend work)

- `src/lib/store/relational-queries.ts:61` — `queryEdits` buffers positional
  query-result edits with no commit action.
- `src/components/query-editor/toolbar.tsx:129-131` — the Save button renders
  with no `onClick`.
- `src/components/query-editor/results-view.tsx:98-101` — nulls flatten to
  `""` before the grid, so NULL is unrecoverable on the query-result path.
- `src/lib/store/edit-strategies.ts:49-58` — commit payloads carry identity
  and set values read from the currently loaded page; originals are never
  stored with the edit.

## Verified dependency and protocol facts

- `Parse`/`Describe` (statement preparation) never executes the statement. It
  is safe on any statement text, including SELECTs invoking volatile
  functions. Preparing a multi-statement string fails with SQLSTATE `42601`
  ("cannot insert multiple commands into a prepared statement") — this is the
  multi-statement detector, not string parsing.
- `Column::table_oid()` returns `Some` only for columns projected directly
  from a relation; expressions, aggregates, and literals return `None`. This
  is exactly the updatability boundary.
- `table_oid` resolves to `(schema, table)` through `pg_class` joined to
  `pg_namespace`; a positive `column_id` is the `attnum` in `pg_attribute`.
- Any non-empty `pg_attribute.attgenerated` marks a generated column that is
  never writable: `'s'` (stored) everywhere, and `'v'` (virtual) on
  PostgreSQL 18, where VIRTUAL is the default for new generated columns. The
  classifier must treat every non-empty value as generated, not just `'s'`;
  the fixture runs PostgreSQL 16, so PG-18 behavior cannot be live-tested and
  the classification must be structural. `attidentity = 'a'` marks
  GENERATED ALWAYS identity columns (not writable without
  `OVERRIDING SYSTEM VALUE`; excluded in v1). `attidentity = 'd'`
  (BY DEFAULT) and `atthasdef` columns are writable but should be omitted,
  not set to NULL, when the user provides no value.
- System columns can appear in projections: `SELECT ctid, xmin, * FROM t`
  yields columns with `table_oid` set and `column_id = Some(-1)` —
  tokio-postgres maps only attnum `0` to `None` and passes negatives
  through. Any `column_id <= 0` is classified as a non-writable system
  origin, never resolved against `pg_attribute`.
- The wire protocol's RowDescription carries table OID and attnum in **both**
  the simple and extended protocols; tokio-postgres merely discards them on
  the simple path (`SimpleColumn` exposes only `name()`). Capturing origins at
  execution time is blocked by the driver API, not the protocol — the ADR must
  state this accurately.
- **Name-resolution hazard**: the query session executes arbitrary SQL through
  the simple protocol, including `SET search_path`, `SET ROLE`, and
  `CREATE TEMP TABLE`. ADR-0013 option parity holds only at connect time. A
  session temp table **shadows** a permanent table of the same name for that
  session, and the session's temp schema is invisible to the mutation socket —
  so `Parse` on the mutation socket can silently resolve `users` to
  `public.users` while the session's result came from a temp scratch copy. If
  that copy was seeded from the real table, staged identity values and guards
  can match real rows by value and commit against the wrong relation. Analysis
  must therefore fail closed: `pg_class` exposes every session's `pg_temp_N`
  schemas, so the analyzer refuses (`notAnalyzable { reason:
  possibleTempShadowing }`) whenever any resolved origin relation's `relname`
  matches a live temp relation of any session. Runtime `SET search_path` drift
  is not detectable from the mutation socket; the mitigations are OID-based
  resolution, the shadowing refusal, and Plan 006's requirement that the
  review surface always display the fully qualified resolved target
  relations. The residual risk is documented in the ADR, not waved away.
- `IS NOT DISTINCT FROM` is null-safe but **not indexable**: the planner
  treats it as a `DistinctExpr`, never an index clause, so rendering identity
  predicates with it would force a sequential scan on every guarded
  operation. The null-safe *and* indexable rendering is value-dependent at
  build time: `"col" = ($N::text)::<cast>` when the old value is non-NULL,
  `"col" IS NULL` (no parameter) when it is NULL. A row whose current value
  went NULL then fails the `=` predicate and surfaces as `rows_affected ==
  0` — the correct conflict outcome.
- A concurrent-update conflict surfaces as `rows_affected == 0` on the guarded
  statement inside the apply transaction; `rows_affected > 1` means the
  claimed key is not actually unique (possible only for user-selected virtual
  keys). Both must abort the transaction.
- `SET LOCAL lock_timeout` inside the apply transaction bounds blocking on
  rows locked by other transactions (including a query tab's own open manual
  transaction on its separate socket); the timeout surfaces as SQLSTATE
  `55P03`, mapped to a typed error, and the transaction rolls back.

## Decided architecture

### Executor and ownership

1. Add `ResultMutationManager` to `AppState`. One lazily created Result
   Mutation Executor owns one dedicated read-write tokio-postgres connection
   per stored Connection, built through `postgres::dedicated::connect` with
   the same `ResolvedPostgresConnectSpec`, TLS, and driver options as query
   sessions. Unlike the browse socket it is **not** read-only — it exists to
   write. The legacy SQLx mutation commands remain live and unchanged for
   every engine until Plan 006 switches PostgreSQL over.
2. Admission: at most 1 mutation socket per Connection and 8 across the app,
   accounted separately from the query-session and browse budgets. An idle
   executor closes after 300 seconds. The socket serves three request kinds:
   analysis, preview validation, and apply.
3. **Analysis requests are supersedable; apply requests are not.** Analysis
   follows the browse supersession model: newest request per tab wins, older
   queued requests drop, in-flight analysis is protocol-cancelled, superseded
   commands resolve with the typed error `superseded`. Queue wait on the
   single per-Connection socket is bounded at 10 seconds before a typed
   `timeout`, matching browse. Apply is exclusive and deliberate: applies
   never queue — at most one may be in flight per Connection, and a second
   concurrent apply resolves `busy` immediately. An apply is never
   superseded — only an explicit cancel or teardown interrupts it, and
   either path rolls the transaction back. `cancel_result_mutation` for a
   tab cancels that tab's in-flight or queued analysis; a running apply is
   cancelable only by the tab that initiated it.
4. Teardown fencing goes through `socket_lifecycle.rs` exactly where Query
   Session and Table Browse are fenced today: Connection save, delete, and
   disconnect, bastion invalidation, managed stop/destroy/recreate,
   destructive credential reset, and app exit. The coordinator closes all
   three managers concurrently. An apply in flight during teardown is
   protocol-cancelled and its transaction rolls back; close is idempotent and
   bounded at 3 seconds. Storage-mode migration does not close executors.

### Mutation catalog descriptor

5. `result_mutation/postgres.rs` loads a `MutationDescriptor` per
   `(schema, table)`: columns in `attnum` order with name, `format_type` cast
   type, `attnotnull`, `attgenerated`, `attidentity`, and `atthasdef`; the
   primary key; and qualifying unique indexes under the ADR-0022 rule. The
   descriptor is cached per executor with the same invalidation contract as
   browse: SQLSTATE `42703`/`42P01` invalidates and retries once, and
   `refreshStructure: true` forces a reload.
6. The ADR-0022 identity resolution rule gets exactly one implementation:
   extract it from `table_browse/builder.rs:41-77` into
   `src-tauri/src/postgres/identity.rs` as a pure function over `(relkind,
   primary_key, unique_indexes)` — the version gate belongs to keyset
   paging, not identity assignment, and stays in the browse builder — with
   `table_browse::builder` delegating to it. This is the consolidation the
   Plan 003 maintenance notes required. Existing browse builder tests must
   pass unchanged; no browse behavior changes.
7. Column writability classification, derived per column: `writable`,
   `generated` (any non-empty `attgenerated` — `'s'` stored everywhere,
   `'v'` virtual on PostgreSQL 18), `identityAlways` (`attidentity = 'a'`),
   and `systemColumn` (projection `column_id <= 0`). Inserts omit columns
   the user left unset whenever the column has a default or identity
   (`atthasdef` or `attidentity` set), letting the database apply
   `DEFAULT`; explicit NULL remains expressible.

### Updatability analysis

8. `analyze_result_set` takes a source, tagged by kind:
   - `statement { sql }` — the executed single-statement SQL text of a query
     result set. The executor prepares it (`Parse`/`Describe`, never
     executing), reads per-column `table_oid`/`column_id`, resolves origin
     tables and attnums against the catalog, loads a `MutationDescriptor` per
     origin table, and classifies.
   - `relation { schema, table }` — a table tab source. No preparation; the
     descriptor alone drives classification. This is how Plan 006 enriches
     the browse grid with writability flags without touching the browse
     contract.
9. The analysis response carries, per projection column: name, origin
   (`table { schema, table, column, attnum } | expression`), cast type,
   nullability, and writability. Per origin table: the resolved identity
   (`primaryKey | uniqueIndex | virtualKey | ctidFallback | none` with
   columns), whether every identity column is present in the projection, the
   projection indexes of those identity columns, and per-capability verdicts
   (`updatable`, `deletable`, `insertable`) each with a typed reason when
   false. A multi-table (join) result is analyzable: each origin table is
   classified independently, and updates are possible for any table whose
   identity is fully projected. Inserts and deletes require a single origin
   table.
10. Non-analyzable statements produce `notAnalyzable { reason }` with typed
    reasons: `multiStatement` (SQLSTATE `42601` from Parse of a
    multi-statement string), `noProjectedColumns`, `noTableOrigins`,
    `possibleTempShadowing`, or `database` details for anything the server
    rejects. The frontend never parses SQL to decide editability.
    The shadowing check runs after origin resolution for `statement`
    sources: if any resolved origin relation's `relname` matches a live
    temp relation in any `pg_temp_%` schema in `pg_class`, analysis refuses.
    This is deliberately conservative — another session's unrelated temp
    table also refuses — because the alternative failure mode is silent
    wrong-table DML. `relation` sources skip the check: the user named the
    relation explicitly.
11. Analysis results are cached per executor under a monotonically increasing
    `analysisId` (bounded LRU, 16 entries per Connection). Preview and apply
    reference an `analysisId`; a missing or evicted id resolves the typed
    error `analysisExpired`, and the caller re-analyzes. The cache empties on
    executor close and on structure invalidation.

### Virtual keys

12. When an origin table's resolved identity is `none` by the ADR-0022 rule
    — or when the resolved identity's columns are not all projected — the
    user may designate a projected column set as that table's **virtual
    key**, persisted per `(connection_id, schema, table_name)` in a new
    SQLite `virtual_keys` migration following the `table_grid_prefs` shape:
    a versioned JSON document holding the ordered column list. Commands:
    `load_virtual_key`, `save_virtual_key`, `clear_virtual_key`.
13. Analysis consults the stored virtual key only when it is needed (identity
    `none` or identity not fully projected) and validates that every virtual
    key column exists in the relation and is projected; a stale virtual key
    (dropped or renamed column) is reported as invalid with a typed reason
    and ignored for classification.
14. A virtual key is a user claim, not a proven constraint. Every operation
    keyed by a virtual key or by `ctid` carries **full-row guards**: the old
    values of every projected column of that table, rendered with the
    value-dependent null-safe scheme (`=` for non-NULL originals, `IS NULL`
    otherwise). `rows_affected > 1` on any guarded statement is the typed
    error `identityNotUnique` and aborts the transaction. `ctid` identity
    (`ctidFallback`) is usable for update and delete only — re-verified
    through full-row guards because CTIDs are reused by vacuum — and never
    for tables where the browse identity kind was `none`.

### Mutation plan, preview, and apply

15. `result_mutation/builder.rs` is a pure module: given the analysis
    snapshot's descriptors and a `MutationPlan`, it returns per-operation
    `(sql, params)` or a typed validation error. No I/O, fully
    unit-testable with exact-string assertions, following the
    `postgres/mutations.rs` and `table_browse/builder.rs` conventions. Every
    value binds as a `$N` text parameter cast as `($N::text)::<cast_type>`
    with the descriptor's `format_type` cast, matching ADR-0022; identifiers
    go through `quote_double`.
16. `MutationPlan` is an ordered `Vec<MutationOp>`, tagged by kind:
    - `update { table, identity: [{column, value}], guards: [{column,
      value}], set: [{column, value}] }` renders
      `UPDATE "s"."t" SET "c" = ($N::text)::<cast> WHERE <identity> AND
      <guards>`.
    - `delete { table, identity, guards }` renders the guarded `DELETE`.
    - `insert { table, values: [{column, value}] }` renders
      `INSERT INTO "s"."t" ("c", ...) VALUES (...)`; unset
      defaultable/identity columns are omitted from the column list. A
      duplicate-row operation is an `insert` built by the frontend from the
      source row minus generated and identity-always columns.
    Every identity and guard predicate renders value-dependently:
    `"col" = ($N::text)::<cast>` when the old value is non-NULL,
    `"col" IS NULL` without a parameter when it is NULL — null-safe and
    indexable, unlike `IS NOT DISTINCT FROM`, which the planner never
    matches to an index. `primaryKey`/`uniqueIndex` identity columns are
    all-non-nullable by the ADR-0022 qualifying rule, so a NULL identity
    value for those kinds is the typed error `invalidPlan`, not generated
    SQL. Guard rules by identity kind: `primaryKey` and `uniqueIndex`
    updates carry guards for the **edited columns'** old values;
    `primaryKey` and `uniqueIndex` deletes carry guards for **all
    projected columns'** old values (deleting destroys the whole row, so
    the whole visible row must still match); `virtualKey` and
    `ctidFallback` operations always carry full-row guards. This policy is
    deliberate and its limits are documented in the ADR and every user
    surface: a keyed update does **not** detect concurrent changes to
    columns the user did not edit, and value-identical (A→B→A) rewrites
    pass. Writing to a `generated`, `identityAlways`, or `systemColumn`
    column, unknown columns, empty `set`, empty identity, and
    guard/identity columns missing from the analysis are typed validation
    errors, never generated SQL.
17. `preview_result_mutations` runs the pure builder against the referenced
    analysis snapshot and returns, per operation, `{ opIndex, sql, params }`
    with params as the tagged `text` union — display-only, never logged,
    mirroring `BrowseInspection`. Preview performs no I/O beyond snapshot
    lookup and never touches the database.
18. `apply_result_mutations` executes the same built statements in one
    transaction on the executor socket: `BEGIN`, `SET LOCAL lock_timeout =
    '10s'`, each operation through the extended protocol in plan order, then
    `COMMIT`. Per-operation enforcement: `update`/`delete` must affect
    exactly 1 row (`0` → `conflict { opIndex }`, `>1` →
    `identityNotUnique { opIndex }`); `insert` must affect exactly 1. Any
    typed failure or database error rolls back the entire transaction —
    all-or-nothing is the partial-failure policy, and the response never
    claims partial success. The success response carries per-operation
    affected counts and `runtimeMs`. Apply is cancelable through the
    protocol `CancelToken` (`cancel_result_mutation`), which aborts the
    in-flight statement and rolls back; after any failed or cancelled apply
    the executor issues `ROLLBACK` and verifies the socket is idle before
    accepting the next request, reconnecting if the socket is poisoned.
19. Apply always reloads the descriptor and re-validates the plan against it
    before the transaction opens: renamed or dropped columns surface as
    typed validation errors before any statement runs, not as database
    errors mid-transaction.

## Wire contract

Rust DTOs live in `src-tauri/src/result_mutation/protocol.rs` with
`#[serde(rename_all = "camelCase")]` and `tag = "kind"` for unions, matching
the table-browse convention. Commands return
`Result<T, ResultMutationError>`; the legacy `Result<T, String>` convention
must not be used here.

Commands:

| Command | Payload | Result |
|---|---|---|
| `analyze_result_set` | `AnalyzeResultSetPayload` | `AnalyzeResultSetResult` |
| `preview_result_mutations` | `connectionId`, `tabId`, `analysisId`, `plan` | `PreviewResult` |
| `apply_result_mutations` | `connectionId`, `tabId`, `requestId`, `analysisId`, `plan` | `ApplyResult` |
| `cancel_result_mutation` | `connectionId`, `tabId` | `{ cancelRequested }` |
| `close_result_mutation_for_connection` | `connectionId` | `()` |
| `load_virtual_key` | `connectionId`, `schema`, `table` | `VirtualKey \| null` |
| `save_virtual_key` | `connectionId`, `schema`, `table`, `columns` | `()` |
| `clear_virtual_key` | `connectionId`, `schema`, `table` | `()` |

`AnalyzeResultSetPayload`: `connectionId`, `tabId`, `requestId`, `source`
(tagged `statement { sql }` or `relation { schema, table }`),
`refreshStructure`.

`AnalyzeResultSetResult`: `requestId`, `analysisId`, `columns:
AnalyzedColumn[]` (`name`, `origin` tagged `table { schema, table, column,
attnum }` / `expression`, `castType`, `nullable`, `writability` tagged
`writable` / `generated` / `identityAlways` / `systemColumn`),
`tables: AnalyzedTable[]`
(`schema`, `table`, `identity { kind: "primaryKey" | "uniqueIndex" |
"virtualKey" | "ctidFallback" | "none", columns }`, `identityProjected`,
`identityProjectionIndexes`, `updatable { allowed, reason? }`, `deletable
{ allowed, reason? }`, `insertable { allowed, reason? }`), `statement`
(tagged `analyzed` or `notAnalyzable { reason }`).

`PreviewResult`: `statements: { opIndex, sql, params: DmlParam[] }[]` with
`DmlParam` tagged `text { value }` — display only, never logged.

`ApplyResult`: `operations: { opIndex, rowsAffected }[]`, `runtimeMs`.

`ResultMutationError` tagged union: `unsupportedEngine`, `notAnalyzable`
with typed reason, `unknownColumn`, `invalidPlan` with reason,
`analysisExpired`, `conflict { opIndex }`, `identityNotUnique { opIndex }`,
`lockTimeout { opIndex }`, `busy`, `superseded`, `cancelled`,
`connectionClosing`, `connectionLost`, `timeout` with operation, `database`
with structured non-loggable display fields (`code`, `message`, `severity`,
`position`, optional `opIndex`).

`VirtualKey` is a versioned JSON document (`version`, ordered `columns`)
stored in a new SQLite migration `virtual_keys` with primary key
`(connection_id, schema, table_name)`, following the `table_grid_prefs`
storage shape. The backend validates shape on save and treats interpretation
as its own: analysis is the only consumer.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Rust format | `just fmt` | exit 0 |
| Rust lint | `just lint` | exit 0, no warnings |
| Rust tests | `just test` | all non-ignored tests pass |
| Plain fixture | `pnpm db:postgres` | healthy on port 15432 |
| TLS fixture | `pnpm db:postgres-tls` | healthy on port 15433 |
| Live tests | `cargo test --manifest-path src-tauri/Cargo.toml result_mutation_live -- --ignored --test-threads=1` | all pass |
| Diff hygiene | `git diff --check` | no output |

Never print environment variables, Connection records, SQL, or parameter
values.

## Scope

**In scope**:

- `CONTEXT.md` (Result Mutation vocabulary)
- `docs/adr/0023-staged-result-mutation-contract.md` (create)
- `src-tauri/src/lib.rs`, `src-tauri/src/types.rs` (registration only)
- `src-tauri/src/result_mutation/mod.rs` (create)
- `src-tauri/src/result_mutation/protocol.rs` (create)
- `src-tauri/src/result_mutation/builder.rs` (create)
- `src-tauri/src/result_mutation/postgres.rs` (create)
- `src-tauri/src/result_mutation/live.rs` (create)
- `src-tauri/src/postgres/identity.rs` (create; extraction only)
- `src-tauri/src/postgres/mod.rs` (register the `identity` module only)
- `src-tauri/src/table_browse/builder.rs` (delegate identity resolution to
  the extracted function; no behavior change)
- `src-tauri/src/postgres/dedicated.rs` (visibility changes only, if needed)
- `src-tauri/src/commands/mod.rs`, new `commands/result_mutation.rs`
- `src-tauri/src/socket_lifecycle.rs` (add the third manager to the fence)
- `src-tauri/src/commands/connections.rs`, `bastions.rs`, `managed.rs`,
  `settings.rs` (teardown fencing only)
- `src-tauri/src/storage.rs` (one new migration)
- `infrastructure/test-db/postgres/*.sql` (fixture tables: generated and
  identity columns, keyless table, non-unique candidate columns; additive
  only)
- `plans/README.md` status text only

**Out of scope**:

- All `src/**/*.ts` and `src/**/*.tsx`; Plan 006 owns activation
- Changing or removing `commit_cell_edits`, `insert_row`, `delete_rows`,
  `import_rows`, or any legacy caller behavior
- MySQL, SQLite, ClickHouse, or Redis mutation adapters
- `OVERRIDING SYSTEM VALUE` writes to identity-always columns
- Batch paste parsing, deep value editors, Quick Look, copy formats
- Safe Mode, environment classification, destructive-statement policy
  (`PAR-004`)
- Applying mutations inside a query tab's open manual transaction
- Commits, pushes, PRs, or publication without authorization

## Resume protocol

1. Read the `plans/README.md` status for Plan 005.
2. Inspect `git status --short` and `git diff -- <Scope paths>`.
3. Accept changes only when they match steps recorded as completed. Compare
   each changed symbol to that step.
4. If dirty work extends beyond recorded steps, STOP. Do not discard it.
5. Continue with the first incomplete step and update status after its gate.

## Git workflow

- Suggested branch: `feat/result-mutation-backend`, only if the operator
  asks.
- Do not commit unless the operator authorizes it. If authorized, use a
  logical message such as `Add PostgreSQL result mutation backend`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Record the contract decision

Create ADR-0023 covering: why updatability analysis is a separate
`Parse`/`Describe` step on an extended-protocol socket rather than metadata
captured at execution time — stating accurately that the wire protocol
carries origins in both protocols and only the tokio-postgres simple-query
API discards them; the read-write mutation socket alongside the read-only
browse socket; the cross-session name-resolution hazard, the conservative
`possibleTempShadowing` refusal, and the residual runtime `search_path`
risk with its qualified-target-display mitigation; analysis supersession
versus exclusive non-supersedable apply; the all-or-nothing transactional
apply with per-operation conflict (`rows_affected == 0`) and uniqueness
(`rows_affected > 1`) enforcement; the value-dependent null-safe predicate
rendering and why `IS NOT DISTINCT FROM` was rejected (not indexable);
guard strength by identity kind — edited-column guards for keyed updates,
full projected-row guards for keyed deletes, full-row guards for virtual
keys and `ctid` — including the documented limits (unedited-column changes
and A→B→A rewrites pass a keyed update); virtual keys as persisted user
claims, never proven constraints; writability rules covering any non-empty
`attgenerated` (PostgreSQL 18 virtual generated columns included),
identity-always, and system columns, plus default omission on insert;
`lock_timeout` bounding; the single extracted identity-resolution rule; and
the analysis snapshot cache with `analysisExpired` recovery. Update
`CONTEXT.md` with Result Mutation, Mutation Analysis, Mutation Plan,
Mutation Draft (noting Plan 006 owns the frontend model), Virtual Key, and
DML Preview vocabulary, distinguishing them from the existing ClickHouse
**Pending Mutation** concept.

**Verify**:

```sh
rg -n "table_oid|shadow|virtual key|attgenerated|attidentity|lock_timeout|analysisExpired|all-or-nothing|indexable" docs/adr/0023-staged-result-mutation-contract.md CONTEXT.md
```

Expected: every concept present and the ADR is `Accepted`.

### Step 2: Extract the shared identity rule

Create `postgres/identity.rs` as a pure move of the ADR-0022 resolution
logic from `table_browse/builder.rs:41-77`; make the browse builder delegate
to it. No behavior change; existing browse builder and live tests are the
proof.

**Verify**: `just fmt && just lint && just test`. Expected: all pass with no
browse test modified.

### Step 3: Build the protocol types and pure DML builder

Implement `protocol.rs` and `builder.rs` with every DTO, validation rule,
operation rendering, guard rendering by identity kind, default-omission
insert rendering, and the text-cast parameter scheme.

Unit tests assert exact SQL strings and exact parameter vectors for: update
with single- and multi-column identity, NULL guard values rendered as
`IS NULL` with parameter indices staying aligned, non-NULL values rendered
as indexable `=` casts, full-row guards for virtualKey and ctidFallback,
keyed delete with full projected-row guards, insert with omitted
defaultable and identity columns, insert of explicit NULL, duplicate-shaped
insert excluding generated columns, quoted identifiers with embedded quotes,
and every typed validation error: write to `generated`, `identityAlways`,
or `systemColumn`, unknown column, empty set, empty identity, NULL identity
value for a keyed identity kind, missing guard for virtual-key ops, and
plan/table mismatch. Include serde wire-shape
snapshots for tags and camelCase, matching `table_browse/protocol.rs:334-520`
style, covering every command payload, result, and error variant.

**Verify**: `just fmt && just lint && just test`. Expected: all pass with no
database required.

### Step 4: Implement the executor, analysis, and commands

Implement the manager, per-Connection executor, dedicated read-write
connection setup through `postgres::dedicated`, the `MutationDescriptor`
loader with `attgenerated`/`attidentity`/`atthasdef` and 42703/42P01
invalidate-retry-once, `Parse`/`Describe` analysis with origin resolution and
per-table classification, virtual-key consultation, the analysis snapshot
cache with `analysisId` and `analysisExpired`, preview, transactional apply
with `SET LOCAL lock_timeout`, per-op enforcement, rollback-and-verify after
failure or cancel, admission, analysis supersession, apply exclusivity
(`busy`), idle close, and the commands. Add the `virtual_keys` migration and
prefs-style commands. Register everything in `lib.rs` and wire
`with_active_connection` activity recording on success only.

Manager-level unit tests (no live server, following
`table_browse/manager.rs` and `executor.rs` test style): admission limits,
analysis supersession state machine, apply exclusivity and `busy`, cancel
bookkeeping, snapshot cache eviction and `analysisExpired`, idle close, and
typed error mapping. Storage tests: migration application and virtual-key
round-trip including clear.

**Verify**: `just fmt && just lint && just test`. Expected: all pass.

### Step 5: Fence teardown and run live characterization gates

Add `ResultMutationManager` to every `socket_lifecycle` fence site so
Connection save/delete/disconnect, bastion invalidation, managed
stop/destroy/recreate, credential reset, and app exit close all three
managers concurrently; storage-mode migration does not.

Add fixture tables: one with a stored generated column and a
GENERATED ALWAYS identity column, one keyless table with duplicate rows, and
one with a non-unique "looks like a key" column for `identityNotUnique`
coverage. Ignored live tests (`result_mutation_live`) cover: analysis of a
single-table SELECT (origins, identity, writability), a join SELECT
(per-table verdicts, update-eligible table with projected identity), an
expression/aggregate projection (`expression` origins), a system-column
projection (`SELECT ctid, xmin, * ...` → `systemColumn` writability, never
resolved against `pg_attribute`), a multi-statement string
(`notAnalyzable multiStatement`), the temp-shadowing refusal (a second
connection creates a temp table named like a fixture table →
`possibleTempShadowing`), analysis via `relation` source, virtual-key
save/consult/invalid-after-drop, preview exactness against the builder,
apply happy path for update/insert/delete/duplicate in one plan with
per-op affected counts, conflict detection (concurrent update between
analysis and apply → `conflict { opIndex }`, full rollback proven by
re-select; a keyed delete whose row changed since fetch likewise refused
through its full projected-row guards), a guarded update using the
primary-key index (plan inspected via `EXPLAIN`, no sequential scan),
`identityNotUnique` rollback on the non-unique virtual key,
lock-timeout surfacing (`lockTimeout`) against a fixture transaction holding
a row lock, ctid-keyed update with full-row guards succeeding and failing
after the row changed, generated-column write rejection, insert omitting
identity/default columns, cancellation of an in-flight apply with verified
rollback and a usable socket afterward, `analysisExpired` after cache
eviction, and teardown while an apply is in flight.

**Verify**: both fixtures, the ignored live command, `just fmt`,
`just lint`, `just test`, and `git diff --check`. Expected: all pass, no SQL
or values in captured logs, and only in-scope files plus the Plan 005 README
row changed.

## Test plan

- `builder.rs`: exact SQL and parameter assertions for every rendering rule
  and every typed validation error; property-style checks that generated SQL
  never contains a user value outside a parameter.
- `protocol.rs`: serde wire-shape snapshots for every command, result, and
  error variant.
- `mod.rs`/manager: admission, supersession, apply exclusivity, cancel,
  snapshot cache, idle close, teardown idempotence.
- `storage.rs`: migration application and virtual-key round-trip.
- Ignored live tests: the Step 5 matrix, each creating a unique schema and
  cleaning up in teardown.

## Done criteria

- [ ] Dark analyze, preview, apply, cancel, close, and virtual-key commands
      are registered; every legacy mutation command and caller is unchanged.
- [ ] Analysis recovers column origins through `Parse`/`Describe` without
      executing, classifies joins per table, refuses temp-shadowed
      resolutions, and reports typed `notAnalyzable` reasons; the frontend
      never parses SQL.
- [ ] Guarded operations use indexable predicates; conflict-detection
      strength per identity kind matches the ADR, including its documented
      keyed-update limits.
- [ ] The descriptor knows generated, identity, and defaulted columns;
      writes to non-writable columns are typed validation errors.
- [ ] Every user value reaches the server as a `$N` text parameter with an
      explicit `format_type`-derived cast; no value is ever interpolated.
- [ ] Preview returns the exact statements and ordered parameters apply will
      run; neither is ever logged.
- [ ] Apply is one transaction with `lock_timeout`, per-op conflict and
      uniqueness enforcement, all-or-nothing rollback, and typed per-op
      error attribution.
- [ ] Virtual keys persist, round-trip, invalidate on structure drift, and
      always operate behind full-row guards; `ctid` mutation likewise.
- [ ] Analysis supersedes deterministically; apply is exclusive, never
      superseded, cancelable, and leaves a verified-idle socket behind.
- [ ] One mutation socket per Connection, 8 app-wide, idle-closed, and
      fenced at every `socket_lifecycle` site alongside the other managers.
- [ ] The identity rule has one implementation shared with table browse; no
      browse behavior changed.
- [ ] No SQL, statement text, parameter, or row value is logged anywhere.
- [ ] All format, lint, test, live-test, and diff gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; a reviewer/operator records
      `DONE: <completion SHA>` after an authorized commit.

## STOP conditions

Stop and report if:

- Live code drifts from a load-bearing excerpt or resume work is
  unexplained.
- `Parse`/`Describe` executes or partially executes any statement, or
  multi-statement detection does not fail closed.
- `table_oid`/`column_id` are unavailable or unstable for ordinary
  single-table SELECT projections on the fixture.
- The extracted identity rule changes any existing browse builder test.
- A guarded update or delete can affect more than one row without producing
  `identityNotUnique`, or a conflict can commit partially.
- Rollback after a failed, conflicted, or cancelled apply leaves the
  executor socket in a non-idle state that a reconnect does not resolve.
- The executor cannot be fenced before any invalidation target, or admission
  conflicts with the query-session or browse budgets.
- A required change falls outside Scope or any gate fails twice.

## Maintenance notes

- Apply runs on its own socket and cannot see uncommitted state in a query
  tab's open manual transaction; the guards then read committed rows and a
  lock conflict surfaces as `lockTimeout`. Routing mutations through the
  session's own transaction is deliberate future work for the `PAR-001`
  savepoint follow-ons plus `PAR-004`, not a v1 defect.
- `PAR-004` should treat `preview_result_mutations` as its generated-SQL
  review surface and the apply command as its enforcement point for GUI
  mutations; the typed error union leaves room for a `policyBlocked`
  variant.
- The v1 analysis restriction to single-statement executions is a frontend
  presentation limit, not a contract limit: per-statement SQL from a future
  script-splitting `PAR-001` follow-on can be analyzed unchanged.
- `RETURNING`-based identity refresh for inserts is a deliberate v2 item;
  v1 refreshes by reloading the grid after apply.
- The legacy SQLx mutation path (`commit_cell_edits`, `insert_row`,
  `delete_rows`) becomes dead for PostgreSQL once Plan 006 activates, but
  must remain for MySQL, SQLite, and ClickHouse until `PAR-014` adapts the
  contract per engine.
