# ADR-0031: Statement-scoped Query Session execution

**Status**: Accepted and implemented (Plan 023, `PAR-001`, 2026-10-01).
Extends ADR-0021. The parameter and row-limit behavior is dark: it is
reachable only through optional payload fields that no frontend caller sends.
Validated against PostgreSQL 16.14 and 17.10 and the TLS fixture, on macOS
only. "Amendments from validation" lists where the implementation departs
from the first draft of this decision and why.

## Problem

A Query Execution was one simple-protocol request. That left three gaps.

- Bind variables were substituted into SQL text by a frontend regular
  expression, which matches `:name` inside strings, comments and array slices
  and guesses value types from their shape. The substituted text is what ran.
- A read could not be stopped at a row count. Once retention limits were
  reached the driver kept reading and counting, so the server sent the whole
  result. On the fixture a 3M-row read took 1,496 ms to transfer; a cursor
  fetch of 201 rows took 8 ms (one run, one machine, an order of magnitude).
- A user Stop settled as `failed` with SQLSTATE 57014, and an execution parked
  on a full credit window never noticed that its session had closed, so it
  kept the socket.

## Decision

A PostgreSQL Query Session runs one statement with driver-bound parameters,
and stops a read at a requested row count, through a server-side cursor. The
statement is declared through the extended protocol with text-format bound
values, and rows are fetched through the simple protocol, which keeps the
server-rendered text results the session contract promises.

### Parameter mode and the three shapes

A payload is in parameter mode when it carries a `parameters` field, even an
empty one. Only then does the backend scan the SQL for named parameters. "Has
parameters" always means the scan found at least one name in the SQL, never
that the payload supplied values. A payload without the field is not scanned,
so a stray `:name` reaches the server unchanged.

The manager picks one shape from the SQL text and the payload before anything
is sent. It never retries a statement in another shape after the server has
seen it.

1. **Script**: one simple-protocol request. Used when the SQL has no
   parameters and either no row limit or a statement that is not
   cursor-eligible.
2. **Cursor read**: used when the SQL is cursor-eligible and has parameters, a
   row limit, or both.
3. **Bound command**: used when the SQL has parameters, is exactly one
   statement, and is not cursor-eligible.

Cursor-eligible means all of:

- the script lexes and contains exactly one statement;
- after parameter rewriting, `classify_statement` returns `Read`;
- the unquoted head keyword is `SELECT`, `VALUES`, `TABLE`, or `WITH`;
- it has no row-locking clause (`FOR` followed by `UPDATE`, `NO`, `SHARE`, or
  `KEY`) at any depth, because a cursor locks rows as they are fetched and a
  row limit would lock a different set than the statement names;
- it has no native `$n` placeholder.

`SHOW`, `EXPLAIN`, `SELECT ... INTO`, statements with a denylisted function,
and statements with a parenthesized or quoted head are therefore not eligible.
The cursor text is built from the statement's own span, so leading or trailing
semicolons and whitespace never follow `FOR`. A native `$n` with only a row
limit stays on the Script shape, where the server answers 42P02 as before.

### Parameter grammar

A parameter is `:` followed by `[A-Za-z_][A-Za-z0-9_]*`, outside every string,
E-string, dollar quote, comment and quoted identifier, unless:

- the character directly before the colon is an identifier character, a digit,
  a closing quote, `)`, `]`, or another `:`. This excludes casts, `a[1:n]`, and
  the PostgreSQL 16 `JSON_OBJECT('k':v)` and `k:v` forms; or
- the innermost enclosing bracket or parenthesis is a subscript bracket. A
  bracket is a constructor bracket when it directly follows the `ARRAY` keyword
  or starts a nested row directly inside a constructor bracket, which means it
  follows `[` or `,` there; every other bracket is a subscript.
  `ARRAY[:a, :b]`, `ARRAY[[:a], [:b]]` and `arr[(:n)]` take parameters;
  `arr[:n]`, `arr[1 :n]` and `ARRAY[arr[:n]]` are slices.

The name is the whole unquoted identifier glued to the colon. An identifier
containing `$` is outside the grammar, so `:a$b` is left for the server.

The scan reuses the classifier's lexer regions through a span-carrying scan,
so the two cannot disagree about where a string or comment ends. Names are
case-sensitive. Each distinct name becomes `$k` in order of first appearance;
repeats reuse the same `$k`. The rewrite keeps a segment map, in characters,
so a server error position translates back to the user's text after the cursor
prefix length is subtracted. A position inside the prefix is dropped.

Values are `string | null`, sent in text format; the server infers each type.
Supplied names the statement does not use are ignored entirely, including for
the duplicate and size checks, because the editor keeps values for every name
in the tab. Values never appear in logs, errors,
events or audit rows: the payload and parameter types redact them in `Debug`,
and the driver's own Debug logging stays behind the existing Warn pin for
`tokio_postgres`. Names may appear, because they come from the SQL text.

### Refusals

Returned by the command before the policy check and before session lookup.
Nothing is sent to the server.

- `parametersRejected { reason, names }`:
  - `unlexable`: parameter mode, and the scan could not lex the SQL.
  - `multipleStatements`: the SQL has parameters and more than one statement.
    The extended protocol runs one statement per request, and splitting a
    script changes its implicit transaction.
  - `positionalPlaceholder`: a native `$n` in SQL that also has named
    parameters.
  - `missingValue`: a name the statement uses has no supplied value.
  - `duplicateName`: a name is supplied twice.
  - `nameTooLong`: a name over 63 bytes. It is refused, not truncated.
  - `tooManyParameters`: more than 256 distinct names in the statement.
  - `valueTooLarge`: a value over 1 MiB, or more than 4 MiB across all values.
- `invalidRowLimit`: `rowLimit` outside 1 to 10,000.

Settled as a failed execution after admission, with no database error:

- `refusal: "parametersReturnRows"`: a Bound command whose prepared statement
  describes result columns. It is prepared and not run. This covers `EXPLAIN`
  with parameters, parameterized `RETURNING`, and parameterized row-locking
  reads, because the driver can only return extended-protocol results in
  binary.

Validation runs before the policy so a user is never asked to confirm a write
that is then refused for a missing value. In parameter mode the policy
classifies the rewritten text, in which each parameter is an opaque `$k`.
Classifying the `:name` text would let a name act as a keyword:
`UPDATE t SET a = :where` would look bounded while the server runs an
unbounded update. The success hook and audit disposition are unchanged for all
shapes and fire only on `completed`.

### Cursor read lifecycle

The cursor name is the reserved constant `dbunk_query_cursor`. A session runs
one execution at a time and always closes the cursor, so only a user cursor of
that exact name collides; the 42P03 is reported as it is. A fixed name keeps
the `DECLARE`, `FETCH` and `CLOSE` texts stable for `pg_stat_statements`.

The `DECLARE` is one extended-protocol round trip through an unnamed
statement with every parameter type left unspecified
(`Client::execute_typed` with `unknown`), so the server infers the types and
nothing is prepared that could outlive the execution.

One `FETCH` per execution: `FETCH FORWARD <rowLimit + 1>` with a limit,
`FETCH ALL` without. PostgreSQL discards a cancel request that arrives while
the backend waits for a command, so fetching in chunks would lose a Stop
between chunks. A single `FETCH` also keeps `statement_timeout` bounding the
whole read.

The `FETCH` is read to its end into memory before any row is handed to the
frontend. What is kept is bounded by the retention limits (10,000 rows per
Result Set, 32 MiB per execution). Cleanup follows at once, and only then are
the rows delivered under frontend credit. Credit therefore holds back rows and
never a transaction: a frontend that is slow to acknowledge cannot keep the
wrapper open, pin its snapshot, or trip `idle_in_transaction_session_timeout`.
The Script shape keeps its socket-level backpressure.

Transaction cases:

- **Autocommit mode, session idle**: `BEGIN`, declare, fetch, `CLOSE`,
  `COMMIT`. The wrapper transaction uses the server defaults, so a read-only
  connection stays read-only.
- **Manual mode, session idle**: the existing `BEGIN ISOLATION LEVEL ...`,
  then declare, fetch, `CLOSE`. No commit; the user owns the transaction.
- **Transaction already active**: declare, fetch, `CLOSE` inside it. A server
  error aborts the user's transaction, as the same statement failing directly
  would.
- **Failed or unknown status**: unchanged admission and server behavior.

Exit rules for the autocommit wrapper, in priority order:

1. **Session closed or the event channel gone**: send nothing further and
   return. Dropping the session drops the socket and the server rolls back.
   Never `COMMIT` on this path. Cleanup holds the lock that guards the closed
   flag, so a close waits for cleanup in progress and none starts after it. A
   vanished event channel closes the session, which is how it is seen here;
   because rows are delivered after cleanup, one that vanishes during delivery
   is found after the `COMMIT`.
2. **Any error, or a cancel observed between statements**: `ROLLBACK`, bounded
   by the 3-second timeout `close_session` uses. If that `ROLLBACK` is itself
   answered with 57014 from a late cancel, it is sent once more.
3. **Success**: `CLOSE`, then `COMMIT`, sent as one request and bounded by
   the same timeout. `ResultSetCompleted` is emitted only after `COMMIT`
   succeeds, so a commit failure is never reported after a complete result. A
   failed `COMMIT` is followed by `ROLLBACK`.

In every case the driver's `Finished` event follows cleanup, so the observer
never samples mid-cleanup. Once cleanup starts, `cancel` reports
`requested: false` and sends nothing, so a late Stop cannot hit the wrapper's
own `COMMIT` or `ROLLBACK`.

The wrapper relies on one invariant: a cached `Idle` status means the backend
is outside a transaction. If it were stale, `BEGIN` would only warn and the
wrapper `COMMIT` would commit the user's work. Every path that can change
transaction state ends in an observer probe, and an execution does not start
the wrapper when the cached status is anything but `Idle`. Probes are applied
in the order they started: a Recheck that began before an execution finished
can return after that execution's own probe, and applying it would cache a
stale `Idle`. The observer still
decides the reported status after every execution; if wrapper cleanup fails it
reports the real state and the existing Rollback and Recheck actions recover.

### Row limit

`rowLimit` is an optional integer from 1 to 10,000, the existing per Result
Set retention cap.

- **Cursor read with a limit**: at most `rowLimit` rows are delivered. The one
  extra probe row only proves more exist; it is dropped and adds no truncation
  reason. The execution completes when the `FETCH` does.
- **Cursor read without a limit**: `FETCH ALL`. Rows past the retention caps
  are counted and dropped, as on the Script shape. Stopping early is only ever
  the result of an explicit limit.
- **Script with a limit** (not cursor-eligible): at most `rowLimit` rows are
  retained per Result Set and the driver keeps reading. The retained rows are
  delivered as soon as the limit is reached. Dropped rows count toward
  `omittedRows` with truncation reason `rowLimit`. The limit applies to every
  Result Set of the script.
- **Bound command**: the limit does not apply.

`ResultSetCompleted` carries `limit`:

- `stopped`: the server stopped at the limit and more rows exist. `rowCount`
  excludes the probe and is a lower bound.
- `drained`: every row was read and the limit withheld some. `rowCount` is the
  exact total.
- `null`: the limit withheld nothing.

`rowCount` always counts rows seen, including rows dropped by byte caps.

### Bound command

The rewritten statement is prepared before any manual-mode transaction is
opened. With result columns it is refused as above. Without them the manual
transaction is opened when the session needs one, the statement runs, and one
command-only Result Set reports the affected-row count. The prepare and
execute futures are never dropped mid-flight; a cancel during them is handled
by the server's answer. No statement prepared for user SQL outlives the
execution. The driver's own `typeinfo` statements are outside that rule.

### Driver errors on the new paths

The cursor and bound paths can fail in the driver without a server error: a
parameter-count mismatch, an encoding failure, an unexpected message. Such an
error becomes a failed execution whose error has no SQLSTATE, unless
`Client::is_closed()` is true, in which case it is `ConnectionLost` as before.
The Script shape keeps its mapping, where every non-server error retires the
session. A cleanup statement that outlasts its timeout fails the execution and
leaves the status to the observer.

### Cancellation outcome

- `cancel` sets a per-execution flag under the credit lock before it sends the
  cancel request. Admitting the next execution clears it.
- The cursor and bound paths check the flag before each statement they send.
- The terminal status is `cancelled` when the flag is set for this execution
  and either the terminal error is SQLSTATE 57014 or the flag stopped the
  execution before a statement was sent.
- A 57014 without the flag, such as a statement timeout, stays `failed`.
- In a user's transaction, a cancel observed between statements closes the
  cursor if it was declared and leaves the transaction active. A 57014 leaves
  it failed, as before.

The cancel request travels on a separate connection and returns once written.
A cancel can therefore still arrive after the statement finished and be
discarded, or coincide with a statement timeout and be reported as
`cancelled`. Both are accepted.

The frontend treats `cancelled` as no history entry and no console event. A
stopped execution therefore leaves no history entry, even if part of a script
had already taken effect. Whether history should record cancellations belongs
to the activation plan.

### Credit loop repair

Every wait on the credit window checks `closed` on each pass, and arms its
wakeup before it reads the window. A closed or expired session's execution
task therefore ends, releases its reference to the session, and the socket
closes; an ACK or close that lands between the read and the wait is not
missed. The wrapper's first exit rule depends on this, and it fixes the same
leak on the Script shape.

`refresh`, `set_mode` and `set_isolation` do not check for a running
execution. A Recheck during a wrapper caches `Active` until the execution's
own probe corrects it, which only makes `close_session` send an extra
`ROLLBACK`. That is left unchanged.

## Known costs and limits

- **Types the server cannot infer.** A parameter needs a typed context. A
  lone `:x IS NULL` is refused with 42P18. `:x IS NULL OR col = :x` needs
  `:x::text IS NULL`: a cursor skips the parse-time consistency check a plain
  statement gets, so the server reports it as 42804 when the `FETCH` runs,
  without a position. Comparisons, `IN`, `BETWEEN`, `LIKE`, `coalesce`,
  `LIMIT`, arithmetic and array contexts infer correctly. How to present this
  to a user belongs to the activation plan.
- **A `FETCH` is materialized.** Over the simple protocol the server runs a
  `FETCH` to its end into a tuplestore before it sends the first row. An
  unlimited read of 3M rows spooled 654 MB to server temp files, and an error
  during a `FETCH` delivers no rows, where the Script shape delivers the rows
  before the error.
- **`rowLimit` is not a planner `LIMIT`.** The server stops producing rows but
  plans as if it would not: a sorted read still sorts everything.
- PostgreSQL never uses a parallel plan for `DECLARE CURSOR`.
- The planner optimizes a cursor for its first rows (`cursor_tuple_fraction`,
  default 0.1). That suits a row-limited read and not one that runs to
  exhaustion. For a read without a limit in the autocommit wrapper, the
  wrapper issues `SET LOCAL cursor_tuple_fraction = 1` after `BEGIN`; it
  reverts with the wrapper transaction. Inside a user's transaction the
  setting is left alone, because `SET LOCAL` there would outlive the
  execution.
- A parameterized aggregate over a large table can be slower than the same
  statement with literals on the Script shape.
- Volatile functions in a row-limited read run only for the rows fetched.

## Driver facts

Checked on 2026-10-01 against the vendored sources of tokio-postgres 0.7.18,
postgres-types 0.2.14 and postgres-protocol 0.6.12:

- `query.rs::encode_bind_raw` requests binary for every extended-protocol
  result. No public API requests text results, which is why rows come from a
  simple-protocol `FETCH`.
- `SimpleQueryMessage::CommandComplete(u64)` carries a row count only. Command
  tags stay unavailable without a driver change.
- `ToSql::encode_format` lets a parameter choose `Format::Text` per value.
- `Client::execute(&str, ..)` prepares first and only then checks the
  parameter count, so a count mismatch is a driver error after one round trip.
  A named statement is closed when its `Statement` is dropped. Dropping a
  response future does not cancel the request.

Checked in a throwaway spike against the disposable PostgreSQL 16.14 fixture:
`DECLARE ... CURSOR FOR` accepts text-format parameters and infers int4, text,
timestamptz, numeric, bool and text[] from context; `FETCH` returns
RowDescription on zero rows; a cursor cannot wrap a data-modifying statement
(42601) and needs a transaction block; `Statement::columns()` is empty for DML
without `RETURNING`; a cancel during `FETCH` inside `BEGIN` yields 57014 and
`ROLLBACK` restores an idle session.

## Validation

2026-10-01, disposable fixtures only, macOS only. PostgreSQL 16.14
(`infrastructure/test-db` profile `postgres`), 17.10 (`postgres:17-alpine`
on the same port), and the TLS fixture. Release build, median of three runs,
a 3M-row table of 690 MB:

| Read | Shape | Wall | First row | Sent by server | Server temp |
| --- | --- | ---: | ---: | ---: | ---: |
| `SELECT *`, no limit | Script | 1,256 ms | 2 ms | 670 MB | 0 |
| `SELECT *`, `rowLimit` 200 | Cursor | 10 ms | 3 ms | 45 KB | 0 |
| two statements, `rowLimit` 200 | Script, drained | 1,267 ms | 1 ms | 670 MB | 0 |
| `WHERE id > :n`, no limit | Cursor | 1,578 ms | 1,553 ms | 670 MB | 654 MB |
| `WHERE id > :n`, `rowLimit` 200 | Cursor | 21 ms | 14 ms | 45 KB | 0 |
| aggregate, literal | Script | 445 ms | | | 0 |
| aggregate, `:n` | Cursor | 806 ms | | | 0 |
| sorted, literal, no limit | Script | 3,009 ms | 1,610 ms | 53 MB | 655 MB |
| sorted, `:n`, no limit | Cursor | 4,163 ms | 4,138 ms | 53 MB | 697 MB |
| sorted, `:n`, `rowLimit` 200 | Cursor | 2,742 ms | 2,735 ms | 5 KB | 655 MB |
| sorted, literal `LIMIT 200` | Script | 291 ms | 283 ms | 4 KB | 0 |

The literal aggregate and sort used two parallel workers; under the cursor
both were serial. With `pg_stat_statements`, 25 cursor reads added 13 entries:
`BEGIN`, `CLOSE`, `COMMIT`, `SET LOCAL` and `FETCH ALL` once each, one
`DECLARE` per distinct statement (constants and parameters are normalized),
and one `FETCH FORWARD n` per distinct row limit.

The Script shape's event sequence for six scripts (caps, a zero-row set, a
notice, commands, failures mid-script and mid-result, an opened and an aborted
transaction) was dumped at the base commit and after: 102 envelopes, identical
once the two new always-null fields are removed.

Not verified: an SSH-tunnel route (the repository has no SSH fixture),
PostgreSQL 18, any platform other than macOS. A cancel request that reaches
the backend during the following execution is reported as that execution's
failure; it did not occur in more than 5,000 timed Stops.

## Amendments from validation

- **Eager `FETCH`.** The first draft streamed the `FETCH` under frontend
  credit. Once the server has written a result into the network buffers its
  backend is idle in the transaction, so a frontend withholding credit kept
  the wrapper open, and a 300 ms `idle_in_transaction_session_timeout` ended
  the session. Reading eagerly costs nothing in latency because the server
  materializes a `FETCH` anyway.
- **Probe order**, **the nested-row bracket rule** and **ignoring unused
  names entirely** are tightenings found while implementing.
- **`execute_typed` for `DECLARE`** was the plan's conditional choice; the
  spike showed the same inference and error positions as prepare-then-execute.
- The statement sequence lives in `query_session/postgres.rs` behind a small
  control trait the session implements, so each exit rule has a deterministic
  live test.

## Wire contract

A payload without the new fields produces the same event sequence as before,
apart from the two new always-present event fields carrying their defaults
and the `cancelled` status.

- `execute_query_session` payload gains `parameters?: Array<{ name, value:
  string | null }>` and `rowLimit?: number`.
- `describe_query_parameters({ sql }) => { names }` is pure, has no database
  access, and uses the same scan as execution. `names` is in first-appearance
  order.
- `QuerySessionError` gains `parametersRejected` and `invalidRowLimit`.
- `resultSetCompleted` gains `limit: null | "stopped" | "drained"`.
- `executionCompleted.status` may be `"cancelled"`, and the event gains
  `refusal: "parametersReturnRows" | null`.
- `truncationReasons` may contain `rowLimit`.

## Out of scope

Editor activation, `EXPLAIN` and `RETURNING` with parameters, parameterized
row-locking reads, script error policy, savepoint controls, command tags and
fetch-more. Non-ASCII identifiers outside strings still make a script
unlexable, which is an existing classifier limit.
