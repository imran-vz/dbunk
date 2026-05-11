# ADR-0001 — PostgreSQL is the reference engine; other engines catch up per-feature

**Status**: Accepted (2026-05-10) — revised 2026-05-11 to acknowledge
ClickHouse reaching mutation parity (ADR-0006) and to broaden the engine
roadmap beyond the original PG-only stance.

## Context

dbunk's `Connection` model supports four SQL engines today: PostgreSQL,
MySQL, SQLite, ClickHouse. Several backend operations have engine-specific
paths (`fetch_table_structure_postgres`, `commit_cell_edits_postgres`,
`run_clickhouse_query`, etc.). The original question was whether each new
operation must ship implementations for all four engines from day one.

The alternative considered was: every Tauri command has a complete matrix of
engine implementations before merging. Each new feature would be ~3× the
backend surface and some operations (e.g. `EXPLAIN ANALYZE FORMAT JSON`)
don't translate cleanly across engines.

We also expect to add engines that don't fit the SQL family at all — Redis
is the most likely near-term addition. KV stores have no `Schema`, no
`Foreign Keys`, no `DDL` in the shape the current model assumes.

## Decision

**PostgreSQL is the reference implementation; other engines catch up
per-feature.** When a feature is new, it may land PG-only and surface a
clear `"<feature> is not supported for <engine> (PostgreSQL only)"` error
on the unimplemented engines, which the frontend renders as an inline
message. Once an engine has a real user-facing need for a feature, the
implementation lands in its own module under `src-tauri/src/<engine>.rs`
(see `clickhouse.rs` after ADR-0006).

Catch-up is **per-feature, not all-at-once.** ClickHouse went from
`schema_explorer` + `run_query` in its initial cut to full Tier 2 mutation
parity in ADR-0006, feature by feature, without a unified Engine trait.
MySQL and SQLite remain at the columns-only structure probe today; they
catch up the same way when there's demand.

The **capability-flag mechanism** (`StructureCapabilities`, ADR-0006) is
the preferred way to gate UI on a per-engine, per-table basis — not
engine-name literals in the frontend. New mutation surfaces should
populate the flags from the backend and read them on the frontend; this is
what lets a CH MergeTree table be editable while a CH Distributed table
stays read-only without any UI-side engine knowledge.

For **non-SQL engines** (Redis, and whatever follows), the existing
`TableStructure` / `Schema` / `Cell Edit` / `Schema Relationships` model
does not fit. Adding one requires a follow-up ADR that defines either:

- a sibling DTO family for the non-SQL shape (e.g. a `KeySpace` analogue
  of `Schema`), with the workspace deciding which surface to render based
  on the engine's class, or
- explicit "not applicable for this engine class" capability flags so the
  SQL-shaped UI gracefully hides what doesn't apply.

This is a deliberate v2 question — we'll define the KV/non-SQL contract
when a real Redis (or similar) integration is in scope, not before.

## Consequences

- New PG features ship in days, not weeks.
- Users on non-PG engines see visible "not supported" walls for features
  that haven't caught up yet — acceptable since the wall names *which
  engine lacks support*, not "dbunk can't do this."
- Catch-up work for a specific engine (Tier 2 CH mutations in ADR-0006 is
  the canonical example) is a deliberate decision, not implicit churn —
  it earns its own ADR when the engine introduces semantic differences
  (async mutations, best-effort row identity, partition keys instead of
  primary keys, etc.).
- Cross-engine abstractions (Engine trait, generic introspection layer,
  shared DDL builder) are evaluated against **actual parity** of the
  engines that exist, not blanket rejected. The original ADR resisted
  them because only PG had real coverage; with CH now at Tier 2 the
  symmetry has materially closed for several operations. A future
  deepening proposal that observes ≥2 engines implementing the same
  shape verbatim is a legitimate signal, not an over-fit. (Per-feature
  traits are still suspect; per-shared-operation-set traits are not.)
- Adding a non-SQL engine (Redis, document stores, etc.) requires a
  follow-up ADR **before** implementation begins, because it changes the
  shape of the `Connection` model itself rather than slotting under it.
- Coverage stays tracked per-feature in `designs/FOLLOWUPS.md` and, for
  larger arcs, in `designs/CLICKHOUSE_PLAN.md`-shaped phased docs.
