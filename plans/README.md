# dbunk plans

This is the single source of truth for plan state. The gap inventory is
[parity-gap-register.md](./parity-gap-register.md); queued work is in
[ROADMAP.md](../ROADMAP.md).

Executors must read a plan completely, honor its STOP conditions, and update
its status here when work finishes. Completed plan bodies are deleted once
recorded `DONE`; the SHA is the pointer into git history. Plans 001–030, their
evidence and mocks were retired on 2026-10-11. The last tree that held them is
[`3e66b65`](https://github.com/imran-vz/dbunk/tree/3e66b65537bbddc18232d8726fb2989b8650c406/plans).

## Open plans

| Plan | Title | Depends on | Status |
| --- | --- | --- | --- |
| [031](./031-native-redesign-and-engines.md) | Native redesign and multi-engine workspace (ADR-0033) | 027–030 | IN PROGRESS: through Step 4. Compact shell, theme pass, and native SQLite, Redis, ClickHouse and MySQL behind one `EngineSurface` seam are merged. Step 5, window, keyboard/AX and IME acceptance, remains. [Mock](./mocks/native-redesign/index.html) |
| [032](./032-drizzle-style-table-grid.md) | Drizzle-style PostgreSQL table grid | 028, 031 | IN PROGRESS: all five packages merged (`3b4681d`, polished in `170180f`); fmt/lint/test green and core flows verified live. Not yet exercised live: popover JSON editor, insert band, columns/pager/overflow popovers, Staging, read-only, outcome-unknown recovery |

## Completed plans

| Plan | Title | Status |
| --- | --- | --- |
| 001 | PostgreSQL Query Session backend foundation | DONE: 657553d |
| 002 | PostgreSQL Query Session editor integration | DONE: 26268ca |
| 003 | PostgreSQL Table Browse backend | DONE: 202f756 |
| 004 | Server-backed browsing in table tabs | DONE: ecefce8 |
| 005 | PostgreSQL Result Mutation backend | DONE: d98f8a1 |
| 006 | Staged mutation review in table and query results | DONE: 4e52c8a |
| 007 | Backend-enforced production safety policy | DONE: bd9f7ef |
| 008 | Safety policy activation and production identity | DONE: 5409d66 |
| 009 | Workspace navigation foundation | DONE: f66abaa |
| 010 | Open Anything activation and connection organization | DONE: 4facea1 |
| 011 | PostgreSQL connection security backend | DONE: b134766 |
| 012 | TLS controls, staged connection diagnosis, and truth pass | DONE: b45e294 |
| 013 | PostgreSQL object catalog and DDL workflow backend | DONE: 4833a42 |
| 014 | Object explorer, viewers, and lifecycle activation | DONE: 2e843a6 |
| 015 | PostgreSQL structure editor switchover to the typed DDL workflow | DONE: 84112dc |
| 016 | Table designer, routine, trigger, policy, and privilege DDL backend | DONE: 6b573f1 |
| 017 | Table designer, routine editor, and table security activation | DONE: 25d36f1 |
| 018 | File-backed PostgreSQL backup and restore foundation | DONE: de3272b |
| 019 | PostgreSQL backup and restore activation | DONE: ab33968 |
| 020 | Bounded PostgreSQL CSV import and export | DONE: 7745946 |
| 021 | Bounded PostgreSQL schema comparison foundation | DONE: 9312b41 |
| 022 | PostgreSQL schema comparison activation | DONE: db2dae2 |
| 023 | Bound parameters and row-limited reads in Query Sessions (dark backend) | DONE: 645417c |
| 024 | Tauri baseline, measurement harness and GPUI spike (migration stage 01) | DONE: 6c35f7e (stage 01 gate PASS, 2026-10-02) |
| 025 | Host-neutral core seam and Query Session extraction (stage 02) | DONE: 6c35f7e |
| 026 | First working native PostgreSQL workflow (stage 03) | DONE: 3f987c9 |
| 027 | Native PostgreSQL workspace shell (stage 04) | DONE: 45b59d0 |
| 028 | Native transactions, Table Browse and mutations (stage 05) | DONE: 45b59d0 |
| 029 | Native catalog, SQL tools, jobs and administration (stage 06) | DONE: 45b59d0 |
| 030 | Native parity, profile compatibility and package acceptance (stage 07) | DONE: 45b59d0; Tauri parity target closed by the hard migration (ADR-0033) |

Plans 027–030 measured parity against the removed Tauri app. ADR-0033 replaced
that target; their behavioral contracts (bounded resources, exact-save
recovery, owned workers, stored policy, no automatic retry) carry into Plan 031.
Their open window/AX/IME acceptance items fold into Plan 031 Step 5. VoiceOver
remains deferred.

`evidence/026/final-verification/performance-summary.json` is kept because
`tools/native/workspace_performance.py` uses it as its baseline.

Status values: `TODO`, `IN PROGRESS: through Step N`, `READY FOR REVIEW`,
`DONE: <completion SHA>`, `BLOCKED: <reason>`, or `REJECTED: <reason>`.

## Planning rules

- PostgreSQL is the reference engine per `docs/adr/0001-postgres-first-engine-coverage.md`.
- Correctness, bounded resource use, cleanup under failure, and predictable
  reconnect behavior take priority over feature breadth.
- A plan must be self-contained and stamped with the commit it was written
  against.
- Plans may not silently broaden from PostgreSQL into every relational engine.
- Backend changes must pass `just fmt`, `just lint`, and `just test`; native
  changes must pass `just fmt-native`, `just lint-native`, and
  `just test-native`.
- Publishing, production changes, commits, pushes, and PR creation require
  separate authorization.
