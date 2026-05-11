# ADR-0006 — ClickHouse is read-only by default; mutations are opt-in and async-aware

**Status**: Accepted (2026-05-11)

## Context

ClickHouse is one of the four engines listed in `CONTEXT.md`, but unlike
PostgreSQL/MySQL/SQLite it is not a row-oriented OLTP store. Three CH-specific
shape differences make a straight port of the PG implementation incorrect, not
just incomplete:

1. **No row-uniqueness primary key, no foreign keys.** `MergeTree` tables have
   an `ORDER BY` / sparse primary index; it is a sorting key, not a uniqueness
   constraint. Foreign keys do not exist in any CH table engine. The Row
   Identity model in `CONTEXT.md` (PK first, fallback to non-null unique
   indexes) has no equivalent — there is nothing in CH that guarantees a
   single row matches a `WHERE` clause.
2. **Mutations are asynchronous.** `ALTER TABLE … UPDATE/DELETE` queues a
   background mutation across all parts of the table; the HTTP call returns
   when the mutation is *accepted*, not when it is *applied*. Mutation status
   lives in `system.mutations` and `is_done` may not flip for seconds to
   minutes on large tables. The `commit_cell_edits` UX assumes a synchronous
   "row updated" — applying it to CH would lie to the user.
3. **No multi-statement transactions.** The PG cell-edit and DDL paths wrap
   batches in `BEGIN/COMMIT`. CH's experimental MergeTree transactions are
   per-partition and not appropriate for our flow. Each statement is
   independent; partial-failure semantics differ.

The frontend currently hard-codes `connection.engine === "PostgreSQL"` for
every editing surface (`store.ts:712`, `:903`, `:995`, `:1415`,
`table-editor-panel.tsx:131`, `table-structure-view.tsx:91`). That gate is the
right *behavior* but the wrong *encoding* — the question of "can this engine
mutate?" should come from a capability flag, not a literal engine name.

## Decision

ClickHouse ships in two tiers, gated separately:

**Tier 1 — Read-only completeness (default).** Everything that maps cleanly:
schema explorer, table data browsing, full table structure (columns + sorting
key + skip indices), database overview stats, schema relationships. The
existing "PostgreSQL only" walls for mutation surfaces stay up. No user-facing
promise of edit/insert/delete on CH yet.

**Tier 2 — Mutations (opt-in).** When this lands:

- **Editability is a backend capability, not an engine name.** Every command
  that mutates state returns or relies on a capability flag. The frontend
  drops `engine === "PostgreSQL"` checks in favor of the flag.
- **CH mutations surface their async nature honestly.** `commit_cell_edits`
  and `delete_rows` against CH return a `mutationId` plus a `state:
  "queued"` discriminant; the UI renders "Queued — applying in background"
  rather than "Committed in N ms". A separate command polls
  `system.mutations` for `is_done`.
- **Mutations are gated to MergeTree-family tables.** The table-engine field
  from `system.tables` is part of the structure response. Distributed/View/
  Kafka/Buffer tables surface as read-only with a clear reason.
- **`INSERT` is synchronous and unaffected.** It uses CH's normal HTTP insert
  path and reports "Inserted N rows" the same way PG does. This is the only
  mutation that doesn't need the async ceremony.
- **DDL is per-statement.** `execute_ddl` against CH runs each statement
  independently — no `BEGIN/COMMIT` wrapper. A mid-batch failure leaves
  earlier statements applied. The frontend preview shows the statement
  list so the user knows what they're committing to.

The PG Row Identity model is replaced for CH by a degraded form: the user
must edit / delete via the **sorting key columns** + any explicit sampling
columns. If the sorting key is non-unique, the mutation is permitted but a
`uniquenessGuarantee: "best-effort"` field on the response surfaces in the
UI — this matches CH's actual semantics.

## Consequences

- The "PostgreSQL is the reference engine" stance from ADR-0001 still holds
  for the *implementation order*. CH catches up on a per-feature basis; the
  capability flag mechanism makes "engine X gained feature Y" a localized
  change.
- Some CH-specific concepts (partition key, TTL, codec, projections,
  compression ratio) have no analogue on the existing `TableStructure`
  shape. We extend the shape with optional engine-specific fields rather
  than reshape the existing one — PG/MySQL/SQLite paths leave the new
  fields `None` and the UI hides those sections when absent.
- The async mutation flow imposes new state on the workspace store
  (in-flight mutation IDs, polling). This is genuinely new behavior, not
  just plumbing — the UI tax is real and is the main reason Tier 2 is
  scoped separately from Tier 1.
- Future engines that share CH's async-mutation shape (e.g. some columnar
  warehouses) inherit this model for free. Engines that don't (DuckDB,
  Trino on certain backends) keep the synchronous flow.
- The Row Identity fallback to "best-effort" is a softening of an
  invariant the rest of the app assumes. New surfaces that depend on
  exact-match identity (audit trails, optimistic concurrency) must
  inspect `uniquenessGuarantee` before assuming PG semantics.
- A future architecture proposal that suggests "let's just unify cell
  edits across engines" should be rejected: the synchronous-vs-async gap
  is fundamental, not cosmetic.
