# ADR-0001 — PostgreSQL gets the reference implementation; other engines stub

**Status**: Accepted (2026-05-10)

## Context

dbunk's `Connection` model supports four engines: PostgreSQL, MySQL, SQLite,
ClickHouse. Several backend operations have engine-specific paths
(`fetch_table_structure_postgres`, `commit_cell_edits_postgres`,
`run_clickhouse_query`, etc.). The question is whether each new operation must
ship implementations for all four engines from day one.

The alternative considered was: every Tauri command has a complete matrix of
engine implementations before merging. Each new feature would be ~3× the
backend surface and some operations (e.g. `EXPLAIN ANALYZE FORMAT JSON`)
don't translate cleanly across engines.

## Decision

PostgreSQL is the reference implementation. New backend operations land
PG-only. MySQL, SQLite, and ClickHouse return a clear
`"<feature> is not supported for <engine> (PostgreSQL only)"` error string,
which the frontend surfaces as an inline message.

When an engine gains real support for a feature, the corresponding stub is
replaced. Coverage is tracked per-feature in `designs/FOLLOWUPS.md`.

## Consequences

- New features ship in days, not weeks.
- Users on non-PG engines hit visible "not supported" walls — acceptable
  since we communicate engine support per-feature.
- Architecture proposals that demand cross-engine abstractions (trait-per-
  feature, sealed engine enum dispatch in helper traits) should be evaluated
  against this ADR before being adopted, since they over-fit a
  PG-first reality.
