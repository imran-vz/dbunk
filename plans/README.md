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
| [004](./004-table-browse-grid-integration.md)    | Server-backed browsing in table tabs        |       P0 |      L | 003        | DONE: ecefce8 (selected mock: B)               |
| [005](./005-result-mutation-backend.md)          | PostgreSQL Result Mutation backend          |       P0 |      L | 003, 004   | DONE: d98f8a1                                  |
| [006](./006-staged-mutation-review-integration.md) | Staged mutation review in table and query results | P0 |      L | 005        | DONE: 4e52c8a (selected mock: A) |
| [007](./007-safety-policy-backend.md)            | Backend-enforced production safety policy   |       P0 |      L | 005, 006   | DONE: cc16b3c                                  |
| [008](./008-safety-policy-activation.md)         | Safety policy activation and production identity | P0 |      L | 007        | TODO                                           |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed. This makes resume state
useful without authorizing commits implicitly.

## Current selection

- **Completed:** Plans 001 and 002 delivered the PostgreSQL query-session
  foundation of `PAR-001` through commit `26268ca`. Plans 003 and 004
  delivered and activated PostgreSQL Table Browse for `PAR-002` through
  commit `ecefce8` on 2026-08-21.
- **Completed (pending stamp):** Plans 005 and 006 delivered the `PAR-003`
  staged mutation review core; Plan 006 is `READY FOR REVIEW` with
  hardening commits through `4e52c8a` awaiting the operator's `DONE` stamp.
  Batch paste, deep value editors, Quick Look, and configurable copy
  formats are the `PAR-003` register items deliberately left for a
  follow-on plan.
- **Selected next:** `PAR-004`, backend-enforced production safety policy.
  Plans 007 and 008 were authored on 2026-08-23 at commit `4e52c8a` and
  execute in order after Plan 006 is stamped `DONE`. Plan 007 lands the
  dark backend: per-connection environment / Safe Mode / relational
  read-only fields (migration 15), a fail-closed PostgreSQL statement
  classifier extracted from the Plan 005 lexer, one shared policy gate
  asserted at all fifteen write-capable surfaces (typed
  `policyBlocked`/`policyNeedsConfirmation` on the query-session and
  result-mutation actors; tagged refusal strings on legacy commands), a
  belt-and-braces `default_transaction_read_only` session GUC, and a
  persisted confirmed-override audit. Plan 008 activates it: form
  controls, environment badges and production identity across sidebar /
  header / tabs / banner / status bar, one shared confirmation dialog that
  re-sends with `confirmed: true`, and the audit view in Settings.
  Smart-commit environment defaults and user-picked connection colors are
  deliberately deferred (`PAR-001` follow-ons / `PAR-005`–`PAR-006`).
- **Selected mock (004, completed):** B — stacked command bar, inspection
  popover, keyset next as the primary pager.
- **Selected mock (006):** A — persistent right-side mutation review
  inspector that keeps the result grid primary while showing grouped changes
  and exact generated DML.
- **Delivered boundary:** PostgreSQL table tabs now use typed server-side
  filters and sorting, bounded pagination/count behavior, cancellation,
  stale-response rejection, query inspection, durable grid preferences, and
  backend-authoritative row identity. Query-result mutation remains in
  `PAR-003`.

## Dependency order after Plans 001 through 004

Items 3 and later are candidate plans, not authored plans. Their identifiers
refer to the gap register rather than files in this directory.

1. Plans 001 and 002 delivered and activated the PostgreSQL query-session
   foundation. Non-blocking `PAR-001` follow-ons remain in the gap register.
2. Plans 003 and 004 delivered and activated `PAR-002` server-backed table
   browsing, reusing the query-session connect-spec, TLS, cancellation, and
   bounded-result semantics.
3. Plans 005 and 006 delivered the `PAR-003` execution path: the dark
   PostgreSQL Result Mutation backend, then one staged Mutation Draft model
   shared by browse-mode table tabs and query results with DML review
   before commit. Deep editors, Quick Look, batch paste, and copy formats
   remain follow-on register scope.
4. Plans 007 and 008 are the authored `PAR-004` execution path: a dark
   backend safety policy (environment classification, Safe Mode levels,
   enforced relational read-only, fail-closed statement classification,
   one gate at every write surface, override audit), then UI activation
   (form fields, production identity, confirmation flows). They build on
   `PAR-001` transaction state and the `PAR-003` mutation preview as
   planned.
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
