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
| 021 | Bounded PostgreSQL schema comparison foundation (dark) | P1 | L | 013–017, 020 | DONE: 9312b41 |
| [022](./022-postgres-schema-comparison-activation.md) | PostgreSQL schema comparison activation | P1 | L | 021 | READY FOR REVIEW (selected mock A); native/WebView fixture gate run 2026-10-01 |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed.

**Ready for review: Plan 022, PostgreSQL schema comparison activation.**
Plan 022 brings Plan 021's read-only comparison into the workbench. Imran
selected **A: Object inspector** on 2026-09-14. The observer, bounded reader,
workbench rail destination and Object inspector workspace were implemented
with focused tests that day. The native/WebView fixture and memory gate ran on
2026-10-01 against owned PostgreSQL 16.15, 16.14 and 17.11 fixtures and an SSH
bastion: 117 scripted checks pass on a production frontend bundle in the real
desktop WebView. The plan's execution record lists the scenarios, measurements,
the one message repaired and the limits of that evidence (one platform, a debug
native build, no screen capture or physical key input). DONE needs a separately
authorized completion commit.
[Plan 022](./022-postgres-schema-comparison-activation.md) ·
[Published brief and mocks](https://dbunk-schema-compare-plan-022.imran-vz.chatgpt.site) ·
[Local artifact](./mocks/schema-compare/index.html).

Plan 021 is DONE at `9312b41`. Its completion record and reviewed fixes were
committed on 2026-09-14. The completed plan body is retired; the
[historical execution record](https://github.com/imran-vz/dbunk/blob/9312b41ab2d2c92f48b54d2b3229332bf74641a2/plans/021-bounded-postgres-schema-comparison.md) retains the checks,
fixture matrix, performance measurements and remaining validation limits.
This bookkeeping update records that commit and does not claim new runtime tests.
The delivered backend covers ordinary-table definitions on PostgreSQL 16,
with bounded capture, structural differences, typed jobs, cancellation and
explicit coverage. UI activation and WebView memory validation are Plan 022;
wider object coverage, migration SQL and data comparison remain later slices.

Plan 020 is DONE at `7745946`, confirmed by Imran on 2026-09-05.
Its historical execution record retains the automated/live results and native
validation limitations known at completion.

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
