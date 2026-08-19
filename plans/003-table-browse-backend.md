# Plan 003: Add the PostgreSQL Table Browse backend

> **Executor instructions**: Do not start until Plans 001 and 002 are `DONE` in
> `plans/README.md`. Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report without improvising. Keep the feature dark: Plan 004
> owns frontend activation, and `load_table_data` remains the live path for
> every engine until then. Update this plan's row in `plans/README.md` to
> `IN PROGRESS: through Step N` after each completed step and to
> `READY FOR REVIEW` after all gates pass. A reviewer/operator records
> `DONE: <completion SHA>` only after an authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat 26268ca..HEAD -- CONTEXT.md docs/adr src-tauri plans/README.md
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
- **Risk**: MEDIUM
- **Depends on**: Plan 001 (`657553d`) and Plan 002 (`26268ca`) complete
- **Category**: direction
- **Planned at**: commit `26268ca`, 2026-08-19
- **Gap**: `PAR-002` in `plans/parity-gap-register.md`

## Why this matters

Table browsing today interpolates every value into one SQL string, runs on the
five-slot SQLx pool with no filter or sort contract, issues an unconditional
`COUNT(*)` on every page, refetches table structure on every page, has no
cancellation or stale-request fencing, and leaves keyless tables both unstably
paged and read-only. Sorting and filtering must operate on the relation, not
the loaded page, and every request must be bounded, cancelable, and keyed to
the grid state that issued it.

This plan lands a dark, typed, PostgreSQL-only Table Browse backend: a
parameterized query builder, a dedicated read-only browse connection with real
protocol cancellation, estimated and deferred counts, keyset pagination with a
documented offset fallback, backend-authoritative row identity including a
`ctid` virtual identity for keyless tables, query inspection, and durable
per-table grid preference storage. Plan 004 activates it in the grid.

## Current state

### Repository constraints

- PostgreSQL is the reference engine per
  `docs/adr/0001-postgres-first-engine-coverage.md`. This plan must not broaden
  into MySQL, SQLite, or ClickHouse; those engines stay on `load_table_data`.
- `docs/adr/0004-last-activity-on-connection-record.md`: record activity after
  a successful operation only. `with_active_connection` already does this.
- `docs/adr/0013-postgres-driver-fields-on-connection-record.md`: effective
  timeouts, search path, and role apply to every PostgreSQL connection. The
  browse connection must apply the same resolved driver options.
- `docs/adr/0021-dedicated-postgres-query-session-driver.md`: dedicated
  tokio-postgres sockets exist outside the five-slot SQLx pool, with connection
  admission, TLS parity through `ResolvedPostgresConnectSpec`, and teardown
  fencing at every invalidation site. Reuse that infrastructure; do not fork it.
- Never log SQL, filter text, row values, parameter values, or structured
  database error detail. `tokio_postgres = Warn` filtering is a security
  invariant (`src-tauri/src/lib.rs:155-184` precedent).
- CI runs `cargo test` with no database service. Every generated-SQL behavior
  must be proven by exact-string unit tests on pure builders; live behavior is
  covered by `--ignored` tests against the local fixture.

### Backend evidence

`src-tauri/src/commands/relational.rs:156-207` is the entire browse
orchestration today. It clamps paging, refetches structure every call, builds
one interpolated string, and always issues a second `COUNT(*)` query:

```rust
let select_query = dispatch::build_paged_select_query(
    &connection.engine(), "*", &qualified, page_size, offset, structure.as_ref(),
);
let select_result = dispatch::run_query(&connection, &select_query).await?;
// Best-effort COUNT(*) — never fail the call if the count fails.
let count_query = format!("SELECT COUNT(*) FROM {}", qualified);
```

`src-tauri/src/dispatch/relational.rs:287-311` interpolates `LIMIT`/`OFFSET`
directly and has no filter or sort input. `stable_order_columns`
(`dispatch/relational.rs:239-277`) orders by primary key, else the smallest
all-non-nullable unique index, else nothing, which makes keyless paging
unstable.

`src-tauri/src/postgres/query.rs:16-62` executes the string through
`sqlx::query(query).fetch_all(...)` with no binds and renders NULL as the
lossy `"NULL"` sentinel string (`postgres/mod.rs:70-148`).

`src-tauri/src/postgres/mutations.rs:18-158` is the existing parameterized
builder convention: return `(String, Vec<Option<String>>)`, `$N` placeholders,
identifiers through `quote_double`, `IS NULL` rendered without a parameter.

`src-tauri/src/postgres/schema.rs:193-232` derives `IndexInfo` from `pg_index`
via `unnest(ix.indkey)`; expression indexes yield `attnum = 0` entries, so
`IndexInfo.columns` is not a trustworthy uniqueness source without excluding
expression and partial indexes.

`src-tauri/src/postgres/admin.rs:22-75` already reads `pg_class.reltuples` as
an estimate; `src-tauri/src/types.rs:1681` documents it. This is the precedent
for estimated counts.

`src-tauri/src/query_session/postgres.rs:32-101` builds a dedicated
tokio-postgres connection from `ResolvedPostgresConnectSpec` with TLS parity
and driver options; `query_session/postgres.rs:523` implements `CancelToken`
cancellation. `src-tauri/src/storage.rs:134-144` plus
`commands/relational.rs:113-135` (`schema_map_prefs`) is the durable
per-scope preference precedent.

`LoadTableDataPayload` (`types.rs:1023-1031`) and `TableDataResult`
(`types.rs:818-827`) carry no filter, sort, cursor, request id, identity, or
inspection fields.

### Verified dependency and protocol facts

- The query-session actor executes through `simple_query_raw`
  (`query_session/postgres.rs:232`). The simple query protocol cannot carry
  bind parameters, so Table Browse must not reuse the session actor for
  execution. It reuses the connect spec, TLS connector, options, and
  CancelToken machinery instead.
- The tokio-postgres extended query protocol prepares exactly one statement;
  a raw filter fragment cannot smuggle a second statement past `Parse`.
  Step 5 must prove this against the fixture rather than assume it.
- tokio-postgres checks Rust-side `ToSql::accepts` against server-inferred
  parameter types. Binding text against an integer parameter fails at runtime.
  Therefore every value parameter is sent as text and cast in SQL:
  `($N::text)::<cast_type>`. The cast type must come from
  `pg_catalog.format_type(atttypid, atttypmod)`, not from
  `information_schema.data_type`, whose display strings (for example `ARRAY`)
  are not valid cast targets.
- `tid` gained full btree comparison operators and TID range scans in
  PostgreSQL 14. `ctid` keyset paging requires `server_version_num >= 140000`;
  older servers fall back to offset paging for keyless tables.
- `pg_class.reltuples` is `-1` (or `0` pre-analyze on old versions) when the
  table has never been analyzed. Treat `reltuples <= 0` as unknown, not zero.
- `EXPLAIN (FORMAT JSON)` returns the planner's `Plan Rows` estimate without
  executing the query; it is the estimated-count source for filtered browsing.
- `SET default_transaction_read_only = on` on the browse connection makes
  every browse statement run in a read-only transaction; PostgreSQL rejects an
  attempt to escalate back to read-write inside it. This is a robustness
  boundary against raw-filter side effects, not a security boundary: the user
  already holds the credentials.

## Decided architecture

### Executor and ownership

1. Add `TableBrowseManager` to `AppState`. One lazily created Table Browse
   Executor owns one dedicated tokio-postgres connection per stored Connection.
   SQLx remains canonical for `load_table_data` (all engines), metadata, admin,
   mutations, and exports. The new commands are PostgreSQL-only and return
   `unsupportedEngine` otherwise.
2. The executor connection is built from the same
   `ResolvedPostgresConnectSpec`, TLS connector, and driver-option SQL as query
   sessions, then additionally runs `SET default_transaction_read_only = on`.
   It holds a socket, not reusable credentials.
3. Admission: at most 1 browse socket per Connection and 8 across the app,
   accounted separately from the query-session budget (7+1 per Connection,
   24 app-wide). An idle executor closes after 300 seconds, matching the SQLx
   pool idle timeout. Never queue more than one pending request per tab.
4. Requests are keyed by `(connectionId, tabId, requestId)`. `requestId` is a
   frontend-monotonic u64 per tab. A newer request for the same tab supersedes
   the older one: if the older is queued it is dropped, if it is in flight the
   executor issues a protocol cancel; the superseded command resolves with the
   typed error `superseded`. `cancel_table_browse` cancels the current request
   for a tab without replacing it. Requests for different tabs on one
   Connection are serialized on the single executor socket; queue wait is
   bounded at 10 seconds before a typed `timeout`.
5. Teardown fencing matches query sessions exactly: Connection save, delete,
   and disconnect, bastion invalidation, managed stop/destroy/recreate, and
   destructive credential reset must fence new browse requests and close the
   executor socket before the target or credentials disappear. Storage-mode
   migration does not close executors. Close is idempotent and bounded at
   3 seconds.

### Structure cache and backend-authoritative identity

6. The executor caches one relation descriptor per `(schema, table)`: columns
   in order with `format_type` cast types and `attnotnull`, the primary key,
   and unique-index candidates restricted to valid, immediate, non-partial,
   non-expression indexes (`indisvalid AND indisunique AND indpred IS NULL AND
   indexprs IS NULL`) whose columns are all non-nullable. This is the
   authoritative identity rule; it intentionally supersedes the drifting
   duplicate pair `stable_order_columns` (backend) and `pickRowIdentity`
   (frontend) for browse mode. Neither legacy implementation changes in this
   plan.
7. Identity resolution order: primary key → smallest qualifying unique index
   (ties by index name, matching `stable_order_columns`) → `ctid` virtual
   identity → `none` is unreachable for PostgreSQL heap tables, but the enum
   keeps `none` for defensive completeness (for example, foreign tables where
   `ctid` is not meaningful; those degrade to offset paging and no identity).
8. A request may set `refreshStructure: true` (Plan 004 sends it on explicit
   Refresh and after dbunk-issued DDL/mutations). On a database error with
   SQLSTATE `42703` (undefined column) or `42P01` (undefined table), the
   executor invalidates the cached descriptor and retries the request once
   before surfacing the error.

### Query construction

9. `table_browse/builder.rs` is a pure module: given a relation descriptor and
   a `BrowseTableDataPayload`, it returns `(sql, params)` with `$N` text
   parameters, or a typed validation error. No I/O, fully unit-testable with
   exact-string assertions, following the `postgres/mutations.rs` convention.
10. Typed filters AND-combine. Operators: `eq`, `neq`, `lt`, `lte`, `gt`,
    `gte`, `contains`, `notContains`, `startsWith`, `endsWith`, `isNull`,
    `isNotNull`, `inList`. Comparison operators render
    `"col" <op> ($N::text)::<cast_type>`; text-match operators render
    `"col"::text ILIKE $N` with the pattern built in Rust from an escaped
    value (`\`, `%`, `_` escaped, `ESCAPE '\'` clause explicit); `isNull` and
    `isNotNull` bind nothing; `inList` renders
    `"col" = ANY(($N::text[])::<cast_type>[])` with one array parameter.
    Unknown column names, empty `inList`, and operators inapplicable to the
    column produce typed errors, never generated SQL.
11. Raw filter mode is a WHERE fragment wrapped as `(<fragment>)` and combined
    with any typed filters by AND. Its safety boundary is: single-statement
    extended protocol, the read-only browse transaction, typed structured
    database errors carrying `position`, and the no-logging invariant. The
    contract documents that raw filters execute with the connection's
    privileges and can be arbitrarily expensive; cancellation is the recourse.
12. Sort is `Vec<BrowseSortKey { column, direction: asc | desc,
    nulls: default | first | last }>`, validated against the descriptor,
    identifiers through `quote_double`. Identity columns not already present
    are appended as tiebreakers so ordering is total whenever identity exists.
13. Pagination request is a tagged union: `offset { page }` or
    `keyset { cursor: Option<BrowseCursor> }`. Keyset is supported only when
    the effective sort is exactly the identity order (no user sort keys),
    including the `ctid` identity on `server_version_num >= 140000`. Keyset
    renders a row-value comparison over the identity columns
    (`("a","b") > (($1::text)::t1, ($2::text)::t2)`), forward only. Everything
    else — user sorts, page-number jumps, previous-page navigation, pre-14
    keyless tables — uses `LIMIT/OFFSET`, and the response labels which mode
    actually ran. An invalid or stale cursor is the typed error
    `invalidCursor`; Plan 004 recovers by re-requesting the first page.
14. The builder always fetches `page_size + 1` rows to derive `hasMore`
    without a count, and returns at most `page_size` rows. `page_size` clamps
    to `1..=1000` (existing `MAX_TABLE_PAGE_SIZE`).

### Counts

15. `browse_table_data` never runs `COUNT(*)`. Its `countPolicy` is `none` or
    `estimated`. Estimated with no filters reads `pg_class.reltuples` and
    reports `unknown` when `reltuples <= 0`; estimated with filters (typed or
    raw) runs `EXPLAIN (FORMAT JSON)` on the generated relation query and
    reports the top plan's `Plan Rows`. The response labels the count kind so
    the UI can never present an estimate as exact.
16. `count_table_browse_rows` is a separate, user-initiated command running
    `SELECT count(*)` with the same generated WHERE on the executor. It is
    supersedable and cancelable like any browse request and bounded only by
    the connection's statement timeout.

### Result bounds and identity payload

17. Cells decode to `Option<String>` — true NULLs, not the `"NULL"` sentinel.
    Per-cell retained bytes cap at 1 MiB with UTF-8-safe truncation, per-row
    at 2 MiB, per-response at 32 MiB including `rowIdentity`. When the
    response cap is hit, remaining fetched rows are dropped, `hasMore` is
    forced true, and typed truncation counters report omitted rows and
    truncated cells. Shared truncation helpers live in
    `postgres/row_budget.rs`.
18. The response carries `identity { kind, columns }` and, when identity
    exists, `rowIdentity: Vec<Vec<String>>` aligned with `rows`, extracted
    from projected columns or from a `ctid::text` column the builder appends
    and strips from the visible projection. This is the metadata `PAR-003`
    consumes for editing query results and keyless tables.
19. The response carries `inspection { sql, params }` — the exact executed
    statement text and ordered parameter values — for display only. It must
    never be logged on either side.

## Wire contract

Rust DTOs live in `src-tauri/src/table_browse/protocol.rs` with
`#[serde(rename_all = "camelCase")]` and `tag = "kind"` for unions, matching
the query-session convention.

Commands:

| Command | Payload | Result |
|---|---|---|
| `browse_table_data` | `BrowseTableDataPayload` | `BrowseTableResult` |
| `cancel_table_browse` | `connectionId`, `tabId` | `{ cancelRequested }` |
| `count_table_browse_rows` | target + filters + `requestId` | `{ kind: "exact", value }` |
| `close_table_browse_for_tab` | `connectionId`, `tabId` | `()` |
| `load_table_grid_prefs` | `connectionId`, `schema`, `table` | `TableGridPrefs \| null` |
| `save_table_grid_prefs` | `connectionId`, `schema`, `table`, `prefs` | `()` |

`BrowseTableDataPayload`: `connectionId`, `tabId`, `requestId`, `schema`,
`table`, `filters: BrowseFilter[]` (tagged `comparison { column, operator,
value }`, `textMatch { column, operator, value }`, `isNull` / `isNotNull
{ column }`, `inList { column, values }` (non-empty), or `rawSql { text }`),
`sort: BrowseSortKey[]`,
`pageRequest` (tagged `offset { page }` or `keyset { cursor }`), `pageSize`,
`countPolicy`, `refreshStructure`.

`BrowseTableResult`: `requestId`, `columns: { name, castType, nullable }[]`,
`rows: (string | null)[][]`, `identity { kind: "primaryKey" | "uniqueIndex" |
"virtual" | "none", columns }`, `rowIdentity`, `pageInfo { mode, page,
hasMore, nextCursor }`, `count { kind: "exact" | "estimated" | "unknown",
value }`, `inspection { sql, params }` where params are tagged `text` /
`textArray`, truncation counters, `runtimeMs`.

`TableBrowseError` tagged union: `unsupportedEngine`, `unknownColumn`,
`invalidFilter` with reason, `invalidSort`, `invalidCursor`, `superseded`,
`cancelled`, `connectionClosing`, `connectionLost`, `timeout` with operation,
`database` with structured non-loggable display fields (`code`, `message`,
`severity`, `position`). The legacy `Result<T, String>` convention cannot
express supersession or cancellation and must not be used here.

`TableGridPrefs` is a versioned JSON document (page size, sort, typed filters,
raw filter text, filter/sort history capped at 20 entries, named presets,
optional column widths). Stored in a new SQLite migration `table_grid_prefs`
with primary key `(connection_id, schema, table_name)`, following the
`schema_map_prefs` shape (`storage.rs:134-144`). The backend treats the
document as opaque validated JSON; interpretation belongs to Plan 004.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Rust format | `just fmt` | exit 0 |
| Rust lint | `just lint` | exit 0, no warnings |
| Rust tests | `just test` | all non-ignored tests pass |
| Plain fixture | `pnpm db:postgres` | healthy on port 15432 |
| TLS fixture | `pnpm db:postgres-tls` | healthy on port 15433 |
| Live tests | `cargo test --manifest-path src-tauri/Cargo.toml table_browse_live -- --ignored --test-threads=1` | all pass |
| Diff hygiene | `git diff --check` | no output |

Never print environment variables, Connection records, SQL, or parameter
values.

## Scope

**In scope**:

- `CONTEXT.md` (Table Browse vocabulary)
- `docs/adr/0022-server-backed-table-browse-contract.md` (create)
- `src-tauri/src/lib.rs`, `src-tauri/src/types.rs`
- `src-tauri/src/table_browse/mod.rs` (create)
- `src-tauri/src/table_browse/protocol.rs` (create)
- `src-tauri/src/table_browse/builder.rs` (create)
- `src-tauri/src/table_browse/postgres.rs` (create)
- `src-tauri/src/query_session/postgres.rs` (only if truncation/connect
  helpers need visibility changes for reuse)
- `src-tauri/src/commands/mod.rs`, `relational.rs` or a new
  `commands/table_browse.rs`
- `src-tauri/src/commands/connections.rs`, `bastions.rs`, `managed.rs`,
  `settings.rs` (teardown fencing only)
- `src-tauri/src/managed.rs` (teardown fencing only)
- `src-tauri/src/storage.rs` (one new migration)
- `infrastructure/test-db/postgres/*.sql` (fixture tables for keyless,
  expression-index, and large-table cases, additive only)
- `plans/README.md` status text only

**Out of scope**:

- All `src/**/*.ts` and `src/**/*.tsx`; Plan 004 owns activation
- Changing or removing `load_table_data`, `build_paged_select_query`,
  `stable_order_columns`, or any legacy caller behavior
- MySQL, SQLite, ClickHouse, or Redis browse adapters
- Mutation, editing, or updatability analysis (`PAR-003`)
- Streaming/portal-based browse, disk spooling, result virtualization
- Filter expression grouping (OR trees); v1 is AND-combined plus raw mode
- Commits, pushes, PRs, or publication without authorization

## Resume protocol

1. Read the `plans/README.md` status for Plan 003.
2. Inspect `git status --short` and `git diff -- <Scope paths>`.
3. Accept changes only when they match steps recorded as completed. Compare
   each changed symbol to that step.
4. If dirty work extends beyond recorded steps, STOP. Do not discard it.
5. Continue with the first incomplete step and update status after its gate.

## Git workflow

- Suggested branch: `feat/table-browse-backend`, only if the operator asks.
- Do not commit unless the operator authorizes it. If authorized, use a
  logical message such as `Add PostgreSQL table browse backend`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Record the contract decision

Create ADR-0022 covering: why browse leaves the simple-query session actor
(no bind parameters) but reuses its connect spec, TLS, options, and cancel
machinery; the read-only browse transaction as a robustness boundary; the
text-parameter-with-explicit-cast scheme and why `format_type` is required;
keyset-only-on-identity-order with documented offset fallback; estimated
`reltuples`/EXPLAIN counts and the separate exact count command; `ctid`
virtual identity and its PostgreSQL 14 requirement; supersede-and-cancel
request semantics; the backend-authoritative identity rule and the drifting
frontend/backend duplicates it replaces for browse mode. Update `CONTEXT.md`
with Table Browse, Browse Request, Browse Filter, Row Identity, and Grid
Preferences vocabulary.

**Verify**:

```sh
rg -n "format_type|read-only|reltuples|ctid|supersede|simple.query|keyset" docs/adr/0022-server-backed-table-browse-contract.md CONTEXT.md
```

Expected: every concept present and the ADR is `Accepted`.

### Step 2: Build the pure query builder and protocol types

Implement `protocol.rs` and `builder.rs` with every DTO, validation rule,
operator rendering, ILIKE escaping, raw-fragment wrapping, sort and tiebreaker
rendering, keyset row-value comparison, offset rendering, `page_size + 1`
fetch, `ctid::text` projection for virtual identity, and cursor
encode/validate. The builder takes only the relation descriptor and payload.

Unit tests assert exact SQL strings and exact parameter vectors for: every
operator, ILIKE metacharacter escaping, `inList`, null operators, raw + typed
combination, multi-column sort with nulls handling, tiebreaker appending,
keyset over single- and multi-column identity, `ctid` keyset, offset fallback
selection, clamped page sizes, and every typed validation error including
unknown column, empty in-list, and cursor arity mismatch. Include serde
wire-shape snapshots for tags and camelCase, matching
`query_session/protocol.rs:225-266` style.

**Verify**: `just fmt && just lint && just test`. Expected: all pass with no
database required.

### Step 3: Implement the executor, manager, and commands

Implement the manager, per-Connection executor task, dedicated read-only
connection setup, relation-descriptor cache with `refreshStructure` and
42703/42P01 invalidate-retry-once, request queue with per-tab supersession,
protocol cancellation, 10-second queue-wait bound, decode with truncation
caps, identity extraction, count policies, admission, and idle close. Register
the commands and wire `with_active_connection` activity recording on success
only.

Manager-level unit tests (no live server, following
`query_session/mod.rs:1047-1126` style): admission limits, supersession state
machine (queued-drop and in-flight-cancel paths), queue-wait timeout, idle
close bookkeeping, and typed error mapping.

**Verify**: `just fmt && just lint && just test`. Expected: all pass.

### Step 4: Fence every teardown path

Fence browse executors at exactly the sites that fence query sessions:
Connection save/delete/disconnect, bastion invalidation, managed
stop/destroy/recreate, destructive credential reset, and app exit. Executor
close precedes target invalidation; storage-mode migration does not close
executors. Add the SQLite `table_grid_prefs` migration and the prefs commands
mirroring `schema_map_prefs` load/save behavior.

**Verify**:

```sh
rg -n "table_browse|TableBrowse" src-tauri/src/commands src-tauri/src/managed.rs
just test
```

Expected: every invalidation site visibly closes browse executors first;
migration and prefs round-trip tests pass.

### Step 5: Run live characterization gates

Add fixture tables: a keyless table with duplicate rows, a table whose only
unique index is an expression index, and a seeded large table (>= 100k rows).
Ignored live tests cover: every typed operator against real column types
(integers, numerics, timestamps, uuids, booleans, text, arrays), the cast
scheme on those types, ILIKE escaping, raw filter execution, a raw fragment
attempting statement smuggling (`1=1); DROP TABLE ...; --`) rejected by the
extended protocol, a raw filter invoking a writing function rejected by the
read-only transaction, keyset paging over PK and `ctid` with correct
continuation and no duplicates/gaps across page boundaries, offset fallback
for user sorts and pre-14 semantics (skip `ctid` keyset when the fixture
server is older than 14), estimated counts (unanalyzed → unknown, analyzed →
estimate, filtered → EXPLAIN estimate), exact count, cancellation of a
`pg_sleep` raw filter, rapid supersession returning `superseded` to the older
request, structure-cache invalidation after a column drop, truncation
counters on oversized cells, and teardown while a request is in flight.

**Verify**: both fixtures, the ignored live command, `just fmt`, `just lint`,
`just test`, and `git diff --check`. Expected: all pass, no SQL or values in
captured logs, and only in-scope files plus the Plan 003 README row changed.

## Test plan

- `builder.rs`: exact SQL and parameter assertions for every rendering rule
  and every typed validation error; property-style checks that generated SQL
  never contains a user value outside a parameter (search generated text for
  sentinel values).
- `protocol.rs`: serde wire-shape snapshots for every command, result, and
  error variant.
- `mod.rs`: admission, supersession, queue bounds, idle close, teardown
  idempotence.
- `storage.rs`: migration application and prefs round-trip.
- Ignored live tests: the Step 5 matrix, each creating a unique schema and
  cleaning up in teardown.

## Done criteria

- [ ] Dark `browse_table_data`, cancel, count, close, and prefs commands are
      registered; `load_table_data` and all legacy callers are unchanged.
- [ ] Every user value reaches the server as a `$N` text parameter with an
      explicit `format_type`-derived cast; no value is ever interpolated.
- [ ] Raw filter mode is single-statement and runs read-only; both properties
      are proven by live tests.
- [ ] Sorting and filtering apply to the relation; responses are bounded by
      cell/row/response caps with typed truncation counters.
- [ ] Keyset paging works for identity-ordered browsing including `ctid` on
      PostgreSQL 14+; every other shape uses labeled offset fallback.
- [ ] No browse request runs `COUNT(*)` implicitly; estimates are labeled and
      exact counts are explicit, supersedable, and cancelable.
- [ ] Responses carry backend-authoritative identity kind, columns, and
      per-row identity values, including `ctid` virtual identity.
- [ ] Rapid navigation supersedes deterministically; cancellation uses the
      protocol cancel token; queue wait is bounded.
- [ ] One browse socket per Connection, 8 app-wide, idle-closed, and fenced
      before every invalidation target.
- [ ] `table_grid_prefs` persists and round-trips.
- [ ] No SQL, filter, parameter, or row value is logged anywhere.
- [ ] All format, lint, test, live-test, and diff gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; a reviewer/operator records
      `DONE: <completion SHA>` after an authorized commit.

## STOP conditions

Stop and report if:

- Live code drifts from a load-bearing excerpt or resume work is unexplained.
- The `($N::text)::<cast_type>` scheme fails for any common fixture column
  type, or `format_type` output is not directly usable as a cast target.
- The extended protocol does not reject a multi-statement raw fragment, or
  the read-only browse transaction can be escalated from a WHERE fragment.
- Keyset paging produces duplicate or missing rows across page boundaries in
  the live matrix.
- Protocol cancellation does not interrupt a running browse statement on the
  fixture.
- The executor cannot be fenced before any invalidation target, or admission
  conflicts with the query-session socket budget.
- A required change falls outside Scope or any gate fails twice.

## Maintenance notes

- The identity rule here is intentionally stricter than
  `stable_order_columns` (it excludes expression and partial indexes).
  When Plan 004 activates, the legacy pair becomes dead for PostgreSQL browse
  but must remain for other engines; `PAR-003` should consolidate them.
- `ctid` identity is only stable between fetches while rows are not updated
  or vacuumed away. It is sufficient for browse continuity and display
  identity; `PAR-003` must re-verify identity before mutating through it.
- If a future SQLx release exposes protocol cancellation and typed-parameter
  inference suitable for this path, the dedicated executor can collapse into
  the pool without changing the wire contract.
- OR filter trees and mixed-direction keyset are deliberate v2 items; the
  tagged unions leave room for both without breaking the contract.
