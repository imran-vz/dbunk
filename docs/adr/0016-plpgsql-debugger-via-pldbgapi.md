# ADR-0016 — PL/pgSQL debugger via `pldbgapi`

**Status**: Proposed — unbuilt (2026-05-14)

## Context

PostgreSQL has no first-party debugger protocol. The de-facto option
that DBeaver, pgAdmin, and EDB Postgres Studio all use is the
`pldbgapi` extension, originally written by EnterpriseDB and
shipping with most server distributions of Postgres but **not** on
managed services (AWS RDS, GCP Cloud SQL, Azure Database for
Postgres). The extension exposes a debug-session RPC over a regular
SQL connection:

- `pldbg_create_listener()` — returns a session id.
- `pldbg_set_breakpoint(session, function_oid, line)` — install bp.
- `pldbg_wait_for_target(session)` — block until a debugged call
  reaches the listener.
- `pldbg_step_into() / pldbg_step_over() / pldbg_continue()` — drive
  the debug session.
- `pldbg_get_stack(session)` — return call stack.
- `pldbg_get_variables(session)` — locals + globals at top frame.

Two open questions before we ship:

1. **Capability detection.** Not every server has `pldbgapi`. Must
   degrade clearly.
2. **Concurrency.** A debug session uses two connections (one to
   drive the listener, one to invoke the target). Pool-aware.

ROADMAP.md §5 marks this as `❌`. `PENDING_TASKS.md §PL/pgSQL
debugger` flags the same.

## Decision

**Detect on connect.** When a Postgres connection succeeds,
introspection (`load_pg_admin_snapshot` follow-up or new
`detect_pg_extensions`) checks `pg_extension WHERE extname =
'pldbgapi'`. The result is cached on the `Connection` runtime
record as `debugCapability: "available" | "missing" | "unknown"`.

**UI**: A "Debug" toolbar button in the query editor is enabled only
when `debugCapability === "available"`. When `missing`, the button
is disabled with a tooltip linking to install instructions
(`CREATE EXTENSION pldbgapi;` + privilege requirements).

**Tauri command surface** (new file
`src-tauri/src/postgres/debug.rs`):

- `debug_start_session(connection_id, fn_oid_or_call)` — opens two
  pooled connections, calls `pldbg_create_listener`, returns
  session id.
- `debug_set_breakpoint(session, fn_oid, line)`.
- `debug_step(session, kind: "into" | "over" | "out" | "continue")`.
- `debug_get_state(session)` — stack + locals snapshot.
- `debug_end_session(session)` — closes both pooled connections.

Each call returns a `DebugState` payload the frontend renders.
Polling vs. event-stream: v1 polls every 250 ms while a step is in
flight (matches DBeaver's behavior). Tauri-side event stream is a
follow-up optimization.

**Frontend surface**: New panel that takes over the query editor's
results region when a debug session is active. Replaces tabs with:

- Variables tree (locals + globals at top frame).
- Call stack (clickable frames).
- Breakpoints list (toggle per row in editor gutter).
- Continue / Step Over / Step Into / Step Out / Stop buttons.

**Out of scope for v1**:

- Watchpoints (`pldbg_wait_for_breakpoint` only — no conditional
  break).
- Multi-session debugging (one debug session per connection).
- Edit-and-continue.

## Consequences

- Hard dependency on a server-side extension that's missing on
  managed Postgres. Plan B is documented (see Alternatives) but
  shipping v1 with the dependency is a clearer message than a
  half-working built-in path.
- The debug command family lives in its own file because the
  lifecycle (two pooled connections per session) doesn't fit the
  one-shot `run_query` pattern.
- New `Connection` runtime field `debugCapability` adds one struct
  member; serializers tolerate missing as `unknown`.
- The first time a user hits Debug without `pldbgapi`, we want a
  clear empty state, not a Rust panic. Detection must precede every
  `debug_*` command.

## Alternatives considered

1. **Hand-rolled `RAISE NOTICE` instrumenter.** Insert auto-print
   statements between each PL/pgSQL line, run the function, tail the
   notice channel. Cheap to ship, doesn't require an extension, but
   no actual stepping or breakpoints — it's a tracer not a debugger.
   Filed for v2 as the "Plan B" for managed-service users.
2. **VS Code debug adapter protocol (DAP) shim.** Implement DAP over
   `pldbgapi` so external editors can debug too. Rejected — adds
   IPC surface area before we know in-app debugging works.
3. **EXPLAIN-based "execution debugger" (no `pldbgapi`).** Show the
   plan tree with row-counts and timings per node. Already exists as
   the EXPLAIN visualizer; not a substitute for stepping through
   PL/pgSQL.

## Open questions (for v1 implementation)

- Function picker: by `pg_proc.oid` or by `schema.name(arg_types)`
  signature? Latter is friendlier but ambiguous with overloads.
- Variables tree: how deep do we render composite types?
- Editor gutter breakpoint UX: click-to-toggle. Does that mean the
  Monaco editor needs a custom decoration source — yes, already wired
  for run-statement glyphs.

## Related

- ROADMAP.md §5 — PL/pgSQL debugger gap this closes.
- `src/components/query-editor/use-monaco-query-editor.ts` — Monaco
  decoration source the breakpoint gutter extends.
- ADR-0001 — Postgres-first. The debug ADR is a single-engine ADR;
  ClickHouse / SQLite have no equivalent surface.
