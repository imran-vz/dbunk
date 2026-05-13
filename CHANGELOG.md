# Changelog

## Postgres parity phases

Completed all remaining phases from `docs/design/PHASES.md`.

| Phase | Commit | Summary |
|---|---|---|
| Phase 3 — Data export upgrade | `a88fd11 feat: add table export formats` | Added export task configs plus SQL INSERT, HTML, Markdown, TXT, and Excel-readable workbook XML serializers. |
| Phase 4 — Data import wizard | `172bd82 feat: add data import planning core` | Added CSV parsing, header/NULL handling, workbook sheet normalization, column mapping, and COPY fast-path selection. |
| Phase 5 — DDL + dump/restore | `c241cd3 feat: add ddl backup planning` | Added DDL export scope planning, `pg_dump`/`pg_restore` argument builders, and cross-connection COPY SQL. |
| Phase 6 — Object navigator depth | `66f15df feat: add postgres object navigator depth` | Added Postgres object tree metadata and action SQL for materialized views and sequences. |
| Phase 7 — Admin tools | `37038e0 feat: add postgres admin tool queries` | Added session, lock, pending transaction, stats, maintenance, and blocker-chain helpers. |
| Phase 8 — SQL editor depth | `06f2f81 feat: add sql editor depth utilities` | Added EXPLAIN wrappers, plan normalization, snippets, and bind-variable substitution. |
| Phase 9 — Compare + generate | `0bee6eb feat: add schema compare and mock data tools` | Added schema comparison, data comparison, migration SQL helpers, and deterministic mock data generation. |
| Phase 10 — Specialized editors | `1d727fc feat: add specialized postgres editor helpers` | Added GRANT/RLS/index/FK/trigger SQL builders plus array, JSON, and PostGIS geometry helpers. |

Verification run during implementation:
- Targeted unit tests for each phase module.
- `bun format`
- `bun lint`
- `bun typecheck`

Assumptions:
- The remaining phases were implemented as reusable, tested core primitives so they can be wired into UI/Tauri command surfaces incrementally without destabilizing existing workflows.
- Phase 8 stretch items, PL/pgSQL debugger and visual query builder, remain deferred because the phase labels them as stretch.

Follow-up work:
- Wire the new phase primitives into dedicated UI flows and backend command endpoints where deeper native integration is desired.
