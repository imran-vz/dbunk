# Plan 023: Bound parameters and row-limited reads in PostgreSQL Query Sessions (dark)

- Priority: P0. Effort: L. Risk: HIGH. Gap: PAR-001.
- Planned against: `49c50e8`, 2026-10-01.
- Depends on: Plans 001 and 002 (`657553d`, `26268ca`), Plan 007 for the
  statement policy (`bd9f7ef`).
- Execution status: see [README.md](./README.md).
- Category: backend foundation. The parameter and row-limit behavior is
  reachable only through new optional payload fields that no frontend caller
  sends yet. Two existing behaviors change on purpose: a user-cancelled
  execution settles as `cancelled`, and a session closed with a full credit
  window releases its connection. See "Cancellation outcome" and "Credit loop
  repair".
- Reviewed: one independent adversarial review on 2026-10-01, amended for its
  findings. See "Review record".

> **Executor instructions**: read this plan completely before editing. Follow
> the steps in order and run each step's gate. Update the Plan 023 row in
> `plans/README.md` after each completed step. Stop on every STOP condition and
> report; do not improvise around one. Mark `READY FOR REVIEW` after all gates.
> Commits, pushes, and PRs need separate authorization.
>
> **Drift check before Step 1**:
>
> ```sh
> git diff --stat 49c50e8 -- src-tauri/src/query_session \
>   src-tauri/src/postgres/sql_lex.rs src-tauri/src/postgres/sql_class.rs \
>   src-tauri/src/postgres/dedicated.rs src-tauri/src/postgres/options.rs \
>   src-tauri/src/commands/query_session.rs src-tauri/src/safety/live.rs
> ```
>
> Expected on a fresh run: no output. Any change to those files after
> `49c50e8` means the evidence below must be re-read before continuing.

## Outcome and boundary

A PostgreSQL Query Session can run one statement with parameters that the
driver binds, and can stop a read at a requested row count without the server
sending the rest. Both use a server-side cursor: the statement is declared
through the extended protocol with text-format bound values, and rows are
fetched through the simple protocol, which keeps the server-rendered text
results the session contract already promises.

This closes two of the remaining PAR-001 pieces at the backend: driver-bound
parameters and the per-execution maximum-row policy. It also repairs two
defects in the existing session actor that the new paths would otherwise make
worse: the cancellation outcome and a credit loop that can park forever.

This plan does not change any React component, store slice, or frontend
caller. The editor keeps `applyBindVariables` literal substitution until a
separate activation plan switches it. Script error policy, savepoint controls,
command tags, fetch-more, and EXPLAIN with parameters are out of scope; see
"Deferred work".

## Why this matters

- `src/lib/bind-variables.ts` substitutes values into SQL text with one regular
  expression. It matches `:name` inside string literals, comments, and array
  slices (`arr[1:n]`), and guesses numeric versus string from the value's
  shape rather than from the column it is compared with. The substituted text
  is what runs and what query history records (`relational-queries.ts:224`),
  so values are spliced into the statement instead of travelling beside it.
- A Query Execution is one simple-protocol request. When retention limits are
  reached the driver keeps reading and counts the rest
  (`src-tauri/src/query_session/postgres.rs:239-254`), so `SELECT * FROM big`
  transfers the whole table before it settles. Measured on the fixture: a 3M-row
  read took 1,496 ms to transfer; a cursor fetch of 201 rows took 8 ms.
- A user Stop settles as `failed` with SQLSTATE 57014. The frontend reducer
  handles `status === "cancelled"` (`src/lib/query-session-budget.ts:161`) but
  `run_execution` only ever emits `"completed"` or `"failed"`
  (`src-tauri/src/query_session/mod.rs:846-864`). No commit in the history of
  `src-tauri/src/query_session/` contains a `cancelled` status.
- `send_with_credit` (`mod.rs:937-952`) waits on `credit_changed` without
  checking `closed`. `close_session` notifies once (`mod.rs:1074`); the window
  is still full because ACKs now fail in `bound()`, so the execution task parks
  again and keeps the `Arc<Session>` and its socket. The `ackTimeout` expiry
  (`mod.rs:121-141`) lands in the same state. The loop also creates its
  `notified()` future after releasing the credit lock, so an ACK in that gap is
  missed until the next one.

The last two were found by reading the code on 2026-10-01 and confirmed by the
independent review. Neither was reproduced in a running app.

## Evidence

Read these before implementation:

- `docs/adr/0021-dedicated-postgres-query-session-driver.md`: why the session
  uses tokio-postgres 0.7.18 and the simple protocol.
- `CONTEXT.md` entries for Query Session, Query Execution, Result Set, and
  Query Outcome.
- `src-tauri/src/query_session/mod.rs`: `execute` (384-427), `run_execution`
  (727-916), `cancel` (494-506), `refresh`/`set_mode`/`set_isolation`
  (507-546), the credit helpers (922-1025), `close_session` (1063-1099).
- `src-tauri/src/query_session/postgres.rs`: `reduce_stream` (135-332) owns
  every retention limit: 10,000 rows per Result Set, 50,000 rows and 32 MiB per
  execution, 1 MiB per cell, 2 MiB per row, 64 Result Sets, 500 notices, 1 MiB
  metadata, batches of 200 rows or 256 KiB. Its `row_count` already includes
  rows dropped by those limits.
- `src-tauri/src/postgres/dedicated.rs:344-358`: `database_error` maps every
  error that is not a server `DbError` to `ConnectionLost`, and `run_execution`
  retires the session on `ConnectionLost` (`mod.rs:865-877`).
- `src-tauri/src/postgres/options.rs:8-12`: the app sets `statement_timeout`
  and `idle_in_transaction_session_timeout` on session sockets when configured.
  Read-only connections also set `default_transaction_read_only = on`.
- `src-tauri/src/postgres/sql_lex.rs` and `sql_class.rs`: the lexer treats
  strings, E-strings, dollar quotes, `$n`, comments, and quoted identifiers as
  opaque or skipped, emits `:` as an opaque token and a following word as an
  ordinary identifier, rejects non-ASCII outside those regions, and rejects
  plain strings whose end depends on `standard_conforming_strings`. Tokens
  carry no source spans. `classify_statement` reads an unquoted head keyword;
  a `SELECT` head goes to `classified_read`, which checks the function denylist
  and `SELECT ... INTO` but not row-locking clauses. `lex_sql` has other
  callers in `result_mutation/postgres.rs` and `postgres/object_ddl/`.
- `src-tauri/src/safety/live.rs:87-94`: a test helper builds `ExecutePayload`
  with a struct literal.
- `src-tauri/src/lib.rs:233`: `tokio_postgres` logging is pinned to Warn. The
  driver logs bound parameters at Debug.
- `src/lib/store/query-sessions.ts:207-223`, `relational-queries.ts:273-279`,
  and `query-sessions.test.ts:621-678`: what the frontend does with a
  `cancelled` execution. Context only; do not edit.
- `src/lib/bind-variables.ts` and its call sites in
  `src/components/query-editor-panel.tsx` (878-883, 974-985, 1348, 1367):
  context only; do not edit.

### Driver facts verified on 2026-10-01

Checked against the vendored sources of tokio-postgres 0.7.18,
postgres-types 0.2.14, and postgres-protocol 0.6.12, and against the upstream
changelog (0.7.18 is the latest release):

- `query.rs::encode_bind_raw` passes `Some(1)` as the result format, so every
  extended-protocol result is binary. No public API requests text results.
- `SimpleQueryMessage::CommandComplete(u64)` carries a row count only. The tag
  is parsed and discarded. Command tags stay unavailable without a driver
  change.
- `ToSql::encode_format` lets a parameter choose `Format::Text` per value.
- `Client::execute(&str, ..)` prepares first (Parse, Describe, Sync on the
  wire) and only then checks the parameter count. A named statement is closed
  when its `Statement` is dropped. Dropping a response future does not cancel
  the request. The driver keeps its own `typeinfo` statements for the life of
  the connection when a type is not built in.

### Spike record (2026-10-01)

Target: the repository's disposable fixture, `infrastructure/test-db` profile
`postgres` (PostgreSQL 16.14, `127.0.0.1:15432`), started for the spike and
removed afterwards. The spike was a throwaway crate outside the repository
pinned to tokio-postgres 0.7.18; nothing from it is committed.

| Check | Result |
|---|---|
| `DECLARE <name> NO SCROLL CURSOR FOR <select with $1..$6>` through `Client::execute` with a text-format `ToSql` wrapper (`accepts` everything) | Accepted. The server inferred int4, text, timestamptz, numeric, bool, and text[] from context. |
| `FETCH FORWARD n FROM <name>` through `simple_query_raw` | Server-rendered text for int, text, timestamptz, numeric, text[], jsonb, bool, bytea. RowDescription arrives on a zero-row FETCH too. `CommandComplete` equals the rows fetched. |
| NULL parameter (`IsNull::Yes`) | Bound as NULL. |
| Value containing `'; DROP TABLE ... --` | Treated as data. |
| Error position inside the declared statement | Server position minus the `DECLARE ... FOR ` prefix length in characters equals the position in the user's statement (47 - 37 = 10; 76 - 37 = 39). |
| Wrong value for an inferred type (`'abc'` for int4) | SQLSTATE 22P02 at `DECLARE`; the transaction is aborted. |
| Parameter count mismatch | Driver error `expected 2 parameters but got 1`, raised after the prepare round trip. It is not a server error. |
| `DECLARE ... FOR UPDATE ... RETURNING` | SQLSTATE 42601. Cursors cannot wrap data-modifying statements. |
| Cursor outside a transaction block | `DECLARE CURSOR can only be used in transaction blocks`. `WITH HOLD` works but materializes the whole result at commit. |
| Bound `UPDATE` through `Client::execute` | Affected-row count returned. `Statement::columns()` is empty for DML without RETURNING and non-empty with RETURNING or for EXPLAIN (one `json` column), so "returns rows" is known after Parse/Describe and before execution. |
| Cancel during `FETCH` inside `BEGIN` | SQLSTATE 57014; `ROLLBACK` restores an idle session. |
| Cursor name collision | SQLSTATE 42P03. |
| Row limit | 3M rows of 200 bytes: full simple-protocol read 1,496 ms; `DECLARE` 0.3 ms + `FETCH FORWARD 201` 8 ms + `CLOSE` 2.5 ms. One run on one machine through psql; an order of magnitude, not a benchmark. |

Not verified: PostgreSQL majors other than 16; TLS and tunnel routes for the
cursor path; plan quality under a cursor (see "Known costs of a cursor");
`Client::execute_typed` with unspecified types as a one-round-trip alternative
to prepare-then-execute. Steps 4 and 6 cover these.

## Decided architecture

### Parameter mode and the three execution shapes

A payload is in **parameter mode** when it carries a `parameters` field, even
an empty one. In parameter mode the backend scans the SQL for named
parameters. "Has parameters" below always means the scan found at least one
name in the SQL; it never means the payload merely supplied values. A payload
without the field is never scanned, so a stray `:name` reaches the server
exactly as it does today.

The manager picks one shape from the SQL text and the payload before anything
is sent, and never retries a statement in another shape after the server has
seen it.

1. **Script** (today's path): one simple-protocol request. Used when the SQL
   has no parameters and either no row limit or a statement that is not
   cursor-eligible.
2. **Cursor read**: used when the SQL is cursor-eligible and has parameters, a
   row limit, or both.
3. **Bound command**: used when the SQL has parameters, is exactly one
   statement, and is not cursor-eligible. It is prepared first; if the
   prepared statement describes result columns the execution is refused
   without running, otherwise it runs through the extended protocol.

Cursor-eligible means all of:

- the script lexes and contains exactly one statement;
- after parameter rewriting, `classify_statement` returns `Read`;
- the unquoted head keyword is `SELECT`, `VALUES`, `TABLE`, or `WITH`;
- it contains no row-locking clause (`FOR` followed by `UPDATE`, `NO`,
  `SHARE`, or `KEY`) at any depth, because a cursor locks rows as they are
  fetched and a row limit would lock a different set than the statement
  names;
- it contains no native `$n` placeholder.

`SHOW`, `EXPLAIN`, `SELECT ... INTO`, statements with a denylisted function,
and statements with a parenthesized or quoted head are therefore not eligible.
The cursor text is built from the statement's own span, so leading or trailing
semicolons and whitespace never end up after `FOR`.

A native `$n` in SQL that also has named parameters is refused. A native `$n`
with only a row limit stays on the Script shape, where the server answers
42P02 as it does today.

Parameters with more than one statement are refused. The extended protocol
runs one statement per request, and splitting a script changes its implicit
transaction; that belongs to the script-policy plan.

### Parameter grammar, scanning, and rewriting

A new pure module owns parameters. It reuses the lexer's region rules through
a span-carrying scan, so the classifier and the scanner cannot disagree about
where a string or comment ends. `lex_sql` keeps its signature and becomes a
projection of the span-carrying scan; its other callers are not edited and
their tests must pass unchanged.

A parameter is `:` followed by `[A-Za-z_][A-Za-z0-9_]*`, outside every opaque
or skipped region, when none of these hold:

- The character directly before the colon is an identifier character, a
  digit, a closing quote, `)`, `]`, or another `:`. This excludes casts,
  `a[1:n]`, and the PostgreSQL 16 `JSON_OBJECT('k':v)` and `k:v` forms.
- The innermost enclosing bracket or parenthesis is a subscript bracket. A
  bracket is a constructor bracket when it directly follows the `ARRAY`
  keyword or sits directly inside a constructor bracket; every other bracket
  is a subscript. So `ARRAY[:a, :b]` and `arr[(:n)]` take parameters, while
  `arr[:n]` and `arr[1 :n]` are slices.

Names are case-sensitive. A name over 63 bytes is refused rather than
truncated.

- Each distinct name becomes `$k` in order of first appearance; repeats reuse
  the same `$k`.
- The rewrite returns a segment map so a server error position in the
  rewritten text translates back to the original text. Server positions and
  the positions the frontend receives are both in characters, not bytes; the
  map works in characters and the cursor prefix length is subtracted first. A
  position inside the prefix is dropped.
- Values are `string | null`. Every value is sent in text format and the
  server infers the type. Supplied names the statement does not use are
  ignored, because the editor keeps values for every name in the tab. A name
  the statement uses without a supplied value is refused. A name supplied
  twice is refused.
- Limits: 256 distinct names per statement, 1 MiB per value, 4 MiB across all
  values.
- Values never appear in logs, errors, events, or audit rows. The payload type
  and the parameter wrapper get a `Debug` implementation that redacts values;
  the secrecy of the driver's own Debug logging continues to rely on the
  existing Warn pin for `tokio_postgres`, and a test asserts that pin. Names
  may appear, because they come from the SQL text.

### Safety policy and audit

Order in the `execute` command: parameter and row-limit validation (pure),
then the statement policy, then session lookup. Validation first means a user
is never asked to confirm a write that is then refused for a missing value.

In parameter mode the policy classifies the **rewritten** text, in which each
parameter is an opaque `$k`. Classifying the `:name` text would let a
parameter name act as a keyword: `UPDATE t SET a = :where` would look bounded
to `is_unbounded_at` while the server runs an unbounded update. The success
hook and audit disposition are unchanged for all three shapes and still fire
only on `completed`.

### Cursor read lifecycle

The cursor name is the reserved constant `dbunk_query_cursor`. A session runs
one execution at a time and always closes the cursor, so only a user cursor of
that exact name can collide; the 42P03 is then reported as it is. A fixed name
keeps the DECLARE, FETCH, and CLOSE texts stable, which matters on servers
that track statements by text in `pg_stat_statements`.

One `FETCH` per execution: `FETCH FORWARD <rowLimit + 1>` with a limit,
`FETCH ALL` without. PostgreSQL discards a cancel request that arrives while
the backend is waiting for a command, so fetching in chunks would lose a Stop
that lands between chunks. A single FETCH is also what keeps
`statement_timeout` bounding the whole read and keeps the backend from
sitting idle in a transaction while the frontend holds back credit.
Backpressure works inside one FETCH through the existing capacity-one handoff.

- **Autocommit mode, session idle**: `BEGIN`, declare, fetch, `CLOSE`,
  `COMMIT`. The transaction uses the server defaults, so a read-only
  connection stays read-only.
- **Manual mode, session idle**: the existing `BEGIN ISOLATION LEVEL ...`,
  then declare, fetch, `CLOSE`. No commit; the user owns the transaction.
- **Transaction already active**: declare, fetch, `CLOSE` inside it. A server
  error aborts the user's transaction, as the same statement failing directly
  would.
- **Failed or unknown status**: unchanged admission and server behavior.

Exit rules for the autocommit wrapper, in priority order:

1. Session closed or the event channel gone: send nothing further and return.
   Dropping the session drops the socket, and the server rolls back. Never
   `COMMIT` on this path.
2. Any error, or a cancel observed between statements: `ROLLBACK`, bounded by
   the 3-second timeout `close_session` uses. If the `ROLLBACK` itself is
   answered with 57014 from a late cancel, send it once more.
3. Success: `CLOSE`, then `COMMIT`. `ResultSetCompleted` is emitted only after
   `COMMIT` succeeds, so a commit failure is never reported after a complete
   result. A failed `COMMIT` is followed by `ROLLBACK`.

In every case the driver's `Finished` event follows cleanup, so the observer
never samples mid-cleanup. Once cleanup starts, `cancel` reports
`requested: false` and sends nothing, so a late Stop cannot hit the wrapper's
own `COMMIT` or `ROLLBACK`.

The wrapper relies on one invariant: a cached `Idle` status means the backend
is outside a transaction. If it were stale, `BEGIN` would only warn and the
wrapper `COMMIT` would commit the user's work. Every path that can change
transaction state already ends in an observer probe; Step 5 adds a test that
enumerates those paths, and an execution refuses to start the wrapper when the
cached status is anything but `Idle`.

The observer still decides the reported status after every execution. If
wrapper cleanup fails, the observer reports the real state and the existing
Rollback and Recheck actions recover it.

### Known costs of a cursor

These are accepted, recorded in the ADR, and measured in Step 6:

- PostgreSQL never uses a parallel plan for `DECLARE CURSOR`.
- The planner optimizes a cursor for its first rows
  (`cursor_tuple_fraction`, default 0.1). That is right for a row-limited
  read and wrong for a read that runs to exhaustion. For a read **without** a
  limit in the autocommit wrapper, the wrapper issues
  `SET LOCAL cursor_tuple_fraction = 1` after `BEGIN`; it reverts at the end
  of the wrapper transaction. Inside a user's transaction the setting is left
  alone, because `SET LOCAL` there would outlive the execution.
- A parameterized aggregate over a large table can therefore be slower than
  the same statement with literals on the Script shape.
- Volatile functions in a row-limited read run only for the rows fetched.

### Row limit

`rowLimit` is an optional integer from 1 to 10,000 (the existing per Result
Set retention cap).

- **Cursor read with a limit**: one `FETCH FORWARD <rowLimit + 1>`. Deliver
  at most `rowLimit` rows; the extra probe row only proves more exist, is
  dropped, and adds no truncation reason. The execution completes when the
  FETCH does, so the probe can delay completion after the limit rows are
  visible.
- **Cursor read without a limit** (parameters only): `FETCH ALL`. Rows past
  the retention caps are counted and dropped, as today. Stopping early is only
  ever the result of an explicit limit.
- **Script with a limit** (not cursor-eligible): retain at most `rowLimit`
  rows per Result Set and keep reading. Dropped rows count toward
  `omittedRows` with truncation reason `rowLimit`. The limit applies to every
  Result Set of the script, including `EXPLAIN` text; activation must not send
  a limit with an EXPLAIN run.
- **Bound command**: the limit does not apply.

`ResultSetCompleted` gains `limit: null | "stopped" | "drained"`:

- `stopped`: the server stopped at the limit and more rows exist. `rowCount`
  is the number of rows seen, excluding the probe, and is a lower bound.
- `drained`: every row was read and the limit withheld some. `rowCount` is
  the exact total.
- `null`: the limit withheld nothing.

`rowCount` always counts rows seen, including rows dropped by byte caps,
which is what `reduce_stream` reports today. The frontend currently adds
`omittedRows` to the summed `rowCount`
(`src/lib/store/query-sessions.ts:218-222`), which double counts dropped rows
whenever truncation happens. That is an existing frontend defect; it is
recorded in the register and left for the activation plan.

### Bound command

Prepare the rewritten statement before opening any manual-mode transaction.
If `Statement::columns()` is non-empty, settle the execution as failed with
`refusal: "parametersReturnRows"` and no database error; nothing has run. If
empty, open the manual transaction when the session needs one, run the
statement, and emit one command-only Result Set whose `rowCount` is the
affected-row count. The prepare and execute futures are never dropped
mid-flight; a cancel during them is handled by the server's answer. No
statement prepared for user SQL outlives the execution; the driver's own
`typeinfo` statements are outside this rule.

### Driver errors on the new paths

The cursor and bound paths can produce driver errors that are not server
errors: a parameter-count mismatch, an encoding failure, an unexpected
message. Mapping those to `ConnectionLost` would retire a healthy session.
On these paths a non-server error becomes a typed failed execution unless
`Client::is_closed()` is true, in which case it is `ConnectionLost` as today.
The Script path's mapping is unchanged.

### Cancellation outcome

- `cancel` sets a per-execution flag under the credit lock before it sends the
  cancel request.
- The cursor and bound paths check the flag before each statement they send.
- The terminal status is `cancelled` when the flag is set for this execution
  and either the terminal error is SQLSTATE 57014 or the flag stopped the
  execution before a statement was sent.
- A 57014 without the flag (a statement timeout) stays `failed`.
- In a user's transaction, a cancel observed between statements closes the
  cursor if it was declared and leaves the transaction active; a 57014 leaves
  it failed, as today.

Residual, documented and not fixed: the cancel request is a separate
connection and returns once written, so a cancel can still arrive after the
statement finished and be discarded, or coincide with a statement timeout and
be reported as `cancelled`.

Accepted behavior change: the frontend already treats `cancelled` as "no
history entry, no console event" (`relational-queries.ts:273-279`, covered by
`query-sessions.test.ts:621-678`). Today a Stop records an error entry
instead. After this plan a stopped execution leaves no history entry, even if
part of a script had already taken effect. Whether history should record
cancellations is a question for the activation plan.

### Credit loop repair

`send_with_credit` checks `closed` on every pass and returns the same error
the other send helpers return, and it registers its `Notified` future before
releasing the credit lock. With that, a closed or expired session's execution
task ends, the `Arc<Session>` is released, and the socket closes. This is
required for the wrapper's first exit rule and also fixes the same leak on the
Script shape.

`refresh`, `set_mode`, and `set_isolation` do not check for a running
execution today. A Recheck during a wrapper caches `Active` until the
execution's own probe corrects it. That is harmless (it only makes
`close_session` send an extra `ROLLBACK`) and is left unchanged.

## Wire contract changes

A payload without the new fields produces the same event sequence as at
`49c50e8`, apart from the two new always-present event fields carrying their
default values and the cancellation status.

`execute_query_session` payload gains:

```ts
parameters?: Array<{ name: string; value: string | null }>;
rowLimit?: number; // 1..=10000
```

New pure command, no database access:

```ts
describe_query_parameters({ sql: string }) => { names: string[] }
```

`names` is in first-appearance order and uses the same scan as execution.

`QuerySessionError` gains two variants, both returned before the policy check:

- `parametersRejected { reason, names }` with reasons `unlexable`,
  `multipleStatements`, `positionalPlaceholder`, `missingValue`,
  `duplicateName`, `nameTooLong`, `tooManyParameters`, `valueTooLarge`.
- `invalidRowLimit`.

Events:

- `resultSetCompleted` gains `limit: null | "stopped" | "drained"`.
- `executionCompleted.status` may be `"cancelled"`.
- `executionCompleted` gains `refusal: "parametersReturnRows" | null`.
- `truncationReasons` may contain `rowLimit`.

## Implementation sequence

### Step 1: Record the decision

Add `docs/adr/0031-statement-scoped-query-session-execution.md` covering the
shapes, the wrapper and its exit rules, the parameter grammar, the refusals,
the known costs of a cursor, and the driver facts above. Amend the Query
Execution entry in `CONTEXT.md`, which says "one simple-query request". Add
one line to ADR-0021 pointing at ADR-0031.

**Gate**: the ADR states each refusal, each transaction case, and each exit
rule in this plan.

### Step 2: Credit loop repair and cancellation flag

Do these first, on the Script shape alone, so they are verified before any new
path depends on them. Fix `send_with_credit`; add the cancellation flag, its
clearing on the next execution, and the `cancelled` status for a flagged
57014.

Tests: closing a session with four unacknowledged batches ends the execution
task and releases the connection; an ACK that arrives between the credit check
and the wait is not missed; the flag is ignored for a stale execution id; a
57014 without the flag stays `failed`. Ignored live tests: Stop during
`pg_sleep` settles `cancelled` once; a `statement_timeout` expiry settles
`failed`; after closing a session mid-stream with ACKs withheld, its backend
pid leaves `pg_stat_activity`.

**Gate**: `just fmt`, `just lint`, `just test`, the ignored live tests against
the disposable fixture (name it before starting it), and `pnpm test` to
confirm the existing frontend cancellation tests still pass.

### Step 3: Span-carrying lexer and the parameter module

Add the span-carrying scan to `sql_lex.rs` and the new
`src-tauri/src/postgres/sql_params.rs` (scan, rewrite, segment map, limits,
redacting `Debug`). Register `describe_query_parameters`.

Unit tests cover: names in strings, E-strings, dollar quotes, line and block
comments, and quoted identifiers; `::cast`; `:=`; `a[1:n]`, `a[:n]`,
`a[1 :n]`, `a[(:n)]`; `ARRAY[:a, :b]` and nested constructors;
`JSON_OBJECT('k':v)`; repeated names; first-appearance ordering; a name
directly after `(`, `,`, `=`, and whitespace; `$n` alone and mixed with names;
non-ASCII outside strings; the ambiguous plain-string backslash case; position
translation before, inside, and after a rewritten placeholder with multibyte
text earlier in the statement; every limit at and just past its boundary;
duplicate supplied names; a 64-byte name. Existing `sql_class`,
`result_mutation`, and `object_ddl` tests pass unchanged.

**Gate**: `just fmt`, `just lint`, `just test`.

### Step 4: Shape planner and policy input

Add a pure function from `(sql, parameter mode, row limit)` to a shape plus
the text to classify, or a typed refusal. Expose from `sql_class` what it
needs (statement spans, class, head keyword, locking clause, native
placeholder).

Table-driven tests cover: single read; `WITH` read; `WITH` containing a write;
`SELECT ... INTO`; a read calling a denylisted function; `SELECT ... FOR
UPDATE` and `FOR SHARE` at depth 0 and in a subquery; `SHOW`; `EXPLAIN`; DML;
two statements; a leading semicolon; an empty script; an unlexable script;
each with and without names in the SQL, with `parameters` absent, empty, and
populated, and with and without a limit. Policy tests cover parameter names
that are keywords: `:where` in an unbounded `UPDATE` still needs confirmation
in protected mode; `:into`, `:update`, and `:nextval` do not escalate a read.

**Gate**: `just fmt`, `just lint`, `just test`.

### Step 5: Driver paths and actor integration

In `query_session/postgres.rs`: the text-format parameter type; the row
reducer extracted from `reduce_stream` so both shapes share one
implementation; the cursor stream and the bound command, both emitting the
existing `DriverEvent`s through the capacity-one handoff; the typed mapping
for non-server driver errors. Spike `Client::execute_typed` with unspecified
types for the `DECLARE`; use it only if it preserves type inference and error
positions, otherwise keep prepare-then-execute.

In `query_session/mod.rs` and `protocol.rs`: extend `ExecutePayload`,
validate, plan, branch in `run_execution`, implement the wrapper and its exit
rules, the manual-mode ordering for bound commands, the refusal terminal, the
flag checks between statements, and the cleanup-started guard in `cancel`.
Keep lock ordering as it is: sequence, then closed, then credit, then
transaction. Update the `ExecutePayload` literal in `safety/live.rs`.

Unit tests: the reducer extraction leaves the Script shape's limits unchanged
(same rows retained, same counters, same batch boundaries for the same input);
admission returns each refusal before the policy check; the idle-status
invariant holds for every state-changing path.

Ignored live tests, alongside the existing `requires pnpm db:postgres` ones:
each spike row; a limit equal to the row count (`limit: null`) and one below
it (`stopped`); a zero-row cursor read (columns still reported); rows that hit
the byte caps before the limit; the autocommit wrapper leaves the session idle
after success, server error, cancel during FETCH, cancel before DECLARE, and a
limit stop; manual mode leaves the transaction active with nothing in
`pg_cursors`; an error inside a user transaction leaves it failed; a temp
table created on the Script shape is visible to a cursor read; a user cursor
named `dbunk_query_cursor` yields 42P03 and no leaked wrapper transaction;
closing the session mid-fetch with ACKs withheld settles once and the backend
pid disappears; a parameter-count or encoding failure fails the execution and
the session stays usable; no user statement remains in
`pg_prepared_statements` after success, error, cancel, and refusal.

**Gate**: `just fmt`, `just lint`, `just test`, the ignored live tests, and
`pnpm format`, `pnpm lint`, `pnpm typecheck`, `pnpm test`.

### Step 6: Failure and boundedness validation

Against the disposable fixture, and against PostgreSQL 17 if a fixture is
available, record:

- The Script shape's event sequence for a fixed multi-statement script before
  and after this plan, compared field by field.
- Wall time and bytes received for a 3M-row read with no limit, with
  `rowLimit: 200` on the cursor shape, and with `rowLimit: 200` on a
  non-eligible statement, so "stopped" versus "drained" is measured.
- Plan and wall time for a parallel-eligible aggregate and for a large sorted
  read, each as a literal Script execution and as a parameterized cursor read,
  with `EXPLAIN` output for both. Report the difference as measured; do not
  set a pass threshold after the fact.
- Credit backpressure on the cursor shape: with ACKs withheld, the reducer
  stops after four batches and the socket stops being read.
- Cancel and close during `BEGIN`, `DECLARE`, `FETCH`, `CLOSE`, and the
  wrapper `COMMIT`. Each settles exactly once and leaves no open transaction
  that the observer does not report.
- `default_transaction_read_only = on`; a `statement_timeout` expiry during
  `FETCH` (stays `failed`); `idle_in_transaction_session_timeout` set to a
  small value with ACKs withheld during a FETCH (the backend is active, so it
  must not fire); a connection drop mid-fetch (existing `sessionLost` path).
- The TLS fixture and one SSH-tunnel route.
- `pg_stat_statements`, if the fixture can load it: the number of distinct
  entries added by repeated cursor reads.

Record what could not be run and why. Do not claim a platform or version that
was not tested.

### Step 7: Completion

Run every required check. Update the PAR-001 entry in
`plans/parity-gap-register.md`, the bind-variable row in `ROADMAP.md`, and
this plan's execution record with exact evidence and limits. Set the README
row to `READY FOR REVIEW`. PAR-001 stays Partial.

## Expected file scope

- New: `src-tauri/src/postgres/sql_params.rs`,
  `docs/adr/0031-statement-scoped-query-session-execution.md`.
- Changed: `src-tauri/src/postgres/sql_lex.rs`, `sql_class.rs`, `mod.rs`
  (module registration); `src-tauri/src/query_session/mod.rs`, `postgres.rs`,
  `protocol.rs`; `src-tauri/src/commands/query_session.rs`;
  `src-tauri/src/safety/live.rs` (test helper literal only); the command
  registration list in `src-tauri/src/lib.rs`; `CONTEXT.md`;
  `docs/adr/0021-dedicated-postgres-query-session-driver.md` (one pointer
  line); `plans/README.md`, `plans/parity-gap-register.md`, `ROADMAP.md`, and
  this plan.
- Not changed: anything under `src/`, `Cargo.toml`, `Cargo.lock`, the
  observer, `dedicated.rs`, Table Browse, Result Mutation, `object_ddl`, the
  SQLx `run_query` path.

## Done criteria

- [x] An execution without `parameters` or `rowLimit` produces the same events
      as at `49c50e8`, apart from the default-valued new fields and
      `cancelled` for a requested cancel.
- [x] A closed or expired session's execution task ends and its backend
      connection closes, with a full credit window. The `ackTimeout` expiry
      shares the close path and was not run end to end (120-second lease).
- [x] Named parameters are found only where the grammar above allows, and
      `describe_query_parameters` agrees with what execution binds.
- [x] The policy classifies the rewritten text, so a parameter name cannot
      change a statement's class.
- [x] Values reach the server only as bound text-format parameters and appear
      in no log, event, error, or audit row that this code writes. A server
      error can quote the value it rejects (22P02); that message is shown to
      the user who supplied it and is not logged.
- [x] A cursor read with a limit stops the server after `rowLimit + 1` rows
      and reports `limit` truthfully at the exact boundary.
- [x] No shape leaves a cursor, a user prepared statement, or a wrapper
      transaction behind on success, error, cancel, limit stop, refusal, or
      close.
- [x] A Stop on any shape is either honored or reported as not requested.
      Once cleanup starts no cancel request is sent; one sent just before can
      still land on cleanup, which the `ROLLBACK` retry and the
      rollback-after-failed-`COMMIT` absorb.
- [x] Driver-side failures on the new paths do not retire a healthy session.
- [x] Server error positions map back to the user's original text.
- [x] All Rust and frontend gates pass and the live results are recorded.

## STOP conditions

Stop and report if:

- The drift check shows changes in the listed files.
- Text-format binding fails for a type family in the live matrix, or the
  server's inferred type for a common case needs a client-side cast.
- The shared reducer cannot be extracted without changing a Script-shape
  limit, counter, or batch boundary.
- Any failure path can leave a wrapper transaction open while the session
  reports idle and the observer cannot see it, or can `COMMIT` after a close.
- The idle-status invariant has a path that does not end in an observer probe.
- An existing frontend test fails after Step 2, or honest cancellation needs a
  frontend change.
- A dependency version or feature has to change, or a driver fork is needed.
- The position map cannot be made exact for multibyte text.
- A required check fails twice, or a change is needed outside the file scope.

## Deferred work

- **Activation** (next plan): send `parameters` from the editor's existing
  bind panel, use `describe_query_parameters` instead of the regular
  expression, add a row-limit control and the `limit` indicator, render
  refusals, decide whether parameter values and cancellations are stored in
  query history, and fix the `rowCount + omittedRows` double count. It
  introduces visible controls, so it needs the usual mock selection. Mutation
  Analysis (ADR-0023) prepares the executed SQL on its own socket, so
  activation must hand it the rewritten `$n` statement, not the `:name` text.
- **EXPLAIN with parameters**: EXPLAIN is not cursor-eligible and returns
  rows, so this plan refuses it. Its result columns are `text` or `json`,
  whose binary and text encodings are the same bytes, which makes a narrow
  extended-protocol result path possible. Decide in the activation plan.
- **Parameters with `RETURNING`**: refused here for the same binary-result
  reason.
- **Parameterized row-locking reads** (`SELECT ... FOR UPDATE` with
  parameters): they return rows and are not cursor-eligible, so they are
  refused as `parametersReturnRows`.
- **Script error policy** (stop, continue, prompt): needs backend statement
  splitting with spans and a per-statement event model; continuing inside a
  transaction needs savepoints.
- **Savepoint controls**: the server exposes no savepoint list, so the stack
  is client-tracked and can diverge from raw SQL. Needs its own design.
- **Command tags**: blocked on the driver. Options are an upstream change or a
  fork; neither is in scope.
- **Fetch more**: keeping a cursor open between executions holds a
  transaction open. Out of scope, as it was for Plan 002.
- Non-ASCII identifiers outside strings still make a script unlexable, which
  is an existing classifier limit and is not changed here.

## Review record

One independent adversarial review ran on 2026-10-01 against the first draft,
reading the code and the vendored driver without running anything. It
confirmed the cited line ranges and driver claims and raised two blockers and
eight major findings. Amendments made:

- Chunked `FETCH` lost cancels between chunks and changed the meaning of both
  timeouts: replaced by one `FETCH` per execution, with flag checks between
  wrapper statements.
- `send_with_credit` parks forever after close: added "Credit loop repair" as
  Step 2 and a done criterion.
- Classifying `:name` text let parameter names act as keywords: the policy now
  classifies the rewritten text, with tests.
- Non-server driver errors retired the session: typed mapping on the new
  paths; native `$n` made ineligible; the spike row about the count mismatch
  corrected.
- `SELECT ... FOR UPDATE` was cursor-eligible: excluded.
- The wrapper was called equivalent to the implicit transaction: replaced by
  "Known costs of a cursor" and a measured comparison in Step 6.
- Wrapper exit paths were underspecified: explicit exit rules, completion
  after `COMMIT`, cleanup-started guard, the idle-status invariant.
- The bracket rule blocked `ARRAY[:a]` and contradicted itself: replaced by
  the preceding-character rule and constructor versus subscript brackets.
- "Has parameters" was undefined: defined by the scan, with parameter mode
  keyed on the payload field.
- File scope missed `safety/live.rs` and the drift check could not pass:
  both fixed.
- Smaller items: fixed cursor name, `limit` as a three-state field, row-count
  definition, duplicate and over-long names, validation before policy,
  redacting `Debug`, prepared-statement wording, the accepted change to
  cancelled history entries.

The review's claims about PostgreSQL behavior (cancel discarded while idle, no
parallel plans under a cursor, row locks taken at fetch) come from the
reviewer's knowledge of the server and were not re-tested here. Steps 2, 5,
and 6 test them.

## Execution record (2026-10-01)

Steps 1 to 7 are complete and uncommitted. `main` was rewritten from `49c50e8`
to `677a7e8` during execution by something outside this work; the two trees
are identical, so the drift check holds, but `49c50e8` is no longer on `main`.

### Gates

- `just fmt`, `just lint`, `just test`: 651 passed, 79 ignored (614 and 63
  before the plan).
- `pnpm format`, `pnpm lint`, `pnpm typecheck`, `pnpm test`: 1,488 passed. No
  file under `src/` changed.
- Live, disposable fixtures only, macOS only:
  - PostgreSQL 16.14 (`infrastructure/test-db` profile `postgres`,
    `127.0.0.1:15432`) and the TLS fixture (`postgres-tls`,
    `127.0.0.1:15433`): 29 ignored tests pass (21 Query Session, 3
    `safety_live`, 5 `dedicated_live`), three consecutive runs.
  - PostgreSQL 17.10 (`postgres:17-alpine`, throwaway container on 15432):
    the 19 fixture-port Query Session tests pass twice on the final code.
- Step 2 defects were reproduced on the unfixed code first: a Stop during
  `pg_sleep` settled `failed`, and a session closed with four unacknowledged
  batches kept its backend past a 10-second wait.

### Step 6 results

- **Script event sequence**: six scripts dumped at `49c50e8` and after. 102
  envelopes, identical once `limit: null` and `refusal: null` are removed.
- **Measurements, plans and `pg_stat_statements`**: in ADR-0031 "Validation".
- **Credit on the cursor shape**: the first build streamed the `FETCH` under
  credit. With ACKs withheld the reducer stopped after four batches, but the
  server finished writing about 9 MB into the network path within 1.4 s and
  went `idle in transaction`; a 300 ms
  `idle_in_transaction_session_timeout` then ended the session. This
  contradicted "the backend is active, so it must not fire". Imran chose the
  eager `FETCH` on 2026-10-01. With it, a 1,000 ms timeout and ACKs withheld
  for 2,000 ms leave the session idle and alive, and the execution completes
  once acknowledged.
- **Cancel and close at each statement**: each checkpoint (before `BEGIN`,
  `DECLARE`, `FETCH`, and at cleanup) has a deterministic live test for a Stop
  and for a close, in the wrapper and in a user's transaction. Real cancel
  requests at 0 to 1,200 microsecond delays: more than 5,000 timed Stops, each
  settled once, idle afterwards, `cancelled` when the request was accepted and
  `completed` when it was not. A real cancel was only observed landing on the
  `FETCH`; none landed on `BEGIN`, `DECLARE`, `CLOSE` or `COMMIT`, which each
  take well under a millisecond on a local fixture. 40 timed closes each
  released the backend with nothing after `sessionClosed`.
- **Read-only, timeouts, connection loss**: `default_transaction_read_only`
  allows the cursor read and the server refuses a bound write with 25006; a
  `statement_timeout` during `FETCH` settles `failed`; a socket cut during a
  `FETCH` takes the existing `sessionLost` path.
- **TLS**: cursor read, bound command, and a Stop during `FETCH` over the TLS
  fixture.

### Not run

- An SSH-tunnel route: the repository has no SSH fixture.
- PostgreSQL 18, and any platform other than macOS.
- A driver-side failure through the actor: the planner always binds exactly
  the scanned names, so a count mismatch cannot be provoked there. The mapping
  is tested against a real driver error instead.

### Departures from the plan as written

- **Eager `FETCH`** replaces socket-level backpressure on the cursor shape
  (decided by Imran). Exit rule 1 is enforced through the closed flag; a
  vanished event channel is found when rows are delivered, after the `COMMIT`.
- **Type inference**: `:x IS NULL OR col = :x` needs `:x::text IS NULL`
  (42804 at `FETCH`), and a lone `:x IS NULL` is 42P18. Imran accepted this as
  a known limit rather than a STOP on 2026-10-01.
- **`execute_typed`** for the `DECLARE`: the spike showed the same inference
  and error positions, one round trip, and no named statement.
- **Where the sequence lives**: the wrapper, exit rules and flag checks are in
  `query_session/postgres.rs` behind an `ExecutionControl` trait the session
  implements, not in `mod.rs`, so each exit rule is testable on its own.
- **Probe order**: observer probes are applied in the order they started. A
  Recheck that began before an execution finished could otherwise overwrite
  that execution's status with a stale `Idle`, and the next cursor read would
  wrap and commit the user's transaction. Not in the plan; found while
  checking the idle-status invariant.
- **`wait_for_row_credit`** had the same missed-wakeup gap as
  `send_with_credit` and was fixed with it. Event delivery state moved into an
  `Outbox` so the credit and cancel rules have unit tests.
- **Bracket rule**: a bracket inside a constructor is a nested row only after
  `[` or `,`, so `ARRAY[a[:n]]` stays a slice.
- **Unused supplied names** are ignored for the duplicate and size checks too.
- **Cleanup**: `CLOSE` and `COMMIT` are one request, and every cleanup
  statement is bounded by the 3-second timeout, because cleanup holds the lock
  that a close needs.
- **Script with a limit** delivers its retained rows when the limit is
  reached, not after draining the result.
- **`src-tauri/src/lib.rs`** gained a `TOKIO_POSTGRES_LOG_LEVEL` constant
  beside the command registration, so a test can assert the Warn pin.
- **The planner** is in `sql_params.rs`, to stay inside the file scope.
- **The idle-status invariant** is checked by live tests that compare the
  cached status with `pg_stat_activity` after every state-changing path, not
  by a unit test.
- **A `FETCH` error delivers no rows** and an unlimited cursor read spools the
  whole result to server temp files; both follow from the server
  materializing a `FETCH`. Recorded in ADR-0031 "Known costs and limits".

### For the activation plan

- How to present the type-inference limit: an error hint, or optional
  per-parameter type names in the wire contract.
- An unlimited parameterized read costs more than its literal form (serial
  plan, full materialization). Sending a row limit with parameterized reads
  avoids the transfer but not the planning cost.
- A cancel request that reaches the backend during the next execution is
  reported as that execution's failure. This predates the plan.

