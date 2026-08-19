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
| [002](./002-query-session-editor-integration.md) | PostgreSQL Query Session editor integration |       P0 |      L | 001        | READY FOR REVIEW (selected mock: B)            |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed. This makes resume state
useful without authorizing commits implicitly.

## Dependency order after Plans 001 and 002

These are candidate plans, not authored plans. Their identifiers refer to the
gap register rather than files in this directory.

1. Plan 001 lands the backend dark. Plan 002 begins only after Plan 001 records
   its completion SHA, passes the local mock checkpoint, and then activates it.
2. `PAR-001` is complete only when both plans are DONE.
3. `PAR-002` server-backed browsing and `PAR-003` editable query results can
   then share its bounded result protocol and cancellation semantics.
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
