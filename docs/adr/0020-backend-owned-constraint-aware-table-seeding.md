# ADR-0020 — Backend-owned, constraint-aware table seeding

**Status**: Accepted (2026-06-11)

Table Seeding — filling an existing relational table with generated fake rows — is owned by a backend Tauri command (`seed_table` → `dispatch::relational`, exhaustive per-engine match per ADR-0008; `not_applicable` for keyvalue engines), not by frontend SQL generation. Constraint-awareness requires reading the database (foreign-key values are sampled from the parent table's actual rows; unique integer columns sample the current MAX), and large row counts need batched inserts with progress rather than one giant SQL string through `runQuery`.

The invariant posture is: **guarantee** NOT NULL / type / FK / unique correctness by construction, **attempt** CHECK constraints and triggers, **never** partially apply. The whole run executes in one transaction; a database rejection rolls back everything and surfaces the engine's own error verbatim. dbunk does not parse or solve CHECK expressions — the user's recourse is per-column overrides (ranges, value lists, constants) in the Seed Spec.

A foreign key whose parent table is empty fails fast before generation with a message naming the parent ("seed `customers` first"). Seeding never writes to tables other than the one requested.

Generation is deterministic under a seed: an omitted seed is auto-picked and displayed after the run so any result is reproducible. Default generators come from column type plus name-based semantic inference (`email`, `first_name`, `created_at`, …); identity columns emit the database `DEFAULT`.

The existing frontend generator (`src/lib/mock-data.ts`, used by the compare tab for copyable mock INSERT SQL) stays as a preview/clipboard tool and is not the seeding engine.

## Per-engine notes

All four relational engines seed. Everything about *what* to generate is shared and pure (`src-tauri/src/seed.rs`); each engine module owns only its sampling SQL and its INSERT shape. Each engine's own type vocabulary is folded into one canonical set before classification (`Nullable(String)` → `text`, `int(11) unsigned` → `integer`, `tinyint(1)` → `boolean`), and `enum` / `set` columns seed from their declared members rather than a generator.

- **PostgreSQL, MySQL, SQLite** — one transaction per run, as above.
- **ClickHouse** — has no transactions, so "never partially apply" is met by sending the whole run as a single INSERT, which ClickHouse writes as one atomic block. That bounds a run to well under `max_insert_block_size`; runs above the cap are refused with an explanation rather than silently split into separately-committed parts. ClickHouse also has no foreign keys and no unique constraints, so its sorting key is deliberately *not* treated as unique, and `MATERIALIZED` / `ALIAS` / `EPHEMERAL` columns are never named in the INSERT.

MySQL and SQLite grew catalog introspection (columns, primary key, foreign keys, indexes) for this — seeding cannot guarantee NOT NULL / FK / unique without it. A run refuses to start if that introspection is unavailable, rather than generating rows the table will reject.

## Considered Options

- **Extend the frontend `mock-data.ts` path**: rejected — it cannot sample parent-table rows or existing unique values without round-tripping data through the UI, and pushing 100k-row INSERT strings through the query path is unpredictable under load.
- **Auto-cascade seeding of empty parent tables**: rejected for v1 — it is a graph problem (cycles, self-references, parent row counts) and performs writes the user did not ask for; the fail-fast error teaches the correct order instead. May return as an explicit opt-in later.
- **Parsing CHECK constraints to generate satisfying values**: rejected — a constraint-solver rabbit hole; transactional all-or-nothing plus verbatim engine errors plus per-column overrides covers real cases honestly.
- **Best-effort partial inserts (skip failing rows)**: rejected — a half-seeded table with an unknowable subset of rows is worse than a clean failure.
