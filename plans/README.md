# dbunk parity plans

Generated from the DBeaver and TablePlus parity audit on 2026-08-18 at commit
`24432fb`. The canonical gap inventory is
[parity-gap-register.md](./parity-gap-register.md).

Executors must read a plan completely, honor its STOP conditions, and update
its status here when work finishes. Completed plan bodies are deleted once
recorded `DONE` — the completion SHA below is the pointer into git history.

## Execution order and status

| Plan                                                  | Title                                                            | Priority | Effort | Depends on | Status                                         |
| ----------------------------------------------------- | ---------------------------------------------------------------- | -------: | -----: | ---------- | ---------------------------------------------- |
| 001                                                   | PostgreSQL Query Session backend foundation                      |       P0 |      L | None       | DONE: 657553d                                  |
| 002                                                   | PostgreSQL Query Session editor integration                      |       P0 |      L | 001        | DONE: 26268ca (selected mock: B)               |
| 003                                                   | PostgreSQL Table Browse backend                                  |       P0 |      L | 001, 002   | DONE: 202f756                                  |
| 004                                                   | Server-backed browsing in table tabs                             |       P0 |      L | 003        | DONE: ecefce8 (selected mock: B)               |
| 005                                                   | PostgreSQL Result Mutation backend                               |       P0 |      L | 003, 004   | DONE: d98f8a1                                  |
| 006                                                   | Staged mutation review in table and query results                |       P0 |      L | 005        | DONE: 4e52c8a (selected mock: A)               |
| 007                                                   | Backend-enforced production safety policy                        |       P0 |      L | 005, 006   | DONE: bd9f7ef                                  |
| 008                                                   | Safety policy activation and production identity                 |       P0 |      L | 007        | DONE: 5409d66 (selected mock: C)               |
| 009                                                   | Workspace navigation foundation (dark)                           |       P0 |      L | 001–008    | DONE: f66abaa                                  |
| 010                                                   | Open Anything activation and connection organization             |       P0 |      L | 009        | DONE: 4facea1 (selected mock: A)               |
| 011                                                   | PostgreSQL connection security backend (dark)                    |       P1 |      L | 001–010    | DONE: b134766                                  |
| 012                                                   | TLS controls, staged connection diagnosis, and truth pass        |       P1 |      L | 011        | DONE: b45e294 (selected mock: A)               |
| 013                                                   | PostgreSQL object catalog and DDL workflow backend (dark)        |       P1 |      L | 001–012    | DONE: 4833a42                                  |
| 014                                                   | Object explorer, viewers, and lifecycle activation               |       P1 |      L | 013        | DONE: 2e843a6 (selected mock: C)               |
| 015                                                   | PostgreSQL structure editor switchover to the typed DDL workflow |       P1 |      M | 013, 014   | DONE: 84112dc                                  |
| 016                                                   | PostgreSQL table designer, routine, trigger, policy, and privilege DDL backend (dark) |       P1 |      L | 013–015    | DONE: 6b573f1                                  |
| 017                                                   | Table designer, routine editor, and table security activation                  |       P1 |      L | 016        | DONE: 25d36f1 (selected mock: A)               |
| 018 | File-backed PostgreSQL backup and restore foundation (dark)                    |       P1 |      L | 017        | DONE: de3272b                     |
| 019 | PostgreSQL backup and restore activation | P1 | L | 018 | DONE: ab33968 (selected mocks: A + C) |
| 020 | Bounded PostgreSQL CSV import and export | P1 | L | 018, 019 | DONE: 7745946 (selected mock: A) |
| [021](./021-bounded-postgres-schema-comparison.md) | Bounded PostgreSQL schema comparison foundation (dark) | P1 | L | 013–017, 020 | IN PROGRESS: through Step 4 |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed.

**Currently active: Plan 021 implementation, through Step 4; Step 5 is next.**
Plan 020 is DONE at `7745946`, confirmed by Imran on 2026-09-05.
Completion is recorded from operator confirmation; the historical execution
record at that SHA retains the automated/live results and the native validation
limitations known at commit time. This status update does not claim new tests.
The completed plan body is retired per the register convention.

Plan 021 starts PAR-008 with a bounded, read-only PostgreSQL schema comparison
backend for ordinary table definitions on PostgreSQL 16 endpoints. The GPT-6
critique is incorporated; capture/deparser feasibility is the first implementation
gate, followed by normalization/byte contracts and the native job manager.
Implementation was authorized on 2026-09-05. The disposable PG16.15 harness
reproduced mixed deparser reads and a hidden dependency in a built-in array
constant. The conservative scalar recognizer and discovery/lock fixtures now
resolve that gate for a limited projection. Typed contracts, normalization,
value paging, allocation/response ownership primitives and bounded native catalog
capture and deterministic structural diff are implemented.
[Gate evidence and field matrix](../infrastructure/test-db/schema-compare/README.md).
Manager integration and runtime validation remain Steps 5–6. UI activation, migration SQL and data
comparison remain subsequent slices.
[Visual planning brief](./next-parity-item.html) · [Implementation draft](./021-bounded-postgres-schema-comparison.md).

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
