# dbunk parity plans

Generated from the DBeaver and TablePlus parity audit on 2026-08-18 at commit
`24432fb`.

This directory is the execution handoff for closing the documented parity gaps.
The canonical inventory is [parity-gap-register.md](./parity-gap-register.md).
Plans should be executed in dependency order unless a plan explicitly says
otherwise. Executors must read a plan completely, honor its STOP conditions,
and update its status here when work finishes.

## Execution order and status

| Plan                                             | Title                                       | Priority | Effort | Depends on | Status                                         |
| ------------------------------------------------ | ------------------------------------------- | -------: | -----: | ---------- | ---------------------------------------------- |
| [001](./001-query-session-foundation.md)         | PostgreSQL Query Session backend foundation |       P0 |      L | None       | DONE: 657553d                                  |
| [002](./002-query-session-editor-integration.md) | PostgreSQL Query Session editor integration |       P0 |      L | 001        | DONE: 26268ca (selected mock: B)               |
| [003](./003-table-browse-backend.md)             | PostgreSQL Table Browse backend             |       P0 |      L | 001, 002   | DONE: 202f756                                  |
| [004](./004-table-browse-grid-integration.md)    | Server-backed browsing in table tabs        |       P0 |      L | 003        | READY FOR REVIEW                               |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed. This makes resume state
useful without authorizing commits implicitly.

## Current selection

- **Completed:** Plans 001 and 002 delivered the PostgreSQL query-session
  foundation of `PAR-001` through commit `26268ca`. Plan 003 landed the
  PostgreSQL Table Browse backend at `202f756`.
- **Selected next:** `PAR-002` frontend activation, Plan 004 (planned at
  commit `26268ca`, 2026-08-19). Selected mock: **B**. Plan 004 wires
  PostgreSQL table tabs onto the typed browse contract.
- **Selected mock (004):** B — stacked command bar, inspection popover,
  keyset next as the primary pager.
- **Planning boundary:** PostgreSQL first, with typed server-side filters and
  sorting, bounded pagination/count behavior, cancellation, stale-response
  rejection, query inspection, and durable grid preferences. Query-result
  mutation remains in `PAR-003`.

## Dependency order after Plans 001 and 002

Items 3 and later are candidate plans, not authored plans. Their identifiers
refer to the gap register rather than files in this directory.

1. Plans 001 and 002 delivered and activated the PostgreSQL query-session
   foundation. Non-blocking `PAR-001` follow-ons remain in the gap register.
2. `PAR-002` server-backed browsing is authored as Plans 003 and 004, reusing
   the connect-spec, TLS, cancellation, and bounded-result semantics from the
   query-session foundation.
3. `PAR-003` editable-query-result work follows `PAR-002` so both table and
   query results can share identity and query metadata.
4. `PAR-004` safety policy should build on explicit transaction state and the
   generated mutation preview from `PAR-003`.
5. `PAR-005` workspace restoration should persist descriptors, never live
   database handles or server transaction state.
6. Professional PostgreSQL features (`PAR-007` through `PAR-011`) should follow
   once execution, editing, safety, and restoration are trustworthy.
7. Cross-engine breadth (`PAR-014`) should reuse the PostgreSQL contracts only
   after those contracts have proven stable.

## Planning rules

- PostgreSQL is the reference engine per `docs/adr/0001-postgres-first-engine-coverage.md`.
- Correctness, bounded resource use, cleanup under failure, and predictable
  reconnect behavior take priority over feature breadth.
- A plan must be self-contained and stamped with the commit it was written
  against.
- Plans may not silently broaden from PostgreSQL into every relational engine.
- Every implementation must pass `pnpm format`, `pnpm lint`, and
  `pnpm typecheck`. Rust changes additionally require `just fmt`, `just lint`,
  and `just test`.
- Publishing, production changes, commits, pushes, and PR creation require
  separate authorization.

## Findings considered and rejected

None. Features outside the recommended PostgreSQL-first boundary are recorded
as deferred decisions in the gap register rather than rejected.
