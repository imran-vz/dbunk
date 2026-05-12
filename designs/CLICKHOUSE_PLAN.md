# ClickHouse — Phased Coverage Plan

Anchored on ADR-0006 (CH is read-only by default; mutations are opt-in and
async-aware). Tier 1 closes the read-only story; Tier 2 lights up mutations
behind capability flags. Each phase is sized to ship as one PR.

References:
- Backend dispatch: `src-tauri/src/lib.rs`
- CH module: `src-tauri/src/clickhouse.rs`
- PG reference: `src-tauri/src/postgres.rs`
- Engine-aware UI sites: `store.ts:712,903,995,1415`,
  `table-editor-panel.tsx:131`, `table-structure-view.tsx:91`
- Existing DDL builder (PG): `src/lib/ddl/postgres.ts`

---

## Tier 1 — Read-only completeness

Goal: every read-only surface that "PostgreSQL only"-walls or silently empties
on CH renders correctly. No user-visible promise of mutation support changes.

### Phase 1.1 — Backend introspection (the bulk of Tier 1)

**Backend (`src-tauri/src/clickhouse.rs`):**
- `fetch_table_structure(connection, schema, table)`:
  - Columns: `system.columns WHERE database = ? AND table = ?` — pull
    `name`, `type`, `default_kind`, `default_expression`, `is_in_sorting_key`,
    `comment`. Sorting-key columns set `is_primary_key = true` so the existing
    `ColumnInfo.is_primary_key` slot carries the CH-correct meaning (sorting
    membership, not uniqueness).
  - Primary key: list of sorting-key column names from
    `system.tables.sorting_key`. Goes into the existing
    `TableStructure.primary_key` field; capability flag stays the lever for
    the UI to relabel as "Sorting key".
  - Foreign keys: empty (`capabilities.foreign_keys = false`).
  - Indexes: `system.data_skipping_indices` — surface name + `expr` +
    `type` (minmax/set/bloom_filter/etc.). Re-uses `IndexInfo` with `method`
    holding the CH index type and `is_unique = false` always.
  - Constraints: `system.tables.constraints` if present (CH supports CHECK).
  - Capabilities: `columns: true`, `primary_key: true`, `foreign_keys: false`,
    `indexes: true`, `constraints: true` (if any).

- `fetch_database_overview_stats(connection)`: aggregate from
  `system.parts WHERE database = ? AND active`:
  - `database_size_bytes` = `sum(bytes_on_disk)` — compressed, on-disk
  - `table_size_bytes` = same (CH doesn't separate "table" vs "index" the
    way PG does)
  - `index_size_bytes` = `sum(primary_key_bytes_in_memory)` (best analogue)
  - `row_count_estimate` = `sum(rows)` — exact and cheap, despite the field
    name
  - `table_count` = distinct tables
  - `schema_count` = 1 (the active database)
  - `index_count` = `(SELECT count() FROM system.data_skipping_indices
    WHERE database = ?)`
  - `connection_count` = `(SELECT count() FROM system.processes)`

- Shared `reqwest::Client` cached once per process (`OnceLock<Client>` so
  TLS handshakes amortize across the schema-explorer fan-out).

- `fetch_schema_relationships(connection, schema)`: returns
  `SchemaRelationships { tables: [...], foreign_keys: [] }` — populate tables
  + columns from `system.columns` so the relationship-map view at least
  renders nodes; the FK array stays empty deliberately.

**Dispatch (`src-tauri/src/lib.rs`):**
- Replace the four CH-related stubs:
  - `load_table_structure` (line 784): route CH to `clickhouse::fetch_table_structure`
  - `load_database_overview_stats` (line 943): route CH to
    `clickhouse::fetch_database_overview_stats`
  - `load_schema_relationships` (line 924): route CH to
    `clickhouse::fetch_schema_relationships`
- `execute_ddl`, `commit_cell_edits`, `insert_row`, `delete_rows` keep their
  "PostgreSQL only" walls until Tier 2.

**Tests:** unit tests on the URL/parser helpers already exist. Add unit tests
for any new SQL builders (e.g. the `system.parts` aggregation) as pure
string-comparison tests, mirroring the PG builder tests.

**Risks / decisions:** none load-bearing — every choice here lines up with
existing types. The only judgment call is overloading `is_primary_key` with
"member of sorting key" semantics, which is documented inline and matches
ADR-0006.

### Phase 1.2 — Connection form gaps

**Backend:**
- Add two optional fields to `StoredConnection`:
  - `useHttps: bool` (default false)
  - `urlPath: String` (default empty; e.g. `/clickhouse` for proxied
    deployments)
- `clickhouse::url` consumes both. Don't ship a request-timeout field yet —
  reqwest defaults are fine for v1; revisit if users complain.

**Frontend:**
- `new-connection-form.tsx` and `edit-connection-dialog.tsx`: when
  `selectedEngine === "ClickHouse"`, show a `Use HTTPS` switch and a
  `URL path` input under Advanced Options. The existing SSL switch is
  PG/MySQL only — leave it as is, don't conflate.
- Default port placeholder follows scheme: `8443` when HTTPS is on, `8123`
  off (existing `portPlaceholder` logic at `new-connection-form.tsx:109`).
- Store schema (`store.ts`): extend the `Connection` and `StoredConnection`
  TypeScript types with the two new fields.

### Phase 1.3 — UI polish for read-only CH

These are tiny but visible — they're the difference between "CH appears to
work" and "CH appears thoughtful".

**`table-structure-view.tsx`:**
- `PrimaryKeySection`: when `engine === "ClickHouse"` and the array is
  populated, render the heading as `Sorting key` instead of `Primary key`,
  with a tooltip: "ClickHouse uses the sorting key as a sparse primary
  index. It does not enforce uniqueness." The data still comes from
  `structure.primary_key`.
- `ForeignKeysSection` `UnsupportedNotice`: when engine is CH, render a
  more honest message: "ClickHouse does not support foreign keys."
  (Currently it says "Foreign keys are not supported on ClickHouse" which
  reads like a dbunk limitation rather than a CH one.)
- `IndexesSection` heading: when CH, label as `Skip indices`. Each row's
  `method` field already carries the CH index type, so individual rows
  read correctly.

**`schema-relationship-map.tsx`** (not opened above — confirm during
implementation):
- When the relationships call returns tables but zero foreign keys *and*
  the engine is CH, render an informational banner explaining that the
  graph will only show tables, not edges.

**Database overview view** (the `WorkspaceDatabaseOverview` referenced in
`designs/FOLLOWUPS.md`):
- For CH, show both compressed (`database_size_bytes`) and uncompressed
  bytes if we add an `uncompressed_size_bytes` field — useful and easy to
  expose. Drop the "estimate" footnote on row count for CH (the value is
  exact for `MergeTree`).

---

## Tier 2 — Mutations

Goal: CH gets edit/insert/delete/DDL. Async mutations surface their async
nature; the frontend stops gating on engine name.

### Phase 2.1 — Capability flags + sync INSERT

**Backend types (`src-tauri/src/types.rs`):**
- Add `MutationCapabilities` (or extend `StructureCapabilities`):
  - `can_insert_rows: bool`
  - `can_update_rows: bool` (true for PG/MySQL; true for CH MergeTree-family
    tables only)
  - `can_delete_rows: bool` (same)
  - `can_alter_schema: bool` (true for PG; true for CH MergeTree)
  - `update_semantics: "synchronous" | "async"` (PG = sync, CH = async)
  - `uniqueness_guarantee: "exact" | "best-effort"` (PG = exact when PK
    exists, CH = best-effort)
- Returned by `fetch_table_structure` so the UI knows what to allow per
  table.

**Backend dispatch:**
- `insert_row` for CH: build `INSERT INTO db.table (cols) VALUES (...)`,
  POST as the request body. Synchronous; returns rows_affected: 1.
- The `commit_cell_edits` and `delete_rows` walls stay up for one more
  phase to let the capability-flag UI plumbing land first.

**Frontend:**
- `store.ts`: replace the four `engine !== "PostgreSQL"` literal checks
  with reads from `tableStructure[key].capabilities.can_*`. Each branch
  produces an engine-aware error message ("This table's engine
  (`Distributed`) does not support inserts" beats "PostgreSQL only").
- `table-editor-panel.tsx:131,143,144`: the `isPostgres` flag becomes
  `canInsert` / `canDelete` / `canEdit` derived from capabilities.
- `table-structure-view.tsx:91`: `editable` derived from
  `capabilities.can_alter_schema` instead of `engine === "PostgreSQL"`.

**Risk:** the per-table capability is the right granularity (a CH instance
mixes MergeTree + Distributed tables freely). Make sure the capability
arrives *with* the structure, not as a separate fetch — otherwise the UI
flickers.

### Phase 2.2 — Async UPDATE/DELETE with mutation polling

**Backend:**
- `commit_cell_edits` for CH: build `ALTER TABLE db.table UPDATE col1 = ?,
  col2 = ? WHERE sorting_key = ?` (one statement per edit). POST, capture
  the response (CH returns the mutation ID via the `X-ClickHouse-Query-Id`
  header — confirm during implementation). Return:
  ```rust
  CommitCellEditsResult {
      rows_affected: 0, // unknown until mutation completes
      runtime_ms: 0,    // ditto
      mutation_id: Option<String>, // new
      state: "queued" | "committed", // new discriminant
  }
  ```
  PG continues to return `state: "committed"`; CH returns
  `state: "queued"` with the mutation ID.
- `delete_rows` for CH: same pattern with `ALTER TABLE … DELETE WHERE …`.
- New command `poll_mutation_status(connection_id, mutation_id)`: queries
  `system.mutations WHERE mutation_id = ?` and returns `is_done` plus
  `latest_failed_part` / `latest_fail_reason` if any.

**Frontend:**
- Cell-edit and delete-row commit flows: when `state === "queued"`, the
  workspace store records `{ mutationId, startedAt, type: "update" |
  "delete" }` and starts polling on a 2 s interval until `is_done` or
  `latest_fail_reason`.
- `data-grid.tsx` (or the cell-edit footer): show "Queued — applying in
  background" with a small spinner instead of "Committed in N ms" when
  the engine is CH. Cancel button stops polling but does NOT cancel the
  CH mutation (CH supports `KILL MUTATION` — possibly worth wiring later;
  scope it explicitly out of this phase).
- `query-history` records the mutation ID so users can find it later.
- Status bar: a new `Pending mutations: N` indicator surfaces when any
  CH mutation is in-flight, mirroring the existing health-check pill.

**Risks:** polling fan-out is the main one — multiple in-flight mutations
shouldn't stampede `system.mutations`. Use a single "select all in-flight"
query keyed by connection rather than one-per-mutation.

### Phase 2.3 — DDL builder + execute_ddl for CH

**Frontend (`src/lib/ddl/clickhouse.ts`):**
- New module mirroring `postgres.ts`, but with CH syntax:
  - `ADD COLUMN name Type [DEFAULT expr]`
  - `DROP COLUMN name`
  - `RENAME COLUMN old TO new`
  - `MODIFY COLUMN name NewType` (no separate set_type / set_nullable —
    CH folds them into one statement; nullable is part of the type, e.g.
    `Nullable(Int32)`)
  - `MODIFY COLUMN name DEFAULT expr` / `REMOVE DEFAULT`
- Same `PendingChange` / `ColumnChangeKind` shapes, with `kind:
  "set_nullable"` translated into a `MODIFY COLUMN` that wraps/unwraps
  `Nullable(...)`.
- A `classifyDestructive` analogue — DROP COLUMN is destructive; type
  changes that narrow are destructive; the test surface mirrors the PG
  test file.

**Frontend (`table-structure-view.tsx`):**
- Switch the import from `@/lib/ddl/postgres` to a dispatcher that picks
  the builder by engine. The structure view stays oblivious to engine
  beyond that one indirection.

**Backend (`src-tauri/src/clickhouse.rs`):**
- `execute_ddl(connection, sql)`: split on `;`, run each statement
  independently, return per-statement results (which one succeeded, which
  one failed). No transaction wrapper.
- Frontend renders the per-statement result list when DDL is multi-
  statement, so a partial failure is legible rather than confusing.

**Risks:** the `;` splitter naively breaks on `;` inside string literals.
Reuse / share a real splitter with the existing query-editor "current
statement" work tracked in `designs/FOLLOWUPS.md` (Phase 6 — Run dropdown).
For v1, document the limitation: comments and string literals containing
`;` are not handled. Same caveat the PG path already implicitly has.

### Phase 2.4 — Engine-of-table awareness everywhere it matters

A handful of polish items that fall out once Phase 2.3 is done:

- Sidebar table list: a small icon next to non-MergeTree tables (View,
  Distributed, Kafka) so users see at a glance that the table won't be
  editable. Tooltip names the engine.
- Connections card: under Details, show CH server version (from `SELECT
  version()`) the same way we'd show PG server version once that lands.
- DDL preview: when editing a CH MergeTree table, surface the existing
  ORDER BY / PARTITION BY in a read-only banner above the column editor —
  changing those is a heavyweight operation that should be a deliberate
  separate flow, not a "modify column" sibling.

---

## Cross-cutting deferred items

Things that will surface during implementation but should not gate Tier 1
or Tier 2:

- **`KILL MUTATION`** support — useful but not on the critical path.
- **Projections, materialized views, dictionaries** — first-class CH
  concepts the structure view doesn't model. Defer until users ask.
- **TTL clauses** — show on the structure view as a banner (read-only) in
  Tier 1; editing TTL waits until users ask.
- **Query EXPLAIN for CH** — a separate effort tracked under the existing
  Explain follow-up in `designs/FOLLOWUPS.md`.
- **SQL completions for CH dialect** — `src/lib/sql-completions.ts` is
  PG-shaped today. Lower priority; defer until we see queries failing
  because of bad completions.
