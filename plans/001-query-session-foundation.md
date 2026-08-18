# Plan 001: Add the PostgreSQL Query Session backend foundation

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving on. If a
> STOP condition occurs, stop and report without improvising. Keep the feature
> dark: Plan 002 owns frontend activation. Update this plan's row in
> `plans/README.md` to `IN PROGRESS: through Step N` after each completed step
> and to `READY FOR REVIEW` after all gates pass. A reviewer/operator records
> `DONE: <completion SHA>` only after an authorized commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat 24432fb..HEAD -- CONTEXT.md docs/adr package.json infrastructure/test-db src-tauri plans/README.md
> git status --short -- CONTEXT.md docs/adr package.json infrastructure/test-db src-tauri plans/README.md
> ```
>
> Expected on a fresh run: no source/config output; advisor-authored artifacts
> under `plans/` may be untracked. If resuming, follow "Resume protocol"
> instead. A load-bearing mismatch with the excerpts below is a STOP condition.

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `24432fb`, 2026-08-18

## Why this matters

Dbunk currently checks out a pooled SQLx connection for each editor run and
materializes one result. It cannot preserve temp state, expose multiple result
sets and notices, cancel through the PostgreSQL protocol, or represent server
transaction state truthfully. This plan lands a dark, bounded PostgreSQL
session backend with lifecycle cleanup and a typed IPC contract. Plan 002 adds
the Zustand reducer and user interface after this backend passes independently.

A PostgreSQL protocol DataRow is decoded before application truncation. This
plan bounds retained batches, queues, connection counts, and IPC payloads; it
does not claim an absolute peak-memory bound for one adversarial value.

## Current state

### Repository constraints

- Tauri 2.11.2 hosts a Rust/SQLx backend. PostgreSQL is the reference engine per
  `docs/adr/0001-postgres-first-engine-coverage.md`.
- `docs/adr/0002-foreground-health-check-tick.md` says the health tick is not
  transaction truth.
- `docs/adr/0004-last-activity-on-connection-record.md` requires activity to be
  recorded after a successful operation, not command acceptance.
- `docs/adr/0013-postgres-driver-fields-on-connection-record.md` requires the
  effective connect timeout, statement timeout, idle-in-transaction timeout,
  search path, and role behavior on every PostgreSQL connection.
- Correctness during cancellation, reconnect, partial streaming, and teardown
  outranks a lower but untrustworthy query latency.
- Never log SQL, row values, notice payloads, passwords, tokens, credentials,
  or structured database error detail.

### Backend evidence

`src-tauri/src/postgres/query.rs:16-61` creates one connection and materializes
one result:

```rust
pub async fn run_query(connection: &StoredConnection, query: &str) -> Result<QueryResult, String> {
    let mut conn = connect(connection).await?;
    // ...
    let rows = sqlx::query(query).fetch_all(&mut *conn).await?;
}
```

`src-tauri/src/postgres/pool.rs:15-26,83-85,139-155` defines a five-slot pool
per Connection. A persistent editor session must not check out and hold one of
those five slots.

`src-tauri/src/postgres/pool.rs:28-50,73-120,158-195` defines endpoint, TLS,
connect deadline, and driver-option behavior. Today `ssl: true` means Prefer:
attempt TLS, fall back to plaintext only when the server refuses TLS, and
accept an unverified certificate and hostname. Preserve this exact behavior.

`src-tauri/src/commands/mod.rs:40-81` hydrates credentials and resolves SSH
tunnels in `find_connection`. The manager must receive a temporary resolved
connect specification from this path and must not retain reusable credentials.

`src-tauri/src/commands/connections.rs:23-77` drops pools and tunnels on save,
delete, and disconnect. `commands/bastions.rs`, `commands/managed.rs`,
`managed.rs`, and `commands/settings.rs` add bastion, managed-server, and
destructive credential-reset invalidation sites. Sessions and observer sockets
must close before those targets or credentials disappear.

`src-tauri/src/postgres/admin.rs:297-320` implements SQL-level
`pg_cancel_backend(pid)`. Do not reuse it: it needs another connection, has no
protocol secret, and can race PID reuse.

`src-tauri/src/lib.rs:155-184` enables application Debug logs in development.
`tokio-postgres` logs complete simple queries at Debug, so an explicit
`tokio_postgres = Warn` filter is a security invariant.

### Verified dependency and lifecycle facts

- SQLx 0.8.6 `raw_sql().fetch_many()` exposes rows and a `PgQueryResult` per
  `CommandComplete`, but not pre-row `RowDescription` for a zero-row result,
  structured notices, or a protocol `CancelToken`.
- tokio-postgres 0.7.18 exposes `simple_query_raw`, zero-row
  `RowDescription`, `Connection::poll_message` notices, and `CancelToken`.
- Neither driver's public API exposes PostgreSQL's ReadyForQuery transaction
  status byte. Transaction truth therefore comes from an observer, not from a
  comparative driver claim.
- `SimpleQueryMessage::CommandComplete(u64)` exposes an affected/selected row
  count, not the command tag. Do not optimize observation by parsing SQL or by
  pretending BEGIN/COMMIT tags are public.
- Tauri 2.11.2 exposes `WindowEvent::Focused` and `WindowEvent::Destroyed`.
  `WebviewEvent` exposes drag/drop only, and `Channel` has no public
  receiver-closed primitive. A Channel send error and window destruction are
  definitive signals; a heartbeat is still required for a stuck renderer.

## Decided architecture

### Driver and ownership

1. Add `QuerySessionManager` to `AppState`. One actor owns one dedicated
   tokio-postgres connection for one Query Tab. SQLx remains canonical for
   table loading, metadata, admin, writes, and the legacy `run_query` command.
2. The backend API is PostgreSQL-only and returns `unsupportedEngine` for other
   engines. Do not introduce a shared engine trait before a second adapter has
   real parity.
3. Each owner is bound to an opaque process-lifetime `ownerId` and the Tauri
   window label injected by the command handler. Never trust a frontend-sent
   window label.
4. `register_query_session_owner` is called by each new renderer instance. It
   closes sessions belonging to the previous owner for that same window before
   the new owner can open sessions. This makes reload cleanup deterministic.

### Focus-aware liveness and ACK credit

5. The frontend will heartbeat every 30 seconds, but missing wall-clock
   heartbeats never roll back a transaction while its window is unfocused.
   `WindowEvent::Focused(false)` pauses owner and ACK-stall deadlines.
   `Focused(true)` grants a fresh 120-second deadline. `Destroyed`, app exit,
   owner replacement, or Channel send failure closes immediately.
   A heartbeat or any successful session command from the bound window refreshes
   owner liveness; row ACKs therefore prove liveness during active streaming.
6. A missing heartbeat or missing ACK for 120 seconds of focused time closes
   the affected session. A renderer crash while unfocused may remain until
   refocus, owner replacement, window destruction, or app exit. This is an
   explicit transaction-safety tradeoff, bounded by connection admission.
7. Use a credit window, not one sequential batch: at most four unacknowledged
   row batches and at most 4 MiB serialized unacknowledged row data per session.
   A single near-4-MiB row consumes all credit. ACKs are cumulative through a
   sequence number, so the frontend may acknowledge several reduced batches in
   one command. All row credit must be returned before the terminal event; the
   terminal event itself requires an exact ACK before execution settlement.
8. Natural driver/TCP backpressure is load-bearing. If pausing the
   `SimpleQueryStream` does not stop unbounded driver buffering, STOP.

### Dedicated transaction observer

9. Do not probe through the five-slot SQLx pool. Create one dedicated,
   short-query tokio-postgres observer connection for each stored Connection
   that currently owns sessions. It holds a socket, not reusable credentials,
   and closes when the last session closes or the Connection is fenced.
10. Admission counts actual feature-created sockets: at most seven Query
    Sessions plus one observer per Connection; at most eight Connections with
    sessions; at most 24 Query Sessions plus their observers across the app.
    Never evict a live session implicitly.
11. The observer coalesces pending status requests for 5 ms and issues one
    parameterized query for all target PIDs. It returns `pid`,
    `backend_start::text`, and `state`; Rust compares both PID and backend-start
    identity. Queue wait and query execution have separate two-second bounds.
12. Probe after every execution, cancellation recovery, Commit, and Rollback.
    This adds one database roundtrip but avoids pool contention and preserves
    raw transaction SQL correctness. `CommandComplete` does not expose tags,
    so selective probing is not an available safe optimization.
13. Probe timeout, observer loss, missing identity, permission failure, or an
    unexpected `active` result produces `unknown`. Add
    `refresh_query_transaction_state`; it rehydrates a temporary connect spec,
    recreates the observer if needed, and rechecks without executing user SQL.
    Unknown never forces Rollback as the only recovery action.

### Retained protocol limits

14. Constants:
    - 1 MiB UTF-8 value bytes per retained cell.
    - 2 MiB serialized row after JSON escaping.
    - Flush target of 200 rows or 256 KiB.
    - 4 MiB hard serialized batch and unacknowledged-byte ceiling.
    - 10,000 retained rows per result set.
    - 50,000 retained rows and 32 MiB serialized rows per execution.
    - 64 retained result sets, 500 notices, and 1 MiB notice/metadata bytes per
      execution.
15. If a row exceeds the row cap after per-cell truncation, shorten later cells
    further. `resultSetStarted.columns` contains one `string | null` position
    per DataRow field; use null names after metadata exhaustion so positions do
    not shift. Terminal totals include omitted rows, result sets, notices,
    metadata bytes, column names, and typed truncation reasons.
16. `retainMoreRows: false` enters protocol-drain mode. The actor drains rows,
    emits bounded result metadata and terminal totals, and never cancels merely
    because the display budget filled.

### Transaction state

`QueryTransactionSnapshot` contains:

- `mode`: `autocommit | manual`
- `status`: `idle | active | failed | unknown`
- `manualIsolation`: `readCommitted | repeatableRead | serializable`

Rules:

- Autocommit is default. PostgreSQL owns multi-statement simple-query semantics.
- Switching to manual while idle is lazy. Immediately before the next
  execution, issue `BEGIN ISOLATION LEVEL <manualIsolation>`.
- Isolation is only a preference for that manager-issued manual transaction.
  It is not a claim about raw `SET TRANSACTION` or session defaults.
- Commit is allowed only in `active`. Rollback is allowed in
  `active | failed | unknown`.
- Mode/isolation changes during `active | failed | unknown` return a typed
  invalid-transition error.
- Raw SQL may begin, commit, or roll back in either mode. Never infer state from
  SQL text, mode, completion, error, cancellation, or health ticks.
- Map observer states `idle`, `idle in transaction`, and
  `idle in transaction (aborted)` to the snapshot. Anything else is `unknown`.
- In `unknown`, disable execute/Commit/mode/isolation, but allow Recheck,
  Rollback, and Close.
- Closing active/failed/unknown attempts Rollback for at most three seconds,
  then drops the socket. Socket close is the final rollback guarantee.

## Wire contract

Rust DTOs live in `src-tauri/src/query_session/protocol.rs`. Use
`#[serde(rename_all = "camelCase", tag = "kind")]` and non-secret UUIDs.
Every command handler receives the Tauri window as an injected argument and
rejects access when its label does not match the session owner. Only
registration, open, and heartbeat carry the process-lifetime `ownerId`.

Commands:

| Command | Payload | Result |
|---|---|---|
| `register_query_session_owner` | injected window + `ownerId` | `{ replacedSessionCount }` |
| `open_query_session` | injected window + `ownerId`, `sessionId`, `tabId`, `connectionId`, `onEvent: Channel<_>` | snapshot |
| `execute_query_session` | `sessionId`, `executionId`, `sql` | accepted |
| `ack_query_session_events` | `sessionId`, `executionId`, `ackThroughSequence`, `retainMoreRows` | `()` |
| `heartbeat_query_sessions` | `ownerId`, `sessionIds` | `{ refreshedSessionIds }` |
| `cancel_query_execution` | `sessionId`, `executionId` | `{ requested }` |
| `refresh_query_transaction_state` | `sessionId` | snapshot |
| `set_query_transaction_mode` | `sessionId`, `mode` | snapshot |
| `set_query_transaction_isolation` | `sessionId`, `manualIsolation` | snapshot |
| `commit_query_transaction` | `sessionId` | snapshot |
| `rollback_query_transaction` | `sessionId` | snapshot |
| `close_query_session` | `sessionId` | `()` |

Every Channel envelope has `sessionId`, `tabId`, `connectionId`, Connection
`generation`, monotonic `sequence`, optional `executionId`, `requiresAck`, and
one tagged event: `sessionState`, `executionStarted`, `resultSetStarted`,
`rowBatch`, `resultSetCompleted`, `notice`, `executionCompleted`, `sessionLost`,
or `sessionClosed`.

`executionCompleted` contains terminal status, timing, structured database
error, final transaction snapshot, all omission counters, and typed truncation
reasons. PostgreSQL stops a simple-query script at the first error. Preserve
earlier result sets and mark the current set partial/failed.

Use a typed `QuerySessionError` union for command rejection:

- `unsupportedEngine`
- `connectionClosing`
- `sessionLimitReached` with applicable limit
- `sessionNotFound`
- `ownerMismatch`
- `executionInProgress`
- `invalidSequence`
- `invalidTransactionTransition` with status, attempted action, and allowed actions
- `transactionStateUnknown` with `canRecheck: true`
- `transactionObserverUnavailable`
- `connectionLost`
- `timeout` with operation
- `database` with structured display fields that must not be logged

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Rust format | `just fmt` | exit 0 |
| Rust lint | `just lint` | exit 0, no warnings |
| Rust tests | `just test` | all non-ignored tests pass |
| Plain fixture | `pnpm db:postgres` | healthy on port 15432 |
| TLS fixture | `pnpm db:postgres-tls` | healthy on port 15433 |
| Live tests | `cargo test --manifest-path src-tauri/Cargo.toml query_session_live -- --ignored --test-threads=1` | all pass |
| Diff hygiene | `git diff --check` | no output |

Do not run install commands unless Cargo reports a missing dependency. Never
print environment variables or Connection records.

## Suggested executor toolkit

- Read tokio-postgres 0.7.18 `simple_query_raw`, `SimpleQueryMessage`,
  `Connection::poll_message`, and `CancelToken` docs.
- Read Tauri 2.11.2 `ipc::Channel`, `WindowEvent`, and `App::run` docs. Do not
  invent a Channel-closed API or desktop suspend event.

## Scope

**In scope**:

- `CONTEXT.md`
- `docs/adr/0021-dedicated-postgres-query-session-driver.md` (create)
- `package.json`
- `infrastructure/test-db/compose.yml`, `Makefile`, `README.md`
- `infrastructure/test-db/bin/test-db.sh`
- `infrastructure/test-db/postgres-tls/Dockerfile` (create)
- `infrastructure/test-db/postgres-tls/configure-tls.sh` (create)
- `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`
- `src-tauri/src/lib.rs`, `src-tauri/src/types.rs`
- `src-tauri/src/query_session/mod.rs` (create)
- `src-tauri/src/query_session/protocol.rs` (create)
- `src-tauri/src/query_session/postgres.rs` (create)
- `src-tauri/src/query_session/observer.rs` (create)
- `src-tauri/src/postgres/mod.rs`, `pool.rs`
- `src-tauri/src/postgres/connect_spec.rs` (create)
- `src-tauri/src/postgres/options.rs` (create)
- `src-tauri/src/commands/mod.rs`, `relational.rs`, `connections.rs`
- `src-tauri/src/commands/bastions.rs`, `managed.rs`, `settings.rs`
- `src-tauri/src/managed.rs`
- `plans/README.md` status text only

**Out of scope**:

- All `src/**/*.ts` and `src/**/*.tsx` files; Plan 002 owns frontend activation
- MySQL, SQLite, ClickHouse, or Redis session adapters
- Moving table/schema/admin/mutation/export paths away from SQLx
- Removing `run_query` or `QueryResult`
- COPY, parameters, prepared statement caching, fetch-more portals, disk spooling
- LISTEN/NOTIFY UI; consume notifications without logging payloads
- New TLS trust settings or production/daily-driver environments
- Commits, pushes, PRs, deployment, or external publication without approval

## Resume protocol

1. Read the `plans/README.md` status for Plan 001.
2. Inspect `git status --short` and `git diff -- <Scope paths>`.
3. Changes are recognized only when they match steps recorded as completed in
   `IN PROGRESS: through Step N`. Compare each changed symbol to that step.
4. If dirty work extends beyond recorded steps, or an excerpt changed for an
   unrelated reason, STOP. Do not discard or overwrite it.
5. Continue with the first incomplete step and update the status after its gate.

## Git workflow

- Suggested branch: `feat/query-session-backend`, only if the operator asks.
- Preserve unrelated changes. Do not commit unless the operator authorizes it.
- If commits are authorized, use logical checkpoint commits such as
  `Add PostgreSQL query session backend`.
- Do not push or open a PR unless instructed.

## Steps

### Step 1: Record the exact driver, observer, and lifecycle decision

Create ADR-0021. Record the accurate SQLx/tokio-postgres comparison, dedicated
observer rationale, one-roundtrip correctness cost, focus-aware owner lease,
four-batch/4-MiB credit window, retained-versus-transient memory distinction,
connection admission, TLS parity, typed errors, dark rollout, and rollback seam.
State that neither driver exposes ReadyForQuery status and that
`CommandComplete` has no command tag. Update `CONTEXT.md` for Query Session,
Query Execution, Result Set, and Query Outcome.

**Verify**:

```sh
rg -n "neither|ReadyForQuery|CommandComplete|observer|focused|credit|run_query" CONTEXT.md docs/adr/0021-dedicated-postgres-query-session-driver.md
```

Expected: every term is present and the ADR is `Accepted`.

### Step 2: Prove connection and TLS parity

Add pinned tokio-postgres 0.7.18, tokio-postgres-rustls 0.14, rustls 0.23 with
ring/std/tls12, and futures-util. Create one driver-neutral
`ResolvedPostgresConnectSpec` used by SQLx construction and the new connector.
Extract pure ordered driver-option SQL. Preserve the keepalive no-op.

Build the narrow driver spike: resolved routing, full socket/TLS/auth deadline,
Disable/Prefer semantics, permissive verifier, options, PID/backend-start,
zero-row columns, multiple result boundaries, bounded notices, and two-second
cancel request using the same transport.

Add the isolated self-signed TLS fixture on port 15433. Keep plaintext port
15432 unchanged to prove server-refusal fallback. Generate test certificates at
container start with correct ownership/mode. Add package, script, Make, and
README commands.

**Verify**: `just fmt && just lint && just test`, then both fixture commands and
the ignored live command. Expected: direct/TLS/refusal/cancel/result/notice
cases pass and no SQL appears in captured logs.

### Step 3: Add the dedicated batched transaction observer

Implement `observer.rs` with one observer per active stored Connection, 5-ms
coalescing, parameterized PID lookup, identity comparison, separate two-second
queue/query limits, reconnect-on-explicit-refresh, and teardown with the last
session. Count observers in admission before opening sockets. Do not acquire
from the SQLx pool.

Add a characterization test that completes eight target sessions together and
asserts requests coalesce, the SQLx pool is never acquired, and every target
receives one authoritative state. Measure and report local-fixture baseline
versus observed-query latency in test output, but do not add a flaky wall-clock
pass threshold.

**Verify**: `just test`. Expected: observer batching, timeout, permission,
identity mismatch, reconnect, admission, and no-pool-acquire tests pass.

### Step 4: Implement the actor, credit protocol, and typed errors

Create the manager, actor, protocol DTOs, commands, and activity recorder.
Enforce every limit and state rule above. Use cumulative ACKs with the exact
four-batch/4-MiB credit. A 10-second interval may mark a diagnostic `stalled`
state, but must not cancel or close; only 120 seconds of focused-time stall may
do so. Select driver reads and ACK waits against Cancel and Close.

Record `lastActivityAt` only after successful open, terminal `completed`, or
successful Commit/Rollback. Failed, cancelled, lost, accepted-only, and probe
events do not touch it.

**Verify**: `just fmt && just lint && just test`. Expected: serialization,
credit, cumulative ACK, overflow/drain, terminal-once, cancellation races,
typed rejection, transaction, refresh, and activity tests pass.

### Step 5: Make backend lifecycle cleanup authoritative

Bind owners to injected window labels. Handle Focused/Destroyed in Tauri's app
run callback. Pause focused-time deadlines on blur; grant a full deadline on
focus. Owner replacement, window destruction, app exit, Connection save/delete/
disconnect, bastion invalidation, managed stop/destroy/recreate, and destructive
credential reset must fence new opens and close actors plus observer before
target invalidation. Storage-mode migration does not close sessions.

Close actors concurrently under one three-second Connection deadline. For
non-restart exit, prevent exactly once, await up to three seconds, then exit.
Restart relies on immediate socket drop. Never wait for frontend ACK on app exit.

**Verify**:

```sh
rg -n "WindowEvent::(Focused|Destroyed)|begin_connection_teardown|close_connection|close_all" src-tauri/src
just test
```

Expected: all teardown paths visibly order session/observer close first; tests
cover reload owner replacement, focus pause/resume, window destroy, and exit.

### Step 6: Run live failure and platform characterization gates

Live tests cover temp-state reuse, isolated tabs, multi-result and zero-row
metadata, notice ordering, cancellation on plaintext/TLS, raw transaction SQL,
observer batching/unknown/recheck, four-batch credit, 120-focused-second ACK
loss, result/notice/metadata limits, 100k rows, one 16-MiB cell, network loss,
low role connection limits, and every backend teardown edge.

On macOS, run a manual characterization with an active transaction while the
window is minimized and while another app occludes it for at least five
minutes. No automatic rollback may occur while Tauri reports the window
unfocused. Refocus must grant 120 seconds for heartbeat/ACK recovery. Record the
result in the ADR implementation notes.

**Verify**: both fixtures, ignored live command, `just fmt`, `just lint`,
`just test`, and `git diff --check`. Expected: all pass, no sensitive logs, and
only in-scope files plus the Plan 001 README status are modified.

## Test plan

- `query_session/protocol.rs`: every command/event/error variant, nullable
  column names, cumulative ACK, omission totals.
- `query_session/mod.rs`: admission including observer sockets, maps/indexes,
  focused-time clocks, owner replacement, generation fences, idempotent close.
- `query_session/postgres.rs`: message reducer, byte ceilings, UTF-8 truncation,
  result boundaries, notice ordering, cancel and terminal races.
- `query_session/observer.rs`: batching, identity, queue/query timeout,
  permission failure, unknown, reconnect, and no SQLx pool acquisition.
- `postgres/connect_spec.rs` and `options.rs`: exact SQLx/tokio-postgres routing,
  TLS intent, deadline, options, ordering, and no secret formatting.
- Ignored live tests create a unique schema and always close sessions in cleanup.

## Done criteria

- [ ] Dark PostgreSQL session commands are registered; legacy callers unchanged.
- [ ] One actor owns one dedicated socket; one observer serves a Connection.
- [ ] Feature sockets obey 7+1 per Connection, 8 Connections, and 24 sessions.
- [ ] Neither the actor nor observer acquires from the five-slot SQLx pool.
- [ ] Four-batch/4-MiB cumulative credit and terminal ACK are exact.
- [ ] No ACK or heartbeat stall closes before 120 seconds of focused time.
- [ ] Unfocused windows do not expire; destroy/reload/exit cleanup is deterministic.
- [ ] Observer truth and Recheck cover idle/active/failed/unknown without SQL parsing.
- [ ] TLS success, refusal fallback, cancellation, and full handshake deadline pass.
- [ ] Every invalidation fences and closes session plus observer sockets first.
- [ ] Retained limits and omission counters hold; transient DataRow is not claimed bounded.
- [ ] No sensitive SQL/data/error payload is logged.
- [ ] `just fmt`, `just lint`, `just test`, ignored live tests, and `git diff --check` pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; after an authorized commit,
      the reviewer/operator changes it to `DONE: <completion SHA>`.

## STOP conditions

Stop and report if:

- Live code drifts from a load-bearing excerpt or resume work is unexplained.
- TLS, resolved tunnel endpoints, deadlines, or effective driver options diverge.
- The driver lacks zero-row RowDescription, notices, usable cancellation, or backpressure.
- The observer must use the SQLx pool or cannot distinguish idle/active/failed
  for `(pid, backend_start)` under ordinary privileges.
- Observation latency or connection overhead is unacceptable to the operator
  after the measured characterization. Do not silently weaken transaction truth.
- macOS focus events cannot protect a minimized/occluded active transaction from
  heartbeat or ACK expiry. Do not ship wall-clock rollback on a background window.
- Exact cumulative ACK/terminal settlement cannot be implemented with Channel.
- Any lifecycle path invalidates a target before session/observer close.
- The 16-MiB characterization crashes, hangs, or loses the session.
- A required source change is outside Scope or a gate fails twice.

## Maintenance notes

- Future PostgreSQL connection fields must map through the shared resolved spec.
- The observer roundtrip is the correctness cost of public-driver limitations.
  Optimize batching, not truth, unless a future driver exposes ReadyForQuery state.
- Tauri 2.11.2 does not provide a desktop webview-crash or Channel-closed event.
  Revisit liveness when that API changes.
- Result-payload eviction and user-visible budget ownership belong to Plan 002;
  never evict session actors to recover display memory.
