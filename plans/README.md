# dbunk parity plans

Generated from the DBeaver and TablePlus parity audit on 2026-08-18 at commit
`24432fb`. The canonical gap inventory is
[parity-gap-register.md](./parity-gap-register.md).

Executors must read a plan completely, honor its STOP conditions, and update
its status here when work finishes. Completed plan bodies are deleted once
recorded `DONE` — the completion SHA below is the pointer into git history.

## Desktop migration proposal

Separate from the parity audit. This is a decision document, not an executable
plan: it has no plan number, effort or steps, and its status is outside the
status values below. Each stage becomes a numbered plan in the table that
follows before it is executed.

| Proposal | Scope | Status |
| --- | --- | --- |
| [GPUI migration](./gpui-migration.html) | macOS first; current feature parity before replacing Tauri | APPROVED TO START by Imran on 2026-10-02. Stage 00 decided the same day: reason is architecture with no performance gain required, margins 30% and 10%, licence GPL-3.0-or-later (path Z, Zed's editor), feature rule backend-first with baseline `102568b`. Stage 01 is Plan 024; the first slice of stage 02 is Plan 025. Both are READY FOR REVIEW pending completion commits. **Stage 01 gate CLOSED: PASS / continue, 2026-10-02**, under Imran's request to finish and close it. All four criteria are met: text geometry and editor/results/cell-editor keyboard focus are verified; remaining full-application work is costed in the [gate review](./evidence/024/stage01-gate.md). Imran confirmed VoiceOver complete on 2026-10-02. No accessibility waiver or daily-driver cutover is approved. Stage 03 is [Plan 026](./026-native-postgres-workflow.md), with A + B + C selected by Imran on 2026-10-02 and a user-facing layout switcher. Stages 04 to 07 are not planned. |

The stage 01 closure activates the stage 00 backend-first rule: React receives
correctness, safety and data-loss fixes only; new capability lands dark in the
core and its UI is built natively. The parity baseline remains `102568b`.
Plan 024 stays `READY FOR REVIEW` until a separately authorized completion
commit supplies its SHA. The gate itself is closed; Plan 025 remains a separate
review.

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
| 022 | PostgreSQL schema comparison activation | P1 | L | 021 | DONE: db2dae2 (selected mock: A) |
| [023](./023-query-session-bound-parameters-and-row-limit.md) | Bound parameters and row-limited reads in PostgreSQL Query Sessions (dark) | P0 | L | 001, 002, 007 | READY FOR REVIEW |
| [024](./024-gpui-baseline-and-spike.md) | Tauri baseline, external measurement harness and GPUI spike (migration stage 01) | P1 | L | Migration stage 00 | READY FOR REVIEW: stage 01 gate CLOSED, PASS / continue on 2026-10-02; all four criteria met ([review](./evidence/024/stage01-gate.md)); completion SHA pending an authorized commit |
| [025](./025-shared-core-query-session-extraction.md) | Host-neutral core seam and Query Session service extraction (migration stage 02, first slice) | P1 | L | Migration stage 00 | READY FOR REVIEW |
| [026](./026-native-postgres-workflow.md) | First working native PostgreSQL workflow (migration stage 03) | P1 | L | Stage 01 gate closed; 025 implementation | READY FOR REVIEW: all five steps verified; 27 actual-window race runs, guarded real-result release performance and final release AX workflow pass ([final evidence](./evidence/026/implementation-review.md#final-verification)). Human VoiceOver PASS; A + B + C retained. Completion SHA pending an authorized commit |

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

Executors update their own status row after each completed step and mark
`READY FOR REVIEW` after all gates. The reviewer or operator records
`DONE: <completion SHA>` after the work is committed.

**Plan 023 is READY FOR REVIEW** (PAR-001 follow-ons, chosen by Imran on
2026-10-01, authored against `49c50e8`, now `677a7e8` on `main` with an
identical tree). All seven steps are complete and uncommitted. The backend is
dark: no frontend caller sends `parameters` or `rowLimit`.

- Delivered: named-parameter scan and `$k` rewrite with a position map, the
  shape planner, the cursor read with a row limit, the bound command, the
  `cancelled` outcome, the credit-loop repair, `describe_query_parameters`,
  ADR-0031.
- Evidence, 2026-10-01, disposable fixtures, macOS only: `just fmt`, `just
  lint`, `just test` (651 passed, 79 ignored), `pnpm format`, `pnpm lint`,
  `pnpm typecheck`, `pnpm test` (1,488 passed); 29 live tests on PostgreSQL
  16.14 and the TLS fixture; 19 of 19 fixture-port tests on PostgreSQL
  17.10. The Script shape's event sequence is unchanged apart from two new
  null fields.
- Two decisions made by Imran during validation: the `FETCH` is read eagerly
  so frontend credit never holds the wrapper transaction open, and the
  server's type-inference limit (`:x IS NULL` needs a cast) is a known limit
  for the activation plan, not a STOP.
- Not run: an SSH-tunnel route (no fixture), PostgreSQL 18, platforms other
  than macOS, the `ackTimeout` expiry end to end.

The [plan's execution record](./023-query-session-bound-parameters-and-row-limit.md)
lists every departure from the plan as written, and ADR-0031 holds the
measurements. Other candidates are in
[parity-gap-register.md](./parity-gap-register.md).

Plan 022 is DONE at `db2dae2`, confirmed by Imran on 2026-10-01. It brought Plan
021's read-only comparison into the workbench as the Object inspector (mock A,
selected 2026-09-14). The native/WebView fixture and memory gate ran on
2026-10-01 against owned PostgreSQL 16.15, 16.14 and 17.11 fixtures and an SSH
bastion: 117 scripted checks pass on a production frontend bundle in the real
desktop WebView. The completed plan body is retired; the
[historical execution record](https://github.com/imran-vz/dbunk/blob/db2dae24c504248d62f15f772bf82e4c8d1f5ff2/plans/022-postgres-schema-comparison-activation.md) retains the scenarios, measurements, the
one message repaired and the limits of that evidence (one platform, a debug
native build, no screen capture or physical key input). This bookkeeping
update records that commit and does not claim new runtime tests.
[Published brief and mocks](https://dbunk-schema-compare-plan-022.imran-vz.chatgpt.site) ·
[Local artifact](./mocks/schema-compare/index.html).

Plan 021 is DONE at `9312b41`. Its completion record and reviewed fixes were
committed on 2026-09-14. The completed plan body is retired; the
[historical execution record](https://github.com/imran-vz/dbunk/blob/9312b41ab2d2c92f48b54d2b3229332bf74641a2/plans/021-bounded-postgres-schema-comparison.md) retains the checks,
fixture matrix, performance measurements and remaining validation limits.
This bookkeeping update records that commit and does not claim new runtime tests.
The delivered backend covers ordinary-table definitions on PostgreSQL 16,
with bounded capture, structural differences, typed jobs, cancellation and
explicit coverage. UI activation and WebView memory validation were Plan 022;
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
